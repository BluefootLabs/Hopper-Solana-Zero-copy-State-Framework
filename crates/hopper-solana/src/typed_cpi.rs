//! Typed CPI wrappers for system and SPL token operations.
//!
//! Thin `#[inline]` functions over `hopper_runtime::system` and `hopper_runtime::token`
//! with semantically named arguments. Complement the low-level
//! `HopperCpi`/`HopperCpiBuf` builders for standard operations.

use hopper_runtime::{AccountView, Address, ProgramResult};
use hopper_runtime::error::ProgramError;
use hopper_runtime::instruction::Signer;

// ────────────────────────────────────────────────────────────────────
// System Program
// ────────────────────────────────────────────────────────────────────

/// Create a new account owned by `owner` with `space` bytes.
#[inline]
pub fn create_account<'a>(
    payer: &'a AccountView,
    new_account: &'a AccountView,
    lamports: u64,
    space: u64,
    owner: &'a Address,
) -> ProgramResult {
    hopper_runtime::system::CreateAccount {
        from: payer,
        to: new_account,
        lamports,
        space,
        owner,
    }
    .invoke()
}

/// Create a new account with PDA signer seeds.
#[inline]
pub fn create_account_signed<'a>(
    payer: &'a AccountView,
    new_account: &'a AccountView,
    lamports: u64,
    space: u64,
    owner: &'a Address,
    signers: &[Signer],
) -> ProgramResult {
    hopper_runtime::system::CreateAccount {
        from: payer,
        to: new_account,
        lamports,
        space,
        owner,
    }
    .invoke_signed(signers)
}

/// Transfer lamports between accounts.
#[inline]
pub fn transfer_sol<'a>(
    from: &'a AccountView,
    to: &'a AccountView,
    lamports: u64,
) -> ProgramResult {
    hopper_runtime::system::Transfer {
        from,
        to,
        lamports,
    }
    .invoke()
}

/// Transfer lamports with PDA signer seeds.
#[inline]
pub fn transfer_sol_signed<'a>(
    from: &'a AccountView,
    to: &'a AccountView,
    lamports: u64,
    signers: &[Signer],
) -> ProgramResult {
    hopper_runtime::system::Transfer {
        from,
        to,
        lamports,
    }
    .invoke_signed(signers)
}

/// Assign account ownership to a new program.
#[inline]
pub fn assign<'a>(
    account: &'a AccountView,
    owner: &'a Address,
) -> ProgramResult {
    hopper_runtime::system::Assign {
        account,
        owner,
    }
    .invoke()
}

/// Allocate space in an account (without changing owner).
#[inline]
pub fn allocate(
    account: &AccountView,
    space: u64,
) -> ProgramResult {
    hopper_runtime::system::Allocate {
        account,
        space,
    }
    .invoke()
}

// ────────────────────────────────────────────────────────────────────
// SPL Token (via hopper_runtime::token)
// ────────────────────────────────────────────────────────────────────

/// Transfer SPL tokens between token accounts.
#[inline]
pub fn token_transfer<'a>(
    source: &'a AccountView,
    destination: &'a AccountView,
    authority: &'a AccountView,
    amount: u64,
) -> ProgramResult {
    hopper_runtime::token::Transfer {
        from: source,
        to: destination,
        authority,
        amount,
    }
    .invoke()
}

/// Transfer SPL tokens with PDA signer seeds.
#[inline]
pub fn token_transfer_signed<'a>(
    source: &'a AccountView,
    destination: &'a AccountView,
    authority: &'a AccountView,
    amount: u64,
    signers: &[Signer],
) -> ProgramResult {
    hopper_runtime::token::Transfer {
        from: source,
        to: destination,
        authority,
        amount,
    }
    .invoke_signed(signers)
}

/// Mint tokens to a destination token account.
#[inline]
pub fn token_mint_to<'a>(
    mint: &'a AccountView,
    destination: &'a AccountView,
    authority: &'a AccountView,
    amount: u64,
) -> ProgramResult {
    hopper_runtime::token::MintTo {
        mint,
        account: destination,
        mint_authority: authority,
        amount,
    }
    .invoke()
}

/// Mint tokens with PDA signer seeds.
#[inline]
pub fn token_mint_to_signed<'a>(
    mint: &'a AccountView,
    destination: &'a AccountView,
    authority: &'a AccountView,
    amount: u64,
    signers: &[Signer],
) -> ProgramResult {
    hopper_runtime::token::MintTo {
        mint,
        account: destination,
        mint_authority: authority,
        amount,
    }
    .invoke_signed(signers)
}

/// Burn tokens from a token account.
#[inline]
pub fn token_burn<'a>(
    token_account: &'a AccountView,
    mint: &'a AccountView,
    authority: &'a AccountView,
    amount: u64,
) -> ProgramResult {
    hopper_runtime::token::Burn {
        account: token_account,
        mint,
        authority,
        amount,
    }
    .invoke()
}

/// Burn tokens with PDA signer seeds.
#[inline]
pub fn token_burn_signed<'a>(
    token_account: &'a AccountView,
    mint: &'a AccountView,
    authority: &'a AccountView,
    amount: u64,
    signers: &[Signer],
) -> ProgramResult {
    hopper_runtime::token::Burn {
        account: token_account,
        mint,
        authority,
        amount,
    }
    .invoke_signed(signers)
}

/// Close a token account, returning remaining lamports to destination.
#[inline]
pub fn token_close_account<'a>(
    token_account: &'a AccountView,
    destination: &'a AccountView,
    authority: &'a AccountView,
) -> ProgramResult {
    hopper_runtime::token::CloseAccount {
        account: token_account,
        destination,
        authority,
    }
    .invoke()
}

/// Close a token account with PDA signer seeds.
#[inline]
pub fn token_close_account_signed<'a>(
    token_account: &'a AccountView,
    destination: &'a AccountView,
    authority: &'a AccountView,
    signers: &[Signer],
) -> ProgramResult {
    hopper_runtime::token::CloseAccount {
        account: token_account,
        destination,
        authority,
    }
    .invoke_signed(signers)
}

/// Approve a delegate for a token account.
#[inline]
pub fn token_approve<'a>(
    token_account: &'a AccountView,
    delegate: &'a AccountView,
    authority: &'a AccountView,
    amount: u64,
) -> ProgramResult {
    hopper_runtime::token::Approve {
        source: token_account,
        delegate,
        authority,
        amount,
    }
    .invoke()
}

/// Revoke a delegate from a token account.
#[inline]
pub fn token_revoke<'a>(
    token_account: &'a AccountView,
    authority: &'a AccountView,
) -> ProgramResult {
    hopper_runtime::token::Revoke {
        source: token_account,
        authority,
    }
    .invoke()
}

// ────────────────────────────────────────────────────────────────────
// Checked Transfer (with mint verification)
// ────────────────────────────────────────────────────────────────────

/// Transfer SPL tokens with mint validation.
///
/// Reads both token accounts' mint fields and verifies they match
/// before performing the transfer. Catches mint mismatch bugs at
/// the CPI boundary.
#[inline]
pub fn checked_token_transfer<'a>(
    source: &'a AccountView,
    destination: &'a AccountView,
    authority: &'a AccountView,
    amount: u64,
) -> ProgramResult {
    let source_data = source.try_borrow()?;
    let dest_data = destination.try_borrow()?;

    let source_mint = crate::token::token_account_mint(&source_data)?;
    let dest_mint = crate::token::token_account_mint(&dest_data)?;
    if source_mint != dest_mint {
        return Err(ProgramError::InvalidAccountData);
    }

    hopper_runtime::token::Transfer {
        from: source,
        to: destination,
        authority,
        amount,
    }
    .invoke()
}
