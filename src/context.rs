//! Execution context for Hopper programs.
//!
//! `Context` is the canonical execution object that Hopper handlers receive.
//! It provides structured access to the program_id, accounts, and instruction
//! data, with indexed access and validation helpers.
//!
//! This is where Hopper programs start feeling like a coherent language
//! instead of raw pointer manipulation.

use crate::account::AccountView;
use crate::address::Address;
use crate::error::ProgramError;
use crate::ProgramResult;

/// Execution context for a Hopper instruction handler.
///
/// Wraps the program_id, account slice, and instruction data into a single
/// object with structured access patterns.
///
/// # Authored flow
///
/// ```ignore
/// pub fn deposit(ctx: &Context, amount: u64) -> ProgramResult {
///     let authority = ctx.account(0)?;
///     let vault = ctx.account(1)?;
///
///     authority.require_signer()?;
///     vault.require_writable()?;
///     vault.check_disc(1)?;
///
///     let state = vault.overlay_mut::<VaultState>()?;
///     state.balance = state.balance.checked_add(amount).ok_or(ProgramError::ArithmeticOverflow)?;
///     Ok(())
/// }
/// ```
pub struct Context<'a> {
    /// The program's own address.
    pub program_id: &'a Address,
    /// All accounts passed to this instruction.
    accounts: &'a [AccountView],
    /// Raw instruction data (past the discriminator byte, if applicable).
    pub instruction_data: &'a [u8],
}

impl<'a> Context<'a> {
    /// Create a new context from the entrypoint parameters.
    #[inline(always)]
    pub fn new(
        program_id: &'a Address,
        accounts: &'a [AccountView],
        instruction_data: &'a [u8],
    ) -> Self {
        Self {
            program_id,
            accounts,
            instruction_data,
        }
    }

    /// Get an account by index.
    #[inline(always)]
    pub fn account(&self, index: usize) -> Result<&AccountView, ProgramError> {
        self.accounts.get(index).ok_or(ProgramError::NotEnoughAccountKeys)
    }

    /// Get an account by index (mutation-intent variant).
    ///
    /// Functionally identical to `account()` since `AccountView` uses
    /// interior mutability for data access (`overlay_mut`, `load_mut`,
    /// `try_borrow_mut`). The distinct name signals that the caller
    /// intends to write through the returned reference.
    #[inline(always)]
    pub fn account_mut(&self, index: usize) -> Result<&AccountView, ProgramError> {
        self.accounts.get(index).ok_or(ProgramError::NotEnoughAccountKeys)
    }

    /// Get the total number of accounts.
    #[inline(always)]
    pub fn num_accounts(&self) -> usize {
        self.accounts.len()
    }

    /// Get all accounts as a slice.
    #[inline(always)]
    pub fn accounts(&self) -> &[AccountView] {
        self.accounts
    }

    /// Get the remaining accounts starting at `from`.
    #[inline(always)]
    pub fn remaining_accounts(&self, from: usize) -> &[AccountView] {
        if from >= self.accounts.len() {
            &[]
        } else {
            &self.accounts[from..]
        }
    }

    /// Require at least `n` accounts are present.
    #[inline(always)]
    pub fn require_accounts(&self, n: usize) -> ProgramResult {
        if self.accounts.len() >= n {
            Ok(())
        } else {
            Err(ProgramError::NotEnoughAccountKeys)
        }
    }

    /// Require at least `n` bytes of instruction data.
    #[inline(always)]
    pub fn require_data_len(&self, n: usize) -> ProgramResult {
        if self.instruction_data.len() >= n {
            Ok(())
        } else {
            Err(ProgramError::InvalidInstructionData)
        }
    }

    /// Read instruction data as a typed value (unaligned, little-endian safe).
    ///
    /// Reads `size_of::<T>()` bytes starting at `offset` via `read_unaligned`.
    /// Caller must ensure `T` is a plain-old-data type where all bit patterns
    /// are valid.
    #[inline(always)]
    pub fn read_data<T: Copy>(&self, offset: usize) -> Result<T, ProgramError> {
        let end = offset.checked_add(core::mem::size_of::<T>())
            .ok_or(ProgramError::ArithmeticOverflow)?;
        if self.instruction_data.len() < end {
            return Err(ProgramError::InvalidInstructionData);
        }
        // SAFETY: bounds checked, T: Copy (no drop glue), read_unaligned handles alignment.
        Ok(unsafe {
            core::ptr::read_unaligned(self.instruction_data.as_ptr().add(offset) as *const T)
        })
    }

    /// Get a byte slice from instruction data.
    #[inline(always)]
    pub fn data_slice(&self, offset: usize, len: usize) -> Result<&[u8], ProgramError> {
        let end = offset.checked_add(len).ok_or(ProgramError::ArithmeticOverflow)?;
        if self.instruction_data.len() < end {
            return Err(ProgramError::InvalidInstructionData);
        }
        Ok(&self.instruction_data[offset..end])
    }

    /// Read the first byte of instruction data as an instruction tag.
    ///
    /// Common pattern for byte-tag dispatch.
    #[inline(always)]
    pub fn instruction_tag(&self) -> Result<u8, ProgramError> {
        self.instruction_data.first().copied().ok_or(ProgramError::InvalidInstructionData)
    }
}
