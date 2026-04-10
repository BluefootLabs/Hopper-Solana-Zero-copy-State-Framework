//! Zero-copy struct projection from account data.
//!
//! `project::<T>()` performs bounds checking, alignment validation, and
//! optional discriminator verification in a single operation, returning
//! a direct `&T` pointer-cast into account data. No copies, no alloc,
//! no separate validation steps.
//!
//! This is genuinely novel: pinocchio only gives raw `&[u8]` from account
//! data. Anchor's `AccountLoader<T>` requires derive macros, borsh traits,
//! and hidden RefCell costs. Hopper's projection is a one-line zero-copy
//! cast with compile-time layout guarantees.
//!
//! # Safety Model
//!
//! `project` requires `T: Pod` (Copy + 'static + no padding) enforced by
//! the `Pod` marker trait from hopper-core. For hopper-native standalone
//! use, we provide a minimal `Projectable` trait that any `#[repr(C)]`
//! struct can implement.
//!
//! # Usage
//!
//! ```ignore
//! use hopper_native::project::{Projectable, project, project_mut};
//!
//! #[repr(C)]
//! #[derive(Clone, Copy)]
//! struct VaultState {
//!     authority: [u8; 32],
//!     balance: u64,
//!     bump: u8,
//! }
//!
//! // SAFETY: VaultState is #[repr(C)], Copy, and has no padding bytes
//! // that could cause UB when read from arbitrary data.
//! unsafe impl Projectable for VaultState {}
//!
//! fn read_vault(account: &AccountView) -> Result<&VaultState, ProgramError> {
//!     // Checks: data_len >= offset + size_of::<VaultState>(),
//!     //         alignment is correct, disc byte matches.
//!     project::<VaultState>(account, 10, Some(1))
//! }
//! ```

use crate::account_view::AccountView;
use crate::error::ProgramError;

/// Marker trait for types that can be safely projected from raw account data.
///
/// # Safety
///
/// The implementor must guarantee that:
/// 1. The type is `#[repr(C)]` (deterministic field ordering).
/// 2. The type is `Copy` (no drop glue, no interior mutability).
/// 3. Every bit pattern is valid (no padding-dependent invariants).
/// 4. No references or pointers (only plain data).
///
/// This is the same contract as `bytemuck::Pod` without the dependency.
pub unsafe trait Projectable: Copy + 'static {}

// Built-in projectable types.
unsafe impl Projectable for u8 {}
unsafe impl Projectable for u16 {}
unsafe impl Projectable for u32 {}
unsafe impl Projectable for u64 {}
unsafe impl Projectable for u128 {}
unsafe impl Projectable for i8 {}
unsafe impl Projectable for i16 {}
unsafe impl Projectable for i32 {}
unsafe impl Projectable for i64 {}
unsafe impl Projectable for i128 {}
unsafe impl Projectable for [u8; 32] {}
unsafe impl Projectable for [u8; 64] {}

/// Project a `#[repr(C)]` struct from account data at the given byte offset.
///
/// Performs three checks in one operation:
/// 1. **Bounds**: `offset + size_of::<T>() <= data_len`
/// 2. **Alignment**: `(data_ptr + offset) % align_of::<T>() == 0`
/// 3. **Discriminator** (optional): `data[0] == expected_disc`
///
/// Returns a direct `&T` reference into the account's data region.
/// No copies, no allocation.
///
/// # Arguments
///
/// * `account` - The account to project from.
/// * `offset` - Byte offset into account data where `T` begins.
///   For Hopper accounts with a standard 10-byte header (disc + version
///   + layout_id), use `offset = 10`.
/// * `expected_disc` - If `Some(d)`, verify that `data[0] == d` before
///   projecting. Pass `None` to skip the discriminator check.
#[inline]
pub fn project<T: Projectable>(
    account: &AccountView,
    offset: usize,
    expected_disc: Option<u8>,
) -> Result<&T, ProgramError> {
    let data_len = account.data_len();
    let type_size = core::mem::size_of::<T>();

    // Bounds check.
    if offset.checked_add(type_size).map_or(true, |end| end > data_len) {
        return Err(ProgramError::AccountDataTooSmall);
    }

    // Discriminator check (if requested).
    if let Some(disc) = expected_disc {
        if account.disc() != disc {
            return Err(ProgramError::InvalidAccountData);
        }
    }

    let data_ptr = account.data_ptr();
    let target_ptr = unsafe { data_ptr.add(offset) };

    // Alignment check.
    let align = core::mem::align_of::<T>();
    if align > 1 && (target_ptr as usize) % align != 0 {
        return Err(ProgramError::InvalidAccountData);
    }

    // SAFETY: bounds checked, alignment verified, T: Projectable guarantees
    // all bit patterns are valid.
    Ok(unsafe { &*(target_ptr as *const T) })
}

/// Project a mutable `#[repr(C)]` struct from account data.
///
/// Same checks as `project()` but returns `&mut T`. The caller is
/// responsible for ensuring no other borrows are active (this does
/// NOT integrate with the borrow tracking system -- use
/// `try_borrow_mut()` first if you need that guarantee).
///
/// # Safety
///
/// The caller must ensure no other references to the same data region
/// are active. For most use cases, call `account.try_borrow_mut()`
/// first, then use `project_mut` on the resulting data.
#[inline]
pub unsafe fn project_mut<T: Projectable>(
    account: &AccountView,
    offset: usize,
    expected_disc: Option<u8>,
) -> Result<&mut T, ProgramError> {
    let data_len = account.data_len();
    let type_size = core::mem::size_of::<T>();

    // Bounds check.
    if offset.checked_add(type_size).map_or(true, |end| end > data_len) {
        return Err(ProgramError::AccountDataTooSmall);
    }

    // Discriminator check (if requested).
    if let Some(disc) = expected_disc {
        if account.disc() != disc {
            return Err(ProgramError::InvalidAccountData);
        }
    }

    let data_ptr = account.data_ptr();
    let target_ptr = unsafe { data_ptr.add(offset) };

    // Alignment check.
    let align = core::mem::align_of::<T>();
    if align > 1 && (target_ptr as usize) % align != 0 {
        return Err(ProgramError::InvalidAccountData);
    }

    // SAFETY: caller guarantees exclusive access, bounds/alignment checked.
    Ok(unsafe { &mut *(target_ptr as *mut T) })
}

/// Project a slice of `T` from account data starting at `offset`.
///
/// Returns `&[T]` with `count` elements, performing bounds and alignment
/// checks.
#[inline]
pub fn project_slice<T: Projectable>(
    account: &AccountView,
    offset: usize,
    count: usize,
) -> Result<&[T], ProgramError> {
    let data_len = account.data_len();
    let type_size = core::mem::size_of::<T>();
    let total = count.checked_mul(type_size).ok_or(ProgramError::ArithmeticOverflow)?;

    if offset.checked_add(total).map_or(true, |end| end > data_len) {
        return Err(ProgramError::AccountDataTooSmall);
    }

    let data_ptr = account.data_ptr();
    let target_ptr = unsafe { data_ptr.add(offset) };

    let align = core::mem::align_of::<T>();
    if align > 1 && (target_ptr as usize) % align != 0 {
        return Err(ProgramError::InvalidAccountData);
    }

    Ok(unsafe { core::slice::from_raw_parts(target_ptr as *const T, count) })
}

/// Project with a Hopper standard header: skip the 10-byte header
/// (1 disc + 1 version + 8 layout_id) and project `T` starting at
/// byte 10. Verifies discriminator.
///
/// This is the most common projection pattern for Hopper accounts.
#[inline]
pub fn project_hopper<T: Projectable>(
    account: &AccountView,
    expected_disc: u8,
) -> Result<&T, ProgramError> {
    project::<T>(account, 10, Some(expected_disc))
}

/// Mutable version of `project_hopper`.
///
/// # Safety
///
/// Caller must ensure exclusive access to the account data.
#[inline]
pub unsafe fn project_hopper_mut<T: Projectable>(
    account: &AccountView,
    expected_disc: u8,
) -> Result<&mut T, ProgramError> {
    unsafe { project_mut::<T>(account, 10, Some(expected_disc)) }
}
