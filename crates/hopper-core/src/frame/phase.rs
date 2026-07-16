//! Phased execution builder for the Frame.
//!
//! Hopper's signature feature: typestate-driven phased execution that
//! enforces correct ordering at compile time.
//!
//! ```text
//! frame.resolve(accounts)?
//!      .validate(|ctx| { ... })?
//!      .execute(|ctx| { ... })?
//! ```
//!
//! The typestate pattern means:
//! - You cannot call `.execute()` before `.validate()`
//! - You cannot call `.validate()` before `.resolve()`
//! - Each transition is a zero-cost abstraction at runtime
//!
//! ## Phase Model
//!
//! ```text
//! Unresolved -> Resolved -> Validated -> Executed
//! ```

use hopper_runtime::{error::ProgramError, AccountView, Address, ProgramResult, Ref, RefMut};

// -- Phase Marker Types (zero-sized, compile-time only) --------------

/// Phase: accounts pending resolution.
pub struct Unresolved;
/// Phase: accounts resolved and typed.
pub struct Resolved;
/// Phase: validation passed.
pub struct Validated;
/// Phase: execution complete.
pub struct Executed;

// -- Phased Frame ----------------------------------------------------

/// A phased execution context that enforces ordering via type state.
///
/// `P` is the current phase -- a zero-sized marker type.
/// The frame itself carries no per-phase overhead at runtime;
/// phase transitions are compile-time checked.
pub struct PhasedFrame<'a, P> {
    program_id: &'a Address,
    accounts: &'a [AccountView<'a>],
    ix_data: &'a [u8],
    _phase: core::marker::PhantomData<P>,
}

impl<'a> PhasedFrame<'a, Unresolved> {
    /// Create a new phased frame in the `Unresolved` state.
    #[inline(always)]
    pub fn new(
        program_id: &'a Address,
        accounts: &'a [AccountView<'a>],
        ix_data: &'a [u8],
    ) -> Result<Self, ProgramError> {
        if accounts.len() > crate::frame::MAX_FRAME_ACCOUNTS {
            return Err(ProgramError::InvalidArgument);
        }
        Ok(Self {
            program_id,
            accounts,
            ix_data,
            _phase: core::marker::PhantomData,
        })
    }

    /// Resolve accounts -- validate account count and transition to `Resolved`.
    ///
    /// The closure receives the accounts slice and program_id, allowing
    /// the caller to parse/index accounts into a typed struct.
    ///
    /// ```ignore
    /// let resolved = frame.resolve(|accounts, program_id| {
    ///     Ok(MyAccounts {
    ///         payer: &accounts[0],
    ///         vault: &accounts[1],
    ///     })
    /// })?;
    /// ```
    #[inline]
    pub fn resolve<T, F>(
        self,
        min_accounts: usize,
        f: F,
    ) -> Result<ResolvedFrame<'a, T>, ProgramError>
    where
        F: FnOnce(&'a [AccountView<'a>], &'a Address) -> Result<T, ProgramError>,
    {
        if self.accounts.len() < min_accounts {
            return Err(ProgramError::NotEnoughAccountKeys);
        }
        let resolved = f(self.accounts, self.program_id)?;
        Ok(ResolvedFrame {
            program_id: self.program_id,
            accounts: self.accounts,
            ix_data: self.ix_data,
            resolved,
        })
    }
}

/// A frame that has been resolved with typed account references.
///
/// `T` is the user's account struct (e.g., `SwapAccounts<'a>`).
pub struct ResolvedFrame<'a, T> {
    pub(crate) program_id: &'a Address,
    pub(crate) accounts: &'a [AccountView<'a>],
    pub(crate) ix_data: &'a [u8],
    pub(crate) resolved: T,
}

impl<'a, T> ResolvedFrame<'a, T> {
    /// Program ID.
    #[inline(always)]
    pub fn program_id(&self) -> &Address {
        self.program_id
    }

    /// Instruction data.
    #[inline(always)]
    pub fn ix_data(&self) -> &[u8] {
        self.ix_data
    }

    /// Access the resolved accounts.
    #[inline(always)]
    pub fn accounts(&self) -> &T {
        &self.resolved
    }

    /// Validate constraints and transition to `ValidatedFrame`.
    ///
    /// The closure receives the resolved accounts for validation. It should
    /// call `check_*` functions and return `Ok(())` on success.
    ///
    /// ```ignore
    /// let validated = resolved.validate(|ctx| {
    ///     check_signer(ctx.payer)?;
    ///     check_owner(ctx.vault, program_id)?;
    ///     Ok(())
    /// })?;
    /// ```
    #[inline]
    pub fn validate<F>(self, f: F) -> Result<ValidatedFrame<'a, T>, ProgramError>
    where
        F: FnOnce(&T, &Address) -> ProgramResult,
    {
        f(&self.resolved, self.program_id)?;
        Ok(ValidatedFrame {
            program_id: self.program_id,
            accounts: self.accounts,
            ix_data: self.ix_data,
            resolved: self.resolved,
        })
    }
}

/// A frame whose accounts have been validated.
pub struct ValidatedFrame<'a, T> {
    pub(crate) program_id: &'a Address,
    pub(crate) accounts: &'a [AccountView<'a>],
    pub(crate) ix_data: &'a [u8],
    pub(crate) resolved: T,
}

impl<'a, T> ValidatedFrame<'a, T> {
    /// Program ID.
    #[inline(always)]
    pub fn program_id(&self) -> &Address {
        self.program_id
    }

    /// Instruction data.
    #[inline(always)]
    pub fn ix_data(&self) -> &[u8] {
        self.ix_data
    }

    /// Access the resolved and validated accounts.
    #[inline(always)]
    pub fn accounts(&self) -> &T {
        &self.resolved
    }

    /// Execute the instruction logic.
    ///
    /// The closure receives an `ExecutionContext` with mutable access to
    /// the validated accounts and mutable borrow tracking.
    ///
    /// ```ignore
    /// validated.execute(|ctx| {
    ///     let vault_data = ctx.borrow_mut(1)?;
    ///     // ... mutate state ...
    ///     Ok(())
    /// })?;
    /// ```
    #[inline]
    pub fn execute<R, F>(self, f: F) -> Result<R, ProgramError>
    where
        F: FnOnce(&mut ExecutionContext<'a, '_, T>) -> Result<R, ProgramError>,
    {
        let mut ctx = ExecutionContext {
            program_id: self.program_id,
            accounts: self.accounts,
            ix_data: self.ix_data,
            resolved: &self.resolved,
        };
        f(&mut ctx)
    }
}

/// Mutable execution context available during the Execute phase.
pub struct ExecutionContext<'a, 'f, T> {
    pub(crate) program_id: &'a Address,
    pub(crate) accounts: &'a [AccountView<'a>],
    pub(crate) ix_data: &'a [u8],
    pub(crate) resolved: &'f T,
}

impl<'a, 'f, T> ExecutionContext<'a, 'f, T> {
    /// Program ID.
    #[inline(always)]
    pub fn program_id(&self) -> &'a Address {
        self.program_id
    }

    /// Instruction data.
    #[inline(always)]
    pub fn ix_data(&self) -> &'a [u8] {
        self.ix_data
    }

    /// Resolved accounts.
    #[inline(always)]
    pub fn resolved(&self) -> &T {
        self.resolved
    }

    /// Borrow account data mutably with runtime aliasing protection.
    ///
    /// Aliasing is enforced by the account-level borrow byte, which is
    /// exact and RAII-released: a second mutable borrow fails while the
    /// returned guard lives and succeeds after it drops, and duplicate
    /// account metas (two indices, one account) are correctly rejected
    /// because the byte is per-account, not per-index. (An earlier
    /// index-bitmask layer here was strictly weaker on duplicates and
    /// never cleared its bit on guard drop, so sequential legal
    /// re-borrows failed for the rest of the phase.)
    #[inline]
    pub fn borrow_mut(&mut self, index: usize) -> Result<RefMut<'a, [u8]>, ProgramError> {
        if index >= self.accounts.len() {
            return Err(ProgramError::NotEnoughAccountKeys);
        }
        self.accounts[index].try_borrow_mut()
    }

    /// Borrow account data immutably.
    #[inline(always)]
    pub fn borrow(&self, index: usize) -> Result<Ref<'a, [u8]>, ProgramError> {
        if index >= self.accounts.len() {
            return Err(ProgramError::NotEnoughAccountKeys);
        }
        // SAFETY: Immutable borrow does not conflict.
        self.accounts[index].try_borrow()
    }

    /// Get raw AccountView by index.
    #[inline(always)]
    pub fn account(&self, index: usize) -> Result<&'a AccountView<'a>, ProgramError> {
        self.accounts
            .get(index)
            .ok_or(ProgramError::NotEnoughAccountKeys)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hopper_native::{
        AccountView as NativeAccountView, Address as NativeAddress, RuntimeAccount, NOT_BORROWED,
    };

    fn make_account(seed: u8) -> (std::vec::Vec<u64>, AccountView<'static>) {
        let mut backing = std::vec![0u64; (RuntimeAccount::SIZE + 16).div_ceil(8)];
        let raw = backing.as_mut_ptr() as *mut RuntimeAccount;
        // SAFETY: backing is sized for the header plus data and outlives
        // the returned view.
        unsafe {
            raw.write(RuntimeAccount {
                borrow_state: NOT_BORROWED,
                is_signer: 1,
                is_writable: 1,
                executable: 0,
                resize_delta: 0,
                address: NativeAddress::new_from_array([seed; 32]),
                owner: NativeAddress::new_from_array([2; 32]),
                lamports: 1,
                data_len: 16,
            });
        }
        // SAFETY: raw points at a fully initialized RuntimeAccount.
        let backend = unsafe { NativeAccountView::new_unchecked(raw) };
        let view =
            // SAFETY: AccountView is repr(transparent) over the native view.
            unsafe { core::mem::transmute::<NativeAccountView, AccountView>(backend) };
        (backing, view)
    }

    #[test]
    fn borrow_mut_is_raii_not_sticky() {
        let (_b, account) = make_account(1);
        let pid = Address::new([9u8; 32]);
        let accounts = [account];

        PhasedFrame::new(&pid, &accounts, &[])
            .unwrap()
            .resolve(1, |_, _| Ok(()))
            .unwrap()
            .validate(|_, _| Ok(()))
            .unwrap()
            .execute(|ctx| {
                // First mutable borrow works; a second while the guard
                // lives fails at the account borrow byte.
                let first = ctx.borrow_mut(0)?;
                assert!(ctx.borrow_mut(0).is_err());
                drop(first);
                // Pre-fix, the index bit stayed set here and this legal
                // sequential re-borrow failed for the rest of the phase.
                let again = ctx.borrow_mut(0)?;
                drop(again);
                // Reads never conflict with each other.
                let r1 = ctx.borrow(0)?;
                let r2 = ctx.borrow(0)?;
                drop((r1, r2));
                Ok(())
            })
            .unwrap();
    }

    #[test]
    fn duplicate_account_metas_share_one_borrow_byte() {
        // Two frame slots aliasing the SAME underlying account: the old
        // per-index bitmask would have allowed both mutable borrows; the
        // per-account borrow byte must reject the second.
        let mut backing = std::vec![0u64; (RuntimeAccount::SIZE + 16).div_ceil(8)];
        let raw = backing.as_mut_ptr() as *mut RuntimeAccount;
        // SAFETY: as in make_account.
        unsafe {
            raw.write(RuntimeAccount {
                borrow_state: NOT_BORROWED,
                is_signer: 0,
                is_writable: 1,
                executable: 0,
                resize_delta: 0,
                address: NativeAddress::new_from_array([7; 32]),
                owner: NativeAddress::new_from_array([2; 32]),
                lamports: 1,
                data_len: 16,
            });
        }
        // SAFETY: same initialized header viewed twice — modelling the
        // loader's duplicate-account-meta case where two instruction
        // slots point at one account buffer.
        let v1 = unsafe {
            core::mem::transmute::<NativeAccountView, AccountView>(
                NativeAccountView::new_unchecked(raw),
            )
        };
        // SAFETY: as above.
        let v2 = unsafe {
            core::mem::transmute::<NativeAccountView, AccountView>(
                NativeAccountView::new_unchecked(raw),
            )
        };
        let pid = Address::new([9u8; 32]);
        let accounts = [v1, v2];

        PhasedFrame::new(&pid, &accounts, &[])
            .unwrap()
            .resolve(2, |_, _| Ok(()))
            .unwrap()
            .validate(|_, _| Ok(()))
            .unwrap()
            .execute(|ctx| {
                let first = ctx.borrow_mut(0)?;
                assert!(ctx.borrow_mut(1).is_err());
                drop(first);
                let second = ctx.borrow_mut(1)?;
                drop(second);
                Ok(())
            })
            .unwrap();
    }
}
