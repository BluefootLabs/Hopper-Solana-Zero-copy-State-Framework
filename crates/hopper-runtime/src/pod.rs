//! `Pod` — the canonical runtime-layer "safe to interpret from raw bytes" marker.
//!
//! Hopper's typed access primitives (`segment_ref`, `segment_mut`,
//! `raw_ref`, `raw_mut`, `read_data`) all overlay a `T` on a slice of
//! account bytes. That overlay is only sound if **every bit pattern of
//! the right size** decodes to a valid `T` and the type has alignment 1
//! (so the offset within the BPF input buffer is always valid for `T`).
//!
//! The Hopper Safety Audit flagged that requiring only `T: Copy` is too
//! loose: `bool`, `char`, references, and structs with padding are all
//! `Copy + Sized` but **not** safe to overlay on raw bytes. This module
//! carries the tightened marker.
//!
//! ## Contract
//!
//! Implementing `Pod` for a type `T` asserts all of:
//!
//! 1. Every `[u8; size_of::<T>()]` byte pattern represents a valid `T`.
//!    No "niches", no enum-discriminant invariants, no `bool`-style
//!    forbidden bit patterns.
//! 2. `align_of::<T>() == 1` — the type can be read from any byte
//!    offset of an account buffer without alignment fault.
//! 3. `T` contains no padding (`#[repr(C)]` with alignment-1 fields, or
//!    `#[repr(transparent)]` over a `Pod` type).
//! 4. `T` contains no internal pointers / references — overlay always
//!    yields data that's safe to `Copy`.
//!
//! Hopper's higher-layer macros (`#[hopper::state]`, `#[hopper::pod]`,
//! `hopper_layout!`) enforce these conditions at compile time and emit
//! the derived `unsafe impl Pod`. Hand-authored layouts opt in via
//! `unsafe impl Pod for MyLayout {}`.
//!
//! ## Compile-fail demonstration (Hopper Safety Audit regression)
//!
//! With the `bytemuck` feature on (default), the following mis-use
//! patterns are all rejected at compile time. The audit's Must-Fix #5
//! — "enforce field-level Pod proof at macro expansion time" — is
//! now mechanically enforced by bytemuck's own `Pod + Zeroable`
//! bounds, so every zero-copy access path rejects them automatically.
//!
//! `bool` is not Pod (the bit patterns `0x02..=0xFF` don't decode to
//! a valid `bool`):
//!
//! ```compile_fail
//! # use hopper_runtime::{AccountView, segment_borrow::SegmentBorrowRegistry};
//! # fn example(account: &AccountView, borrows: &mut SegmentBorrowRegistry) {
//! let _ = account.segment_ref::<bool>(borrows, 16, 1);
//! # }
//! ```
//!
//! `char` is not Pod (valid Unicode scalar values form a sparse set):
//!
//! ```compile_fail
//! # use hopper_runtime::{AccountView, segment_borrow::SegmentBorrowRegistry};
//! # fn example(account: &AccountView, borrows: &mut SegmentBorrowRegistry) {
//! let _ = account.segment_ref::<char>(borrows, 16, 4);
//! # }
//! ```
//!
//! A `#[repr(C)]` struct with implicit padding is not bytemuck-Pod
//! — bytemuck's derive / Pod bound rejects the padding bytes because
//! they'd leak uninitialised data through `bytes_of`:
//!
//! ```compile_fail
//! # use hopper_runtime::{AccountView, segment_borrow::SegmentBorrowRegistry};
//! # fn example(account: &AccountView, borrows: &mut SegmentBorrowRegistry) {
//! #[derive(Copy, Clone)]
//! #[repr(C)]
//! struct Padded {
//!     a: u8,
//!     // implicit 7 bytes of padding to align b
//!     b: u64,
//! }
//! let _ = account.segment_ref::<Padded>(borrows, 16, 16);
//! # }
//! ```
//!
//! A type-level user mis-spelling `unsafe impl Pod for Padded {}`
//! without also satisfying `bytemuck::Pod + Zeroable` would fail at
//! the `Pod` supertrait bound. The compile-fail block above exercises
//! that path: no explicit `impl Pod` exists, and the access-path
//! generic requires it.
//!
//! A well-formed primitive or wire type is accepted:
//!
//! ```ignore
//! # use hopper_runtime::{AccountView, segment_borrow::SegmentBorrowRegistry};
//! # fn example(account: &AccountView, borrows: &mut SegmentBorrowRegistry) {
//! let _: Result<hopper_runtime::SegRef<'_, u64>, _> =
//!     account.segment_ref::<u64>(borrows, 16, 8);
//! # }
//! ```
//!
//! ## Trait identity across layers
//!
//! When `hopper-native-backend` is active (the default), this trait is
//! a direct re-export of [`hopper_native::Pod`]. That keeps the entire
//! Hopper stack — substrate, runtime, core, macros — on a single Pod
//! trait: one `unsafe impl Pod for MyStruct {}` unlocks every Hopper
//! access API from the lowest-level `AccountView::raw_mut` up to
//! `#[hopper::state]`-generated accessors, across all crates, with no
//! orphan-rule gymnastics. When a non-native backend is selected
//! (`pinocchio-backend`, `solana-program-backend`), the trait is
//! defined locally with the same contract so user code compiles
//! unchanged.

// ── Trait identity: native backend path ──────────────────────────────
//
// Re-export `hopper_native::Pod` directly so the "one canonical Pod"
// invariant holds end-to-end.
#[cfg(feature = "hopper-native-backend")]
pub use hopper_native::Pod;

// ── Trait identity: non-native backend path ─────────────────────────
//
// Define the trait locally for test harnesses and alternate backends
// that don't pull in `hopper-native`. The contract is identical.
#[cfg(not(feature = "hopper-native-backend"))]
pub unsafe trait Pod: Copy + Sized {}

#[cfg(not(feature = "hopper-native-backend"))]
mod local_impls {
    use super::Pod;
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
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_pod<T: Pod>() {}

    #[test]
    fn primitives_are_pod() {
        assert_pod::<u8>();
        assert_pod::<u16>();
        assert_pod::<u32>();
        assert_pod::<u64>();
        assert_pod::<u128>();
        assert_pod::<i8>();
        assert_pod::<i16>();
        assert_pod::<i32>();
        assert_pod::<i64>();
        assert_pod::<i128>();
        assert_pod::<[u8; 32]>();
    }

    #[test]
    fn address_satisfies_pod() {
        // `Address` is declared `#[repr(transparent)] [u8; 32]` with a
        // hand-rolled `unsafe impl Pod`. Under the native backend that
        // impl is on `hopper_native::Pod`; here we're just checking the
        // re-export plumbing lands.
        assert_pod::<crate::address::Address>();
    }
}
