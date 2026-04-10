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

use hopper_runtime::{error::ProgramError, AccountView, Address, ProgramResult};

// -- Phase Marker Types (zero-sized, compile-time only) --------------

/// Phase: accounts not yet resolved.
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
    accounts: &'a [AccountView],
    ix_data: &'a [u8],
    mutable_borrows: u64,
    _phase: core::marker::PhantomData<P>,
}

impl<'a> PhasedFrame<'a, Unresolved> {
    /// Create a new phased frame in the `Unresolved` state.
    #[inline(always)]
    pub fn new(
        program_id: &'a Address,
        accounts: &'a [AccountView],
        ix_data: &'a [u8],
    ) -> Result<Self, ProgramError> {
        if accounts.len() > crate::frame::MAX_FRAME_ACCOUNTS {
            return Err(ProgramError::InvalidArgument);
        }
        Ok(Self {
            program_id,
            accounts,
            ix_data,
            mutable_borrows: 0,
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
    pub fn resolve<T, F>(self, min_accounts: usize, f: F) -> Result<ResolvedFrame<'a, T>, ProgramError>
    where
        F: FnOnce(&'a [AccountView], &'a Address) -> Result<T, ProgramError>,
    {
        if self.accounts.len() < min_accounts {
            return Err(ProgramError::NotEnoughAccountKeys);
        }
        let resolved = f(self.accounts, self.program_id)?;
        Ok(ResolvedFrame {
            program_id: self.program_id,
            accounts: self.accounts,
            ix_data: self.ix_data,
            mutable_borrows: self.mutable_borrows,
            resolved,
        })
    }
}

/// A frame that has been resolved with typed account references.
///
/// `T` is the user's account struct (e.g., `SwapAccounts<'a>`).
pub struct ResolvedFrame<'a, T> {
    pub(crate) program_id: &'a Address,
    pub(crate) accounts: &'a [AccountView],
    pub(crate) ix_data: &'a [u8],
    pub(crate) mutable_borrows: u64,
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
            mutable_borrows: self.mutable_borrows,
            resolved: self.resolved,
        })
    }
}

/// A frame whose accounts have been validated.
pub struct ValidatedFrame<'a, T> {
    pub(crate) program_id: &'a Address,
    pub(crate) accounts: &'a [AccountView],
    pub(crate) ix_data: &'a [u8],
    pub(crate) mutable_borrows: u64,
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
    pub fn execute<R, F>(mut self, f: F) -> Result<R, ProgramError>
    where
        F: FnOnce(&mut ExecutionContext<'a, '_, T>) -> Result<R, ProgramError>,
    {
        let mut ctx = ExecutionContext {
            program_id: self.program_id,
            accounts: self.accounts,
            ix_data: self.ix_data,
            mutable_borrows: &mut self.mutable_borrows,
            resolved: &self.resolved,
        };
        f(&mut ctx)
    }
}

/// Mutable execution context available during the Execute phase.
pub struct ExecutionContext<'a, 'f, T> {
    pub(crate) program_id: &'a Address,
    pub(crate) accounts: &'a [AccountView],
    pub(crate) ix_data: &'a [u8],
    pub(crate) mutable_borrows: &'f mut u64,
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
    #[inline]
    pub fn borrow_mut(&mut self, index: usize) -> Result<&'a mut [u8], ProgramError> {
        if index >= self.accounts.len() {
            return Err(ProgramError::NotEnoughAccountKeys);
        }
        let bit = 1u64 << (index as u32);
        if *self.mutable_borrows & bit != 0 {
            return Err(ProgramError::AccountBorrowFailed);
        }
        *self.mutable_borrows |= bit;
        // SAFETY: Borrow tracking prevents aliasing. Caller proved validation.
        Ok(unsafe { self.accounts[index].borrow_unchecked_mut() })
    }

    /// Borrow account data immutably.
    #[inline(always)]
    pub fn borrow(&self, index: usize) -> Result<&'a [u8], ProgramError> {
        if index >= self.accounts.len() {
            return Err(ProgramError::NotEnoughAccountKeys);
        }
        // SAFETY: Immutable borrow does not conflict.
        Ok(unsafe { self.accounts[index].borrow_unchecked() })
    }

    /// Get raw AccountView by index.
    #[inline(always)]
    pub fn account(&self, index: usize) -> Result<&'a AccountView, ProgramError> {
        self.accounts.get(index).ok_or(ProgramError::NotEnoughAccountKeys)
    }
}
