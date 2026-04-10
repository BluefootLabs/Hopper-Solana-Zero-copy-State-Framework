//! Transaction introspection via Sysvar Instructions.
//!
//! Parse the Sysvar Instructions account data to inspect transaction
//! composition: program IDs, instruction data, account keys, CPI detection.
//!
//! # Layout reference
//!
//! ```text
//! [num_instructions: u16 LE]
//! [offset_0: u16 LE] [offset_1: u16 LE] ... [offset_N-1: u16 LE]
//!
//! At each offset:
//!   [num_accounts: u16 LE]
//!   [accounts: 33 bytes each (flags:u8 + pubkey:32)]
//!   [program_id: 32 bytes]
//!   [data_len: u16 LE]
//!   [data: data_len bytes]
//!
//! At end of sysvar:
//!   [current_instruction_index: u16 LE]
//! ```

use hopper_runtime::{error::ProgramError, Address, ProgramResult};

/// Read the number of instructions in the transaction.
#[inline(always)]
pub fn instruction_count(data: &[u8]) -> Result<u16, ProgramError> {
    if data.len() < 2 {
        return Err(ProgramError::AccountDataTooSmall);
    }
    Ok(u16::from_le_bytes([data[0], data[1]]))
}

/// Read the current instruction index (last 2 bytes of sysvar data).
#[inline(always)]
pub fn current_index(data: &[u8]) -> Result<u16, ProgramError> {
    if data.len() < 2 {
        return Err(ProgramError::AccountDataTooSmall);
    }
    let offset = data.len() - 2;
    Ok(u16::from_le_bytes([data[offset], data[offset + 1]]))
}

/// Read the program_id of the instruction at `index`.
#[inline(always)]
pub fn program_id_at(data: &[u8], index: u16) -> Result<Address, ProgramError> {
    let (offset, num_accounts) = instruction_meta(data, index)?;
    let num_accounts = num_accounts as usize;
    let program_id_offset = offset + 2 + num_accounts * 33;
    if program_id_offset + 32 > data.len() {
        return Err(ProgramError::AccountDataTooSmall);
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&data[program_id_offset..program_id_offset + 32]);
    Ok(Address::new_from_array(out))
}

/// Read the instruction data range (offset, length) for instruction at `index`.
#[inline(always)]
pub fn instruction_data_range(
    data: &[u8],
    index: u16,
) -> Result<(usize, usize), ProgramError> {
    let (offset, num_accounts) = instruction_meta(data, index)?;
    let num_accounts = num_accounts as usize;
    let after_program_id = offset + 2 + num_accounts * 33 + 32;
    if after_program_id + 2 > data.len() {
        return Err(ProgramError::AccountDataTooSmall);
    }
    let data_len =
        u16::from_le_bytes([data[after_program_id], data[after_program_id + 1]]) as usize;
    let data_start = after_program_id + 2;
    if data_start + data_len > data.len() {
        return Err(ProgramError::AccountDataTooSmall);
    }
    Ok((data_start, data_len))
}

/// Read the account pubkey at position `account_index` within instruction `index`.
#[inline(always)]
pub fn instruction_account_key(
    data: &[u8],
    index: u16,
    account_index: u16,
) -> Result<Address, ProgramError> {
    let (offset, num_accounts) = instruction_meta(data, index)?;
    if account_index >= num_accounts {
        return Err(ProgramError::InvalidArgument);
    }
    let key_offset = offset + 2 + (account_index as usize) * 33 + 1;
    if key_offset + 32 > data.len() {
        return Err(ProgramError::AccountDataTooSmall);
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&data[key_offset..key_offset + 32]);
    Ok(Address::new_from_array(out))
}

/// Determine the program that invoked the current instruction.
///
/// Returns `Some(address)` if the current instruction's program_id differs
/// from `our_program`, indicating CPI. Returns `None` if top-level.
#[inline(always)]
pub fn caller_program(
    data: &[u8],
    our_program: &Address,
) -> Result<Option<Address>, ProgramError> {
    let idx = current_index(data)?;
    let pid = program_id_at(data, idx)?;
    if pid == *our_program {
        Ok(None)
    } else {
        Ok(Some(pid))
    }
}

/// Require that the current instruction is top-level (not via CPI).
#[inline(always)]
pub fn require_top_level(data: &[u8], our_program: &Address) -> ProgramResult {
    let idx = current_index(data)?;
    let pid = program_id_at(data, idx)?;
    if pid != *our_program {
        return Err(ProgramError::InvalidArgument);
    }
    Ok(())
}

/// Require that the CPI caller is a specific trusted program.
#[inline(always)]
pub fn require_cpi_from(
    data: &[u8],
    expected_caller: &Address,
) -> ProgramResult {
    let idx = current_index(data)?;
    let pid = program_id_at(data, idx)?;
    if pid != *expected_caller {
        return Err(ProgramError::InvalidArgument);
    }
    Ok(())
}

/// Count how many instructions in the transaction invoke `program_id`.
#[inline(always)]
pub fn count_program_invocations(
    data: &[u8],
    program_id: &Address,
) -> Result<u16, ProgramError> {
    let count = instruction_count(data)?;
    let mut n = 0u16;
    let mut i = 0u16;
    while i < count {
        if let Ok(pid) = program_id_at(data, i) {
            if pid == *program_id {
                n += 1;
            }
        }
        i += 1;
    }
    Ok(n)
}

/// Detect a flash-loan bracket: instructions from `lender_program` both
/// before and after `current_idx`.
#[inline(always)]
pub fn detect_flash_loan_bracket(
    data: &[u8],
    current_idx: u16,
    lender_program: &Address,
) -> Result<bool, ProgramError> {
    let count = instruction_count(data)?;
    let mut before = false;
    let mut after = false;
    let mut i = 0u16;
    while i < count {
        if i != current_idx {
            if let Ok(pid) = program_id_at(data, i) {
                if pid == *lender_program {
                    if i < current_idx {
                        before = true;
                    } else {
                        after = true;
                    }
                    if before && after {
                        return Ok(true);
                    }
                }
            }
        }
        i += 1;
    }
    Ok(before && after)
}

/// Fail if any instruction other than `current_idx` invokes `program_id`.
#[inline(always)]
pub fn check_no_other_invocation(
    data: &[u8],
    current_idx: u16,
    program_id: &Address,
) -> ProgramResult {
    let count = instruction_count(data)?;
    let mut i = 0u16;
    while i < count {
        if i != current_idx {
            if let Ok(pid) = program_id_at(data, i) {
                if pid == *program_id {
                    return Err(ProgramError::InvalidArgument);
                }
            }
        }
        i += 1;
    }
    Ok(())
}

/// Fail if any instruction after `current_idx` invokes `program_id`.
#[inline(always)]
pub fn check_no_subsequent_invocation(
    data: &[u8],
    current_idx: u16,
    program_id: &Address,
) -> ProgramResult {
    let count = instruction_count(data)?;
    let mut i = current_idx.saturating_add(1);
    while i < count {
        if let Ok(pid) = program_id_at(data, i) {
            if pid == *program_id {
                return Err(ProgramError::InvalidArgument);
            }
        }
        i += 1;
    }
    Ok(())
}

// ── Internal ─────────────────────────────────────────────────────────────────

/// Read instruction offset and num_accounts for instruction at `index`.
#[inline(always)]
fn instruction_meta(data: &[u8], index: u16) -> Result<(usize, u16), ProgramError> {
    if data.len() < 2 {
        return Err(ProgramError::AccountDataTooSmall);
    }
    let num_instructions = u16::from_le_bytes([data[0], data[1]]);
    if index >= num_instructions {
        return Err(ProgramError::InvalidAccountData);
    }
    let offset_pos = 2 + (index as usize) * 2;
    if offset_pos + 2 > data.len() {
        return Err(ProgramError::AccountDataTooSmall);
    }
    let instr_offset =
        u16::from_le_bytes([data[offset_pos], data[offset_pos + 1]]) as usize;
    if instr_offset + 2 > data.len() {
        return Err(ProgramError::AccountDataTooSmall);
    }
    let num_accounts =
        u16::from_le_bytes([data[instr_offset], data[instr_offset + 1]]);
    Ok((instr_offset, num_accounts))
}
