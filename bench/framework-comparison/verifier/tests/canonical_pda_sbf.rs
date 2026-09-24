//! Both paths must accept the same canonical address and bump and reject every
//! other bump, including valid off-curve noncanonical addresses.
#[path = "../../../canonical-pda/program/src/config.rs"]
mod config;
use config::Config;
use mollusk_svm::{program::loader_keys::LOADER_V3, Mollusk};
use solana_account::Account;
use solana_instruction::{AccountMeta, Instruction};
use solana_instruction_error::InstructionError;
use solana_pubkey::Pubkey;
use std::str::FromStr;

#[test]
#[ignore = "requires HOPPER_CANONICAL_PDA_SBF pointing to the compiled fixture"]
fn canonical_constant_matches_runtime_search_and_rejections() {
    let elf = std::fs::read(std::env::var("HOPPER_CANONICAL_PDA_SBF").unwrap()).unwrap();
    let program_id = Pubkey::from_str("8RJxAyfAMnpb5ghwA4comPDJw6KqbDmZ28LDZHcccaVH").unwrap();
    let (canonical, bump) = Pubkey::find_program_address(&[b"config", b"v1"], &program_id);
    let mut svm = Mollusk::default();
    svm.add_program_with_loader_and_elf(&program_id, &LOADER_V3, &elf);
    let mut config_data = vec![0; Config::LEN];
    Config::write_init_header(&mut config_data).unwrap();
    *config_data.last_mut().unwrap() = 42;
    let exercise = |mode, address: Pubkey, supplied_bump, valid| {
        let snapshot = vec![(
            address,
            Account {
                lamports: 1_000_000,
                data: config_data.clone(),
                owner: program_id,
                executable: false,
                rent_epoch: 0,
            },
        )];
        let instruction = Instruction::new_with_bytes(
            program_id,
            &[mode, supplied_bump],
            vec![AccountMeta::new_readonly(address, false)],
        );
        let result = svm.process_instruction(&instruction, &snapshot);
        assert_eq!(
            result.raw_result,
            if valid {
                Ok(())
            } else {
                Err(InstructionError::InvalidSeeds)
            },
            "mode {mode}, address {address}, bump {supplied_bump}"
        );
        assert_eq!(result.resulting_accounts, snapshot);
        result.compute_units_consumed
    };
    for mode in 0..=6 {
        let cu = exercise(mode, canonical, bump, true);
        println!(
            "canonical PDA mode={mode}, CU={cu}, bump={bump}, address={canonical}, ELF bytes={}",
            elf.len()
        );
        exercise(mode, Pubkey::new_from_array([91; 32]), bump, false);
        for candidate in 0..=255u8 {
            if candidate != bump {
                exercise(mode, canonical, candidate, false);
                if let Ok(noncanonical) =
                    Pubkey::create_program_address(&[b"config", b"v1", &[candidate]], &program_id)
                {
                    exercise(mode, noncanonical, candidate, false);
                }
            }
        }
    }
}

#[test]
#[ignore = "requires HOPPER_CANONICAL_PDA_SBF pointing to the compiled fixture"]
fn stored_bump_binds_the_same_account_byte_without_claiming_canonicality() {
    let elf = std::fs::read(std::env::var("HOPPER_CANONICAL_PDA_SBF").unwrap()).unwrap();
    let program_id = Pubkey::from_str("8RJxAyfAMnpb5ghwA4comPDJw6KqbDmZ28LDZHcccaVH").unwrap();
    let (canonical, canonical_bump) =
        Pubkey::find_program_address(&[b"config", b"v1"], &program_id);
    let mut svm = Mollusk::default();
    svm.add_program_with_loader_and_elf(&program_id, &LOADER_V3, &elf);
    let exercise = |address: Pubkey, stored_bump: u8, owner: Pubkey, valid| {
        let mut data = vec![0; Config::LEN];
        Config::write_init_header(&mut data).unwrap();
        data[Config::CANONICAL_BUMP_ABS_OFFSET as usize] = stored_bump;
        let accounts = vec![(
            address,
            Account {
                lamports: 1_000_000,
                data,
                owner,
                executable: false,
                rent_epoch: 0,
            },
        )];
        let ix = Instruction::new_with_bytes(
            program_id,
            &[7, stored_bump],
            vec![AccountMeta::new_readonly(address, false)],
        );
        let result = svm.process_instruction(&ix, &accounts);
        assert_eq!(
            result.raw_result.is_ok(),
            valid,
            "address={address}, stored={stored_bump}, result={:?}",
            result.raw_result
        );
        assert_eq!(result.resulting_accounts, accounts);
        result.compute_units_consumed
    };
    println!(
        "stored-bump retained CU={}",
        exercise(canonical, canonical_bump, program_id, true)
    );
    exercise(canonical, canonical_bump, Pubkey::new_unique(), false);
    for bump in 0..=255u8 {
        if bump == canonical_bump {
            continue;
        }
        // A canonical address with the wrong persisted byte is rejected.
        exercise(canonical, bump, program_id, false);
        if let Ok(address) =
            Pubkey::create_program_address(&[b"config", b"v1", &[bump]], &program_id)
        {
            // Explicit stored-bump verification accepts a matching selected
            // off-curve bump, even when it is not the canonical bump.
            exercise(address, bump, program_id, true);
        }
    }
}

#[test]
#[ignore = "requires HOPPER_CANONICAL_PDA_SBF pointing to the compiled fixture"]
fn canonical_initialization_persists_the_validated_bump_and_rejects_reinitialization() {
    let elf = std::fs::read(std::env::var("HOPPER_CANONICAL_PDA_SBF").unwrap()).unwrap();
    let program_id = Pubkey::from_str("8RJxAyfAMnpb5ghwA4comPDJw6KqbDmZ28LDZHcccaVH").unwrap();
    let (config, bump) = Pubkey::find_program_address(&[b"config", b"v1"], &program_id);
    let payer = Pubkey::new_unique();
    let mut svm = Mollusk::default();
    svm.add_program_with_loader_and_elf(&program_id, &LOADER_V3, &elf);
    let accounts = vec![
        (config, Account::default()),
        (payer, Account::new(1_000_000_000, 0, &Pubkey::default())),
        mollusk_svm::program::keyed_account_for_system_program(),
    ];
    let mut ix = Instruction::new_with_bytes(
        program_id,
        &[8, bump ^ 1],
        vec![
            AccountMeta::new(config, false),
            AccountMeta::new(payer, true),
            AccountMeta::new_readonly(Pubkey::default(), false),
        ],
    );
    let wrong = svm.process_instruction(&ix, &accounts);
    assert_eq!(wrong.raw_result, Err(InstructionError::InvalidSeeds));
    assert_eq!(wrong.resulting_accounts, accounts);
    ix.data[1] = bump;
    let initialized = svm.process_instruction(&ix, &accounts);
    assert_eq!(initialized.raw_result, Ok(()));
    let state = initialized.get_account(&config).unwrap();
    assert_eq!(state.owner, program_id);
    assert_eq!(state.data.len(), Config::LEN);
    assert_eq!(state.data[Config::CANONICAL_BUMP_ABS_OFFSET as usize], bump);
    assert_eq!(
        state.lamports,
        svm.sysvars.rent.minimum_balance(Config::LEN)
    );
    let reused = svm.process_instruction(&ix, &initialized.resulting_accounts);
    assert!(reused.raw_result.is_err());
    assert_eq!(reused.resulting_accounts, initialized.resulting_accounts);
    for mode in [5, 6, 7] {
        let read = Instruction::new_with_bytes(
            program_id,
            &[mode, bump],
            vec![AccountMeta::new_readonly(config, false)],
        );
        let checked = svm.process_instruction(&read, &[(config, state.clone())]);
        assert_eq!(checked.raw_result, Ok(()));
        println!(
            "initialized PDA bind mode={mode} CU={}",
            checked.compute_units_consumed
        );
    }
}
