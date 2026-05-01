//! Minimal syscall shims exposed through Hopper Runtime.
//!
//! Hopper-owned crates use this module instead of binding directly to backend
//! SDK syscall paths. That keeps backend differences inside Hopper Runtime.

/// Emit a `sol_log_data` event payload.
///
/// # Safety
///
/// `segments` must point to a valid array of slice descriptors for the active
/// backend ABI, and `segments_len` must match the number of entries.
#[inline(always)]
pub unsafe fn sol_log_data(segments: *const u8, segments_len: u64) {
    #[cfg(all(target_os = "solana", feature = "hopper-native-backend"))]
    unsafe {
        hopper_native::syscalls::sol_log_data(segments, segments_len);
    }

    #[cfg(all(target_os = "solana", feature = "legacy-pinocchio-compat"))]
    unsafe {
        pinocchio::syscalls::sol_log_data(segments, segments_len);
    }

    #[cfg(all(target_os = "solana", feature = "solana-program-backend"))]
    {
        let slices = unsafe {
            core::slice::from_raw_parts(segments as *const &[u8], segments_len as usize)
        };
        ::solana_program::log::sol_log_data(slices);
    }

    #[cfg(not(target_os = "solana"))]
    {
        let _ = (segments, segments_len);
    }
}

/// Compute SHA-256 over a slice-of-slices payload.
///
/// # Safety
///
/// `vals` must point to a valid array of slice descriptors and `result` must
/// point to writable storage for 32 output bytes.
#[inline(always)]
pub unsafe fn sol_sha256(vals: *const u8, vals_len: u64, result: *mut u8) {
    #[cfg(all(target_os = "solana", feature = "hopper-native-backend"))]
    unsafe {
        hopper_native::syscalls::sol_sha256(vals, vals_len, result);
    }

    #[cfg(all(target_os = "solana", feature = "legacy-pinocchio-compat"))]
    unsafe {
        pinocchio::syscalls::sol_sha256(vals, vals_len, result);
    }

    #[cfg(all(target_os = "solana", feature = "solana-program-backend"))]
    {
        let slices = unsafe {
            core::slice::from_raw_parts(vals as *const &[u8], vals_len as usize)
        };
        let digest = ::solana_program::hash::hashv(slices).to_bytes();
        unsafe {
            core::ptr::copy_nonoverlapping(digest.as_ptr(), result, digest.len());
        }
    }

    #[cfg(not(target_os = "solana"))]
    {
        let _ = (vals, vals_len, result);
    }
}