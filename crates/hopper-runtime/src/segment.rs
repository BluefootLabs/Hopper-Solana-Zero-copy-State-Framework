//! Runtime-local segment primitive.
//!
//! `Segment` is the tiny memory-contract descriptor that every segment
//! access routes through: `{offset, size}`, 8 bytes on 32-bit accounts,
//! `Copy`, `const`-constructable, no strings, no extra fields. It is the
//! runtime counterpart to `hopper_core::segment_map::StaticSegment`
//! (which carries a human-readable name for tooling), the runtime
//! never needs the name, so this primitive stays bare.
//!
//! # Design
//!
//! The finish-line audit was explicit: segment access must be
//! compile-time enforced and runtime cheap. Every Hopper segment
//! accessor should eventually lower to `ptr + const_offset -> cast`
//! and nothing more. Using this primitive means:
//!
//! - macros emit `const BALANCE: Segment = Segment::body(0, 8);`
//! - call sites read `account.segment_mut_const::<u64>(&mut b, BALANCE)?`
//! - the compiler substitutes the constant, collapses the call chain,
//!   and on Solana SBF you see one register-add over `data_ptr`.
//!
//! `Segment` never appears in an on-chain layout, it is a compile-time
//! description only. Use `hopper_core::account::SegmentDescriptor` for
//! bytes that travel on the wire.

use crate::layout::HopperHeader;

/// Compile-time descriptor of a typed byte range inside an account.
///
/// Fields are `u32` because every Solana account is bounded by
/// `u32::MAX` in practice and we want the whole primitive to fit in a
/// single 64-bit register.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(C)]
pub struct Segment {
    /// Absolute byte offset from the start of account data (includes
    /// the 16-byte Hopper header). This is what the access primitives
    /// want, so storing it absolute avoids a runtime addition.
    pub offset: u32,
    /// Byte size of the segment.
    pub size: u32,
}

impl Segment {
    /// Construct a segment from an absolute offset (measured from the
    /// start of account data, including the Hopper header).
    #[inline(always)]
    pub const fn new(offset: u32, size: u32) -> Self {
        Self { offset, size }
    }

    /// Construct a segment from a body-relative offset (offset measured
    /// past the 16-byte Hopper header). This is the form that macros
    /// most often emit: `#[hopper::state]` computes field offsets
    /// relative to the struct body, and body-relative is what
    /// `SegmentMap::SEGMENTS` stores.
    #[inline(always)]
    pub const fn body(body_offset: u32, size: u32) -> Self {
        Self {
            offset: HopperHeader::SIZE as u32 + body_offset,
            size,
        }
    }

    /// One-past-the-end byte offset.
    #[inline(always)]
    pub const fn end(&self) -> u32 {
        self.offset + self.size
    }

    /// Whether two segments share any byte.
    #[inline(always)]
    pub const fn overlaps(&self, other: &Segment) -> bool {
        self.offset < other.end() && other.offset < self.end()
    }

    /// Whether this segment is contained fully within `container`.
    #[inline(always)]
    pub const fn contained_in(&self, container: &Segment) -> bool {
        self.offset >= container.offset && self.end() <= container.end()
    }
}

// ══════════════════════════════════════════════════════════════════════
//  TypedSegment<T, const OFFSET: u32>
// ══════════════════════════════════════════════════════════════════════
//
// Where `Segment` carries `(offset, size)` at runtime, `TypedSegment`
// folds **both** values into the type system: `T` determines the size
// via `size_of::<T>()`, and `OFFSET` is a const generic. The struct
// itself is a ZST, no memory at all. This is the finish-line audit's
// "const-generic segments & compile-time offsets" innovation: at every
// call site the compiler substitutes the literal offset and literal
// size into the bounds check + pointer add, leaving pure
// `ptr + constant` arithmetic in the emitted BPF.
//
// Use `TypedSegment` when you know the layout at compile time (i.e.
// every `#[hopper::state]` field). Fall back to `Segment` when the
// offset is data-dependent (e.g. a user-provided index into a fixed
// array).

/// Compile-time typed segment descriptor: `T` is the overlay type,
/// `OFFSET` is the absolute byte offset from the start of account
/// data. Zero-sized.
///
/// ```ignore
/// // Matches Vault.balance at body offset 0, past the 16-byte header:
/// const VAULT_BALANCE: TypedSegment<WireU64, { HopperHeader::SIZE as u32 }>
///     = TypedSegment::new();
///
/// let bal = account.segment_ref_typed(&mut borrows, VAULT_BALANCE)?;
/// ```
#[derive(Copy, Clone, Debug, Default)]
pub struct TypedSegment<T: crate::Pod, const OFFSET: u32> {
    _marker: core::marker::PhantomData<fn() -> T>,
}

impl<T: crate::Pod, const OFFSET: u32> TypedSegment<T, OFFSET> {
    /// Construct the marker. Runs entirely at compile time.
    #[inline(always)]
    pub const fn new() -> Self {
        Self { _marker: core::marker::PhantomData }
    }

    /// The absolute byte offset of this segment (`OFFSET` const-generic).
    #[inline(always)]
    pub const fn offset() -> u32 {
        OFFSET
    }

    /// The byte size of this segment (`size_of::<T>()`, folded at compile time).
    #[inline(always)]
    pub const fn size() -> u32 {
        core::mem::size_of::<T>() as u32
    }

    /// One-past-the-end byte offset.
    #[inline(always)]
    pub const fn end() -> u32 {
        OFFSET + core::mem::size_of::<T>() as u32
    }

    /// Lower to a runtime [`Segment`] when a heterogeneous collection
    /// of segments is needed (e.g. a validation pass that iterates).
    #[inline(always)]
    pub const fn as_segment() -> Segment {
        Segment::new(OFFSET, core::mem::size_of::<T>() as u32)
    }
}

// SAFETY: Proof that `TypedSegment` really is zero-sized.
const _: () = {
    assert!(
        core::mem::size_of::<TypedSegment<u64, 0>>() == 0,
        "TypedSegment must be zero-sized so it costs nothing to pass around",
    );
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_segment_is_zero_sized() {
        assert_eq!(core::mem::size_of::<TypedSegment<u64, 16>>(), 0);
    }

    #[test]
    fn typed_segment_offset_and_size_fold() {
        const S: TypedSegment<u64, 16> = TypedSegment::new();
        // The values come from the type system directly.
        assert_eq!(TypedSegment::<u64, 16>::offset(), 16);
        assert_eq!(TypedSegment::<u64, 16>::size(), 8);
        assert_eq!(TypedSegment::<u64, 16>::end(), 24);
        let _ = S; // ensure const ctor works
    }

    #[test]
    fn typed_segment_lowers_to_runtime_segment() {
        const S: Segment = TypedSegment::<u64, 16>::as_segment();
        assert_eq!(S.offset, 16);
        assert_eq!(S.size, 8);
    }

    #[test]
    fn body_adds_header() {
        let s = Segment::body(0, 8);
        assert_eq!(s.offset, HopperHeader::SIZE as u32);
        assert_eq!(s.size, 8);
        assert_eq!(s.end(), HopperHeader::SIZE as u32 + 8);
    }

    #[test]
    fn overlaps_detects_shared_bytes() {
        let a = Segment::new(0, 16);
        let b = Segment::new(8, 16);
        let c = Segment::new(16, 16);
        assert!(a.overlaps(&b));
        assert!(!a.overlaps(&c)); // adjacent, no shared bytes
        assert!(b.overlaps(&c));
    }

    #[test]
    fn contained_in_reports_proper_nesting() {
        let outer = Segment::new(0, 32);
        let inner = Segment::new(8, 8);
        let equal = Segment::new(0, 32);
        let escape = Segment::new(24, 16);
        assert!(inner.contained_in(&outer));
        assert!(equal.contained_in(&outer));
        assert!(!escape.contained_in(&outer));
    }
}
