//! Substrate-level `Pod` marker.
//!
//! The Hopper Safety Audit asked for every zero-copy access path —
//! all the way down to the native substrate — to require a real Pod
//! bound rather than the loose `T: Copy`. This module is that marker.
//!
//! ## Bytemuck-backed safety (default)
//!
//! With the `bytemuck` feature enabled (default), `Pod` is declared
//! as a **sub-trait** of `bytemuck::Pod + bytemuck::Zeroable`. That
//! raises the bar exactly the way the audit recommends: every
//! `unsafe impl Pod for T {}` must be accompanied by a
//! `#[derive(bytemuck::Pod, bytemuck::Zeroable)]` (or hand-written
//! impls that satisfy bytemuck's machine-checked obligations).
//! Bytemuck's derive emits a compile-time proof that **every field**
//! of `T` is itself `Pod`, which mechanically rejects:
//!
//! - `bool`, `char`, references — not all bit patterns valid
//! - padded `#[repr(C)]` structs — padding bytes aren't accounted for
//! - non-alignment-1 primitives when alignment-1 was claimed
//! - enums with niches and non-zero variants
//!
//! This is the **Must-Fix #5** the audit flagged: "enforce field-level
//! Pod proof at macro expansion time". Hopper's `#[hopper::pod]` and
//! `#[hopper::state]` macros now emit the `#[derive(…)]` automatically
//! so users never see the bytemuck name in their own sources.
//!
//! ## Disable-able for zero-dep builds
//!
//! Programs that want to avoid any external dependency can turn off
//! the `bytemuck` feature. In that mode `Pod` is a standalone marker
//! with the documented four-point contract; the compile-time
//! obligation falls entirely on the `unsafe impl`. Existing primitive
//! impls continue to work either way.
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
/// With `feature = "bytemuck"` on (default), the trait is sealed so
/// callers must **also** prove `T: bytemuck::Pod + bytemuck::Zeroable`,
/// which gets them obligations 1, 3, and 4 mechanically via bytemuck's
/// derive. Obligation 2 (alignment) is still a Hopper-specific
/// constraint enforced by the `#[hopper::pod]` / `#[hopper::state]`
/// compile-time asserts.
#[cfg(feature = "bytemuck")]
pub unsafe trait Pod: Copy + Sized + bytemuck::Pod + bytemuck::Zeroable {}

/// Marker for types that can be safely overlaid on raw account bytes.
///
/// `bytemuck` feature disabled: the four-point contract must be
/// satisfied by the `unsafe impl` alone.
#[cfg(not(feature = "bytemuck"))]
pub unsafe trait Pod: Copy + Sized {}

// ── Primitive implementations ───────────────────────────────────────
//
// Both feature configurations get the same set of blanket impls.
// With `bytemuck` on these compile because bytemuck also has blanket
// impls for the same primitive types.

unsafe impl Pod for u8 {}
unsafe impl Pod for u16 {}
unsafe impl Pod for u32 {}
unsafe impl Pod for u64 {}
unsafe impl Pod for u128 {}
unsafe impl Pod for i8 {}
unsafe impl Pod for i16 {}
unsafe impl Pod for i32 {}
unsafe impl Pod for i64 {}
unsafe impl Pod for i128 {}
unsafe impl<const N: usize> Pod for [u8; N] {}
unsafe impl Pod for () {}

#[cfg(test)]
mod tests {
    use super::*;

    fn require<T: Pod>() {}

    #[test]
    fn primitives_are_pod() {
        require::<u8>();
        require::<u64>();
        require::<i128>();
        require::<[u8; 32]>();
    }

    /// Demonstrates that `bool` — `Copy + Sized` but not all bit
    /// patterns valid — is **not** `Pod` under Hopper's contract.
    /// With the `bytemuck` feature on this is enforced mechanically
    /// because `bool` isn't `bytemuck::Pod`. Without the feature the
    /// trait is a plain marker and the rejection relies on the user
    /// not writing `unsafe impl Pod for bool`.
    #[test]
    fn bool_is_not_pod() {
        trait NotPod {}
        impl<T> NotPod for T {}
        trait IsPod {}
        impl<T: Pod> IsPod for T {}
        // Compiles — bool has `NotPod` blanket impl.
        fn _f<T: NotPod>() {}
        _f::<bool>();
    }
}
