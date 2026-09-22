//! Run explicitly after building bench/runtime-gate/program for SBF.
use mollusk_svm::{program::loader_keys::LOADER_V3, Mollusk};
use solana_account::Account;
use solana_instruction::{AccountMeta, Instruction};
use solana_instruction_error::InstructionError;
use solana_pubkey::Pubkey;

#[test]
#[ignore = "requires HOPPER_GATE_SBF pointing to the compiled runtime-gate fixture"]
fn ambient_gate_enforces_public_surfaces_in_sbf() {
    let path = std::env::var("HOPPER_GATE_SBF").expect("set HOPPER_GATE_SBF to the fixture ELF");
    let elf = std::fs::read(path).expect("read gate fixture ELF");
    let program_id = Pubkey::new_from_array([81; 32]);
    let state = Pubkey::new_from_array([82; 32]);
    let foreign = Pubkey::new_from_array([83; 32]);
    let mut svm = Mollusk::default();
    svm.add_program_with_loader_and_elf(&program_id, &LOADER_V3, &elf);
    let account = Account {
        lamports: 10_000_000,
        data: vec![0; 32],
        owner: program_id,
        executable: false,
        rent_epoch: 0,
    };
    let initial = vec![(state, account.clone()), (foreign, account)];
    // Run a no-policy write again after the leaked guard to prove VM isolation.
    for case in [0, 1, 2, 3, 4, 5, 6, 7, 0, 8, 9] {
        let instruction = Instruction::new_with_bytes(
            program_id,
            &[case],
            vec![
                AccountMeta::new(state, false),
                AccountMeta::new(foreign, false),
            ],
        );
        let result = svm.process_instruction(&instruction, &initial);
        let denied = match case {
            2 | 6 | 7 => Some(0xD000),
            3 | 8 | 9 => Some(0xD0FF),
            _ => None,
        };
        if let Some(code) = denied {
            assert_eq!(
                result.raw_result,
                Err(InstructionError::Custom(code)),
                "case {case}"
            );
            assert_eq!(
                result.resulting_accounts, initial,
                "case {case}: rejected write changed state"
            );
        } else {
            assert!(
                result.raw_result.is_ok(),
                "case {case}: {:?}",
                result.raw_result
            );
            let mut expected = initial.clone();
            match case {
                0 => expected[0].1.data.fill(7),
                1 => expected[0].1.data[8..16].fill(7),
                4 => {
                    expected[0].1.data[8..16].fill(7);
                    expected[1].1.data.fill(7);
                }
                5 => expected[0].1.data[16..24].fill(7),
                _ => unreachable!(),
            }
            assert_eq!(
                result.resulting_accounts, expected,
                "case {case}: incorrect write footprint"
            );
        }
        println!(
            "case {case}: {} CU, expected result verified",
            result.compute_units_consumed
        );
    }
}
