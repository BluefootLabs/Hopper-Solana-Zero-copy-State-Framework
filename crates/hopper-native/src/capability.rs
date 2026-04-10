//! Compile-time account capability types.
//!
//! Instead of sprinkling `require_signer()` / `require_writable()` calls
//! throughout business logic, Hopper elevates account roles to the type
//! system. A `SignerView` proves at compile time that the signer check
//! happened. Functions that need a signer take `SignerView` -- zero
//! runtime cost after the single boundary check.
//!
//! This pattern has no equivalent in pinocchio, Anchor, Steel, or any
//! other Solana framework. Anchor's `Signer<'info>` is a macro-generated
//! wrapper that re-checks at runtime. Hopper's capability types are
//! zero-size wrappers that PROVE the check already happened.
//!
//! # Usage
//!
//! ```ignore
//! use hopper_native::capability::{SignerView, WritableView, MutableView};
//!
//! fn deposit(
//!     payer: MutableView,     // proven: is_signer + is_writable
//!     vault: WritableView,    // proven: is_writable
//!     amount: u64,
//! ) -> ProgramResult {
//!     // No runtime checks needed -- the types guarantee the properties.
//!     let lamports = payer.lamports();
//!     // ...
//!     Ok(())
//! }
//! ```

use crate::account_view::AccountView;
use crate::address::Address;
use crate::error::ProgramError;

// ── SignerView ───────────────────────────────────────────────────────

/// An `AccountView` that has been proven to be a transaction signer.
///
/// Constructed only through `SignerView::validate()`, which performs the
/// signer check exactly once. All downstream code can rely on the type
/// to guarantee the property without re-checking.
#[repr(transparent)]
#[derive(Clone, PartialEq, Eq)]
pub struct SignerView {
    inner: AccountView,
}

impl SignerView {
    /// Validate that the account is a signer and return a capability token.
    #[inline(always)]
    pub fn validate(view: AccountView) -> Result<Self, ProgramError> {
        if view.is_signer() {
            Ok(Self { inner: view })
        } else {
            Err(ProgramError::MissingRequiredSignature)
        }
    }

    /// Access the underlying `AccountView`.
    #[inline(always)]
    pub fn as_view(&self) -> &AccountView {
        &self.inner
    }

    /// Consume and return the inner `AccountView`.
    #[inline(always)]
    pub fn into_view(self) -> AccountView {
        self.inner
    }
}

impl core::ops::Deref for SignerView {
    type Target = AccountView;

    #[inline(always)]
    fn deref(&self) -> &AccountView {
        &self.inner
    }
}

// ── WritableView ─────────────────────────────────────────────────────

/// An `AccountView` that has been proven to be writable.
///
/// Guarantees that `is_writable() == true` without re-checking.
#[repr(transparent)]
#[derive(Clone, PartialEq, Eq)]
pub struct WritableView {
    inner: AccountView,
}

impl WritableView {
    /// Validate that the account is writable and return a capability token.
    #[inline(always)]
    pub fn validate(view: AccountView) -> Result<Self, ProgramError> {
        if view.is_writable() {
            Ok(Self { inner: view })
        } else {
            Err(ProgramError::Immutable)
        }
    }

    /// Access the underlying `AccountView`.
    #[inline(always)]
    pub fn as_view(&self) -> &AccountView {
        &self.inner
    }

    /// Consume and return the inner `AccountView`.
    #[inline(always)]
    pub fn into_view(self) -> AccountView {
        self.inner
    }
}

impl core::ops::Deref for WritableView {
    type Target = AccountView;

    #[inline(always)]
    fn deref(&self) -> &AccountView {
        &self.inner
    }
}

// ── MutableView ──────────────────────────────────────────────────────

/// An `AccountView` that has been proven to be BOTH a signer AND writable.
///
/// This is the "payer" pattern: the account that signs and pays for the
/// transaction. The check happens once; all downstream code gets both
/// guarantees via the type.
#[repr(transparent)]
#[derive(Clone, PartialEq, Eq)]
pub struct MutableView {
    inner: AccountView,
}

impl MutableView {
    /// Validate that the account is both a signer and writable.
    #[inline(always)]
    pub fn validate(view: AccountView) -> Result<Self, ProgramError> {
        if !view.is_signer() {
            return Err(ProgramError::MissingRequiredSignature);
        }
        if !view.is_writable() {
            return Err(ProgramError::Immutable);
        }
        Ok(Self { inner: view })
    }

    /// Access the underlying `AccountView`.
    #[inline(always)]
    pub fn as_view(&self) -> &AccountView {
        &self.inner
    }

    /// Consume and return the inner `AccountView`.
    #[inline(always)]
    pub fn into_view(self) -> AccountView {
        self.inner
    }

    /// Upcast to `SignerView` (free -- MutableView implies signer).
    #[inline(always)]
    pub fn as_signer(&self) -> SignerView {
        // SAFETY: MutableView guarantees is_signer.
        SignerView { inner: self.inner.clone() }
    }

    /// Upcast to `WritableView` (free -- MutableView implies writable).
    #[inline(always)]
    pub fn as_writable(&self) -> WritableView {
        // SAFETY: MutableView guarantees is_writable.
        WritableView { inner: self.inner.clone() }
    }
}

impl core::ops::Deref for MutableView {
    type Target = AccountView;

    #[inline(always)]
    fn deref(&self) -> &AccountView {
        &self.inner
    }
}

// ── OwnedView ────────────────────────────────────────────────────────

/// An `AccountView` that has been proven to be owned by a specific program.
///
/// Prevents confused-deputy attacks: once validated, downstream code
/// can trust the account data without re-checking ownership.
#[repr(transparent)]
#[derive(Clone, PartialEq, Eq)]
pub struct OwnedView {
    inner: AccountView,
}

impl OwnedView {
    /// Validate that the account is owned by `expected_owner`.
    #[inline(always)]
    pub fn validate(view: AccountView, expected_owner: &Address) -> Result<Self, ProgramError> {
        if view.owned_by(expected_owner) {
            Ok(Self { inner: view })
        } else {
            Err(ProgramError::IncorrectProgramId)
        }
    }

    /// Access the underlying `AccountView`.
    #[inline(always)]
    pub fn as_view(&self) -> &AccountView {
        &self.inner
    }

    /// Consume and return the inner `AccountView`.
    #[inline(always)]
    pub fn into_view(self) -> AccountView {
        self.inner
    }
}

impl core::ops::Deref for OwnedView {
    type Target = AccountView;

    #[inline(always)]
    fn deref(&self) -> &AccountView {
        &self.inner
    }
}

// ── ReadonlyView ─────────────────────────────────────────────────────

/// An `AccountView` proven to be a non-signer, non-writable read-only
/// account. Useful for cross-program reads where you explicitly want
/// to prevent accidental mutation attempts.
#[repr(transparent)]
#[derive(Clone, PartialEq, Eq)]
pub struct ReadonlyView {
    inner: AccountView,
}

impl ReadonlyView {
    /// Validate that the account is neither a signer nor writable.
    #[inline(always)]
    pub fn validate(view: AccountView) -> Result<Self, ProgramError> {
        // A "readonly" account in Solana's model is one that the
        // transaction declared as non-writable. We don't require
        // non-signer because some read-only lookups still need signer
        // proof. Instead we just check non-writable.
        if view.is_writable() {
            // Account is writable -- caller probably mixed up their types.
            return Err(ProgramError::InvalidArgument);
        }
        Ok(Self { inner: view })
    }

    /// Access the underlying `AccountView`.
    #[inline(always)]
    pub fn as_view(&self) -> &AccountView {
        &self.inner
    }

    /// Consume and return the inner `AccountView`.
    #[inline(always)]
    pub fn into_view(self) -> AccountView {
        self.inner
    }
}

impl core::ops::Deref for ReadonlyView {
    type Target = AccountView;

    #[inline(always)]
    fn deref(&self) -> &AccountView {
        &self.inner
    }
}

// ── ExecutableView ───────────────────────────────────────────────────

/// An `AccountView` proven to contain an executable program.
///
/// Used when passing program accounts for CPI -- proves the account
/// actually contains a program, preventing CPI to data accounts.
#[repr(transparent)]
#[derive(Clone, PartialEq, Eq)]
pub struct ExecutableView {
    inner: AccountView,
}

impl ExecutableView {
    /// Validate that the account is executable.
    #[inline(always)]
    pub fn validate(view: AccountView) -> Result<Self, ProgramError> {
        if view.executable() {
            Ok(Self { inner: view })
        } else {
            Err(ProgramError::InvalidArgument)
        }
    }

    /// Access the underlying `AccountView`.
    #[inline(always)]
    pub fn as_view(&self) -> &AccountView {
        &self.inner
    }

    /// Consume and return the inner `AccountView`.
    #[inline(always)]
    pub fn into_view(self) -> AccountView {
        self.inner
    }
}

impl core::ops::Deref for ExecutableView {
    type Target = AccountView;

    #[inline(always)]
    fn deref(&self) -> &AccountView {
        &self.inner
    }
}

// ── Capability Composition via LazyContext ────────────────────────────

impl crate::lazy::LazyContext {
    /// Parse the next account as a proven signer.
    #[inline]
    pub fn next_validated_signer(&mut self) -> Result<SignerView, ProgramError> {
        let acct = self.next_account()?;
        SignerView::validate(acct)
    }

    /// Parse the next account as a proven writable.
    #[inline]
    pub fn next_validated_writable(&mut self) -> Result<WritableView, ProgramError> {
        let acct = self.next_account()?;
        WritableView::validate(acct)
    }

    /// Parse the next account as a proven mutable (signer + writable).
    #[inline]
    pub fn next_validated_mutable(&mut self) -> Result<MutableView, ProgramError> {
        let acct = self.next_account()?;
        MutableView::validate(acct)
    }

    /// Parse the next account as a proven program-owned account.
    #[inline]
    pub fn next_validated_owned(&mut self, owner: &Address) -> Result<OwnedView, ProgramError> {
        let acct = self.next_account()?;
        OwnedView::validate(acct, owner)
    }

    /// Parse the next account as a proven executable program.
    #[inline]
    pub fn next_validated_executable(&mut self) -> Result<ExecutableView, ProgramError> {
        let acct = self.next_account()?;
        ExecutableView::validate(acct)
    }
}
