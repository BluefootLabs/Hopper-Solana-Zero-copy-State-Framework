//! Execution engine seam.
//!
//! The harness is engine-agnostic: it asks an [`Engine`] to execute
//! an instruction against a slice of accounts and a context, and
//! the engine returns an [`ExecutionOutcome`]. Phase 1 ships one
//! engine, the [`BuiltinEngine`], which dispatches into a registered
//! [`BuiltinProgram`]. Phase 2 will ship a `BpfEngine` that wraps
//! [`solana-sbpf`] for real `.so` execution; the harness will
//! fall through to it when no built-in is registered for the
//! program ID.
//!
//! Why a trait? Because the seam is the only place "where does
//! execution happen" is decided. Keeping it isolated means Phase 2
//! lands as one new file (`bpf_engine.rs`), one new variant in the
//! dispatch chain, and zero churn in any other module.
//!
//! [`solana-sbpf`]: https://crates.io/crates/solana-sbpf

use crate::account::KeyedAccount;
use crate::builtin::{BuiltinProgram, InvokeContext};
use crate::compute::ComputeBudget;
use crate::error::HopperSvmError;
use crate::log::LogCapture;
use crate::sysvar::Sysvars;
use solana_sdk::instruction::Instruction;

/// Outcome of executing one instruction. Distinct from the
/// public-facing [`crate::HopperExecutionResult`] — that type wraps
/// `ExecutionOutcome` and adds log-buffer ownership + assertion
/// helpers.
#[derive(Debug, Clone)]
pub struct ExecutionOutcome {
    /// Account state after execution. Includes every account that
    /// was passed in, mutated or not, plus any newly-created
    /// accounts.
    pub resulting_accounts: Vec<KeyedAccount>,
    /// CUs the built-in (or BPF program) charged.
    pub compute_units_consumed: u64,
    /// Program return data. Empty when the program didn't call
    /// `set_return_data` / its built-in equivalent.
    pub return_data: Vec<u8>,
    /// Cross-Program Invocations recorded during this execution,
    /// in dispatch order. Each entry captures the inner program
    /// ID, account metas, instruction data, and the stack height
    /// at which the CPI ran (1 = outermost, 2 = first-level CPI,
    /// 3 = nested, …). Useful for snapshot tests that count or
    /// pattern-match the calls a Hopper program makes. Empty
    /// when the instruction didn't issue any CPIs.
    pub inner_instructions: Vec<InnerInstruction>,
    /// Wall-clock execution time in microseconds. Measured from
    /// dispatch start to outcome construction; useful for
    /// regression-tracking the cost of a Hopper program over
    /// time. The number is non-deterministic — it depends on
    /// the host machine — so don't pin exact values in tests,
    /// but it's stable enough to catch order-of-magnitude
    /// regressions. Mirrors `quasar-svm`'s
    /// `ExecutionResult.execution_time_us`.
    pub execution_time_us: u64,
    /// `None` on success, `Some(err)` on failure.
    pub error: Option<HopperSvmError>,
}

/// One Cross-Program Invocation recorded during execution.
/// Mirrors the shape `solana-program-runtime` records on
/// mainnet for the `inner_instructions` slice in transaction
/// metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InnerInstruction {
    /// Program ID the CPI invoked.
    pub program_id: solana_sdk::pubkey::Pubkey,
    /// Account metas (pubkey + signer/writable flags) the CPI
    /// passed.
    pub accounts: Vec<solana_sdk::instruction::AccountMeta>,
    /// Instruction data bytes.
    pub data: Vec<u8>,
    /// Stack height at which the CPI ran. 1 = the outermost
    /// program; 2 = a CPI from the outermost; 3 = a CPI from a
    /// CPI; etc., capped at [`crate::bpf::context::MAX_CPI_DEPTH`].
    pub stack_height: u32,
}

/// An execution engine. The harness owns one or more engines and
/// dispatches each instruction to the first one that claims the
/// program ID. Phase 1 has one engine type; Phase 2 adds the BPF
/// engine.
pub trait Engine: Send + Sync {
    /// Execute one instruction. The engine is responsible for
    /// charging CUs, capturing logs, and returning the post-state.
    fn execute(
        &self,
        program: &dyn BuiltinProgram,
        ix: &Instruction,
        accounts: &[KeyedAccount],
        budget: &mut ComputeBudget,
        sysvars: &Sysvars,
        logs: &mut LogCapture,
    ) -> ExecutionOutcome;
}

/// Phase 1 built-in engine. Dispatches into a [`BuiltinProgram`]'s
/// `invoke()` method, manages the log framing (invoke / consumed /
/// success), charges the per-built-in CU cost up front, and
/// snapshots the resulting account state.
pub struct BuiltinEngine;

impl Engine for BuiltinEngine {
    fn execute(
        &self,
        program: &dyn BuiltinProgram,
        ix: &Instruction,
        accounts: &[KeyedAccount],
        budget: &mut ComputeBudget,
        sysvars: &Sysvars,
        logs: &mut LogCapture,
    ) -> ExecutionOutcome {
        budget.reset();
        logs.invoke(&ix.program_id);

        // Charge the built-in's CU cost up front. If the budget
        // can't cover it, fail without invoking — same as the
        // on-chain runtime, which rejects an instruction that
        // can't pay its baseline cost.
        let cost = program.cost(budget);
        if let Err(err) = budget.consume(cost) {
            logs.failure(&ix.program_id, budget.consumed(), budget.limit(), &err);
            return ExecutionOutcome {
                resulting_accounts: accounts.to_vec(),
                compute_units_consumed: budget.consumed(),
                return_data: Vec::new(),
                inner_instructions: Vec::new(),
                execution_time_us: 0,
                error: Some(err),
            };
        }

        // Resolve the instruction's account_metas against the
        // supplied account state. Built-ins receive accounts in
        // metas order, with any account not in the supplied list
        // surfaced as an UnknownAccount error.
        let mut working: Vec<KeyedAccount> = match resolve_accounts(ix, accounts) {
            Ok(v) => v,
            Err(err) => {
                logs.failure(&ix.program_id, budget.consumed(), budget.limit(), &err);
                return ExecutionOutcome {
                    resulting_accounts: accounts.to_vec(),
                    compute_units_consumed: budget.consumed(),
                    return_data: Vec::new(),
                    inner_instructions: Vec::new(),
                    execution_time_us: 0,
                    error: Some(err),
                };
            }
        };

        let result = {
            let mut ctx = InvokeContext {
                program_id: &ix.program_id,
                account_metas: &ix.accounts,
                sysvars,
                logs,
                budget,
            };
            program.invoke(&ix.data, &mut working, &mut ctx)
        };

        match result {
            Ok(()) => {
                logs.success(&ix.program_id, budget.consumed(), budget.limit());
                // Merge the working set back into the full account
                // list: replace any address that was touched, leave
                // the rest intact, and keep any newly-created
                // accounts that weren't in the input list.
                let merged = merge_accounts(accounts, &working);
                ExecutionOutcome {
                    resulting_accounts: merged,
                    compute_units_consumed: budget.consumed(),
                    return_data: Vec::new(),
                    inner_instructions: Vec::new(),
                    execution_time_us: 0,
                    error: None,
                }
            }
            Err(err) => {
                logs.failure(&ix.program_id, budget.consumed(), budget.limit(), &err);
                ExecutionOutcome {
                    // On failure, do NOT propagate partial mutations.
                    // The on-chain runtime rolls back account state on
                    // instruction failure; matching that here means
                    // tests can rely on "either the whole instruction
                    // ran or none of it did."
                    resulting_accounts: accounts.to_vec(),
                    compute_units_consumed: budget.consumed(),
                    return_data: Vec::new(),
                    inner_instructions: Vec::new(),
                    execution_time_us: 0,
                    error: Some(err),
                }
            }
        }
    }
}

/// Resolve the instruction's `account_metas` against the supplied
/// account state. Returns the accounts in metas order. Each meta's
/// pubkey must appear in `accounts` — duplicate metas (the same
/// pubkey passed twice) are honored by cloning, matching the runtime.
fn resolve_accounts(
    ix: &Instruction,
    accounts: &[KeyedAccount],
) -> Result<Vec<KeyedAccount>, HopperSvmError> {
    let mut out = Vec::with_capacity(ix.accounts.len());
    for meta in &ix.accounts {
        let acct = accounts
            .iter()
            .find(|a| a.address == meta.pubkey)
            .cloned()
            .ok_or(HopperSvmError::UnknownAccount(meta.pubkey))?;
        out.push(acct);
    }
    Ok(out)
}

/// Merge a built-in's mutated account view back into the harness's
/// full account list. Built-ins see only the accounts named in the
/// instruction; the rest carry through unchanged.
fn merge_accounts(original: &[KeyedAccount], working: &[KeyedAccount]) -> Vec<KeyedAccount> {
    // Start from the originals so any account the built-in didn't
    // touch carries through.
    let mut out: Vec<KeyedAccount> = original.to_vec();
    for w in working {
        match out.iter_mut().find(|a| a.address == w.address) {
            Some(slot) => *slot = w.clone(),
            None => out.push(w.clone()),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builtin::BuiltinProgram;
    use solana_sdk::instruction::AccountMeta;
    use solana_sdk::pubkey::Pubkey;

    /// A tiny built-in for engine tests — adds a constant byte to
    /// account 0's data on success.
    struct AddByte(u8);
    impl BuiltinProgram for AddByte {
        fn name(&self) -> &'static str {
            "AddByte"
        }
        fn cost(&self, _: &ComputeBudget) -> u64 {
            10
        }
        fn invoke(
            &self,
            _data: &[u8],
            accounts: &mut [KeyedAccount],
            ctx: &mut InvokeContext<'_>,
        ) -> Result<(), HopperSvmError> {
            ctx.log("running AddByte");
            accounts[0].data.push(self.0);
            Ok(())
        }
    }

    /// Successful built-in execution: account 0's data grows, log
    /// framing is correct, CUs charged at invoke time.
    #[test]
    fn builtin_engine_runs_and_charges_cu() {
        let pid = Pubkey::new_unique();
        let acct_addr = Pubkey::new_unique();
        let accounts = vec![KeyedAccount::new(
            acct_addr,
            1_000,
            Pubkey::new_unique(),
            vec![1, 2, 3],
            false,
        )];
        let ix = Instruction {
            program_id: pid,
            accounts: vec![AccountMeta::new(acct_addr, false)],
            data: vec![],
        };
        let mut budget = ComputeBudget::default();
        let sysvars = Sysvars::default();
        let mut logs = LogCapture::default();
        let outcome = BuiltinEngine.execute(
            &AddByte(0xAB),
            &ix,
            &accounts,
            &mut budget,
            &sysvars,
            &mut logs,
        );
        assert!(outcome.error.is_none(), "{:?}", outcome.error);
        assert_eq!(outcome.compute_units_consumed, 10);
        assert_eq!(
            outcome.resulting_accounts[0].data,
            vec![1, 2, 3, 0xAB],
            "built-in should have appended a byte"
        );
        // Log transcript: invoke, program log, consumed, success.
        let lines = logs.lines();
        assert!(lines[0].contains("invoke [1]"), "got {lines:?}");
        assert_eq!(lines[1], "Program log: running AddByte");
        assert!(lines[2].contains("consumed 10 of"), "got {lines:?}");
        assert!(lines[3].contains("success"), "got {lines:?}");
    }

    /// On failure, the engine must roll back mutations — the
    /// resulting account list equals the input list. Solana's
    /// runtime behaves this way; matching it here lets tests rely
    /// on all-or-nothing instruction effects.
    #[test]
    fn builtin_engine_rolls_back_state_on_failure() {
        struct AlwaysFail;
        impl BuiltinProgram for AlwaysFail {
            fn name(&self) -> &'static str {
                "AlwaysFail"
            }
            fn invoke(
                &self,
                _: &[u8],
                accounts: &mut [KeyedAccount],
                _: &mut InvokeContext<'_>,
            ) -> Result<(), HopperSvmError> {
                accounts[0].data.push(0xFF);
                Err(HopperSvmError::Custom(1))
            }
        }
        let pid = Pubkey::new_unique();
        let acct_addr = Pubkey::new_unique();
        let original = vec![KeyedAccount::new(
            acct_addr,
            1_000,
            Pubkey::new_unique(),
            vec![1, 2, 3],
            false,
        )];
        let ix = Instruction {
            program_id: pid,
            accounts: vec![AccountMeta::new(acct_addr, false)],
            data: vec![],
        };
        let mut budget = ComputeBudget::default();
        let sysvars = Sysvars::default();
        let mut logs = LogCapture::default();
        let outcome = BuiltinEngine.execute(
            &AlwaysFail,
            &ix,
            &original,
            &mut budget,
            &sysvars,
            &mut logs,
        );
        assert!(matches!(outcome.error, Some(HopperSvmError::Custom(1))));
        // Mutations rolled back.
        assert_eq!(outcome.resulting_accounts, original);
    }
}
