//! Execute the actual allowance program; compare every account byte and lamport.
use mollusk_svm::{program::loader_keys::LOADER_V3, Mollusk};
use solana_account::Account;
use solana_instruction::{AccountMeta, Instruction};
use solana_instruction_error::InstructionError;
use solana_pubkey::Pubkey;

#[allow(dead_code)]
#[path = "../../../../examples/hopper-byte-allowance/src/lib.rs"]
mod allowance_program;

const AUTHORITY: usize = 16;
const DELEGATES: usize = AUTHORITY + 32;
const LIMITS: usize = DELEGATES + 4 * 32;
const SPENT: usize = LIMITS + 4 * 8;
const REVISIONS: usize = SPENT + 4 * 8;
const LEN: usize = REVISIONS + 4 * 8;

fn expected_header() -> [u8; 16] {
    assert_eq!(allowance_program::AllowanceBook::LEN, LEN);
    let mut header = [0; 16];
    hopper::hopper_runtime::layout::init_header::<allowance_program::AllowanceBook>(&mut header)
        .unwrap();
    header
}

struct Harness {
    svm: Mollusk,
    program: Pubkey,
    authority: Pubkey,
    book: Pubkey,
    delegates: [Pubkey; 4],
    init: Instruction,
    initial: Vec<(Pubkey, Account)>,
}

impl Harness {
    fn new() -> Self {
        let elf = std::fs::read(std::env::var("HOPPER_BYTE_ALLOWANCE_SBF").unwrap()).unwrap();
        let program = Pubkey::new_unique();
        let authority = Pubkey::new_unique();
        let book = Pubkey::new_unique();
        let delegates = core::array::from_fn(|_| Pubkey::new_unique());
        let mut svm = Mollusk::default();
        svm.add_program_with_loader_and_elf(&program, &LOADER_V3, &elf);
        let mut data = vec![0];
        for delegate in delegates {
            data.extend_from_slice(delegate.as_ref());
        }
        data.extend_from_slice(&100u64.to_le_bytes());
        let init = Instruction::new_with_bytes(
            program,
            &data,
            vec![
                AccountMeta::new(authority, true),
                AccountMeta::new(book, true),
                AccountMeta::new_readonly(Pubkey::default(), false),
            ],
        );
        let initial = vec![
            (
                authority,
                Account::new(1_000_000_000, 0, &Pubkey::default()),
            ),
            (book, Account::default()),
            mollusk_svm::program::keyed_account_for_system_program(),
        ];
        Self {
            svm,
            program,
            authority,
            book,
            delegates,
            init,
            initial,
        }
    }

    fn initialized(&self) -> Vec<(Pubkey, Account)> {
        let header = expected_header();
        println!(
            "ALLOWANCE_HEADER_HEX={}",
            header
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        );
        let mut expected = self.initial.clone();
        let rent = self.svm.sysvars.rent.minimum_balance(LEN);
        expected[0].1.lamports -= rent;
        let mut book = Account::new(rent, LEN, &self.program);
        book.data[..16].copy_from_slice(&header);
        book.data[AUTHORITY..DELEGATES].copy_from_slice(self.authority.as_ref());
        for (slot, delegate) in self.delegates.iter().enumerate() {
            book.data[DELEGATES + slot * 32..DELEGATES + (slot + 1) * 32]
                .copy_from_slice(delegate.as_ref());
            book.data[LIMITS + slot * 8..LIMITS + (slot + 1) * 8]
                .copy_from_slice(&100u64.to_le_bytes());
        }
        expected[1].1 = book;
        let result = self.svm.process_instruction(&self.init, &self.initial);
        assert_eq!(result.raw_result, Ok(()));
        assert_eq!(
            result.resulting_accounts, expected,
            "initialize: unexpected account changes"
        );
        let mut accounts = result.resulting_accounts;
        accounts.extend(
            self.delegates
                .iter()
                .map(|key| (*key, Account::new(1_000_000, 0, &Pubkey::default()))),
        );
        accounts
    }

    fn ix(&self, op: u8, slot: u16, revision: u64, amount: u64, signer: Pubkey) -> Instruction {
        let mut data = vec![op];
        data.extend_from_slice(&slot.to_le_bytes());
        data.extend_from_slice(&revision.to_le_bytes());
        data.extend_from_slice(&amount.to_le_bytes());
        Instruction::new_with_bytes(
            self.program,
            &data,
            vec![
                AccountMeta::new_readonly(signer, true),
                AccountMeta::new(self.book, false),
            ],
        )
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
            "{name}: failure changed accounts"
        );
        println!(
            "{name}: {:?}, {} CU, complete rollback",
            result.raw_result, result.compute_units_consumed
        );
    }
}

#[test]
#[ignore = "requires HOPPER_BYTE_ALLOWANCE_SBF"]
fn lifecycle_changes_only_selected_cells() {
    let h = Harness::new();
    let mut accounts = h.initialized();
    let book = &accounts[1].1;
    assert_eq!(book.data.len(), LEN);
    assert_eq!(book.owner, h.program);
    assert_eq!(book.lamports, h.svm.sysvars.rent.minimum_balance(LEN));
    assert_eq!(&book.data[AUTHORITY..DELEGATES], h.authority.as_ref());
    for slot in 0..4 {
        assert_eq!(
            &book.data[DELEGATES + slot * 32..DELEGATES + (slot + 1) * 32],
            h.delegates[slot].as_ref()
        );
        assert_eq!(
            &book.data[LIMITS + slot * 8..LIMITS + (slot + 1) * 8],
            &100u64.to_le_bytes()
        );
    }
    assert!(book.data[SPENT..].iter().all(|v| *v == 0));
    for slot in 0..4u16 {
        for (op, revision, amount, column, value, signer) in [
            (1, 0, 40, SPENT, 40, h.delegates[slot as usize]),
            (2, 1, 80, LIMITS, 80, h.authority),
            (1, 2, 40, SPENT, 80, h.delegates[slot as usize]),
        ] {
            let mut expected = accounts.clone();
            let offset = column + usize::from(slot) * 8;
            expected[1].1.data[offset..offset + 8].copy_from_slice(&u64::to_le_bytes(value));
            let offset = REVISIONS + usize::from(slot) * 8;
            expected[1].1.data[offset..offset + 8]
                .copy_from_slice(&(revision + 1u64).to_le_bytes());
            let result = h
                .svm
                .process_instruction(&h.ix(op, slot, revision, amount, signer), &accounts);
            assert_eq!(result.raw_result, Ok(()), "op={op}, slot={slot}");
            assert_eq!(
                result.resulting_accounts, expected,
                "unexpected mutation op={op}, slot={slot}"
            );
            println!(
                "op={op}, slot={slot}, {} CU, exact two-cell write",
                result.compute_units_consumed
            );
            accounts = result.resulting_accounts;
        }
    }
    h.rejected("reinitialize", &h.init, &accounts, None);
}

#[test]
#[ignore = "requires HOPPER_BYTE_ALLOWANCE_SBF"]
fn adversarial_inputs_preserve_all_account_state() {
    let h = Harness::new();
    let accounts = h.initialized();
    for (name, op, slot, rev, amount, signer, error) in [
        (
            "wrong delegate",
            1,
            0,
            0,
            1,
            h.delegates[1],
            Some(InstructionError::Custom(7800)),
        ),
        (
            "stale revision",
            1,
            0,
            1,
            1,
            h.delegates[0],
            Some(InstructionError::Custom(7801)),
        ),
        (
            "limit exceeded",
            1,
            0,
            0,
            101,
            h.delegates[0],
            Some(InstructionError::Custom(7802)),
        ),
        (
            "zero amount",
            1,
            0,
            0,
            0,
            h.delegates[0],
            Some(InstructionError::Custom(7803)),
        ),
        (
            "out of bounds",
            1,
            4,
            0,
            1,
            h.delegates[0],
            Some(InstructionError::InvalidInstructionData),
        ),
        (
            "max selector",
            1,
            u16::MAX,
            0,
            1,
            h.delegates[0],
            Some(InstructionError::InvalidInstructionData),
        ),
        ("wrong admin", 2, 0, 0, 200, h.delegates[0], None),
        (
            "stale admin revision",
            2,
            0,
            1,
            200,
            h.authority,
            Some(InstructionError::Custom(7801)),
        ),
    ] {
        h.rejected(name, &h.ix(op, slot, rev, amount, signer), &accounts, error);
    }
    let valid = h.ix(1, 0, 0, 1, h.delegates[0]);
    let mut ix = valid.clone();
    ix.accounts[0].is_signer = false;
    h.rejected(
        "missing signer",
        &ix,
        &accounts,
        Some(InstructionError::MissingRequiredSignature),
    );
    let mut ix = valid.clone();
    ix.accounts[1].is_writable = false;
    h.rejected("readonly book", &ix, &accounts, None);
    let mut ix = valid.clone();
    ix.data.pop();
    h.rejected(
        "truncated arguments",
        &ix,
        &accounts,
        Some(InstructionError::InvalidInstructionData),
    );
    let mut ix = valid.clone();
    ix.data.push(0);
    h.rejected(
        "trailing arguments",
        &ix,
        &accounts,
        Some(InstructionError::InvalidInstructionData),
    );
    let mut ix = valid.clone();
    ix.accounts
        .push(AccountMeta::new_readonly(h.authority, false));
    let baseline = h.svm.process_instruction(&valid, &accounts);
    assert_eq!(baseline.raw_result, Ok(()));
    let with_extra = h.svm.process_instruction(&ix, &accounts);
    assert_eq!(with_extra.raw_result, Ok(()));
    assert_eq!(
        with_extra.resulting_accounts, baseline.resulting_accounts,
        "ignored surplus account changed consume behavior"
    );
    println!(
        "surplus consume account ignored: exact expected state, {} CU",
        with_extra.compute_units_consumed
    );
    let mut ix = valid.clone();
    ix.accounts[0].pubkey = h.book;
    h.rejected("delegate aliases book", &ix, &accounts, None);
    let mut ix = h.ix(2, 0, 0, 100, h.authority);
    ix.accounts[0].is_signer = false;
    h.rejected(
        "unsigned administrator",
        &ix,
        &accounts,
        Some(InstructionError::MissingRequiredSignature),
    );
    for name in [
        "foreign owner",
        "bad discriminator",
        "bad layout id",
        "bad version",
        "bad schema epoch",
        "short layout",
        "spent overflow",
        "revision overflow",
        "limit below spent",
    ] {
        let mut altered = accounts.clone();
        let book = &mut altered[1].1;
        let mut ix = valid.clone();
        match name {
            "foreign owner" => book.owner = Pubkey::new_unique(),
            "bad discriminator" => book.data[0] ^= 255,
            "bad layout id" => book.data[4] ^= 255,
            "bad version" => book.data[1] = 0,
            "bad schema epoch" => book.data[12..16].copy_from_slice(&u32::MAX.to_le_bytes()),
            "short layout" => {
                book.data.pop();
            }
            "spent overflow" => {
                book.data[SPENT..SPENT + 8].copy_from_slice(&u64::MAX.to_le_bytes())
            }
            "revision overflow" => {
                book.data[REVISIONS..REVISIONS + 8].copy_from_slice(&u64::MAX.to_le_bytes());
                ix = h.ix(1, 0, u64::MAX, 1, h.delegates[0]);
            }
            "limit below spent" => {
                book.data[SPENT..SPENT + 8].copy_from_slice(&10u64.to_le_bytes());
                ix = h.ix(2, 0, 0, 9, h.authority);
            }
            _ => unreachable!(),
        }
        h.rejected(name, &ix, &altered, None);
    }
}

#[test]
#[ignore = "requires HOPPER_BYTE_ALLOWANCE_SBF"]
fn initialization_refusals_preserve_all_accounts() {
    let h = Harness::new();
    for name in [
        "unsigned payer",
        "readonly payer",
        "unsigned book",
        "readonly book",
        "wrong system program",
        "payer aliases book",
        "truncated init arguments",
        "trailing init arguments",
        "foreign book owner",
        "preallocated book data",
        "insufficient rent payer",
    ] {
        let mut ix = h.init.clone();
        let mut accounts = h.initial.clone();
        match name {
            "unsigned payer" => ix.accounts[0].is_signer = false,
            "readonly payer" => ix.accounts[0].is_writable = false,
            "unsigned book" => ix.accounts[1].is_signer = false,
            "readonly book" => ix.accounts[1].is_writable = false,
            "wrong system program" => ix.accounts[2].pubkey = h.authority,
            "payer aliases book" => ix.accounts[1].pubkey = h.authority,
            "truncated init arguments" => {
                ix.data.pop();
            }
            "trailing init arguments" => ix.data.push(0),
            "foreign book owner" => accounts[1].1.owner = Pubkey::new_unique(),
            "preallocated book data" => accounts[1].1.data = vec![0; LEN],
            "insufficient rent payer" => accounts[0].1.lamports = 1,
            _ => unreachable!(),
        }
        h.rejected(name, &ix, &accounts, None);
    }
    let extra = Pubkey::new_unique();
    let mut accounts = h.initial.clone();
    accounts.push((extra, Account::new(1_000_000, 0, &Pubkey::default())));
    let baseline = h.svm.process_instruction(&h.init, &accounts);
    assert_eq!(baseline.raw_result, Ok(()));
    let mut ix = h.init.clone();
    ix.accounts.push(AccountMeta::new_readonly(extra, false));
    let with_extra = h.svm.process_instruction(&ix, &accounts);
    assert_eq!(with_extra.raw_result, Ok(()));
    assert_eq!(
        with_extra.resulting_accounts, baseline.resulting_accounts,
        "ignored surplus account changed initialization behavior"
    );
    assert_eq!(with_extra.resulting_accounts.last(), accounts.last());
    println!(
        "surplus initialization account ignored: exact expected state, {} CU",
        with_extra.compute_units_consumed
    );
}
