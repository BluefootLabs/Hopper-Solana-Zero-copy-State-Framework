//! Generic program-owned account wrapper.
//!
//! Used for accounts owned by external programs (e.g. SPL Token accounts,
//! Mint accounts). Validates owner and provides raw typed overlay access.

use hopper_runtime::{AccountView, Address};
use hopper_runtime::error::ProgramError;

use crate::account::{Pod, FixedLayout, VerifiedAccount};
use crate::check;

/// A generic program-owned account.
///
/// Unlike `HopperAccount<T>` which validates against the executing program,
/// `ProgramAccount<T>` validates against an arbitrary expected owner.
/// Used for external program accounts (token accounts, mints, etc.).
pub struct ProgramAccount<'a, T: Pod + FixedLayout> {
    view: &'a AccountView,
    _marker: core::marker::PhantomData<T>,
}

impl<'a, T: Pod + FixedLayout> ProgramAccount<'a, T> {
    /// Construct from an AccountView, validating the owner.
    #[inline]
    pub fn from_account(
        account: &'a AccountView,
        expected_owner: &Address,
    ) -> Result<Self, ProgramError> {
        check::check_owner(account, expected_owner)?;
        let data = account.try_borrow()?;
        check::check_size(&data, core::mem::size_of::<T>())?;
        Ok(Self {
            view: account,
            _marker: core::marker::PhantomData,
        })
    }

    /// Construct without owner validation (caller's responsibility).
    #[inline]
    pub fn from_account_unchecked(account: &'a AccountView) -> Self {
        Self {
            view: account,
            _marker: core::marker::PhantomData,
        }
    }

    /// Read the typed overlay (immutable).
    ///
    /// # Safety
    ///
    /// Caller must ensure no conflicting mutable borrows.
    #[inline]
    pub fn read(&self) -> Result<VerifiedAccount<'a, T>, ProgramError> {
        let data = self.view.try_borrow()?;
        VerifiedAccount::from_ref(data)
    }

    /// The account's address.
    #[inline(always)]
    pub fn address(&self) -> &Address {
        self.view.address()
    }

    /// The account's owner.
    ///
    /// # Safety
    ///
    /// Caller must ensure no conflicting mutable borrows on the account.
    #[inline(always)]
    pub unsafe fn owner(&self) -> &Address {
        // SAFETY: Caller guarantees no conflicting borrows.
        unsafe { self.view.owner() }
    }

    /// The underlying AccountView.
    #[inline(always)]
    pub fn to_account_view(&self) -> &'a AccountView {
        self.view
    }
}
