//! Small Hopper-owned wrappers for individual runtime syscalls used directly by
//! framework crates.

/// Emit the current compute-unit counter.
#[inline(always)]
pub fn sol_log_compute_units() {
    #[cfg(target_os = "solana")]
    // SAFETY: The syscall takes no pointer and has no memory precondition.
    unsafe {
        hopper_native::syscalls::sol_log_compute_units_();
    }
}
