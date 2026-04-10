//! Instruction introspection -- stack height and sibling instruction access.
//!
//! These syscalls are critical for security patterns that no framework wraps:
//!
//! - **CPI guard**: Detect if the current instruction is running inside a CPI
//!   call (stack height > 1). Prevents unauthorized composition -- e.g., a
//!   governance instruction that must be top-level only.
//!
//! - **Ed25519 signature verification**: Check that a previous instruction in
//!   the same transaction was to the Ed25519 precompile with specific data.
//!   This is how on-chain programs verify off-chain signatures without heavy
//!   crypto libraries.
//!
//! - **Secp256k1 recovery**: Same pattern for Ethereum-compatible signatures.
//!
//! No existing framework (pinocchio, Anchor, Steel, Quasar) wraps these
//! syscalls with ergonomic APIs. Programs that need them write raw unsafe
//! glue every time.

use crate::address::Address;
use crate::error::ProgramError;

/// Get the current instruction stack height.
///
/// Returns 1 for top-level instructions invoked by the runtime.
/// Returns 2+ for instructions running inside a CPI call.
///
/// Use this to implement CPI guards that prevent unauthorized composition.
#[inline(always)]
pub fn get_stack_height() -> u64 {
    #[cfg(target_os = "solana")]
    {
        unsafe { crate::syscalls::sol_get_stack_height() }
    }
    #[cfg(not(target_os = "solana"))]
    {
        1 // Off-chain: simulate top-level.
    }
}

/// Returns true if the current instruction is at the top level
/// (not running inside a CPI).
#[inline(always)]
pub fn is_top_level() -> bool {
    get_stack_height() <= 1
}

/// Returns true if the current instruction is running inside a CPI.
#[inline(always)]
pub fn is_cpi() -> bool {
    get_stack_height() > 1
}

/// Require that the current instruction is NOT a CPI call.
///
/// Programs that should never be composed via CPI (governance, admin
/// instructions, emergency controls) should call this at the top of
/// their handler. Returns `Err` if the instruction is inside a CPI.
#[inline(always)]
pub fn require_top_level() -> Result<(), ProgramError> {
    if is_top_level() {
        Ok(())
    } else {
        Err(ProgramError::InvalidArgument)
    }
}

/// Require that the current instruction IS inside a CPI.
///
/// Some instructions are designed to be called only via CPI (callback
/// patterns, module-internal helpers). This enforces that contract.
#[inline(always)]
pub fn require_cpi() -> Result<(), ProgramError> {
    if is_cpi() {
        Ok(())
    } else {
        Err(ProgramError::InvalidArgument)
    }
}

// ---- Processed sibling instructions ----------------------------------

/// Metadata about a previously processed sibling instruction.
#[derive(Clone, Debug)]
pub struct ProcessedInstruction {
    /// Program ID that executed the instruction.
    pub program_id: Address,
    /// Instruction data.
    pub data: [u8; 1232],
    /// Actual length of instruction data.
    pub data_len: usize,
    /// Number of accounts involved.
    pub accounts_len: usize,
}

/// Retrieve a previously processed sibling instruction from the current
/// transaction.
///
/// `index` is 0-based: 0 = most recently processed instruction before
/// the current one, 1 = the one before that, etc.
///
/// Returns `None` if no instruction exists at that index.
///
/// # Use case: Ed25519 signature verification
///
/// To verify an Ed25519 signature on-chain:
/// 1. The transaction includes an instruction to the Ed25519 precompile
///    with the message, signature, and public key.
/// 2. Your program calls `get_processed_instruction(0)` to read that
///    instruction.
/// 3. Verify the program_id is the Ed25519 precompile address.
/// 4. Parse the instruction data to extract the verified message.
#[inline]
pub fn get_processed_instruction(index: u64) -> Option<ProcessedInstruction> {
    #[cfg(target_os = "solana")]
    {
        let mut meta = ProcessedInstructionMeta { data_len: 0, accounts_len: 0 };
        let mut program_id = Address::default();
        let mut data = [0u8; 1232];
        // SolAccountMeta is 34 bytes each (32-byte pubkey + 2 bools).
        // Max accounts per instruction is ~64, so 64 * 34 = 2176 bytes.
        let mut accounts_buf = [0u8; 2176];

        // The syscall populates meta first to indicate required buffer sizes,
        // then fills the buffers.
        meta.data_len = data.len() as u64;
        meta.accounts_len = (accounts_buf.len() / 34) as u64;

        let rc = unsafe {
            crate::syscalls::sol_get_processed_sibling_instruction(
                index,
                &mut meta as *mut ProcessedInstructionMeta as *mut u8,
                program_id.0.as_mut_ptr(),
                data.as_mut_ptr(),
                accounts_buf.as_mut_ptr(),
            )
        };

        if rc != 0 {
            return None;
        }

        Some(ProcessedInstruction {
            program_id,
            data,
            data_len: meta.data_len as usize,
            accounts_len: meta.accounts_len as usize,
        })
    }
    #[cfg(not(target_os = "solana"))]
    {
        let _ = index;
        None
    }
}

/// Well-known precompile address for Ed25519 signature verification.
pub const ED25519_PROGRAM_ID: Address =
    crate::address!("Ed25519SigVerify111111111111111111111111111");

/// Well-known precompile address for Secp256k1 signature recovery.
pub const SECP256K1_PROGRAM_ID: Address =
    crate::address!("KeccakSecp256k11111111111111111111111111111");

/// Check that a previous sibling instruction was to the Ed25519 precompile.
///
/// Returns the instruction data from the Ed25519 precompile instruction.
/// The caller should parse this data to extract the verified message,
/// public key, and signature.
///
/// `sibling_index` is 0 for the most recent sibling, 1 for the one before, etc.
#[inline]
pub fn require_ed25519_instruction(
    sibling_index: u64,
) -> Result<ProcessedInstruction, ProgramError> {
    let ix = get_processed_instruction(sibling_index)
        .ok_or(ProgramError::InvalidArgument)?;

    if !crate::address::address_eq(&ix.program_id, &ED25519_PROGRAM_ID) {
        return Err(ProgramError::IncorrectProgramId);
    }

    Ok(ix)
}

/// Check that a previous sibling instruction was to the Secp256k1 precompile.
#[inline]
pub fn require_secp256k1_instruction(
    sibling_index: u64,
) -> Result<ProcessedInstruction, ProgramError> {
    let ix = get_processed_instruction(sibling_index)
        .ok_or(ProgramError::InvalidArgument)?;

    if !crate::address::address_eq(&ix.program_id, &SECP256K1_PROGRAM_ID) {
        return Err(ProgramError::IncorrectProgramId);
    }

    Ok(ix)
}

// ---- Internal types for syscall FFI ----------------------------------

#[repr(C)]
#[allow(dead_code)]
struct ProcessedInstructionMeta {
    data_len: u64,
    accounts_len: u64,
}
