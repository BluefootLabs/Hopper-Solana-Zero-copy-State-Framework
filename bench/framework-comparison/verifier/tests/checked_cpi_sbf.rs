use mollusk_svm::{program::loader_keys::LOADER_V3, Mollusk};
use solana_account::Account;
use solana_instruction::{AccountMeta, Instruction};
use solana_pubkey::Pubkey;

#[test]
#[ignore = "requires compiled native CPI fixture"]
fn specialized_signer_preflight_returns_before_system_or_token_cpi() {
    let program = Pubkey::new_unique();
    let mut svm = Mollusk::default();
    svm.add_program_with_loader_and_elf(
        &program,
        &LOADER_V3,
        &std::fs::read(std::env::var("HOPPER_CHECKED_CPI_SBF").unwrap()).unwrap(),
    );
    let from = Pubkey::new_unique();
    let to = Pubkey::new_unique();
    let accounts = vec![
        (
            from,
            Account {
                lamports: 1_000_000,
                ..Account::default()
            },
        ),
        (
            to,
            Account {
                lamports: 1_000_000,
                ..Account::default()
            },
        ),
    ];
    for tag in [0, 1] {
        let ix = Instruction::new_with_bytes(
            program,
            &[tag],
            vec![AccountMeta::new(from, false), AccountMeta::new(to, false)],
        );
        let result = svm.process_instruction(&ix, &accounts);
        assert_eq!(result.raw_result, Ok(()));
        assert_eq!(result.resulting_accounts, accounts);
        assert_eq!(result.return_data, vec![tag, 0xAC]);
    }
    for tag in [2, 3] {
        let ix = Instruction::new_with_bytes(
            program,
            &[tag],
            vec![AccountMeta::new(from, false), AccountMeta::new(to, false)],
        );
        let result = svm.process_instruction(&ix, &accounts);
        assert!(result.raw_result.is_err());
        assert_eq!(result.resulting_accounts, accounts);
        assert!(
            result.compute_units_consumed < 1000,
            "abort used {} CU",
            result.compute_units_consumed
        );
        println!("abort tag {tag}: {} CU", result.compute_units_consumed);
    }
}
