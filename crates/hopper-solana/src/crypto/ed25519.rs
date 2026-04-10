//! Ed25519 precompile signature verification.
//!
//! On Solana, Ed25519 signature verification happens via the Ed25519
//! precompile program. A transaction includes an Ed25519 instruction with
//! the signature data, and your program reads the Sysvar Instructions to
//! verify the precompile ran with the expected signer and message.
//!
//! Used for gasless relayers, signed price feeds, and off-chain authorization.
//!
//! ## Ed25519 precompile instruction data layout
//!
//! ```text
//! [num_signatures: u8]
//! [padding: u8]
//! For each signature:
//!   [signature_offset: u16 LE]
//!   [signature_instruction_index: u16 LE]
//!   [public_key_offset: u16 LE]
//!   [public_key_instruction_index: u16 LE]
//!   [message_data_offset: u16 LE]
//!   [message_data_size: u16 LE]
//!   [message_instruction_index: u16 LE]
//! ```
//!
//! When all data is in the same instruction (the typical case),
//! all `*_instruction_index` fields equal `0xFFFF`.
//!
//! ```rust,ignore
//! let sysvar_data = sysvar_ix.try_borrow()?;
//! check_ed25519_signature(&sysvar_data, 0, expected_signer.as_ref(), expected_message)?;
//! ```

use hopper_runtime::{error::ProgramError, Address};
use hopper_runtime::address;

use crate::introspect::{instruction_data_range, program_id_at};

/// Ed25519 precompile program address.
pub const ED25519_PROGRAM: Address =
    address!("Ed25519SigVerify111111111111111111111111111");

/// Size of one Ed25519 signature parameter block (14 bytes).
const SIG_PARAM_SIZE: usize = 14;

/// Size of an Ed25519 public key (32 bytes).
const PUBKEY_LEN: usize = 32;

/// Verify that an Ed25519 precompile instruction at `ed25519_ix_index`
/// contains a valid signature from `expected_signer` over `expected_message`.
///
/// This checks:
/// 1. The instruction at `ed25519_ix_index` is the Ed25519 precompile
/// 2. At least one signature exists
/// 3. The first signature's public key matches `expected_signer`
/// 4. The first signature's message matches `expected_message`
///
/// The runtime already verified the cryptographic signature. We just
/// need to confirm it was over the right key + message.
#[inline]
pub fn check_ed25519_signature(
    sysvar_data: &[u8],
    ed25519_ix_index: u16,
    expected_signer: &[u8; 32],
    expected_message: &[u8],
) -> Result<(), ProgramError> {
    // 1. Verify the instruction at that index is the Ed25519 precompile.
    let program_id = program_id_at(sysvar_data, ed25519_ix_index)?;
    if program_id != ED25519_PROGRAM {
        return Err(ProgramError::InvalidArgument);
    }

    // 2. Read the Ed25519 instruction data.
    let (ix_data_offset, ix_data_len) =
        instruction_data_range(sysvar_data, ed25519_ix_index)?;
    let ix_data = &sysvar_data[ix_data_offset..ix_data_offset + ix_data_len];

    // Minimum: 2 bytes header + at least 1 signature param block (14 bytes)
    if ix_data_len < 2 + SIG_PARAM_SIZE {
        return Err(ProgramError::InvalidAccountData);
    }

    let num_signatures = ix_data[0] as usize;
    if num_signatures == 0 {
        return Err(ProgramError::InvalidAccountData);
    }

    // 3. Read the first signature's parameter block.
    let params = &ix_data[2..2 + SIG_PARAM_SIZE];

    let pubkey_offset = u16::from_le_bytes([params[4], params[5]]) as usize;
    let pubkey_ix_index = u16::from_le_bytes([params[6], params[7]]);
    let message_offset = u16::from_le_bytes([params[8], params[9]]) as usize;
    let message_size = u16::from_le_bytes([params[10], params[11]]) as usize;
    let message_ix_index = u16::from_le_bytes([params[12], params[13]]);

    // 4. Verify pubkey. When pubkey_ix_index == 0xFFFF, pubkey is inline.
    if pubkey_ix_index != 0xFFFF {
        return Err(ProgramError::InvalidArgument);
    }
    if pubkey_offset + PUBKEY_LEN > ix_data_len {
        return Err(ProgramError::AccountDataTooSmall);
    }
    if &ix_data[pubkey_offset..pubkey_offset + PUBKEY_LEN] != expected_signer {
        return Err(ProgramError::InvalidArgument);
    }

    // 5. Verify message. Same inline check.
    if message_ix_index != 0xFFFF {
        return Err(ProgramError::InvalidArgument);
    }
    if message_offset + message_size > ix_data_len {
        return Err(ProgramError::AccountDataTooSmall);
    }
    if message_size != expected_message.len() {
        return Err(ProgramError::InvalidArgument);
    }
    if &ix_data[message_offset..message_offset + message_size] != expected_message {
        return Err(ProgramError::InvalidArgument);
    }

    Ok(())
}

/// Same as [`check_ed25519_signature`] but only verifies the signer,
/// not the message content. Useful when you just need to confirm "this
/// key signed something in this transaction".
#[inline]
pub fn check_ed25519_signer(
    sysvar_data: &[u8],
    ed25519_ix_index: u16,
    expected_signer: &[u8; 32],
) -> Result<(), ProgramError> {
    let program_id = program_id_at(sysvar_data, ed25519_ix_index)?;
    if program_id != ED25519_PROGRAM {
        return Err(ProgramError::InvalidArgument);
    }

    let (ix_data_offset, ix_data_len) =
        instruction_data_range(sysvar_data, ed25519_ix_index)?;
    let ix_data = &sysvar_data[ix_data_offset..ix_data_offset + ix_data_len];

    if ix_data_len < 2 + SIG_PARAM_SIZE {
        return Err(ProgramError::InvalidAccountData);
    }

    let num_signatures = ix_data[0] as usize;
    if num_signatures == 0 {
        return Err(ProgramError::InvalidAccountData);
    }

    let params = &ix_data[2..2 + SIG_PARAM_SIZE];
    let pubkey_offset = u16::from_le_bytes([params[4], params[5]]) as usize;
    let pubkey_ix_index = u16::from_le_bytes([params[6], params[7]]);

    if pubkey_ix_index != 0xFFFF {
        return Err(ProgramError::InvalidArgument);
    }
    if pubkey_offset + PUBKEY_LEN > ix_data_len {
        return Err(ProgramError::AccountDataTooSmall);
    }
    if &ix_data[pubkey_offset..pubkey_offset + PUBKEY_LEN] != expected_signer {
        return Err(ProgramError::InvalidArgument);
    }

    Ok(())
}
