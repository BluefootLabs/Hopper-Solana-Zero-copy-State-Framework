//! Alignment-1 little-endian ABI wire types.
//!
//! All types are `#[repr(transparent)]` over `[u8; N]` with `align_of == 1`.
//! This guarantees safe zero-copy overlay from any byte boundary in Solana
//! account data without alignment padding.
//!
//! Unlike raw integer types (which have alignment requirements matching their
//! size), these wire types can be placed at any offset in a `#[repr(C)]` struct
//! and will never introduce padding bytes.

mod integers;
mod boolean;
mod field_ref;
mod typed_address;

pub use boolean::WireBool;
pub use field_ref::{FieldMut, FieldRef};
pub use integers::{
    WireI16, WireI32, WireI64, WireI128,
    WireU16, WireU32, WireU64, WireU128,
};
pub use typed_address::{
    TypedAddress, UntypedAddress,
    Authority, Mint, TokenAccount, Token, Program,
};

/// Marker trait for types safe to use as zero-copy wire fields.
///
/// # Safety
///
/// Implementors must guarantee:
/// - `align_of::<Self>() == 1`
/// - `size_of::<Self>() == Self::WIRE_SIZE`
/// - All bit patterns are valid (no invalid states)
/// - The type is `Copy` and has no drop glue
pub unsafe trait WireType: Copy + Sized {
    /// Byte size of this wire type on the wire.
    const WIRE_SIZE: usize;

    /// The canonical type name for schema/fingerprint generation.
    const CANONICAL_NAME: &'static str;
}

// -- Layout Fingerprint --

/// An 8-byte deterministic layout fingerprint.
///
/// Generated from `SHA-256("hopper:v1:" + name + ":" + version + ":" + fields)[..8]`.
/// Two layouts with identical fields, types, sizes, and ordering produce
/// the same fingerprint. Any structural change produces a different one.
///
/// Use this to assert compatibility between layout versions at compile time,
/// verify on-chain accounts match expected schemas, and detect schema drift
/// in migration paths.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct LayoutFingerprint {
    bytes: [u8; 8],
}

impl LayoutFingerprint {
    /// Create a fingerprint from raw bytes.
    #[inline(always)]
    pub const fn from_bytes(bytes: [u8; 8]) -> Self {
        Self { bytes }
    }

    /// Raw fingerprint bytes.
    #[inline(always)]
    pub const fn as_bytes(&self) -> &[u8; 8] {
        &self.bytes
    }

    /// Check if two fingerprints match.
    #[inline(always)]
    pub const fn matches(&self, other: &LayoutFingerprint) -> bool {
        let a = &self.bytes;
        let b = &other.bytes;
        a[0] == b[0] && a[1] == b[1] && a[2] == b[2] && a[3] == b[3]
            && a[4] == b[4] && a[5] == b[5] && a[6] == b[6] && a[7] == b[7]
    }

    /// Check if two fingerprints differ (schema changed between versions).
    #[inline(always)]
    pub const fn differs_from(&self, other: &LayoutFingerprint) -> bool {
        !self.matches(other)
    }

    /// Verify this fingerprint matches data read from an account header.
    ///
    /// Reads the layout_id from bytes 4..12 of the header and compares.
    #[inline]
    pub fn verify_header(&self, data: &[u8]) -> Result<(), hopper_runtime::error::ProgramError> {
        if data.len() < 12 {
            return Err(hopper_runtime::error::ProgramError::AccountDataTooSmall);
        }
        let mut id = [0u8; 8];
        id.copy_from_slice(&data[4..12]);
        if id != self.bytes {
            return Err(hopper_runtime::error::ProgramError::InvalidAccountData);
        }
        Ok(())
    }
}

/// Pair of fingerprints for asserting version transitions.
///
/// Used in migration paths to prove that a layout_id changed between
/// the old and new version (required for safe append-only evolution).
pub struct FingerprintTransition {
    pub from: LayoutFingerprint,
    pub to: LayoutFingerprint,
}

impl FingerprintTransition {
    /// Create a transition pair.
    #[inline(always)]
    pub const fn new(from: LayoutFingerprint, to: LayoutFingerprint) -> Self {
        Self { from, to }
    }

    /// Assert the transition is valid: fingerprints must differ.
    /// Call this as a `const` assertion in your migration code.
    #[inline(always)]
    pub const fn assert_valid(&self) {
        assert!(
            self.from.differs_from(&self.to),
            "Layout fingerprints must differ between versions"
        );
    }
}
