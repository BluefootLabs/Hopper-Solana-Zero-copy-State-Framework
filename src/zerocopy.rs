//! Unified zero-copy trait family.
//!
//! The Hopper Safety Audit's "structural" recommendation was to
//! consolidate `Pod`, `FixedLayout`, `Projectable`, `SafeProjectable`,
//! `LayoutContract`, header metadata, and schema export into one
//! coherent trait stack. This module delivers the foundation:
//!
//! - [`ZeroCopy`] — the canonical "safe to overlay on raw bytes"
//!   marker. Equivalent-in-contract to [`Pod`](crate::pod::Pod), which
//!   (under the default `hopper-native-backend` + `bytemuck` features)
//!   is a sub-trait of `bytemuck::Pod + bytemuck::Zeroable`.
//!   `ZeroCopy` is implemented for every `Pod` type via a blanket
//!   impl, so existing layouts participate automatically.
//!
//! - [`WireLayout`] — a `ZeroCopy` type with a fixed wire size.
//!   Declared once via `const WIRE_SIZE = size_of::<Self>()` by
//!   default; macros may override if the in-memory and on-wire sizes
//!   diverge (none do today, but the hook is there for future
//!   compressed / tagged encodings).
//!
//! - [`AccountLayout`] — a `WireLayout` that also carries Hopper's
//!   account header identity (disc, version, wire fingerprint, schema
//!   epoch, type offset). This is the audit's proposed top-level
//!   trait, matching its exact member list so the contract is
//!   frozen-in-place for migrations and client generation.
//!
//! ## Why three traits, not one
//!
//! The layering mirrors a real capability hierarchy. Every account
//! layout is a wire layout; every wire layout is zero-copy; but not
//! every zero-copy type is a full account layout (`u64`, `WireBool`,
//! `TypedAddress<T>` are zero-copy but carry no header). Splitting
//! the traits lets generic helpers demand just what they need.
//!
//! ## Relation to `LayoutContract`
//!
//! The existing [`crate::layout::LayoutContract`] trait predates this
//! module. `LayoutContract` and `AccountLayout` intentionally overlap:
//! both describe "a Hopper layout with disc/version/layout_id".
//! `AccountLayout` is the audit-blessed name with the richer member
//! list; `LayoutContract` is kept for backward compatibility and gets
//! a blanket impl so any type deriving the latter automatically
//! satisfies the former. New authoring surfaces (the proposed
//! `#[hopper::state]` v2 expansion) should reach for `AccountLayout`.

use crate::layout::LayoutContract;
use crate::pod::Pod;

// ══════════════════════════════════════════════════════════════════════
//  ZeroCopy
// ══════════════════════════════════════════════════════════════════════

/// Canonical marker for types that may be overlaid on raw bytes.
///
/// # Safety
///
/// The contract is the same four-point obligation as [`Pod`]:
///
/// 1. Every `[u8; size_of::<T>()]` bit pattern decodes to a valid `T`.
/// 2. `align_of::<T>() == 1`.
/// 3. `T` contains no padding.
/// 4. `T` contains no internal pointers or references.
///
/// Since this trait is a blanket re-export of [`Pod`], every `Pod`
/// type automatically satisfies it — which means **every existing
/// `#[hopper::state]`-derived layout participates for free**.
pub unsafe trait ZeroCopy: Pod + 'static {}

// Blanket: every Pod + 'static type is ZeroCopy. Existing layouts
// opt in automatically with no source changes.
unsafe impl<T> ZeroCopy for T where T: Pod + 'static {}

// ══════════════════════════════════════════════════════════════════════
//  WireLayout
// ══════════════════════════════════════════════════════════════════════

/// A `ZeroCopy` type with a compile-time-known wire size.
///
/// The default associated-const body returns `size_of::<Self>()`,
/// which matches every Hopper layout today. Macros may override it
/// in a future revision if the in-memory and on-wire representations
/// ever diverge (e.g. compact trailing tags for optional fields).
pub trait WireLayout: ZeroCopy {
    /// Size of the on-wire representation, in bytes.
    const WIRE_SIZE: usize = core::mem::size_of::<Self>();
}

// Blanket: every `ZeroCopy` type gets `WireLayout` with the default
// `WIRE_SIZE`. Keeps the trait free for user code.
impl<T: ZeroCopy> WireLayout for T {}

// ══════════════════════════════════════════════════════════════════════
//  AccountLayout
// ══════════════════════════════════════════════════════════════════════

/// Hopper account layout identity — the top of the unified trait stack.
///
/// This is the audit-blessed trait: its member list matches the PDF's
/// "proposed trait model" section exactly, so Hopper's long-term ABI
/// story is anchored in the vocabulary the audit uses.
///
/// `WIRE_FINGERPRINT` is the first 8 bytes of the canonical SHA-256
/// wire descriptor (see `hopper_macros_proc::state::layout_id_bytes`)
/// reinterpreted as a little-endian `u64`, so the runtime can compare
/// against the on-account header byte-for-byte.
///
/// `SCHEMA_EPOCH` defaults to `1`; programs that publish later epochs
/// via their on-chain manifest bump it to signal a version transition.
pub trait AccountLayout: WireLayout {
    /// On-chain discriminator (header byte 0).
    const DISC: u8;
    /// Layout version (header byte 1).
    const VERSION: u8;
    /// Canonical wire fingerprint (header bytes 4..12, little-endian).
    const WIRE_FINGERPRINT: u64;
    /// Schema-evolution epoch (header bytes 12..16).
    const SCHEMA_EPOCH: u32 = 1;
    /// Offset at which `Self` starts inside the account buffer.
    /// `0` for header-inclusive layouts, `HEADER_LEN` for body-only.
    const TYPE_OFFSET: usize;

    /// Total data length an account must carry to hold `Self`.
    #[inline(always)]
    fn required_len() -> usize {
        Self::TYPE_OFFSET + Self::WIRE_SIZE
    }
}

// Blanket: every `LayoutContract` type automatically is an
// `AccountLayout`. This makes the transition source-compatible —
// `#[hopper::state]` emits `LayoutContract` today; downstream can
// reach for either trait interchangeably.
//
// Fingerprint translation: `LayoutContract::LAYOUT_ID` is already a
// `[u8; 8]` produced by the canonical wire-descriptor hash. We reinterpret
// it as a little-endian `u64` for the `WIRE_FINGERPRINT` slot.
impl<T: LayoutContract + ZeroCopy> AccountLayout for T {
    const DISC: u8 = <T as LayoutContract>::DISC;
    const VERSION: u8 = <T as LayoutContract>::VERSION;
    const WIRE_FINGERPRINT: u64 = u64::from_le_bytes(<T as LayoutContract>::LAYOUT_ID);
    const SCHEMA_EPOCH: u32 = 1;
    const TYPE_OFFSET: usize = <T as LayoutContract>::TYPE_OFFSET;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn require_zero_copy<T: ZeroCopy>() {}
    fn require_wire<T: WireLayout>() {}

    #[test]
    fn primitives_are_zero_copy_and_wire() {
        require_zero_copy::<u8>();
        require_zero_copy::<u64>();
        require_zero_copy::<[u8; 32]>();
        require_wire::<u8>();
        require_wire::<u64>();
        require_wire::<[u8; 32]>();
        assert_eq!(<u64 as WireLayout>::WIRE_SIZE, 8);
        assert_eq!(<[u8; 32] as WireLayout>::WIRE_SIZE, 32);
    }

    #[test]
    fn address_is_zero_copy() {
        require_zero_copy::<crate::address::Address>();
        assert_eq!(<crate::address::Address as WireLayout>::WIRE_SIZE, 32);
    }
}
