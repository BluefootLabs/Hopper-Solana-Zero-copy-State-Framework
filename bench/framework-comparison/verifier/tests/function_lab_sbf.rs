use mollusk_svm::{program::loader_keys::LOADER_V3, Mollusk};
use solana_account::Account;
use solana_instruction::{AccountMeta, Instruction};
use solana_instruction_error::InstructionError;
use solana_pubkey::Pubkey;

fn bytes(hex: &str) -> Vec<u8> {
    hex.as_bytes()
        .chunks_exact(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect()
}

#[test]
#[ignore = "requires compiled function-lab ELF and independent case JSON"]
fn function_lab_known_answers_on_the_vm() {
    let program = Pubkey::new_unique();
    let elf =
        std::fs::read(std::env::var("HOPPER_FUNCTION_LAB_SBF").expect("set ELF path")).unwrap();
    let cases = std::fs::read_to_string(
        std::env::var("HOPPER_FUNCTION_LAB_CASES").expect("run scripts/function-lab-cases.py"),
    )
    .unwrap();
    let cases: Vec<serde_json::Value> = serde_json::from_str(&cases).unwrap();
    let mut svm = Mollusk::default();
    svm.add_program_with_loader_and_elf(&program, &LOADER_V3, &elf);
    for case in &cases {
        let name = case["name"].as_str().unwrap();
        let result = svm.process_instruction(
            &Instruction::new_with_bytes(program, &bytes(case["data"].as_str().unwrap()), vec![]),
            &[],
        );
        let expected = match case["error"].as_str() {
            None => Ok(()),
            Some("InvalidInstructionData") => Err(InstructionError::InvalidInstructionData),
            Some("InvalidArgument") => Err(InstructionError::InvalidArgument),
            other => panic!("unknown expected error: {other:?}"),
        };
        assert_eq!(result.raw_result, expected, "{name}");
        if expected.is_ok() {
            assert_eq!(
                result.return_data,
                bytes(case["return"].as_str().unwrap()),
                "{name}"
            );
        }
        println!("{name}: {} CU", result.compute_units_consumed);
    }
    // Duplicate account metas must share the runtime borrow state.
    let state = Pubkey::new_unique();
    let signer = Pubkey::new_unique();
    let accounts = vec![
        (
            state,
            Account {
                lamports: 1_000_000,
                data: vec![0; 16],
                owner: program,
                ..Account::default()
            },
        ),
        (
            signer,
            Account {
                lamports: 1_000_000,
                ..Account::default()
            },
        ),
    ];
    let metas = vec![
        AccountMeta::new(state, false),
        AccountMeta::new_readonly(signer, true),
        AccountMeta::new(state, false),
    ];
    for (mode, error) in [
        (0, None),
        (1, Some(InstructionError::AccountBorrowFailed)),
        (2, Some(InstructionError::AccountDataTooSmall)),
        (3, None),
    ] {
        let mut data = vec![6, mode];
        data.extend_from_slice(&42u64.to_le_bytes());
        let result = svm.process_instruction(
            &Instruction::new_with_bytes(program, &data, metas.clone()),
            &accounts,
        );
        assert_eq!(
            result.raw_result,
            error.map_or(Ok(()), Err),
            "state mode {mode}"
        );
        let mut expected = accounts.clone();
        if mode == 0 {
            expected[0].1.data[1..9].copy_from_slice(&42u64.to_le_bytes());
        }
        assert_eq!(result.resulting_accounts, expected);
    }
    for (name, error) in [
        ("unsigned", InstructionError::MissingRequiredSignature),
        ("readonly", InstructionError::Immutable),
        ("foreign-owner", InstructionError::IncorrectProgramId),
    ] {
        let mut inputs = accounts.clone();
        let mut privileges = metas.clone();
        match name {
            "unsigned" => privileges[1].is_signer = false,
            "readonly" => {
                privileges[0].is_writable = false;
                privileges[2].is_writable = false;
            }
            "foreign-owner" => inputs[0].1.owner = Pubkey::new_unique(),
            _ => unreachable!(),
        }
        let mut data = vec![6, 0];
        data.extend_from_slice(&42u64.to_le_bytes());
        let result = svm.process_instruction(
            &Instruction::new_with_bytes(program, &data, privileges),
            &inputs,
        );
        assert_eq!(result.raw_result, Err(error), "{name}");
        assert_eq!(result.resulting_accounts, inputs, "{name}: state changed");
    }
    println!(
        "{} known-answer cases and 7 account cases passed",
        cases.len()
    );
}
