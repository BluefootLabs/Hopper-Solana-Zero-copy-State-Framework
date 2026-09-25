//! Exercise the real segmented order-storage ELF, including initialization CU.
use hopper::hopper_core::account::registry::segment_id;
use mollusk_svm::{program::loader_keys::LOADER_V3, Mollusk};
use solana_account::Account;
use solana_instruction::{AccountMeta, Instruction};
use solana_instruction_error::InstructionError;
use solana_pubkey::Pubkey;

const LEN: usize = 139_356;
const BIDS: usize = 68;
const ASKS: usize = 57_420;
const EVENTS: usize = 114_772;
const BOOK: usize = 2;

struct Harness {
    svm: Mollusk,
    program: Pubkey,
    payer: Pubkey,
    maker: Pubkey,
    book: Pubkey,
    stranger: Pubkey,
    initial: Vec<(Pubkey, Account)>,
}

impl Harness {
    fn new() -> Self {
        let elf = std::fs::read(std::env::var("HOPPER_ORDERBOOK_SBF").unwrap()).unwrap();
        let program = Pubkey::new_unique();
        let payer = Pubkey::new_unique();
        let maker = Pubkey::new_unique();
        let book = Pubkey::new_unique();
        let stranger = Pubkey::new_unique();
        let mut svm = Mollusk::default();
        // Measure the entire scan even if it exceeds the ordinary 200k
        // instruction budget; the initialization test separately enforces it.
        svm.compute_budget.compute_unit_limit = 1_400_000;
        svm.add_program_with_loader_and_elf(&program, &LOADER_V3, &elf);
        let initial = vec![
            (payer, Account::new(1_000_000_000, 0, &Pubkey::default())),
            (maker, Account::new(1_000_000, 0, &Pubkey::default())),
            (
                book,
                Account::new(svm.sysvars.rent.minimum_balance(LEN), LEN, &program),
            ),
            (stranger, Account::new(1_000_000, 0, &Pubkey::default())),
            mollusk_svm::program::keyed_account_for_system_program(),
        ];
        Self {
            svm,
            program,
            payer,
            maker,
            book,
            stranger,
            initial,
        }
    }

    fn init_ix(&self) -> Instruction {
        Instruction::new_with_bytes(
            self.program,
            &[0],
            vec![
                AccountMeta::new_readonly(self.payer, true),
                AccountMeta::new(self.book, true),
                AccountMeta::new_readonly(Pubkey::default(), false),
            ],
        )
    }

    fn initialized(&self) -> Vec<(Pubkey, Account)> {
        let mut expected = self.initial.clone();
        let data = &mut expected[BOOK].1.data;
        data[0] = 30;
        data[1] = 1;
        data[4..12].copy_from_slice(b"OBDEMO01");
        data[12..16].copy_from_slice(&1u32.to_le_bytes());
        data[16..18].copy_from_slice(&3u16.to_le_bytes());
        for (i, (name, offset, size)) in [
            ("bids", BIDS, 57_352usize),
            ("asks", ASKS, 57_352usize),
            ("events", EVENTS, 24_584usize),
        ]
        .iter()
        .enumerate()
        {
            let start = 20 + i * 16;
            data[start..start + 4].copy_from_slice(&segment_id(name));
            data[start + 4..start + 8].copy_from_slice(&(*offset as u32).to_le_bytes());
            data[start + 8..start + 12].copy_from_slice(&(*size as u32).to_le_bytes());
            data[start + 14] = 1;
        }
        let result = self.svm.process_instruction(&self.init_ix(), &self.initial);
        println!(
            "orderbook initialize: {:?}, {} CU",
            result.raw_result, result.compute_units_consumed
        );
        assert_eq!(result.raw_result, Ok(()));
        assert_eq!(
            result.resulting_accounts, expected,
            "initialize changed unexpected state"
        );
        assert!(
            result.compute_units_consumed <= 200_000,
            "initialization exceeds the default 200,000 CU instruction budget"
        );
        result.resulting_accounts
    }

    fn ix(&self, tag: u8, signer: Pubkey) -> Instruction {
        let mut data = vec![tag];
        if tag == 1 || tag == 2 {
            data.extend_from_slice(&100u64.to_le_bytes());
            data.extend_from_slice(&5u64.to_le_bytes());
            data.extend_from_slice(&7u64.to_le_bytes());
        }
        let mut metas = vec![];
        if tag != 4 {
            metas.push(AccountMeta::new_readonly(signer, true));
        }
        metas.push(AccountMeta::new(self.book, false));
        Instruction::new_with_bytes(self.program, &data, metas)
    }

    fn rejected(
        &self,
        name: &str,
        ix: &Instruction,
        accounts: &[(Pubkey, Account)],
        error: Option<InstructionError>,
    ) {
        let result = self.svm.process_instruction(ix, accounts);
        if let Some(error) = error {
            assert_eq!(result.raw_result, Err(error), "{name}");
        } else {
            assert!(result.raw_result.is_err(), "{name}");
        }
        assert_eq!(
            result.resulting_accounts, accounts,
            "{name}: changed account state"
        );
        println!(
            "{name}: {:?}, {} CU, full rollback",
            result.raw_result, result.compute_units_consumed
        );
    }
}

#[test]
#[ignore = "requires HOPPER_ORDERBOOK_SBF"]
fn orderbook_initialization_and_lifecycle_change_only_expected_bytes() {
    let h = Harness::new();
    let mut accounts = h.initialized();
    for (tag, name) in [
        (1, "post bid"),
        (2, "post ask"),
        (3, "record ask event"),
        (4, "drain event"),
    ] {
        let mut expected = accounts.clone();
        let data = &mut expected[BOOK].1.data;
        match tag {
            1 | 2 => {
                let start = if tag == 1 { BIDS } else { ASKS };
                data[start..start + 4].copy_from_slice(&1u32.to_le_bytes());
                data[start + 8..start + 40].copy_from_slice(h.maker.as_ref());
                data[start + 40..start + 48].copy_from_slice(&100u64.to_le_bytes());
                data[start + 48..start + 56].copy_from_slice(&5u64.to_le_bytes());
                data[start + 56..start + 64].copy_from_slice(&7u64.to_le_bytes());
            }
            3 => {
                h.rejected(
                    "stranger cannot remove ask",
                    &h.ix(3, h.stranger),
                    &accounts,
                    Some(InstructionError::Custom(7300)),
                );
                data[ASKS..ASKS + 4].fill(0);
                data[EVENTS..EVENTS + 4].copy_from_slice(&1u32.to_le_bytes());
                data[EVENTS + 8..EVENTS + 40].copy_from_slice(h.maker.as_ref());
                data[EVENTS + 40..EVENTS + 48].copy_from_slice(&100u64.to_le_bytes());
                data[EVENTS + 48..EVENTS + 56].copy_from_slice(&5u64.to_le_bytes());
            }
            4 => data[EVENTS + 4..EVENTS + 8].copy_from_slice(&1u32.to_le_bytes()),
            _ => unreachable!(),
        }
        let result = h.svm.process_instruction(&h.ix(tag, h.maker), &accounts);
        assert_eq!(result.raw_result, Ok(()), "{name}");
        assert_eq!(
            result.resulting_accounts, expected,
            "{name}: unexpected mutation"
        );
        println!(
            "{name}: {} CU, exact account state",
            result.compute_units_consumed
        );
        accounts = result.resulting_accounts;
    }
    h.rejected(
        "empty drain",
        &h.ix(4, h.maker),
        &accounts,
        Some(InstructionError::Custom(7305)),
    );
    h.rejected(
        "reinitialize",
        &h.init_ix(),
        &accounts,
        Some(InstructionError::AccountAlreadyInitialized),
    );
}

#[test]
#[ignore = "requires HOPPER_ORDERBOOK_SBF"]
fn orderbook_initialization_and_mutation_refusals_preserve_state() {
    let h = Harness::new();
    let mut ix = h.init_ix();
    ix.accounts[1].is_signer = false;
    h.rejected(
        "unsigned book init",
        &ix,
        &h.initial,
        Some(InstructionError::MissingRequiredSignature),
    );
    let mut dirty = h.initial.clone();
    dirty[BOOK].1.data[LEN - 1] = 1;
    h.rejected(
        "dirty trailing byte init",
        &h.init_ix(),
        &dirty,
        Some(InstructionError::AccountAlreadyInitialized),
    );
    let accounts = h.initialized();
    for name in [
        "missing maker signature",
        "readonly book",
        "foreign book owner",
        "bad layout",
        "overlapping registry",
        "excessive count",
        "full side",
    ] {
        let mut altered = accounts.clone();
        let mut ix = h.ix(1, h.maker);
        match name {
            "missing maker signature" => ix.accounts[0].is_signer = false,
            "readonly book" => ix.accounts[1].is_writable = false,
            "foreign book owner" => altered[BOOK].1.owner = Pubkey::new_unique(),
            "bad layout" => altered[BOOK].1.data[4] ^= 255,
            "overlapping registry" => {
                altered[BOOK].1.data[40..44].copy_from_slice(&(BIDS as u32).to_le_bytes())
            }
            "excessive count" => {
                altered[BOOK].1.data[BIDS..BIDS + 4].copy_from_slice(&1025u32.to_le_bytes())
            }
            "full side" => {
                altered[BOOK].1.data[BIDS..BIDS + 4].copy_from_slice(&1024u32.to_le_bytes())
            }
            _ => unreachable!(),
        }
        h.rejected(name, &ix, &altered, None);
    }
    let result = h.svm.process_instruction(&h.ix(2, h.maker), &accounts);
    assert_eq!(result.raw_result, Ok(()));
    let mut full = result.resulting_accounts;
    full[BOOK].1.data[EVENTS..EVENTS + 4].copy_from_slice(&512u32.to_le_bytes());
    h.rejected(
        "full event ring preserves ask",
        &h.ix(3, h.maker),
        &full,
        Some(InstructionError::Custom(7307)),
    );
}
