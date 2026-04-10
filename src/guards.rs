//! Validation guards for Hopper programs.
//!
//! These functions define the "sentence structure" of Hopper handlers.
//! They support the canonical authored flow:
//!
//! **Validate -> Load -> Mutate -> Emit**
//!
//! Each guard reads as a clear, auditable assertion. Functions return
//! `ProgramResult` (i.e. `Result<(), ProgramError>`) so they compose
//! naturally with `?`.
//!
//! ```ignore
//! use hopper::prelude::*;
//!
//! fn deposit(ctx: &Context, amount: u64) -> ProgramResult {
//!     let authority = ctx.account(0)?;
//!     let vault = ctx.account(1)?;
//!
//!     require_signer(authority)?;
//!     require_writable(vault)?;
//!     require_disc(vault, 1)?;
//!     require_owner(vault, ctx.program_id)?;
//!
//!     let state = vault.load_mut::<VaultState>()?;
//!     // ...
//!     Ok(())
//! }
//! ```

use hopper_runtime::{AccountView, Address, LayoutContract, ProgramError, ProgramResult};

/// Require a boolean condition, returning `err` if false.
#[inline(always)]
pub fn require(cond: bool, err: ProgramError) -> ProgramResult {
    if cond { Ok(()) } else { Err(err) }
}

/// Require two values to be equal.
#[inline(always)]
pub fn require_eq<T: PartialEq>(a: T, b: T, err: ProgramError) -> ProgramResult {
    if a == b { Ok(()) } else { Err(err) }
}

/// Require two values to be different.
#[inline(always)]
pub fn require_neq<T: PartialEq>(a: T, b: T, err: ProgramError) -> ProgramResult {
    if a != b { Ok(()) } else { Err(err) }
}

/// Require a >= b.
#[inline(always)]
pub fn require_gte<T: PartialOrd>(a: T, b: T, err: ProgramError) -> ProgramResult {
    if a >= b { Ok(()) } else { Err(err) }
}

/// Require a > b.
#[inline(always)]
pub fn require_gt<T: PartialOrd>(a: T, b: T, err: ProgramError) -> ProgramResult {
    if a > b { Ok(()) } else { Err(err) }
}

/// Require that the account signed the transaction.
#[inline(always)]
pub fn require_signer(account: &AccountView) -> ProgramResult {
    account.require_signer()
}

/// Require that the account is writable.
#[inline(always)]
pub fn require_writable(account: &AccountView) -> ProgramResult {
    account.require_writable()
}

/// Require both signer and writable (common payer pattern).
#[inline(always)]
pub fn require_payer(account: &AccountView) -> ProgramResult {
    account.require_payer()
}

/// Require that the account is owned by the given program.
#[inline(always)]
pub fn require_owner(account: &AccountView, owner: &Address) -> ProgramResult {
    account.require_owned_by(owner)
}

/// Require that the account has the given address.
#[inline(always)]
pub fn require_address(account: &AccountView, expected: &Address) -> ProgramResult {
    if hopper_runtime::address::address_eq(account.address(), expected) {
        Ok(())
    } else {
        Err(ProgramError::InvalidArgument)
    }
}

/// Require two addresses to be equal.
#[inline(always)]
pub fn require_keys_eq(a: &Address, b: &Address, err: ProgramError) -> ProgramResult {
    if hopper_runtime::address::address_eq(a, b) { Ok(()) } else { Err(err) }
}

/// Require two addresses to be different.
#[inline(always)]
pub fn require_keys_neq(a: &Address, b: &Address, err: ProgramError) -> ProgramResult {
    if !hopper_runtime::address::address_eq(a, b) { Ok(()) } else { Err(err) }
}

/// Require the account has the given discriminator byte.
#[inline(always)]
pub fn require_disc(account: &AccountView, expected: u8) -> ProgramResult {
    account.require_disc(expected)
}

/// Require the account passes a full layout contract check (disc + version + layout_id).
#[inline(always)]
pub fn require_layout<T: LayoutContract>(account: &AccountView) -> ProgramResult {
    account.check_layout::<T>().map(|_| ())
}

/// Require the account has non-empty data.
#[inline(always)]
pub fn require_has_data(account: &AccountView) -> ProgramResult {
    if !account.is_data_empty() { Ok(()) } else { Err(ProgramError::AccountDataTooSmall) }
}

/// Require the account has at least `min_len` bytes of data.
#[inline(always)]
pub fn require_data_len(account: &AccountView, min_len: usize) -> ProgramResult {
    if account.data_len() >= min_len { Ok(()) } else { Err(ProgramError::AccountDataTooSmall) }
}

/// Require that `n` accounts are different (pairwise uniqueness, up to 6).
///
/// For more than 6 accounts, use `check_accounts_unique!` macro from jiminy-core.
#[inline(always)]
pub fn require_unique_2(a: &AccountView, b: &AccountView) -> ProgramResult {
    if hopper_runtime::address::address_eq(a.address(), b.address()) {
        Err(ProgramError::InvalidArgument)
    } else {
        Ok(())
    }
}

/// Require the account's version byte matches the layout contract's VERSION.
#[inline(always)]
pub fn require_version<T: LayoutContract>(account: &AccountView) -> ProgramResult {
    account.check_version(T::VERSION).map(|_| ())
}

/// Require 3 accounts are pairwise unique.
#[inline(always)]
pub fn require_unique_3(a: &AccountView, b: &AccountView, c: &AccountView) -> ProgramResult {
    require_unique_2(a, b)?;
    require_unique_2(a, c)?;
    require_unique_2(b, c)
}
