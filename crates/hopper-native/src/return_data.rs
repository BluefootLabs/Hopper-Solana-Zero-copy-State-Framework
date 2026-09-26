//! CPI return data retrieval and typed deserialization.
//!
//! The Solana runtime supports return data from CPI calls (up to 1024 bytes).
//! This module combines invocation, program-id validation, and typed return-data
//! decoding.

use crate::address::Address;
use crate::error::ProgramError;
use crate::project::Projectable;
use core::mem::MaybeUninit;

#[cfg(feature = "cpi")]
use crate::instruction::{InstructionView, Signer};

/// Maximum return data size (1 KiB), matching Solana runtime limit.
pub const MAX_RETURN_DATA: usize = 1024;

/// Return data from a previous CPI call.
///
/// The buffer is deliberately left uninitialized until the
/// `sol_get_return_data` syscall fills it; only the syscall-initialized
/// prefix (`len` bytes) is ever exposed to callers.
pub struct ReturnData {
    /// Buffer holding the return data (stack-allocated; only the first
    /// `len` bytes are initialized).
    buf: [MaybeUninit<u8>; MAX_RETURN_DATA],
    /// Actual length of the return data.
    len: usize,
    /// Program ID that set the return data.
    program_id: Address,
}

impl ReturnData {
    /// Get the return data bytes.
    #[inline(always)]
    pub fn data(&self) -> &[u8] {
        // Fail-closed backstop for the invariant the SAFETY comment relies
        // on: `len` can never exceed the buffer capacity.
        debug_assert!(self.len <= MAX_RETURN_DATA);
        // SAFETY: `sol_get_return_data` initializes exactly
        // `min(actual_len, MAX_RETURN_DATA)` bytes of the buffer it was
        // handed, and `get_return_data` sets `len` to that same value (the
        // test constructor likewise writes `len` bytes before setting it), so
        // the first `len` bytes are always initialized `u8`s.
        unsafe { core::slice::from_raw_parts(self.buf.as_ptr() as *const u8, self.len) }
    }

    /// Get the program that set the return data.
    #[inline(always)]
    pub fn program_id(&self) -> &Address {
        &self.program_id
    }

    /// Length of the return data.
    #[inline(always)]
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether the return data is empty.
    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Interpret the return data as a `Projectable` type.
    ///
    /// Returns `Err(AccountDataTooSmall)` if the return data is smaller
    /// than `size_of::<T>()`.
    #[inline]
    pub fn as_type<T: Projectable>(&self) -> Result<&T, ProgramError> {
        let size = core::mem::size_of::<T>();
        if self.len < size {
            return Err(ProgramError::AccountDataTooSmall);
        }

        let data = self.data();
        let align = core::mem::align_of::<T>();
        let ptr = data.as_ptr();
        if !(ptr as usize).is_multiple_of(align) {
            return Err(ProgramError::InvalidAccountData);
        }

        // SAFETY: `data` is the initialized `len`-byte prefix of the buffer,
        // the length check above guarantees `len >= size_of::<T>()`, the
        // alignment check guarantees `ptr` is aligned for `T`, and
        // `T: Projectable` is valid for any initialized bit pattern.
        Ok(unsafe { &*(ptr as *const T) })
    }

    /// Read a typed prefix only when the expected program produced this data.
    /// Nested CPIs can leave a different producer's return data behind.
    /// The application remains responsible for the payload's business rules.
    #[inline]
    pub fn as_type_from<T: Projectable>(
        &self,
        expected_program: &Address,
    ) -> Result<&T, ProgramError> {
        if !crate::address::address_eq(self.program_id(), expected_program) {
            return Err(ProgramError::IncorrectProgramId);
        }
        self.as_type::<T>()
    }

    /// Read a u64 from the first 8 bytes of return data.
    #[inline]
    pub fn as_u64(&self) -> Result<u64, ProgramError> {
        if self.len < 8 {
            return Err(ProgramError::AccountDataTooSmall);
        }
        let mut bytes = [0u8; 8];
        bytes.copy_from_slice(&self.data()[..8]);
        Ok(u64::from_le_bytes(bytes))
    }

    /// Read a u32 from the first 4 bytes of return data.
    #[inline]
    pub fn as_u32(&self) -> Result<u32, ProgramError> {
        if self.len < 4 {
            return Err(ProgramError::AccountDataTooSmall);
        }
        let mut bytes = [0u8; 4];
        bytes.copy_from_slice(&self.data()[..4]);
        Ok(u32::from_le_bytes(bytes))
    }
}

/// Retrieve return data from the most recent CPI call.
///
/// Returns `None` if no return data was set (length == 0).
///
/// Only the initialized prefix reported by the syscall is exposed. Empty
/// return data yields `None`; accessors never read the uninitialized remainder.
/// This raw snapshot does not authenticate a producer. Use `as_type_from` or
/// `invoke_and_read` when interpreting a particular program's result.
#[inline]
pub fn get_return_data() -> Option<ReturnData> {
    #[allow(unused_mut)]
    let mut rd = ReturnData {
        buf: [const { MaybeUninit::uninit() }; MAX_RETURN_DATA],
        len: 0,
        program_id: Address::default(),
    };

    #[cfg(target_os = "solana")]
    {
        // SAFETY: The buffer and program-id pointers are stack-allocated with
        // the exact capacities advertised to the runtime syscall; the buffer
        // may be uninitialized because the syscall only writes (never reads)
        // it.
        let actual_len = unsafe {
            crate::syscalls::sol_get_return_data(
                rd.buf.as_mut_ptr() as *mut u8,
                MAX_RETURN_DATA as u64,
                rd.program_id.0.as_mut_ptr(),
            )
        };
        rd.len = (actual_len as usize).min(MAX_RETURN_DATA);
    }

    #[cfg(not(target_os = "solana"))]
    {
        // Off-chain: no return data available; `len` stays 0 so the
        // uninitialized buffer is discarded below without being read.
    }

    if rd.len == 0 {
        None
    } else {
        Some(rd)
    }
}

/// Invoke a CPI and capture a producer-checked, type-validated return snapshot.
///
/// The producing program must equal `instruction.program_id`. The snapshot
/// must contain an aligned `T` prefix; trailing bytes are permitted, matching
/// `ReturnData::as_type`. No data is `InvalidAccountData`, a different producer
/// is `IncorrectProgramId`, and a short result is `AccountDataTooSmall`.
/// Call `as_type::<T>()` on the returned snapshot to borrow the value.
///
/// ```ignore
/// let snapshot = invoke_and_read::<PriceData, 2>(&instruction, &accounts, &[])?;
/// let oracle_price = snapshot.as_type::<PriceData>()?;
/// ```
#[cfg(feature = "cpi")]
#[inline]
pub fn invoke_and_read<T: Projectable, const ACCOUNTS: usize>(
    instruction: &InstructionView<'_, '_, '_, '_>,
    account_views: &[&crate::account_view::AccountView<'_>; ACCOUNTS],
    signers_seeds: &[Signer<'_, '_>],
) -> Result<ReturnData, ProgramError> {
    crate::cpi::invoke_signed::<ACCOUNTS>(instruction, account_views, signers_seeds)?;

    let returned = get_return_data().ok_or(ProgramError::InvalidAccountData)?;
    returned.as_type_from::<T>(instruction.program_id)?;
    Ok(returned)
}

#[cfg(test)]
impl ReturnData {
    /// Test-only constructor: builds a snapshot whose buffer prefix is fully
    /// initialized from `bytes`, mirroring what the syscall produces on-chain.
    fn test_snapshot(bytes: &[u8], program_id: Address) -> Self {
        assert!(bytes.len() <= MAX_RETURN_DATA);
        let mut buf = [const { MaybeUninit::uninit() }; MAX_RETURN_DATA];
        for (dst, src) in buf.iter_mut().zip(bytes) {
            dst.write(*src);
        }
        ReturnData {
            buf,
            len: bytes.len(),
            program_id,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_return_requires_the_expected_producer_and_initialized_type() {
        let expected = Address::new_from_array([1; 32]);
        let nested = Address::new_from_array([2; 32]);
        let correct = ReturnData::test_snapshot(&42u64.to_le_bytes(), expected.clone());
        assert_eq!(*correct.as_type_from::<u64>(&expected).unwrap(), 42);
        assert_eq!(
            correct.as_type_from::<u64>(&nested),
            Err(ProgramError::IncorrectProgramId)
        );
        let short = ReturnData::test_snapshot(&[42], expected.clone());
        assert_eq!(
            short.as_type_from::<u64>(&expected),
            Err(ProgramError::AccountDataTooSmall)
        );
        let forwarded = ReturnData::test_snapshot(&42u64.to_le_bytes(), nested);
        assert_eq!(
            forwarded.as_type_from::<u64>(&expected),
            Err(ProgramError::IncorrectProgramId)
        );
    }

    #[test]
    fn offchain_get_return_data_is_none() {
        assert!(get_return_data().is_none());
    }

    #[test]
    fn data_exposes_exactly_the_written_prefix() {
        let payload = [0xAB, 0xCD, 0xEF];
        let rd = ReturnData::test_snapshot(&payload, Address::default());
        assert_eq!(rd.data(), &payload);
        assert_eq!(rd.len(), payload.len());
        assert!(!rd.is_empty());
    }

    #[test]
    fn as_u64_and_as_u32_never_read_past_the_prefix() {
        let short = ReturnData::test_snapshot(&[1, 2, 3], Address::default());
        assert!(short.as_u64().is_err());
        assert!(short.as_u32().is_err());

        let rd = ReturnData::test_snapshot(&7u64.to_le_bytes(), Address::default());
        assert_eq!(rd.as_u64().unwrap(), 7);
        assert_eq!(rd.as_u32().unwrap(), 7);
    }

    #[test]
    fn as_type_length_checks_against_the_prefix() {
        let rd = ReturnData::test_snapshot(&[5u8], Address::default());
        assert!(rd.as_type::<u64>().is_err());
        assert_eq!(*rd.as_type::<u8>().unwrap(), 5);
    }
}
