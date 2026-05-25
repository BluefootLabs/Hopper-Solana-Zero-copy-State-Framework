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
    // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
    unsafe {
        hopper_native::syscalls::sol_log_data(segments, segments_len);
    }

    #[cfg(all(target_os = "solana", feature = "legacy-pinocchio-compat"))]
    // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
    unsafe {
        pinocchio::syscalls::sol_log_data(segments, segments_len);
    }

    #[cfg(all(target_os = "solana", feature = "solana-program-backend"))]
    {
        // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
        let slices =
            unsafe { core::slice::from_raw_parts(segments as *const &[u8], segments_len as usize) };
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
    // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
    unsafe {
        hopper_native::syscalls::sol_sha256(vals, vals_len, result);
    }

    #[cfg(all(target_os = "solana", feature = "legacy-pinocchio-compat"))]
    // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
    unsafe {
        pinocchio::syscalls::sol_sha256(vals, vals_len, result);
    }

    #[cfg(all(target_os = "solana", feature = "solana-program-backend"))]
    {
        // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
        let slices =
            unsafe { core::slice::from_raw_parts(vals as *const &[u8], vals_len as usize) };
        let digest = ::solana_program::hash::hashv(slices).to_bytes();
        // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
        unsafe {
            core::ptr::copy_nonoverlapping(digest.as_ptr(), result, digest.len());
        }
    }

    #[cfg(not(target_os = "solana"))]
    {
        let _ = (vals, vals_len, result);
    }
}

/// Compute Keccak-256 over a slice-of-slices payload.
///
/// # Safety
///
/// `vals` must point to a valid array of slice descriptors and `result` must
/// point to writable storage for 32 output bytes.
#[inline(always)]
pub unsafe fn sol_keccak256(vals: *const u8, vals_len: u64, result: *mut u8) {
    #[cfg(all(target_os = "solana", feature = "hopper-native-backend"))]
    // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
    unsafe {
        hopper_native::syscalls::sol_keccak256(vals, vals_len, result);
    }

    #[cfg(all(target_os = "solana", feature = "legacy-pinocchio-compat"))]
    // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
    unsafe {
        pinocchio::syscalls::sol_keccak256(vals, vals_len, result);
    }

    #[cfg(all(target_os = "solana", feature = "solana-program-backend"))]
    {
        // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
        let slices =
            unsafe { core::slice::from_raw_parts(vals as *const &[u8], vals_len as usize) };
        let digest = ::solana_program::keccak::hashv(slices).to_bytes();
        // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
        unsafe {
            core::ptr::copy_nonoverlapping(digest.as_ptr(), result, digest.len());
        }
    }

    #[cfg(not(target_os = "solana"))]
    {
        let _ = (vals, vals_len, result);
    }
}

/// Validate whether a point lies on a runtime-supported curve.
///
/// Returns the runtime status code. For Solana curve-validation syscalls,
/// `0` means the point is valid for the selected curve.
///
/// # Safety
///
/// `point_addr` must point to a valid 32-byte encoded point. `result_point_addr`
/// is forwarded to the runtime syscall and may be null for validation-only use.
#[inline(always)]
pub unsafe fn sol_curve_validate_point(
    curve_id: u64,
    point_addr: *const u8,
    result_point_addr: *mut u8,
) -> u64 {
    #[cfg(all(target_os = "solana", feature = "hopper-native-backend"))]
    // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
    unsafe {
        return hopper_native::syscalls::sol_curve_validate_point(
            curve_id,
            point_addr,
            result_point_addr,
        );
    }

    #[cfg(all(target_os = "solana", feature = "legacy-pinocchio-compat"))]
    // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
    unsafe {
        return pinocchio::syscalls::sol_curve_validate_point(
            curve_id,
            point_addr,
            result_point_addr,
        );
    }

    #[cfg(all(target_os = "solana", feature = "solana-program-backend"))]
    // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
    unsafe {
        return ::solana_program::syscalls::sol_curve_validate_point(
            curve_id,
            point_addr,
            result_point_addr,
        );
    }

    #[cfg(not(target_os = "solana"))]
    {
        let _ = (curve_id, point_addr, result_point_addr);
        1
    }
}

/// Return the current Solana instruction stack height.
#[inline(always)]
pub fn sol_get_stack_height() -> u64 {
    #[cfg(all(target_os = "solana", feature = "hopper-native-backend"))]
    // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
    unsafe {
        return hopper_native::syscalls::sol_get_stack_height();
    }

    #[cfg(all(target_os = "solana", feature = "legacy-pinocchio-compat"))]
    // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
    unsafe {
        return pinocchio::syscalls::sol_get_stack_height();
    }

    #[cfg(all(target_os = "solana", feature = "solana-program-backend"))]
    // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
    unsafe {
        return ::solana_program::syscalls::sol_get_stack_height();
    }

    #[cfg(not(target_os = "solana"))]
    {
        1
    }
}

/// Read a previously processed sibling instruction from the current transaction.
///
/// # Safety
///
/// The output pointers must refer to writable buffers large enough for the
/// runtime to fill according to the processed-instruction syscall contract.
#[inline(always)]
pub unsafe fn sol_get_processed_sibling_instruction(
    index: u64,
    meta: *mut u8,
    program_id: *mut u8,
    data: *mut u8,
    accounts: *mut u8,
) -> u64 {
    #[cfg(all(target_os = "solana", feature = "hopper-native-backend"))]
    // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
    unsafe {
        return hopper_native::syscalls::sol_get_processed_sibling_instruction(
            index, meta, program_id, data, accounts,
        );
    }

    #[cfg(all(target_os = "solana", feature = "legacy-pinocchio-compat"))]
    // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
    unsafe {
        return pinocchio::syscalls::sol_get_processed_sibling_instruction(
            index, meta, program_id, data, accounts,
        );
    }

    #[cfg(all(target_os = "solana", feature = "solana-program-backend"))]
    // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
    unsafe {
        return ::solana_program::syscalls::sol_get_processed_sibling_instruction(
            index, meta, program_id, data, accounts,
        );
    }

    #[cfg(not(target_os = "solana"))]
    {
        let _ = (index, meta, program_id, data, accounts);
        1
    }
}
