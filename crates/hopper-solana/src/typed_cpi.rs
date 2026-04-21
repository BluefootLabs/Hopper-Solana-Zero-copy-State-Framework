//! Typed CPI wrappers for system, token, Token-2022, and ATA operations.
//!
//! Thin `#[inline]` functions over `hopper-system` and `hopper-token` with
//! semantically named arguments. Complement the low-level
//! `HopperCpi`/`HopperCpiBuf` builders for standard operations.

use hopper_runtime::{AccountView, Address, ProgramResult};
use hopper_runtime::error::ProgramError;
use hopper_runtime::instruction::{InstructionAccount, InstructionView, Signer};

use crate::constants::{ATA_PROGRAM_ID, TOKEN_2022_PROGRAM_ID};

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
    hopper_system::CreateAccount {
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
    hopper_system::CreateAccount {
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
    hopper_system::Transfer {
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
    hopper_system::Transfer {
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
    hopper_system::Assign {
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
    hopper_system::Allocate {
        account,
        space,
    }
    .invoke()
}

// ────────────────────────────────────────────────────────────────────
// SPL Token (via hopper-token)
// ────────────────────────────────────────────────────────────────────

/// Transfer SPL tokens between token accounts.
///
/// Prefer [`token_transfer_checked`] for the Token-2022-safe path
/// (adds mint + decimals validation at CPI time). This unchecked
/// variant remains available for pre-Token-2022 deployments.
#[deprecated(
    since = "0.2.0",
    note = "use token_transfer_checked for Token-2022 safety (mint + decimals validation)"
)]
#[allow(deprecated)]
#[inline]
pub fn token_transfer<'a>(
    source: &'a AccountView,
    destination: &'a AccountView,
    authority: &'a AccountView,
    amount: u64,
) -> ProgramResult {
    hopper_token::Transfer {
        from: source,
        to: destination,
        authority,
        amount,
    }
    .invoke()
}

/// Transfer SPL tokens between token accounts with mint + decimals
/// validation (Token-2022-safe).
///
/// The SPL token program rejects the CPI if the mint's stored
/// decimals disagree with the supplied value, which defends against
/// wrong-mint attacks where the caller passed a different mint than
/// the account expects.
#[inline]
pub fn token_transfer_checked<'a>(
    source: &'a AccountView,
    mint: &'a AccountView,
    destination: &'a AccountView,
    authority: &'a AccountView,
    amount: u64,
    decimals: u8,
) -> ProgramResult {
    hopper_token::TransferChecked {
        from: source,
        mint,
        to: destination,
        authority,
        amount,
        decimals,
    }
    .invoke()
}

/// `token_transfer_checked` with explicit PDA signer seeds.
#[inline]
pub fn token_transfer_checked_signed<'a>(
    source: &'a AccountView,
    mint: &'a AccountView,
    destination: &'a AccountView,
    authority: &'a AccountView,
    amount: u64,
    decimals: u8,
    signers: &[Signer],
) -> ProgramResult {
    hopper_token::TransferChecked {
        from: source,
        mint,
        to: destination,
        authority,
        amount,
        decimals,
    }
    .invoke_signed(signers)
}

/// Transfer SPL tokens with PDA signer seeds.
///
/// Prefer [`token_transfer_checked_signed`] for Token-2022 safety.
#[deprecated(
    since = "0.2.0",
    note = "use token_transfer_checked_signed for Token-2022 safety"
)]
#[allow(deprecated)]
#[inline]
pub fn token_transfer_signed<'a>(
    source: &'a AccountView,
    destination: &'a AccountView,
    authority: &'a AccountView,
    amount: u64,
    signers: &[Signer],
) -> ProgramResult {
    hopper_token::Transfer {
        from: source,
        to: destination,
        authority,
        amount,
    }
    .invoke_signed(signers)
}

/// Mint tokens to a destination token account.
///
/// Prefer [`token_mint_to_checked`] for Token-2022 safety.
#[deprecated(
    since = "0.2.0",
    note = "use token_mint_to_checked for Token-2022 safety (mint + decimals validation)"
)]
#[allow(deprecated)]
#[inline]
pub fn token_mint_to<'a>(
    mint: &'a AccountView,
    destination: &'a AccountView,
    authority: &'a AccountView,
    amount: u64,
) -> ProgramResult {
    hopper_token::MintTo {
        mint,
        account: destination,
        mint_authority: authority,
        amount,
    }
    .invoke()
}

/// Mint tokens with PDA signer seeds.
///
/// Prefer [`token_mint_to_checked_signed`] for Token-2022 safety.
#[deprecated(
    since = "0.2.0",
    note = "use token_mint_to_checked_signed for Token-2022 safety"
)]
#[allow(deprecated)]
#[inline]
pub fn token_mint_to_signed<'a>(
    mint: &'a AccountView,
    destination: &'a AccountView,
    authority: &'a AccountView,
    amount: u64,
    signers: &[Signer],
) -> ProgramResult {
    hopper_token::MintTo {
        mint,
        account: destination,
        mint_authority: authority,
        amount,
    }
    .invoke_signed(signers)
}

/// Mint tokens with mint + decimals validation (Token-2022-safe).
#[inline]
pub fn token_mint_to_checked<'a>(
    mint: &'a AccountView,
    destination: &'a AccountView,
    authority: &'a AccountView,
    amount: u64,
    decimals: u8,
) -> ProgramResult {
    hopper_token::MintToChecked {
        mint,
        account: destination,
        mint_authority: authority,
        amount,
        decimals,
    }
    .invoke()
}

/// `token_mint_to_checked` with explicit PDA signer seeds.
#[inline]
pub fn token_mint_to_checked_signed<'a>(
    mint: &'a AccountView,
    destination: &'a AccountView,
    authority: &'a AccountView,
    amount: u64,
    decimals: u8,
    signers: &[Signer],
) -> ProgramResult {
    hopper_token::MintToChecked {
        mint,
        account: destination,
        mint_authority: authority,
        amount,
        decimals,
    }
    .invoke_signed(signers)
}

/// Burn tokens from a token account.
///
/// Prefer [`token_burn_checked`] for Token-2022 safety.
#[deprecated(
    since = "0.2.0",
    note = "use token_burn_checked for Token-2022 safety (mint + decimals validation)"
)]
#[allow(deprecated)]
#[inline]
pub fn token_burn<'a>(
    token_account: &'a AccountView,
    mint: &'a AccountView,
    authority: &'a AccountView,
    amount: u64,
) -> ProgramResult {
    hopper_token::Burn {
        account: token_account,
        mint,
        authority,
        amount,
    }
    .invoke()
}

/// Burn tokens with PDA signer seeds.
///
/// Prefer [`token_burn_checked_signed`] for Token-2022 safety.
#[deprecated(
    since = "0.2.0",
    note = "use token_burn_checked_signed for Token-2022 safety"
)]
#[allow(deprecated)]
#[inline]
pub fn token_burn_signed<'a>(
    token_account: &'a AccountView,
    mint: &'a AccountView,
    authority: &'a AccountView,
    amount: u64,
    signers: &[Signer],
) -> ProgramResult {
    hopper_token::Burn {
        account: token_account,
        mint,
        authority,
        amount,
    }
    .invoke_signed(signers)
}

/// Burn tokens with mint + decimals validation (Token-2022-safe).
#[inline]
pub fn token_burn_checked<'a>(
    token_account: &'a AccountView,
    mint: &'a AccountView,
    authority: &'a AccountView,
    amount: u64,
    decimals: u8,
) -> ProgramResult {
    hopper_token::BurnChecked {
        account: token_account,
        mint,
        authority,
        amount,
        decimals,
    }
    .invoke()
}

/// `token_burn_checked` with explicit PDA signer seeds.
#[inline]
pub fn token_burn_checked_signed<'a>(
    token_account: &'a AccountView,
    mint: &'a AccountView,
    authority: &'a AccountView,
    amount: u64,
    decimals: u8,
    signers: &[Signer],
) -> ProgramResult {
    hopper_token::BurnChecked {
        account: token_account,
        mint,
        authority,
        amount,
        decimals,
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
    hopper_token::CloseAccount {
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
    hopper_token::CloseAccount {
        account: token_account,
        destination,
        authority,
    }
    .invoke_signed(signers)
}

/// Approve a delegate for a token account.
///
/// Prefer [`token_approve_checked`] for Token-2022 safety.
#[deprecated(
    since = "0.2.0",
    note = "use token_approve_checked for Token-2022 safety (mint + decimals validation)"
)]
#[allow(deprecated)]
#[inline]
pub fn token_approve<'a>(
    token_account: &'a AccountView,
    delegate: &'a AccountView,
    authority: &'a AccountView,
    amount: u64,
) -> ProgramResult {
    hopper_token::Approve {
        source: token_account,
        delegate,
        authority,
        amount,
    }
    .invoke()
}

/// Approve a delegate with mint + decimals validation (Token-2022-safe).
#[inline]
pub fn token_approve_checked<'a>(
    token_account: &'a AccountView,
    mint: &'a AccountView,
    delegate: &'a AccountView,
    authority: &'a AccountView,
    amount: u64,
    decimals: u8,
) -> ProgramResult {
    hopper_token::ApproveChecked {
        source: token_account,
        mint,
        delegate,
        authority,
        amount,
        decimals,
    }
    .invoke()
}

/// Revoke a delegate from a token account.
#[inline]
pub fn token_revoke<'a>(
    token_account: &'a AccountView,
    authority: &'a AccountView,
) -> ProgramResult {
    hopper_token::Revoke {
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
///
/// Prefer [`token_transfer_checked`] (SPL's `TransferChecked` at CPI
/// level) over this host-side mint-match check. This function
/// remains for pre-Token-2022 deployments; new code should route
/// through the runtime-level checked variant.
#[deprecated(
    since = "0.2.0",
    note = "use token_transfer_checked (SPL TransferChecked CPI) for Token-2022 safety"
)]
#[allow(deprecated)]
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

    hopper_token::Transfer {
        from: source,
        to: destination,
        authority,
        amount,
    }
    .invoke()
}

// ────────────────────────────────────────────────────────────────────
// Token-2022 (via hopper-token-2022)
// ────────────────────────────────────────────────────────────────────

/// Transfer Token-2022 tokens between token accounts.
#[inline]
pub fn token_2022_transfer<'a>(
    source: &'a AccountView,
    destination: &'a AccountView,
    authority: &'a AccountView,
    amount: u64,
) -> ProgramResult {
    token_2022_transfer_signed(source, destination, authority, amount, &[])
}

/// Transfer Token-2022 tokens with PDA signer seeds.
#[inline]
pub fn token_2022_transfer_signed<'a>(
    source: &'a AccountView,
    destination: &'a AccountView,
    authority: &'a AccountView,
    amount: u64,
    signers: &[Signer],
) -> ProgramResult {
    let mut data = [0u8; 9];
    data[0] = 3;
    data[1..9].copy_from_slice(&amount.to_le_bytes());

    let accounts = [
        InstructionAccount::writable(source.address()),
        InstructionAccount::writable(destination.address()),
        InstructionAccount::readonly_signer(authority.address()),
    ];
    let views = [source, destination, authority];
    let instruction = InstructionView {
        program_id: &TOKEN_2022_PROGRAM_ID,
        data: &data,
        accounts: &accounts,
    };

    hopper_runtime::cpi::invoke_signed(&instruction, &views, signers)
}

/// Mint Token-2022 tokens to a destination token account.
#[inline]
pub fn token_2022_mint_to<'a>(
    mint: &'a AccountView,
    destination: &'a AccountView,
    authority: &'a AccountView,
    amount: u64,
) -> ProgramResult {
    token_2022_mint_to_signed(mint, destination, authority, amount, &[])
}

/// Mint Token-2022 tokens with PDA signer seeds.
#[inline]
pub fn token_2022_mint_to_signed<'a>(
    mint: &'a AccountView,
    destination: &'a AccountView,
    authority: &'a AccountView,
    amount: u64,
    signers: &[Signer],
) -> ProgramResult {
    let mut data = [0u8; 9];
    data[0] = 7;
    data[1..9].copy_from_slice(&amount.to_le_bytes());

    let accounts = [
        InstructionAccount::writable(mint.address()),
        InstructionAccount::writable(destination.address()),
        InstructionAccount::readonly_signer(authority.address()),
    ];
    let views = [mint, destination, authority];
    let instruction = InstructionView {
        program_id: &TOKEN_2022_PROGRAM_ID,
        data: &data,
        accounts: &accounts,
    };

    hopper_runtime::cpi::invoke_signed(&instruction, &views, signers)
}

/// Burn Token-2022 tokens from a token account.
#[inline]
pub fn token_2022_burn<'a>(
    token_account: &'a AccountView,
    mint: &'a AccountView,
    authority: &'a AccountView,
    amount: u64,
) -> ProgramResult {
    let mut data = [0u8; 9];
    data[0] = 8;
    data[1..9].copy_from_slice(&amount.to_le_bytes());

    let accounts = [
        InstructionAccount::writable(token_account.address()),
        InstructionAccount::writable(mint.address()),
        InstructionAccount::readonly_signer(authority.address()),
    ];
    let views = [token_account, mint, authority];
    let instruction = InstructionView {
        program_id: &TOKEN_2022_PROGRAM_ID,
        data: &data,
        accounts: &accounts,
    };

    hopper_runtime::cpi::invoke(&instruction, &views)
}

/// Create an associated token account.
#[inline]
pub fn ata_create<'a>(
    payer: &'a AccountView,
    associated_account: &'a AccountView,
    wallet: &'a AccountView,
    mint: &'a AccountView,
    system_program: &'a AccountView,
    token_program: &'a AccountView,
) -> ProgramResult {
    let data = [0u8];
    let accounts = [
        InstructionAccount::writable_signer(payer.address()),
        InstructionAccount::writable(associated_account.address()),
        InstructionAccount::readonly(wallet.address()),
        InstructionAccount::readonly(mint.address()),
        InstructionAccount::readonly(system_program.address()),
        InstructionAccount::readonly(token_program.address()),
    ];
    let views = [payer, associated_account, wallet, mint, system_program, token_program];
    let instruction = InstructionView {
        program_id: &ATA_PROGRAM_ID,
        data: &data,
        accounts: &accounts,
    };

    hopper_runtime::cpi::invoke(&instruction, &views)
}

/// Create an associated token account idempotently.
#[inline]
pub fn ata_create_idempotent<'a>(
    payer: &'a AccountView,
    associated_account: &'a AccountView,
    wallet: &'a AccountView,
    mint: &'a AccountView,
    system_program: &'a AccountView,
    token_program: &'a AccountView,
) -> ProgramResult {
    let data = [1u8];
    let accounts = [
        InstructionAccount::writable_signer(payer.address()),
        InstructionAccount::writable(associated_account.address()),
        InstructionAccount::readonly(wallet.address()),
        InstructionAccount::readonly(mint.address()),
        InstructionAccount::readonly(system_program.address()),
        InstructionAccount::readonly(token_program.address()),
    ];
    let views = [payer, associated_account, wallet, mint, system_program, token_program];
    let instruction = InstructionView {
        program_id: &ATA_PROGRAM_ID,
        data: &data,
        accounts: &accounts,
    };

    hopper_runtime::cpi::invoke(&instruction, &views)
}

/// Recover a nested associated token account.
#[inline]
pub fn ata_recover_nested<'a>(
    nested_associated_account: &'a AccountView,
    nested_token_mint: &'a AccountView,
    destination_associated_account: &'a AccountView,
    owner_associated_account: &'a AccountView,
    owner_token_mint: &'a AccountView,
    wallet: &'a AccountView,
    token_program: &'a AccountView,
) -> ProgramResult {
    let data = [2u8];
    let accounts = [
        InstructionAccount::writable(nested_associated_account.address()),
        InstructionAccount::readonly(nested_token_mint.address()),
        InstructionAccount::writable(destination_associated_account.address()),
        InstructionAccount::readonly(owner_associated_account.address()),
        InstructionAccount::readonly(owner_token_mint.address()),
        InstructionAccount::writable_signer(wallet.address()),
        InstructionAccount::readonly(token_program.address()),
    ];
    let views = [
        nested_associated_account,
        nested_token_mint,
        destination_associated_account,
        owner_associated_account,
        owner_token_mint,
        wallet,
        token_program,
    ];
    let instruction = InstructionView {
        program_id: &ATA_PROGRAM_ID,
        data: &data,
        accounts: &accounts,
    };

    hopper_runtime::cpi::invoke(&instruction, &views)
}
