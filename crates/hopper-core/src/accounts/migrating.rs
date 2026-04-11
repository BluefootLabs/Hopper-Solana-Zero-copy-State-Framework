//! Dual-layout migration account wrapper.
//!
//! Wraps an account that is transitioning from one layout version to another.
//! Provides access to the old layout for reading and the new layout for writing
//! after migration.

use hopper_runtime::{AccountView, Address};
use hopper_runtime::error::ProgramError;

use crate::account::{Pod, FixedLayout, VerifiedAccount, VerifiedAccountMut};
use crate::check;
use crate::check::modifier::HopperLayout;
use crate::migrate::MigrationKind;

/// An account undergoing migration from layout `From` to layout `To`.
///
/// Construction validates that the account currently holds a `From` layout.
/// After calling `migrate_append()` or performing manual migration, the
/// caller can access the `To` layout via `into_latest()`.
pub struct MigratingAccount<'a, From, To>
where
    From: Pod + FixedLayout + HopperLayout,
    To: Pod + FixedLayout + HopperLayout,
{
    view: &'a AccountView,
    program_id: &'a Address,
    _from: core::marker::PhantomData<From>,
    _to: core::marker::PhantomData<To>,
}

impl<'a, From, To> MigratingAccount<'a, From, To>
where
    From: Pod + FixedLayout + HopperLayout,
    To: Pod + FixedLayout + HopperLayout,
{
    /// Construct from an AccountView, validating it holds the `From` layout.
    #[inline]
    pub fn from_account(
        account: &'a AccountView,
        program_id: &'a Address,
    ) -> Result<Self, ProgramError> {
        check::check_owner(account, program_id)?;
        check::check_writable(account)?;
        let data = account.try_borrow()?;
        crate::account::check_header(&data, From::DISC, From::VERSION, &From::LAYOUT_ID)?;
        check::check_size(&data, From::LEN_WITH_HEADER)?;
        Ok(Self {
            view: account,
            program_id,
            _from: core::marker::PhantomData,
            _to: core::marker::PhantomData,
        })
    }

    /// Read the old layout (immutable).
    #[inline]
    pub fn old(&self) -> Result<VerifiedAccount<'a, From>, ProgramError> {
        let data = self.view.try_borrow()?;
        VerifiedAccount::from_ref(data)
    }

    /// Read the old layout (mutable) for in-place transformation.
    #[inline]
    pub fn old_mut(&self) -> Result<VerifiedAccountMut<'a, From>, ProgramError> {
        let data = self.view.try_borrow_mut()?;
        VerifiedAccountMut::from_ref_mut(data)
    }

    /// Access the new layout after migration has been applied.
    ///
    /// The caller must have already performed the migration (realloc + header
    /// update) before calling this. The header is re-validated against `To`.
    #[inline]
    pub fn into_latest(&self) -> Result<VerifiedAccountMut<'a, To>, ProgramError> {
        let data = self.view.try_borrow_mut()?;
        crate::account::check_header(&data, To::DISC, To::VERSION, &To::LAYOUT_ID)?;
        check::check_size(&data, To::LEN_WITH_HEADER)?;
        VerifiedAccountMut::from_ref_mut(data)
    }

    /// Perform an append migration in-place using the existing migration helper.
    ///
    /// After this returns successfully, `into_latest()` will succeed.
    #[inline]
    pub fn migrate_append(&self, payer: &AccountView) -> Result<(), ProgramError> {
        crate::migrate::migrate_append(
            self.view,
            payer,
            self.program_id,
            &From::LAYOUT_ID,
            To::VERSION,
            &To::LAYOUT_ID,
            To::DISC,
            To::LEN_WITH_HEADER,
        )
    }

    /// Determine what kind of migration is needed.
    ///
    /// Append migration is valid when the new layout is strictly larger
    /// and shares the same base prefix as the old layout.
    #[inline]
    pub fn migration_kind(&self) -> MigrationKind {
        if To::LEN_WITH_HEADER > From::LEN_WITH_HEADER
            && To::DISC == From::DISC
        {
            MigrationKind::Append
        } else {
            MigrationKind::Full
        }
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
}
