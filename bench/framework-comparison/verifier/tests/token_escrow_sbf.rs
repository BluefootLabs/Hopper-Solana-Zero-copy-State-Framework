//! Real escrow ELF + canonical SPL Token, including atomic CPI rollback.
use mollusk_svm::{program::loader_keys::LOADER_V3, Mollusk};
use solana_account::Account;
use solana_instruction::{AccountMeta, Instruction};
use solana_pubkey::Pubkey;
#[path = "../../../../examples/hopper-escrow/src/state.rs"]
mod state;

struct Harness {
    svm: Mollusk,
    program: Pubkey,
    accounts: Vec<(Pubkey, Account)>,
}
fn amount(account: &Account) -> u64 {
    u64::from_le_bytes(account.data[64..72].try_into().unwrap())
}
fn set_amount(account: &mut Account, n: u64) {
    account.data[64..72].copy_from_slice(&n.to_le_bytes());
}
impl Harness {
    fn new() -> Self {
        let mut svm = Mollusk::default();
        let program = Pubkey::new_unique();
        svm.add_program_with_loader_and_elf(
            &program,
            &LOADER_V3,
            &std::fs::read(std::env::var("HOPPER_ESCROW_SBF").expect("set HOPPER_ESCROW_SBF"))
                .unwrap(),
        );
        mollusk_svm_programs_token::token::add_program(&mut svm);
        let mut keys: Vec<_> = (0..14).map(|_| Pubkey::new_unique()).collect();
        keys[2] = Pubkey::find_program_address(&[b"escrow-vault", keys[1].as_ref()], &program).0;
        keys[11] = mollusk_svm_programs_token::token::ID;
        keys[12] = Pubkey::default();
        let mut accounts: Vec<_> = keys.iter().map(|key| (*key, Account::default())).collect();
        accounts[0].1.lamports = 1_000_000_000;
        accounts[8].1.lamports = 10_000_000;
        accounts[13].1.lamports = 10_000_000;
        for i in [4, 5] {
            let mut a = Account::new(svm.sysvars.rent.minimum_balance(82), 82, &keys[11]);
            a.data[36..44].copy_from_slice(&1_000_000u64.to_le_bytes());
            a.data[44] = 6;
            a.data[45] = 1;
            accounts[i].1 = a;
        }
        for (i, mint, owner, n) in [
            (6, 4, 0, 5000u64),
            (7, 5, 0, 0),
            (9, 4, 8, 0),
            (10, 5, 8, 5000),
        ] {
            let mut a = Account::new(svm.sysvars.rent.minimum_balance(165), 165, &keys[11]);
            a.data[..32].copy_from_slice(keys[mint].as_ref());
            a.data[32..64].copy_from_slice(keys[owner].as_ref());
            a.data[108] = 1;
            set_amount(&mut a, n);
            accounts[i].1 = a;
        }
        accounts[11].1 = mollusk_svm_programs_token::token::account();
        accounts[12] = mollusk_svm::program::keyed_account_for_system_program();
        Self {
            svm,
            program,
            accounts,
        }
    }
    fn ix(&self, op: u8) -> Instruction {
        let list: &[(usize, bool, bool)] = match op {
            0 => &[
                (0, true, true),
                (1, true, true),
                (2, false, false),
                (3, true, true),
                (4, false, false),
                (5, false, false),
                (6, true, false),
                (7, false, false),
                (11, false, false),
                (12, false, false),
            ],
            1 => &[
                (8, false, true),
                (1, true, false),
                (0, true, false),
                (2, false, false),
                (3, true, false),
                (4, false, false),
                (5, false, false),
                (9, true, false),
                (10, true, false),
                (7, true, false),
                (6, true, false),
                (11, false, false),
            ],
            2 => &[
                (0, true, true),
                (1, true, false),
                (2, false, false),
                (3, true, false),
                (4, false, false),
                (6, true, false),
                (11, false, false),
            ],
            _ => unreachable!(),
        };
        let metas = list
            .iter()
            .map(|&(i, w, s)| {
                if w {
                    AccountMeta::new(self.accounts[i].0, s)
                } else {
                    AccountMeta::new_readonly(self.accounts[i].0, s)
                }
            })
            .collect();
        let mut data = vec![op];
        if op < 2 {
            data.extend_from_slice(&1000u64.to_le_bytes());
            data.extend_from_slice(&2000u64.to_le_bytes());
        }
        Instruction::new_with_bytes(self.program, &data, metas)
    }
    fn make(&self) -> Vec<(Pubkey, Account)> {
        let r = self.svm.process_instruction(&self.ix(0), &self.accounts);
        assert_eq!(r.raw_result, Ok(()));
        let mut expected = self.accounts.clone();
        let rent = self.svm.sysvars.rent.minimum_balance(state::Escrow::LEN);
        let token_rent = self.svm.sysvars.rent.minimum_balance(165);
        expected[0].1.lamports -= rent + token_rent;
        expected[1].1 = Account::new(rent, state::Escrow::LEN, &self.program);
        state::Escrow::write_init_header(&mut expected[1].1.data).unwrap();
        for (offset, index) in [(16, 0), (48, 7), (80, 4), (112, 5), (144, 3)] {
            expected[1].1.data[offset..offset + 32]
                .copy_from_slice(self.accounts[index].0.as_ref());
        }
        expected[1].1.data[176..184].copy_from_slice(&1000u64.to_le_bytes());
        expected[1].1.data[184..192].copy_from_slice(&2000u64.to_le_bytes());
        expected[3].1 = Account::new(token_rent, 165, &self.accounts[11].0);
        expected[3].1.data[..32].copy_from_slice(self.accounts[4].0.as_ref());
        expected[3].1.data[32..64].copy_from_slice(self.accounts[2].0.as_ref());
        expected[3].1.data[108] = 1;
        set_amount(&mut expected[3].1, 1000);
        set_amount(&mut expected[6].1, 4000);
        assert_eq!(r.resulting_accounts, expected, "make exact account state");
        println!(
            "escrow make CU={}, header={}",
            r.compute_units_consumed,
            expected[1].1.data[..16]
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        );
        expected
    }
    fn reject(&self, label: &str, ix: &Instruction, accounts: &[(Pubkey, Account)]) {
        let r = self.svm.process_instruction(ix, accounts);
        assert!(r.raw_result.is_err(), "{label}");
        assert_eq!(r.resulting_accounts, accounts, "{label}: complete rollback");
        println!(
            "{label}: {:?}, CU={}",
            r.raw_result, r.compute_units_consumed
        );
    }
}
#[test]
#[ignore = "requires HOPPER_ESCROW_SBF"]
fn funded_make_take_cancel_and_donations() {
    let h = Harness::new();
    for op in [1, 2] {
        for extra in [0, 37] {
            let mut accounts = h.make();
            set_amount(&mut accounts[3].1, 1000 + extra);
            let r = h.svm.process_instruction(&h.ix(op), &accounts);
            assert_eq!(r.raw_result, Ok(()), "op={op}");
            let mut expected = accounts.clone();
            expected[0].1.lamports += accounts[1].1.lamports + accounts[3].1.lamports;
            // Mollusk preserves the in-instruction sentinel state; a committed
            // bank removes the zero-lamport state account after the transaction.
            expected[1].1.lamports = 0;
            expected[1].1.data.fill(0);
            expected[1].1.data[0] = 255;
            expected[3].1 = Account::default();
            if op == 1 {
                set_amount(&mut expected[10].1, 3000);
                set_amount(&mut expected[7].1, 2000);
                set_amount(&mut expected[9].1, 1000);
                set_amount(&mut expected[6].1, 4000 + extra);
            } else {
                set_amount(&mut expected[6].1, 5000 + extra);
            }
            assert_eq!(r.resulting_accounts, expected, "op={op}, donation={extra}");
            assert_eq!(
                amount(&r.resulting_accounts[9].1),
                if op == 1 { 1000 } else { 0 }
            );
            h.reject("repeated take", &h.ix(1), &r.resulting_accounts);
            h.reject("repeated cancel", &h.ix(2), &r.resulting_accounts);
            println!(
                "escrow op={op}, donation={extra}, CU={}",
                r.compute_units_consumed
            );
        }
    }
}
#[test]
#[ignore = "requires HOPPER_ESCROW_SBF"]
fn adversarial_accounts_and_cpi_failures_rollback() {
    let h = Harness::new();
    let funded = h.make();
    h.reject("reinitialize", &h.ix(0), &funded);
    for op in [0, 1, 2] {
        let accounts = if op == 0 { &h.accounts } else { &funded };
        let ix = h.ix(op);
        for slot in 0..ix.accounts.len() {
            if ix.accounts[slot].is_signer {
                let mut bad = ix.clone();
                bad.accounts[slot].is_signer = false;
                h.reject("missing signer", &bad, accounts);
            }
            if ix.accounts[slot].is_writable {
                let mut bad = ix.clone();
                bad.accounts[slot].is_writable = false;
                h.reject("missing writable", &bad, accounts);
            }
        }
        let mut bad = ix.clone();
        bad.data.push(0);
        h.reject("trailing payload", &bad, accounts);
        let mut bad = ix.clone();
        bad.data.pop();
        h.reject("short payload", &bad, accounts);
        let mut bad = ix.clone();
        let token_slot = if op == 0 { 8 } else { ix.accounts.len() - 1 };
        bad.accounts[token_slot].pubkey = h.accounts[12].0;
        h.reject("wrong token program", &bad, accounts);
        let mut bad = ix.clone();
        let authority_slot = if op == 1 { 3 } else { 2 };
        bad.accounts[authority_slot].pubkey = h.accounts[13].0;
        h.reject("wrong vault PDA", &bad, accounts);
    }
    for (label, index, offset) in [
        ("source mint", 6, 0),
        ("source owner", 6, 32),
        ("maker payment owner", 7, 32),
    ] {
        let mut bad = h.accounts.clone();
        bad[index].1.data[offset] ^= 1;
        h.reject(label, &h.ix(0), &bad);
    }
    let mut poor = h.accounts.clone();
    set_amount(&mut poor[6].1, 999);
    h.reject(
        "make funding fails after state and vault creation",
        &h.ix(0),
        &poor,
    );
    let mut bad = h.ix(0);
    bad.data[1..9].fill(0);
    h.reject("zero offer", &bad, &h.accounts);
    let mut bad = h.accounts.clone();
    bad[4].1.data[46] = 1;
    h.reject("freeze authority mint", &h.ix(0), &bad);
    let mut bad = h.accounts.clone();
    bad[4].1.owner = Pubkey::new_unique();
    h.reject("foreign mint owner", &h.ix(0), &bad);
    for index in [3, 6, 7, 9, 10] {
        for offset in [0, 32, 108, 109] {
            let mut bad = funded.clone();
            bad[index].1.data[offset] ^= 1;
            h.reject("token policy substitution", &h.ix(1), &bad);
        }
    }
    for offset in [72, 129] {
        let mut bad = funded.clone();
        bad[3].1.data[offset] = 1;
        h.reject("vault delegate or close authority", &h.ix(1), &bad);
        h.reject("cancel vault authority substitution", &h.ix(2), &bad);
    }
    for offset in [16, 48, 80, 112, 144] {
        let mut bad = funded.clone();
        bad[1].1.data[offset] ^= 1;
        h.reject("stored binding substitution", &h.ix(1), &bad);
    }
    let mut bad = h.ix(1);
    bad.data[9..17].copy_from_slice(&1u64.to_le_bytes());
    h.reject("stale quote", &bad, &funded);
    let mut bad = h.ix(1);
    bad.accounts[7].pubkey = bad.accounts[10].pubkey;
    h.reject("aliased recipient", &bad, &funded);
    let mut bad = h.ix(2);
    bad.accounts[0].pubkey = h.accounts[13].0;
    h.reject("unauthorized cancel", &bad, &funded);
    let mut bad = funded.clone();
    set_amount(&mut bad[10].1, 1999);
    h.reject("insufficient taker funds", &h.ix(1), &bad);
    // Synthetic lamport fault: the token vault can close, but returning the
    // state rent then overflows. Prove rollback after BOTH exchange CPIs and
    // the token close have succeeded; no impossible token-supply fixture.
    let mut bad = funded.clone();
    bad[0].1.lamports = u64::MAX - bad[3].1.lamports;
    h.reject(
        "state-close overflow after exchange and vault-close CPIs",
        &h.ix(1),
        &bad,
    );
    h.reject(
        "state-close overflow after refund and vault-close CPIs",
        &h.ix(2),
        &bad,
    );
}
