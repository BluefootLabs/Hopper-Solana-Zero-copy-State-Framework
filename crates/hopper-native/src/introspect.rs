//! Instruction introspection -- stack height and sibling instruction access.
//!
//! These wrappers support security patterns based on transaction and call-stack
//! introspection:
//!
//! - **CPI guard**: Detect if the current instruction is running inside a CPI
//!   call (stack height > 1). Prevents unauthorized composition -- e.g., a
//!   governance instruction that must be top-level only.
//!
//! - **Precompile inspection**: Read a previous sibling's program ID and data.
//!   Program-ID checks alone do not authorize an action. Validate signature
//!   count, offsets, referenced instruction bytes, and the expected key/message.
//!
//! - **Secp256k1 recovery**: Same pattern for Ethereum-compatible signatures.
//!
//! Hopper wraps these syscalls behind small typed helpers so programs do not
//! need to repeat raw unsafe glue at every call site.

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
        // SAFETY: The syscall takes no pointer and has no memory
        // precondition.
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

/// Account metadata returned by the sibling-instruction syscall.
///
/// This is an owned-address record, unlike CPI's pointer-based metadata.
#[repr(C)]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProcessedInstructionAccount {
    pub address: Address,
    pub is_signer: bool,
    pub is_writable: bool,
}

const _: () = {
    assert!(core::mem::size_of::<ProcessedInstructionAccount>() == 34);
    assert!(core::mem::align_of::<ProcessedInstructionAccount>() == 1);
    assert!(core::mem::offset_of!(ProcessedInstructionAccount, address) == 0);
    assert!(core::mem::offset_of!(ProcessedInstructionAccount, is_signer) == 32);
    assert!(core::mem::offset_of!(ProcessedInstructionAccount, is_writable) == 33);
};

/// A processed sibling copied into caller-owned scratch buffers.
///
/// The slices expose only the initialized instruction prefixes. This describes
/// an instruction in the runtime trace; it does not prove a token balance change,
/// validate a signature payload, or replace application authorization.
#[derive(Debug)]
pub struct ProcessedInstructionView<'a> {
    pub program_id: Address,
    pub data: &'a [u8],
    pub accounts: &'a [ProcessedInstructionAccount],
}

/// Read a processed sibling without heap allocation or fixed-size scratch space.
///
/// Index zero is the most recent sibling at the current call depth and caller.
/// Parents and children are not siblings. The first syscall queries exact lengths;
/// the second copies only when both caller buffers fit. Returns `Ok(None)` for
/// absence, or `AccountDataTooSmall` for insufficient capacity, never truncation.
/// No syscall result is treated as an ordinary zero-success program error code.
///
/// On host targets there is no instruction trace, so this returns `Ok(None)`.
/// Test execution history in an SVM or on a cluster. Use the Instructions sysvar
/// to inspect the transaction-level list by absolute index, especially for
/// precompile signature payloads and their cross-instruction references.
#[inline]
pub fn get_processed_instruction_into<'a>(
    index: u64,
    data: &'a mut [u8],
    accounts: &'a mut [ProcessedInstructionAccount],
) -> Result<Option<ProcessedInstructionView<'a>>, ProgramError> {
    read_processed_with(index, data, accounts, sibling_syscall)
}

fn sibling_syscall(
    index: u64,
    meta: &mut ProcessedInstructionMeta,
    program: &mut Address,
    data: &mut [u8],
    accounts: &mut [ProcessedInstructionAccount],
) -> u64 {
    #[cfg(target_os = "solana")]
    {
        // SAFETY: the private reader advertises only initialized buffer prefixes
        // that fit these disjoint outputs. Metadata, program and account records
        // have the runtime's checked C layout; the syscall writes valid bools.
        unsafe {
            crate::syscalls::sol_get_processed_sibling_instruction(
                index,
                meta as *mut _ as *mut u8,
                program.0.as_mut_ptr(),
                data.as_mut_ptr(),
                accounts.as_mut_ptr().cast(),
            )
        }
    }
    #[cfg(not(target_os = "solana"))]
    {
        let _ = (index, meta, program, data, accounts);
        0
    }
}

fn read_processed_with<'a>(
    index: u64,
    data: &'a mut [u8],
    accounts: &'a mut [ProcessedInstructionAccount],
    mut syscall: impl FnMut(
        u64,
        &mut ProcessedInstructionMeta,
        &mut Address,
        &mut [u8],
        &mut [ProcessedInstructionAccount],
    ) -> u64,
) -> Result<Option<ProcessedInstructionView<'a>>, ProgramError> {
    let mut meta = ProcessedInstructionMeta {
        data_len: 0,
        accounts_len: 0,
    };
    let mut program_id = Address::default();
    // Real writable scratch also handles a zero-length sibling during the probe.
    let mut probe_data = [0];
    let mut probe_accounts = [ProcessedInstructionAccount::default()];
    match syscall(
        index,
        &mut meta,
        &mut program_id,
        &mut probe_data,
        &mut probe_accounts,
    ) {
        0 => return Ok(None),
        1 => {}
        _ => return Err(ProgramError::InvalidAccountData),
    }
    let data_len = usize::try_from(meta.data_len).map_err(|_| ProgramError::AccountDataTooSmall)?;
    let accounts_len =
        usize::try_from(meta.accounts_len).map_err(|_| ProgramError::AccountDataTooSmall)?;
    if data_len > data.len() || accounts_len > accounts.len() {
        return Err(ProgramError::AccountDataTooSmall);
    }
    let rc = syscall(
        index,
        &mut meta,
        &mut program_id,
        &mut data[..data_len],
        &mut accounts[..accounts_len],
    );
    if rc != 1 || meta.data_len != data_len as u64 || meta.accounts_len != accounts_len as u64 {
        return Err(ProgramError::InvalidAccountData);
    }
    Ok(Some(ProcessedInstructionView {
        program_id,
        data: &data[..data_len],
        accounts: &accounts[..accounts_len],
    }))
}

/// Convenience reader for up to 1,232 data bytes and 64 account metas.
///
/// Returns `None` for absence or insufficient capacity. Prefer
/// [`get_processed_instruction_into`] to select your own scratch budget, inspect
/// account metadata, and distinguish missing siblings from capacity errors.
#[inline]
pub fn get_processed_instruction(index: u64) -> Option<ProcessedInstruction> {
    let mut data = [0; 1232];
    let mut accounts = core::array::from_fn::<_, 64, _>(|_| ProcessedInstructionAccount::default());
    let view = get_processed_instruction_into(index, &mut data, &mut accounts).ok()??;
    let program_id = view.program_id;
    let data_len = view.data.len();
    let accounts_len = view.accounts.len();
    Some(ProcessedInstruction {
        program_id,
        data,
        data_len,
        accounts_len,
    })
}

/// Well-known precompile address for Ed25519 signature verification.
pub const ED25519_PROGRAM_ID: Address =
    crate::address!("Ed25519SigVerify111111111111111111111111111");

/// Well-known precompile address for Secp256k1 signature recovery.
pub const SECP256K1_PROGRAM_ID: Address =
    crate::address!("KeccakSecp256k11111111111111111111111111111");

/// Well-known precompile address for Secp256r1 (P-256) signature
/// verification (SIMD-0075). This is the precompile that backs passkey /
/// WebAuthn signature checks on Solana.
pub const SECP256R1_PROGRAM_ID: Address =
    crate::address!("Secp256r1SigVerify1111111111111111111111111");

/// Check that a previous sibling instruction was to the Ed25519 precompile.
///
/// Checks only the program ID. The caller must validate signature count, offsets,
/// referenced instruction bytes, and the expected public key and message. Use
/// the Instructions sysvar for transaction-level cross-instruction references.
///
/// `sibling_index` is 0 for the most recent sibling, 1 for the one before, etc.
#[inline]
pub fn require_ed25519_instruction(
    sibling_index: u64,
) -> Result<ProcessedInstruction, ProgramError> {
    let ix = get_processed_instruction(sibling_index).ok_or(ProgramError::InvalidArgument)?;

    if !crate::address::address_eq(&ix.program_id, &ED25519_PROGRAM_ID) {
        return Err(ProgramError::IncorrectProgramId);
    }

    Ok(ix)
}

/// Check that a previous sibling instruction was to the Secp256k1 precompile.
/// Checks only the program ID, not payload validity or application authorization.
#[inline]
pub fn require_secp256k1_instruction(
    sibling_index: u64,
) -> Result<ProcessedInstruction, ProgramError> {
    let ix = get_processed_instruction(sibling_index).ok_or(ProgramError::InvalidArgument)?;

    if !crate::address::address_eq(&ix.program_id, &SECP256K1_PROGRAM_ID) {
        return Err(ProgramError::IncorrectProgramId);
    }

    Ok(ix)
}

/// Check that a previous sibling instruction was to the Secp256r1
/// (P-256) precompile, the verification path for passkeys / WebAuthn.
///
/// Checks only the program ID. The caller must validate the signature payload,
/// cross-instruction offsets, expected key/message and application authorization.
/// This helper does not validate a WebAuthn challenge or relying-party policy.
///
/// `sibling_index` is 0 for the most recent sibling, 1 for the one
/// before, etc.
#[inline]
pub fn require_secp256r1_instruction(
    sibling_index: u64,
) -> Result<ProcessedInstruction, ProgramError> {
    let ix = get_processed_instruction(sibling_index).ok_or(ProgramError::InvalidArgument)?;

    if !crate::address::address_eq(&ix.program_id, &SECP256R1_PROGRAM_ID) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absence_does_not_fabricate_an_instruction_or_touch_outputs() {
        let mut data = [0xa5; 8];
        let mut accounts = [ProcessedInstructionAccount::default()];
        let mut calls = 0;
        let result = read_processed_with(7, &mut data, &mut accounts, |index, _, _, _, _| {
            assert_eq!(index, 7);
            calls += 1;
            0
        })
        .unwrap();
        assert!(result.is_none());
        assert_eq!(calls, 1);
        assert_eq!(data, [0xa5; 8]);
        assert_eq!(accounts, [ProcessedInstructionAccount::default()]);
        assert!(get_processed_instruction(0).is_none());
    }

    #[test]
    fn probes_then_copies_exact_lengths_and_preserves_unused_capacity() {
        let mut data = [0xa5; 8];
        let mut accounts =
            core::array::from_fn::<_, 3, _>(|_| ProcessedInstructionAccount::default());
        let expected = ProcessedInstructionAccount {
            address: Address::new_from_array([9; 32]),
            is_signer: true,
            is_writable: false,
        };
        let mut calls = 0;
        let view = read_processed_with(
            2,
            &mut data,
            &mut accounts,
            |index, meta, program, bytes, metas| {
                assert_eq!(index, 2);
                calls += 1;
                if calls == 1 {
                    assert_eq!((meta.data_len, meta.accounts_len), (0, 0));
                    meta.data_len = 3;
                    meta.accounts_len = 1;
                } else {
                    assert_eq!((meta.data_len, meta.accounts_len), (3, 1));
                    assert_eq!((bytes.len(), metas.len()), (3, 1));
                    *program = Address::new_from_array([7; 32]);
                    bytes.copy_from_slice(&[4, 5, 6]);
                    metas[0] = expected.clone();
                }
                1
            },
        )
        .unwrap()
        .unwrap();
        assert_eq!(calls, 2);
        assert_eq!(view.program_id, Address::new_from_array([7; 32]));
        assert_eq!(view.data, &[4, 5, 6]);
        assert_eq!(view.accounts, &[expected]);
        assert_eq!(&data[3..], &[0xa5; 5]);
        assert_eq!(
            &accounts[1..],
            &[
                ProcessedInstructionAccount::default(),
                ProcessedInstructionAccount::default()
            ]
        );
    }

    #[test]
    fn insufficient_buffers_are_rejected_before_copy() {
        for (data_len, accounts_len) in [(9, 1), (3, 2), (u64::MAX, 0), (0, u64::MAX)] {
            let mut calls = 0;
            let mut data = [0xa5; 8];
            let mut accounts = [ProcessedInstructionAccount::default()];
            let result = read_processed_with(0, &mut data, &mut accounts, |_, meta, _, _, _| {
                calls += 1;
                meta.data_len = data_len;
                meta.accounts_len = accounts_len;
                1
            });
            assert_eq!(result.unwrap_err(), ProgramError::AccountDataTooSmall);
            assert_eq!(calls, 1);
            assert_eq!(data, [0xa5; 8]);
        }
    }

    #[test]
    fn zero_length_sibling_is_distinct_from_absence() {
        let mut calls = 0;
        let view = read_processed_with(0, &mut [], &mut [], |_, meta, program, _, _| {
            calls += 1;
            assert_eq!((meta.data_len, meta.accounts_len), (0, 0));
            *program = Address::new_from_array([8; 32]);
            1
        })
        .unwrap()
        .unwrap();
        assert_eq!(calls, 2);
        assert_eq!(view.program_id, Address::new_from_array([8; 32]));
        assert!(view.data.is_empty() && view.accounts.is_empty());
    }

    #[test]
    fn unexpected_return_or_changing_lengths_fail_closed() {
        for (probe_rc, copy_rc, change_lengths) in
            [(2, 1, false), (1, 0, false), (1, 2, false), (1, 1, true)]
        {
            let mut calls = 0;
            let mut data = [0; 8];
            let mut accounts = [ProcessedInstructionAccount::default()];
            let result = read_processed_with(0, &mut data, &mut accounts, |_, meta, _, _, _| {
                calls += 1;
                if calls == 1 {
                    meta.data_len = 3;
                    probe_rc
                } else {
                    if change_lengths {
                        meta.data_len = 4;
                    }
                    copy_rc
                }
            });
            assert_eq!(result.unwrap_err(), ProgramError::InvalidAccountData);
        }
    }
}
