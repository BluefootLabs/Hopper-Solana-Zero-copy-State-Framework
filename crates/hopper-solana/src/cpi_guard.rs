//! CPI safety guards.
//!
//! Detect at runtime whether the current instruction was invoked via CPI
//! and optionally reject it. Uses the Instructions sysvar to inspect the
//! call stack depth -- zero overhead when the sysvar account is already loaded.
//!
//! ## Instructions Sysvar Wire Format
//!
//! ```text
//! [u16 num_instructions]                         offset 0
//! [u16 offset_0] [u16 offset_1] ... [u16 offset_{n-1}]
//! [serialized instruction 0] ...
//! [u16 current_instruction_index]                last 2 bytes
//! ```

use hopper_runtime::{error::ProgramError, AccountView, Address, ProgramResult};

use crate::constants::{SYSVAR_INSTRUCTIONS_ID, TOKEN_PROGRAM_ID, TOKEN_2022_PROGRAM_ID};

/// Read the current instruction index from the Instructions sysvar.
///
/// The current index is stored at the **last 2 bytes** of the sysvar data.
/// Returns 0 for the first instruction in the transaction.
#[inline(always)]
pub fn current_instruction_index(sysvar_data: &[u8]) -> Result<u16, ProgramError> {
    let len = sysvar_data.len();
    if len < 2 {
        return Err(ProgramError::InvalidAccountData);
    }
    Ok(u16::from_le_bytes([
        sysvar_data[len - 2],
        sysvar_data[len - 1],
    ]))
}

/// Read the number of instructions from the Instructions sysvar.
///
/// The count is stored at the **first 2 bytes** (offset 0) of the sysvar data.
#[inline(always)]
pub fn instruction_count(sysvar_data: &[u8]) -> Result<u16, ProgramError> {
    if sysvar_data.len() < 2 {
        return Err(ProgramError::InvalidAccountData);
    }
    Ok(u16::from_le_bytes([sysvar_data[0], sysvar_data[1]]))
}

/// Assert that the current instruction is NOT invoked via CPI.
///
/// Reads the Instructions sysvar to verify the current instruction's
/// program_id matches ours. If we were called via CPI, the sysvar's
/// "current instruction" would be the outer program, not us.
///
/// Programs should pass the Instructions sysvar AccountView.
///
/// # CU Cost
/// Minimal -- sysvar data reads + one 32-byte comparison. No syscall.
#[inline(always)]
pub fn assert_no_cpi(
    instructions_sysvar: &AccountView,
    our_program_id: &Address,
) -> ProgramResult {
    // Verify the account is actually the Instructions sysvar
    if *instructions_sysvar.address() != SYSVAR_INSTRUCTIONS_ID {
        return Err(ProgramError::InvalidArgument);
    }

    let data = instructions_sysvar.try_borrow()?;
    if data.len() < 4 {
        return Err(ProgramError::InvalidAccountData);
    }

    // Read current instruction index from last 2 bytes
    let current_idx = current_instruction_index(&data)?;
    let num_instructions = instruction_count(&data)?;

    // Sanity: current_idx must be within bounds
    if current_idx >= num_instructions {
        return Err(ProgramError::InvalidAccountData);
    }

    // Read the program_id of the current instruction from the offset table.
    // Offset table starts at byte 2, each entry is u16 pointing to the
    // serialized instruction data.
    let offset_entry = 2 + (current_idx as usize) * 2;
    if offset_entry + 2 > data.len() {
        return Err(ProgramError::InvalidAccountData);
    }
    let ix_offset = u16::from_le_bytes([
        data[offset_entry],
        data[offset_entry + 1],
    ]) as usize;

    // Per-instruction format: [u16 num_accounts][33 bytes * N accounts][32 bytes program_id]...
    if ix_offset + 2 > data.len() {
        return Err(ProgramError::InvalidAccountData);
    }
    let num_accounts = u16::from_le_bytes([
        data[ix_offset],
        data[ix_offset + 1],
    ]) as usize;
    let program_id_offset = ix_offset + 2 + num_accounts * 33;
    if program_id_offset + 32 > data.len() {
        return Err(ProgramError::InvalidAccountData);
    }

    // If the current instruction's program_id != our_program_id,
    // we are being invoked via CPI.
    if data[program_id_offset..program_id_offset + 32] != *our_program_id.as_array() {
        return Err(ProgramError::InvalidAccountData);
    }

    Ok(())
}

/// Check that an account is owned by the SPL Token program.
#[inline(always)]
pub fn check_token_program_owner(account: &AccountView) -> ProgramResult {
    if !account.owned_by(&TOKEN_PROGRAM_ID) {
        return Err(ProgramError::IncorrectProgramId);
    }
    Ok(())
}

/// Check that an account is owned by the Token-2022 program.
#[inline(always)]
pub fn check_token_2022_program_owner(account: &AccountView) -> ProgramResult {
    if !account.owned_by(&TOKEN_2022_PROGRAM_ID) {
        return Err(ProgramError::IncorrectProgramId);
    }
    Ok(())
}

/// Check that an account is owned by either SPL Token or Token-2022.
#[inline(always)]
pub fn check_any_token_program_owner(account: &AccountView) -> ProgramResult {
    if check_token_program_owner(account).is_ok()
        || check_token_2022_program_owner(account).is_ok()
    {
        return Ok(());
    }
    Err(ProgramError::IncorrectProgramId)
}
