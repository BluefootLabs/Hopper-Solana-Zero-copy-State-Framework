//! Runtime-local segment primitive.
//!
//! `Segment` is the tiny memory-contract descriptor that every segment
//! access routes through: `{offset, size}`, 8 bytes on 32-bit accounts,
//! `Copy`, `const`-constructable, no strings, no extra fields. It is the
//! runtime counterpart to `hopper_core::segment_map::StaticSegment`
//! (which carries a human-readable name for tooling) — the runtime
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
//! `Segment` never appears in an on-chain layout — it is a compile-time
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

#[cfg(test)]
mod tests {
    use super::*;

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
