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
