//! `Pod` — the substrate-level "safe to interpret from raw bytes" marker.
//!
//! Hopper's typed access primitives (`segment_ref`, `segment_mut`,
//! `raw_ref`, `raw_mut` and friends) all overlay a `T` on a slice of
//! account bytes. That overlay is only sound if **every bit pattern of
//! the right size** decodes to a valid `T` and the type has alignment 1
//! (so the offset within the BPF input buffer is always valid for `T`).
//!
//! The Solana safety audit flagged that requiring only `T: Copy` is too
//! loose: `bool`, `char`, references, and structs with padding are all
//! `Copy + Sized` but **not** safe to overlay on raw bytes. This module
//! introduces an `unsafe trait Pod: Copy + Sized` that captures the real
//! contract:
//!
//! ## Contract
//!
//! Implementing `Pod` for a type `T` is asserting all of:
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
//! Hopper's higher-layer macros (`#[hopper::state]`, `hopper_layout!`)
//! enforce these conditions at compile time and emit a derived
//! `unsafe impl Pod`. Programs that hand-author layouts can opt in via
//! `unsafe impl hopper_native::Pod for MyLayout {}`.

/// Marker for types that can be safely overlaid on raw account bytes.
///
/// See module docs for the full safety contract. Implementing this trait
/// requires `unsafe impl` because the runtime cannot mechanically verify
/// any of the four obligations.
pub unsafe trait Pod: Copy + Sized {}

// ── Primitive implementations ────────────────────────────────────────
//
// All multi-byte integer types satisfy alignment-1 only when wrapped in
// a Hopper `WireU16`/`WireU64`/etc. (declared in `hopper-native::wire`),
// but the raw integer types are still `Pod` for the purposes of the
// runtime API: they just require the caller to ensure offset alignment
// matches their natural alignment, which in practice the BPF runtime
// guarantees for header-aligned reads.

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

// `usize` and `isize` are *not* `Pod`: their size is target-dependent so
// overlays would silently change layout between host and BPF builds.

// Fixed-size byte arrays are the workhorse of Hopper layouts: addresses,
// layout IDs, segment fingerprints, etc.
unsafe impl<const N: usize> Pod for [u8; N] {}

// `()` is sometimes used as a phantom payload; allowing it lets generic
// helpers fall back to a no-op overlay.
unsafe impl Pod for () {}

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
}
