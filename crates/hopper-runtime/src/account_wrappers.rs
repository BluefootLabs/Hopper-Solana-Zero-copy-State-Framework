//! Anchor-grade typed account wrappers for `#[derive(Accounts)]` and Hopper
//! context lowering.
//!
//! Closes Hopper Safety Audit Stage 2.3: zero-cost, zero-alignment,
//! type-directed wrappers that programs can use in context structs to
//! name an account's *role* rather than paint it with an
//! `#[account(signer)]` attribute.
//!
//! ```ignore
//! #[derive(Accounts)]
//! pub struct Deposit<'info> {
//!     pub authority: Signer<'info>,
//!     pub vault: Account<'info, Vault>,
//!     pub system_program: Program<'info, SystemId>,
//! }
//! ```
//!
//! The derive/context lowering recognizes these type names via
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
/// Hopper's `#[derive(Accounts)]` / context lowering treats a `Signer<'info>`
/// field identically to `#[account(signer)] pub x: AccountView`. The emitted
/// validation calls `check_signer()`.
#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct Signer<'info> {
    inner: &'info AccountView<'info>,
}

impl<'info> Signer<'info> {
    /// Wrap an `AccountView` that has already been verified as a
    /// signer. The macro-generated `validate_{field}()` call emits
    /// the `check_signer` first, so by the time the wrapper is
    /// constructed the invariant already holds.
    #[inline(always)]
    ///
    /// # Safety
    ///
    /// Caller must uphold the invariants documented for this unsafe API before invoking it.
    pub unsafe fn new_unchecked(view: &'info AccountView<'info>) -> Self {
        Self { inner: view }
    }

    /// Wrap an `AccountView` after verifying the signer invariant.
    /// Prefer the macro-emitted `validate_{field}()` path when the
    /// account is part of a `#[derive(Accounts)]` context struct.
    #[inline]
    pub fn try_new(view: &'info AccountView<'info>) -> Result<Self, crate::error::ProgramError> {
        view.check_signer()?;
        Ok(Self { inner: view })
    }

    /// The underlying account view.
    #[inline(always)]
    pub fn as_account(&self) -> &'info AccountView<'info> {
        self.inner
    }

    /// The signer's public key.
    #[inline(always)]
    pub fn key(&self) -> &Address {
        self.inner.address()
    }
}

impl<'info> core::ops::Deref for Signer<'info> {
    type Target = AccountView<'info>;
    #[inline(always)]
    fn deref(&self) -> &AccountView<'info> {
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
pub struct Account<'info, T: crate::layout::LayoutContract + crate::Pod> {
    inner: &'info AccountView<'info>,
    _ty: PhantomData<T>,
}

impl<'info, T: crate::layout::LayoutContract + crate::Pod> Clone for Account<'info, T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<'info, T: crate::layout::LayoutContract + crate::Pod> Copy for Account<'info, T> {}

impl<'info, T: crate::layout::LayoutContract + crate::Pod> Account<'info, T> {
    /// Wrap an already-validated `AccountView`. Unsafe because the
    /// caller must have verified owner + layout header.
    #[inline(always)]
    ///
    /// # Safety
    ///
    /// Caller must uphold the invariants documented for this unsafe API before invoking it.
    pub unsafe fn new_unchecked(view: &'info AccountView<'info>) -> Self {
        Self {
            inner: view,
            _ty: PhantomData,
        }
    }

    /// Wrap with owner + layout verification.
    #[inline]
    pub fn try_new(
        view: &'info AccountView<'info>,
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
    pub fn as_account(&self) -> &'info AccountView<'info> {
        self.inner
    }

    /// The account public key.
    #[inline(always)]
    pub fn key(&self) -> &Address {
        self.inner.address()
    }

    /// Borrow the typed layout for reading.
    #[inline(always)]
    pub fn load(&self) -> Result<crate::borrow::Ref<'_, T>, crate::error::ProgramError> {
        self.inner.load::<T>()
    }

    /// Friendly alias for [`Self::load`].
    #[inline(always)]
    pub fn get(&self) -> Result<crate::borrow::Ref<'_, T>, crate::error::ProgramError> {
        self.load()
    }

    /// Borrow the typed layout for the duration of a closure.
    ///
    /// The borrow guard stays scoped to the closure, so handlers can keep the
    /// Quasar/Anchor-simple shape without bypassing Hopper's validation path.
    #[inline]
    pub fn with<R, F>(&self, f: F) -> Result<R, crate::error::ProgramError>
    where
        F: FnOnce(&T) -> Result<R, crate::error::ProgramError>,
    {
        self.inner.with::<T, R, F>(f)
    }

    /// Borrow the typed layout for writing.
    ///
    /// This takes `&self` because `Account<'info, T>` is a transparent
    /// role wrapper over `AccountView`. Mutable exclusivity is enforced
    /// by the account data borrow guard and Hopper borrow registry, so
    /// copying the wrapper cannot create aliased writable access.
    #[inline(always)]
    pub fn load_mut(&self) -> Result<crate::borrow::RefMut<'_, T>, crate::error::ProgramError> {
        self.inner.load_mut::<T>()
    }

    /// Friendly alias for [`Self::load_mut`].
    #[inline(always)]
    pub fn get_mut(&self) -> Result<crate::borrow::RefMut<'_, T>, crate::error::ProgramError> {
        self.load_mut()
    }

    /// Mutably borrow the typed layout for the duration of a closure.
    ///
    /// This is the first-touch mutation sugar:
    /// `ctx.accounts.counter.with_mut(|counter| counter.value.checked_add_assign(1))?;`
    #[inline]
    pub fn with_mut<R, F>(&self, f: F) -> Result<R, crate::error::ProgramError>
    where
        F: FnOnce(&mut T) -> Result<R, crate::error::ProgramError>,
    {
        self.inner.with_mut::<T, R, F>(f)
    }
}

impl<'info, T: crate::layout::LayoutContract + crate::Pod> core::ops::Deref for Account<'info, T> {
    type Target = AccountView<'info>;

    #[inline(always)]
    fn deref(&self) -> &AccountView<'info> {
        self.inner
    }
}

/// Account that is expected to be *created* during this instruction.
///
/// `InitAccount<'info, T>` skips the layout-header check at validation
/// time (there's nothing to validate yet. the CPI hasn't run) but
/// otherwise behaves like `Account<'info, T>`. Hopper context lowering pairs it
/// with `#[account(init, payer = ..., space = ...)]` to emit the
/// `init_{field}()` lifecycle helper that actually performs the System Program
/// CPI.
#[repr(transparent)]
pub struct InitAccount<'info, T: crate::layout::LayoutContract + crate::Pod> {
    inner: &'info AccountView<'info>,
    _ty: PhantomData<T>,
}

impl<'info, T: crate::layout::LayoutContract + crate::Pod> Clone for InitAccount<'info, T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<'info, T: crate::layout::LayoutContract + crate::Pod> Copy for InitAccount<'info, T> {}

impl<'info, T: crate::layout::LayoutContract + crate::Pod> InitAccount<'info, T> {
    /// Wrap an `AccountView` slot that will be created + initialised
    /// by a lifecycle helper later in this instruction. Unsafe
    /// because no state invariants hold for the account at wrap time.
    #[inline(always)]
    ///
    /// # Safety
    ///
    /// Caller must uphold the invariants documented for this unsafe API before invoking it.
    pub unsafe fn new_unchecked(view: &'info AccountView<'info>) -> Self {
        Self {
            inner: view,
            _ty: PhantomData,
        }
    }

    /// The underlying account view.
    #[inline(always)]
    pub fn as_account(&self) -> &'info AccountView<'info> {
        self.inner
    }

    /// The account public key.
    #[inline(always)]
    pub fn key(&self) -> &Address {
        self.inner.address()
    }

    /// After `init_{field}()` has run, load the freshly-initialised
    /// layout for reads / writes. The caller is responsible for
    /// ordering this after the lifecycle helper.
    #[inline(always)]
    pub fn load_after_init(
        &self,
    ) -> Result<crate::borrow::RefMut<'_, T>, crate::error::ProgramError> {
        self.inner.load_mut::<T>()
    }

    /// Friendly alias for [`Self::load_after_init`].
    #[inline(always)]
    pub fn get_mut_after_init(
        &self,
    ) -> Result<crate::borrow::RefMut<'_, T>, crate::error::ProgramError> {
        self.load_after_init()
    }

    /// Anchor-compatible alias for [`Self::load_after_init`].
    #[inline(always)]
    pub fn load_init(&self) -> Result<crate::borrow::RefMut<'_, T>, crate::error::ProgramError> {
        self.load_after_init()
    }

    /// Mutably borrow the freshly-initialised layout for the duration of a closure.
    #[inline]
    pub fn with_mut_after_init<R, F>(&self, f: F) -> Result<R, crate::error::ProgramError>
    where
        F: FnOnce(&mut T) -> Result<R, crate::error::ProgramError>,
    {
        self.inner.with_mut::<T, R, F>(f)
    }
}

impl<'info, T: crate::layout::LayoutContract + crate::Pod> core::ops::Deref for InitAccount<'info, T> {
    type Target = AccountView<'info>;

    #[inline(always)]
    fn deref(&self) -> &AccountView<'info> {
        self.inner
    }
}

/// Account with no role or layout validation.
///
/// Use this when a context needs a raw account in the `ctx.accounts.*`
/// facade while keeping the role explicit in the type signature. Add
/// field-level constraints such as `#[account(mut)]`, `owner = ...`, or
/// `address = ...` when the account must satisfy additional checks.
#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct UncheckedAccount<'info> {
    inner: &'info AccountView<'info>,
}

impl<'info> UncheckedAccount<'info> {
    /// Wrap without validation.
    #[inline(always)]
    ///
    /// # Safety
    ///
    /// Caller must ensure any required invariants are checked elsewhere.
    pub unsafe fn new_unchecked(view: &'info AccountView<'info>) -> Self {
        Self { inner: view }
    }

    /// Wrap without validation. This is intentionally explicit at the
    /// type level: `UncheckedAccount` means no role has been proven.
    #[inline(always)]
    pub fn new(view: &'info AccountView<'info>) -> Self {
        Self { inner: view }
    }

    /// The underlying account view.
    #[inline(always)]
    pub fn as_account(&self) -> &'info AccountView<'info> {
        self.inner
    }

    /// The account public key.
    #[inline(always)]
    pub fn key(&self) -> &Address {
        self.inner.address()
    }
}

impl<'info> core::ops::Deref for UncheckedAccount<'info> {
    type Target = AccountView<'info>;

    #[inline(always)]
    fn deref(&self) -> &AccountView<'info> {
        self.inner
    }
}

/// Account owned by the System Program.
#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct SystemAccount<'info> {
    inner: &'info AccountView<'info>,
}

impl<'info> SystemAccount<'info> {
    /// Wrap after verifying System Program ownership.
    #[inline]
    pub fn try_new(view: &'info AccountView<'info>) -> Result<Self, crate::error::ProgramError> {
        view.check_owned_by(&SystemId::ID)?;
        Ok(Self { inner: view })
    }

    /// Wrap an already-verified system-owned account.
    #[inline(always)]
    ///
    /// # Safety
    ///
    /// Caller must have verified the account is owned by the System Program.
    pub unsafe fn new_unchecked(view: &'info AccountView<'info>) -> Self {
        Self { inner: view }
    }

    /// The underlying account view.
    #[inline(always)]
    pub fn as_account(&self) -> &'info AccountView<'info> {
        self.inner
    }

    /// The account public key.
    #[inline(always)]
    pub fn key(&self) -> &Address {
        self.inner.address()
    }
}

impl<'info> core::ops::Deref for SystemAccount<'info> {
    type Target = AccountView<'info>;

    #[inline(always)]
    fn deref(&self) -> &AccountView<'info> {
        self.inner
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
    inner: &'info AccountView<'info>,
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
    pub fn try_new(view: &'info AccountView<'info>) -> Result<Self, crate::error::ProgramError> {
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
    pub fn as_account(&self) -> &'info AccountView<'info> {
        self.inner
    }

    /// The program account public key.
    #[inline(always)]
    pub fn key(&self) -> &Address {
        self.inner.address()
    }
}

impl<'info, P: ProgramId> core::ops::Deref for Program<'info, P> {
    type Target = AccountView<'info>;

    #[inline(always)]
    fn deref(&self) -> &AccountView<'info> {
        self.inner
    }
}

/// Compile-time owner/program set for generic interface wrappers.
///
/// Implement this on a zero-sized marker type when a program context accepts
/// one of several compatible programs. `Interface<'info, I>` validates an
/// executable program account by key against this set, while
/// `InterfaceAccount<'info, T>` validates account ownership through
/// `T::Interface`.
pub trait InterfaceSpec: 'static {
    /// Program IDs accepted by this interface.
    const IDS: &'static [Address];

    /// Whether `program_id` belongs to this interface.
    #[inline(always)]
    fn contains(program_id: &Address) -> bool {
        Self::IDS.iter().any(|candidate| candidate == program_id)
    }
}

/// Hopper layout whose owner may be any program in an interface set.
///
/// This is the generic counterpart to token-specific interface helpers.
/// Use it for Hopper-header layouts shared across compatible programs:
///
/// ```ignore
/// pub struct VaultPrograms;
/// impl InterfaceSpec for VaultPrograms {
///     const IDS: &'static [Address] = &[PROGRAM_A, PROGRAM_B];
/// }
///
/// impl InterfaceAccountLayout for SharedVault {
///     type Interface = VaultPrograms;
/// }
/// ```
pub trait InterfaceAccountLayout: crate::layout::LayoutContract {
    /// The owner/program set accepted for this layout.
    type Interface: InterfaceSpec;

    /// Validate this interface account's bytes after owner-set validation.
    ///
    /// Concrete Hopper layouts keep the default: validate and borrow through
    /// layout metadata checks (discriminator/version/layout id + required
    /// length) without requiring the owner to be the executing program. Marker
    /// interface layouts can override this to accept a bounded set of concrete
    /// layout variants while still using the same `InterfaceAccount` wrapper.
    #[inline]
    fn validate_interface_account(view: &AccountView<'_>) -> Result<(), crate::error::ProgramError> {
        let info = view
            .layout_info()
            .ok_or(crate::error::ProgramError::InvalidAccountData)?;
        if !info.matches::<Self>() {
            return Err(crate::error::ProgramError::InvalidAccountData);
        }
        if view.data_len() < Self::required_len() {
            return Err(crate::error::ProgramError::AccountDataTooSmall);
        }
        Ok(())
    }
}

/// Runtime resolver for marker interface account layouts.
///
/// Implement this for an `InterfaceAccountLayout` marker when one account slot
/// may legally hold several concrete Hopper layouts, for example a migration
/// reader that accepts `VaultV1` or `VaultV2`. The marker's
/// `validate_interface_account` should accept exactly the same variants that
/// `resolve` can return.
pub trait InterfaceAccountResolve: InterfaceAccountLayout {
    /// Borrowed resolved view returned by [`InterfaceAccount::resolve`].
    type Resolved<'a>
    where
        Self: 'a;

    /// Resolve the account bytes to a concrete borrowed variant.
    fn resolve<'a>(view: &'a AccountView<'a>)
        -> Result<Self::Resolved<'a>, crate::error::ProgramError>;
}

/// Executable program account whose key is one of an interface's program IDs.
#[repr(transparent)]
pub struct Interface<'info, I: InterfaceSpec> {
    inner: &'info AccountView<'info>,
    _ty: PhantomData<I>,
}

impl<'info, I: InterfaceSpec> Clone for Interface<'info, I> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<'info, I: InterfaceSpec> Copy for Interface<'info, I> {}

impl<'info, I: InterfaceSpec> Interface<'info, I> {
    /// Wrap an already-validated interface program account.
    #[inline(always)]
    ///
    /// # Safety
    ///
    /// Caller must have verified the account address is in `I::IDS` and the
    /// account is executable.
    pub unsafe fn new_unchecked(view: &'info AccountView<'info>) -> Self {
        Self {
            inner: view,
            _ty: PhantomData,
        }
    }

    /// Wrap after verifying address membership and executability.
    #[inline]
    pub fn try_new(view: &'info AccountView<'info>) -> Result<Self, crate::error::ProgramError> {
        if !I::contains(view.address()) {
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

    /// The underlying account view.
    #[inline(always)]
    pub fn as_account(&self) -> &'info AccountView<'info> {
        self.inner
    }

    /// The selected program id.
    #[inline(always)]
    pub fn key(&self) -> &Address {
        self.inner.address()
    }
}

impl<'info, I: InterfaceSpec> core::ops::Deref for Interface<'info, I> {
    type Target = AccountView<'info>;

    #[inline(always)]
    fn deref(&self) -> &AccountView<'info> {
        self.inner
    }
}

/// Hopper-layout account owned by one of a declared interface's programs.
///
/// `InterfaceAccount<'info, T>` validates two things before binding:
/// account owner is in `T::Interface::IDS`, and the account bytes match
/// `T`'s Hopper layout header. Reads use `load_cross_program` so ownership is
/// intentionally decoupled from the executing program.
#[repr(transparent)]
pub struct InterfaceAccount<'info, T: InterfaceAccountLayout> {
    inner: &'info AccountView<'info>,
    _ty: PhantomData<T>,
}

impl<'info, T: InterfaceAccountLayout> Clone for InterfaceAccount<'info, T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<'info, T: InterfaceAccountLayout> Copy for InterfaceAccount<'info, T> {}

impl<'info, T: InterfaceAccountLayout> InterfaceAccount<'info, T> {
    /// Wrap an already-validated interface-owned layout account.
    #[inline(always)]
    ///
    /// # Safety
    ///
    /// Caller must have verified owner membership and layout identity.
    pub unsafe fn new_unchecked(view: &'info AccountView<'info>) -> Self {
        Self {
            inner: view,
            _ty: PhantomData,
        }
    }

    /// Wrap after verifying owner membership and layout identity.
    #[inline]
    pub fn try_new(view: &'info AccountView<'info>) -> Result<Self, crate::error::ProgramError> {
        let owner = view.read_owner();
        if !<T::Interface as InterfaceSpec>::contains(&owner) {
            return Err(crate::error::ProgramError::IncorrectProgramId);
        }
        T::validate_interface_account(view)?;
        Ok(Self {
            inner: view,
            _ty: PhantomData,
        })
    }

    /// The underlying account view.
    #[inline(always)]
    pub fn as_account(&self) -> &'info AccountView<'info> {
        self.inner
    }

    /// The account public key.
    #[inline(always)]
    pub fn key(&self) -> &Address {
        self.inner.address()
    }

    /// The owning program, copied out of the account header.
    #[inline(always)]
    pub fn owner(&self) -> Address {
        self.inner.read_owner()
    }

    /// Borrow the cross-program layout for reading.
    #[inline(always)]
    pub fn load(&self) -> Result<crate::borrow::Ref<'_, T>, crate::error::ProgramError>
    where
        T: crate::Pod,
    {
        self.inner.load_cross_program::<T>()
    }

    /// Friendly alias for [`Self::load`].
    #[inline(always)]
    pub fn get(&self) -> Result<crate::borrow::Ref<'_, T>, crate::error::ProgramError>
    where
        T: crate::Pod,
    {
        self.load()
    }

    /// Borrow the cross-program layout for the duration of a closure.
    #[inline]
    pub fn with<R, F>(&self, f: F) -> Result<R, crate::error::ProgramError>
    where
        T: crate::Pod,
        F: FnOnce(&T) -> Result<R, crate::error::ProgramError>,
    {
        let account = self.load()?;
        f(&*account)
    }

    /// Borrow the account as another concrete layout in the same interface set.
    ///
    /// This is useful for marker interface accounts whose validation accepts a
    /// bounded set of compatible layouts. The associated-type equality keeps a
    /// caller from accidentally loading a layout governed by a different owner
    /// set.
    #[inline(always)]
    pub fn load_as<U>(&self) -> Result<crate::borrow::Ref<'_, U>, crate::error::ProgramError>
    where
        U: InterfaceAccountLayout<Interface = <T as InterfaceAccountLayout>::Interface> + crate::Pod,
    {
        self.inner.load_cross_program::<U>()
    }

    /// Friendly alias for [`Self::load_as`].
    #[inline(always)]
    pub fn get_as<U>(&self) -> Result<crate::borrow::Ref<'_, U>, crate::error::ProgramError>
    where
        U: InterfaceAccountLayout<Interface = <T as InterfaceAccountLayout>::Interface> + crate::Pod,
    {
        self.load_as::<U>()
    }

    /// Borrow another concrete interface layout for the duration of a closure.
    #[inline]
    pub fn with_as<U, R, F>(&self, f: F) -> Result<R, crate::error::ProgramError>
    where
        U: InterfaceAccountLayout<Interface = <T as InterfaceAccountLayout>::Interface> + crate::Pod,
        F: FnOnce(&U) -> Result<R, crate::error::ProgramError>,
    {
        let account = self.load_as::<U>()?;
        f(&*account)
    }

    /// Whether the current bytes match another concrete layout in the same
    /// interface set.
    #[inline(always)]
    pub fn is<U>(&self) -> bool
    where
        U: InterfaceAccountLayout<Interface = <T as InterfaceAccountLayout>::Interface> + crate::Pod,
    {
        self.inner
            .layout_info()
            .is_some_and(|info| info.matches::<U>())
    }

    /// Resolve a marker interface account to one of its concrete variants.
    #[inline(always)]
    pub fn resolve(&self) -> Result<T::Resolved<'_>, crate::error::ProgramError>
    where
        T: InterfaceAccountResolve,
    {
        T::resolve(self.inner)
    }
}

impl<'info, T: InterfaceAccountLayout> core::ops::Deref for InterfaceAccount<'info, T> {
    type Target = AccountView<'info>;

    #[inline(always)]
    fn deref(&self) -> &AccountView<'info> {
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
            core::mem::size_of::<&'static AccountView<'static>>()
        );
    }

    #[test]
    fn system_program_id_is_all_zero() {
        let sys = SystemId::ID;
        assert_eq!(sys.as_array(), &[0u8; 32]);
    }
}

#[cfg(test)]
mod resolver_tests {
    use super::*;
    use crate::layout::{HopperHeader, LayoutContract};

    use hopper_native::{
        AccountView as NativeAccountView, Address as NativeAddress, RuntimeAccount, NOT_BORROWED,
    };

    const PROGRAM_A: Address = Address::new_from_array([0xA1; 32]);
    const PROGRAM_B: Address = Address::new_from_array([0xB2; 32]);
    const OTHER_PROGRAM: Address = Address::new_from_array([0xCC; 32]);

    struct VaultPrograms;
    impl InterfaceSpec for VaultPrograms {
        const IDS: &'static [Address] = &[PROGRAM_A, PROGRAM_B];
    }

    #[repr(C)]
    #[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
    struct VaultV1 {
        balance: [u8; 8],
    }

    unsafe impl crate::Pod for VaultV1 {}

    impl crate::field_map::FieldMap for VaultV1 {
        const FIELDS: &'static [crate::field_map::FieldInfo] = &[crate::field_map::FieldInfo::new(
            "balance",
            HopperHeader::SIZE,
            8,
        )];
    }

    impl LayoutContract for VaultV1 {
        const DISC: u8 = 11;
        const VERSION: u8 = 1;
        const LAYOUT_ID: [u8; 8] = [0x11; 8];
        const SIZE: usize = HopperHeader::SIZE + core::mem::size_of::<Self>();
    }

    impl InterfaceAccountLayout for VaultV1 {
        type Interface = VaultPrograms;
    }

    #[repr(C)]
    #[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
    struct VaultV2 {
        balance: [u8; 8],
        bump: [u8; 8],
    }

    unsafe impl crate::Pod for VaultV2 {}

    impl crate::field_map::FieldMap for VaultV2 {
        const FIELDS: &'static [crate::field_map::FieldInfo] = &[
            crate::field_map::FieldInfo::new("balance", HopperHeader::SIZE, 8),
            crate::field_map::FieldInfo::new("bump", HopperHeader::SIZE + 8, 8),
        ];
    }

    impl LayoutContract for VaultV2 {
        const DISC: u8 = 12;
        const VERSION: u8 = 2;
        const LAYOUT_ID: [u8; 8] = [0x22; 8];
        const SIZE: usize = HopperHeader::SIZE + core::mem::size_of::<Self>();
    }

    impl InterfaceAccountLayout for VaultV2 {
        type Interface = VaultPrograms;
    }

    #[repr(C)]
    #[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
    struct AnyVault {
        _reserved: [u8; 1],
    }

    unsafe impl crate::Pod for AnyVault {}

    impl crate::field_map::FieldMap for AnyVault {
        const FIELDS: &'static [crate::field_map::FieldInfo] = &[];
    }

    impl LayoutContract for AnyVault {
        const DISC: u8 = 0;
        const VERSION: u8 = 0;
        const LAYOUT_ID: [u8; 8] = [0; 8];
        const SIZE: usize = HopperHeader::SIZE;
    }

    impl InterfaceAccountLayout for AnyVault {
        type Interface = VaultPrograms;

        fn validate_interface_account(
            view: &AccountView<'_>,
        ) -> Result<(), crate::error::ProgramError> {
            let data = view.try_borrow()?;
            if VaultV1::validate_header(&data).is_ok() || VaultV2::validate_header(&data).is_ok() {
                Ok(())
            } else {
                Err(crate::error::ProgramError::InvalidAccountData)
            }
        }
    }

    enum ResolvedVault<'a> {
        V1(crate::borrow::Ref<'a, VaultV1>),
        V2(crate::borrow::Ref<'a, VaultV2>),
    }

    impl InterfaceAccountResolve for AnyVault {
        type Resolved<'a> = ResolvedVault<'a>;

        fn resolve<'a>(
            view: &'a AccountView<'a>,
        ) -> Result<Self::Resolved<'a>, crate::error::ProgramError> {
            let info = view
                .layout_info()
                .ok_or(crate::error::ProgramError::AccountDataTooSmall)?;
            if info.matches::<VaultV1>() {
                return Ok(ResolvedVault::V1(view.load_cross_program::<VaultV1>()?));
            }
            if info.matches::<VaultV2>() {
                return Ok(ResolvedVault::V2(view.load_cross_program::<VaultV2>()?));
            }
            Err(crate::error::ProgramError::InvalidAccountData)
        }
    }

    fn make_account(total_data_len: usize, owner: Address) -> (std::vec::Vec<u8>, AccountView<'static>) {
        let mut backing = std::vec![0u8; RuntimeAccount::SIZE + total_data_len];
        let raw = backing.as_mut_ptr() as *mut RuntimeAccount;
        // SAFETY: The test owns `backing`, writes one RuntimeAccount header,
        // and keeps the buffer alive for the returned AccountView.
        unsafe {
            raw.write(RuntimeAccount {
                borrow_state: NOT_BORROWED,
                is_signer: 1,
                is_writable: 1,
                executable: 0,
                resize_delta: 0,
                address: NativeAddress::new_from_array([0x44; 32]),
                owner: NativeAddress::new_from_array(*owner.as_array()),
                lamports: 42,
                data_len: total_data_len as u64,
            });
        }
        // SAFETY: `raw` points to the RuntimeAccount header initialized above.
        let backend = unsafe { NativeAccountView::new_unchecked(raw) };
        (backing, AccountView::from_backend(backend))
    }

    #[test]
    fn interface_account_resolves_bounded_layout_variants() {
        let (_v1_backing, v1_account) = make_account(VaultV1::SIZE, PROGRAM_B);
        {
            let mut data = v1_account.try_borrow_mut().unwrap();
            crate::layout::init_header::<VaultV1>(&mut data).unwrap();
            data[HopperHeader::SIZE..HopperHeader::SIZE + 8].copy_from_slice(&300u64.to_le_bytes());
        }

        let v1_vault = InterfaceAccount::<AnyVault>::try_new(&v1_account).unwrap();
        match v1_vault.resolve().unwrap() {
            ResolvedVault::V1(v1) => {
                assert_eq!(u64::from_le_bytes(v1.balance), 300);
            }
            ResolvedVault::V2(_) => panic!("expected v1"),
        }

        let (_backing, account) = make_account(VaultV2::SIZE, PROGRAM_A);
        {
            let mut data = account.try_borrow_mut().unwrap();
            crate::layout::init_header::<VaultV2>(&mut data).unwrap();
            data[HopperHeader::SIZE..HopperHeader::SIZE + 8].copy_from_slice(&700u64.to_le_bytes());
            data[HopperHeader::SIZE + 8..HopperHeader::SIZE + 16]
                .copy_from_slice(&9u64.to_le_bytes());
        }

        let vault = InterfaceAccount::<AnyVault>::try_new(&account).unwrap();
        assert!(vault.is::<VaultV2>());
        assert!(!vault.is::<VaultV1>());

        match vault.resolve().unwrap() {
            ResolvedVault::V2(v2) => {
                assert_eq!(u64::from_le_bytes(v2.balance), 700);
                assert_eq!(u64::from_le_bytes(v2.bump), 9);
            }
            ResolvedVault::V1(_) => panic!("expected v2"),
        }

        let v2 = vault.load_as::<VaultV2>().unwrap();
        assert_eq!(u64::from_le_bytes(v2.balance), 700);
        assert!(vault.get_as::<VaultV1>().is_err());
    }

    #[test]
    fn interface_account_resolver_keeps_owner_and_layout_checks() {
        let (_wrong_owner_backing, wrong_owner) = make_account(VaultV1::SIZE, OTHER_PROGRAM);
        {
            let mut data = wrong_owner.try_borrow_mut().unwrap();
            crate::layout::init_header::<VaultV1>(&mut data).unwrap();
        }
        let wrong_owner_result = InterfaceAccount::<AnyVault>::try_new(&wrong_owner);
        assert!(matches!(
            wrong_owner_result,
            Err(crate::error::ProgramError::IncorrectProgramId)
        ));

        let (_bad_layout_backing, bad_layout) = make_account(VaultV1::SIZE, PROGRAM_B);
        {
            let mut data = bad_layout.try_borrow_mut().unwrap();
            crate::layout::write_header(&mut data, 99, 1, &[0x99; 8]).unwrap();
        }
        let bad_layout_result = InterfaceAccount::<AnyVault>::try_new(&bad_layout);
        assert!(matches!(
            bad_layout_result,
            Err(crate::error::ProgramError::InvalidAccountData)
        ));
    }
}
