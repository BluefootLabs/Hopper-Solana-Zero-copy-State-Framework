//! PDA derivation helpers for Metaplex's metadata and master-edition
//! accounts.
//!
//! Metaplex's metadata PDA seeds: `["metadata", mpl_token_metadata_program_id, mint]`.
//! Master-edition PDA seeds:        `["metadata", mpl_token_metadata_program_id, mint, "edition"]`.
//!
//! The `mpl_token_metadata_program_id` appears as a literal seed inside
//! the seed list (this is unusual — most programs put their own ID as
//! the third argument to `find_program_address`, not as a seed). Both
//! shapes are PDA-derived using the Metaplex program ID as the
//! `program_id` argument too. The helpers below construct both seed
//! slices and call into Hopper's PDA derivation path.

#[cfg(target_os = "solana")]
use crate::constants::{EDITION_SEED_PREFIX, METADATA_SEED_PREFIX, MPL_TOKEN_METADATA_PROGRAM_ID};
use hopper_runtime::address::Address;

/// Derive the metadata PDA address for `mint` and return it together
/// with the bump seed.
///
/// On-chain this is a single `sol_create_program_address`-style call
/// that walks bumps 255 → 0; the helper costs ~1500 CU on a worst-case
/// derivation but is typically called once per instruction.
#[cfg(target_os = "solana")]
pub fn metadata_pda(mint: &Address) -> (Address, u8) {
    let seeds: [&[u8]; 3] = [
        METADATA_SEED_PREFIX,
        MPL_TOKEN_METADATA_PROGRAM_ID.as_array(),
        mint.as_array(),
    ];
    hopper_runtime::pda::find_program_address(&seeds, &MPL_TOKEN_METADATA_PROGRAM_ID)
}

/// Derive the master-edition PDA address for `mint` and return it
/// together with the bump seed.
#[cfg(target_os = "solana")]
pub fn master_edition_pda(mint: &Address) -> (Address, u8) {
    let seeds: [&[u8]; 4] = [
        METADATA_SEED_PREFIX,
        MPL_TOKEN_METADATA_PROGRAM_ID.as_array(),
        mint.as_array(),
        EDITION_SEED_PREFIX,
    ];
    hopper_runtime::pda::find_program_address(&seeds, &MPL_TOKEN_METADATA_PROGRAM_ID)
}

/// Verify that `expected` is the metadata PDA for `mint` using a
/// stored bump byte. Cheaper than `metadata_pda` because it skips the
/// bump-iteration loop.
#[cfg(target_os = "solana")]
pub fn metadata_pda_with_bump(
    mint: &Address,
    bump: u8,
) -> Result<Address, hopper_runtime::error::ProgramError> {
    let seeds: [&[u8]; 4] = [
        METADATA_SEED_PREFIX,
        MPL_TOKEN_METADATA_PROGRAM_ID.as_array(),
        mint.as_array(),
        &[bump],
    ];
    hopper_runtime::pda::create_program_address(&seeds, &MPL_TOKEN_METADATA_PROGRAM_ID)
}

/// Verify that `expected` is the master-edition PDA for `mint` using
/// a stored bump byte.
#[cfg(target_os = "solana")]
pub fn master_edition_pda_with_bump(
    mint: &Address,
    bump: u8,
) -> Result<Address, hopper_runtime::error::ProgramError> {
    let seeds: [&[u8]; 5] = [
        METADATA_SEED_PREFIX,
        MPL_TOKEN_METADATA_PROGRAM_ID.as_array(),
        mint.as_array(),
        EDITION_SEED_PREFIX,
        &[bump],
    ];
    hopper_runtime::pda::create_program_address(&seeds, &MPL_TOKEN_METADATA_PROGRAM_ID)
}

// Off-chain stubs so the crate compiles in host tests without the
// Solana target's PDA syscalls. Programs targeting on-chain only ever
// see the `cfg(target_os = "solana")` versions above.
#[cfg(not(target_os = "solana"))]
pub fn metadata_pda(_mint: &Address) -> (Address, u8) {
    (Address::new_from_array([0u8; 32]), 0)
}
#[cfg(not(target_os = "solana"))]
pub fn master_edition_pda(_mint: &Address) -> (Address, u8) {
    (Address::new_from_array([0u8; 32]), 0)
}
#[cfg(not(target_os = "solana"))]
pub fn metadata_pda_with_bump(
    _mint: &Address,
    _bump: u8,
) -> Result<Address, hopper_runtime::error::ProgramError> {
    Ok(Address::new_from_array([0u8; 32]))
}
#[cfg(not(target_os = "solana"))]
pub fn master_edition_pda_with_bump(
    _mint: &Address,
    _bump: u8,
) -> Result<Address, hopper_runtime::error::ProgramError> {
    Ok(Address::new_from_array([0u8; 32]))
}
