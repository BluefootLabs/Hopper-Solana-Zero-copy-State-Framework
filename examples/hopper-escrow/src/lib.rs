//! # Hopper Escrow Example
//!
//! Demonstrates a token escrow using the Hopper framework.
//!
//! Instructions:
//! - `0` = Make (create escrow offer)
//! - `1` = Take (accept escrow)
//! - `2` = Cancel (reclaim escrowed tokens)

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
    /// An escrow account holding an offer to swap tokens.
    pub struct Escrow, disc = 2, version = 1 {
        maker:          TypedAddress<Authority> = 32,
        maker_ta:       TypedAddress<Token>     = 32,
        mint_a:         TypedAddress<Mint>       = 32,
        mint_b:         TypedAddress<Mint>       = 32,
        amount_offered: WireU64                  = 8,
        amount_wanted:  WireU64                  = 8,
        bump:           u8                       = 1,
    }
}

// --- Errors ---------------------------------------------------------

hopper_error! {
    base = 6100;
    MintMismatch,
    AmountMismatch,
    EscrowUnauthorized,
    EscrowAlreadyFilled,
    ZeroEscrowAmount,
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
        0 => process_make,
        1 => process_take,
        2 => process_cancel,
    }
}

// --- Make -----------------------------------------------------------

fn process_make(program_id: &Address, accounts: &[AccountView], data: &[u8]) -> ProgramResult {
    if accounts.len() < 3 {
        return Err(ProgramError::NotEnoughAccountKeys);
    }
    let maker = &accounts[0];
    let escrow_account = &accounts[1];
    let system_program = &accounts[2];

    maker.check_signer()?.check_writable()?;
    escrow_account.check_writable()?;

    // Parse: mint_a (32) + mint_b (32) + amount_offered (8) + amount_wanted (8)
    if data.len() < 80 {
        return Err(ProgramError::InvalidInstructionData);
    }
    let mint_a = &data[0..32];
    let mint_b = &data[32..64];
    let amount_offered = u64::from_le_bytes([
        data[64], data[65], data[66], data[67], data[68], data[69], data[70], data[71],
    ]);
    let amount_wanted = u64::from_le_bytes([
        data[72], data[73], data[74], data[75], data[76], data[77], data[78], data[79],
    ]);

    hopper_require!(amount_offered > 0, ZeroEscrowAmount);
    hopper_require!(amount_wanted > 0, ZeroEscrowAmount);

    // Create escrow account
    hopper_init!(maker, escrow_account, system_program, program_id, Escrow)?;

    // Write state
    let mut escrow = Escrow::load_mut(escrow_account, program_id)?;
    let escrow = escrow.get_mut();
    escrow.maker = TypedAddress::from_account(maker);
    escrow.maker_ta = TypedAddress::zeroed(); // Simplified: would be maker's token account
    escrow.mint_a = TypedAddress::from_slice(
        mint_a
            .try_into()
            .map_err(|_| ProgramError::InvalidInstructionData)?,
    );
    escrow.mint_b = TypedAddress::from_slice(
        mint_b
            .try_into()
            .map_err(|_| ProgramError::InvalidInstructionData)?,
    );
    escrow.amount_offered = WireU64::new(amount_offered);
    escrow.amount_wanted = WireU64::new(amount_wanted);

    Ok(())
}

// --- Take -----------------------------------------------------------

fn process_take(program_id: &Address, accounts: &[AccountView], _data: &[u8]) -> ProgramResult {
    if accounts.len() < 3 {
        return Err(ProgramError::NotEnoughAccountKeys);
    }
    let taker = &accounts[0];
    let escrow_account = &accounts[1];
    let maker_account = &accounts[2];

    taker.check_signer()?;
    escrow_account.check_writable()?;

    // Load and validate escrow
    let escrow = Escrow::load(escrow_account, program_id)?;
    let e = escrow.get();

    // Verify not already filled
    if e.amount_offered.get() == 0 {
        return Err(EscrowAlreadyFilled.into());
    }

    // In a real program, this would:
    // 1. Transfer mint_a tokens from escrow vault to taker
    // 2. Transfer mint_b tokens from taker to maker
    // 3. Close escrow account

    // Close escrow, return rent to maker
    hopper_close!(escrow_account, maker_account)?;

    Ok(())
}

// --- Cancel ---------------------------------------------------------

fn process_cancel(program_id: &Address, accounts: &[AccountView], _data: &[u8]) -> ProgramResult {
    if accounts.len() < 2 {
        return Err(ProgramError::NotEnoughAccountKeys);
    }
    let maker = &accounts[0];
    let escrow_account = &accounts[1];

    maker.check_signer()?;
    escrow_account.check_writable()?;

    // Load and verify maker is the authority
    let escrow = Escrow::load(escrow_account, program_id)?;
    let e = escrow.get();
    e.maker.require_eq_account(maker)?;

    // Close escrow, return rent to maker
    hopper_close!(escrow_account, maker)?;

    Ok(())
}
