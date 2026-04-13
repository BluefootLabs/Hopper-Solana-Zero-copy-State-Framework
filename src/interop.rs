//! Cross-framework type interop for Hopper.
//!
//! Hopper keeps its own `Address` and `AccountView` types because they
//! carry segment metadata, layout fingerprints, and borrow-tracking that
//! external types lack. This module provides `From`/`Into` conversions
//! so Hopper code can interoperate with the wider Solana ecosystem
//! without loss of type safety.
//!
//! # Zero-cost reference casts
//!
//! Both Hopper's `Address` and the upstream types (`pinocchio::Address`,
//! `solana_program::pubkey::Pubkey`) are `#[repr(transparent)]` over
//! `[u8; 32]`. This means reference casts are valid and zero-cost:
//!
//! ```ignore
//! let hopper_addr: &Address = Address::from_ref(upstream_addr);
//! let upstream_ref: &[u8; 32] = hopper_addr.as_array();
//! ```
//!
//! # By-value conversions
//!
//! `From`/`Into` impls are provided automatically via the active backend.
//! When `pinocchio-backend` is enabled, `From<pinocchio::Address>` and
//! `From<Address> for pinocchio::Address` are available. When
//! `solana-program-backend` is enabled, the same exists for `Pubkey`.
//!
//! # Backend-agnostic conversions
//!
//! Regardless of backend, Hopper `Address` always converts to/from
//! `[u8; 32]`, making it trivially interoperable with any type that
//! also wraps 32 bytes.

use crate::address::Address;

// ── Zero-cost reference conversions ──────────────────────────────────

impl Address {
    /// Zero-cost borrow as a reference to any `#[repr(transparent)]`
    /// 32-byte type that shares layout with `[u8; 32]`.
    ///
    /// This is the preferred way to pass a Hopper `Address` where an
    /// upstream reference is expected (e.g. `&pinocchio::Address` or
    /// `&Pubkey`).
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

#[cfg(feature = "pinocchio-backend")]
unsafe impl TransparentAddress for pinocchio::pubkey::Pubkey {}

#[cfg(feature = "solana-program-backend")]
unsafe impl TransparentAddress for ::solana_program::pubkey::Pubkey {}

// ── By-value conversions (re-documented for discoverability) ─────────
//
// The actual From/Into impls live in the compat modules (compat/pinocchio.rs,
// compat/solana_program.rs) where backend types are in scope. This module
// just makes them discoverable and documents the interop story.
//
// Available conversions by backend:
//
// pinocchio-backend:
//   From<pinocchio::Address>     for Address
//   From<Address>                for pinocchio::Address
//
// solana-program-backend:
//   From<solana_program::Pubkey> for Address
//   From<Address>                for solana_program::Pubkey
//
// hopper-native-backend:
//   From<hopper_native::Address> for Address  (via [u8; 32])
//   From<Address> for hopper_native::Address  (via [u8; 32])

// ── hopper-native backend conversions ────────────────────────────────
//
// From/Into impls for hopper_native::Address <-> Address already live
// in compat/native.rs. We only add the TransparentAddress marker here.

#[cfg(feature = "hopper-native-backend")]
unsafe impl TransparentAddress for hopper_native::address::Address {}
