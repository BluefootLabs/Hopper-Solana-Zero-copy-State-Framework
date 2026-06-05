//! Small compatibility shims for individual runtime syscalls used directly by
//! Hopper-owned crates.

/// Emit the current compute-unit counter.
#[inline(always)]
pub fn sol_log_compute_units() {
    #[cfg(target_os = "solana")]
    // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
    unsafe {
        hopper_native::syscalls::sol_log_compute_units_();
    }
}
