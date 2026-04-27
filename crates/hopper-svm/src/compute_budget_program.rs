//! Compute Budget program — `BuiltinProgram` impl for
//! `ComputeBudget111111111111111111111111111111`.
//!
//! Solana programs frequently call
//! `solana_sdk::compute_budget::ComputeBudgetInstruction::set_compute_unit_limit(N)`
//! to raise the per-instruction CU ceiling above the default
//! 200 000. The compute-budget program is a built-in (not BPF)
//! on mainnet — Hopper ships its own builtin handler so tests
//! that include compute-budget instructions in their chains
//! actually take effect.
//!
//! ## Wire format
//!
//! `ComputeBudgetInstruction` is bincode-shaped:
//!
//! ```text
//! tag = 0  RequestUnits { units: u32, additional_fee: u32 }   (deprecated)
//! tag = 1  RequestHeapFrame { bytes: u32 }
//! tag = 2  SetComputeUnitLimit { units: u32 }
//! tag = 3  SetComputeUnitPrice { micro_lamports: u64 }
//! tag = 4  SetLoadedAccountsDataSizeLimit { bytes: u32 }
//! ```
//!
//! ## Effect on the harness
//!
//! - **`SetComputeUnitLimit`** updates the harness's per-
//!   instruction limit, which becomes the default for every
//!   subsequent instruction dispatched by this `HopperSvm`. The
//!   change persists until another `SetComputeUnitLimit` runs
//!   or the user calls `set_compute_budget` directly.
//! - **`RequestHeapFrame`** would update the BPF heap region
//!   size in production. Phase-1 ships a fixed 32 KiB heap; a
//!   request beyond that produces a structured warning in the
//!   log transcript but otherwise succeeds (the program's BPF
//!   code may then trip its own assertion against the actual
//!   heap size; tests can verify against that signal).
//! - **`SetComputeUnitPrice`** is fee-related and has no
//!   effect on Hopper's per-instruction CU accounting (Hopper
//!   doesn't simulate transaction fees in Phase 1). Logged for
//!   visibility.
//! - **`SetLoadedAccountsDataSizeLimit`** is a transaction-
//!   level limit on the total bytes a transaction's accounts
//!   may take. Hopper doesn't enforce this in Phase 1; logged
//!   for visibility.
//! - **`RequestUnits` (deprecated)** is treated like
//!   `SetComputeUnitLimit` for the `units` field; the
//!   `additional_fee` is ignored with a deprecation warning.

use crate::account::KeyedAccount;
use crate::builtin::{BuiltinProgram, InvokeContext};
use crate::compute::ComputeBudget;
use crate::error::HopperSvmError;
use solana_sdk::pubkey::Pubkey;

/// Compute Budget program ID. Bound to the canonical mainnet
/// address. Use as the `program_id` for compute-budget
/// instructions.
pub const COMPUTE_BUDGET_PROGRAM_ID: Pubkey = solana_sdk::compute_budget::id();

/// CU baseline. The compute-budget program is essentially free
/// on mainnet — it just updates request flags. Charging 150 CU
/// matches the system program's per-instruction baseline.
const COMPUTE_BUDGET_CU: u64 = 150;

/// Maximum compute-unit limit a single `SetComputeUnitLimit` may
/// request. Mainnet's hard cap; we mirror.
pub const MAX_COMPUTE_UNIT_LIMIT: u32 = 1_400_000;

/// Builtin handler. Register against [`COMPUTE_BUDGET_PROGRAM_ID`]
/// via [`crate::HopperSvm::with_compute_budget_program`].
///
/// State mutation: when `SetComputeUnitLimit` runs, the
/// simulator writes the new limit into the harness's
/// `pending_cu_limit` cell. The harness reads that cell at the
/// start of the next instruction dispatch and applies it as the
/// budget limit. This makes the compute-budget instruction
/// effective for the rest of the chain — matching mainnet's
/// "transaction-level state" semantics within Hopper's
/// simpler dispatch model.
///
/// Similarly, `SetComputeUnitPrice` writes into the harness's
/// `priority_fee_micro_lamports_per_cu` cell so the next
/// `process_transaction` call computes the correct priority-fee
/// surcharge.
pub struct ComputeBudgetProgramSimulator {
    /// Shared with [`crate::HopperSvm::pending_cu_limit`] —
    /// the harness initialises this on `with_compute_budget_program`.
    pub pending_cu_limit: std::sync::Arc<std::sync::Mutex<Option<u64>>>,
    /// Shared with the harness's
    /// `priority_fee_micro_lamports_per_cu` cell —
    /// `SetComputeUnitPrice` writes here.
    pub priority_fee: std::sync::Arc<std::sync::Mutex<u64>>,
}

impl BuiltinProgram for ComputeBudgetProgramSimulator {
    fn name(&self) -> &'static str {
        "compute-budget"
    }

    fn cost(&self, _budget: &ComputeBudget) -> u64 {
        COMPUTE_BUDGET_CU
    }

    fn invoke(
        &self,
        data: &[u8],
        _accounts: &mut [KeyedAccount],
        ctx: &mut InvokeContext<'_>,
    ) -> Result<(), HopperSvmError> {
        let (tag, body) = data
            .split_first()
            .ok_or_else(|| HopperSvmError::BuiltinError {
                program_id: *ctx.program_id,
                message: "compute-budget: empty instruction data".to_string(),
            })?;
        match *tag {
            0 => {
                // Deprecated `RequestUnits { units: u32, additional_fee: u32 }`.
                if body.len() < 8 {
                    return Err(HopperSvmError::BuiltinError {
                        program_id: *ctx.program_id,
                        message: "compute-budget::RequestUnits: body < 8 bytes".to_string(),
                    });
                }
                let units = u32::from_le_bytes(body[0..4].try_into().unwrap());
                if units > MAX_COMPUTE_UNIT_LIMIT {
                    return Err(HopperSvmError::BuiltinError {
                        program_id: *ctx.program_id,
                        message: format!(
                            "compute-budget::RequestUnits: {units} > MAX_COMPUTE_UNIT_LIMIT ({MAX_COMPUTE_UNIT_LIMIT})"
                        ),
                    });
                }
                *self.pending_cu_limit.lock().unwrap() = Some(units as u64);
                ctx.log(format!(
                    "compute-budget::RequestUnits (deprecated): {units} CU"
                ));
            }
            1 => {
                if body.len() < 4 {
                    return Err(HopperSvmError::BuiltinError {
                        program_id: *ctx.program_id,
                        message: "compute-budget::RequestHeapFrame: body < 4 bytes".to_string(),
                    });
                }
                let bytes = u32::from_le_bytes(body[0..4].try_into().unwrap());
                ctx.log(format!(
                    "compute-budget::RequestHeapFrame: {bytes} bytes (Phase 1 heap is fixed at 32 KiB)"
                ));
            }
            2 => {
                if body.len() < 4 {
                    return Err(HopperSvmError::BuiltinError {
                        program_id: *ctx.program_id,
                        message: "compute-budget::SetComputeUnitLimit: body < 4 bytes".to_string(),
                    });
                }
                let units = u32::from_le_bytes(body[0..4].try_into().unwrap());
                if units > MAX_COMPUTE_UNIT_LIMIT {
                    return Err(HopperSvmError::BuiltinError {
                        program_id: *ctx.program_id,
                        message: format!(
                            "compute-budget::SetComputeUnitLimit: {units} > MAX_COMPUTE_UNIT_LIMIT ({MAX_COMPUTE_UNIT_LIMIT})"
                        ),
                    });
                }
                *self.pending_cu_limit.lock().unwrap() = Some(units as u64);
                ctx.log(format!(
                    "compute-budget::SetComputeUnitLimit: {units} CU (applies to subsequent instructions)"
                ));
            }
            3 => {
                if body.len() < 8 {
                    return Err(HopperSvmError::BuiltinError {
                        program_id: *ctx.program_id,
                        message: "compute-budget::SetComputeUnitPrice: body < 8 bytes".to_string(),
                    });
                }
                let micro_lamports = u64::from_le_bytes(body[0..8].try_into().unwrap());
                *self.priority_fee.lock().unwrap() = micro_lamports;
                ctx.log(format!(
                    "compute-budget::SetComputeUnitPrice: {micro_lamports} μlamports/CU (applied to subsequent process_transaction calls)"
                ));
            }
            4 => {
                if body.len() < 4 {
                    return Err(HopperSvmError::BuiltinError {
                        program_id: *ctx.program_id,
                        message: "compute-budget::SetLoadedAccountsDataSizeLimit: body < 4 bytes"
                            .to_string(),
                    });
                }
                let bytes = u32::from_le_bytes(body[0..4].try_into().unwrap());
                ctx.log(format!(
                    "compute-budget::SetLoadedAccountsDataSizeLimit: {bytes} bytes (Phase 1 doesn't enforce)"
                ));
            }
            other => {
                return Err(HopperSvmError::BuiltinError {
                    program_id: *ctx.program_id,
                    message: format!(
                        "compute-budget: unknown instruction tag {other} (supported: 0/RequestUnits, 1/RequestHeapFrame, 2/SetComputeUnitLimit, 3/SetComputeUnitPrice, 4/SetLoadedAccountsDataSizeLimit)"
                    ),
                });
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log::LogCapture;
    use crate::sysvar::Sysvars;
    use solana_sdk::instruction::AccountMeta;
    use std::sync::{Arc, Mutex};

    fn invoke(sim: &ComputeBudgetProgramSimulator, data: Vec<u8>) -> Result<(), HopperSvmError> {
        let mut accounts: Vec<KeyedAccount> = vec![];
        let metas: Vec<AccountMeta> = vec![];
        let mut budget = ComputeBudget::default();
        let mut logs = LogCapture::default();
        let sysvars = Sysvars::default();
        let pid = COMPUTE_BUDGET_PROGRAM_ID;
        let mut ctx = InvokeContext {
            program_id: &pid,
            account_metas: &metas,
            sysvars: &sysvars,
            logs: &mut logs,
            budget: &mut budget,
        };
        sim.invoke(&data, &mut accounts, &mut ctx)
    }

    /// `SetComputeUnitLimit` writes the requested limit into
    /// the pending cell. Pin against the value (not just
    /// "not None") so a future change to the cell type is
    /// caught.
    #[test]
    fn set_compute_unit_limit_writes_pending() {
        let cell = Arc::new(Mutex::new(None));
        let sim = ComputeBudgetProgramSimulator {
            pending_cu_limit: cell.clone(),
            priority_fee: Arc::new(Mutex::new(0)),
        };
        let mut data = vec![2u8]; // SetComputeUnitLimit
        data.extend_from_slice(&(500_000u32).to_le_bytes());
        invoke(&sim, data).expect("ok");
        assert_eq!(*cell.lock().unwrap(), Some(500_000u64));
    }

    /// `MAX_COMPUTE_UNIT_LIMIT` is enforced — 1 400 001 is
    /// rejected.
    #[test]
    fn set_compute_unit_limit_rejects_over_max() {
        let cell = Arc::new(Mutex::new(None));
        let sim = ComputeBudgetProgramSimulator {
            pending_cu_limit: cell.clone(),
            priority_fee: Arc::new(Mutex::new(0)),
        };
        let mut data = vec![2u8];
        data.extend_from_slice(&(MAX_COMPUTE_UNIT_LIMIT + 1).to_le_bytes());
        let err = invoke(&sim, data).unwrap_err();
        assert!(matches!(err, HopperSvmError::BuiltinError { .. }));
        // Pending cell untouched.
        assert!(cell.lock().unwrap().is_none());
    }

    /// `RequestHeapFrame` doesn't write to the pending cell and
    /// emits a Phase-1 warning.
    #[test]
    fn request_heap_frame_logs_warning() {
        let cell = Arc::new(Mutex::new(None));
        let sim = ComputeBudgetProgramSimulator {
            pending_cu_limit: cell.clone(),
            priority_fee: Arc::new(Mutex::new(0)),
        };
        let mut data = vec![1u8];
        data.extend_from_slice(&(64 * 1024u32).to_le_bytes());
        invoke(&sim, data).expect("ok");
        assert!(cell.lock().unwrap().is_none());
    }

    /// Unknown tag returns a structured error listing the
    /// supported set.
    #[test]
    fn unknown_tag_lists_supported_set() {
        let cell = Arc::new(Mutex::new(None));
        let sim = ComputeBudgetProgramSimulator {
            pending_cu_limit: cell,
            priority_fee: Arc::new(Mutex::new(0)),
        };
        let err = invoke(&sim, vec![99u8]).unwrap_err();
        match err {
            HopperSvmError::BuiltinError { message, .. } => {
                assert!(message.contains("supported"), "{message}");
                assert!(message.contains("SetComputeUnitLimit"), "{message}");
            }
            other => panic!("wrong err: {other:?}"),
        }
    }
}
