use mollusk_svm::{program::loader_keys::LOADER_V3, Mollusk};
use solana_account::Account;
use solana_instruction::{AccountMeta, Instruction};
use solana_instruction_error::InstructionError;
use solana_pubkey::Pubkey;

#[test]
#[ignore = "requires compiled return provenance fixture"]
fn return_data_checks_direct_producer_and_typed_prefix() {
    let driver = Pubkey::new_unique();
    let callee = Pubkey::new_unique();
    let nested = Pubkey::new_unique();
    let elf = std::fs::read(std::env::var("HOPPER_RETURN_PROVENANCE_SBF").unwrap()).unwrap();
    let mut svm = Mollusk::default();
    for key in [driver, callee, nested] {
        svm.add_program_with_loader_and_elf(&key, &LOADER_V3, &elf);
    }
    let executable = Account {
        lamports: 1_000_000,
        owner: LOADER_V3,
        executable: true,
        ..Account::default()
    };
    for (tag, expected) in [
        (4, Ok(())),
        (7, Err(InstructionError::IncorrectProgramId)),
        (5, Err(InstructionError::AccountDataTooSmall)),
        (6, Err(InstructionError::InvalidAccountData)),
    ] {
        let mut metas = vec![AccountMeta::new_readonly(callee, false)];
        let mut accounts = vec![(callee, executable.clone())];
        if tag == 7 {
            metas.push(AccountMeta::new_readonly(nested, false));
            accounts.push((nested, executable.clone()));
        }
        let result = svm.process_instruction(
            &Instruction::new_with_bytes(driver, &[tag], metas),
            &accounts,
        );
        assert_eq!(result.raw_result, expected, "tag {tag}");
        assert_eq!(result.resulting_accounts, accounts);
        if tag == 4 {
            assert_eq!(result.return_data, 42u64.to_le_bytes());
        }
        println!("return tag {tag}: {} CU", result.compute_units_consumed);
    }
}
