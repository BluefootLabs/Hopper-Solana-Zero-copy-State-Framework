//! Compute budget metering.
//!
//! Phase 1: flat per-built-in cost charged at invoke time. The cost
//! is configurable per built-in via [`BuiltinProgram::cost`]; the
//! default is 150 CU which roughly matches the system program's
//! per-instruction baseline. Phase 2 adds per-eBPF-instruction
//! metering (1 CU per instruction, the standard Solana rate)
//! delegated to the engine.

use crate::error::HopperSvmError;

/// Default Solana compute-budget limit (200,000 CU per instruction).
/// Matches the on-chain default — tests that don't override get
/// the same budget the runtime gives them in production.
pub const DEFAULT_COMPUTE_LIMIT: u64 = 200_000;

/// Phase 1 default cost charged for each built-in invocation.
/// Consciously low so per-instruction tests don't trip the budget;
/// raise it in [`ComputeBudget::with_default_builtin_cost`] for
/// tighter accounting.
pub const DEFAULT_BUILTIN_COST: u64 = 150;

/// Compute meter — tracks how many CUs the current execution has
/// consumed and whether it is over the configured limit.
#[derive(Clone, Debug)]
pub struct ComputeBudget {
    /// CUs consumed since the meter was last reset.
    consumed: u64,
    /// CU ceiling. Execution aborts when `consumed > limit`.
    limit: u64,
    /// Default per-built-in cost — used when a [`crate::BuiltinProgram`]
    /// doesn't override [`crate::BuiltinProgram::cost`].
    default_builtin_cost: u64,
}

impl ComputeBudget {
    /// Build a fresh meter with the given limit and default cost.
    pub fn new(limit: u64, default_builtin_cost: u64) -> Self {
        Self {
            consumed: 0,
            limit,
            default_builtin_cost,
        }
    }

    /// Reset the consumed counter — called by the engine at the start
    /// of every instruction so each instruction sees a fresh budget
    /// of `limit` CUs (matching Solana's per-instruction accounting).
    pub fn reset(&mut self) {
        self.consumed = 0;
    }

    /// Override the per-instruction limit.
    pub fn set_limit(&mut self, limit: u64) {
        self.limit = limit;
    }

    /// Read the current limit.
    pub fn limit(&self) -> u64 {
        self.limit
    }

    /// Read the consumed CUs since the last reset.
    pub fn consumed(&self) -> u64 {
        self.consumed
    }

    /// Read the configured default built-in cost.
    pub fn default_builtin_cost(&self) -> u64 {
        self.default_builtin_cost
    }

    /// Override the default built-in cost. Useful for tests that
    /// want tight CU accounting against a specific built-in budget.
    pub fn with_default_builtin_cost(mut self, cost: u64) -> Self {
        self.default_builtin_cost = cost;
        self
    }

    /// Charge `units` CUs against the meter. Returns
    /// [`HopperSvmError::OutOfComputeUnits`] if the new total would
    /// exceed the limit. The meter does NOT roll back on failure —
    /// once charged, the CU is gone, matching how the on-chain
    /// runtime behaves.
    pub fn consume(&mut self, units: u64) -> Result<(), HopperSvmError> {
        // Charge first, then check. A program that would exceed the
        // budget gets the charge applied (so `consumed > limit` is
        // observable), and any subsequent `consume(0)` re-fails. This
        // matches mainnet's "out of compute units" semantics where
        // the meter is poisoned once the budget is breached.
        let new = self.consumed.saturating_add(units);
        self.consumed = new;
        if new > self.limit {
            return Err(HopperSvmError::OutOfComputeUnits {
                consumed: new,
                limit: self.limit,
            });
        }
        Ok(())
    }
}

impl Default for ComputeBudget {
    fn default() -> Self {
        Self::new(DEFAULT_COMPUTE_LIMIT, DEFAULT_BUILTIN_COST)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Charging up to the limit must succeed; one CU past must trip
    /// the OutOfComputeUnits error and leave the consumed counter
    /// at the would-be total (no roll-back).
    #[test]
    fn consume_up_to_limit_then_one_more() {
        let mut b = ComputeBudget::new(100, 10);
        assert!(b.consume(60).is_ok());
        assert!(b.consume(40).is_ok());
        assert_eq!(b.consumed(), 100);
        match b.consume(1).unwrap_err() {
            HopperSvmError::OutOfComputeUnits { consumed, limit } => {
                assert_eq!(consumed, 101);
                assert_eq!(limit, 100);
            }
            other => panic!("wrong error: {other:?}"),
        }
        // Failure leaves the consumed at the post-charge value, so a
        // subsequent attempt is also rejected.
        assert!(b.consume(0).is_err());
    }

    /// Reset zeroes the consumed counter.
    #[test]
    fn reset_clears_consumed() {
        let mut b = ComputeBudget::new(100, 10);
        b.consume(50).unwrap();
        b.reset();
        assert_eq!(b.consumed(), 0);
        assert!(b.consume(99).is_ok());
    }
}
