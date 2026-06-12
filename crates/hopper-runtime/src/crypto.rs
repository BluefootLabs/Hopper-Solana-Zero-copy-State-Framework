//! Runtime cryptography helpers built on Solana syscalls and precompiles.

use crate::{Address, ProgramError};

pub type Sha256Hash = [u8; 32];
pub type Keccak256Hash = [u8; 32];
pub type Blake3Hash = [u8; 32];
pub type Secp256k1Pubkey = [u8; 64];
pub type EthereumAddress = [u8; 20];
pub type CurvePoint = [u8; 32];
pub type CurveScalar = [u8; 32];
pub type PoseidonHash = [u8; 32];
pub type AltBn128G1 = [u8; 64];
pub type AltBn128G1Compressed = [u8; 32];
pub type AltBn128G2 = [u8; 128];
pub type AltBn128G2Compressed = [u8; 64];
pub type AltBn128PairingResult = [u8; 32];

pub const MAX_HASH_SEGMENTS: usize = 16;
pub const CURVE25519_EDWARDS: u64 = 0;
pub const CURVE25519_RISTRETTO: u64 = 1;
pub const CURVE_GROUP_ADD: u64 = 0;
pub const CURVE_GROUP_SUB: u64 = 1;
pub const CURVE_GROUP_MUL: u64 = 2;
pub const POSEIDON_BN254_X5: u64 = 0;
pub const POSEIDON_BIG_ENDIAN: u64 = 0;
pub const POSEIDON_LITTLE_ENDIAN: u64 = 1;
pub const MAX_POSEIDON_INPUTS: usize = 12;
pub const POSEIDON_INPUT_LEN: usize = 32;
pub const ALT_BN128_LE_FLAG: u64 = 0x80;
pub const ALT_BN128_FIELD_SIZE: usize = 32;
pub const ALT_BN128_G1_POINT_SIZE: usize = 64;
pub const ALT_BN128_G2_POINT_SIZE: usize = 128;
pub const ALT_BN128_G1_ADDITION_INPUT_SIZE: usize = 128;
pub const ALT_BN128_G2_ADDITION_INPUT_SIZE: usize = 256;
pub const ALT_BN128_G1_MULTIPLICATION_INPUT_SIZE: usize = 96;
pub const ALT_BN128_G2_MULTIPLICATION_INPUT_SIZE: usize = 160;
pub const ALT_BN128_PAIRING_ELEMENT_SIZE: usize = 192;
pub const ALT_BN128_G1_ADD_BE: u64 = 0;
pub const ALT_BN128_G1_SUB_BE: u64 = 1;
pub const ALT_BN128_G1_MUL_BE: u64 = 2;
pub const ALT_BN128_PAIRING_BE: u64 = 3;
pub const ALT_BN128_G2_ADD_BE: u64 = 4;
pub const ALT_BN128_G2_SUB_BE: u64 = 5;
pub const ALT_BN128_G2_MUL_BE: u64 = 6;
pub const ALT_BN128_G1_ADD_LE: u64 = ALT_BN128_G1_ADD_BE | ALT_BN128_LE_FLAG;
pub const ALT_BN128_G1_SUB_LE: u64 = ALT_BN128_G1_SUB_BE | ALT_BN128_LE_FLAG;
pub const ALT_BN128_G1_MUL_LE: u64 = ALT_BN128_G1_MUL_BE | ALT_BN128_LE_FLAG;
pub const ALT_BN128_PAIRING_LE: u64 = ALT_BN128_PAIRING_BE | ALT_BN128_LE_FLAG;
pub const ALT_BN128_G2_ADD_LE: u64 = ALT_BN128_G2_ADD_BE | ALT_BN128_LE_FLAG;
pub const ALT_BN128_G2_SUB_LE: u64 = ALT_BN128_G2_SUB_BE | ALT_BN128_LE_FLAG;
pub const ALT_BN128_G2_MUL_LE: u64 = ALT_BN128_G2_MUL_BE | ALT_BN128_LE_FLAG;
pub const ALT_BN128_G1_COMPRESS_BE: u64 = 0;
pub const ALT_BN128_G1_DECOMPRESS_BE: u64 = 1;
pub const ALT_BN128_G2_COMPRESS_BE: u64 = 2;
pub const ALT_BN128_G2_DECOMPRESS_BE: u64 = 3;
pub const ALT_BN128_G1_COMPRESS_LE: u64 = ALT_BN128_G1_COMPRESS_BE | ALT_BN128_LE_FLAG;
pub const ALT_BN128_G1_DECOMPRESS_LE: u64 = ALT_BN128_G1_DECOMPRESS_BE | ALT_BN128_LE_FLAG;
pub const ALT_BN128_G2_COMPRESS_LE: u64 = ALT_BN128_G2_COMPRESS_BE | ALT_BN128_LE_FLAG;
pub const ALT_BN128_G2_DECOMPRESS_LE: u64 = ALT_BN128_G2_DECOMPRESS_BE | ALT_BN128_LE_FLAG;
pub const MAX_INSTRUCTION_DATA_LEN: usize = 1232;
pub const MAX_INSTRUCTION_ACCOUNTS_BYTES: usize = 2176;

pub const ED25519_PROGRAM_ID: Address = Address::new_from_array(
    crate::__decode_base58_32("Ed25519SigVerify111111111111111111111111111"),
);

pub const SECP256K1_PROGRAM_ID: Address = Address::new_from_array(
    crate::__decode_base58_32("KeccakSecp256k11111111111111111111111111111"),
);

#[derive(Clone, Debug)]
pub struct ProcessedInstruction {
    pub program_id: Address,
    pub data: [u8; MAX_INSTRUCTION_DATA_LEN],
    pub data_len: usize,
    pub accounts_len: usize,
}

#[derive(Clone, Debug)]
pub struct ProcessedInstructionData<const MAX_DATA: usize> {
    pub program_id: Address,
    pub data: [u8; MAX_DATA],
    pub data_len: usize,
}

#[repr(C)]
struct ProcessedInstructionMeta {
    data_len: u64,
    accounts_len: u64,
}

#[cfg(feature = "crypto-big-mod-exp")]
#[repr(C)]
struct BigModExpParams {
    base: *const u8,
    base_len: u64,
    exponent: *const u8,
    exponent_len: u64,
    modulus: *const u8,
    modulus_len: u64,
}

#[cfg(any(
    feature = "crypto-curve",
    feature = "crypto-poseidon",
    feature = "crypto-bn254",
    feature = "crypto-big-mod-exp"
))]
#[inline]
fn syscall_error(status: u64) -> ProgramError {
    if status <= u32::MAX as u64 {
        ProgramError::Custom(status as u32)
    } else {
        ProgramError::InvalidArgument
    }
}

#[inline]
pub fn sha256(inputs: &[&[u8]]) -> Result<Sha256Hash, ProgramError> {
    if inputs.len() > MAX_HASH_SEGMENTS {
        return Err(ProgramError::InvalidArgument);
    }

    let mut result = [0u8; 32];
    // SAFETY: `inputs` is a valid slice of slice descriptors and `result`
    // points to exactly 32 writable output bytes.
    unsafe {
        crate::syscalls::sol_sha256(
            inputs as *const _ as *const u8,
            inputs.len() as u64,
            result.as_mut_ptr(),
        );
    }
    Ok(result)
}

#[inline]
pub fn sha256_single(input: &[u8]) -> Result<Sha256Hash, ProgramError> {
    sha256(&[input])
}

#[inline]
pub fn keccak256(inputs: &[&[u8]]) -> Result<Keccak256Hash, ProgramError> {
    if inputs.len() > MAX_HASH_SEGMENTS {
        return Err(ProgramError::InvalidArgument);
    }

    let mut result = [0u8; 32];
    // SAFETY: `inputs` is a valid slice of slice descriptors and `result`
    // points to exactly 32 writable output bytes.
    unsafe {
        crate::syscalls::sol_keccak256(
            inputs as *const _ as *const u8,
            inputs.len() as u64,
            result.as_mut_ptr(),
        );
    }
    Ok(result)
}

#[inline]
pub fn keccak256_single(input: &[u8]) -> Result<Keccak256Hash, ProgramError> {
    keccak256(&[input])
}

#[inline]
pub fn blake3(inputs: &[&[u8]]) -> Result<Blake3Hash, ProgramError> {
    if inputs.len() > MAX_HASH_SEGMENTS {
        return Err(ProgramError::InvalidArgument);
    }

    let mut result = [0u8; 32];
    // SAFETY: `inputs` is a valid slice of slice descriptors and `result`
    // points to exactly 32 writable output bytes.
    let rc = unsafe {
        crate::syscalls::sol_blake3(
            inputs as *const _ as *const u8,
            inputs.len() as u64,
            result.as_mut_ptr(),
        )
    };
    if rc != 0 {
        return Err(ProgramError::InvalidArgument);
    }
    Ok(result)
}

#[inline]
pub fn blake3_single(input: &[u8]) -> Result<Blake3Hash, ProgramError> {
    blake3(&[input])
}

#[inline]
pub fn secp256k1_recover(
    message_hash: &[u8; 32],
    recovery_id: u8,
    signature: &[u8; 64],
) -> Result<Secp256k1Pubkey, ProgramError> {
    let mut result = [0u8; 64];
    // SAFETY: all pointers refer to fixed-width buffers required by the
    // Solana secp256k1 recover syscall.
    let rc = unsafe {
        crate::syscalls::sol_secp256k1_recover(
            message_hash.as_ptr(),
            recovery_id as u64,
            signature.as_ptr(),
            result.as_mut_ptr(),
        )
    };
    if rc != 0 {
        return Err(ProgramError::InvalidArgument);
    }
    Ok(result)
}

#[inline]
pub fn recover_ethereum_address(
    message_hash: &[u8; 32],
    recovery_id: u8,
    signature: &[u8; 64],
) -> Result<EthereumAddress, ProgramError> {
    let pubkey = secp256k1_recover(message_hash, recovery_id, signature)?;
    let digest = keccak256(&[&pubkey])?;
    let mut address = [0u8; 20];
    address.copy_from_slice(&digest[12..32]);
    Ok(address)
}

#[inline]
pub fn curve_validate_point(curve_id: u64, point: &[u8; 32]) -> Result<bool, ProgramError> {
    // SAFETY: `point` points to exactly 32 bytes; null output pointer requests
    // validation-only behavior from Solana's curve syscall.
    let rc = unsafe {
        crate::syscalls::sol_curve_validate_point(curve_id, point.as_ptr(), core::ptr::null_mut())
    };
    Ok(rc == 0)
}

#[inline]
pub fn curve25519_edwards_validate_point(point: &[u8; 32]) -> Result<bool, ProgramError> {
    curve_validate_point(CURVE25519_EDWARDS, point)
}

#[cfg(feature = "crypto-curve")]
#[inline]
fn curve_group_op(
    curve_id: u64,
    group_op: u64,
    left: &CurvePoint,
    right: &CurvePoint,
) -> Result<CurvePoint, ProgramError> {
    let mut result = [0u8; 32];
    // SAFETY: all operands and the output are fixed-width 32-byte curve buffers.
    let rc = unsafe {
        crate::syscalls::sol_curve_group_op(
            curve_id,
            group_op,
            left.as_ptr(),
            right.as_ptr(),
            result.as_mut_ptr(),
        )
    };
    if rc != 0 {
        return Err(syscall_error(rc));
    }
    Ok(result)
}

#[cfg(feature = "crypto-curve")]
#[inline]
pub fn curve_group_add(
    curve_id: u64,
    left: &CurvePoint,
    right: &CurvePoint,
) -> Result<CurvePoint, ProgramError> {
    curve_group_op(curve_id, CURVE_GROUP_ADD, left, right)
}

#[cfg(feature = "crypto-curve")]
#[inline]
pub fn curve_group_sub(
    curve_id: u64,
    left: &CurvePoint,
    right: &CurvePoint,
) -> Result<CurvePoint, ProgramError> {
    curve_group_op(curve_id, CURVE_GROUP_SUB, left, right)
}

#[cfg(feature = "crypto-curve")]
#[inline]
pub fn curve_group_mul(
    curve_id: u64,
    scalar: &CurveScalar,
    point: &CurvePoint,
) -> Result<CurvePoint, ProgramError> {
    let mut result = [0u8; 32];
    // SAFETY: Solana's multiply op expects scalar left, point right, and a
    // fixed-width 32-byte result buffer.
    let rc = unsafe {
        crate::syscalls::sol_curve_group_op(
            curve_id,
            CURVE_GROUP_MUL,
            scalar.as_ptr(),
            point.as_ptr(),
            result.as_mut_ptr(),
        )
    };
    if rc != 0 {
        return Err(syscall_error(rc));
    }
    Ok(result)
}

#[cfg(feature = "crypto-curve")]
#[inline]
pub fn curve_multiscalar_mul(
    curve_id: u64,
    scalars: &[CurveScalar],
    points: &[CurvePoint],
) -> Result<CurvePoint, ProgramError> {
    if scalars.len() != points.len() || points.is_empty() {
        return Err(ProgramError::InvalidArgument);
    }

    let mut result = [0u8; 32];
    // SAFETY: `scalars` and `points` are contiguous arrays of 32-byte encodings
    // with matching lengths, and `result` is a 32-byte output buffer.
    let rc = unsafe {
        crate::syscalls::sol_curve_multiscalar_mul(
            curve_id,
            scalars.as_ptr() as *const u8,
            points.as_ptr() as *const u8,
            points.len() as u64,
            result.as_mut_ptr(),
        )
    };
    if rc != 0 {
        return Err(syscall_error(rc));
    }
    Ok(result)
}

#[cfg(feature = "crypto-poseidon")]
#[inline]
pub fn poseidon_hashv(
    parameters: u64,
    endianness: u64,
    inputs: &[&[u8]],
) -> Result<PoseidonHash, ProgramError> {
    if inputs.is_empty() || inputs.len() > MAX_POSEIDON_INPUTS {
        return Err(ProgramError::InvalidArgument);
    }
    if inputs.iter().any(|input| input.len() != POSEIDON_INPUT_LEN) {
        return Err(ProgramError::InvalidArgument);
    }

    let mut result = [0u8; 32];
    // SAFETY: `inputs` is a valid slice-descriptor array and `result` points to
    // exactly 32 writable bytes.
    let rc = unsafe {
        crate::syscalls::sol_poseidon(
            parameters,
            endianness,
            inputs as *const _ as *const u8,
            inputs.len() as u64,
            result.as_mut_ptr(),
        )
    };
    if rc != 0 {
        return Err(syscall_error(rc));
    }
    Ok(result)
}

#[cfg(feature = "crypto-poseidon")]
#[inline]
pub fn poseidon_hash(
    parameters: u64,
    endianness: u64,
    input: &[u8; 32],
) -> Result<PoseidonHash, ProgramError> {
    poseidon_hashv(parameters, endianness, &[input])
}

#[cfg(feature = "crypto-poseidon")]
#[inline]
pub fn poseidon_bn254_x5(inputs: &[&[u8]]) -> Result<PoseidonHash, ProgramError> {
    poseidon_hashv(POSEIDON_BN254_X5, POSEIDON_BIG_ENDIAN, inputs)
}

#[cfg(feature = "crypto-bn254")]
#[inline]
fn alt_bn128_group_op<const OUT: usize>(
    group_op: u64,
    input: &[u8],
) -> Result<[u8; OUT], ProgramError> {
    let mut result = [0u8; OUT];
    // SAFETY: callers choose `OUT` to match the selected BN254 syscall op.
    let rc = unsafe {
        crate::syscalls::sol_alt_bn128_group_op(
            group_op,
            input.as_ptr(),
            input.len() as u64,
            result.as_mut_ptr(),
        )
    };
    if rc != 0 {
        return Err(syscall_error(rc));
    }
    Ok(result)
}

#[cfg(feature = "crypto-bn254")]
#[inline]
fn alt_bn128_compression_op<const OUT: usize>(
    op: u64,
    input: &[u8],
) -> Result<[u8; OUT], ProgramError> {
    let mut result = [0u8; OUT];
    // SAFETY: callers choose `OUT` to match the selected BN254 compression op.
    let rc = unsafe {
        crate::syscalls::sol_alt_bn128_compression(
            op,
            input.as_ptr(),
            input.len() as u64,
            result.as_mut_ptr(),
        )
    };
    if rc != 0 {
        return Err(syscall_error(rc));
    }
    Ok(result)
}

#[cfg(feature = "crypto-bn254")]
#[inline]
pub fn alt_bn128_g1_addition_be(input: &[u8]) -> Result<AltBn128G1, ProgramError> {
    if input.len() > ALT_BN128_G1_ADDITION_INPUT_SIZE {
        return Err(ProgramError::InvalidArgument);
    }
    alt_bn128_group_op::<ALT_BN128_G1_POINT_SIZE>(ALT_BN128_G1_ADD_BE, input)
}

#[cfg(feature = "crypto-bn254")]
#[inline]
pub fn alt_bn128_g1_multiplication_be(input: &[u8]) -> Result<AltBn128G1, ProgramError> {
    if input.len() > ALT_BN128_G1_MULTIPLICATION_INPUT_SIZE {
        return Err(ProgramError::InvalidArgument);
    }
    alt_bn128_group_op::<ALT_BN128_G1_POINT_SIZE>(ALT_BN128_G1_MUL_BE, input)
}

#[cfg(feature = "crypto-bn254")]
#[inline]
pub fn alt_bn128_pairing_be(input: &[u8]) -> Result<AltBn128PairingResult, ProgramError> {
    if !input.len().is_multiple_of(ALT_BN128_PAIRING_ELEMENT_SIZE) {
        return Err(ProgramError::InvalidArgument);
    }
    alt_bn128_group_op::<ALT_BN128_FIELD_SIZE>(ALT_BN128_PAIRING_BE, input)
}

#[cfg(feature = "crypto-bn254")]
#[inline]
pub fn alt_bn128_add(input: &[u8]) -> Result<AltBn128G1, ProgramError> {
    alt_bn128_g1_addition_be(input)
}

#[cfg(feature = "crypto-bn254")]
#[inline]
pub fn alt_bn128_mul(input: &[u8]) -> Result<AltBn128G1, ProgramError> {
    alt_bn128_g1_multiplication_be(input)
}

#[cfg(feature = "crypto-bn254")]
#[inline]
pub fn alt_bn128_pairing(input: &[u8]) -> Result<AltBn128PairingResult, ProgramError> {
    alt_bn128_pairing_be(input)
}

#[cfg(feature = "crypto-bn254")]
#[inline]
pub fn alt_bn128_g1_compress_be(input: &AltBn128G1) -> Result<AltBn128G1Compressed, ProgramError> {
    alt_bn128_compression_op::<ALT_BN128_FIELD_SIZE>(ALT_BN128_G1_COMPRESS_BE, input)
}

#[cfg(feature = "crypto-bn254")]
#[inline]
pub fn alt_bn128_g1_decompress_be(
    input: &AltBn128G1Compressed,
) -> Result<AltBn128G1, ProgramError> {
    alt_bn128_compression_op::<ALT_BN128_G1_POINT_SIZE>(ALT_BN128_G1_DECOMPRESS_BE, input)
}

#[cfg(feature = "crypto-bn254")]
#[inline]
pub fn alt_bn128_g2_compress_be(input: &AltBn128G2) -> Result<AltBn128G2Compressed, ProgramError> {
    alt_bn128_compression_op::<ALT_BN128_G1_POINT_SIZE>(ALT_BN128_G2_COMPRESS_BE, input)
}

#[cfg(feature = "crypto-bn254")]
#[inline]
pub fn alt_bn128_g2_decompress_be(
    input: &AltBn128G2Compressed,
) -> Result<AltBn128G2, ProgramError> {
    alt_bn128_compression_op::<ALT_BN128_G2_POINT_SIZE>(ALT_BN128_G2_DECOMPRESS_BE, input)
}

#[cfg(feature = "crypto-big-mod-exp")]
#[inline]
pub fn big_mod_exp(
    base: &[u8],
    exponent: &[u8],
    modulus: &[u8],
    output: &mut [u8],
) -> Result<(), ProgramError> {
    if modulus.is_empty() || output.len() != modulus.len() {
        return Err(ProgramError::InvalidArgument);
    }

    let params = BigModExpParams {
        base: base.as_ptr(),
        base_len: base.len() as u64,
        exponent: exponent.as_ptr(),
        exponent_len: exponent.len() as u64,
        modulus: modulus.as_ptr(),
        modulus_len: modulus.len() as u64,
    };
    // SAFETY: params has Solana's C layout and `output` has exactly modulus.len()
    // writable bytes, which is the syscall's output size contract.
    let rc = unsafe {
        crate::syscalls::sol_big_mod_exp(
            &params as *const BigModExpParams as *const u8,
            output.as_mut_ptr(),
        )
    };
    if rc != 0 {
        return Err(syscall_error(rc));
    }
    Ok(())
}

#[inline(always)]
pub fn get_stack_height() -> u64 {
    crate::syscalls::sol_get_stack_height()
}

#[inline(always)]
pub fn is_top_level() -> bool {
    get_stack_height() <= 1
}

#[inline(always)]
pub fn is_cpi() -> bool {
    get_stack_height() > 1
}

#[inline(always)]
pub fn require_top_level() -> Result<(), ProgramError> {
    if is_top_level() {
        Ok(())
    } else {
        Err(ProgramError::InvalidArgument)
    }
}

#[inline]
pub fn get_processed_instruction(index: u64) -> Option<ProcessedInstruction> {
    let mut meta = ProcessedInstructionMeta {
        data_len: MAX_INSTRUCTION_DATA_LEN as u64,
        accounts_len: (MAX_INSTRUCTION_ACCOUNTS_BYTES / 34) as u64,
    };
    let mut program_id = Address::default();
    let mut data = [0u8; MAX_INSTRUCTION_DATA_LEN];
    let mut accounts = [0u8; MAX_INSTRUCTION_ACCOUNTS_BYTES];

    // SAFETY: output pointers refer to writable stack buffers sized for the
    // processed-sibling-instruction syscall contract.
    let rc = unsafe {
        crate::syscalls::sol_get_processed_sibling_instruction(
            index,
            &mut meta as *mut ProcessedInstructionMeta as *mut u8,
            program_id.as_mut().as_mut_ptr(),
            data.as_mut_ptr(),
            accounts.as_mut_ptr(),
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

#[inline]
pub fn get_processed_instruction_data<const MAX_DATA: usize>(
    index: u64,
) -> Option<ProcessedInstructionData<MAX_DATA>> {
    let mut meta = ProcessedInstructionMeta {
        data_len: MAX_DATA as u64,
        accounts_len: 0,
    };
    let mut program_id = Address::default();
    let mut data = [0u8; MAX_DATA];
    let mut accounts = [0u8; 0];

    // SAFETY: output pointers refer to writable buffers. The account-meta
    // capacity is advertised as zero because callers of this helper only need
    // program id and instruction data.
    let rc = unsafe {
        crate::syscalls::sol_get_processed_sibling_instruction(
            index,
            &mut meta as *mut ProcessedInstructionMeta as *mut u8,
            program_id.as_mut().as_mut_ptr(),
            data.as_mut_ptr(),
            accounts.as_mut_ptr(),
        )
    };
    if rc != 0 {
        return None;
    }

    Some(ProcessedInstructionData {
        program_id,
        data,
        data_len: meta.data_len as usize,
    })
}

#[inline]
pub fn require_ed25519_instruction(
    sibling_index: u64,
) -> Result<ProcessedInstruction, ProgramError> {
    let instruction =
        get_processed_instruction(sibling_index).ok_or(ProgramError::InvalidArgument)?;
    if instruction.program_id != ED25519_PROGRAM_ID {
        return Err(ProgramError::IncorrectProgramId);
    }
    Ok(instruction)
}

#[inline]
pub fn require_ed25519_instruction_data<const MAX_DATA: usize>(
    sibling_index: u64,
) -> Result<ProcessedInstructionData<MAX_DATA>, ProgramError> {
    let instruction =
        get_processed_instruction_data(sibling_index).ok_or(ProgramError::InvalidArgument)?;
    if instruction.program_id != ED25519_PROGRAM_ID {
        return Err(ProgramError::IncorrectProgramId);
    }
    Ok(instruction)
}

#[inline]
pub fn require_secp256k1_instruction(
    sibling_index: u64,
) -> Result<ProcessedInstruction, ProgramError> {
    let instruction =
        get_processed_instruction(sibling_index).ok_or(ProgramError::InvalidArgument)?;
    if instruction.program_id != SECP256K1_PROGRAM_ID {
        return Err(ProgramError::IncorrectProgramId);
    }
    Ok(instruction)
}

#[cfg(test)]
mod tests {
    use super::*;

    const EMPTY: &[u8] = b"";

    #[test]
    fn hash_helpers_accept_sixteen_segments() {
        let inputs = [EMPTY; MAX_HASH_SEGMENTS];

        assert!(sha256(&inputs).is_ok());
        assert!(keccak256(&inputs).is_ok());
        assert!(blake3(&inputs).is_ok());
    }

    #[test]
    fn hash_helpers_reject_more_than_sixteen_segments() {
        let inputs = [EMPTY; MAX_HASH_SEGMENTS + 1];

        assert_eq!(sha256(&inputs), Err(ProgramError::InvalidArgument));
        assert_eq!(keccak256(&inputs), Err(ProgramError::InvalidArgument));
        assert_eq!(blake3(&inputs), Err(ProgramError::InvalidArgument));
    }

    #[cfg(feature = "crypto-curve")]
    #[test]
    fn curve_msm_requires_matching_nonempty_inputs() {
        let scalars = [[0u8; 32]; 2];
        let points = [[0u8; 32]; 1];

        assert_eq!(
            curve_multiscalar_mul(CURVE25519_EDWARDS, &scalars, &points),
            Err(ProgramError::InvalidArgument)
        );
        assert_eq!(
            curve_multiscalar_mul(CURVE25519_EDWARDS, &[], &[]),
            Err(ProgramError::InvalidArgument)
        );
    }

    #[cfg(feature = "crypto-poseidon")]
    #[test]
    fn poseidon_rejects_bad_input_shape() {
        let short = [0u8; 31];
        let input = [0u8; 32];
        let too_many = [&input[..]; MAX_POSEIDON_INPUTS + 1];

        assert_eq!(
            poseidon_hashv(POSEIDON_BN254_X5, POSEIDON_BIG_ENDIAN, &[]),
            Err(ProgramError::InvalidArgument)
        );
        assert_eq!(
            poseidon_hashv(POSEIDON_BN254_X5, POSEIDON_BIG_ENDIAN, &[&short]),
            Err(ProgramError::InvalidArgument)
        );
        assert_eq!(
            poseidon_hashv(POSEIDON_BN254_X5, POSEIDON_BIG_ENDIAN, &too_many),
            Err(ProgramError::InvalidArgument)
        );
    }

    #[cfg(feature = "crypto-bn254")]
    #[test]
    fn bn254_rejects_bad_input_lengths() {
        let oversized_add = [0u8; ALT_BN128_G1_ADDITION_INPUT_SIZE + 1];
        let bad_pairing = [0u8; ALT_BN128_PAIRING_ELEMENT_SIZE + 1];

        assert_eq!(
            alt_bn128_g1_addition_be(&oversized_add),
            Err(ProgramError::InvalidArgument)
        );
        assert_eq!(
            alt_bn128_pairing_be(&bad_pairing),
            Err(ProgramError::InvalidArgument)
        );
    }

    #[cfg(feature = "crypto-big-mod-exp")]
    #[test]
    fn big_mod_exp_requires_output_matching_modulus() {
        let mut output = [0u8; 1];
        let mut empty_output = [];

        assert_eq!(
            big_mod_exp(&[1], &[1], &[1, 2], &mut output),
            Err(ProgramError::InvalidArgument)
        );
        assert_eq!(
            big_mod_exp(&[1], &[1], &[], &mut empty_output),
            Err(ProgramError::InvalidArgument)
        );
    }
}
