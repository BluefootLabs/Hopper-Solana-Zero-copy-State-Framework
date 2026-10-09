use mollusk_svm::{program::loader_keys::LOADER_V3, Mollusk};
use solana_account::Account;
use solana_instruction::{AccountMeta, Instruction};
use solana_instruction_error::InstructionError;
use solana_pubkey::Pubkey;

fn payload(mode: u8, tags: [u8; 4], amount: u64, tail: &[u8]) -> Vec<u8> {
    let mut bytes = vec![mode];
    bytes.extend_from_slice(&tags);
    bytes.extend_from_slice(&amount.to_le_bytes());
    bytes.extend_from_slice(tail);
    bytes
}

#[test]
#[ignore = "requires compiled borrowed-argument fixture"]
fn borrowed_arguments_validate_before_writing_on_the_vm() {
    exercise(0);
}

#[test]
#[ignore = "requires compiled borrowed-argument fixture"]
fn generated_borrowed_dispatch_validates_before_writing_on_the_vm() {
    exercise(2);
}

fn exercise(exact_mode: u8) {
    let program = Pubkey::new_unique();
    let state = Pubkey::new_unique();
    let authority = Pubkey::new_unique();
    let mut svm = Mollusk::default();
    let elf =
        std::fs::read(std::env::var("HOPPER_BORROWED_ARGS_SBF").expect("set fixture ELF path"))
            .expect("read compiled fixture");
    svm.add_program_with_loader_and_elf(&program, &LOADER_V3, &elf);
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
            authority,
            Account {
                lamports: 1_000_000,
                ..Account::default()
            },
        ),
    ];
    let metas = vec![
        AccountMeta::new(state, false),
        AccountMeta::new_readonly(authority, true),
    ];
    let run = |data: Vec<u8>, accounts: &[(Pubkey, Account)], metas: Vec<AccountMeta>| {
        svm.process_instruction(
            &Instruction::new_with_bytes(program, &data, metas),
            accounts,
        )
    };
    // Every byte is visited as an option tag and as a present enum payload.
    for byte in 0..=u8::MAX {
        for (tags, accepted) in [
            ([byte, 7, 0, 255], byte <= 1),
            ([1, byte, 0, 255], matches!(byte, 1 | 7)),
            ([0, byte, 1, 1], true),
            ([1, 1, 1, byte], matches!(byte, 1 | 7)),
        ] {
            let result = run(payload(exact_mode, tags, 42, &[]), &accounts, metas.clone());
            if accepted {
                assert_eq!(result.raw_result, Ok(()));
                let mut expected = 1u64.to_le_bytes().to_vec();
                expected.extend_from_slice(&42u64.to_le_bytes());
                assert_eq!(result.return_data, expected);
                assert_eq!(result.resulting_accounts[0].1.data, expected);
                assert_eq!(result.resulting_accounts[1], accounts[1]);
            } else {
                assert_eq!(
                    result.raw_result,
                    Err(InstructionError::InvalidInstructionData)
                );
                assert_eq!(result.resulting_accounts, accounts);
            }
        }
    }
    let valid = payload(exact_mode, [1, 1, 0, 255], 42, &[]);
    for length in 0..valid.len() {
        let result = run(valid[..length].to_vec(), &accounts, metas.clone());
        assert_eq!(
            result.raw_result,
            Err(InstructionError::InvalidInstructionData)
        );
        assert_eq!(result.resulting_accounts, accounts);
    }
    for (mode, tail, accepted) in [
        (exact_mode, &b"x"[..], false),
        (exact_mode + 1, &b"memo"[..], true),
        (exact_mode + 1, &b""[..], true),
        (exact_mode + 1, &[b'x'; 32][..], true),
        (exact_mode + 1, &[b'x'; 33][..], false),
        (exact_mode + 1, &[255][..], false),
    ] {
        let result = run(
            payload(mode, [1, 7, 0, 255], 42, tail),
            &accounts,
            metas.clone(),
        );
        assert_eq!(
            result.raw_result,
            if accepted {
                Ok(())
            } else {
                Err(InstructionError::InvalidInstructionData)
            }
        );
        if !accepted {
            assert_eq!(result.resulting_accounts, accounts);
        }
    }
    let mut unsigned = metas.clone();
    unsigned[1].is_signer = false;
    let result = run(valid.clone(), &accounts, unsigned);
    assert_eq!(
        result.raw_result,
        Err(InstructionError::MissingRequiredSignature)
    );
    assert_eq!(result.resulting_accounts, accounts);
    let mut readonly = metas.clone();
    readonly[0].is_writable = false;
    let result = run(valid.clone(), &accounts, readonly);
    assert_eq!(result.raw_result, Err(InstructionError::InvalidAccountData));
    assert_eq!(result.resulting_accounts, accounts);
    let mut foreign = accounts.clone();
    foreign[0].1.owner = Pubkey::new_unique();
    let result = run(valid.clone(), &foreign, metas.clone());
    assert_eq!(result.raw_result, Err(InstructionError::IllegalOwner));
    assert_eq!(result.resulting_accounts, foreign);
    for range in [0..8, 8..16] {
        let mut overflow = accounts.clone();
        overflow[0].1.data[range].fill(255);
        let result = run(valid.clone(), &overflow, metas.clone());
        assert_eq!(result.raw_result, Err(InstructionError::ArithmeticOverflow));
        assert_eq!(result.resulting_accounts, overflow);
    }
    for length in [0, 15, 17] {
        let mut wrong_size = accounts.clone();
        wrong_size[0].1.data.resize(length, 0);
        let result = run(valid.clone(), &wrong_size, metas.clone());
        assert_eq!(result.raw_result, Err(InstructionError::InvalidAccountData));
        assert_eq!(result.resulting_accounts, wrong_size);
    }
    for (name, mode, tags, tail) in [
        ("exact", exact_mode, [1, 1, 0, 255], &b""[..]),
        ("tail32", exact_mode + 1, [1, 1, 0, 255], &[b'x'; 32][..]),
        ("bad-option", exact_mode, [2, 1, 0, 255], &b""[..]),
    ] {
        let result = run(payload(mode, tags, 42, tail), &accounts, metas.clone());
        assert_eq!(
            result.raw_result,
            if name == "bad-option" {
                Err(InstructionError::InvalidInstructionData)
            } else {
                Ok(())
            }
        );
        println!(
            "borrowed-dispatch mode={mode} case={name} cu={}",
            result.compute_units_consumed
        );
    }
}
