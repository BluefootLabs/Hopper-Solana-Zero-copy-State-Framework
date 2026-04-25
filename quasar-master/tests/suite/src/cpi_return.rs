use {crate::helpers::*, quasar_svm::Instruction, quasar_test_misc::cpi::*};

#[test]
fn cpi_invoke_with_return_round_trips_u64() {
    let mut svm = svm_misc();
    let ix: Instruction = CpiInvokeWithReturnInstruction {
        program: quasar_test_misc::ID,
    }
    .into();

    let result = svm.process_instruction(&ix, &[]);
    assert!(result.is_ok(), "u64 return: {:?}", result.raw_result);
}

#[test]
fn cpi_invoke_with_return_round_trips_struct() {
    let mut svm = svm_misc();
    let ix: Instruction = CpiInvokeStructReturnInstruction {
        program: quasar_test_misc::ID,
    }
    .into();

    let result = svm.process_instruction(&ix, &[]);
    assert!(result.is_ok(), "struct return: {:?}", result.raw_result);
}

#[test]
fn cpi_plain_invoke_ignores_return_data() {
    let mut svm = svm_misc();
    let ix: Instruction = CpiInvokeIgnoreReturnInstruction {
        program: quasar_test_misc::ID,
    }
    .into();

    let result = svm.process_instruction(&ix, &[]);
    assert!(result.is_ok(), "plain invoke: {:?}", result.raw_result);
}

#[test]
fn cpi_invoke_with_return_detects_missing_return_after_prior_success() {
    let mut svm = svm_misc();
    let ix: Instruction = CpiInvokeMissingReturnInstruction {
        program: quasar_test_misc::ID,
    }
    .into();

    let result = svm.process_instruction(&ix, &[]);
    assert!(result.is_ok(), "missing return: {:?}", result.raw_result);
}
