//! Modifier-style composable account wrappers.
//!
//! Account constraints as nested type wrappers that compose at compile time.
//! Each wrapper adds one constraint and delegates the rest to the inner type.
//!
//! ```text
//! Signer<Mut<Account<'a, Vault>>>  -- verified signer + writable + typed overlay
//! Mut<Account<'a, Vault>>          -- verified writable + typed overlay
//! Account<'a, Vault>               -- verified owner + disc + layout_id
//! ReadOnly<'a, Vault>              -- foreign account, read-only
//! ```
//!
//! ## Design Principles
//!
//! - Zero runtime cost: wrapper structs are transparent or ZST stacks
//! - Each modifier validates exactly one property
//! - Inner value is accessible via `.inner()` or `Deref`
//! - All validation happens in `from_account()` -- if you have the type, it's valid

use hopper_runtime::error::ProgramError;
use hopper_runtime::{AccountView, Address};
use crate::account::{Pod, FixedLayout, VerifiedAccount, VerifiedAccountMut};
use crate::check;

/// A typed, owner-validated account (immutable).
///
/// Proves: owner == program_id, disc match, version match, layout_id match, size match.
pub struct Account<'a, T: Pod + FixedLayout> {
    view: &'a AccountView,
    verified: VerifiedAccount<'a, T>,
}

impl<'a, T: Pod + FixedLayout> Account<'a, T> {
    /// The underlying AccountView.
    #[inline(always)]
    pub fn view(&self) -> &'a AccountView {
        self.view
    }

    /// The verified typed overlay.
    #[inline(always)]
    pub fn get(&self) -> &T {
        self.verified.get()
    }

    /// The raw verified data.
    #[inline(always)]
    pub fn data(&self) -> &[u8] {
        self.verified.data()
    }

    /// Project a field from the typed overlay.
    #[inline(always)]
    pub fn map<U, F>(&self, f: F) -> U
    where
        F: FnOnce(&T) -> U,
    {
        self.verified.map(f)
    }
}

/// A typed, owner-validated account (mutable).
///
/// Proves: owner == program_id, writable, disc match, layout_id match, size match.
pub struct AccountMut<'a, T: Pod + FixedLayout> {
    view: &'a AccountView,
    verified: VerifiedAccountMut<'a, T>,
}

impl<'a, T: Pod + FixedLayout> AccountMut<'a, T> {
    /// The underlying AccountView.
    #[inline(always)]
    pub fn view(&self) -> &'a AccountView {
        self.view
    }

    /// The verified typed overlay (immutable).
    #[inline(always)]
    pub fn get(&self) -> &T {
        self.verified.get()
    }

    /// The verified typed overlay (mutable).
    #[inline(always)]
    pub fn get_mut(&mut self) -> &mut T {
        self.verified.get_mut()
    }

    /// Map a field mutably.
    #[inline(always)]
    pub fn map_mut<U, F>(&mut self, f: F) -> U
    where
        F: FnOnce(&mut T) -> U,
    {
        self.verified.map_mut(f)
    }
}

/// Signer wrapper -- validates that the account is a signer.
///
/// Wraps any inner account type that provides `view()`.
pub struct Signer<I> {
    inner: I,
}

impl<I> Signer<I> {
    /// Access the inner account.
    #[inline(always)]
    pub fn inner(&self) -> &I {
        &self.inner
    }

    /// Consume the wrapper and return the inner account.
    #[inline(always)]
    pub fn into_inner(self) -> I {
        self.inner
    }
}

/// Writable wrapper -- validates that the account is writable.
pub struct Mut<I> {
    inner: I,
}

impl<I> Mut<I> {
    /// Access the inner account.
    #[inline(always)]
    pub fn inner(&self) -> &I {
        &self.inner
    }

    /// Consume the wrapper and return the inner account.
    #[inline(always)]
    pub fn into_inner(self) -> I {
        self.inner
    }
}

// -- Construction traits --

/// Trait for types that can be constructed from an AccountView with validation.
pub trait FromAccount<'a>: Sized {
    /// Construct this type from an account, performing all required validation.
    fn from_account(
        account: &'a AccountView,
        program_id: &Address,
    ) -> Result<Self, ProgramError>;
}

// Account<T>: owner + disc + version + layout_id + size
impl<'a, T: Pod + FixedLayout + HopperLayout> FromAccount<'a> for Account<'a, T> {
    #[inline]
    fn from_account(
        account: &'a AccountView,
        program_id: &Address,
    ) -> Result<Self, ProgramError> {
        check::check_owner(account, program_id)?;
        let data = account.try_borrow()?;
        crate::account::check_header(&data, T::DISC, T::VERSION, &T::LAYOUT_ID)?;
        check::check_size(&data, T::LEN_WITH_HEADER)?;
        let verified = VerifiedAccount::from_ref(data)?;
        Ok(Self { view: account, verified })
    }
}

// AccountMut<T>: owner + writable + disc + version + layout_id + size
impl<'a, T: Pod + FixedLayout + HopperLayout> FromAccount<'a> for AccountMut<'a, T> {
    #[inline]
    fn from_account(
        account: &'a AccountView,
        program_id: &Address,
    ) -> Result<Self, ProgramError> {
        check::check_owner(account, program_id)?;
        check::check_writable(account)?;
        let data = account.try_borrow_mut()?;
        crate::account::check_header(&data, T::DISC, T::VERSION, &T::LAYOUT_ID)?;
        check::check_size(&data, T::LEN_WITH_HEADER)?;
        let verified = VerifiedAccountMut::from_ref_mut(data)?;
        Ok(Self { view: account, verified })
    }
}

// Signer<I>: validates signer, then delegates to inner
impl<'a, I: FromAccount<'a> + HasView<'a>> FromAccount<'a> for Signer<I> {
    #[inline]
    fn from_account(
        account: &'a AccountView,
        program_id: &Address,
    ) -> Result<Self, ProgramError> {
        check::check_signer(account)?;
        let inner = I::from_account(account, program_id)?;
        Ok(Self { inner })
    }
}

// Mut<I>: validates writable, then delegates to inner
impl<'a, I: FromAccount<'a> + HasView<'a>> FromAccount<'a> for Mut<I> {
    #[inline]
    fn from_account(
        account: &'a AccountView,
        program_id: &Address,
    ) -> Result<Self, ProgramError> {
        check::check_writable(account)?;
        let inner = I::from_account(account, program_id)?;
        Ok(Self { inner })
    }
}

/// Helper trait for types that hold an `&AccountView`.
pub trait HasView<'a> {
    fn view(&self) -> &'a AccountView;
}

impl<'a, T: Pod + FixedLayout> HasView<'a> for Account<'a, T> {
    #[inline(always)]
    fn view(&self) -> &'a AccountView {
        self.view
    }
}

impl<'a, T: Pod + FixedLayout> HasView<'a> for AccountMut<'a, T> {
    #[inline(always)]
    fn view(&self) -> &'a AccountView {
        self.view
    }
}

impl<'a, I: HasView<'a>> HasView<'a> for Signer<I> {
    #[inline(always)]
    fn view(&self) -> &'a AccountView {
        self.inner.view()
    }
}

impl<'a, I: HasView<'a>> HasView<'a> for Mut<I> {
    #[inline(always)]
    fn view(&self) -> &'a AccountView {
        self.inner.view()
    }
}

/// Trait implemented by `hopper_layout!` types providing layout metadata.
///
/// This bridges the macro-generated constants (DISC, VERSION, LAYOUT_ID, LEN)
/// into a trait for generic consumption by modifier wrappers.
pub trait HopperLayout: Pod + FixedLayout {
    const DISC: u8;
    const VERSION: u8;
    const LAYOUT_ID: [u8; 8];
    const LEN_WITH_HEADER: usize;
}
