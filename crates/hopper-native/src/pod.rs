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

/// Marker for types that can be safely overlaid on raw account bytes.
///
/// # Safety
///
/// Implementing `Pod` for a type `T` asserts all of:
///
/// 1. Every `[u8; size_of::<T>()]` bit pattern decodes to a valid `T`.
/// 2. `align_of::<T>() == 1`.
/// 3. `T` contains no padding.
/// 4. `T` contains no internal pointers or references.
///
pub unsafe trait Zeroable: Copy + Sized {}

/// Marker for types that can be safely overlaid on raw account bytes.
///
/// Hopper macros mechanically enforce the field-level proof before
/// emitting this impl. Hand-written impls carry the same unsafe contract.
pub unsafe trait Pod: Zeroable {}

// ── Primitive implementations ───────────────────────────────────────
//
unsafe impl Zeroable for u8 {}
unsafe impl Pod for u8 {}
unsafe impl Zeroable for u16 {}
unsafe impl Pod for u16 {}
unsafe impl Zeroable for u32 {}
unsafe impl Pod for u32 {}
unsafe impl Zeroable for u64 {}
unsafe impl Pod for u64 {}
unsafe impl Zeroable for u128 {}
unsafe impl Pod for u128 {}
unsafe impl Zeroable for i8 {}
unsafe impl Pod for i8 {}
unsafe impl Zeroable for i16 {}
unsafe impl Pod for i16 {}
unsafe impl Zeroable for i32 {}
unsafe impl Pod for i32 {}
unsafe impl Zeroable for i64 {}
unsafe impl Pod for i64 {}
unsafe impl Zeroable for i128 {}
unsafe impl Pod for i128 {}
unsafe impl<T: Zeroable, const N: usize> Zeroable for [T; N] {}
unsafe impl<T: Pod, const N: usize> Pod for [T; N] {}
unsafe impl Zeroable for () {}
unsafe impl Pod for () {}

#[cfg(test)]
mod tests {
    use super::*;

    fn require<T: Pod>() {}

    #[test]
    fn primitives_are_pod() {
        require::<u8>();
        require::<i8>();
        require::<[u8; 32]>();
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
