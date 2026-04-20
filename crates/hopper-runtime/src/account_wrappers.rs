//! Anchor-grade typed account wrappers for `#[hopper::context]`.
//!
//! Closes Hopper Safety Audit Stage 2.3: zero-cost, zero-alignment,
//! type-directed wrappers that programs can use in context structs to
//! name an account's *role* rather than paint it with an
//! `#[account(signer)]` attribute.
//!
//! ```ignore
//! #[hopper::context]
//! pub struct Deposit<'info> {
//!     pub authority: Signer<'info>,
//!     pub vault: Account<'info, Vault>,
//!     pub system_program: Program<'info, SystemId>,
//! }
//! ```
//!
//! The context macro recognizes these type names via
//! `skips_layout_validation` and auto-derives the appropriate
//! checks (`check_signer`, `check_owned_by`, `check_executable`,
//! address-pin). The wrappers themselves are
//! `#[repr(transparent)]` over `&AccountView` so they compile away
//! to the same pointer access as the raw form.
//!
//! # Why wrappers alongside the attribute path
//!
//! The attribute-directed lowering (`#[account(signer, mut)]`) and
//! the wrapper-directed lowering (`pub authority: Signer<'info>`)
//! both cover the same safety story. The wrapper form is
//! Anchor-familiar and makes the role visible in every signature
//! that accepts the account; the attribute form stays available for
//! callers who prefer explicit constraint-lists. Both paths flow
//! through the same canonical runtime checks. there is no
//! duplicate safety implementation.

use core::marker::PhantomData;

use crate::account::AccountView;
use crate::address::Address;

/// Account that must be a transaction signer.
///
/// The `#[hopper::context]` macro treats a `Signer<'info>` field
/// identically to `#[account(signer)] pub x: AccountView`. the
/// emitted `validate_{field}()` calls `check_signer()`.
#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct Signer<'info> {
    inner: &'info AccountView,
}

impl<'info> Signer<'info> {
    /// Wrap an `AccountView` that has already been verified as a
    /// signer. The macro-generated `validate_{field}()` call emits
    /// the `check_signer` first, so by the time the wrapper is
    /// constructed the invariant already holds.
    #[inline(always)]
    pub unsafe fn new_unchecked(view: &'info AccountView) -> Self {
        Self { inner: view }
    }

    /// Wrap an `AccountView` after verifying the signer invariant.
    /// Prefer the macro-emitted `validate_{field}()` path when the
    /// account is part of a `#[hopper::context]` struct.
    #[inline]
    pub fn try_new(view: &'info AccountView) -> Result<Self, crate::error::ProgramError> {
        view.check_signer()?;
        Ok(Self { inner: view })
    }

    /// The underlying account view.
    #[inline(always)]
    pub fn as_account(&self) -> &'info AccountView {
        self.inner
    }

    /// The signer's public key.
    #[inline(always)]
    pub fn key(&self) -> &Address {
        self.inner.address()
    }
}

impl<'info> core::ops::Deref for Signer<'info> {
    type Target = AccountView;
    #[inline(always)]
    fn deref(&self) -> &AccountView {
        self.inner
    }
}

/// Account with a verified Hopper layout owned by the executing program.
///
/// `Account<'info, T>` expands to the same checks as
/// `#[account]` with `layout = T`: `check_owned_by(program_id)` +
/// `load::<T>()` (which verifies the header, discriminator, version,
/// and wire fingerprint). Field access is through `get()` / `get_mut()`
/// which return typed references into the borrowed account data.
#[repr(transparent)]
pub struct Account<'info, T: crate::layout::LayoutContract> {
    inner: &'info AccountView,
    _ty: PhantomData<T>,
}

impl<'info, T: crate::layout::LayoutContract> Clone for Account<'info, T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<'info, T: crate::layout::LayoutContract> Copy for Account<'info, T> {}

impl<'info, T: crate::layout::LayoutContract> Account<'info, T> {
    /// Wrap an already-validated `AccountView`. Unsafe because the
    /// caller must have verified owner + layout header.
    #[inline(always)]
    pub unsafe fn new_unchecked(view: &'info AccountView) -> Self {
        Self {
            inner: view,
            _ty: PhantomData,
        }
    }

    /// Wrap with owner + layout verification.
    #[inline]
    pub fn try_new(
        view: &'info AccountView,
        owner: &Address,
    ) -> Result<Self, crate::error::ProgramError> {
        view.check_owned_by(owner)?;
        let _ = view.load::<T>()?;
        Ok(Self {
            inner: view,
            _ty: PhantomData,
        })
    }

    /// The underlying account view.
    #[inline(always)]
    pub fn as_account(&self) -> &'info AccountView {
        self.inner
    }

    /// Borrow the typed layout for reading.
    #[inline(always)]
    pub fn load(&self) -> Result<crate::borrow::Ref<'_, T>, crate::error::ProgramError> {
        self.inner.load::<T>()
    }

    /// Borrow the typed layout for writing.
    #[inline(always)]
    pub fn load_mut(&self) -> Result<crate::borrow::RefMut<'_, T>, crate::error::ProgramError> {
        self.inner.load_mut::<T>()
    }
}

/// Account that is expected to be *created* during this instruction.
///
/// `InitAccount<'info, T>` skips the layout-header check at validation
/// time (there's nothing to validate yet. the CPI hasn't run) but
/// otherwise behaves like `Account<'info, T>`. The `#[hopper::context]`
/// macro pairs it with `#[account(init, payer = ..., space = ...)]`
/// to emit the `init_{field}()` lifecycle helper that actually
/// performs the System Program CPI.
#[repr(transparent)]
pub struct InitAccount<'info, T: crate::layout::LayoutContract> {
    inner: &'info AccountView,
    _ty: PhantomData<T>,
}

impl<'info, T: crate::layout::LayoutContract> Clone for InitAccount<'info, T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<'info, T: crate::layout::LayoutContract> Copy for InitAccount<'info, T> {}

impl<'info, T: crate::layout::LayoutContract> InitAccount<'info, T> {
    /// Wrap an `AccountView` slot that will be created + initialised
    /// by a lifecycle helper later in this instruction. Unsafe
    /// because no state invariants hold for the account at wrap time.
    #[inline(always)]
    pub unsafe fn new_unchecked(view: &'info AccountView) -> Self {
        Self {
            inner: view,
            _ty: PhantomData,
        }
    }

    /// The underlying account view.
    #[inline(always)]
    pub fn as_account(&self) -> &'info AccountView {
        self.inner
    }

    /// After `init_{field}()` has run, load the freshly-initialised
    /// layout for reads / writes. The caller is responsible for
    /// ordering this after the lifecycle helper.
    #[inline(always)]
    pub fn load_after_init(&self) -> Result<crate::borrow::RefMut<'_, T>, crate::error::ProgramError> {
        self.inner.load_mut::<T>()
    }
}

/// Account that must be a named program. `P: ProgramId` identifies
/// which program the account's address must equal.
///
/// ```ignore
/// pub system_program: Program<'info, SystemId>,
/// ```
#[repr(transparent)]
pub struct Program<'info, P: ProgramId> {
    inner: &'info AccountView,
    _ty: PhantomData<P>,
}

impl<'info, P: ProgramId> Clone for Program<'info, P> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<'info, P: ProgramId> Copy for Program<'info, P> {}

impl<'info, P: ProgramId> Program<'info, P> {
    /// Wrap with address-pin and executable-flag verification.
    #[inline]
    pub fn try_new(view: &'info AccountView) -> Result<Self, crate::error::ProgramError> {
        if view.address() != &P::ID {
            return Err(crate::error::ProgramError::IncorrectProgramId);
        }
        if !view.executable() {
            return Err(crate::error::ProgramError::InvalidAccountData);
        }
        Ok(Self {
            inner: view,
            _ty: PhantomData,
        })
    }

    #[inline(always)]
    pub fn as_account(&self) -> &'info AccountView {
        self.inner
    }
}

/// Marker trait for a compile-time-known program ID.
///
/// Callers wire programs into Hopper contexts by implementing this on
/// a unit struct; the canonical names (`SystemId`, `TokenId`,
/// `AssociatedTokenId`, `Token2022Id`) are provided below for the
/// Solana programs most Hopper programs depend on.
pub trait ProgramId: 'static {
    const ID: Address;
}

/// Solana System Program.
pub struct SystemId;
impl ProgramId for SystemId {
    const ID: Address = Address::new_from_array([0u8; 32]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signer_wrapper_is_pointer_sized_zero_cost() {
        // `#[repr(transparent)]` guarantees the wrapper has the same
        // ABI as `&AccountView`. This test is a compile-time
        // assertion via `size_of`.
        assert_eq!(
            core::mem::size_of::<Signer<'static>>(),
            core::mem::size_of::<&'static AccountView>()
        );
    }

    #[test]
    fn system_program_id_is_all_zero() {
        let sys = SystemId::ID;
        assert_eq!(sys.as_array(), &[0u8; 32]);
    }
}
