//! Runtime cryptography helpers built on Solana syscalls and precompiles.

use crate::{Address, ProgramError};

pub type Sha256Hash = [u8; 32];
pub type Keccak256Hash = [u8; 32];

pub const CURVE25519_EDWARDS: u64 = 0;
pub const MAX_INSTRUCTION_DATA_LEN: usize = 1232;
pub const MAX_INSTRUCTION_ACCOUNTS_BYTES: usize = 2176;

pub const ED25519_PROGRAM_ID: Address = Address::new_from_array(
    crate::__five8_const::decode_32_const("Ed25519SigVerify111111111111111111111111111"),
);

pub const SECP256K1_PROGRAM_ID: Address = Address::new_from_array(
    crate::__five8_const::decode_32_const("KeccakSecp256k11111111111111111111111111111"),
);

#[derive(Clone, Debug)]
pub struct ProcessedInstruction {
    pub program_id: Address,
    pub data: [u8; MAX_INSTRUCTION_DATA_LEN],
    pub data_len: usize,
    pub accounts_len: usize,
}

#[repr(C)]
struct ProcessedInstructionMeta {
    data_len: u64,
    accounts_len: u64,
}

#[inline]
pub fn sha256(inputs: &[&[u8]]) -> Result<Sha256Hash, ProgramError> {
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
