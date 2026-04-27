//! Cross-program account lenses -- read foreign fields by offset.
//!
//! When Program A wants to read a field from Program B's account, every
//! existing framework requires importing Program B's full type definition
//! at compile time. This creates tight coupling between programs.
//!
//! Hopper lenses solve this: read specific fields from foreign account
//! data by byte offset and type, no compile-time dependency required.
//! This enables composability patterns that were previously impossible
//! without shared crate dependencies.
//!
//! # Safety
//!
//! Lenses bypass type-level layout guarantees. The caller must know the
//! correct offset and type for the target field. Incorrect offsets will
//! read garbage data (but never cause UB, since all reads go through
//! bounds-checked accessors).
//!
//! # Usage
//!
//! ```ignore
//! use hopper_native::lens;
//!
//! // Read a 32-byte address at offset 10 from a foreign program's account
//! // (skip 10-byte Hopper header: disc + version + layout_id).
//! let authority = lens::read_address(oracle_account, 10)?;
//!
//! // Read a u64 price at offset 42.
//! let price = lens::read_le_u64(oracle_account, 42)?;
//!
//! // Read a typed struct at an offset.
//! let data: &MyPodType = lens::read_field::<MyPodType>(account, 10)?;
//! ```

use crate::account_view::AccountView;
use crate::address::Address;
use crate::error::ProgramError;
use crate::project::Projectable;

/// Read a `Projectable` field from account data at the given byte offset.
///
/// **Tier-C escape hatch** per the Hopper Safety Audit. `Projectable`
/// only requires `Copy + 'static`, which is too permissive to protect
/// against padding/alignment bugs. New code should prefer
/// [`read_field_pod`] which enforces the stronger [`crate::Pod`]
/// bound at the type level.
#[inline]
pub fn read_field<T: Projectable>(
    account: &AccountView,
    offset: usize,
) -> Result<&T, ProgramError> {
    crate::project::project::<T>(account, offset, None)
}

/// Read a `Pod` field from account data at the given byte offset.
///
/// This is the Safety-Audit-compliant lens: requires the substrate
/// [`crate::Pod`] bound, so the compiler rejects types with padding,
/// non-alignment-1 fields, or forbidden bit patterns at the call site.
/// Bounds and alignment are still checked at runtime, just as in the
/// generic [`read_field`] escape hatch.
///
/// Use this in cross-program readers that want the audit-grade
/// guarantee without dropping down to hand-written pointer arithmetic.
///
/// # Example
///
/// ```ignore
/// use hopper_native::{lens, wire::LeU64};
/// let counter: &LeU64 = lens::read_field_pod(foreign_account, 16)?;
/// ```
#[inline]
pub fn read_field_pod<T: crate::Pod>(
    account: &AccountView,
    offset: usize,
) -> Result<&T, ProgramError> {
    let data_len = account.data_len();
    let size = core::mem::size_of::<T>();
    let end = offset
        .checked_add(size)
        .ok_or(ProgramError::ArithmeticOverflow)?;
    if end > data_len {
        return Err(ProgramError::AccountDataTooSmall);
    }
    let ptr = unsafe { account.data_ptr().add(offset) };
    // SAFETY: T: Pod ⇒ align 1, every bit pattern valid, no padding.
    // Bounds and arithmetic overflow checked above. No alignment check
    // needed (Pod's align-1 obligation subsumes it).
    Ok(unsafe { &*(ptr as *const T) })
}

/// Read a 32-byte address from account data.
///
/// The most common cross-program read: check the authority, mint, owner,
/// or any other public key stored in a foreign account.
#[inline]
pub fn read_address(account: &AccountView, offset: usize) -> Result<&Address, ProgramError> {
    let data_len = account.data_len();
    if offset.checked_add(32).map_or(true, |end| end > data_len) {
        return Err(ProgramError::AccountDataTooSmall);
    }
    let ptr = unsafe { account.data_ptr().add(offset) };
    // SAFETY: Address is #[repr(transparent)] over [u8; 32].
    // Alignment 1, bounds checked above.
    Ok(unsafe { &*(ptr as *const Address) })
}

/// Read a little-endian u64 from account data.
///
/// Returns the value by copy (no alignment concerns). This is the
/// safest way to read a u64 from potentially unaligned account data --
/// no pointer cast, just a byte copy.
#[inline]
pub fn read_le_u64(account: &AccountView, offset: usize) -> Result<u64, ProgramError> {
    let data_len = account.data_len();
    if offset.checked_add(8).map_or(true, |end| end > data_len) {
        return Err(ProgramError::AccountDataTooSmall);
    }
    let ptr = unsafe { account.data_ptr().add(offset) };
    let mut bytes = [0u8; 8];
    unsafe {
        core::ptr::copy_nonoverlapping(ptr, bytes.as_mut_ptr(), 8);
    }
    Ok(u64::from_le_bytes(bytes))
}

/// Read a little-endian u32 from account data.
#[inline]
pub fn read_le_u32(account: &AccountView, offset: usize) -> Result<u32, ProgramError> {
    let data_len = account.data_len();
    if offset.checked_add(4).map_or(true, |end| end > data_len) {
        return Err(ProgramError::AccountDataTooSmall);
    }
    let ptr = unsafe { account.data_ptr().add(offset) };
    let mut bytes = [0u8; 4];
    unsafe {
        core::ptr::copy_nonoverlapping(ptr, bytes.as_mut_ptr(), 4);
    }
    Ok(u32::from_le_bytes(bytes))
}

/// Read a little-endian u16 from account data.
#[inline]
pub fn read_le_u16(account: &AccountView, offset: usize) -> Result<u16, ProgramError> {
    let data_len = account.data_len();
    if offset.checked_add(2).map_or(true, |end| end > data_len) {
        return Err(ProgramError::AccountDataTooSmall);
    }
    let ptr = unsafe { account.data_ptr().add(offset) };
    let mut bytes = [0u8; 2];
    unsafe {
        core::ptr::copy_nonoverlapping(ptr, bytes.as_mut_ptr(), 2);
    }
    Ok(u16::from_le_bytes(bytes))
}

/// Read a single byte from account data.
#[inline]
pub fn read_u8(account: &AccountView, offset: usize) -> Result<u8, ProgramError> {
    if offset >= account.data_len() {
        return Err(ProgramError::AccountDataTooSmall);
    }
    Ok(unsafe { *account.data_ptr().add(offset) })
}

/// Read a boolean from account data (0 = false, nonzero = true).
#[inline]
pub fn read_bool(account: &AccountView, offset: usize) -> Result<bool, ProgramError> {
    read_u8(account, offset).map(|b| b != 0)
}

/// Read a byte slice from account data.
///
/// Returns a reference to `len` bytes starting at `offset`.
/// Useful for reading variable-length fields when you know the layout.
#[inline]
pub fn read_bytes(account: &AccountView, offset: usize, len: usize) -> Result<&[u8], ProgramError> {
    let data_len = account.data_len();
    if offset.checked_add(len).map_or(true, |end| end > data_len) {
        return Err(ProgramError::AccountDataTooSmall);
    }
    let ptr = unsafe { account.data_ptr().add(offset) };
    Ok(unsafe { core::slice::from_raw_parts(ptr, len) })
}

/// Compare a field in account data against an expected value without copying.
///
/// Returns true if the `len` bytes at `offset` match `expected`.
/// Useful for checking discriminators or magic numbers in foreign accounts.
#[inline]
pub fn field_eq(
    account: &AccountView,
    offset: usize,
    expected: &[u8],
) -> Result<bool, ProgramError> {
    let actual = read_bytes(account, offset, expected.len())?;
    Ok(actual == expected)
}
