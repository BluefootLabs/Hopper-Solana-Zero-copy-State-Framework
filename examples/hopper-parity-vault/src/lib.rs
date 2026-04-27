//! # Hopper Parity Vault
//!
//! Minimal vault used for fair cross-framework comparison against Quasar's
//! `vault` example and Quasar's `pinocchio-vault` example.

#![cfg_attr(target_os = "solana", no_std)]
#![allow(dead_code)]

use hopper::prelude::*;

#[cfg(target_os = "solana")]
mod __hopper_sbf {
    use super::*;

    #[cfg(not(feature = "solana-program-backend"))]
    no_allocator!();

    #[cfg(not(feature = "solana-program-backend"))]
    nostd_panic_handler!();
}

#[cfg(target_os = "solana")]
fast_entrypoint!(process_instruction, 3);

fn process_instruction(
    program_id: &Address,
    accounts: &[AccountView],
    instruction_data: &[u8],
) -> ProgramResult {
    let (discriminator, data) = instruction_data
        .split_first()
        .ok_or(ProgramError::InvalidInstructionData)?;

    match *discriminator {
        0 => process_deposit(program_id, accounts, data),
        1 => process_withdraw(program_id, accounts, data),
        2 => process_authorize(program_id, accounts),
        3 => process_counter_access(program_id, accounts),
        _ => Err(ProgramError::InvalidInstructionData),
    }
}

#[inline(always)]
fn parse_amount(data: &[u8]) -> Result<u64, ProgramError> {
    if data.len() < 8 {
        return Err(ProgramError::InvalidInstructionData);
    }

    Ok(u64::from_le_bytes([
        data[0], data[1], data[2], data[3], data[4], data[5], data[6], data[7],
    ]))
}

#[inline(always)]
fn verify_vault_pda(
    user: &AccountView,
    vault: &AccountView,
    program_id: &Address,
) -> ProgramResult {
    find_and_verify_pda(vault, &[b"vault", user.address().as_ref()], program_id)?;
    Ok(())
}

#[inline(always)]
fn validate_authority(user: &AccountView) -> ProgramResult {
    if !user.is_signer() {
        return Err(ProgramError::MissingRequiredSignature);
    }
    if !user.is_writable() {
        return Err(ProgramError::Immutable);
    }
    Ok(())
}

#[inline(always)]
fn validate_writable(account: &AccountView) -> ProgramResult {
    if !account.is_writable() {
        return Err(ProgramError::Immutable);
    }
    Ok(())
}

#[inline(always)]
fn transfer_unchecked(from: &AccountView, to: &AccountView, lamports: u64) -> ProgramResult {
    let mut data = [0u8; 12];
    data[..4].copy_from_slice(&2u32.to_le_bytes());
    data[4..12].copy_from_slice(&lamports.to_le_bytes());

    let accounts = [
        InstructionAccount::writable_signer(from.address()),
        InstructionAccount::writable(to.address()),
    ];
    let cpi_accounts = [
        hopper::hopper_runtime::CpiAccount::from(from),
        hopper::hopper_runtime::CpiAccount::from(to),
    ];
    let instruction = InstructionView {
        program_id: &SYSTEM_PROGRAM_ID,
        data: &data,
        accounts: &accounts,
    };

    unsafe { hopper::hopper_runtime::cpi::invoke_unchecked(&instruction, &cpi_accounts) }
}

fn process_deposit(program_id: &Address, accounts: &[AccountView], data: &[u8]) -> ProgramResult {
    let [user, vault, system_program, ..] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };

    validate_authority(user)?;
    process_deposit_accounts(program_id, user, vault, system_program, data)
}

fn process_deposit_accounts(
    program_id: &Address,
    user: &AccountView,
    vault: &AccountView,
    system_program: &AccountView,
    data: &[u8],
) -> ProgramResult {
    let amount = parse_amount(data)?;

    validate_writable(vault)?;
    if !hopper::hopper_runtime::address::address_eq(system_program.address(), &SYSTEM_PROGRAM_ID) {
        return Err(ProgramError::IncorrectProgramId);
    }
    verify_vault_pda(user, vault, program_id)?;

    transfer_unchecked(user, vault, amount)
}

fn process_withdraw(program_id: &Address, accounts: &[AccountView], data: &[u8]) -> ProgramResult {
    let [user, vault, ..] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };

    validate_authority(user)?;
    process_withdraw_accounts(program_id, user, vault, data)
}

fn process_withdraw_accounts(
    program_id: &Address,
    user: &AccountView,
    vault: &AccountView,
    data: &[u8],
) -> ProgramResult {
    let amount = parse_amount(data)?;

    vault.check_owned_by(program_id)?;
    validate_writable(vault)?;
    verify_vault_pda(user, vault, program_id)?;

    let vault_lamports = vault.lamports();
    if amount > vault_lamports {
        return Err(ProgramError::InsufficientFunds);
    }

    vault.set_lamports(vault_lamports - amount);
    user.set_lamports(
        user.lamports()
            .checked_add(amount)
            .ok_or(ProgramError::ArithmeticOverflow)?,
    );

    Ok(())
}

fn process_authorize(program_id: &Address, accounts: &[AccountView]) -> ProgramResult {
    let [user, vault, ..] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };

    validate_authority(user)?;
    process_authorize_accounts(program_id, user, vault)
}

fn process_authorize_accounts(
    program_id: &Address,
    user: &AccountView,
    vault: &AccountView,
) -> ProgramResult {
    validate_writable(vault)?;
    verify_vault_pda(user, vault, program_id)
}

fn process_counter_access(program_id: &Address, accounts: &[AccountView]) -> ProgramResult {
    let [user, vault, ..] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };

    validate_authority(user)?;
    verify_vault_pda(user, vault, program_id)?;

    let mut borrows = SegmentBorrowRegistry::new();
    {
        let authority = vault.segment_ref::<Address>(&mut borrows, 0, 32)?;
        if !hopper::hopper_runtime::address::address_eq(&*authority, user.address()) {
            return Err(ProgramError::InvalidAccountData);
        }
    }

    let mut counter = vault.segment_mut::<WireU64>(&mut borrows, 32, 8)?;
    let next = (*counter)
        .get()
        .checked_add(1)
        .ok_or(ProgramError::ArithmeticOverflow)?;
    *counter = WireU64::new(next);
    Ok(())
}
