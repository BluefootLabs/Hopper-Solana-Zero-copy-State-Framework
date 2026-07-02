//! Pod and FixedLayout traits for zero-copy account access.
//!
//! `Pod` is the canonical "safe to overlay on raw bytes" marker trait.
//! It lives in [`hopper_runtime::pod`] so every Hopper access path, the
//! native substrate, runtime accessors, frame, and core helpers, can
//! agree on one contract. Hopper-core re-exports it unchanged so
//! existing `use hopper_core::account::Pod` call sites keep compiling.
//!
//! The safety contract (enforced by `unsafe impl`): every
//! `[u8; size_of::<T>()]` bit pattern decodes to a valid `T`, alignment
//! is 1, no padding, no internal pointers. `#[hopper::state]` emits the
//! derived `unsafe impl Pod` for every generated layout; hand-authored
//! layouts must opt in explicitly.

use hopper_runtime::error::ProgramError;

pub use hopper_runtime::pod::{Pod, Zeroable};

/// Trait for zero-copy overlay types with a compile-time-known wire size.
///
/// # Self-proving size (Hopper's sovereign design)
///
/// For an align-1, no-padding overlay type — which is exactly the `Pod`
/// contract this trait is used alongside — the wire size *is*
/// `size_of::<Self>()`, always. So [`SIZE`](Self::SIZE) **defaults** to
/// it: conforming types implement `FixedLayout` with an empty body and
/// get the correct size for free.
///
/// The value is not merely defaulted, it is *proven*.
/// [`_SIZE_IS_HONEST`](Self::_SIZE_IS_HONEST) is a compile-time assertion
/// that `SIZE == size_of::<Self>()`; every consumer that trusts `SIZE`
/// in unsafe pointer arithmetic references it, so an impl that overrides
/// `SIZE` with a wrong value is a **build error the moment the type is
/// used** — not a latent out-of-bounds waiting for a fuzzer. This
/// replaces the hand-written `const _: () = assert!(size_of == N)` that
/// each overlay type previously had to remember to write next to its
/// `SIZE`. The framework owns the invariant now, not the author.
pub trait FixedLayout: Sized {
    /// Total byte size on the wire. Defaults to `size_of::<Self>()`,
    /// which is correct for every align-1, no-padding `Pod` overlay;
    /// override only if you have a genuine reason (and it must still
    /// equal `size_of::<Self>()`, enforced by [`Self::_SIZE_IS_HONEST`]).
    const SIZE: usize = core::mem::size_of::<Self>();

    /// Compile-time proof that [`SIZE`](Self::SIZE) is the true byte
    /// size of `Self`. Consumers that feed `SIZE` into unsafe pointer
    /// arithmetic touch this const so a dishonest override cannot reach
    /// runtime. Not intended to be referenced by name in user code.
    #[doc(hidden)]
    const _SIZE_IS_HONEST: () = assert!(
        Self::SIZE == core::mem::size_of::<Self>(),
        "FixedLayout::SIZE must equal size_of::<Self>()",
    );
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
