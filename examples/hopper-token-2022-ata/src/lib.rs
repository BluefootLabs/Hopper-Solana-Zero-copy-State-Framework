//! # Hopper Token-2022 + ATA Example
//!
//! Demonstrates Hopper-authored usage of Hopper-owned companion crates:
//!
//! - `hopper_associated_token::CreateIdempotent`
//! - `hopper_token_2022::MintTo`
//! - whole-layout Hopper state via `load_mut()`

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

hopper_layout! {
    /// Minimal state tracking for a Token-2022 vault.
    pub struct TokenVault, disc = 31, version = 1 {
        authority:    TypedAddress<Authority> = 32,
        mint:         TypedAddress<Mint>      = 32,
        vault_ata:    TypedAddress<Token>     = 32,
        minted_total: WireU64                 = 8,
        bump:         u8                      = 1,
    }
}

hopper_error! {
    base = 6300;
    ZeroAmount,
    WrongTokenProgram,
    WrongSystemProgram,
}

#[cfg(target_os = "solana")]
program_entrypoint!(process_instruction);

fn process_instruction(
    program_id: &Address,
    accounts: &[AccountView],
    instruction_data: &[u8],
) -> ProgramResult {
    hopper::hopper_dispatch! {
        program_id, accounts, instruction_data;
        0 => process_init_vault,
        1 => process_prepare_ata_and_mint,
    }
}

fn process_init_vault(
    program_id: &Address,
    accounts: &[AccountView],
    _data: &[u8],
) -> ProgramResult {
    if accounts.len() < 4 {
        return Err(ProgramError::NotEnoughAccountKeys);
    }

    let payer = &accounts[0];
    let vault_state = &accounts[1];
    let authority = &accounts[2];
    let system_program = &accounts[3];

    require_payer(payer)?;
    authority.check_signer()?;
    if *system_program.address() != SYSTEM_PROGRAM_ID {
        return Err(WrongSystemProgram.into());
    }

    hopper_init!(payer, vault_state, system_program, program_id, TokenVault)?;

    let mut vault = TokenVault::load_mut(vault_state, program_id)?;
    let vault = vault.get_mut();
    vault.authority = TypedAddress::from_account(authority);
    vault.mint = TypedAddress::zeroed();
    vault.vault_ata = TypedAddress::zeroed();
    vault.minted_total = WireU64::new(0);
    vault.bump = 0;

    Ok(())
}

fn process_prepare_ata_and_mint(
    program_id: &Address,
    accounts: &[AccountView],
    data: &[u8],
) -> ProgramResult {
    if accounts.len() < 7 || data.len() < 8 {
        return Err(ProgramError::InvalidInstructionData);
    }

    let payer = &accounts[0];
    let authority = &accounts[1];
    let vault_state = &accounts[2];
    let vault_ata = &accounts[3];
    let mint = &accounts[4];
    let system_program = &accounts[5];
    let token_program_2022 = &accounts[6];

    require_payer(payer)?;
    authority.check_signer()?;
    vault_ata.check_writable()?;
    mint.check_writable()?;

    if *system_program.address() != SYSTEM_PROGRAM_ID {
        return Err(WrongSystemProgram.into());
    }
    if *token_program_2022.address() != TOKEN_2022_PROGRAM_ID {
        return Err(WrongTokenProgram.into());
    }

    let amount = u64::from_le_bytes([
        data[0], data[1], data[2], data[3], data[4], data[5], data[6], data[7],
    ]);
    hopper_require!(amount > 0, ZeroAmount);

    hopper::hopper_associated_token::CreateIdempotent {
        payer,
        associated_account: vault_ata,
        wallet: authority,
        mint,
        system_program,
        token_program: token_program_2022,
    }
    .invoke()?;

    hopper::hopper_token_2022::MintTo {
        mint,
        account: vault_ata,
        mint_authority: authority,
        amount,
    }
    .invoke()?;

    let mut vault = TokenVault::load_mut(vault_state, program_id)?;
    let vault = vault.get_mut();
    vault.authority.require_eq_account(authority)?;
    vault.mint = TypedAddress::from_account(mint);
    vault.vault_ata = TypedAddress::from_account(vault_ata);
    let next_total = vault
        .minted_total
        .get()
        .checked_add(amount)
        .ok_or(ProgramError::ArithmeticOverflow)?;
    vault.minted_total = WireU64::new(next_total);

    Ok(())
}
