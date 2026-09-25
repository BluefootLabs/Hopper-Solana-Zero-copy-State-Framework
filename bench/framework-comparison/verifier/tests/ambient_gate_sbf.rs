//! Run explicitly after building bench/runtime-gate/program for SBF.
use mollusk_svm::{program::loader_keys::LOADER_V3, Mollusk};
use solana_account::Account;
use solana_instruction::{AccountMeta, Instruction};
use solana_instruction_error::InstructionError;
use solana_pubkey::Pubkey;

#[allow(dead_code)]
#[path = "../../../runtime-gate/program/src/lib.rs"]
mod fixture;

fn typed_header_hex() -> String {
    let mut bytes = [0u8; 16];
    hopper::layout::write_header(
        &mut bytes,
        fixture::CellProbe::DISC,
        fixture::CellProbe::VERSION,
        &fixture::CellProbe::LAYOUT_ID,
    )
    .unwrap();
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Export the exact header for scripts/test-runtime-gate-devnet.py without
/// loading an ELF. The integration test below verifies its on-chain bytes.
#[test]
fn typed_cell_probe_header() {
    assert_eq!(fixture::CellProbe::LEN, 32);
    assert_eq!(fixture::CellProbe::PROTECTED_ABS_OFFSET, 16);
    assert_eq!(fixture::CellProbe::VALUES_ABS_OFFSET, 24);
    assert_eq!(fixture::CellProbe::VALUES_ELEMENT_COUNT, 8);
    println!("typed-header-hex: {}", typed_header_hex());
}

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

    // Build the real typed fixture on-chain, then exercise generated bound
    // cell accessors and all surrounding refusal boundaries against it.
    let instruction = |case| {
        Instruction::new_with_bytes(
            program_id,
            &[case],
            vec![
                AccountMeta::new(state, false),
                AccountMeta::new(foreign, false),
            ],
        )
    };
    let mut typed_initial = initial.clone();
    hopper::layout::write_header(
        &mut typed_initial[0].1.data,
        fixture::CellProbe::DISC,
        fixture::CellProbe::VERSION,
        &fixture::CellProbe::LAYOUT_ID,
    )
    .unwrap();
    println!("typed-header-hex: {}", typed_header_hex());
    let initialized = svm.process_instruction(&instruction(16), &initial);
    assert_eq!(initialized.raw_result, Ok(()));
    assert_eq!(initialized.resulting_accounts, typed_initial);
    println!(
        "case 16: {} CU, typed initialization verified",
        initialized.compute_units_consumed
    );
    let reinitialized = svm.process_instruction(&instruction(16), &typed_initial);
    assert_eq!(
        reinitialized.raw_result,
        Err(InstructionError::AccountAlreadyInitialized)
    );
    assert_eq!(reinitialized.resulting_accounts, typed_initial);

    for case in 10..=15 {
        let result = svm.process_instruction(&instruction(case), &typed_initial);
        let mut expected = typed_initial.clone();
        match case {
            10 => {
                assert_eq!(result.raw_result, Ok(()));
                expected[0].1.data[fixture::CellProbe::VALUES_ABS_OFFSET as usize + 2] = 7;
            }
            11..=14 => assert_eq!(result.raw_result, Err(InstructionError::Custom(0xD000))),
            15 => assert_eq!(
                result.raw_result,
                Err(InstructionError::InvalidInstructionData)
            ),
            _ => unreachable!(),
        }
        assert_eq!(
            result.resulting_accounts, expected,
            "typed case {case}: incorrect footprint"
        );
        println!(
            "case {case}: {} CU, expected typed result verified",
            result.compute_units_consumed
        );
    }
}
