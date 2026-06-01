//! Secp256k1 precompile instruction validation.
//!
//! Solana's secp256k1 native program verifies Ethereum-style secp256k1
//! signatures. Hopper checks the sibling instruction so a program can assert
//! that the runtime verified the expected Ethereum address and message bytes.

use hopper_runtime::address;
use hopper_runtime::{error::ProgramError, Address, ProgramResult};

use crate::introspect::{instruction_data_range, program_id_at};

/// Secp256k1 native program address.
pub const SECP256K1_PROGRAM: Address = address!("KeccakSecp256k11111111111111111111111111111");

const SIGNATURE_OFFSETS_SIZE: usize = 11;
const SIGNATURE_LEN: usize = 64;
const RECOVERY_ID_LEN: usize = 1;
const SIGNATURE_PAYLOAD_LEN: usize = SIGNATURE_LEN + RECOVERY_ID_LEN;
const ETH_ADDRESS_LEN: usize = 20;

#[derive(Clone, Copy)]
struct Secp256k1Offsets {
    signature_offset: usize,
    signature_instruction_index: u8,
    eth_address_offset: usize,
    eth_address_instruction_index: u8,
    message_data_offset: usize,
    message_data_size: usize,
    message_instruction_index: u8,
}

/// Verify the first secp256k1 signature entry in `secp256k1_ix_index`.
///
/// This is the strict default: signature, recovery ID, Ethereum address, and
/// message bytes must all be stored in the secp256k1 instruction itself.
#[inline]
pub fn check_secp256k1_instruction(
    sysvar_data: &[u8],
    secp256k1_ix_index: u16,
    expected_eth_address: &[u8; 20],
    expected_message: &[u8],
) -> ProgramResult {
    check_secp256k1_instruction_at(
        sysvar_data,
        secp256k1_ix_index,
        0,
        expected_eth_address,
        expected_message,
    )
}

/// Verify a selected secp256k1 signature entry in `secp256k1_ix_index`.
///
/// This strict variant rejects offset records that reference any other
/// instruction. Use [`check_secp256k1_instruction_at_cross_instruction`] only
/// when the protocol deliberately stores signature material elsewhere.
#[inline]
pub fn check_secp256k1_instruction_at(
    sysvar_data: &[u8],
    secp256k1_ix_index: u16,
    signature_index: usize,
    expected_eth_address: &[u8; 20],
    expected_message: &[u8],
) -> ProgramResult {
    check_secp256k1_instruction_at_inner(
        sysvar_data,
        secp256k1_ix_index,
        signature_index,
        expected_eth_address,
        expected_message,
        false,
    )
}

/// Verify a selected secp256k1 signature entry and allow references into other
/// transaction instructions.
#[inline]
pub fn check_secp256k1_instruction_at_cross_instruction(
    sysvar_data: &[u8],
    secp256k1_ix_index: u16,
    signature_index: usize,
    expected_eth_address: &[u8; 20],
    expected_message: &[u8],
) -> ProgramResult {
    check_secp256k1_instruction_at_inner(
        sysvar_data,
        secp256k1_ix_index,
        signature_index,
        expected_eth_address,
        expected_message,
        true,
    )
}

/// Convenience wrapper for protocols that store a 32-byte message hash as the
/// secp256k1 message payload.
#[inline]
pub fn check_secp256k1_message_hash(
    sysvar_data: &[u8],
    secp256k1_ix_index: u16,
    expected_eth_address: &[u8; 20],
    expected_message_hash: &[u8; 32],
) -> ProgramResult {
    check_secp256k1_instruction(
        sysvar_data,
        secp256k1_ix_index,
        expected_eth_address,
        expected_message_hash,
    )
}

#[inline]
fn check_secp256k1_instruction_at_inner(
    sysvar_data: &[u8],
    secp256k1_ix_index: u16,
    signature_index: usize,
    expected_eth_address: &[u8; 20],
    expected_message: &[u8],
    allow_cross_instruction: bool,
) -> ProgramResult {
    let secp_ix_data = secp256k1_instruction_data(sysvar_data, secp256k1_ix_index)?;
    let offsets = signature_offsets(secp_ix_data, signature_index)?;

    let signature_data = referenced_instruction_data(
        sysvar_data,
        secp_ix_data,
        secp256k1_ix_index,
        offsets.signature_instruction_index,
        allow_cross_instruction,
    )?;
    inline_range(signature_data, offsets.signature_offset, SIGNATURE_PAYLOAD_LEN)?;

    let eth_data = referenced_instruction_data(
        sysvar_data,
        secp_ix_data,
        secp256k1_ix_index,
        offsets.eth_address_instruction_index,
        allow_cross_instruction,
    )?;
    let eth_address = inline_range(eth_data, offsets.eth_address_offset, ETH_ADDRESS_LEN)?;
    if eth_address != expected_eth_address {
        return Err(ProgramError::InvalidArgument);
    }

    let message_data = referenced_instruction_data(
        sysvar_data,
        secp_ix_data,
        secp256k1_ix_index,
        offsets.message_instruction_index,
        allow_cross_instruction,
    )?;
    if offsets.message_data_size != expected_message.len() {
        return Err(ProgramError::InvalidArgument);
    }
    let message = inline_range(message_data, offsets.message_data_offset, offsets.message_data_size)?;
    if message != expected_message {
        return Err(ProgramError::InvalidArgument);
    }

    Ok(())
}

#[inline]
fn secp256k1_instruction_data(
    sysvar_data: &[u8],
    secp256k1_ix_index: u16,
) -> Result<&[u8], ProgramError> {
    let program_id = program_id_at(sysvar_data, secp256k1_ix_index)?;
    if program_id != SECP256K1_PROGRAM {
        return Err(ProgramError::InvalidArgument);
    }

    let (ix_data_offset, ix_data_len) = instruction_data_range(sysvar_data, secp256k1_ix_index)?;
    Ok(&sysvar_data[ix_data_offset..ix_data_offset + ix_data_len])
}

#[inline]
fn signature_offsets(
    secp_ix_data: &[u8],
    signature_index: usize,
) -> Result<Secp256k1Offsets, ProgramError> {
    let num_signatures = *secp_ix_data
        .first()
        .ok_or(ProgramError::InvalidAccountData)? as usize;
    if num_signatures == 0 || signature_index >= num_signatures {
        return Err(ProgramError::InvalidAccountData);
    }

    let table_len = num_signatures
        .checked_mul(SIGNATURE_OFFSETS_SIZE)
        .and_then(|len| len.checked_add(1))
        .ok_or(ProgramError::InvalidAccountData)?;
    if table_len > secp_ix_data.len() {
        return Err(ProgramError::InvalidAccountData);
    }

    let start = signature_index
        .checked_mul(SIGNATURE_OFFSETS_SIZE)
        .and_then(|offset| offset.checked_add(1))
        .ok_or(ProgramError::InvalidAccountData)?;
    let chunk = &secp_ix_data[start..start + SIGNATURE_OFFSETS_SIZE];

    Ok(Secp256k1Offsets {
        signature_offset: read_u16(chunk, 0) as usize,
        signature_instruction_index: chunk[2],
        eth_address_offset: read_u16(chunk, 3) as usize,
        eth_address_instruction_index: chunk[5],
        message_data_offset: read_u16(chunk, 6) as usize,
        message_data_size: read_u16(chunk, 8) as usize,
        message_instruction_index: chunk[10],
    })
}

#[inline]
fn referenced_instruction_data<'a>(
    sysvar_data: &'a [u8],
    secp_ix_data: &'a [u8],
    secp256k1_ix_index: u16,
    referenced_index: u8,
    allow_cross_instruction: bool,
) -> Result<&'a [u8], ProgramError> {
    if referenced_index as u16 == secp256k1_ix_index {
        return Ok(secp_ix_data);
    }
    if !allow_cross_instruction {
        return Err(ProgramError::InvalidArgument);
    }
    let (data_offset, data_len) = instruction_data_range(sysvar_data, referenced_index as u16)?;
    Ok(&sysvar_data[data_offset..data_offset + data_len])
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

#[inline]
fn read_u16(data: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([data[offset], data[offset + 1]])
}

#[cfg(test)]
mod tests {
    extern crate alloc;

    use super::*;
    use alloc::vec;
    use alloc::vec::Vec;

    const ETH_A: [u8; 20] = [5; 20];
    const ETH_B: [u8; 20] = [6; 20];
    const MESSAGE_A: &[u8] = b"hopper-secp-auth";
    const MESSAGE_B: &[u8] = b"hopper-secp-second";
    const PROGRAM_A: Address = Address::new([42; 32]);

    #[test]
    fn checks_single_inline_signature() {
        let secp_data = inline_secp_data(&[(ETH_A, MESSAGE_A)], 0);
        let sysvar = sysvar_with_instructions(&[(SECP256K1_PROGRAM, &secp_data)]);

        assert_eq!(
            check_secp256k1_instruction(&sysvar, 0, &ETH_A, MESSAGE_A),
            Ok(())
        );
    }

    #[test]
    fn checks_selected_inline_signature() {
        let secp_data = inline_secp_data(&[(ETH_A, MESSAGE_A), (ETH_B, MESSAGE_B)], 0);
        let sysvar = sysvar_with_instructions(&[(SECP256K1_PROGRAM, &secp_data)]);

        assert_eq!(
            check_secp256k1_instruction_at(&sysvar, 0, 1, &ETH_B, MESSAGE_B),
            Ok(())
        );
        assert_eq!(
            check_secp256k1_instruction(&sysvar, 0, &ETH_B, MESSAGE_B),
            Err(ProgramError::InvalidArgument)
        );
    }

    #[test]
    fn rejects_cross_instruction_reference_by_default() {
        let (secp_data, payload) = cross_instruction_secp_data(1, ETH_A, MESSAGE_A);
        let sysvar = sysvar_with_instructions(&[
            (SECP256K1_PROGRAM, &secp_data),
            (PROGRAM_A, &payload),
        ]);

        assert_eq!(
            check_secp256k1_instruction(&sysvar, 0, &ETH_A, MESSAGE_A),
            Err(ProgramError::InvalidArgument)
        );
    }

    #[test]
    fn cross_instruction_variant_checks_referenced_payload() {
        let (secp_data, payload) = cross_instruction_secp_data(1, ETH_A, MESSAGE_A);
        let sysvar = sysvar_with_instructions(&[
            (SECP256K1_PROGRAM, &secp_data),
            (PROGRAM_A, &payload),
        ]);

        assert_eq!(
            check_secp256k1_instruction_at_cross_instruction(&sysvar, 0, 0, &ETH_A, MESSAGE_A),
            Ok(())
        );
    }

    #[test]
    fn rejects_signature_bounds() {
        let mut secp_data = inline_secp_data(&[(ETH_A, MESSAGE_A)], 0);
        let out_of_bounds = secp_data.len() as u16;
        secp_data[1..3].copy_from_slice(&out_of_bounds.to_le_bytes());
        let sysvar = sysvar_with_instructions(&[(SECP256K1_PROGRAM, &secp_data)]);

        assert_eq!(
            check_secp256k1_instruction(&sysvar, 0, &ETH_A, MESSAGE_A),
            Err(ProgramError::AccountDataTooSmall)
        );
    }

    fn inline_secp_data(signatures: &[([u8; 20], &[u8])], instruction_index: u8) -> Vec<u8> {
        let mut data = vec![signatures.len() as u8];
        data.resize(1 + signatures.len() * SIGNATURE_OFFSETS_SIZE, 0);

        let mut payload_offset = data.len();
        for (index, (eth_address, message)) in signatures.iter().enumerate() {
            let signature_offset = payload_offset as u16;
            data.extend_from_slice(&[4; SIGNATURE_LEN]);
            data.push(1);
            payload_offset += SIGNATURE_PAYLOAD_LEN;

            let eth_address_offset = payload_offset as u16;
            data.extend_from_slice(eth_address);
            payload_offset += ETH_ADDRESS_LEN;

            let message_data_offset = payload_offset as u16;
            data.extend_from_slice(message);
            payload_offset += message.len();

            write_offsets(
                &mut data,
                index,
                signature_offset,
                instruction_index,
                eth_address_offset,
                instruction_index,
                message_data_offset,
                message.len() as u16,
                instruction_index,
            );
        }

        data
    }

    fn cross_instruction_secp_data(
        payload_instruction_index: u8,
        eth_address: [u8; 20],
        message: &[u8],
    ) -> (Vec<u8>, Vec<u8>) {
        let mut payload = Vec::new();
        let signature_offset = 0u16;
        payload.extend_from_slice(&[4; SIGNATURE_LEN]);
        payload.push(1);

        let eth_address_offset = payload.len() as u16;
        payload.extend_from_slice(&eth_address);

        let message_data_offset = payload.len() as u16;
        payload.extend_from_slice(message);

        let mut secp_data = vec![0; 1 + SIGNATURE_OFFSETS_SIZE];
        secp_data[0] = 1;
        write_offsets(
            &mut secp_data,
            0,
            signature_offset,
            payload_instruction_index,
            eth_address_offset,
            payload_instruction_index,
            message_data_offset,
            message.len() as u16,
            payload_instruction_index,
        );
        (secp_data, payload)
    }

    #[allow(clippy::too_many_arguments)]
    fn write_offsets(
        data: &mut [u8],
        index: usize,
        signature_offset: u16,
        signature_instruction_index: u8,
        eth_address_offset: u16,
        eth_address_instruction_index: u8,
        message_data_offset: u16,
        message_data_size: u16,
        message_instruction_index: u8,
    ) {
        let start = 1 + index * SIGNATURE_OFFSETS_SIZE;
        data[start..start + 2].copy_from_slice(&signature_offset.to_le_bytes());
        data[start + 2] = signature_instruction_index;
        data[start + 3..start + 5].copy_from_slice(&eth_address_offset.to_le_bytes());
        data[start + 5] = eth_address_instruction_index;
        data[start + 6..start + 8].copy_from_slice(&message_data_offset.to_le_bytes());
        data[start + 8..start + 10].copy_from_slice(&message_data_size.to_le_bytes());
        data[start + 10] = message_instruction_index;
    }

    fn sysvar_with_instructions(instructions: &[(Address, &[u8])]) -> Vec<u8> {
        let mut data = Vec::new();
        data.extend_from_slice(&(instructions.len() as u16).to_le_bytes());
        data.resize(2 + instructions.len() * 2, 0);

        for (index, (program_id, instruction_data)) in instructions.iter().enumerate() {
            let instruction_offset = data.len() as u16;
            let offset_start = 2 + index * 2;
            data[offset_start..offset_start + 2]
                .copy_from_slice(&instruction_offset.to_le_bytes());

            data.extend_from_slice(&0u16.to_le_bytes());
            data.extend_from_slice(program_id.as_ref());
            data.extend_from_slice(&(instruction_data.len() as u16).to_le_bytes());
            data.extend_from_slice(instruction_data);
        }

        data.extend_from_slice(&0u16.to_le_bytes());
        data
    }
}