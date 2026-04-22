//! Compile-proven borrow-guard constraint.
//!
//! The Hopper Safety Audit's Finding 2 asks for *compile-time* proof
//! that no raw `&T` / `&mut T` can escape an account access path.
//! Every runtime surface already returns a [`Ref`], [`RefMut`],
//! [`SegRef`], or [`SegRefMut`], but that guarantee is embedded in the
//! function return types alone. [`HopperRefOnly`] is the nominal
//! version of that promise: a sealed marker trait implemented only by
//! Hopper's four borrow guards.
//!
//! API authors can now write `fn f<G: HopperRefOnly>(g: G)` and rely
//! on the compiler to reject a naked `&mut U` at the call site. The
//! sealed trait pattern means no downstream crate can stamp the marker
//! onto arbitrary types, which closes the audit's "prove no raw refs"
//! gate at compile time instead of by convention.
//!
//! # Grep receipt
//!
//! An auditor running `grep -r "HopperRefOnly"` sees exactly five
//! lines: the trait declaration plus the four guard impls. There is
//! no macro-generated expansion, no procedural indirection. Every
//! impl is visible at the byte level.
//!
//! [`Ref`]: crate::borrow::Ref
//! [`RefMut`]: crate::borrow::RefMut
//! [`SegRef`]: crate::segment_lease::SegRef
//! [`SegRefMut`]: crate::segment_lease::SegRefMut

use crate::borrow::{Ref, RefMut};
use crate::segment_lease::{SegRef, SegRefMut};

mod sealed {
    /// Doc-hidden seal. Implementing this for a non-Hopper type would
    /// require naming `hopper_runtime::ref_only::sealed::Sealed`,
    /// which this private module makes impossible from outside the
    /// crate.
    pub trait Sealed {}
}

/// Marker trait implemented exclusively by Hopper's four account-data
/// borrow guards: [`Ref`], [`RefMut`], [`SegRef`], [`SegRefMut`].
///
/// Use this as a bound on APIs that must accept only drop-guarded
/// borrows. A naked `&T` or `&mut T` will fail the bound at compile
/// time, which is the closure proof for Finding 2 of the audit
/// ("borrow safety compile-proven, not just runtime-enforced").
///
/// [`Ref`]: crate::borrow::Ref
/// [`RefMut`]: crate::borrow::RefMut
/// [`SegRef`]: crate::segment_lease::SegRef
/// [`SegRefMut`]: crate::segment_lease::SegRefMut
pub trait HopperRefOnly: sealed::Sealed {}

impl<T: ?Sized> sealed::Sealed for Ref<'_, T> {}
impl<T: ?Sized> sealed::Sealed for RefMut<'_, T> {}
impl<T: ?Sized> sealed::Sealed for SegRef<'_, T> {}
impl<T: ?Sized> sealed::Sealed for SegRefMut<'_, T> {}

impl<T: ?Sized> HopperRefOnly for Ref<'_, T> {}
impl<T: ?Sized> HopperRefOnly for RefMut<'_, T> {}
impl<T: ?Sized> HopperRefOnly for SegRef<'_, T> {}
impl<T: ?Sized> HopperRefOnly for SegRefMut<'_, T> {}

#[cfg(test)]
mod tests {
    use super::HopperRefOnly;

    fn require_guard<G: HopperRefOnly>() {}

    #[test]
    fn hopper_guards_satisfy_the_bound() {
        require_guard::<crate::borrow::Ref<'_, u64>>();
        require_guard::<crate::borrow::RefMut<'_, u64>>();
        require_guard::<crate::segment_lease::SegRef<'_, u64>>();
        require_guard::<crate::segment_lease::SegRefMut<'_, u64>>();
    }
}
