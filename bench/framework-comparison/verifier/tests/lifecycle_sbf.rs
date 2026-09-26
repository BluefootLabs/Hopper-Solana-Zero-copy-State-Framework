use mollusk_svm::{program::loader_keys::LOADER_V3, Mollusk};
use solana_account::Account;
use solana_instruction::{AccountMeta, Instruction};
use solana_instruction_error::InstructionError;
use solana_pubkey::Pubkey;

#[test]
#[ignore = "requires compiled lifecycle fixtures"]
fn lifecycle_and_segment_guards_preserve_complete_account_state() {
    let kind = std::env::var("HOPPER_LIFECYCLE_KIND").unwrap();
    let runtime = kind == "runtime";
    let program = Pubkey::new_unique();
    let state = Pubkey::new_unique();
    let recipient = Pubkey::new_unique();
    let payer = Pubkey::new_unique();
    let mut svm = Mollusk::default();
    svm.add_program_with_loader_and_elf(
        &program,
        &LOADER_V3,
        &std::fs::read(std::env::var("HOPPER_LIFECYCLE_SBF").unwrap()).unwrap(),
    );
    let rent16 = svm.sysvars.rent.minimum_balance(16);
    let rent32 = svm.sysvars.rent.minimum_balance(32);
    let mut cases: Vec<(Vec<u8>, bool, Result<(), InstructionError>)> = if runtime {
        (0..=6).map(|tag| (vec![tag], true, Ok(()))).collect()
    } else {
        (0..=9)
            .map(|tag| {
                (
                    if tag == 5 { vec![5, 32, 0] } else { vec![tag] },
                    true,
                    Ok(()),
                )
            })
            .collect()
    };
    if !runtime {
        cases.push((
            vec![5, 32, 0],
            false,
            Err(InstructionError::MissingRequiredSignature),
        ));
        cases.push((
            vec![5, 255, 255],
            true,
            Err(InstructionError::InvalidRealloc),
        ));
    }
    for (data, signer, error) in cases {
        let tag = data[0];
        let mut accounts = vec![
            (
                state,
                Account {
                    lamports: rent16,
                    data: vec![5; 16],
                    owner: program,
                    ..Account::default()
                },
            ),
            (
                recipient,
                Account {
                    lamports: 1_000_000,
                    ..Account::default()
                },
            ),
        ];
        let mut metas = vec![
            AccountMeta::new(state, false),
            if !runtime && tag == 3 {
                AccountMeta::new_readonly(recipient, false)
            } else {
                AccountMeta::new(recipient, false)
            },
        ];
        if !runtime {
            accounts.push((
                payer,
                Account {
                    lamports: 10_000_000,
                    ..Account::default()
                },
            ));
            metas.push(AccountMeta::new(payer, signer));
        }
        let system = mollusk_svm::program::keyed_account_for_system_program();
        metas.push(AccountMeta::new_readonly(system.0, false));
        accounts.push(system);
        let result = svm.process_instruction(
            &Instruction::new_with_bytes(program, &data, metas),
            &accounts,
        );
        assert_eq!(result.raw_result, error, "{kind} {data:?} signer={signer}");
        let mut expected = accounts.clone();
        if error.is_ok() {
            match (runtime, tag) {
                (true, 2) => expected[0].1.data = [vec![7; 8], vec![9; 8]].concat(),
                (false, 8) => expected[0].1.data[8..].fill(3),
                (false, 4) => {
                    expected[0].1.lamports -= 100;
                    expected[1].1.lamports += 100;
                }
                (false, 5 | 9) => {
                    expected[0].1.data.resize(32, 0);
                    expected[0].1.lamports = rent32;
                    expected[2].1.lamports -= rent32 - rent16;
                }
                (true, 5) | (false, 6) => {
                    expected[1].1.lamports += rent16;
                    expected[0].1.lamports = 0;
                    expected[0].1.data.fill(0);
                    if !runtime {
                        expected[0].1 = Account::default();
                    }
                }
                _ => {}
            }
        }
        assert_eq!(result.resulting_accounts, expected, "{kind} {data:?}");
        println!(
            "{kind} {data:?} signer={signer}: {} CU",
            result.compute_units_consumed
        );
    }
}
