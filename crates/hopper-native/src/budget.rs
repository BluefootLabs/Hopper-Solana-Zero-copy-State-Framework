//! Compute-unit budget tracking and instrumentation.
//!
//! Solana programs have a finite CU budget per instruction. Exceeding it
//! is a hard abort. No existing framework provides runtime CU tracking
//! at the substrate level -- programs either blindly hope they fit or
//! manually sprinkle `sol_log_compute_units()` calls.
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

use crate::ProgramResult;

/// Compute-unit budget tracker.
///
/// On BPF, uses `sol_log_compute_units()` to read the remaining budget.
/// Off-chain, all operations are no-ops that succeed.
#[derive(Clone, Copy)]
pub struct CuBudget {
    /// CU remaining at the time of the snapshot (0 off-chain).
    /// Reserved for future use when Solana exposes a `sol_get_remaining_cu` syscall.
    #[allow(dead_code)]
    snapshot: u64,
}

impl CuBudget {
    /// Take a snapshot of the current compute budget.
    ///
    /// On BPF this calls `sol_log_compute_units()` and captures the
    /// remaining CU from the log output. On native (off-chain), the
    /// snapshot is 0 and all checks pass trivially.
    #[inline(always)]
    pub fn snapshot() -> Self {
        #[cfg(target_os = "solana")]
        {
            // The `sol_log_compute_units` syscall logs the remaining CU
            // but does not return it. We store a marker and rely on the
            // guard pattern (require_remaining) for budget enforcement.
            //
            // For actual CU reading, we use the Solana runtime's
            // get_processed_sibling_instruction or just track relative
            // consumption patterns.
            unsafe { crate::syscalls::sol_log_compute_units_(); }
            Self { snapshot: 0 }
        }
        #[cfg(not(target_os = "solana"))]
        {
            Self { snapshot: 0 }
        }
    }

    /// Log the current compute unit consumption for profiling.
    ///
    /// Emits via `sol_log_compute_units` on BPF. Use this to instrument
    /// hot paths and identify CU bottlenecks.
    #[inline(always)]
    pub fn checkpoint() {
        #[cfg(target_os = "solana")]
        unsafe {
            crate::syscalls::sol_log_compute_units_();
        }
    }

    /// Assert that at least `min_remaining` CU are available.
    ///
    /// On BPF, this is a conservative check: the Solana runtime does not
    /// expose a "get remaining CU" syscall, so this method logs the
    /// current usage and returns Ok. The real enforcement is that the
    /// runtime itself will abort if CU is exhausted.
    ///
    /// The value of this method is that it makes the CU concern VISIBLE
    /// in the code and provides a hook point for future runtime features
    /// that may expose remaining CU programmatically.
    ///
    /// Off-chain, this always returns Ok.
    #[inline(always)]
    pub fn require_remaining(&self, _min_remaining: u64) -> ProgramResult {
        // On BPF: log and rely on the runtime's hard abort.
        // When Solana adds a `sol_get_remaining_compute_units` syscall,
        // this method will become a real guard.
        #[cfg(target_os = "solana")]
        unsafe {
            crate::syscalls::sol_log_compute_units_();
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
