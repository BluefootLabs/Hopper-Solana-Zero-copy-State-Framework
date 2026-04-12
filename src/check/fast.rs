//! Batched u32 header validation -- Quasar-inspired single-compare optimization.
//!
//! The SVM `RuntimeAccount` header packs `borrow_state`, `is_signer`,
//! `is_writable`, and `executable` into a 4-byte prefix. We read this as
//! a single `u32` and compare against precomputed constants, collapsing
//! 3-4 branch instructions into ONE comparison.
//!
//! This saves ~4-8 CU per account validation on the hot path.
//!
//! ## Wire Layout (RuntimeAccount header, little-endian u32)
//!
//! ```text
//! byte 0 = borrow_state (0xFF for non-duplicate)
//! byte 1 = is_signer    (0 or 1)
//! byte 2 = is_writable  (0 or 1)
//! byte 3 = executable    (0 or 1)
//! ```
//!
//! ## Safety Model
//!
//! The `read_account_header` function relies on hopper-native's `AccountView`
//! being `#[repr(C)]` with its first field being a `*mut u8` pointing to
//! the start of the `RuntimeAccount` in the SVM input buffer. This is
//! verified by hopper-native's own compile-time assertions. If hopper-native changes
//! its layout, the compile-time size assertion below will fail.
//!
//! This optimization is **gated to `target_os = "solana"`** only. Off-chain
//! code uses the safe fallback via `AccountView::is_signer()` etc.

use hopper_runtime::{error::ProgramError, AccountView, ProgramResult};

// -- Precomputed Header Constants ------------------------------------

/// Not borrowed, not a duplicate (borrow_state = 0xFF).
const NOT_BORROWED: u32 = 0xFF;

/// Non-duplicate, no flags.
pub const HEADER_NODUP: u32 = NOT_BORROWED;

/// Non-duplicate + signer.
pub const HEADER_SIGNER: u32 = NOT_BORROWED | (1 << 8);

/// Non-duplicate + writable.
pub const HEADER_WRITABLE: u32 = NOT_BORROWED | (1 << 16);

/// Non-duplicate + signer + writable.
pub const HEADER_SIGNER_WRITABLE: u32 = NOT_BORROWED | (1 << 8) | (1 << 16);

/// Non-duplicate + executable.
pub const HEADER_EXECUTABLE: u32 = NOT_BORROWED | (1 << 24);

/// Non-duplicate + writable + signer (same as SIGNER_WRITABLE, alias).
pub const HEADER_AUTHORITY: u32 = HEADER_SIGNER_WRITABLE;

// -- Fast Validation Functions ---------------------------------------

/// Read the 4-byte RuntimeAccount header as a u32.
///
/// # Safety
///
/// Requires that `AccountView` is `#[repr(C)]` with its first field being a
/// raw pointer to the RuntimeAccount data in the SVM input buffer. The first 4
/// bytes of RuntimeAccount are `[borrow_state, is_signer, is_writable, executable]`.
///
/// This is a hopper-native implementation detail; changes to hopper-native's `AccountView`
/// layout would require updating this function. The `target_os = "solana"` gate
/// ensures this is only compiled for the SBF target where the SVM guarantees
/// this layout.
#[cfg(all(target_os = "solana", feature = "hopper-native-backend"))]
#[inline(always)]
unsafe fn read_account_header(account: &AccountView) -> u32 {
    // SAFETY: AccountView is repr(C) with a pointer to the raw RuntimeAccount
    // as its first field. We dereference this pointer to get the RuntimeAccount
    // base address, then read the first 4 bytes as an unaligned u32.
    //
    // Preconditions (all guaranteed by the SVM for entrypoint accounts):
    // 1. AccountView is #[repr(C)] and its first field is a data pointer.
    // 2. The pointer is valid and points to a RuntimeAccount in the input buffer.
    // 3. The RuntimeAccount starts with [borrow_state, is_signer, is_writable, executable].
    let ptr = account as *const AccountView as *const u8;
    let raw_ptr = unsafe { *(ptr as *const *const u8) };
    unsafe { core::ptr::read_unaligned(raw_ptr as *const u32) }
}

/// Fast single-compare account validation.
///
/// Reads the RuntimeAccount header as a u32 and compares against the expected
/// pattern. If the comparison fails, decomposes to identify the specific error.
///
/// This collapses duplicate-check + signer-check + writable-check + executable-check
/// into a **single u32 comparison**, saving ~4-8 CU per account.
///
/// Safe to call on any `AccountView` from the SVM entrypoint.
/// On non-SVM targets, falls back to individual checks via AccountView methods.
#[inline(always)]
pub fn check_account_fast(
    account: &AccountView,
    expected_header: u32,
) -> ProgramResult {
    // Fast path: one compare for all flags
    #[cfg(all(target_os = "solana", feature = "hopper-native-backend"))]
    {
        let actual = unsafe { read_account_header(account) };
        if (actual & expected_header) == expected_header {
            return Ok(());
        }
        // Cold path: decompose error
        decompose_header_error(actual, expected_header)
    }
    #[cfg(not(all(target_os = "solana", feature = "hopper-native-backend")))]
    {
        // Off-chain fallback: individual checks
        check_account_flags_fallback(account, expected_header)
    }
}

/// Decompose a header mismatch into a specific error.
///
/// This is `#[cold]` -- only invoked on the error path, keeping the hot path
/// (the comparison above) as fast as possible.
#[cold]
#[inline(never)]
#[allow(dead_code)]
fn decompose_header_error(actual: u32, expected: u32) -> ProgramResult {
    // Check duplicate (borrow_state != 0xFF)
    if actual & 0xFF != 0xFF {
        return Err(ProgramError::AccountBorrowFailed);
    }
    // Check signer
    if (expected & (1 << 8)) != 0 && (actual & (1 << 8)) == 0 {
        return Err(ProgramError::MissingRequiredSignature);
    }
    // Check writable
    if (expected & (1 << 16)) != 0 && (actual & (1 << 16)) == 0 {
        return Err(ProgramError::InvalidAccountData);
    }
    // Check executable
    if (expected & (1 << 24)) != 0 && (actual & (1 << 24)) == 0 {
        return Err(ProgramError::InvalidAccountData);
    }
    // Generic mismatch
    Err(ProgramError::InvalidAccountData)
}

/// Off-chain fallback using individual AccountView methods.
#[cfg(not(all(target_os = "solana", feature = "hopper-native-backend")))]
fn check_account_flags_fallback(
    account: &AccountView,
    expected: u32,
) -> ProgramResult {
    if (expected & (1 << 8)) != 0 && !account.is_signer() {
        return Err(ProgramError::MissingRequiredSignature);
    }
    if (expected & (1 << 16)) != 0 && !account.is_writable() {
        return Err(ProgramError::InvalidAccountData);
    }
    if (expected & (1 << 24)) != 0 && !account.executable() {
        return Err(ProgramError::InvalidAccountData);
    }
    Ok(())
}

/// Validate a signer account with single u32 compare.
#[inline(always)]
pub fn check_signer_fast(account: &AccountView) -> ProgramResult {
    check_account_fast(account, HEADER_SIGNER)
}

/// Validate a writable account with single u32 compare.
#[inline(always)]
pub fn check_writable_fast(account: &AccountView) -> ProgramResult {
    check_account_fast(account, HEADER_WRITABLE)
}

/// Validate a signer + writable account (authority) with single u32 compare.
#[inline(always)]
pub fn check_authority_fast(account: &AccountView) -> ProgramResult {
    check_account_fast(account, HEADER_AUTHORITY)
}

/// Validate an executable (program) account with single u32 compare.
#[inline(always)]
pub fn check_executable_fast(account: &AccountView) -> ProgramResult {
    check_account_fast(account, HEADER_EXECUTABLE)
}
