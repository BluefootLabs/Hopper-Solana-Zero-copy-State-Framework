use mollusk_svm::{program::loader_keys::LOADER_V3, Mollusk};
use solana_account::Account;
use solana_instruction::{AccountMeta, Instruction};
use solana_instruction_error::InstructionError;
use solana_pubkey::Pubkey;

fn payload(mode: u8, rows: &[(u8, u8, u64)]) -> Vec<u8> {
    let mut data = vec![mode];
    data.extend_from_slice(&(rows.len() as u16).to_le_bytes());
    for (present, side, amount) in rows {
        data.extend_from_slice(&[*present, *side]);
        data.extend_from_slice(&amount.to_le_bytes());
    }
    data.extend_from_slice(&513u16.to_le_bytes());
    data
}

#[test]
#[ignore = "requires compiled borrowed-slices fixture"]
fn borrowed_batches_validate_all_elements_before_writing_on_the_vm() {
    let program = Pubkey::new_unique();
    let state = Pubkey::new_unique();
    let signer = Pubkey::new_unique();
    let elf =
        std::fs::read(std::env::var("HOPPER_BORROWED_SLICES_SBF").expect("set fixture ELF path"))
            .expect("read compiled fixture");
    let mut svm = Mollusk::default();
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
    ];
    let mut executed = 0;
    let mut run = |data: Vec<u8>,
                   accounts: &[(Pubkey, Account)],
                   metas: Vec<AccountMeta>,
                   expected: Result<(), InstructionError>,
                   count: u64,
                   total: u64| {
        let result = svm.process_instruction(
            &Instruction::new_with_bytes(program, &data, metas),
            accounts,
        );
        assert_eq!(result.raw_result, expected, "input: {data:?}");
        executed += 1;
        if expected.is_err() {
            assert_eq!(result.resulting_accounts, accounts);
        } else {
            let mut bytes = count.to_le_bytes().to_vec();
            bytes.extend_from_slice(&total.to_le_bytes());
            assert_eq!(result.return_data, bytes);
            assert_eq!(result.resulting_accounts[0].1.data, bytes);
            assert_eq!(result.resulting_accounts[0].1.owner, accounts[0].1.owner);
            assert_eq!(
                result.resulting_accounts[0].1.lamports,
                accounts[0].1.lamports
            );
            assert_eq!(result.resulting_accounts[1], accounts[1]);
        }
        result.compute_units_consumed
    };
    for mode in [0, 1] {
        for count in [0, 1, 2, 16, 32] {
            let cu = run(
                payload(mode, &vec![(1, 7, 42); count]),
                &accounts,
                metas.clone(),
                Ok(()),
                count as u64,
                count as u64 * 42,
            );
            println!("borrowed-slices mode={mode} count={count} cu={cu}");
        }
        for byte in 0..=255 {
            // The second element must be checked even after a valid first one.
            for (present, side, valid) in [
                (byte, 7, byte <= 1),
                (1, byte, matches!(byte, 1 | 7)),
                (0, byte, true),
            ] {
                let expected = if valid {
                    Ok(())
                } else {
                    Err(InstructionError::InvalidInstructionData)
                };
                run(
                    payload(mode, &[(1, 1, 42), (present, side, 99)]),
                    &accounts,
                    metas.clone(),
                    expected,
                    2,
                    141,
                );
            }
        }
        let valid = payload(mode, &[(1, 1, 42), (0, 255, 99)]);
        for end in 0..valid.len() {
            run(
                valid[..end].to_vec(),
                &accounts,
                metas.clone(),
                Err(InstructionError::InvalidInstructionData),
                0,
                0,
            );
        }
        let mut extra = valid.clone();
        extra.push(0);
        run(
            extra,
            &accounts,
            metas.clone(),
            Err(InstructionError::InvalidInstructionData),
            0,
            0,
        );
        run(
            payload(mode, &[(1, 1, 1); 33]),
            &accounts,
            metas.clone(),
            Err(InstructionError::InvalidInstructionData),
            0,
            0,
        );
        for count in [3u16, 32, 65535] {
            let mut short = valid.clone();
            short[1..3].copy_from_slice(&count.to_le_bytes());
            run(
                short,
                &accounts,
                metas.clone(),
                Err(InstructionError::InvalidInstructionData),
                0,
                0,
            );
        }
        let mut bad_nonce = valid.clone();
        *bad_nonce.last_mut().unwrap() = 3;
        run(
            bad_nonce,
            &accounts,
            metas.clone(),
            Err(InstructionError::InvalidInstructionData),
            0,
            0,
        );
        let mut unsigned = metas.clone();
        unsigned[1].is_signer = false;
        run(
            valid.clone(),
            &accounts,
            unsigned,
            Err(InstructionError::MissingRequiredSignature),
            0,
            0,
        );
        let mut readonly = metas.clone();
        readonly[0].is_writable = false;
        run(
            valid.clone(),
            &accounts,
            readonly,
            Err(InstructionError::InvalidAccountData),
            0,
            0,
        );
        let mut foreign = accounts.clone();
        foreign[0].1.owner = Pubkey::new_unique();
        run(
            valid.clone(),
            &foreign,
            metas.clone(),
            Err(InstructionError::IllegalOwner),
            0,
            0,
        );
        for size in [0, 15, 17] {
            let mut wrong = accounts.clone();
            wrong[0].1.data.resize(size, 0);
            run(
                valid.clone(),
                &wrong,
                metas.clone(),
                Err(InstructionError::InvalidAccountData),
                0,
                0,
            );
        }
        for offset in [0, 8] {
            let mut overflow = accounts.clone();
            overflow[0].1.data[offset..offset + 8].fill(255);
            run(
                valid.clone(),
                &overflow,
                metas.clone(),
                Err(InstructionError::ArithmeticOverflow),
                0,
                0,
            );
        }
        run(
            payload(mode, &[(1, 1, u64::MAX), (1, 7, 1)]),
            &accounts,
            metas.clone(),
            Err(InstructionError::ArithmeticOverflow),
            0,
            0,
        );
    }
    println!("borrowed-slices executed={executed}");
}
