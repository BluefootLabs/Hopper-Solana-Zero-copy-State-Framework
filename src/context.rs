//! Execution context for Hopper programs.
//!
//! `Context` is the canonical execution object that Hopper handlers receive.
//! It provides structured access to the program_id, accounts, and instruction
//! data, with indexed access and validation helpers.
//!
//! Keep it boring: `Context` is the container for accounts, instruction data,
//! and the instruction-scoped segment borrow registry. `AccountView` owns the
//! actual access operations.

use crate::account::AccountView;
use crate::audit::AccountAudit;
use crate::address::Address;
use crate::error::ProgramError;
use crate::segment_borrow::SegmentBorrowRegistry;
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
///     let mut state = vault.load_mut::<VaultState>()?;
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
    /// Segment-level borrow tracking for fine-grained access control.
    ///
    /// Enables safe concurrent mutable access to non-overlapping regions
    /// of the same account. This is what makes Hopper strictly safer than
    /// raw Pinocchio without adding meaningful CU overhead.
    /// Prefer the `borrows()` / `borrows_mut()` accessors in new code.
    pub(crate) segment_borrows: SegmentBorrowRegistry,
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
            segment_borrows: SegmentBorrowRegistry::new(),
        }
    }

    /// Program ID.
    #[inline(always)]
    pub fn program_id(&self) -> &Address {
        self.program_id
    }

    /// Raw instruction data.
    #[inline(always)]
    pub fn instruction_data(&self) -> &'a [u8] {
        self.instruction_data
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

    /// Access the instruction-scoped segment borrow registry.
    #[inline(always)]
    pub fn borrows(&self) -> &SegmentBorrowRegistry {
        &self.segment_borrows
    }

    /// Mutably access the instruction-scoped segment borrow registry.
    #[inline(always)]
    pub fn borrows_mut(&mut self) -> &mut SegmentBorrowRegistry {
        &mut self.segment_borrows
    }

    /// Inspect the instruction account slice for duplicate aliases.
    #[inline(always)]
    pub fn audit_accounts(&self) -> AccountAudit<'a> {
        AccountAudit::new(self.accounts)
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

    /// Require all account addresses to be unique.
    #[inline(always)]
    pub fn require_unique_accounts(&self) -> ProgramResult {
        self.audit_accounts().require_all_unique()
    }

    /// Require that no duplicated account is writable in this instruction.
    #[inline(always)]
    pub fn require_unique_writable_accounts(&self) -> ProgramResult {
        self.audit_accounts().require_unique_writable()
    }

    /// Require that no duplicated account is used as a signer role.
    #[inline(always)]
    pub fn require_unique_signer_accounts(&self) -> ProgramResult {
        self.audit_accounts().require_unique_signers()
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

    // --- Segment-Level Access (fine-grained borrow tracking) --------

    /// Register a read borrow for a segment of an account.
    ///
    /// Validates bounds and registers the borrow in the segment registry,
    /// then returns a shared `Ref<T>` that keeps the borrow guard alive.
    ///
    /// `index` is the account index. `abs_offset` is the absolute byte
    /// offset within the account data (including header bytes).
    ///
    /// # Type Safety
    ///
    /// T must be `Copy`. The returned reference is valid for the lifetime
    /// of the borrow guard. Segment borrow tracking prevents conflicting
    /// write access to the same byte range.
    #[inline(always)]
    pub fn segment_ref<T: Copy>(
        &mut self,
        index: usize,
        abs_offset: u32,
    ) -> Result<crate::Ref<'_, T>, ProgramError> {
        let view = self.accounts.get(index)
            .ok_or(ProgramError::NotEnoughAccountKeys)?;
        view.segment_ref::<T>(&mut self.segment_borrows, abs_offset, core::mem::size_of::<T>() as u32)
    }

    /// Register a write borrow for a segment of an account.
    ///
    /// Validates bounds, checks writable, and registers an exclusive
    /// borrow, then returns a mutable `RefMut<T>` that keeps the guard alive.
    ///
    /// This is the primitive that enables safe concurrent mutation of
    /// non-overlapping account regions — the core Hopper innovation.
    #[inline(always)]
    pub fn segment_mut<T: Copy>(
        &mut self,
        index: usize,
        abs_offset: u32,
    ) -> Result<crate::RefMut<'_, T>, ProgramError> {
        let view = self.accounts.get(index)
            .ok_or(ProgramError::NotEnoughAccountKeys)?;
        view.segment_mut::<T>(&mut self.segment_borrows, abs_offset, core::mem::size_of::<T>() as u32)
    }

    /// Explicit unsafe whole-account typed read.
    #[inline(always)]
    pub unsafe fn raw_ref<T: Copy>(
        &self,
        index: usize,
    ) -> Result<crate::Ref<'_, T>, ProgramError> {
        let view = self.accounts.get(index)
            .ok_or(ProgramError::NotEnoughAccountKeys)?;
        unsafe { view.raw_ref::<T>() }
    }

    /// Explicit unsafe whole-account typed write.
    #[inline(always)]
    pub unsafe fn raw_mut<T: Copy>(
        &self,
        index: usize,
    ) -> Result<crate::RefMut<'_, T>, ProgramError> {
        let view = self.accounts.get(index)
            .ok_or(ProgramError::NotEnoughAccountKeys)?;
        unsafe { view.raw_mut::<T>() }
    }

    /// Explicit unsafe escape hatch for whole-account typed projection.
    ///
    /// This bypasses segment borrow tracking. The caller is responsible for
    /// alias safety and for using a type that matches the account bytes.
    #[inline(always)]
    pub unsafe fn raw_unchecked<T: Copy>(
        &self,
        index: usize,
    ) -> Result<crate::RefMut<'_, T>, ProgramError> {
        unsafe { self.raw_mut::<T>(index) }
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
