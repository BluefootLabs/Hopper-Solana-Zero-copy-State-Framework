//! Compute-unit budget tracking and instrumentation.
//!
//! Solana programs have a finite CU budget per instruction. Exceeding it
//! is a hard abort. Hopper provides runtime CU tracking at the substrate
//! level so programs can avoid scattering ad hoc `sol_log_compute_units()`
//! calls through business logic.
//!
//! Hopper's `CuBudget` provides:
//!
//! 1. **Snapshot/check pattern**: Take a CU snapshot, do work, check how
//!    much was consumed. Useful for profiling individual code paths.
//!
//! 2. **Guard pattern**: Set a CU floor and periodically check that you
//!    have enough budget remaining before expensive operations (like CPI).
//!
//! 3. **Feature-gated tracing**: With `#[cfg(feature = "cu-trace")]`,
//!    emit structured CU consumption logs at function boundaries that
//!    off-chain tools can parse into flame graphs.
//!
//! # Usage
//!
//! ```ignore
//! use hopper_native::budget::CuBudget;
//!
//! fn process(accounts: &[AccountView], data: &[u8]) -> ProgramResult {
//!     let budget = CuBudget::snapshot();
//!
//!     // ... do work ...
//!
//!     // Before an expensive CPI, check we have at least 50k CU left.
//!     budget.require_remaining(50_000)?;
//!
//!     // CPI call...
//!     Ok(())
//! }
//! ```
//!
//! With `cu-trace` enabled:
//!
//! ```ignore
//! use hopper_native::budget::cu_trace;
//!
//! fn process_deposit(/* ... */) -> ProgramResult {
//!     cu_trace!("deposit::start");
//!     // ... work ...
//!     cu_trace!("deposit::after_validation");
//!     // ... CPI ...
//!     cu_trace!("deposit::end");
//!     Ok(())
//! }
//! ```

use crate::{ProgramError, ProgramResult};

/// Custom error code returned by [`CuBudget::require_remaining`] when the
/// remaining compute budget is below the requested floor.
///
/// Substrate-coded like the other Hopper refusal ranges (`0xC000 | i`
/// context acquisition, `0xD000 | i` write policy); programs should keep
/// their own custom codes out of `0xE000..0xF000`.
pub const ERR_INSUFFICIENT_CU: u32 = 0xE000;

/// Compute-unit budget tracker.
///
/// On BPF this is backed by the `sol_remaining_compute_units` syscall
/// (SIMD-0049), so snapshots and guards read the *real* remaining budget.
/// Off-chain there is no CU metering: [`remaining`](Self::remaining)
/// reports `u64::MAX`, so every guard passes trivially and
/// [`used`](Self::used) reports 0.
#[derive(Clone, Copy)]
pub struct CuBudget {
    /// Remaining CU at the moment [`snapshot`](Self::snapshot) was taken
    /// (`u64::MAX` off-chain, where nothing is metered).
    snapshot: u64,
}

impl CuBudget {
    /// Remaining compute units for the current invocation.
    ///
    /// On BPF, reads the `sol_remaining_compute_units` syscall
    /// (SIMD-0049; live on all current clusters). Off-chain, returns
    /// `u64::MAX` — host builds have no CU meter, so guards built on
    /// this pass trivially, matching the crate's other host fallbacks.
    #[inline(always)]
    pub fn remaining() -> u64 {
        #[cfg(target_os = "solana")]
        {
            // SAFETY: nullary syscall with no memory arguments; the
            // runtime returns the invocation's remaining CU by value, so
            // there are no pointer, layout, or aliasing obligations.
            unsafe { crate::syscalls::sol_remaining_compute_units() }
        }
        #[cfg(not(target_os = "solana"))]
        {
            u64::MAX
        }
    }

    /// Take a snapshot of the current compute budget.
    ///
    /// Stores the remaining CU at call time (via
    /// [`remaining`](Self::remaining)), enabling the snapshot/check
    /// pattern: `let b = CuBudget::snapshot(); ...; b.used()`.
    ///
    /// Note: this no longer emits a `sol_log_compute_units` log line the
    /// way pre-SIMD-0049 versions did — it *reads* instead. Use
    /// [`checkpoint`](Self::checkpoint) for the logging behavior.
    #[inline(always)]
    pub fn snapshot() -> Self {
        Self {
            snapshot: Self::remaining(),
        }
    }

    /// Compute units consumed since this snapshot was taken.
    ///
    /// Saturates at 0 (the remaining budget can only decrease within an
    /// invocation, but saturating keeps hostile or host-side inputs from
    /// wrapping). Off-chain this is always 0.
    #[inline(always)]
    pub fn used(&self) -> u64 {
        self.snapshot.saturating_sub(Self::remaining())
    }

    /// Log the current compute unit consumption for profiling.
    ///
    /// Emits via `sol_log_compute_units` on BPF. Use this to instrument
    /// hot paths and identify CU bottlenecks.
    #[inline(always)]
    pub fn checkpoint() {
        #[cfg(target_os = "solana")]
        // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
        unsafe {
            crate::syscalls::sol_log_compute_units_();
        }
    }

    /// Assert that at least `min_remaining` CU are available.
    ///
    /// On BPF this is a **real guard** (SIMD-0049): it reads the remaining
    /// budget and returns `Err(ProgramError::Custom(`[`ERR_INSUFFICIENT_CU`]`))`
    /// when it is below the floor, letting the program fail cleanly (state
    /// untouched, clear error) instead of hard-aborting mid-CPI when the
    /// runtime exhausts the meter.
    ///
    /// Off-chain, [`remaining`](Self::remaining) is `u64::MAX`, so this
    /// always returns Ok.
    #[inline(always)]
    pub fn require_remaining(&self, min_remaining: u64) -> ProgramResult {
        Self::floor_check(Self::remaining(), min_remaining)
    }

    /// Pure comparison core of [`require_remaining`](Self::require_remaining),
    /// factored out so the reject branch is host-testable (host builds can
    /// never observe a low `remaining()`).
    #[inline(always)]
    fn floor_check(remaining: u64, min_remaining: u64) -> ProgramResult {
        if remaining < min_remaining {
            return Err(ProgramError::Custom(ERR_INSUFFICIENT_CU));
        }
        Ok(())
    }

    /// Log CU consumed since the snapshot.
    ///
    /// Emits a structured log that off-chain tools can parse.
    /// Format: `"cu-delta: <label>"`
    #[inline(always)]
    pub fn log_delta(&self, label: &str) {
        Self::checkpoint();
        crate::log::log(label);
    }
}

/// Structured CU tracing macro for profiling.
///
/// When the `cu-trace` feature is enabled, emits both a compute-unit
/// log and a label log, allowing off-chain tooling to reconstruct
/// a CU flame graph from program logs.
///
/// When `cu-trace` is NOT enabled, this is a complete no-op with zero
/// CU cost.
///
/// # Usage
///
/// ```ignore
/// cu_trace!("validate_accounts");
/// // ... validation code ...
/// cu_trace!("begin_cpi");
/// ```
#[macro_export]
macro_rules! cu_trace {
    ( $label:expr ) => {{
        #[cfg(feature = "cu-trace")]
        {
            $crate::budget::CuBudget::checkpoint();
            $crate::log::log(concat!("[cu-trace] ", $label));
        }
    }};
}

/// Run a closure and log the CU consumed by it (feature-gated).
///
/// Returns the closure's result. When `cu-trace` is not enabled,
/// just runs the closure with zero overhead.
///
/// # Usage
///
/// ```ignore
/// let result = cu_measure!("deserialize", || {
///     parse_instruction_data(data)
/// });
/// ```
#[macro_export]
macro_rules! cu_measure {
    ( $label:expr, $body:expr ) => {{
        #[cfg(feature = "cu-trace")]
        {
            $crate::budget::CuBudget::checkpoint();
            $crate::log::log(concat!("[cu-start] ", $label));
        }
        let __result = $body;
        #[cfg(feature = "cu-trace")]
        {
            $crate::budget::CuBudget::checkpoint();
            $crate::log::log(concat!("[cu-end] ", $label));
        }
        __result
    }};
}

#[cfg(test)]
mod tests {
    use super::*;

    // Host builds have no CU meter, so the on-chain read path cannot be
    // exercised here; these pin the host contract (guards pass trivially)
    // and the pure reject/accept core the on-chain path routes through.

    #[test]
    fn host_remaining_is_unmetered_max() {
        assert_eq!(CuBudget::remaining(), u64::MAX);
    }

    #[test]
    fn host_snapshot_reports_zero_used_and_passes_guards() {
        let budget = CuBudget::snapshot();
        assert_eq!(budget.used(), 0);
        assert!(budget.require_remaining(u64::MAX).is_ok());
    }

    #[test]
    fn floor_check_rejects_below_floor_with_coded_error() {
        assert_eq!(
            CuBudget::floor_check(49_999, 50_000),
            Err(ProgramError::Custom(ERR_INSUFFICIENT_CU))
        );
        assert_eq!(ERR_INSUFFICIENT_CU, 0xE000);
    }

    #[test]
    fn floor_check_accepts_at_and_above_floor() {
        assert!(CuBudget::floor_check(50_000, 50_000).is_ok());
        assert!(CuBudget::floor_check(u64::MAX, 0).is_ok());
        assert!(CuBudget::floor_check(0, 0).is_ok());
    }
}
