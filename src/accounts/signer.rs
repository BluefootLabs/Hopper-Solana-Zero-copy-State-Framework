//! Verified signer account wrapper.
//!
//! A thin wrapper proving that the account is a signer. Does not require
//! any layout or owner validation -- just the signer flag. Used for
//! authority/payer accounts in instruction contexts.

use hopper_runtime::{AccountView, Address};
use hopper_runtime::error::ProgramError;

use crate::check;
use super::traits::ValidateAccount;

/// A verified signer account.
///
/// Construction validates `is_signer`. After construction, the signer
/// property is proven by the type system.
#[derive(Clone, Copy)]
pub struct SignerAccount<'a> {
    view: &'a AccountView,
}

impl<'a> SignerAccount<'a> {
    /// Construct from an AccountView, validating the signer flag.
    #[inline]
    pub fn from_account(account: &'a AccountView) -> Result<Self, ProgramError> {
        check::check_signer(account)?;
        Ok(Self { view: account })
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

    /// Whether the account is also writable.
    #[inline(always)]
    pub fn is_writable(&self) -> bool {
        self.view.is_writable()
    }

    /// Current lamports.
    #[inline(always)]
    pub fn lamports(&self) -> u64 {
        self.view.lamports()
    }
}

impl<'a> ValidateAccount for SignerAccount<'a> {
    #[inline]
    fn validate(&self) -> Result<(), ProgramError> {
        check::check_signer(self.view)
    }
}
