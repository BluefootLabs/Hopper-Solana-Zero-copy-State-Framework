//! Hopper-owned memory helpers backed by Solana memory syscalls.
//!
//! These helpers keep raw SVM memory operations behind Hopper Runtime, while
//! the safe wrappers operate on Rust slices for normal program code.

use crate::{ProgramError, ProgramResult};

/// Copy bytes from `src` to `dst`. The regions must not overlap.
///
/// # Safety
///
/// `src` and `dst` must be valid for `len` bytes and must not overlap.
#[inline(always)]
pub unsafe fn copy_nonoverlapping(dst: *mut u8, src: *const u8, len: usize) {
    // SAFETY: Caller upholds the non-overlapping raw-memory contract.
    unsafe {
        crate::syscalls::sol_memcpy_(dst, src, len as u64);
    }
}

/// Copy bytes from `src` to `dst`, allowing overlap.
///
/// # Safety
///
/// `src` and `dst` must be valid for `len` bytes.
#[inline(always)]
pub unsafe fn copy(dst: *mut u8, src: *const u8, len: usize) {
    // SAFETY: Caller upholds the raw-memory contract; memmove allows overlap.
    unsafe {
        crate::syscalls::sol_memmove_(dst, src, len as u64);
    }
}

/// Fill a raw memory range with one byte.
///
/// # Safety
///
/// `dst` must be valid for `len` writable bytes.
#[inline(always)]
pub unsafe fn fill(dst: *mut u8, byte: u8, len: usize) {
    // SAFETY: Caller guarantees the destination range is writable.
    unsafe {
        crate::syscalls::sol_memset_(dst, byte, len as u64);
    }
}

/// Lexicographically compare two raw memory ranges.
///
/// # Safety
///
/// `left` and `right` must be valid for `len` bytes.
#[inline(always)]
pub unsafe fn compare(left: *const u8, right: *const u8, len: usize) -> core::cmp::Ordering {
    let mut result = 0i32;
    // SAFETY: Caller guarantees both ranges are readable and result is local.
    unsafe {
        crate::syscalls::sol_memcmp_(left, right, len as u64, &mut result as *mut i32);
    }
    match result {
        0 => core::cmp::Ordering::Equal,
        value if value < 0 => core::cmp::Ordering::Less,
        _ => core::cmp::Ordering::Greater,
    }
}

/// Copy `src` into the beginning of `dst`.
#[inline]
pub fn copy_bytes(dst: &mut [u8], src: &[u8]) -> ProgramResult {
    if dst.len() < src.len() {
        return Err(ProgramError::InvalidArgument);
    }
    if src.is_empty() {
        return Ok(());
    }
    // SAFETY: Slices are valid and distinct borrows, so they do not overlap.
    unsafe {
        copy_nonoverlapping(dst.as_mut_ptr(), src.as_ptr(), src.len());
    }
    Ok(())
}

/// Move a byte range inside one buffer, allowing overlap.
#[inline]
pub fn move_within(
    buffer: &mut [u8],
    src_start: usize,
    len: usize,
    dst_start: usize,
) -> ProgramResult {
    let src_end = src_start
        .checked_add(len)
        .ok_or(ProgramError::InvalidArgument)?;
    let dst_end = dst_start
        .checked_add(len)
        .ok_or(ProgramError::InvalidArgument)?;
    if src_end > buffer.len() || dst_end > buffer.len() {
        return Err(ProgramError::InvalidArgument);
    }
    if len == 0 || src_start == dst_start {
        return Ok(());
    }
    // SAFETY: Bounds are checked above; memmove supports overlap.
    unsafe {
        copy(
            buffer.as_mut_ptr().add(dst_start),
            buffer.as_ptr().add(src_start),
            len,
        );
    }
    Ok(())
}

/// Fill a byte slice with `byte`.
#[inline]
pub fn fill_bytes(buffer: &mut [u8], byte: u8) {
    if buffer.is_empty() {
        return;
    }
    // SAFETY: The mutable slice is valid for its full length.
    unsafe {
        fill(buffer.as_mut_ptr(), byte, buffer.len());
    }
}

/// Zero-fill a byte slice.
#[inline(always)]
pub fn zero_bytes(buffer: &mut [u8]) {
    fill_bytes(buffer, 0);
}

/// Compare two slices through Hopper's memory boundary.
#[inline]
pub fn compare_bytes(left: &[u8], right: &[u8]) -> core::cmp::Ordering {
    let prefix_len = core::cmp::min(left.len(), right.len());
    if prefix_len != 0 {
        // SAFETY: Slices are readable for `prefix_len` bytes.
        let prefix_order = unsafe { compare(left.as_ptr(), right.as_ptr(), prefix_len) };
        if prefix_order != core::cmp::Ordering::Equal {
            return prefix_order;
        }
    }
    left.len().cmp(&right.len())
}

/// Equality helper for byte slices.
#[inline]
pub fn bytes_eq(left: &[u8], right: &[u8]) -> bool {
    left.len() == right.len() && compare_bytes(left, right) == core::cmp::Ordering::Equal
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copy_bytes_copies_prefix() {
        let mut dst = [0u8; 5];
        copy_bytes(&mut dst, &[1, 2, 3]).unwrap();
        assert_eq!(dst, [1, 2, 3, 0, 0]);
    }

    #[test]
    fn move_within_allows_overlap() {
        let mut data = [1u8, 2, 3, 4, 5];
        move_within(&mut data, 0, 4, 1).unwrap();
        assert_eq!(data, [1, 1, 2, 3, 4]);
    }

    #[test]
    fn fill_and_compare_bytes() {
        let mut data = [9u8; 4];
        zero_bytes(&mut data);
        assert_eq!(data, [0u8; 4]);
        assert!(bytes_eq(&data, &[0, 0, 0, 0]));
        assert_eq!(compare_bytes(&[1, 2], &[1, 3]), core::cmp::Ordering::Less);
    }
}
