//! Execute Hopper's mint plan against canonical SPL Token and Token-2022 ELFs.
use mollusk_svm::{program::loader_keys::LOADER_V3, Mollusk};
use solana_account::Account;
use solana_instruction::{AccountMeta, Instruction};
use solana_pubkey::Pubkey;
use spl_token_2022_interface::{
    extension::{BaseStateWithExtensions, ExtensionType, StateWithExtensions},
    state::Mint,
};

fn harness() -> (Mollusk, Pubkey) {
    let elf =
        std::fs::read(std::env::var("HOPPER_MINT_PLAN_SBF").expect("set HOPPER_MINT_PLAN_SBF"))
            .unwrap();
    let program = Pubkey::new_unique();
    let mut svm = Mollusk::default();
    svm.add_program_with_loader_and_elf(&program, &LOADER_V3, &elf);
    mollusk_svm_programs_token::token::add_program(&mut svm);
    mollusk_svm_programs_token::token2022::add_program(&mut svm);
    (svm, program)
}

fn extensions(mask: u8) -> Vec<ExtensionType> {
    [
        ExtensionType::TransferFeeConfig,
        ExtensionType::MintCloseAuthority,
        ExtensionType::NonTransferable,
        ExtensionType::PermanentDelegate,
        ExtensionType::TransferHook,
        ExtensionType::MetadataPointer,
    ]
    .into_iter()
    .enumerate()
    .filter(|(i, _)| mask & (1 << i) != 0)
    .map(|(_, e)| e)
    .collect()
}

fn setup(
    program: Pubkey,
    legacy: bool,
    mask: u8,
    operation: u8,
    mint: Pubkey,
    mint_account: Account,
) -> (Instruction, Vec<(Pubkey, Account)>) {
    let payer = Pubkey::new_from_array([31; 32]);
    let token = if legacy {
        mollusk_svm_programs_token::token::ID
    } else {
        mollusk_svm_programs_token::token2022::ID
    };
    let token_account = if legacy {
        mollusk_svm_programs_token::token::account()
    } else {
        mollusk_svm_programs_token::token2022::account()
    };
    let system = Pubkey::default();
    let instruction = Instruction::new_with_bytes(
        program,
        &[u8::from(!legacy), mask, operation, 0, 0],
        vec![
            AccountMeta::new(payer, true),
            AccountMeta::new(mint, operation == 0),
            AccountMeta::new_readonly(token, false),
            AccountMeta::new_readonly(system, false),
        ],
    );
    let accounts = vec![
        (payer, Account::new(1_000_000_000, 0, &system)),
        (mint, mint_account),
        (token, token_account),
        mollusk_svm::program::keyed_account_for_system_program(),
    ];
    (instruction, accounts)
}

#[test]
#[ignore = "requires compiled HOPPER_MINT_PLAN_SBF fixture"]
fn all_extension_subsets_initialize_with_exact_canonical_state_and_rent() {
    let (svm, program) = harness();
    for (legacy, mask) in [(true, 0)].into_iter().chain((0..64).map(|m| (false, m))) {
        let expected = extensions(mask);
        let size = ExtensionType::try_calculate_account_len::<Mint>(&expected).unwrap();
        let rent = svm.sysvars.rent.minimum_balance(size);
        let mint = Pubkey::new_unique();
        let (ix, accounts) = setup(program, legacy, mask, 0, mint, Account::default());
        let result = svm.process_instruction(&ix, &accounts);
        assert_eq!(
            result.raw_result,
            Ok(()),
            "legacy={legacy}, extensions={mask}"
        );
        let account = result.get_account(&mint).unwrap();
        assert_eq!(account.owner, ix.accounts[2].pubkey);
        assert_eq!(account.lamports, rent);
        assert_eq!(account.data.len(), size);
        let parsed = StateWithExtensions::<Mint>::unpack(&account.data).unwrap();
        assert!(parsed.base.is_initialized);
        assert_eq!(parsed.base.supply, 0);
        assert_eq!(parsed.base.decimals, 9);
        assert_eq!(parsed.base.mint_authority.unwrap(), accounts[0].0);
        assert_eq!(parsed.base.freeze_authority.unwrap(), accounts[0].0);
        assert_eq!(parsed.get_extension_types().unwrap(), expected);
        if mask & 1 != 0 {
            let fee = parsed.get_extension::<spl_token_2022_interface::extension::transfer_fee::TransferFeeConfig>().unwrap();
            assert_eq!(
                u16::from(fee.newer_transfer_fee.transfer_fee_basis_points),
                250
            );
            assert_eq!(u64::from(fee.newer_transfer_fee.maximum_fee), 1_000_000);
        }
        if mask & 32 != 0 {
            let pointer = parsed.get_extension::<spl_token_2022_interface::extension::metadata_pointer::MetadataPointer>().unwrap();
            assert_eq!(Option::<Pubkey>::from(pointer.metadata_address), Some(mint));
        }
        assert_eq!(
            result.get_account(&accounts[0].0).unwrap().lamports,
            accounts[0].1.lamports - rent
        );
        println!(
            "mint-plan legacy={legacy} extensions={mask} bytes={size} CU={}",
            result.compute_units_consumed
        );
    }
}

#[test]
#[ignore = "requires compiled HOPPER_MINT_PLAN_SBF fixture"]
fn prefunding_and_pda_signers_charge_only_the_live_rent_shortfall() {
    let (svm, program) = harness();
    let size = ExtensionType::try_calculate_account_len::<Mint>(&extensions(63)).unwrap();
    let rent = svm.sysvars.rent.minimum_balance(size);
    for prefund in [1, rent, rent + 123] {
        for pda in [false, true] {
            let payer = Pubkey::new_from_array([31; 32]);
            let (mint, bump) = if pda {
                Pubkey::find_program_address(&[b"mint", payer.as_ref()], &program)
            } else {
                (Pubkey::new_unique(), 0)
            };
            let (mut ix, accounts) = setup(
                program,
                false,
                63,
                if pda { 3 } else { 0 },
                mint,
                Account::new(prefund, 0, &Pubkey::default()),
            );
            ix.data[4] = bump;
            let result = svm.process_instruction(&ix, &accounts);
            assert_eq!(result.raw_result, Ok(()), "prefund={prefund}, pda={pda}");
            assert_eq!(
                result.get_account(&mint).unwrap().lamports,
                rent.max(prefund)
            );
            assert_eq!(
                result.get_account(&payer).unwrap().lamports,
                accounts[0].1.lamports - rent.saturating_sub(prefund)
            );
        }
    }
}

#[test]
#[ignore = "requires compiled HOPPER_MINT_PLAN_SBF fixture"]
fn invalid_creation_and_initialization_leave_accounts_unchanged() {
    let (svm, program) = harness();
    let token = mollusk_svm_programs_token::token2022::ID;
    let size = ExtensionType::try_calculate_account_len::<Mint>(&extensions(63)).unwrap();
    let rent = svm.sysvars.rent.minimum_balance(size + 1);
    for case in 0..13 {
        let mint = Pubkey::new_unique();
        let (mut ix, mut accounts) = setup(program, false, 63, 0, mint, Account::default());
        match case {
            0 => ix.data[3] = 255, // requested allocation one byte short
            1 => ix.data[3] = 1,   // arbitrary spare space is also invalid
            2 => ix.accounts[1].is_signer = false,
            3 => ix.accounts[1].is_writable = false,
            4 => accounts[1].1.owner = Pubkey::new_unique(),
            5 => accounts[0].1.lamports = 0,
            6 => {
                ix.data[2] = 1;
                accounts[1].1 = Account::new(rent, size - 1, &token);
            }
            7 => {
                ix.data[2] = 1;
                accounts[1].1 = Account::new(rent, size + 1, &token);
            }
            8 => {
                ix.data[2] = 1;
                accounts[1].1 = Account::new(1, size, &token);
            }
            9 => {
                ix.data[2] = 1;
                accounts[1].1 = Account::new(rent, size, &token);
                accounts[1].1.data[45] = 1;
            }
            10 => {
                ix.data[2] = 2;
                accounts[1].1 = Account::new(rent, size, &token);
            } // missing initialized extensions: canonical processor refuses
            11 => {
                ix.accounts[0].is_signer = false;
            }
            12 => {
                accounts[0].1.data = vec![0; 1];
            }
            _ => unreachable!(),
        }
        let result = svm.process_instruction(&ix, &accounts);
        assert!(
            result.raw_result.is_err(),
            "case {case} unexpectedly passed"
        );
        for (key, account) in &accounts {
            assert_eq!(
                result.get_account(key),
                Some(account),
                "case {case}, account {key}"
            );
        }
    }
    // Reusing a successfully initialized mint must not spend the payer's funds.
    let mint = Pubkey::new_unique();
    let (ix, accounts) = setup(program, false, 63, 0, mint, Account::default());
    let first = svm.process_instruction(&ix, &accounts);
    assert_eq!(first.raw_result, Ok(()));
    let second = svm.process_instruction(&ix, &first.resulting_accounts);
    assert!(second.raw_result.is_err());
    assert_eq!(second.resulting_accounts, first.resulting_accounts);
}
#[test]
#[ignore = "requires compiled HOPPER_MINT_PLAN_SBF fixture"]
fn extension_constraint_rejects_identical_bytes_under_a_forged_owner() {
    let (svm, program) = harness();
    let mint = Pubkey::new_unique();
    let (mut ix, accounts) = setup(program, false, 4, 0, mint, Account::default());
    let created = svm.process_instruction(&ix, &accounts);
    assert_eq!(created.raw_result, Ok(()));
    ix.data[2] = 4;
    ix.accounts[1].is_signer = false;
    ix.accounts[1].is_writable = false;
    let good = svm.process_instruction(&ix, &created.resulting_accounts);
    assert_eq!(good.raw_result, Ok(()));
    for forged_owner in [
        program,
        Pubkey::default(),
        mollusk_svm_programs_token::token::ID,
    ] {
        let mut forged = created.resulting_accounts.clone();
        forged
            .iter_mut()
            .find(|(key, _)| *key == mint)
            .unwrap()
            .1
            .owner = forged_owner;
        let rejected = svm.process_instruction(&ix, &forged);
        assert!(
            rejected.raw_result.is_err(),
            "extension bytes must not establish ownership"
        );
        assert_eq!(rejected.resulting_accounts, forged);
    }
}

#[test]
#[ignore = "requires compiled HOPPER_MINT_PLAN_SBF fixture"]
fn exhausted_budget_rolls_back_successful_creation_and_extension_cpis() {
    use solana_instruction_error::InstructionError;
    use solana_svm_log_collector::LogCollector;
    use std::{cell::RefCell, rc::Rc};

    let (mut svm, program) = harness();
    svm.compute_budget.compute_unit_limit = 10_000;
    let logger = Rc::new(RefCell::new(LogCollector::default()));
    svm.logger = Some(logger.clone());
    let mint = Pubkey::new_unique();
    let (ix, accounts) = setup(program, false, 63, 0, mint, Account::default());
    let result = svm.process_instruction(&ix, &accounts);
    assert_eq!(
        result.raw_result,
        Err(InstructionError::ComputationalBudgetExceeded)
    );
    let logs = logger.borrow();
    assert!(logs
        .get_recorded_content()
        .iter()
        .any(|line| { line == &format!("Program {} success", Pubkey::default()) }));
    assert!(logs.get_recorded_content().iter().any(|line| {
        line == &format!(
            "Program {} success",
            mollusk_svm_programs_token::token2022::ID
        )
    }));
    assert_eq!(result.resulting_accounts, accounts);
}
