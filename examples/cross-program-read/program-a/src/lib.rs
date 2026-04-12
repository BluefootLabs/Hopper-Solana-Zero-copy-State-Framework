//! # Program A -- Vault Owner
//!
//! Defines a `Vault` account with `hopper_layout!`. This is the canonical
//! layout definition. Program B reads it using `hopper_interface!` without
//! importing this crate.

#![cfg_attr(target_os = "solana", no_std)]
#![allow(dead_code, unused_variables)]

use hopper::prelude::*;

#[cfg(target_os = "solana")]
mod __hopper_sbf {
    use super::*;

    #[cfg(not(feature = "solana-program-backend"))]
    no_allocator!();

    #[cfg(not(feature = "solana-program-backend"))]
    nostd_panic_handler!();
}

// --- Layout ---------------------------------------------------------

hopper_layout! {
    /// A simple SOL vault owned by Program A.
    pub struct Vault, disc = 1, version = 1 {
        authority: TypedAddress<Authority> = 32,
        balance:   WireU64                = 8,
        bump:      u8                     = 1,
    }
}

// --- Errors ---------------------------------------------------------

hopper_error! {
    base = 6000;
    Unauthorized,
    InsufficientBalance,
}

// --- Entrypoint -----------------------------------------------------

#[cfg(target_os = "solana")]
program_entrypoint!(process_instruction);

fn process_instruction(
    program_id: &Address,
    accounts: &[AccountView],
    instruction_data: &[u8],
) -> ProgramResult {
    hopper::hopper_dispatch! {
        program_id, accounts, instruction_data;
        0 => process_init,
        1 => process_deposit,
    }
}

// --- Init -----------------------------------------------------------

fn process_init(
    program_id: &Address,
    accounts: &[AccountView],
    _data: &[u8],
) -> ProgramResult {
    if accounts.len() < 3 {
        return Err(ProgramError::NotEnoughAccountKeys);
    }
    let payer = &accounts[0];
    let vault_account = &accounts[1];
    let system_program = &accounts[2];

    require_payer(payer)?;
    check_writable(vault_account)?;

    hopper_init!(payer, vault_account, system_program, program_id, Vault)?;

    let mut data = vault_account.try_borrow_mut()?;
    let vault = Vault::overlay_mut(&mut data)?;
    vault.authority = TypedAddress::from_account(payer);
    vault.balance = WireU64::new(0);

    Ok(())
}

// --- Deposit --------------------------------------------------------

fn process_deposit(
    program_id: &Address,
    accounts: &[AccountView],
    data: &[u8],
) -> ProgramResult {
    if accounts.len() < 2 {
        return Err(ProgramError::NotEnoughAccountKeys);
    }
    let authority = &accounts[0];
    let vault_account = &accounts[1];

    check_signer(authority)?;
    let mut verified = Vault::load_mut(vault_account, program_id)?;
    let vault = verified.get_mut();

    vault.authority.require_eq_account(authority)?;

    if data.len() < 8 {
        return Err(ProgramError::InvalidInstructionData);
    }
    let amount = u64::from_le_bytes([
        data[0], data[1], data[2], data[3],
        data[4], data[5], data[6], data[7],
    ]);
    let new_balance = vault
        .balance
        .get()
        .checked_add(amount)
        .ok_or(ProgramError::ArithmeticOverflow)?;
    vault.balance = WireU64::new(new_balance);

    Ok(())
}
