use {crate::prelude::*, core::marker::PhantomData};

/// Shared owner check -- called by both `from_account_view` and
/// `from_account_view_mut`.
#[inline(always)]
fn check_owners(view: &AccountView, owners: &[Address]) -> Result<(), ProgramError> {
    let owner = view.owner();
    let mut i = 0;
    while i < owners.len() {
        if crate::keys_eq(owner, &owners[i]) {
            return Ok(());
        }
        i += 1;
    }
    Err(ProgramError::IllegalOwner)
}

/// Generic interface account wrapper -- accepts accounts owned by any of the
/// programs listed in `T::owners()`.
///
/// `InterfaceAccount<T>` is a peer to `Account<T>`. Where `Account<Token>`
/// only accepts SPL Token-owned accounts, `InterfaceAccount<Token>` accepts
/// both SPL Token and Token-2022. The inner marker `T` provides the data
/// layout check and zero-copy deref target.
///
/// ```ignore
/// pub vault: &'info InterfaceAccount<Token>,
/// pub mint: &'info InterfaceAccount<Mint>,
/// ```
#[repr(transparent)]
pub struct InterfaceAccount<T> {
    view: AccountView,
    _marker: PhantomData<T>,
}

impl<T> AsAccountView for InterfaceAccount<T> {
    #[inline(always)]
    fn to_account_view(&self) -> &AccountView {
        &self.view
    }
}

impl<T: Owners + AccountCheck> InterfaceAccount<T> {
    /// Construct an interface account reference from an `AccountView`,
    /// validating that the owner matches one of `T::owners()`.
    ///
    /// # Errors
    ///
    /// Returns `IllegalOwner` if the owner does not match any entry in
    /// `T::owners()`, or any error from `T::check`.
    #[inline(always)]
    pub fn from_account_view(view: &AccountView) -> Result<&Self, ProgramError> {
        check_owners(view, T::owners())?;
        T::check(view)?;
        // SAFETY: `InterfaceAccount<T>` is `#[repr(transparent)]` over
        // `AccountView` -- the pointer cast is layout-compatible. Owner
        // and data-length checks ran above.
        Ok(unsafe { &*(view as *const AccountView as *const Self) })
    }

    /// Construct a mutable interface account reference from an
    /// `AccountView`, validating owner and writability.
    ///
    /// # Errors
    ///
    /// Returns `Immutable` if the account is not writable, `IllegalOwner`
    /// if the owner does not match any entry in `T::owners()`, or any
    /// error from `T::check`.
    #[inline(always)]
    pub fn from_account_view_mut(view: &mut AccountView) -> Result<&mut Self, ProgramError> {
        if crate::utils::hint::unlikely(!view.is_writable()) {
            return Err(ProgramError::Immutable);
        }
        check_owners(view, T::owners())?;
        T::check(view)?;
        // SAFETY: Same as `from_account_view` -- `#[repr(transparent)]`
        // guarantees layout compatibility. Writability checked above.
        Ok(unsafe { &mut *(view as *mut AccountView as *mut Self) })
    }

    /// Construct an interface account reference without validation.
    ///
    /// # Safety
    ///
    /// The caller must ensure:
    /// - `view.owner()` matches one of the addresses in `T::owners()`
    /// - `view.data_len()` is sufficient for the zero-copy layout
    #[inline(always)]
    pub unsafe fn from_account_view_unchecked(view: &AccountView) -> &Self {
        &*(view as *const AccountView as *const Self)
    }

    /// Construct a mutable interface account reference without validation.
    ///
    /// # Safety
    ///
    /// The caller must ensure:
    /// - `view.owner()` matches one of the addresses in `T::owners()`
    /// - `view.data_len()` is sufficient for the zero-copy layout
    /// - `view.is_writable()` is true
    #[inline(always)]
    pub unsafe fn from_account_view_unchecked_mut(view: &mut AccountView) -> &mut Self {
        &mut *(view as *mut AccountView as *mut Self)
    }
}

impl<T: ZeroCopyDeref> core::ops::Deref for InterfaceAccount<T> {
    type Target = T::Target;

    #[inline(always)]
    fn deref(&self) -> &Self::Target {
        // SAFETY: Owner and data-length checks ran during construction.
        // `T::deref_from` performs the zero-copy cast.
        unsafe { T::deref_from(&self.view) }
    }
}

impl<T: ZeroCopyDeref> core::ops::DerefMut for InterfaceAccount<T> {
    #[inline(always)]
    fn deref_mut(&mut self) -> &mut Self::Target {
        // SAFETY: Same as Deref -- length validated, writability checked.
        unsafe { T::deref_from_mut(&mut self.view) }
    }
}
