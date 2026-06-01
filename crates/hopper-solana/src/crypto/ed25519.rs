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
//! check_ed25519_signature(&sysvar_data, 0, expected_signer, expected_message)?;
//! ```

use hopper_runtime::address;
use hopper_runtime::{error::ProgramError, Address};

use crate::introspect::{instruction_data_range, program_id_at};

/// Ed25519 precompile program address.
pub const ED25519_PROGRAM: Address = address!("Ed25519SigVerify111111111111111111111111111");

/// Size of one Ed25519 signature parameter block (14 bytes).
const SIG_PARAM_SIZE: usize = 14;

/// Size of an Ed25519 public key (32 bytes).
const PUBKEY_LEN: usize = 32;

/// Size of one Ed25519 signature (64 bytes).
const SIGNATURE_LEN: usize = 64;

/// Solana's sentinel for data stored in the same precompile instruction.
const SAME_INSTRUCTION: u16 = u16::MAX;

#[derive(Clone, Copy)]
struct Ed25519Offsets {
    signature_offset: usize,
    signature_instruction_index: u16,
    public_key_offset: usize,
    public_key_instruction_index: u16,
    message_offset: usize,
    message_size: usize,
    message_instruction_index: u16,
}

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
    check_ed25519_signature_at(
        sysvar_data,
        ed25519_ix_index,
        0,
        expected_signer,
        expected_message,
    )
}

/// Same as [`check_ed25519_signature`], but verifies a selected signature
/// entry inside a multi-signature Ed25519 precompile instruction.
#[inline]
pub fn check_ed25519_signature_at(
    sysvar_data: &[u8],
    ed25519_ix_index: u16,
    signature_index: usize,
    expected_signer: &[u8; 32],
    expected_message: &[u8],
) -> Result<(), ProgramError> {
    let ix_data = ed25519_instruction_data(sysvar_data, ed25519_ix_index)?;
    let offsets = signature_offsets(ix_data, signature_index)?;
    offsets.require_inline()?;
    offsets.require_payload_bounds(ix_data)?;

    let signer = inline_range(ix_data, offsets.public_key_offset, PUBKEY_LEN)?;
    if signer != expected_signer {
        return Err(ProgramError::InvalidArgument);
    }

    if offsets.message_size != expected_message.len() {
        return Err(ProgramError::InvalidArgument);
    }
    let message = inline_range(ix_data, offsets.message_offset, offsets.message_size)?;
    if message != expected_message {
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
    check_ed25519_signer_at(sysvar_data, ed25519_ix_index, 0, expected_signer)
}

/// Same as [`check_ed25519_signer`], but verifies a selected signature entry
/// inside a multi-signature Ed25519 precompile instruction.
#[inline]
pub fn check_ed25519_signer_at(
    sysvar_data: &[u8],
    ed25519_ix_index: u16,
    signature_index: usize,
    expected_signer: &[u8; 32],
) -> Result<(), ProgramError> {
    let ix_data = ed25519_instruction_data(sysvar_data, ed25519_ix_index)?;
    let offsets = signature_offsets(ix_data, signature_index)?;
    offsets.require_inline()?;
    offsets.require_payload_bounds(ix_data)?;

    let signer = inline_range(ix_data, offsets.public_key_offset, PUBKEY_LEN)?;
    if signer != expected_signer {
        return Err(ProgramError::InvalidArgument);
    }

    Ok(())
}

#[inline]
fn ed25519_instruction_data(
    sysvar_data: &[u8],
    ed25519_ix_index: u16,
) -> Result<&[u8], ProgramError> {
    let program_id = program_id_at(sysvar_data, ed25519_ix_index)?;
    if program_id != ED25519_PROGRAM {
        return Err(ProgramError::InvalidArgument);
    }

    let (ix_data_offset, ix_data_len) = instruction_data_range(sysvar_data, ed25519_ix_index)?;
    Ok(&sysvar_data[ix_data_offset..ix_data_offset + ix_data_len])
}

#[inline]
fn signature_offsets(
    ix_data: &[u8],
    signature_index: usize,
) -> Result<Ed25519Offsets, ProgramError> {
    if ix_data.len() < 2 + SIG_PARAM_SIZE {
        return Err(ProgramError::InvalidAccountData);
    }

    let num_signatures = ix_data[0] as usize;
    if num_signatures == 0 || signature_index >= num_signatures {
        return Err(ProgramError::InvalidAccountData);
    }

    let table_len = num_signatures
        .checked_mul(SIG_PARAM_SIZE)
        .and_then(|len| len.checked_add(2))
        .ok_or(ProgramError::InvalidAccountData)?;
    if table_len > ix_data.len() {
        return Err(ProgramError::InvalidAccountData);
    }

    let params_start = signature_index
        .checked_mul(SIG_PARAM_SIZE)
        .and_then(|offset| offset.checked_add(2))
        .ok_or(ProgramError::InvalidAccountData)?;
    let params_end = params_start + SIG_PARAM_SIZE;
    let params = &ix_data[params_start..params_end];

    Ok(Ed25519Offsets {
        signature_offset: u16::from_le_bytes([params[0], params[1]]) as usize,
        signature_instruction_index: u16::from_le_bytes([params[2], params[3]]),
        public_key_offset: u16::from_le_bytes([params[4], params[5]]) as usize,
        public_key_instruction_index: u16::from_le_bytes([params[6], params[7]]),
        message_offset: u16::from_le_bytes([params[8], params[9]]) as usize,
        message_size: u16::from_le_bytes([params[10], params[11]]) as usize,
        message_instruction_index: u16::from_le_bytes([params[12], params[13]]),
    })
}

impl Ed25519Offsets {
    #[inline]
    fn require_inline(self) -> Result<(), ProgramError> {
        if self.signature_instruction_index != SAME_INSTRUCTION
            || self.public_key_instruction_index != SAME_INSTRUCTION
            || self.message_instruction_index != SAME_INSTRUCTION
        {
            return Err(ProgramError::InvalidArgument);
        }

        Ok(())
    }

    #[inline]
    fn require_payload_bounds(self, data: &[u8]) -> Result<(), ProgramError> {
        inline_range(data, self.signature_offset, SIGNATURE_LEN)?;
        inline_range(data, self.public_key_offset, PUBKEY_LEN)?;
        inline_range(data, self.message_offset, self.message_size)?;
        Ok(())
    }
}

#[inline]
fn inline_range(data: &[u8], offset: usize, len: usize) -> Result<&[u8], ProgramError> {
    let end = offset
        .checked_add(len)
        .ok_or(ProgramError::AccountDataTooSmall)?;
    if end > data.len() {
        return Err(ProgramError::AccountDataTooSmall);
    }
    Ok(&data[offset..end])
}

#[cfg(test)]
mod tests {
    extern crate alloc;

    use super::*;
    use alloc::vec;
    use alloc::vec::Vec;

    const SIGNER_A: [u8; 32] = [7; 32];
    const SIGNER_B: [u8; 32] = [8; 32];
    const MESSAGE_A: &[u8] = b"hopper-auth";
    const MESSAGE_B: &[u8] = b"hopper-second";

    #[test]
    fn checks_single_signature_payload() {
        let ix_data = ed25519_data(&[(SIGNER_A, MESSAGE_A, SAME_INSTRUCTION)]);
        let sysvar = sysvar_with_instruction(&ix_data);

        assert_eq!(
            check_ed25519_signature(&sysvar, 0, &SIGNER_A, MESSAGE_A),
            Ok(())
        );
    }

    #[test]
    fn checks_selected_signature_payload() {
        let ix_data = ed25519_data(&[
            (SIGNER_A, MESSAGE_A, SAME_INSTRUCTION),
            (SIGNER_B, MESSAGE_B, SAME_INSTRUCTION),
        ]);
        let sysvar = sysvar_with_instruction(&ix_data);

        assert_eq!(
            check_ed25519_signature_at(&sysvar, 0, 1, &SIGNER_B, MESSAGE_B),
            Ok(())
        );
        assert_eq!(
            check_ed25519_signature(&sysvar, 0, &SIGNER_B, MESSAGE_B),
            Err(ProgramError::InvalidArgument)
        );
    }

    #[test]
    fn rejects_cross_instruction_offsets() {
        let ix_data = ed25519_data(&[(SIGNER_A, MESSAGE_A, 0)]);
        let sysvar = sysvar_with_instruction(&ix_data);

        assert_eq!(
            check_ed25519_signature(&sysvar, 0, &SIGNER_A, MESSAGE_A),
            Err(ProgramError::InvalidArgument)
        );
    }

    #[test]
    fn rejects_signature_offset_out_of_bounds() {
        let mut ix_data = ed25519_data(&[(SIGNER_A, MESSAGE_A, SAME_INSTRUCTION)]);
        let out_of_bounds = ix_data.len() as u16;
        ix_data[2..4].copy_from_slice(&out_of_bounds.to_le_bytes());
        let sysvar = sysvar_with_instruction(&ix_data);

        assert_eq!(
            check_ed25519_signature(&sysvar, 0, &SIGNER_A, MESSAGE_A),
            Err(ProgramError::AccountDataTooSmall)
        );
    }

    #[test]
    fn rejects_selected_signature_out_of_range() {
        let ix_data = ed25519_data(&[(SIGNER_A, MESSAGE_A, SAME_INSTRUCTION)]);
        let sysvar = sysvar_with_instruction(&ix_data);

        assert_eq!(
            check_ed25519_signature_at(&sysvar, 0, 1, &SIGNER_A, MESSAGE_A),
            Err(ProgramError::InvalidAccountData)
        );
    }

    fn ed25519_data(signatures: &[([u8; 32], &[u8], u16)]) -> Vec<u8> {
        let mut data = vec![signatures.len() as u8, 0];
        data.resize(2 + signatures.len() * SIG_PARAM_SIZE, 0);

        let mut payload_offset = data.len();
        for (index, (signer, message, instruction_index)) in signatures.iter().enumerate() {
            let signature_offset = payload_offset as u16;
            data.extend_from_slice(&[3; SIGNATURE_LEN]);
            payload_offset += SIGNATURE_LEN;

            let public_key_offset = payload_offset as u16;
            data.extend_from_slice(signer);
            payload_offset += PUBKEY_LEN;

            let message_offset = payload_offset as u16;
            data.extend_from_slice(message);
            payload_offset += message.len();

            let params_start = 2 + index * SIG_PARAM_SIZE;
            data[params_start..params_start + 2].copy_from_slice(&signature_offset.to_le_bytes());
            data[params_start + 2..params_start + 4]
                .copy_from_slice(&instruction_index.to_le_bytes());
            data[params_start + 4..params_start + 6]
                .copy_from_slice(&public_key_offset.to_le_bytes());
            data[params_start + 6..params_start + 8]
                .copy_from_slice(&instruction_index.to_le_bytes());
            data[params_start + 8..params_start + 10]
                .copy_from_slice(&message_offset.to_le_bytes());
            data[params_start + 10..params_start + 12]
                .copy_from_slice(&(message.len() as u16).to_le_bytes());
            data[params_start + 12..params_start + 14]
                .copy_from_slice(&instruction_index.to_le_bytes());
        }

        data
    }

    fn sysvar_with_instruction(ix_data: &[u8]) -> Vec<u8> {
        let instruction_offset = 4u16;
        let mut data = Vec::new();
        data.extend_from_slice(&1u16.to_le_bytes());
        data.extend_from_slice(&instruction_offset.to_le_bytes());
        data.extend_from_slice(&0u16.to_le_bytes());
        data.extend_from_slice(ED25519_PROGRAM.as_ref());
        data.extend_from_slice(&(ix_data.len() as u16).to_le_bytes());
        data.extend_from_slice(ix_data);
        data.extend_from_slice(&0u16.to_le_bytes());
        data
    }
}
