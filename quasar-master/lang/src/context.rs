//! Instruction context types used by the `dispatch!` macro.
//!
//! Three levels of context exist, each wrapping the previous:
//!
//! - `Context` — raw entrypoint data (program ID, account slice, instruction
//!   data). Produced by the entrypoint; consumed by `Ctx::new()` or
//!   `CtxWithRemaining::new()`.
//!
//! - `Ctx` — parsed and validated accounts with PDA bumps. Use this for most
//!   instructions where remaining accounts are not needed.
//!
//! - `CtxWithRemaining` — like `Ctx` but also captures the remaining accounts
//!   region for instructions that inspect or forward trailing accounts.

use crate::{prelude::*, remaining::RemainingAccounts, traits::ParseAccountsUnchecked};

/// Cast `&[u8; 32]` to `&Address`.
///
/// The entrypoint owns the original 32-byte program-id storage for the entire
/// instruction, so the returned reference is valid for `'info`. This avoids
/// copying the program ID into a stack-local `Address` on every dispatch path.
#[inline(always)]
unsafe fn as_address(bytes: &[u8; 32]) -> &Address {
    &*(bytes as *const [u8; 32] as *const Address)
}

/// Raw entrypoint context before parsing.
///
/// Produced by the `dispatch!` macro from the entrypoint's raw pointers.
/// Consumed by [`Ctx::new()`] or [`CtxWithRemaining::new()`] which parse
/// and validate the accounts.
pub struct Context<'info> {
    /// 32-byte program ID passed by the runtime.
    pub program_id: &'info [u8; 32],

    /// Declared accounts (first `N` accounts deserialized from the input).
    pub accounts: &'info mut [AccountView],

    /// Pointer to the first remaining account (past the declared accounts).
    pub remaining_ptr: *mut u8,

    /// Raw instruction data (discriminator already consumed by `dispatch!`).
    pub data: &'info [u8],

    /// End of accounts region: `ix_data_ptr - sizeof(u64)`.
    pub accounts_boundary: *const u8,
}

/// Parsed instruction context with typed accounts and PDA bumps.
///
/// Use [`CtxWithRemaining`] for instructions that need
/// `remaining_accounts()`.
pub struct Ctx<'info, T: ParseAccounts<'info> + ParseAccountsUnchecked<'info> + AccountCount> {
    /// Validated and typed account struct.
    pub accounts: T,

    /// PDA bump seeds discovered during validation.
    pub bumps: T::Bumps,

    /// 32-byte program ID (raw bytes, not [`Address`]).
    pub program_id: &'info [u8; 32],

    /// Instruction data with discriminator already consumed.
    pub data: &'info [u8],
}

impl<'info, T: ParseAccounts<'info> + ParseAccountsUnchecked<'info> + AccountCount> Ctx<'info, T> {
    #[inline(always)]
    pub fn new(ctx: Context<'info>) -> Result<Self, ProgramError> {
        let program_id_addr = unsafe { as_address(ctx.program_id) };
        let (accounts, bumps) = unsafe {
            T::parse_with_instruction_data_unchecked(ctx.accounts, ctx.data, program_id_addr)?
        };
        Ok(Self {
            accounts,
            bumps,
            program_id: ctx.program_id,
            data: ctx.data,
        })
    }

    /// Compile-time check for whether `T` has a custom `validate()` override.
    #[inline(always)]
    pub const fn has_validate(&self) -> bool {
        T::HAS_VALIDATE
    }
}

/// Like [`Ctx`] but also captures the remaining accounts region.
///
/// Use this for instructions that call `remaining_accounts()` — e.g.
/// when inspecting trailing accounts in local logic, or
/// `remaining_accounts_passthrough()` when forwarding a variable number of
/// accounts to a downstream CPI.
pub struct CtxWithRemaining<
    'info,
    T: ParseAccounts<'info> + ParseAccountsUnchecked<'info> + AccountCount,
> {
    /// Validated and typed account struct.
    pub accounts: T,

    /// PDA bump seeds discovered during validation.
    pub bumps: T::Bumps,

    /// 32-byte program ID (raw bytes).
    pub program_id: &'info [u8; 32],

    /// Instruction data with discriminator already consumed.
    pub data: &'info [u8],

    /// Pointer to the first remaining account in the input buffer.
    remaining_ptr: *mut u8,

    /// Declared accounts slice (for duplicate resolution in remaining).
    declared: &'info [AccountView],

    /// End-of-accounts boundary pointer.
    accounts_boundary: *const u8,
}

impl<'info, T: ParseAccounts<'info> + ParseAccountsUnchecked<'info> + AccountCount>
    CtxWithRemaining<'info, T>
{
    #[inline(always)]
    pub fn new(ctx: Context<'info>) -> Result<Self, ProgramError> {
        let program_id_addr = unsafe { as_address(ctx.program_id) };
        // Save slice metadata before parse consumes the &mut borrow.
        // The declared `AccountView`s are copied by value during parsing, so the
        // backing slice header stays valid for read-only duplicate resolution
        // after `parse_with_instruction_data_unchecked` returns.
        let declared_ptr = ctx.accounts.as_ptr();
        let declared_len = ctx.accounts.len();
        let (accounts, bumps) = unsafe {
            T::parse_with_instruction_data_unchecked(ctx.accounts, ctx.data, program_id_addr)?
        };
        let declared = unsafe { core::slice::from_raw_parts(declared_ptr, declared_len) };
        Ok(Self {
            accounts,
            bumps,
            program_id: ctx.program_id,
            data: ctx.data,
            remaining_ptr: ctx.remaining_ptr,
            declared,
            accounts_boundary: ctx.accounts_boundary,
        })
    }

    /// Compile-time check for whether `T` has a custom `validate()` override.
    #[inline(always)]
    pub const fn has_validate(&self) -> bool {
        T::HAS_VALIDATE
    }

    /// Strict remaining-account accessor.
    ///
    /// Rejects any duplicate of a declared or prior remaining account. Use
    /// this for local program logic so each trailing account has a unique
    /// identity within the instruction context.
    #[inline(always)]
    pub fn remaining_accounts(&self) -> RemainingAccounts<'info> {
        RemainingAccounts::new(self.remaining_ptr, self.accounts_boundary, self.declared)
    }

    /// Passthrough remaining-account accessor.
    ///
    /// Preserves duplicate account metas exactly as they appeared in the input
    /// for CPI forwarding scenarios. Prefer `remaining_accounts()` unless you
    /// explicitly need Solana's raw duplicate-meta behavior.
    #[inline(always)]
    pub fn remaining_accounts_passthrough(&self) -> RemainingAccounts<'info> {
        RemainingAccounts::new_passthrough(
            self.remaining_ptr,
            self.accounts_boundary,
            self.declared,
        )
    }
}
