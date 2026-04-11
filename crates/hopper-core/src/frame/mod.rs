//! Borrowed-state execution context.
//!
//! The `Frame` is Hopper's execution model. It wraps the instruction's accounts
//! and data, enforcing single-mutable-borrow discipline and phased execution.
//!
//! ## Execution Phases
//!
//! 1. **Resolve** -- Parse accounts from the input slice into named typed slots
//! 2. **Validate** -- Run the validation graph (account-local, cross-account, state-transition)
//! 3. **Borrow** -- Obtain zero-copy overlays with borrow discipline
//! 4. **Mutate** -- Execute state changes through verified mutable references
//! 5. **Emit** -- Fire events
//! 6. **Commit** -- (implicit: Solana runtime commits on success)
//!
//! The `Frame` ensures that:
//! - Each account is borrowed at most once mutably
//! - Immutable borrows can coexist
//! - Validation runs before mutation
//! - Events are emitted after state changes

pub mod phase;
pub mod args;

use hopper_runtime::{error::ProgramError, AccountView, Address, ProgramResult, Ref, RefMut};
use crate::account::SliceCursor;

/// Maximum accounts in a single frame. Matches Solana's transaction limit.
pub const MAX_FRAME_ACCOUNTS: usize = 64;

/// Execution frame holding the instruction's accounts and data.
///
/// `Frame` is the entry point for Hopper's phased execution model.
/// It tracks which accounts have been borrowed (mutably or immutably)
/// to prevent aliasing violations at runtime.
pub struct Frame<'a> {
    /// Program ID that is executing.
    program_id: &'a Address,
    /// Raw account views.
    accounts: &'a [AccountView],
    /// Instruction data cursor.
    ix_data: SliceCursor<'a>,
    /// Borrow tracking: bit N = 1 means account N is mutably borrowed.
    /// This is a runtime check -- not as strong as the borrow checker, but
    /// catches the most dangerous pattern (double-mutable-borrow).
    mutable_borrows: u64,
}

impl<'a> Frame<'a> {
    /// Create a new execution frame.
    #[inline(always)]
    pub fn new(
        program_id: &'a Address,
        accounts: &'a [AccountView],
        instruction_data: &'a [u8],
    ) -> Result<Self, ProgramError> {
        if accounts.len() > MAX_FRAME_ACCOUNTS {
            return Err(ProgramError::InvalidArgument);
        }
        Ok(Self {
            program_id,
            accounts,
            ix_data: SliceCursor::new(instruction_data),
            mutable_borrows: 0,
        })
    }

    /// Program ID.
    #[inline(always)]
    pub fn program_id(&self) -> &Address {
        self.program_id
    }

    /// Number of accounts in this frame.
    #[inline(always)]
    pub fn account_count(&self) -> usize {
        self.accounts.len()
    }

    /// Get raw account view by index.
    #[inline(always)]
    pub fn account_view(&self, index: usize) -> Result<&AccountView, ProgramError> {
        self.accounts.get(index).ok_or(ProgramError::NotEnoughAccountKeys)
    }

    /// Get instruction data cursor.
    #[inline(always)]
    pub fn ix_data(&mut self) -> &mut SliceCursor<'a> {
        &mut self.ix_data
    }

    /// Get raw instruction data.
    #[inline(always)]
    pub fn ix_data_raw(&self) -> &[u8] {
        self.ix_data.data_from_position()
    }

    // --- Immutable Account Access -----------------------------------

    /// Get an immutable account view (no borrow tracking needed for reads).
    #[inline(always)]
    pub fn account(&self, index: usize) -> Result<FrameAccount<'_>, ProgramError> {
        let view = self.accounts.get(index).ok_or(ProgramError::NotEnoughAccountKeys)?;
        Ok(FrameAccount { view })
    }

    // --- Mutable Account Access (with borrow tracking) -------------

    /// Get a mutable account view with runtime borrow checking.
    ///
    /// Returns an error if this account is already borrowed mutably.
    /// This prevents the most dangerous aliasing pattern in Solana programs.
    #[inline]
    pub fn account_mut(
        &mut self,
        index: usize,
    ) -> Result<FrameAccountMut<'_>, ProgramError> {
        if index >= self.accounts.len() {
            return Err(ProgramError::NotEnoughAccountKeys);
        }

        let bit = 1u64 << (index as u32);
        if self.mutable_borrows & bit != 0 {
            // Already mutably borrowed -- prevent aliasing.
            return Err(ProgramError::AccountBorrowFailed);
        }

        self.mutable_borrows |= bit;
        let view = &self.accounts[index];

        Ok(FrameAccountMut {
            view,
            borrow_mask: &mut self.mutable_borrows,
            bit,
        })
    }

    // --- Validation Helpers -----------------------------------------

    /// Validate that account at `index` is a signer.
    #[inline(always)]
    pub fn require_signer(&self, index: usize) -> ProgramResult {
        crate::check::check_signer(self.account_view(index)?)
    }

    /// Validate that account at `index` is writable.
    #[inline(always)]
    pub fn require_writable(&self, index: usize) -> ProgramResult {
        crate::check::check_writable(self.account_view(index)?)
    }

    /// Validate that account at `index` is owned by this program.
    #[inline(always)]
    pub fn require_owned(&self, index: usize) -> ProgramResult {
        crate::check::check_owner(self.account_view(index)?, self.program_id)
    }

    /// Validate signer + writable (common pattern for authority accounts).
    #[inline(always)]
    pub fn require_authority(&self, index: usize) -> ProgramResult {
        let view = self.account_view(index)?;
        crate::check::check_signer(view)?;
        crate::check::check_writable(view)?;
        Ok(())
    }

    /// Validate two accounts are unique.
    #[inline(always)]
    pub fn require_unique(&self, a: usize, b: usize) -> ProgramResult {
        let va = self.account_view(a)?;
        let vb = self.account_view(b)?;
        crate::check::check_accounts_unique(va, vb)
    }

    /// Require an account matches a specific program address.
    #[inline(always)]
    pub fn require_program(&self, index: usize, program: &Address) -> ProgramResult {
        crate::check::check_address(self.account_view(index)?, program)
    }
}

/// Immutable account view within a Frame.
pub struct FrameAccount<'a> {
    view: &'a AccountView,
}

impl<'a> FrameAccount<'a> {
    /// The underlying AccountView.
    #[inline(always)]
    pub fn view(&self) -> &AccountView {
        self.view
    }

    /// The account's address.
    #[inline(always)]
    pub fn address(&self) -> &Address {
        self.view.address()
    }

    /// Borrow account data (read-only).
    #[inline(always)]
    pub fn data(&self) -> Result<Ref<'a, [u8]>, ProgramError> {
        self.view.try_borrow()
    }

    /// Lamports balance.
    #[inline(always)]
    pub fn lamports(&self) -> u64 {
        self.view.lamports()
    }

    /// Is this account a signer?
    #[inline(always)]
    pub fn is_signer(&self) -> bool {
        self.view.is_signer()
    }

    /// Is this account writable?
    #[inline(always)]
    pub fn is_writable(&self) -> bool {
        self.view.is_writable()
    }
}

/// Mutable account view within a Frame.
///
/// When this is dropped, the mutable borrow tracking bit is cleared,
/// allowing the account to be re-borrowed.
pub struct FrameAccountMut<'a> {
    view: &'a AccountView,
    borrow_mask: &'a mut u64,
    bit: u64,
}

impl<'a> FrameAccountMut<'a> {
    /// The underlying AccountView.
    #[inline(always)]
    pub fn view(&self) -> &AccountView {
        self.view
    }

    /// The account's address.
    #[inline(always)]
    pub fn address(&self) -> &Address {
        self.view.address()
    }

    /// Borrow account data (read-only).
    #[inline(always)]
    pub fn data(&self) -> Result<Ref<'a, [u8]>, ProgramError> {
        self.view.try_borrow()
    }

    /// Borrow account data (mutable).
    #[inline(always)]
    pub fn data_mut(&self) -> Result<RefMut<'a, [u8]>, ProgramError> {
        self.view.try_borrow_mut()
    }

    /// Lamports balance.
    #[inline(always)]
    pub fn lamports(&self) -> u64 {
        self.view.lamports()
    }
}

impl<'a> Drop for FrameAccountMut<'a> {
    fn drop(&mut self) {
        // Release the borrow tracking bit.
        *self.borrow_mask &= !self.bit;
    }
}
