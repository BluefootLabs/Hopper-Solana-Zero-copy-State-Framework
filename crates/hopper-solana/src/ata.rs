//! Associated Token Account derivation and verification.
//!
//! Derives the canonical ATA address from a wallet + mint using
//! `Address::find_program_address` with the standard seed layout:
//! `[wallet, token_program_id, mint]` + the ATA program id.
//!
//! These functions use the Solana `find_program_address` syscall and are
//! only available when targeting `target_os = "solana"`.

#[cfg(target_os = "solana")]
use hopper_runtime::{error::ProgramError, Address, ProgramResult};

#[cfg(target_os = "solana")]
use crate::constants::{ATA_PROGRAM_ID, TOKEN_2022_PROGRAM_ID, TOKEN_PROGRAM_ID};

/// Derive the canonical Associated Token Account address for the given wallet
/// and mint under the SPL Token program.
///
/// Returns `(ata_address, bump)`.
///
/// # CU Cost
/// ~1500 CU (`find_program_address` syscall).
#[cfg(target_os = "solana")]
#[inline(always)]
pub fn derive_ata(wallet: &Address, mint: &Address) -> (Address, u8) {
    derive_ata_for_program(wallet, mint, &TOKEN_PROGRAM_ID)
}

/// Derive the canonical ATA address under the Token-2022 program.
///
/// Returns `(ata_address, bump)`.
#[cfg(target_os = "solana")]
#[inline(always)]
pub fn derive_ata_2022(wallet: &Address, mint: &Address) -> (Address, u8) {
    derive_ata_for_program(wallet, mint, &TOKEN_2022_PROGRAM_ID)
}

/// Derive an ATA address for any token program id.
///
/// Seeds: `[wallet, token_program_id, mint]`, program: ATA_PROGRAM_ID.
#[cfg(target_os = "solana")]
#[inline(always)]
pub fn derive_ata_for_program(
    wallet: &Address,
    mint: &Address,
    token_program_id: &Address,
) -> (Address, u8) {
    let seeds: &[&[u8]] = &[
        wallet.as_ref(),
        token_program_id.as_ref(),
        mint.as_ref(),
    ];
    Address::find_program_address(seeds, &ATA_PROGRAM_ID)
}

/// Verify that `account_key` is the canonical ATA for the given wallet and
/// mint under the SPL Token program.
///
/// Returns `InvalidSeeds` if the address doesn't match.
#[cfg(target_os = "solana")]
#[inline(always)]
pub fn verify_ata(
    account_key: &Address,
    wallet: &Address,
    mint: &Address,
) -> ProgramResult {
    let (expected, _) = derive_ata(wallet, mint);
    if *account_key != expected {
        return Err(ProgramError::InvalidSeeds);
    }
    Ok(())
}

/// Verify that `account_key` is the canonical ATA under Token-2022.
#[cfg(target_os = "solana")]
#[inline(always)]
pub fn verify_ata_2022(
    account_key: &Address,
    wallet: &Address,
    mint: &Address,
) -> ProgramResult {
    let (expected, _) = derive_ata_2022(wallet, mint);
    if *account_key != expected {
        return Err(ProgramError::InvalidSeeds);
    }
    Ok(())
}

/// Verify that `account_key` is the canonical ATA for either SPL Token
/// or Token-2022.
///
/// Tries SPL Token first, then Token-2022. Returns `InvalidSeeds` if
/// neither matches.
#[cfg(target_os = "solana")]
#[inline(always)]
pub fn verify_ata_any(
    account_key: &Address,
    wallet: &Address,
    mint: &Address,
) -> ProgramResult {
    if verify_ata(account_key, wallet, mint).is_ok() {
        return Ok(());
    }
    verify_ata_2022(account_key, wallet, mint)
}
