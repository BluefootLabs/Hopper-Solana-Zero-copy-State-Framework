//! Pod and FixedLayout traits for zero-copy account access.

use hopper_runtime::error::ProgramError;

/// Marker trait for plain-old-data types safe for zero-copy overlay.
///
/// # Safety
///
/// Implementors must guarantee:
/// - All bit patterns of `Self` are valid.
/// - `Self` is `Copy` and has no drop glue.
/// - `Self` has no internal padding that carries invariants.
pub unsafe trait Pod: Copy + Sized {}

// SAFETY: Primitive byte types -- all bit patterns trivially valid.
unsafe impl Pod for u8 {}
unsafe impl Pod for [u8; 32] {}

/// Trait for types with a compile-time known wire size.
pub trait FixedLayout {
    /// Total byte size on the wire (including any header if applicable).
    const SIZE: usize;
}

/// Zero-copy cast from bytes to an immutable reference.
///
/// # Safety
///
/// The returned reference aliases the input slice. Callers must not create
/// overlapping mutable references to the same memory.
#[inline(always)]
pub fn pod_from_bytes<T: Pod + FixedLayout>(data: &[u8]) -> Result<&T, ProgramError> {
    if data.len() < T::SIZE {
        return Err(ProgramError::InvalidAccountData);
    }
    // SAFETY: T: Pod guarantees all bit patterns valid. We checked length.
    // Alignment is 1 for all our wire types (compile-time enforced by WireType).
    // For user structs, alignment is 1 via #[repr(C)] over alignment-1 fields.
    Ok(unsafe { &*(data.as_ptr() as *const T) })
}

/// Zero-copy cast from bytes to a mutable reference.
///
/// # Safety
///
/// The returned reference aliases the input slice mutably. Callers must not
/// create overlapping references (mutable or immutable) to the same memory.
#[inline(always)]
pub fn pod_from_bytes_mut<T: Pod + FixedLayout>(data: &mut [u8]) -> Result<&mut T, ProgramError> {
    if data.len() < T::SIZE {
        return Err(ProgramError::InvalidAccountData);
    }
    // SAFETY: Same as pod_from_bytes, plus we have exclusive (&mut) access.
    Ok(unsafe { &mut *(data.as_mut_ptr() as *mut T) })
}

/// Copy a Pod value from bytes (alignment-safe).
#[inline(always)]
pub fn pod_read<T: Pod + FixedLayout>(data: &[u8]) -> Result<T, ProgramError> {
    if data.len() < T::SIZE {
        return Err(ProgramError::InvalidAccountData);
    }
    // SAFETY: T: Pod, all bit patterns valid. read_unaligned handles alignment.
    Ok(unsafe { core::ptr::read_unaligned(data.as_ptr() as *const T) })
}

/// Write a Pod value to bytes (alignment-safe).
#[inline(always)]
pub fn pod_write<T: Pod + FixedLayout>(data: &mut [u8], value: &T) -> Result<(), ProgramError> {
    if data.len() < T::SIZE {
        return Err(ProgramError::InvalidAccountData);
    }
    // SAFETY: T: Pod, we checked length, write_unaligned handles alignment.
    unsafe {
        core::ptr::write_unaligned(data.as_mut_ptr() as *mut T, *value);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tier C -- Unchecked Raw Escape Hatch
// ---------------------------------------------------------------------------

/// Raw unchecked cast from bytes to an immutable reference.
///
/// **Tier C escape hatch.** No size check, no header validation, no
/// fingerprint verification. The caller owns all layout, compatibility,
/// and upgrade risk.
///
/// # Safety
///
/// - `data.len()` must be at least `size_of::<T>()`.
/// - `T` must be `Pod` (all bit patterns valid, alignment-1, `Copy`).
/// - No concurrent mutable references may alias `data`.
#[inline(always)]
pub unsafe fn cast_unchecked<T: Pod>(data: &[u8]) -> &T {
    // SAFETY: Caller guarantees length and aliasing requirements.
    unsafe { &*(data.as_ptr() as *const T) }
}

/// Raw unchecked cast from bytes to a mutable reference.
///
/// **Tier C escape hatch.** Same as [`cast_unchecked`] but returns `&mut T`.
///
/// # Safety
///
/// - `data.len()` must be at least `size_of::<T>()`.
/// - `T` must be `Pod` (all bit patterns valid, alignment-1, `Copy`).
/// - No other references (mutable or immutable) may alias `data`.
#[inline(always)]
pub unsafe fn cast_unchecked_mut<T: Pod>(data: &mut [u8]) -> &mut T {
    // SAFETY: Caller guarantees length, aliasing, and exclusive access.
    unsafe { &mut *(data.as_mut_ptr() as *mut T) }
}
