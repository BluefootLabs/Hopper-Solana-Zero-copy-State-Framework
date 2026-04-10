//! Unchecked account wrapper -- no validation at construction.
//!
//! Used for accounts whose validity is the program's responsibility.
//! Passes through the raw AccountView for arbitrary inspection.

use hopper_runtime::{AccountView, Address};

/// An unchecked account. No validation is performed.
///
/// Use when the program must inspect the account manually before deciding
/// what to do (e.g. conditional logic based on owner or data).
#[derive(Clone, Copy)]
pub struct UncheckedAccount<'a> {
    view: &'a AccountView,
}

impl<'a> UncheckedAccount<'a> {
    /// Wrap an account without validation.
    #[inline(always)]
    pub fn new(account: &'a AccountView) -> Self {
        Self { view: account }
    }

    /// The account's address.
    #[inline(always)]
    pub fn address(&self) -> &Address {
        self.view.address()
    }

    /// The underlying AccountView.
    #[inline(always)]
    pub fn to_account_view(&self) -> &'a AccountView {
        self.view
    }

    /// Whether the account is a signer.
    #[inline(always)]
    pub fn is_signer(&self) -> bool {
        self.view.is_signer()
    }

    /// Whether the account is writable.
    #[inline(always)]
    pub fn is_writable(&self) -> bool {
        self.view.is_writable()
    }

    /// The account owner.
    ///
    /// # Safety
    ///
    /// Caller must ensure no conflicting mutable borrows on the account.
    #[inline(always)]
    pub unsafe fn owner(&self) -> &Address {
        // SAFETY: Caller guarantees no conflicting borrows.
        unsafe { self.view.owner() }
    }
}
