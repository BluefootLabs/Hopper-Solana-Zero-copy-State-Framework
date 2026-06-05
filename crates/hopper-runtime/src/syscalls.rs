//! Minimal syscall shims exposed through Hopper Runtime.
//!
//! Hopper-owned crates use this module instead of depending on external SDK
//! syscall paths.

#[cfg(target_os = "solana")]
extern "C" {
    #[link_name = "sol_set_return_data"]
    fn syscall_sol_set_return_data(data: *const u8, length: u64);

    #[link_name = "sol_get_return_data"]
    fn syscall_sol_get_return_data(data: *mut u8, length: u64, program_id: *mut u8) -> u64;

    #[link_name = "sol_remaining_compute_units"]
    fn syscall_sol_remaining_compute_units() -> u64;

    #[link_name = "sol_memcpy_"]
    fn syscall_sol_memcpy(dst: *mut u8, src: *const u8, n: u64);

    #[link_name = "sol_memmove_"]
    fn syscall_sol_memmove(dst: *mut u8, src: *const u8, n: u64);

    #[link_name = "sol_memcmp_"]
    fn syscall_sol_memcmp(left: *const u8, right: *const u8, n: u64, result: *mut i32);

    #[link_name = "sol_memset_"]
    fn syscall_sol_memset(dst: *mut u8, byte: u8, n: u64);
}

/// Emit a `sol_log_data` event payload.
///
/// # Safety
///
/// `segments` must point to a valid array of slice descriptors for the active
/// Hopper's direct runtime ABI, and `segments_len` must match the number of entries.
#[inline(always)]
pub unsafe fn sol_log_data(segments: *const u8, segments_len: u64) {
    #[cfg(target_os = "solana")]
    // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
    unsafe {
        hopper_native::syscalls::sol_log_data(segments, segments_len);
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
    #[cfg(target_os = "solana")]
    // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
    unsafe {
        hopper_native::syscalls::sol_sha256(vals, vals_len, result);
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
    #[cfg(target_os = "solana")]
    // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
    unsafe {
        hopper_native::syscalls::sol_keccak256(vals, vals_len, result);
    }

    #[cfg(not(target_os = "solana"))]
    {
        let _ = (vals, vals_len, result);
    }
}

/// Compute BLAKE3 over a slice-of-slices payload.
///
/// Returns the Solana runtime status code (`0` on success).
///
/// # Safety
///
/// `vals` must point to a valid array of slice descriptors and `result` must
/// point to writable storage for 32 output bytes.
#[inline(always)]
pub unsafe fn sol_blake3(vals: *const u8, vals_len: u64, result: *mut u8) -> u64 {
    #[cfg(target_os = "solana")]
    // SAFETY: Caller supplies the Solana hash syscall descriptors and output buffer.
    unsafe {
        return hopper_native::syscalls::sol_blake3(vals, vals_len, result);
    }

    #[cfg(not(target_os = "solana"))]
    {
        let _ = (vals, vals_len, result);
        0
    }
}

/// Recover a secp256k1 public key from a 32-byte message hash and 64-byte
/// compact signature.
///
/// Returns the Solana runtime status code (`0` on success).
///
/// # Safety
///
/// `hash` must point to 32 bytes, `signature` must point to 64 bytes, and
/// `result` must point to 64 writable bytes.
#[inline(always)]
pub unsafe fn sol_secp256k1_recover(
    hash: *const u8,
    recovery_id: u64,
    signature: *const u8,
    result: *mut u8,
) -> u64 {
    #[cfg(target_os = "solana")]
    // SAFETY: Caller supplies fixed-width secp256k1 recover syscall buffers.
    unsafe {
        return hopper_native::syscalls::sol_secp256k1_recover(
            hash,
            recovery_id,
            signature,
            result,
        );
    }

    #[cfg(not(target_os = "solana"))]
    {
        let _ = (hash, recovery_id, signature, result);
        1
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
    #[cfg(target_os = "solana")]
    // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
    unsafe {
        return hopper_native::syscalls::sol_curve_validate_point(
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

/// Run a group operation on a runtime-supported curve.
///
/// Returns the Solana runtime status code (`0` on success).
///
/// # Safety
///
/// `left_input_addr`, `right_input_addr`, and `result_point_addr` must point to
/// buffers matching the selected curve and group operation. For multiplication,
/// Solana expects the scalar as the left input and the point as the right input.
#[inline(always)]
pub unsafe fn sol_curve_group_op(
    curve_id: u64,
    group_op: u64,
    left_input_addr: *const u8,
    right_input_addr: *const u8,
    result_point_addr: *mut u8,
) -> u64 {
    #[cfg(target_os = "solana")]
    // SAFETY: Caller supplies curve syscall buffers matching the selected operation.
    unsafe {
        return hopper_native::syscalls::sol_curve_group_op(
            curve_id,
            group_op,
            left_input_addr,
            right_input_addr,
            result_point_addr,
        );
    }

    #[cfg(not(target_os = "solana"))]
    {
        let _ = (
            curve_id,
            group_op,
            left_input_addr,
            right_input_addr,
            result_point_addr,
        );
        1
    }
}

/// Run variable-length multiscalar multiplication on a runtime-supported curve.
///
/// Returns the Solana runtime status code (`0` on success).
///
/// # Safety
///
/// `scalars_addr` and `points_addr` must point to contiguous arrays of 32-byte
/// scalar and point encodings with exactly `points_len` entries. `result_point_addr`
/// must point to 32 writable output bytes.
#[inline(always)]
pub unsafe fn sol_curve_multiscalar_mul(
    curve_id: u64,
    scalars_addr: *const u8,
    points_addr: *const u8,
    points_len: u64,
    result_point_addr: *mut u8,
) -> u64 {
    #[cfg(target_os = "solana")]
    // SAFETY: Caller supplies curve syscall buffers matching the selected operation.
    unsafe {
        return hopper_native::syscalls::sol_curve_multiscalar_mul(
            curve_id,
            scalars_addr,
            points_addr,
            points_len,
            result_point_addr,
        );
    }

    #[cfg(not(target_os = "solana"))]
    {
        let _ = (
            curve_id,
            scalars_addr,
            points_addr,
            points_len,
            result_point_addr,
        );
        1
    }
}

/// Compute a Poseidon hash using Solana's runtime syscall.
///
/// Returns the Solana runtime status code (`0` on success).
///
/// # Safety
///
/// `vals` must point to a valid array of slice descriptors and `hash_result`
/// must point to 32 writable output bytes.
#[inline(always)]
pub unsafe fn sol_poseidon(
    parameters: u64,
    endianness: u64,
    vals: *const u8,
    val_len: u64,
    hash_result: *mut u8,
) -> u64 {
    #[cfg(target_os = "solana")]
    // SAFETY: Caller supplies Poseidon slice descriptors and output buffer.
    unsafe {
        return hopper_native::syscalls::sol_poseidon(
            parameters,
            endianness,
            vals,
            val_len,
            hash_result,
        );
    }

    #[cfg(not(target_os = "solana"))]
    {
        let _ = (parameters, endianness, vals, val_len, hash_result);
        1
    }
}

/// Run a BN254 / alt_bn128 group operation.
///
/// Returns the Solana runtime status code (`0` on success).
///
/// # Safety
///
/// `input` must point to `input_size` bytes in the operation-specific encoding
/// and `result` must point to the operation-specific output buffer.
#[inline(always)]
pub unsafe fn sol_alt_bn128_group_op(
    group_op: u64,
    input: *const u8,
    input_size: u64,
    result: *mut u8,
) -> u64 {
    #[cfg(target_os = "solana")]
    // SAFETY: Caller supplies BN254 operation buffers matching `group_op`.
    unsafe {
        return hopper_native::syscalls::sol_alt_bn128_group_op(
            group_op, input, input_size, result,
        );
    }

    #[cfg(not(target_os = "solana"))]
    {
        let _ = (group_op, input, input_size, result);
        1
    }
}

/// Run a BN254 / alt_bn128 compression operation.
///
/// Returns the Solana runtime status code (`0` on success).
///
/// # Safety
///
/// `input` must point to `input_size` bytes in the operation-specific encoding
/// and `result` must point to the operation-specific output buffer.
#[inline(always)]
pub unsafe fn sol_alt_bn128_compression(
    op: u64,
    input: *const u8,
    input_size: u64,
    result: *mut u8,
) -> u64 {
    #[cfg(target_os = "solana")]
    // SAFETY: Caller supplies BN254 compression buffers matching `op`.
    unsafe {
        return hopper_native::syscalls::sol_alt_bn128_compression(op, input, input_size, result);
    }

    #[cfg(not(target_os = "solana"))]
    {
        let _ = (op, input, input_size, result);
        1
    }
}

/// Run big integer modular exponentiation.
///
/// Returns the Solana runtime status code (`0` on success).
///
/// # Safety
///
/// `params` must point to a `BigModExpParams`-compatible C layout and `result`
/// must point to writable storage with exactly the modulus length.
#[inline(always)]
pub unsafe fn sol_big_mod_exp(params: *const u8, result: *mut u8) -> u64 {
    #[cfg(target_os = "solana")]
    // SAFETY: Caller supplies a valid big-mod-exp parameter block and output buffer.
    unsafe {
        return hopper_native::syscalls::sol_big_mod_exp(params, result);
    }

    #[cfg(not(target_os = "solana"))]
    {
        let _ = (params, result);
        1
    }
}

/// Return the current Solana instruction stack height.
#[inline(always)]
pub fn sol_get_stack_height() -> u64 {
    #[cfg(target_os = "solana")]
    // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
    unsafe {
        return hopper_native::syscalls::sol_get_stack_height();
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
    #[cfg(target_os = "solana")]
    // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
    unsafe {
        return hopper_native::syscalls::sol_get_processed_sibling_instruction(
            index, meta, program_id, data, accounts,
        );
    }

    #[cfg(not(target_os = "solana"))]
    {
        let _ = (index, meta, program_id, data, accounts);
        1
    }
}

/// Set CPI return data for the current instruction.
///
/// # Safety
///
/// `data` must be valid for `length` bytes.
#[inline(always)]
pub unsafe fn sol_set_return_data(data: *const u8, length: u64) {
    #[cfg(target_os = "solana")]
    // SAFETY: Caller guarantees that `data` is valid for `length` bytes.
    unsafe {
        syscall_sol_set_return_data(data, length);
    }

    #[cfg(not(target_os = "solana"))]
    {
        let _ = (data, length);
    }
}

/// Read CPI return data set by the most recent CPI.
///
/// Returns the runtime-reported data length. If the return data is larger than
/// the provided buffer, only `length` bytes are copied into `data`.
///
/// # Safety
///
/// `data` must be valid for `length` writable bytes and `program_id` must point
/// to 32 writable bytes.
#[inline(always)]
pub unsafe fn sol_get_return_data(data: *mut u8, length: u64, program_id: *mut u8) -> u64 {
    #[cfg(target_os = "solana")]
    // SAFETY: Caller provides writable buffers for return bytes and program id.
    unsafe {
        return syscall_sol_get_return_data(data, length, program_id);
    }

    #[cfg(not(target_os = "solana"))]
    {
        let _ = (data, length, program_id);
        0
    }
}

/// Return the remaining compute units reported by the Solana runtime.
#[inline(always)]
pub fn sol_remaining_compute_units() -> u64 {
    #[cfg(target_os = "solana")]
    // SAFETY: Zero-argument Solana runtime syscall.
    unsafe {
        return syscall_sol_remaining_compute_units();
    }

    #[cfg(not(target_os = "solana"))]
    {
        u64::MAX
    }
}

/// Copy `n` non-overlapping bytes from `src` to `dst`.
///
/// # Safety
///
/// `src` and `dst` must be valid for `n` bytes and must not overlap.
#[inline(always)]
pub unsafe fn sol_memcpy_(dst: *mut u8, src: *const u8, n: u64) {
    #[cfg(target_os = "solana")]
    // SAFETY: Caller upholds the Solana memory syscall contract.
    unsafe {
        syscall_sol_memcpy(dst, src, n);
    }

    #[cfg(not(target_os = "solana"))]
    // SAFETY: Caller guarantees valid, non-overlapping ranges.
    unsafe {
        core::ptr::copy_nonoverlapping(src, dst, n as usize);
    }
}

/// Copy `n` bytes from `src` to `dst`, allowing overlap.
///
/// # Safety
///
/// `src` and `dst` must be valid for `n` bytes.
#[inline(always)]
pub unsafe fn sol_memmove_(dst: *mut u8, src: *const u8, n: u64) {
    #[cfg(target_os = "solana")]
    // SAFETY: Caller upholds the Solana memory syscall contract.
    unsafe {
        syscall_sol_memmove(dst, src, n);
    }

    #[cfg(not(target_os = "solana"))]
    // SAFETY: Caller guarantees valid ranges; `copy` allows overlap.
    unsafe {
        core::ptr::copy(src, dst, n as usize);
    }
}

/// Compare two byte ranges and write a negative, zero, or positive result.
///
/// # Safety
///
/// `left` and `right` must be valid for `n` bytes. `result` must be writable
/// for one `i32`.
#[inline(always)]
pub unsafe fn sol_memcmp_(left: *const u8, right: *const u8, n: u64, result: *mut i32) {
    #[cfg(target_os = "solana")]
    // SAFETY: Caller upholds the Solana memory syscall contract.
    unsafe {
        syscall_sol_memcmp(left, right, n, result);
    }

    #[cfg(not(target_os = "solana"))]
    // SAFETY: Caller guarantees valid ranges and writable result.
    unsafe {
        let left_bytes = core::slice::from_raw_parts(left, n as usize);
        let right_bytes = core::slice::from_raw_parts(right, n as usize);
        *result = match left_bytes.cmp(right_bytes) {
            core::cmp::Ordering::Less => -1,
            core::cmp::Ordering::Equal => 0,
            core::cmp::Ordering::Greater => 1,
        };
    }
}

/// Fill `n` bytes at `dst` with `byte`.
///
/// # Safety
///
/// `dst` must be valid for `n` writable bytes.
#[inline(always)]
pub unsafe fn sol_memset_(dst: *mut u8, byte: u8, n: u64) {
    #[cfg(target_os = "solana")]
    // SAFETY: Caller upholds the Solana memory syscall contract.
    unsafe {
        syscall_sol_memset(dst, byte, n);
    }

    #[cfg(not(target_os = "solana"))]
    // SAFETY: Caller guarantees `dst` is valid for `n` bytes.
    unsafe {
        core::ptr::write_bytes(dst, byte, n as usize);
    }
}
