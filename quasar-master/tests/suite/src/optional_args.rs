use {
    crate::helpers::*,
    quasar_svm::{Instruction, Pubkey},
    quasar_test_misc::cpi::*,
};

// Happy-path tests use generated CPI structs

#[test]
fn option_u64_some_happy() {
    let mut svm = svm_misc();
    let signer = Pubkey::new_unique();
    let ix: Instruction = OptionU64SomeInstruction {
        signer,
        value: Some(42),
    }
    .into();
    let result = svm.process_instruction(&ix, &[signer_account(signer)]);
    assert!(
        result.is_ok(),
        "Option<u64> Some(42): {:?}",
        result.raw_result
    );
}

#[test]
fn option_u64_none_happy() {
    let mut svm = svm_misc();
    let signer = Pubkey::new_unique();
    let ix: Instruction = OptionU64NoneInstruction {
        signer,
        value: None,
    }
    .into();
    let result = svm.process_instruction(&ix, &[signer_account(signer)]);
    assert!(result.is_ok(), "Option<u64> None: {:?}", result.raw_result);
}

#[test]
fn option_u64_some_wrong_value() {
    let mut svm = svm_misc();
    let signer = Pubkey::new_unique();
    let ix: Instruction = OptionU64SomeInstruction {
        signer,
        value: Some(99),
    }
    .into();
    let result = svm.process_instruction(&ix, &[signer_account(signer)]);
    assert!(result.is_err(), "Option<u64> Some(99) should fail require");
}

#[test]
fn option_address_some_happy() {
    let mut svm = svm_misc();
    let signer = Pubkey::new_unique();
    let addr = Pubkey::new_unique();
    let ix: Instruction = OptionAddressSomeInstruction {
        signer,
        addr: Some(addr),
    }
    .into();
    let result = svm.process_instruction(&ix, &[signer_account(signer)]);
    assert!(
        result.is_ok(),
        "Option<Address> Some: {:?}",
        result.raw_result
    );
}

#[test]
fn option_address_none_happy() {
    let mut svm = svm_misc();
    let signer = Pubkey::new_unique();
    let ix: Instruction = OptionAddressNoneInstruction { signer, addr: None }.into();
    let result = svm.process_instruction(&ix, &[signer_account(signer)]);
    assert!(
        result.is_ok(),
        "Option<Address> None: {:?}",
        result.raw_result
    );
}

// Adversarial test: manually craft instruction data with tag=2 (invalid)
#[test]
fn option_u64_tag_two_rejected() {
    let mut svm = svm_misc();
    let signer = Pubkey::new_unique();
    // Discriminator 52 (option_u64_some expects Some(42))
    // Wire format: [disc, tag, PodU64 le bytes]
    let mut data = vec![52u8]; // discriminator
    data.push(2); // tag = 2 (invalid — only 0 and 1 are valid)
    data.extend_from_slice(&42u64.to_le_bytes()); // PodU64(42)
    let ix = solana_instruction::Instruction {
        program_id: quasar_test_misc::ID,
        accounts: vec![solana_instruction::AccountMeta::new_readonly(signer, true)],
        data,
    };
    let result = svm.process_instruction(&ix, &[signer_account(signer)]);
    assert!(result.is_err(), "tag=2 should be rejected by validate_zc");
}

// Adversarial test: tag=0xFF (invalid)
#[test]
fn option_u64_tag_0xff_rejected() {
    let mut svm = svm_misc();
    let signer = Pubkey::new_unique();
    let mut data = vec![52u8]; // discriminator
    data.push(0xFF); // tag = 0xFF (invalid)
    data.extend_from_slice(&42u64.to_le_bytes());
    let ix = solana_instruction::Instruction {
        program_id: quasar_test_misc::ID,
        accounts: vec![solana_instruction::AccountMeta::new_readonly(signer, true)],
        data,
    };
    let result = svm.process_instruction(&ix, &[signer_account(signer)]);
    assert!(
        result.is_err(),
        "tag=0xFF should be rejected by validate_zc"
    );
}

// Adversarial test: truncated instruction data (disc only, no Option payload)
#[test]
fn option_u64_truncated_data_fails() {
    let mut svm = svm_misc();
    let signer = Pubkey::new_unique();
    // Only send the discriminator byte, no Option<u64> payload
    let ix = solana_instruction::Instruction {
        program_id: quasar_test_misc::ID,
        accounts: vec![solana_instruction::AccountMeta::new_readonly(signer, true)],
        data: vec![52u8], // just discriminator, missing 9 bytes of OptionZc<PodU64>
    };
    let result = svm.process_instruction(&ix, &[signer_account(signer)]);
    assert!(result.is_err(), "truncated instruction data should fail");
}
