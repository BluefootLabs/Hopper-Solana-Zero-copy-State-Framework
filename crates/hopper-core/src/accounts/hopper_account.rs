//! Layout-bound typed account with read/write/init.
//!
//! `HopperAccount<T>` bridges the account DSL to Hopper's existing zero-copy
//! overlay infrastructure. It wraps an AccountView and provides ergonomic
//! `read()`, `write()`, and `init()` methods that delegate to the verified
//! overlay path.

use hopper_runtime::{AccountView, Address};
use hopper_runtime::error::ProgramError;

use crate::account::{Pod, FixedLayout, VerifiedAccount, VerifiedAccountMut};
use crate::check;
use crate::check::modifier::HopperLayout;

/// A layout-bound, owner-validated account.
///
/// Wraps a hopper-native `AccountView` with layout type information.
/// Access the typed data via `read()` for immutable or `write()` for mutable.
///
/// Validation is performed at construction time:
/// - Owner matches program_id
/// - Discriminator, version, layout_id match T's constants
/// - Account data size matches T's expected layout size
pub struct HopperAccount<'a, T: Pod + FixedLayout + HopperLayout> {
    view: &'a AccountView,
    #[allow(dead_code)] // stored for future PDA derivation and CPI use
    program_id: &'a Address,
    _marker: core::marker::PhantomData<T>,
}

impl<'a, T: Pod + FixedLayout + HopperLayout> HopperAccount<'a, T> {
    /// Construct from an AccountView with full header validation.
    ///
    /// Validates: owner == program_id, discriminator, version, layout_id, size.
    #[inline]
    pub fn from_account(
        account: &'a AccountView,
        program_id: &'a Address,
    ) -> Result<Self, ProgramError> {
        check::check_owner(account, program_id)?;
        let data = account.try_borrow()?;
        crate::account::check_header(&data, T::DISC, T::VERSION, &T::LAYOUT_ID)?;
        check::check_size(&data, T::LEN_WITH_HEADER)?;
        Ok(Self {
            view: account,
            program_id,
            _marker: core::marker::PhantomData,
        })
    }

    /// Construct from an AccountView that must also be writable.
    #[inline]
    pub fn from_account_mut(
        account: &'a AccountView,
        program_id: &'a Address,
    ) -> Result<Self, ProgramError> {
        check::check_writable(account)?;
        Self::from_account(account, program_id)
    }

    /// Read the typed layout overlay (immutable).
    ///
    /// Returns a reference to the typed layout struct overlaid on account data.
    ///
    /// # Safety
    ///
    /// The caller must ensure no conflicting mutable borrows exist on this
    /// account's data. Frame-level borrow tracking handles this in normal usage.
    #[inline]
    pub fn read(&self) -> Result<VerifiedAccount<'a, T>, ProgramError> {
        let data = self.view.try_borrow()?;
        VerifiedAccount::from_ref(data)
    }

    /// Write to the typed layout overlay (mutable).
    ///
    /// Returns a mutable reference to the typed layout struct.
    /// The account must have been constructed via `from_account_mut` or
    /// the caller must ensure writability.
    ///
    /// # Safety
    ///
    /// The caller must ensure exclusive access to this account's data.
    #[inline]
    pub fn write(&self) -> Result<VerifiedAccountMut<'a, T>, ProgramError> {
        let data = self.view.try_borrow_mut()?;
        VerifiedAccountMut::from_ref_mut(data)
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

    /// Current lamports on the account.
    #[inline(always)]
    pub fn lamports(&self) -> u64 {
        self.view.lamports()
    }
}
