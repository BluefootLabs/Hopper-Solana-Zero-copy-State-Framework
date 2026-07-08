//! CPI return data retrieval and typed deserialization.
//!
//! The Solana runtime supports return data from CPI calls (up to 1024 bytes).
//! No framework provides a typed wrapper that combines invoke + deserialize
//! in one step. Hopper does.

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
        // SAFETY: `sol_get_return_data` initializes exactly
        // `min(actual_len, MAX_RETURN_DATA)` bytes of the buffer it was
        // handed, and `get_return_data` sets `len` to that same value (the
        // test constructor likewise writes `len` bytes before setting it), so
        // the first `len` bytes are always initialized `u8`s.
        // Fail-closed backstop for the invariant the SAFETY comment relies
        // on: `len` can never exceed the buffer capacity.
        debug_assert!(self.len <= MAX_RETURN_DATA);
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
/// The 1 KiB buffer is *not* zero-filled before the syscall — the syscall
/// initializes exactly the reported prefix, and `None` is returned before any
/// read when the length is 0. This is the bug class behind Quasar #238/#234
/// (an `assume_init` over a buffer the syscall never wrote, exposing
/// uninitialized stack bytes as return data); Hopper's shape is immune
/// because uninitialized bytes can never escape: empty return data (including
/// the off-chain path, where `len` stays 0) short-circuits to `None`, and
/// every accessor reads only the syscall-initialized `len`-byte prefix.
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

/// Invoke a CPI and immediately read back typed return data.
///
/// Combines `invoke_signed` + `get_return_data` + `as_type::<T>()` into
/// a single operation. This is the cleanest way to call a program that
/// returns structured data.
///
/// # Example
///
/// ```ignore
/// let oracle_price: &PriceData = invoke_and_read::<PriceData, 2>(
///     &instruction,
///     &[&oracle_program, &price_feed],
///     &[],
/// )?;
/// ```
#[cfg(feature = "cpi")]
#[inline]
pub fn invoke_and_read<T: Projectable, const ACCOUNTS: usize>(
    instruction: &InstructionView<'_, '_, '_, '_>,
    account_views: &[&crate::account_view::AccountView<'_>; ACCOUNTS],
    signers_seeds: &[Signer<'_, '_>],
) -> Result<ReturnData, ProgramError> {
    crate::cpi::invoke_signed::<ACCOUNTS>(instruction, account_views, signers_seeds)?;

    get_return_data().ok_or(ProgramError::InvalidAccountData)
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
