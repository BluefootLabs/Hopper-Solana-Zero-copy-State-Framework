//! Lazy account parser -- on-demand account deserialization.
//!
//! The standard entrypoint parses every account upfront, burning CU even
//! for accounts the instruction never touches. The lazy parser gives you
//! instruction data and program ID immediately, then hands back an
//! iterator that parses accounts one at a time ON DEMAND.
//!
//! Hopper's lazy path is distinct not because Pinocchio lacks lazy parsing,
//! but because Hopper Native pre-scans the instruction tail, preserves
//! canonical duplicate-account handling in `raw_input`, and then exposes a
//! `LazyContext` that already knows `instruction_data` and `program_id`
//! before the first account is materialized.
//!
//! # CU Savings
//!
//! Programs that dispatch on `instruction_data[0]` and only need a subset
//! of accounts save measurable CU. A vault program that routes 8 instruction
//! variants through a single entrypoint might only parse 2-3 of 10 accounts
//! for a given variant.
//!
//! # Usage
//!
//! ```ignore
//! use hopper_native::lazy::LazyContext;
//! use hopper_native::hopper_lazy_entrypoint;
//!
//! hopper_lazy_entrypoint!(process);
//!
//! fn process(ctx: LazyContext) -> ProgramResult {
//!     let disc = ctx.instruction_data().first().copied().unwrap_or(0);
//!     match disc {
//!         0 => {
//!             let payer = ctx.next_account()?;
//!             let vault = ctx.next_account()?;
//!             // Remaining accounts are never parsed.
//!             do_deposit(payer, vault, &ctx.instruction_data()[1..])
//!         }
//!         _ => Err(ProgramError::InvalidInstructionData),
//!     }
//! }
//! ```

use crate::account_view::AccountView;
use crate::address::Address;
use crate::error::ProgramError;
use crate::raw_account::RuntimeAccount;
use crate::MAX_PERMITTED_DATA_INCREASE;

const BPF_ALIGN_OF_U128: usize = 8;

/// Pre-parsed header from the BPF input buffer: instruction data +
/// program ID, plus a cursor positioned at the first account.
///
/// Accounts are parsed lazily as you call `next_account()`.
pub struct LazyContext {
    /// Raw pointer into the BPF input buffer, positioned at the first
    /// account (or past the account count if num_accounts == 0).
    cursor: *mut u8,
    /// Total number of accounts declared in the input.
    total_accounts: usize,
    /// Number of accounts already parsed.
    parsed_count: usize,
    /// Instruction data slice (lifetime tied to the BPF input buffer).
    instruction_data: *const u8,
    instruction_data_len: usize,
    /// Program ID (32 bytes, copied from the input buffer tail).
    program_id: Address,
    /// Stack of already-parsed AccountViews so we can resolve duplicates
    /// that reference earlier accounts. Fixed size = MAX_TX_ACCOUNTS.
    resolved: [AccountView; 254],
}

// SAFETY: Single-threaded BPF runtime.
unsafe impl Send for LazyContext {}
unsafe impl Sync for LazyContext {}

impl LazyContext {
    /// Instruction data for this invocation.
    #[inline(always)]
    pub fn instruction_data(&self) -> &[u8] {
        // SAFETY: instruction_data points into the BPF input buffer which
        // outlives the entire instruction execution.
        unsafe { core::slice::from_raw_parts(self.instruction_data, self.instruction_data_len) }
    }

    /// The program ID of this invocation.
    #[inline(always)]
    pub fn program_id(&self) -> &Address {
        &self.program_id
    }

    /// Number of accounts declared in the transaction.
    #[inline(always)]
    pub fn total_accounts(&self) -> usize {
        self.total_accounts
    }

    /// Number of accounts parsed so far.
    #[inline(always)]
    pub fn parsed_count(&self) -> usize {
        self.parsed_count
    }

    /// Number of accounts remaining to be parsed.
    #[inline(always)]
    pub fn remaining(&self) -> usize {
        self.total_accounts - self.parsed_count
    }

    /// Parse and return the next account from the input buffer.
    ///
    /// Each call advances the internal cursor by one account. Returns
    /// `Err(NotEnoughAccountKeys)` if all accounts have been consumed.
    #[inline]
    pub fn next_account(&mut self) -> Result<AccountView, ProgramError> {
        if self.parsed_count >= self.total_accounts {
            return Err(ProgramError::NotEnoughAccountKeys);
        }

        let view = unsafe { self.parse_one_account() };
        self.resolved[self.parsed_count] = view.clone();
        self.parsed_count += 1;
        Ok(view)
    }

    /// Parse the next account and validate it is a signer.
    #[inline]
    pub fn next_signer(&mut self) -> Result<AccountView, ProgramError> {
        let acct = self.next_account()?;
        acct.require_signer()?;
        Ok(acct)
    }

    /// Parse the next account and validate it is writable.
    #[inline]
    pub fn next_writable(&mut self) -> Result<AccountView, ProgramError> {
        let acct = self.next_account()?;
        acct.require_writable()?;
        Ok(acct)
    }

    /// Parse the next account and validate it is a writable signer (payer).
    #[inline]
    pub fn next_payer(&mut self) -> Result<AccountView, ProgramError> {
        let acct = self.next_account()?;
        acct.require_payer()?;
        Ok(acct)
    }

    /// Parse the next account and validate it is owned by `program`.
    #[inline]
    pub fn next_owned_by(&mut self, program: &Address) -> Result<AccountView, ProgramError> {
        let acct = self.next_account()?;
        acct.require_owned_by(program)?;
        Ok(acct)
    }

    /// Skip `n` accounts without returning them.
    ///
    /// Advances the cursor through the raw buffer without constructing
    /// full AccountView values, only doing enough work to find account
    /// boundaries.
    #[inline]
    pub fn skip(&mut self, n: usize) -> Result<(), ProgramError> {
        for _ in 0..n {
            if self.parsed_count >= self.total_accounts {
                return Err(ProgramError::NotEnoughAccountKeys);
            }
            // Advance cursor past this account without storing it.
            unsafe { self.advance_cursor() };
            self.parsed_count += 1;
        }
        Ok(())
    }

    /// Collect all remaining accounts into a slice of the internal buffer.
    ///
    /// Parses all remaining accounts eagerly and returns them as a slice.
    /// After this call, `remaining()` returns 0.
    #[inline]
    pub fn drain_remaining(&mut self) -> Result<&[AccountView], ProgramError> {
        let start = self.parsed_count;
        while self.parsed_count < self.total_accounts {
            let view = unsafe { self.parse_one_account() };
            self.resolved[self.parsed_count] = view;
            self.parsed_count += 1;
        }
        Ok(&self.resolved[start..self.parsed_count])
    }

    /// Get an already-parsed account by index.
    ///
    /// Returns `None` if `index >= parsed_count`.
    #[inline(always)]
    pub fn get(&self, index: usize) -> Option<&AccountView> {
        if index < self.parsed_count {
            Some(&self.resolved[index])
        } else {
            None
        }
    }

    /// Parse one account at the current cursor position and advance cursor.
    ///
    /// # Safety
    ///
    /// Caller must ensure `parsed_count < total_accounts` and that `cursor`
    /// points to valid BPF input buffer data.
    #[inline(always)]
    unsafe fn parse_one_account(&mut self) -> AccountView {
        unsafe {
            let dup_marker = *self.cursor;

            if dup_marker == u8::MAX {
                // Non-duplicate: RuntimeAccount header starts here.
                let raw = self.cursor as *mut RuntimeAccount;
                let view = AccountView::new_unchecked(raw);
                self.advance_non_dup_cursor(raw);
                view
            } else {
                // Duplicate: references an earlier account.
                let original_idx = dup_marker as usize;
                self.cursor = self.cursor.add(8); // skip 8-byte padding
                                                  // The loader guarantees duplicate markers refer to
                                                  // **previously parsed** slots. A marker that points at
                                                  // ourselves or forward is malformed loader input -
                                                  // pre-audit we returned `self.resolved[0]` which is a
                                                  // zeroed `AccountView` until a real account has been
                                                  // parsed, silently handing out a null-pointer view. The
                                                  // Hopper Safety Audit flagged this; we now trap.
                if original_idx >= self.parsed_count {
                    crate::raw_input::malformed_duplicate_marker(dup_marker, self.parsed_count);
                }
                self.resolved[original_idx].clone()
            }
        }
    }

    /// Advance the cursor past one account slot without constructing a view.
    ///
    /// # Safety
    ///
    /// Caller must ensure `parsed_count < total_accounts` and cursor is valid.
    #[inline(always)]
    unsafe fn advance_cursor(&mut self) {
        unsafe {
            let dup_marker = *self.cursor;
            if dup_marker == u8::MAX {
                let raw = self.cursor as *mut RuntimeAccount;
                self.advance_non_dup_cursor(raw);
            } else {
                self.cursor = self.cursor.add(8);
            }
        }
    }

    /// Advance cursor past a non-duplicate account (shared by parse + skip).
    #[inline(always)]
    unsafe fn advance_non_dup_cursor(&mut self, raw: *mut RuntimeAccount) {
        unsafe {
            let data_len = (*raw).data_len as usize;
            let mut offset = RuntimeAccount::SIZE + data_len + MAX_PERMITTED_DATA_INCREASE;
            offset += self.cursor.add(offset).align_offset(BPF_ALIGN_OF_U128);
            offset += 8;
            self.cursor = self.cursor.add(offset);
        }
    }
}

/// Deserialize a BPF input buffer into a `LazyContext`.
///
/// Reads the account count, then scans forward to find instruction data
/// and program ID WITHOUT parsing any individual accounts. The actual
/// account parsing is deferred to `LazyContext::next_account()`.
///
/// # Safety
///
/// `input` must point to a valid Solana BPF input buffer.
#[inline(always)]
pub unsafe fn lazy_deserialize(input: *mut u8) -> LazyContext {
    let frame = unsafe { crate::raw_input::scan_instruction_frame(input) };
    // SAFETY: AccountView is a single raw pointer, zeroed is a valid
    // sentinel (null). These slots are only read after `next_account()`
    // initializes them via `parse_one_account()`.
    let resolved: [AccountView; 254] = unsafe { core::mem::zeroed() };

    LazyContext {
        cursor: frame.accounts_start,
        total_accounts: frame.account_count,
        parsed_count: 0,
        instruction_data: frame.instruction_data.as_ptr(),
        instruction_data_len: frame.instruction_data.len(),
        program_id: frame.program_id,
        resolved,
    }
}
