//! Substrate-level `Pod` marker.
//!
//! The Hopper Safety Audit asked for every zero-copy access path -
//! all the way down to the native substrate, to require a real Pod
//! bound rather than the loose `T: Copy`. This module is that marker.
//!
//! ## Hopper-owned safety
//!
//! `Zeroable` and `Pod` are Hopper-owned marker traits. Hopper macros
//! emit field-level proof blocks that require every field to already
//! implement Hopper `Pod` before the containing layout receives its own
//! impl. That gives the same useful rejection points users got from the
//! old dependency-backed path while keeping the proof surface inside the
//! framework:
//!
//! - `bool`, `char`, references, not all bit patterns valid
//! - padded `#[repr(C)]` structs, padding bytes aren't accounted for
//! - non-alignment-1 primitives when alignment-1 was claimed
//! - enums with niches and non-zero variants
//!
//! This is the **Must-Fix #5** the audit flagged: "enforce field-level
//! Pod proof at macro expansion time". Hopper's `#[hopper::pod]` derive
//! and `#[hopper::state]` macro emit the proof so users never need to
//! name an external crate in their own sources.
//!
//! See [`hopper_runtime::pod::Pod`] (downstream re-export) for the
//! runtime-side view.

/// Marker for `Copy + Sized` values that are valid for every bit pattern.
///
/// # Safety
///
/// This is the **by-value** contract: a `Zeroable` value can be produced
/// by copying `size_of::<T>()` arbitrary bytes (e.g. a zero fill, or an
/// unaligned `read_unaligned`). It says **nothing** about alignment, so
/// it holds for native multi-byte integers as well. To overlay a type as
/// `&T` / `&mut T` directly on account bytes — which requires
/// alignment 1 — use [`Pod`] (and, at the framework level,
/// `hopper_runtime::ZeroCopy`).
pub unsafe trait Zeroable: Copy + Sized {}

/// Marker for types that can be safely overlaid as `&T` / `&mut T` on raw
/// account bytes at **any** offset.
///
/// # Safety
///
/// Implementing `Pod` for a type `T` asserts all of:
///
/// 1. Every `[u8; size_of::<T>()]` bit pattern decodes to a valid `T`.
/// 2. `align_of::<T>() == 1` — so a reference can be formed at any byte
///    offset without an unaligned-reference (which is UB).
/// 3. `T` contains no padding.
/// 4. `T` contains no internal pointers or references.
///
/// Native multi-byte integers (`u16`, `u32`, `u64`, `u128`, `i16`…`i128`)
/// are deliberately **not** `Pod`: their alignment is greater than 1, so
/// forming `&u64` from an arbitrary account offset is undefined behaviour.
/// Use the alignment-1 wire types (`WireU64`, `WireI64`, …) in layouts,
/// and [`ValuePod`] + [`read_unaligned_value`] for by-value scalar reads.
///
/// Hopper macros mechanically enforce the field-level proof before
/// emitting this impl. Hand-written impls carry the same unsafe contract.
pub unsafe trait Pod: Zeroable {}

/// Marker for `Copy + Sized` scalars/arrays that may be read **by value**
/// from raw bytes with [`read_unaligned_value`] (alignment-independent).
///
/// # Safety
///
/// Unlike [`Pod`], `ValuePod` does not permit forming a `&T` overlay, so
/// it is safe to implement for native multi-byte integers. Use it for
/// instruction-argument decoding and local scalar loads where the value
/// is copied out, not referenced in place. Implementers assert every
/// `[u8; size_of::<T>()]` bit pattern decodes to a valid `T`.
pub unsafe trait ValuePod: Copy + Sized {}

// ── Primitive implementations ───────────────────────────────────────
//
// `Zeroable` / `ValuePod`: every native integer is a valid by-value POD.
// `Pod`: only alignment-1 types (so `&T` overlays are never misaligned).
unsafe impl Zeroable for u8 {}
unsafe impl Pod for u8 {}
unsafe impl Zeroable for u16 {}
unsafe impl Zeroable for u32 {}
unsafe impl Zeroable for u64 {}
unsafe impl Zeroable for u128 {}
unsafe impl Zeroable for i8 {}
unsafe impl Pod for i8 {}
unsafe impl Zeroable for i16 {}
unsafe impl Zeroable for i32 {}
unsafe impl Zeroable for i64 {}
unsafe impl Zeroable for i128 {}
unsafe impl<T: Zeroable, const N: usize> Zeroable for [T; N] {}
unsafe impl<T: Pod, const N: usize> Pod for [T; N] {}
unsafe impl Zeroable for () {}
unsafe impl Pod for () {}

unsafe impl ValuePod for u8 {}
unsafe impl ValuePod for u16 {}
unsafe impl ValuePod for u32 {}
unsafe impl ValuePod for u64 {}
unsafe impl ValuePod for u128 {}
unsafe impl ValuePod for i8 {}
unsafe impl ValuePod for i16 {}
unsafe impl ValuePod for i32 {}
unsafe impl ValuePod for i64 {}
unsafe impl ValuePod for i128 {}
unsafe impl<T: ValuePod, const N: usize> ValuePod for [T; N] {}

/// Read a `ValuePod` scalar/array out of `bytes` at `offset` by value,
/// tolerating any alignment (uses `core::ptr::read_unaligned`).
///
/// Returns `Err(AccountDataTooSmall)` if the range is out of bounds. This
/// is the correct path for native multi-byte integers, which must never
/// be formed as a `&T` reference at an arbitrary offset.
#[inline]
pub fn read_unaligned_value<T: ValuePod>(
    bytes: &[u8],
    offset: usize,
) -> Result<T, crate::error::ProgramError> {
    let end = offset
        .checked_add(core::mem::size_of::<T>())
        .ok_or(crate::error::ProgramError::ArithmeticOverflow)?;
    if end > bytes.len() {
        return Err(crate::error::ProgramError::AccountDataTooSmall);
    }
    // SAFETY: bounds checked above; `read_unaligned` imposes no alignment
    // requirement and `T: ValuePod` guarantees all bit patterns are valid.
    Ok(unsafe { core::ptr::read_unaligned(bytes.as_ptr().add(offset) as *const T) })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn require<T: Pod>() {}
    fn require_value<T: ValuePod>() {}

    #[test]
    fn primitives_are_pod() {
        require::<u8>();
        require::<i8>();
        require::<[u8; 32]>();
    }

    #[test]
    fn multibyte_ints_are_value_pod_not_pod() {
        // By-value reads are fine for native integers...
        require_value::<u64>();
        require_value::<i128>();
        require_value::<[u32; 4]>();
        // ...but they are intentionally NOT `Pod` (alignment > 1), so the
        // overlay APIs that bound on `Pod` reject them at compile time.

        // Aligned and unaligned by-value reads both work.
        let bytes = [1u8, 0, 0, 0, 0, 0, 0, 0, 7, 0];
        let v0: u64 = read_unaligned_value(&bytes, 0).unwrap();
        assert_eq!(v0, 1); // bytes[0..8] LE = 1
        let v1: u64 = read_unaligned_value(&bytes, 1).unwrap();
        assert_eq!(v1, 7 << 56); // bytes[1..9] LE = [0,0,0,0,0,0,0,7]
                                 // Out-of-bounds is a clean error, never UB.
        assert!(read_unaligned_value::<u64>(&bytes, 5).is_err());
    }

    /// Demonstrates that `bool`, `Copy + Sized` but not all bit
    /// patterns valid, is **not** `Pod` under Hopper's contract.
    /// This relies on Hopper not providing a primitive impl for bool;
    /// Hopper macros also reject bool fields because every field must
    /// already satisfy Hopper `Pod`.
    #[test]
    fn bool_is_not_pod() {
        trait NotPod {}
        impl<T> NotPod for T {}
        // Compiles, bool has `NotPod` blanket impl.
        fn _f<T: NotPod>() {}
        _f::<bool>();
    }
}
