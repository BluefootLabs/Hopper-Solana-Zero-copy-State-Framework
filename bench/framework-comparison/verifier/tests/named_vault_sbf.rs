//! Exercise the published example's actual ELF, including CPI and rollback.
use mollusk_svm::{program::loader_keys::LOADER_V3, Mollusk};
use solana_account::Account;
use solana_instruction::{AccountMeta, Instruction};
use solana_instruction_error::InstructionError;
use solana_pubkey::Pubkey;

#[path = "../../../../examples/hopper-vault/src/state.rs"]
mod state;

struct VaultHarness {
    svm: Mollusk,
    program: Pubkey,
    payer: Pubkey,
    vault: Pubkey,
    initial: Vec<(Pubkey, Account)>,
}

impl VaultHarness {
    fn new(prefund: u64) -> Self {
        let elf = std::fs::read(std::env::var("HOPPER_NAMED_VAULT_SBF").unwrap()).unwrap();
        let program = Pubkey::new_unique();
        let payer = Pubkey::new_unique();
        let vault = Pubkey::new_unique();
        let mut svm = Mollusk::default();
        svm.add_program_with_loader_and_elf(&program, &LOADER_V3, &elf);
        Self {
            svm,
            program,
            payer,
            vault,
            initial: vec![
                (payer, Account::new(1_000_000_000, 0, &Pubkey::default())),
                (vault, Account::new(prefund, 0, &Pubkey::default())),
                mollusk_svm::program::keyed_account_for_system_program(),
            ],
        }
    }

    fn ix(&self, op: u8, amount: u64) -> Instruction {
        let mut data = vec![op];
        if op != 0 {
            data.extend_from_slice(&amount.to_le_bytes());
        }
        let mut metas = vec![
            AccountMeta::new(self.payer, true),
            AccountMeta::new(self.vault, op == 0),
        ];
        if op <= 1 {
            metas.push(AccountMeta::new_readonly(Pubkey::default(), false));
        }
        Instruction::new_with_bytes(self.program, &data, metas)
    }

    fn init(&self) -> Vec<(Pubkey, Account)> {
        let result = self.svm.process_instruction(&self.ix(0, 0), &self.initial);
        assert_eq!(result.raw_result, Ok(()));
        let mut expected = self.initial.clone();
        let rent = self.svm.sysvars.rent.minimum_balance(state::Vault::LEN);
        let top_up = rent.saturating_sub(expected[1].1.lamports);
        expected[0].1.lamports -= top_up;
        expected[1].1.lamports += top_up;
        expected[1].1.owner = self.program;
        expected[1].1.data = vec![0; state::Vault::LEN];
        state::Vault::write_init_header(&mut expected[1].1.data).unwrap();
        println!(
            "VAULT_HEADER_HEX={}",
            expected[1].1.data[..16]
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        );
        expected[1].1.data[16..48].copy_from_slice(self.payer.as_ref());
        assert_eq!(result.resulting_accounts, expected);
        println!(
            "vault init prefund={} CU={}",
            self.initial[1].1.lamports, result.compute_units_consumed
        );
        result.resulting_accounts
    }

    fn reject(
        &self,
        label: &str,
        ix: &Instruction,
        accounts: &[(Pubkey, Account)],
    ) -> InstructionError {
        let result = self.svm.process_instruction(ix, accounts);
        assert!(result.raw_result.is_err(), "{label}");
        assert_eq!(
            result.resulting_accounts, accounts,
            "{label}: partial state change"
        );
        println!(
            "{label}: {:?}, CU={}, rollback",
            result.raw_result, result.compute_units_consumed
        );
        result.raw_result.unwrap_err()
    }
}

#[test]
#[ignore = "requires HOPPER_NAMED_VAULT_SBF"]
fn named_initialization_and_custody_round_trip() {
    for prefund in [0, 500_000, 3_000_000] {
        let h = VaultHarness::new(prefund);
        let accounts = h.init();
        h.reject("reinitialization", &h.ix(0, 0), &accounts);
        let result = h.svm.process_instruction(&h.ix(1, 321), &accounts);
        assert_eq!(result.raw_result, Ok(()));
        let mut expected = accounts.clone();
        expected[0].1.lamports -= 321;
        expected[1].1.lamports += 321;
        expected[1].1.data[48..56].copy_from_slice(&321u64.to_le_bytes());
        assert_eq!(result.resulting_accounts, expected);
        println!("vault deposit CU={}", result.compute_units_consumed);
        let result = h.svm.process_instruction(&h.ix(2, 321), &expected);
        assert_eq!(result.raw_result, Ok(()));
        assert_eq!(result.resulting_accounts, accounts);
        println!("vault withdraw CU={}", result.compute_units_consumed);
    }
}

#[test]
#[ignore = "requires HOPPER_NAMED_VAULT_SBF"]
fn initialization_and_access_refusals_preserve_all_accounts() {
    let h = VaultHarness::new(0);
    for slot in [0, 1] {
        let mut ix = h.ix(0, 0);
        ix.accounts[slot].is_signer = false;
        h.reject("unsigned init", &ix, &h.initial);
        let mut ix = h.ix(0, 0);
        ix.accounts[slot].is_writable = false;
        h.reject("readonly init", &ix, &h.initial);
    }
    let mut poor = h.initial.clone();
    poor[0].1.lamports = 1;
    h.reject("init insufficient rent", &h.ix(0, 0), &poor);
    let accounts = h.init();
    for op in [1, 2] {
        h.reject("zero amount", &h.ix(op, 0), &accounts);
        let mut ix = h.ix(op, 1);
        ix.accounts[0].is_signer = false;
        h.reject("unsigned authority", &ix, &accounts);
        let mut ix = h.ix(op, 1);
        ix.accounts[1].is_writable = false;
        h.reject("readonly vault", &ix, &accounts);
        let mut wrong = accounts.clone();
        wrong[1].1.data[16] ^= 1;
        h.reject("wrong authority", &h.ix(op, 1), &wrong);
        let mut wrong = accounts.clone();
        wrong[1].1.owner = Pubkey::new_unique();
        h.reject("wrong owner", &h.ix(op, 1), &wrong);
        let mut wrong = accounts.clone();
        wrong[1].1.data[4] ^= 1;
        h.reject("wrong layout", &h.ix(op, 1), &wrong);
    }
    assert_eq!(
        h.reject("insufficient tracked balance", &h.ix(2, 1), &accounts),
        InstructionError::Custom(6001)
    );
    assert_eq!(
        h.reject(
            "insufficient depositor funds",
            &h.ix(1, 1_000_000_000),
            &accounts,
        ),
        InstructionError::Custom(1)
    );
    let mut overflow = accounts.clone();
    overflow[1].1.data[48..56].copy_from_slice(&u64::MAX.to_le_bytes());
    assert_eq!(
        h.reject(
            "overflow after successful transfer CPI",
            &h.ix(1, 1),
            &overflow,
        ),
        InstructionError::ArithmeticOverflow
    );
}
