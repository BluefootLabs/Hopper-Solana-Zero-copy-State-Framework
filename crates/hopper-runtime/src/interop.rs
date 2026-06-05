//! Type interop for Hopper-owned address values.
//!
//! Hopper keeps its own `Address` and `AccountView` types because they
//! carry segment metadata, layout fingerprints, and borrow-tracking that
//! external types lack. This module provides `From`/`Into` conversions
//! so Hopper code can interoperate with the wider Solana ecosystem
//! without loss of type safety.
//!
//! # Zero-cost reference casts
//!
//! Hopper's `Address` is `#[repr(transparent)]` over `[u8; 32]`. This means
//! reference casts to other transparent 32-byte address wrappers are valid
//! when the caller opts into the marker trait:
//!
//! ```ignore
//! let hopper_addr: &Address = Address::from_ref(upstream_addr);
//! let upstream_ref: &[u8; 32] = hopper_addr.as_array();
//! ```
//!
//! # By-value conversions
//!
//! Hopper's runtime uses its own canonical `Address`. By-value conversions to
//! Hopper Native's address live in the direct runtime bridge.
//!
//! # Backend-agnostic conversions
//!
//! Hopper `Address` always converts to/from `[u8; 32]`, making it trivially
//! interoperable with any type that also wraps 32 bytes.

use crate::address::Address;

// ── Zero-cost reference conversions ──────────────────────────────────

impl Address {
    /// Zero-cost borrow as a reference to any `#[repr(transparent)]`
    /// 32-byte type that shares layout with `[u8; 32]`.
    ///
    /// This is the preferred way to pass a Hopper `Address` where an
    /// upstream reference is expected.
    ///
    /// # Safety
    ///
    /// Safe because `Address` is `#[repr(transparent)]` over `[u8; 32]`
    /// and any upstream 32-byte address type shares this layout.
    #[inline(always)]
    pub fn as_upstream<T>(&self) -> &T
    where
        T: TransparentAddress,
    {
        // SAFETY: Both types are #[repr(transparent)] over [u8; 32].
        unsafe { &*(self as *const Address as *const T) }
    }

    /// Construct a Hopper `Address` reference from any `#[repr(transparent)]`
    /// 32-byte address type.
    #[inline(always)]
    pub fn from_upstream<T>(upstream: &T) -> &Address
    where
        T: TransparentAddress,
    {
        // SAFETY: Both types are #[repr(transparent)] over [u8; 32].
        unsafe { &*(upstream as *const T as *const Address) }
    }
}

/// Marker trait for types that are `#[repr(transparent)]` over `[u8; 32]`.
///
/// # Safety
///
/// Implementors must be `#[repr(transparent)]` wrappers around `[u8; 32]`
/// with no additional invariants. This enables zero-cost reference casts.
pub unsafe trait TransparentAddress: Sized {}

// Hopper's own Address is trivially transparent.
unsafe impl TransparentAddress for Address {}

unsafe impl TransparentAddress for hopper_native::address::Address {}
