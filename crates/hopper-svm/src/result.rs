//! Public result type — wraps an `ExecutionOutcome` plus the
//! captured log buffer and adds Hopper-aware decoders + assertion
//! helpers.

use crate::account::KeyedAccount;
use crate::engine::ExecutionOutcome;
use crate::error::HopperSvmError;
use crate::log::LogCapture;
use solana_sdk::pubkey::Pubkey;

/// Result of a `process_instruction` (or chain) call.
#[derive(Debug, Clone)]
pub struct HopperExecutionResult {
    /// The underlying engine outcome.
    pub outcome: ExecutionOutcome,
    /// Captured log lines.
    pub logs: Vec<String>,
    /// Transaction fee charged to the fee payer. `0` for
    /// `process_instruction` and `process_instruction_chain`
    /// (which don't simulate transaction fees); set by
    /// `process_transaction` to the deducted amount in
    /// lamports. Mainnet-equivalent: this is what the validator
    /// would charge the payer at the start of the transaction.
    pub transaction_fee_paid: u64,
}

impl HopperExecutionResult {
    /// Internal — wrap an outcome + log buffer into the public type.
    pub(crate) fn from_outcome(outcome: ExecutionOutcome, logs: LogCapture) -> Self {
        Self {
            outcome,
            logs: logs.into_lines(),
            transaction_fee_paid: 0,
        }
    }

    /// Read the transaction fee deducted from the fee payer.
    /// Always 0 for `process_instruction` /
    /// `process_instruction_chain`; non-zero only when called
    /// through `process_transaction`.
    pub fn transaction_fee_paid(&self) -> u64 {
        self.transaction_fee_paid
    }

    /// Returns true if the instruction (or every instruction in a
    /// chain) succeeded.
    pub fn is_success(&self) -> bool {
        self.outcome.error.is_none()
    }

    /// Returns true if the instruction failed.
    pub fn is_error(&self) -> bool {
        self.outcome.error.is_some()
    }

    /// Borrow the failure if any.
    pub fn error(&self) -> Option<&HopperSvmError> {
        self.outcome.error.as_ref()
    }

    /// Compute units consumed by the instruction.
    pub fn compute_units_consumed(&self) -> u64 {
        self.outcome.compute_units_consumed
    }

    /// Convenience accessor for the field of the same name —
    /// idiomatic where users have already destructured the result.
    pub fn return_data(&self) -> &[u8] {
        &self.outcome.return_data
    }

    /// Account state after execution.
    pub fn resulting_accounts(&self) -> &[KeyedAccount] {
        &self.outcome.resulting_accounts
    }

    /// Cross-Program Invocations recorded during execution, in
    /// dispatch order. Each entry carries the inner program ID,
    /// account metas, instruction data, and stack height (1 =
    /// outermost program; 2 = first-level CPI; etc.). Empty
    /// when the instruction made no CPIs.
    ///
    /// Useful for snapshot tests that count CPIs or assert on
    /// the call pattern a program produces. Mirrors the
    /// `inner_instructions` slice mainnet records on transaction
    /// metadata.
    pub fn inner_instructions(&self) -> &[crate::engine::InnerInstruction] {
        &self.outcome.inner_instructions
    }

    /// Wall-clock execution time in microseconds. Useful for
    /// regression-tracking the cost of a Hopper program over
    /// time. Non-deterministic — depends on the host machine —
    /// so don't pin exact values, but stable enough to catch
    /// order-of-magnitude regressions.
    pub fn execution_time_us(&self) -> u64 {
        self.outcome.execution_time_us
    }

    /// Panic if the instruction's CPI count doesn't match
    /// `expected`. Mirrors `mollusk-svm`'s
    /// `Check::inner_instruction_count(N)` and `quasar-svm`'s
    /// equivalent assertion. Use to pin "this Hopper program
    /// makes exactly 2 CPIs (system create + token init)" in
    /// snapshot tests.
    pub fn assert_inner_instruction_count(&self, expected: usize) {
        let actual = self.outcome.inner_instructions.len();
        if actual != expected {
            panic!(
                "hopper-svm: expected {expected} inner instructions, got {actual}.\nrecorded:\n{}",
                self.format_inner_instructions(),
            );
        }
    }

    /// Format the inner-instruction list as a multi-line
    /// human-readable transcript. Used by
    /// [`assert_inner_instruction_count`] panic messages and
    /// available for ad-hoc debugging.
    pub fn format_inner_instructions(&self) -> String {
        if self.outcome.inner_instructions.is_empty() {
            return "  (none)".to_string();
        }
        self.outcome
            .inner_instructions
            .iter()
            .map(|i| {
                format!(
                    "  [depth {}] {} ({} accounts, {} bytes data)",
                    i.stack_height,
                    i.program_id,
                    i.accounts.len(),
                    i.data.len(),
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Look up a resulting account by address. Returns `None` if the
    /// account wasn't part of the instruction's account list.
    pub fn account(&self, addr: &Pubkey) -> Option<&KeyedAccount> {
        self.outcome
            .resulting_accounts
            .iter()
            .find(|a| &a.address == addr)
    }

    /// Print the program logs to stderr — handy for ad-hoc debugging
    /// when a test fails and you want the full trace immediately.
    pub fn print_logs(&self) {
        for line in &self.logs {
            eprintln!("{line}");
        }
    }

    /// Return all logs joined with newlines. Easier to compare in
    /// snapshot tests than the raw `Vec<String>`.
    pub fn all_logs(&self) -> String {
        self.logs.join("\n")
    }

    /// Panic with a clear message if the instruction did not succeed.
    /// Idiomatic in test bodies: `result.assert_success();`.
    pub fn assert_success(&self) {
        if let Some(err) = &self.outcome.error {
            panic!(
                "hopper-svm: expected success, got Err({}).\nlogs:\n{}",
                err.describe(),
                self.all_logs(),
            );
        }
    }

    /// Panic if the instruction did not fail with the expected
    /// error variant. Compares by `describe()` equality of the
    /// pre-formatted descriptions, so this is a structural match
    /// that's robust to addition of new fields on the error
    /// types but tight enough to discriminate variants.
    ///
    /// Mirrors `quasar-svm`'s `result.assert_error(ProgramError::InsufficientFunds)`
    /// with Hopper's structured error model.
    ///
    /// ```ignore
    /// result.assert_error(&HopperSvmError::OutOfComputeUnits {
    ///     consumed: 200_001,
    ///     limit: 200_000,
    /// });
    /// ```
    pub fn assert_error(&self, expected: &HopperSvmError) {
        match &self.outcome.error {
            None => panic!(
                "hopper-svm: expected error {}, got Ok(()).\nlogs:\n{}",
                expected.describe(),
                self.all_logs(),
            ),
            Some(actual) if actual.describe() == expected.describe() => {}
            Some(actual) => panic!(
                "hopper-svm: error mismatch.\n  expected: {}\n  got:      {}\nlogs:\n{}",
                expected.describe(),
                actual.describe(),
                self.all_logs(),
            ),
        }
    }

    /// Panic if the instruction did not fail with an error whose
    /// description contains `needle`. Substring matching keeps the
    /// assertion robust to error-formatting changes.
    pub fn assert_error_contains(&self, needle: &str) {
        match &self.outcome.error {
            None => panic!(
                "hopper-svm: expected error containing `{needle}`, got Ok(()).\nlogs:\n{}",
                self.all_logs()
            ),
            Some(err) => {
                let formatted = err.describe();
                if !formatted.contains(needle) {
                    panic!(
                        "hopper-svm: expected error containing `{needle}`, got `{formatted}`.\nlogs:\n{}",
                        self.all_logs()
                    );
                }
            }
        }
    }

    // -----------------------------------------------------------------
    // Hopper-aware decoders — the harness's first-class advantage
    // -----------------------------------------------------------------

    /// Read the 16-byte Hopper account header out of a resulting
    /// account by address. Returns `None` if the account is missing
    /// or its data is shorter than the header.
    ///
    /// The returned bytes match `hopper_runtime::layout::HopperHeader`'s
    /// wire shape: `[disc (4) | version (4) | layout_id (8)]`. Callers
    /// can run this through `hopper_runtime::layout::read_disc`,
    /// `read_version`, or `read_layout_id` for typed access.
    pub fn decode_header<'a>(&'a self, addr: &Pubkey) -> Option<&'a [u8]> {
        let acct = self.account(addr)?;
        if acct.data.len() < 16 {
            return None;
        }
        Some(&acct.data[..16])
    }

    /// Walk all resulting accounts and return any whose first 16
    /// bytes look like a valid Hopper header (non-zero discriminator,
    /// non-zero layout_id). Useful for "find all the Hopper accounts
    /// in this result" assertions in end-to-end tests.
    pub fn hopper_accounts(&self) -> Vec<&KeyedAccount> {
        self.outcome
            .resulting_accounts
            .iter()
            .filter(|a| {
                if a.data.len() < 16 {
                    return false;
                }
                let disc = u32::from_le_bytes([a.data[0], a.data[1], a.data[2], a.data[3]]);
                let layout_id = &a.data[8..16];
                disc != 0 && layout_id != [0u8; 8]
            })
            .collect()
    }

    /// Filter logs to only `Program log:` lines — strips runtime
    /// framing (`invoke`, `consumed`, `success`) so snapshot tests
    /// can assert on the program's own output without coupling to
    /// the framing format.
    pub fn decoded_logs(&self) -> Vec<&str> {
        self.logs
            .iter()
            .filter_map(|l| l.strip_prefix("Program log: "))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `decoded_logs` strips runtime framing and returns only the
    /// program-emitted lines. Pin against a known transcript.
    #[test]
    fn decoded_logs_strips_runtime_framing() {
        let outcome = ExecutionOutcome {
            resulting_accounts: vec![],
            compute_units_consumed: 10,
            return_data: vec![],
            inner_instructions: vec![],
            execution_time_us: 0,
            error: None,
        };
        let mut logs = LogCapture::default();
        let pid = Pubkey::new_unique();
        logs.invoke(&pid);
        logs.program_log("first");
        logs.program_log("second");
        logs.success(&pid, 10, 200_000);
        let result = HopperExecutionResult::from_outcome(outcome, logs);
        assert_eq!(result.decoded_logs(), vec!["first", "second"]);
    }

    /// `assert_success` should panic on a failed result and include
    /// the error description in the panic message.
    #[test]
    #[should_panic(expected = "Custom(7)")]
    fn assert_success_panics_with_error_description() {
        let outcome = ExecutionOutcome {
            resulting_accounts: vec![],
            compute_units_consumed: 0,
            return_data: vec![],
            inner_instructions: vec![],
            execution_time_us: 0,
            error: Some(HopperSvmError::Custom(7)),
        };
        let result = HopperExecutionResult::from_outcome(outcome, LogCapture::default());
        result.assert_success();
    }
}
