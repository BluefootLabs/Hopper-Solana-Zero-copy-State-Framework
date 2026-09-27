use mollusk_svm::{program::loader_keys::LOADER_V3, Mollusk};
use solana_account::Account;
use solana_instruction::{AccountMeta, Instruction};
use solana_instruction_error::InstructionError;
use solana_pubkey::Pubkey;

#[test]
#[ignore = "requires compiled sibling introspection fixture"]
fn sibling_reads_respect_lengths_privileges_order_and_scope() {
    let program = Pubkey::new_unique();
    let payer = Pubkey::new_unique();
    let elf = std::fs::read(std::env::var("HOPPER_SIBLING_SBF").unwrap()).unwrap();
    let baseline = std::env::var_os("HOPPER_SIBLING_BASELINE").is_some();
    let mut svm = Mollusk::default();
    svm.add_program_with_loader_and_elf(&program, &LOADER_V3, &elf);
    let accounts = vec![
        (
            payer,
            Account {
                lamports: 1_000_000,
                ..Account::default()
            },
        ),
        (
            program,
            Account {
                lamports: 1_000_000,
                owner: LOADER_V3,
                executable: true,
                ..Account::default()
            },
        ),
    ];
    for case in 1..=11 {
        if baseline && ![1, 2, 3, 6].contains(&case) {
            continue;
        }
        let expected = if baseline {
            Err(InstructionError::Custom(100 + u32::from(case)))
        } else if [5, 11].contains(&case) {
            Err(InstructionError::AccountDataTooSmall)
        } else {
            Ok(())
        };
        let result = svm.process_instruction(
            &Instruction::new_with_bytes(
                program,
                &[20, case],
                vec![
                    AccountMeta::new(payer, true),
                    AccountMeta::new_readonly(program, false),
                ],
            ),
            &accounts,
        );
        assert_eq!(
            result.raw_result, expected,
            "case {case}, baseline {baseline}"
        );
        assert_eq!(result.resulting_accounts, accounts);
        println!(
            "sibling case {case}: {} CU; {:?}",
            result.compute_units_consumed, result.raw_result
        );
    }
}
