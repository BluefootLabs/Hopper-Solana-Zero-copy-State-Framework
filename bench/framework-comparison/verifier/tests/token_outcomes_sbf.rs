use mollusk_svm::{program::loader_keys::LOADER_V3, Mollusk};
use solana_account::Account;
use solana_instruction::{AccountMeta, Instruction};
use solana_instruction_error::InstructionError;
use solana_pubkey::Pubkey;

#[test]
#[ignore = "requires HOPPER_TOKEN_OUTCOMES_SBF"]
fn token_receipt_policy_and_dedup_cpi_match_complete_svm_state() {
    let mut svm = Mollusk::default();
    let program = Pubkey::new_unique();
    svm.add_program_with_loader_and_elf(
        &program,
        &LOADER_V3,
        &std::fs::read(std::env::var("HOPPER_TOKEN_OUTCOMES_SBF").unwrap()).unwrap(),
    );
    mollusk_svm_programs_token::token::add_program(&mut svm);
    mollusk_svm_programs_token::token2022::add_program(&mut svm);
    for extended in [false, true] {
        let token = if extended {
            mollusk_svm_programs_token::token2022::ID
        } else {
            mollusk_svm_programs_token::token::ID
        };
        let token_program = if extended {
            mollusk_svm_programs_token::token2022::account()
        } else {
            mollusk_svm_programs_token::token::account()
        };
        let mint = Pubkey::new_unique();
        let source = Pubkey::new_unique();
        let destination = Pubkey::new_unique();
        let authority = Pubkey::new_unique();
        let mut mint_account = Account::new(10_000_000, if extended { 278 } else { 82 }, &token);
        mint_account.data[..4].copy_from_slice(&1u32.to_le_bytes());
        mint_account.data[4..36].copy_from_slice(authority.as_ref());
        mint_account.data[36..44].copy_from_slice(&1100u64.to_le_bytes());
        mint_account.data[44..46].copy_from_slice(&[6, 1]);
        if extended {
            mint_account.data[165] = 1;
            mint_account.data[166..168].copy_from_slice(&1u16.to_le_bytes());
            mint_account.data[168..170].copy_from_slice(&108u16.to_le_bytes());
            for at in [170 + 72, 170 + 90] {
                mint_account.data[at + 8..at + 16].copy_from_slice(&100u64.to_le_bytes());
                mint_account.data[at + 16..at + 18].copy_from_slice(&100u16.to_le_bytes());
            }
        }
        let make_token = |amount: u64| {
            let mut a = Account::new(10_000_000, if extended { 178 } else { 165 }, &token);
            a.data[..32].copy_from_slice(mint.as_ref());
            a.data[32..64].copy_from_slice(authority.as_ref());
            a.data[64..72].copy_from_slice(&amount.to_le_bytes());
            a.data[108] = 1;
            if extended {
                a.data[165] = 2;
                a.data[166..168].copy_from_slice(&2u16.to_le_bytes());
                a.data[168..170].copy_from_slice(&8u16.to_le_bytes());
            }
            a
        };
        let accounts = vec![
            (source, make_token(1000)),
            (mint, mint_account),
            (destination, make_token(100)),
            (authority, Account::new(1_000_000, 0, &Pubkey::default())),
            (token, token_program),
        ];
        for (expected_debit, minimum, actual, ok) in [
            (100u64, 99u64, 100u64, true),
            (100, 100, 100, !extended),
            (100, 99, 99, false),
            (100, 1, 0, false),
            (100, 0, 101, false),
            (0, 0, 0, false),
            (100, 101, 100, false),
        ] {
            let mut data = vec![0];
            for n in [expected_debit, minimum, actual] {
                data.extend_from_slice(&n.to_le_bytes());
            }
            let ix = Instruction::new_with_bytes(
                program,
                &data,
                vec![
                    AccountMeta::new(source, false),
                    AccountMeta::new_readonly(mint, false),
                    AccountMeta::new(destination, false),
                    AccountMeta::new_readonly(authority, true),
                    AccountMeta::new_readonly(token, false),
                ],
            );
            let result = svm.process_instruction(&ix, &accounts);
            let expected_error = if expected_debit == 0 || minimum > expected_debit {
                InstructionError::InvalidArgument
            } else {
                InstructionError::InvalidAccountData
            };
            assert_eq!(
                result.raw_result,
                if ok { Ok(()) } else { Err(expected_error) },
                "extended={extended}, {data:?}"
            );
            let mut expected = accounts.clone();
            if ok {
                let fee = if extended { 1u64 } else { 0 };
                expected[0].1.data[64..72].copy_from_slice(&(1000 - actual).to_le_bytes());
                expected[2].1.data[64..72].copy_from_slice(&(100 + actual - fee).to_le_bytes());
                if extended {
                    expected[2].1.data[170..178].copy_from_slice(&fee.to_le_bytes());
                }
            }
            assert_eq!(
                result.resulting_accounts, expected,
                "extended={extended}, {data:?}"
            );
            println!("token extended={extended}, policy={expected_debit}/{minimum}, actual={actual}: {} CU",result.compute_units_consumed);
        }
    }
    let source = Pubkey::new_unique();
    let destination = Pubkey::new_unique();
    let extra = Pubkey::new_unique();
    let system = Pubkey::default();
    let accounts = vec![
        (source, Account::new(10_000_000, 0, &system)),
        (destination, Account::new(2_000_000, 0, &system)),
        (extra, Account::new(3_000_000, 0, &system)),
        mollusk_svm::program::keyed_account_for_system_program(),
    ];
    let mut data = vec![1];
    data.extend_from_slice(&100u64.to_le_bytes());
    let ix = Instruction::new_with_bytes(
        program,
        &data,
        vec![
            AccountMeta::new(source, true),
            AccountMeta::new(destination, false),
            AccountMeta::new_readonly(extra, false),
            AccountMeta::new_readonly(system, false),
        ],
    );
    let result = svm.process_instruction(&ix, &accounts);
    assert_eq!(result.raw_result, Ok(()));
    let mut expected = accounts.clone();
    expected[0].1.lamports -= 100;
    expected[1].1.lamports += 100;
    assert_eq!(result.resulting_accounts, expected);
}
