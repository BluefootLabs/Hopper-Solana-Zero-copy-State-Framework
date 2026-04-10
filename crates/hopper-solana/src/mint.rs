//! Zero-copy SPL Mint account readers.
//!
//! Read mint fields directly from raw bytes at fixed offsets.

use hopper_runtime::error::ProgramError;
use hopper_runtime::Address;

/// SPL Mint Account total size.
pub const MINT_LEN: usize = 82;

// Field offsets within an SPL Mint Account (v1 layout)
const MINT_AUTH_OFFSET: usize = 0; // COption<Pubkey>: 4 tag + 32 pubkey
const SUPPLY_OFFSET: usize = 36;
const DECIMALS_OFFSET: usize = 44;
const IS_INIT_OFFSET: usize = 45;
const FREEZE_AUTH_OFFSET: usize = 46; // COption<Pubkey>: 4 tag + 32 pubkey

/// Read the supply from a mint account.
#[inline(always)]
pub fn mint_supply(data: &[u8]) -> Result<u64, ProgramError> {
    if data.len() < MINT_LEN {
        return Err(ProgramError::InvalidAccountData);
    }
    Ok(u64::from_le_bytes([
        data[SUPPLY_OFFSET],
        data[SUPPLY_OFFSET + 1],
        data[SUPPLY_OFFSET + 2],
        data[SUPPLY_OFFSET + 3],
        data[SUPPLY_OFFSET + 4],
        data[SUPPLY_OFFSET + 5],
        data[SUPPLY_OFFSET + 6],
        data[SUPPLY_OFFSET + 7],
    ]))
}

/// Read the decimals from a mint account.
#[inline(always)]
pub fn mint_decimals(data: &[u8]) -> Result<u8, ProgramError> {
    if data.len() < MINT_LEN {
        return Err(ProgramError::InvalidAccountData);
    }
    Ok(data[DECIMALS_OFFSET])
}

/// Check the mint is initialized.
#[inline(always)]
pub fn check_mint_initialized(data: &[u8]) -> Result<(), ProgramError> {
    if data.len() < MINT_LEN {
        return Err(ProgramError::InvalidAccountData);
    }
    if data[IS_INIT_OFFSET] == 0 {
        return Err(ProgramError::UninitializedAccount);
    }
    Ok(())
}

/// Read the mint authority (returns None if COption tag is 0).
#[inline(always)]
pub fn mint_authority(data: &[u8]) -> Result<Option<&Address>, ProgramError> {
    if data.len() < MINT_LEN {
        return Err(ProgramError::InvalidAccountData);
    }
    let tag = u32::from_le_bytes([
        data[MINT_AUTH_OFFSET],
        data[MINT_AUTH_OFFSET + 1],
        data[MINT_AUTH_OFFSET + 2],
        data[MINT_AUTH_OFFSET + 3],
    ]);
    if tag == 0 {
        return Ok(None);
    }
    // SAFETY: Length checked. Pubkey is [u8; 32], alignment 1.
    Ok(Some(unsafe {
        &*(data.as_ptr().add(MINT_AUTH_OFFSET + 4) as *const Address)
    }))
}

/// Read the freeze authority (returns None if COption tag is 0).
#[inline(always)]
pub fn mint_freeze_authority(data: &[u8]) -> Result<Option<&Address>, ProgramError> {
    if data.len() < MINT_LEN {
        return Err(ProgramError::InvalidAccountData);
    }
    let tag = u32::from_le_bytes([
        data[FREEZE_AUTH_OFFSET],
        data[FREEZE_AUTH_OFFSET + 1],
        data[FREEZE_AUTH_OFFSET + 2],
        data[FREEZE_AUTH_OFFSET + 3],
    ]);
    if tag == 0 {
        return Ok(None);
    }
    Ok(Some(unsafe {
        &*(data.as_ptr().add(FREEZE_AUTH_OFFSET + 4) as *const Address)
    }))
}

/// Check that mint_authority matches the expected pubkey.
#[inline(always)]
pub fn check_mint_authority(data: &[u8], expected: &Address) -> Result<(), ProgramError> {
    match mint_authority(data)? {
        Some(auth) if auth == expected => Ok(()),
        _ => Err(ProgramError::InvalidAccountData),
    }
}
