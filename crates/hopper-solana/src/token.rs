//! Zero-copy SPL Token account readers.
//!
//! Read token account fields directly from raw bytes without deserialization.
//! Each function reads exactly the bytes needed at the correct offset.

use hopper_runtime::error::ProgramError;
use hopper_runtime::Address;

/// SPL Token Account total size.
pub const TOKEN_ACCOUNT_LEN: usize = 165;

// Field offsets within an SPL Token Account (v1 layout)
const MINT_OFFSET: usize = 0;
const OWNER_OFFSET: usize = 32;
const AMOUNT_OFFSET: usize = 64;
#[allow(dead_code)]
const DELEGATE_OFFSET: usize = 72;
const STATE_OFFSET: usize = 108;
#[allow(dead_code)]
const DELEGATED_AMOUNT_OFFSET: usize = 121;
#[allow(dead_code)]
const CLOSE_AUTH_OFFSET: usize = 129;

/// Read the mint pubkey from a token account.
#[inline(always)]
pub fn token_account_mint(data: &[u8]) -> Result<&Address, ProgramError> {
    if data.len() < TOKEN_ACCOUNT_LEN {
        return Err(ProgramError::InvalidAccountData);
    }
    // SAFETY: Checked length. Address is [u8; 32], alignment 1.
    Ok(unsafe { &*(data.as_ptr().add(MINT_OFFSET) as *const Address) })
}

/// Read the owner pubkey from a token account.
#[inline(always)]
pub fn token_account_owner(data: &[u8]) -> Result<&Address, ProgramError> {
    if data.len() < TOKEN_ACCOUNT_LEN {
        return Err(ProgramError::InvalidAccountData);
    }
    Ok(unsafe { &*(data.as_ptr().add(OWNER_OFFSET) as *const Address) })
}

/// Read the amount from a token account.
#[inline(always)]
pub fn token_account_amount(data: &[u8]) -> Result<u64, ProgramError> {
    if data.len() < TOKEN_ACCOUNT_LEN {
        return Err(ProgramError::InvalidAccountData);
    }
    Ok(u64::from_le_bytes([
        data[AMOUNT_OFFSET],
        data[AMOUNT_OFFSET + 1],
        data[AMOUNT_OFFSET + 2],
        data[AMOUNT_OFFSET + 3],
        data[AMOUNT_OFFSET + 4],
        data[AMOUNT_OFFSET + 5],
        data[AMOUNT_OFFSET + 6],
        data[AMOUNT_OFFSET + 7],
    ]))
}

/// Read the state byte (0=uninitialized, 1=initialized, 2=frozen).
#[inline(always)]
pub fn token_account_state(data: &[u8]) -> Result<u8, ProgramError> {
    if data.len() < TOKEN_ACCOUNT_LEN {
        return Err(ProgramError::InvalidAccountData);
    }
    Ok(data[STATE_OFFSET])
}

/// Check the token account is initialized (state != 0).
#[inline(always)]
pub fn check_token_initialized(data: &[u8]) -> Result<(), ProgramError> {
    if token_account_state(data)? == 0 {
        return Err(ProgramError::UninitializedAccount);
    }
    Ok(())
}

/// Check the token account owner matches expected.
#[inline(always)]
pub fn check_token_owner(data: &[u8], expected: &Address) -> Result<(), ProgramError> {
    let owner = token_account_owner(data)?;
    if owner != expected {
        return Err(ProgramError::InvalidAccountData);
    }
    Ok(())
}

/// Check the token account mint matches expected.
#[inline(always)]
pub fn check_token_mint(data: &[u8], expected: &Address) -> Result<(), ProgramError> {
    let mint = token_account_mint(data)?;
    if mint != expected {
        return Err(ProgramError::InvalidAccountData);
    }
    Ok(())
}

/// Check the token account is not frozen.
#[inline(always)]
pub fn check_not_frozen(data: &[u8]) -> Result<(), ProgramError> {
    if token_account_state(data)? == 2 {
        return Err(ProgramError::InvalidAccountData);
    }
    Ok(())
}

/// Check the token balance >= min_amount.
#[inline(always)]
pub fn check_token_balance_gte(data: &[u8], min_amount: u64) -> Result<(), ProgramError> {
    let amount = token_account_amount(data)?;
    if amount < min_amount {
        return Err(ProgramError::InsufficientFunds);
    }
    Ok(())
}
