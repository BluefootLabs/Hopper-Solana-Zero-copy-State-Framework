use {
    crate::helpers::*,
    quasar_svm::{Instruction, Pubkey},
    quasar_test_token_cpi::cpi::*,
};

#[test]
fn close_spl() {
    let mut svm = svm_cpi();
    let authority = Pubkey::new_unique();
    let mint_key = Pubkey::new_unique();
    let account_key = Pubkey::new_unique();
    let token_program = spl_token_program_id();

    let instruction: Instruction = CloseTokenAccountInstruction {
        authority,
        account: account_key,
        destination: authority,
        token_program,
    }
    .into();

    let result = svm.process_instruction(
        &instruction,
        &[
            signer_account(authority),
            token_account(account_key, mint_key, authority, 0, token_program),
        ],
    );
    assert!(
        result.is_ok(),
        "close SPL should succeed: {:?}",
        result.raw_result
    );
}

#[test]
fn close_t22() {
    let mut svm = svm_cpi();
    let authority = Pubkey::new_unique();
    let mint_key = Pubkey::new_unique();
    let account_key = Pubkey::new_unique();
    let token_program = token_2022_program_id();

    let instruction: Instruction = CloseTokenAccountT22Instruction {
        authority,
        account: account_key,
        destination: authority,
        token_program,
    }
    .into();

    let result = svm.process_instruction(
        &instruction,
        &[
            signer_account(authority),
            token_account(account_key, mint_key, authority, 0, token_program),
        ],
    );
    assert!(
        result.is_ok(),
        "close Token-2022 should succeed: {:?}",
        result.raw_result
    );
}

#[test]
fn close_interface_spl() {
    let mut svm = svm_cpi();
    let authority = Pubkey::new_unique();
    let mint_key = Pubkey::new_unique();
    let account_key = Pubkey::new_unique();
    let token_program = spl_token_program_id();

    let instruction: Instruction = CloseTokenAccountInterfaceInstruction {
        authority,
        account: account_key,
        destination: authority,
        token_program,
    }
    .into();

    let result = svm.process_instruction(
        &instruction,
        &[
            signer_account(authority),
            token_account(account_key, mint_key, authority, 0, token_program),
        ],
    );
    assert!(
        result.is_ok(),
        "close interface SPL should succeed: {:?}",
        result.raw_result
    );
}

#[test]
fn close_interface_t22() {
    let mut svm = svm_cpi();
    let authority = Pubkey::new_unique();
    let mint_key = Pubkey::new_unique();
    let account_key = Pubkey::new_unique();
    let token_program = token_2022_program_id();

    let instruction: Instruction = CloseTokenAccountInterfaceInstruction {
        authority,
        account: account_key,
        destination: authority,
        token_program,
    }
    .into();

    let result = svm.process_instruction(
        &instruction,
        &[
            signer_account(authority),
            token_account(account_key, mint_key, authority, 0, token_program),
        ],
    );
    assert!(
        result.is_ok(),
        "close interface Token-2022 should succeed: {:?}",
        result.raw_result
    );
}
