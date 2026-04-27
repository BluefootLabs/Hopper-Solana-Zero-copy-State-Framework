//! SVM-optimized memory operations.
//!
//! The Solana BPF VM provides syscall-level memory operations that are
//! faster than Rust's default libc implementations because they are
//! JIT-compiled intrinsics in the VM. No framework exposes these at the
//! substrate level.
//!
//! On BPF, these dispatch to `sol_memcpy_`, `sol_memmove_`, `sol_memcmp_`,
//! and `sol_memset_`. Off-chain, they fall back to standard library
//! implementations.
//!
//! Use these instead of `core::ptr::copy_nonoverlapping`, `ptr::write_bytes`,
//! etc. for best performance on the SVM.

use crate::error::ProgramError;

/// Copy `n` bytes from `src` to `dst`.
///
/// The memory regions **must not overlap**. For overlapping copies, use
/// `memmove`. This is enforced by the SVM runtime on BPF.
///
/// # Safety
///
/// Both `src` and `dst` must be valid for `n` bytes. Regions must not overlap.
#[inline(always)]
pub unsafe fn memcpy(dst: *mut u8, src: *const u8, n: usize) {
    #[cfg(target_os = "solana")]
    unsafe {
        crate::syscalls::sol_memcpy_(dst, src, n as u64);
    }
    #[cfg(not(target_os = "solana"))]
    unsafe {
        core::ptr::copy_nonoverlapping(src, dst, n);
    }
}

/// Copy `n` bytes from `src` to `dst`, handling overlapping regions.
///
/// Safe for any src/dst alignment and overlap pattern.
///
/// # Safety
///
/// Both `src` and `dst` must be valid for `n` bytes.
#[inline(always)]
pub unsafe fn memmove(dst: *mut u8, src: *const u8, n: usize) {
    #[cfg(target_os = "solana")]
    unsafe {
        crate::syscalls::sol_memmove_(dst, src, n as u64);
    }
    #[cfg(not(target_os = "solana"))]
    unsafe {
        core::ptr::copy(src, dst, n);
    }
}

/// Fill `n` bytes starting at `dst` with `byte`.
///
/// The fastest way to zero-fill or pattern-fill a memory region on the SVM.
///
/// # Safety
///
/// `dst` must be valid for `n` bytes.
#[inline(always)]
pub unsafe fn memset(dst: *mut u8, byte: u8, n: usize) {
    #[cfg(target_os = "solana")]
    unsafe {
        crate::syscalls::sol_memset_(dst, byte, n as u64);
    }
    #[cfg(not(target_os = "solana"))]
    unsafe {
        core::ptr::write_bytes(dst, byte, n);
    }
}

/// Compare `n` bytes between two memory regions.
///
/// Returns `Ordering::Equal` if the regions are identical, or the
/// ordering of the first differing byte (lexicographic comparison).
///
/// # Safety
///
/// Both `a` and `b` must be valid for `n` bytes.
#[inline(always)]
pub unsafe fn memcmp(a: *const u8, b: *const u8, n: usize) -> core::cmp::Ordering {
    #[cfg(target_os = "solana")]
    {
        let mut result: i32 = 0;
        unsafe {
            crate::syscalls::sol_memcmp_(a, b, n as u64, &mut result as *mut i32);
        }
        match result {
            0 => core::cmp::Ordering::Equal,
            x if x < 0 => core::cmp::Ordering::Less,
            _ => core::cmp::Ordering::Greater,
        }
    }
    #[cfg(not(target_os = "solana"))]
    {
        let a_slice = unsafe { core::slice::from_raw_parts(a, n) };
        let b_slice = unsafe { core::slice::from_raw_parts(b, n) };
        a_slice.cmp(b_slice)
    }
}

// ---- Safe wrappers ---------------------------------------------------

/// Zero-fill a mutable byte slice using the SVM-optimized memset.
#[inline(always)]
pub fn zero_fill(buf: &mut [u8]) {
    if buf.is_empty() {
        return;
    }
    unsafe {
        memset(buf.as_mut_ptr(), 0, buf.len());
    }
}

/// Copy bytes from one slice to another (no overlap).
///
/// Returns `Err(InvalidArgument)` if lengths differ.
#[inline]
pub fn copy_bytes(dst: &mut [u8], src: &[u8]) -> Result<(), ProgramError> {
    if dst.len() < src.len() {
        return Err(ProgramError::InvalidArgument);
    }
    unsafe {
        memcpy(dst.as_mut_ptr(), src.as_ptr(), src.len());
    }
    Ok(())
}

/// Compare two byte slices for equality using SVM-optimized memcmp.
#[inline]
pub fn bytes_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    if a.is_empty() {
        return true;
    }
    unsafe { memcmp(a.as_ptr(), b.as_ptr(), a.len()) == core::cmp::Ordering::Equal }
}

/// Zero-fill account data using SVM-optimized memset.
///
/// More efficient than the byte-by-byte loop in `AccountView::close()`.
/// Use this when you need to clear account data without closing the account.
#[inline]
pub fn zero_account_data(account: &crate::account_view::AccountView) {
    let len = account.data_len();
    if len == 0 {
        return;
    }
    unsafe {
        memset(account.data_ptr(), 0, len);
    }
}
