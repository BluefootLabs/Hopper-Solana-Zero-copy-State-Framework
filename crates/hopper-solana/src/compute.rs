//! Compute unit budget guards.
//!
//! Wraps `sol_remaining_compute_units()` so programs with variable work
//! (batch processing, multi-proof verification, iterative liquidations)
//! can bail early with a clean error instead of hitting "compute budget
//! exceeded" at some random instruction.

use hopper_runtime::error::ProgramError;

#[cfg(target_os = "solana")]
extern "C" {
    fn sol_remaining_compute_units() -> u64;
}

/// Read remaining CU available to this transaction.
///
/// Costs ~100 CU itself, so avoid calling it in tight inner loops.
#[inline(always)]
pub fn remaining_compute_units() -> u64 {
    #[cfg(target_os = "solana")]
    {
        // SAFETY: BPF syscall, always available on Solana runtime.
        unsafe { sol_remaining_compute_units() }
    }
    #[cfg(not(target_os = "solana"))]
    {
        u64::MAX // Off-chain: treat as unlimited
    }
}

/// Fail if fewer than `min_cu` compute units remain.
///
/// Put this at the top of expensive handlers or inside batch loops:
///
/// ```rust,ignore
/// for item in items.iter() {
///     check_compute_remaining(5_000)?;
///     process(item)?;
/// }
/// ```
#[inline(always)]
pub fn check_compute_remaining(min_cu: u64) -> Result<(), ProgramError> {
    if remaining_compute_units() < min_cu {
        return Err(ProgramError::InvalidArgument);
    }
    Ok(())
}

/// Like [`check_compute_remaining`] but also returns the actual remaining
/// count on success. Useful for adaptive code paths:
///
/// ```rust,ignore
/// let cu = require_compute_remaining(10_000)?;
/// if cu > 50_000 {
///     expensive_path()?;
/// } else {
///     cheap_path()?;
/// }
/// ```
#[inline(always)]
pub fn require_compute_remaining(min_cu: u64) -> Result<u64, ProgramError> {
    let remaining = remaining_compute_units();
    if remaining < min_cu {
        return Err(ProgramError::InvalidArgument);
    }
    Ok(remaining)
}
