//! Small compatibility shims for individual runtime syscalls used directly by
//! Hopper-owned crates.

/// Emit the current compute-unit counter.
#[inline(always)]
pub fn sol_log_compute_units() {
    #[cfg(all(target_os = "solana", feature = "hopper-native-backend"))]
    unsafe {
        hopper_native::syscalls::sol_log_compute_units_();
    }

    #[cfg(all(target_os = "solana", feature = "legacy-pinocchio-compat"))]
    unsafe {
        pinocchio::syscalls::sol_log_compute_units_();
    }

    #[cfg(all(target_os = "solana", feature = "solana-program-backend"))]
    {
        ::solana_program::log::sol_log_compute_units();
    }
}