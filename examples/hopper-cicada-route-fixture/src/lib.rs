//! Compiled route fixture for Cicada's SVM suite.
//!
//! The test loads this ELF under the canonical SPL Token program id. It
//! implements the two token instructions Cicada uses (`TransferChecked` and
//! `SetAuthority`) plus deliberately hostile route commands. That makes the
//! test exercise real cross-program SBF frames without depending on a host
//! mock or a second token-program implementation.

#![cfg_attr(target_os = "solana", no_std)]

use hopper::prelude::*;

#[cfg(target_os = "solana")]
mod __hopper_sbf {
    hopper::no_allocator!();
    hopper::nostd_panic_handler!();
}

#[cfg(target_os = "solana")]
hopper::program_entrypoint!(process_instruction, 4);

const TOKEN_ACCOUNT_LEN: usize = 165;
const OWNER_OFFSET: usize = 32;
const AMOUNT_OFFSET: usize = 64;
const CLOSE_AUTHORITY_OPTION_OFFSET: usize = 129;

/// Fixture route command: honest source debit + destination credit.
pub const ROUTE_HONEST: u8 = 0xA0;
/// Fixture route command: economic movement plus a forbidden policy mutation.
pub const ROUTE_MUTATE_POLICY: u8 = 0xA1;
/// Fixture route command: mint-like destination credit with no source debit.
pub const ROUTE_SPOOF_OUTPUT: u8 = 0xA2;

pub fn process_instruction(
    _program_id: &Address,
    accounts: &[AccountView<'_>],
    data: &[u8],
) -> ProgramResult {
    let command = *data.first().ok_or(ProgramError::InvalidInstructionData)?;
    match command {
        // SPL Token TransferChecked.
        12 => transfer_checked(accounts, data),
        // SPL Token SetAuthority(AccountOwner, Some(pubkey)).
        6 => set_authority(accounts, data),
        ROUTE_HONEST | ROUTE_MUTATE_POLICY | ROUTE_SPOOF_OUTPUT => route(accounts, data, command),
        _ => Err(ProgramError::InvalidInstructionData),
    }
}

fn route(accounts: &[AccountView<'_>], data: &[u8], command: u8) -> ProgramResult {
    if accounts.len() < 3 || data.len() != 17 {
        return Err(ProgramError::InvalidInstructionData);
    }
    accounts[0].require_writable()?;
    accounts[1].require_writable()?;
    accounts[2].require_signer()?;
    let spend = u64::from_le_bytes(data[1..9].try_into().unwrap());
    let output = u64::from_le_bytes(data[9..17].try_into().unwrap());

    if command != ROUTE_SPOOF_OUTPUT {
        let current = token_amount(&accounts[0])?;
        write_token_amount(
            &accounts[0],
            current
                .checked_sub(spend)
                .ok_or(ProgramError::InsufficientFunds)?,
        )?;
    }
    let destination = token_amount(&accounts[1])?;
    write_token_amount(
        &accounts[1],
        destination
            .checked_add(output)
            .ok_or(ProgramError::ArithmeticOverflow)?,
    )?;

    if command == ROUTE_MUTATE_POLICY {
        let mut bytes = accounts[1].try_borrow_mut()?;
        if bytes.len() < TOKEN_ACCOUNT_LEN {
            return Err(ProgramError::InvalidAccountData);
        }
        bytes[CLOSE_AUTHORITY_OPTION_OFFSET] ^= 1;
    }
    Ok(())
}

fn transfer_checked(accounts: &[AccountView<'_>], data: &[u8]) -> ProgramResult {
    if accounts.len() < 4 || data.len() != 10 {
        return Err(ProgramError::InvalidInstructionData);
    }
    accounts[0].require_writable()?;
    accounts[2].require_writable()?;
    accounts[3].require_signer()?;
    let amount = u64::from_le_bytes(data[1..9].try_into().unwrap());
    let from = token_amount(&accounts[0])?;
    let to = token_amount(&accounts[2])?;
    write_token_amount(
        &accounts[0],
        from.checked_sub(amount)
            .ok_or(ProgramError::InsufficientFunds)?,
    )?;
    write_token_amount(
        &accounts[2],
        to.checked_add(amount)
            .ok_or(ProgramError::ArithmeticOverflow)?,
    )
}

fn set_authority(accounts: &[AccountView<'_>], data: &[u8]) -> ProgramResult {
    if accounts.len() < 2 || data.len() != 35 || data[1] != 2 || data[2] != 1 {
        return Err(ProgramError::InvalidInstructionData);
    }
    accounts[0].require_writable()?;
    accounts[1].require_signer()?;
    let mut bytes = accounts[0].try_borrow_mut()?;
    if bytes.len() < TOKEN_ACCOUNT_LEN {
        return Err(ProgramError::InvalidAccountData);
    }
    bytes[OWNER_OFFSET..OWNER_OFFSET + 32].copy_from_slice(&data[3..35]);
    Ok(())
}

fn token_amount(account: &AccountView<'_>) -> Result<u64> {
    let bytes = account.try_borrow()?;
    if bytes.len() < TOKEN_ACCOUNT_LEN {
        return Err(ProgramError::InvalidAccountData);
    }
    Ok(u64::from_le_bytes(
        bytes[AMOUNT_OFFSET..AMOUNT_OFFSET + 8].try_into().unwrap(),
    ))
}

fn write_token_amount(account: &AccountView<'_>, amount: u64) -> ProgramResult {
    let mut bytes = account.try_borrow_mut()?;
    if bytes.len() < TOKEN_ACCOUNT_LEN {
        return Err(ProgramError::InvalidAccountData);
    }
    bytes[AMOUNT_OFFSET..AMOUNT_OFFSET + 8].copy_from_slice(&amount.to_le_bytes());
    Ok(())
}
