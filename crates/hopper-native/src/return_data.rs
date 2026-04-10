//! CPI return data retrieval and typed deserialization.
//!
//! The Solana runtime supports return data from CPI calls (up to 1024 bytes).
//! No framework provides a typed wrapper that combines invoke + deserialize
//! in one step. Hopper does.

use crate::address::Address;
use crate::error::ProgramError;
use crate::project::Projectable;

#[cfg(feature = "cpi")]
use crate::instruction::{InstructionView, Signer};

/// Maximum return data size (1 KiB), matching Solana runtime limit.
pub const MAX_RETURN_DATA: usize = 1024;

/// Return data from a previous CPI call.
pub struct ReturnData {
    /// Buffer holding the return data (stack-allocated).
    buf: [u8; MAX_RETURN_DATA],
    /// Actual length of the return data.
    len: usize,
    /// Program ID that set the return data.
    program_id: Address,
}

impl ReturnData {
    /// Get the return data bytes.
    #[inline(always)]
    pub fn data(&self) -> &[u8] {
        &self.buf[..self.len]
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

        let align = core::mem::align_of::<T>();
        let ptr = self.buf.as_ptr();
        if align > 1 && (ptr as usize) % align != 0 {
            return Err(ProgramError::InvalidAccountData);
        }

        Ok(unsafe { &*(ptr as *const T) })
    }

    /// Read a u64 from the first 8 bytes of return data.
    #[inline]
    pub fn as_u64(&self) -> Result<u64, ProgramError> {
        if self.len < 8 {
            return Err(ProgramError::AccountDataTooSmall);
        }
        let mut bytes = [0u8; 8];
        bytes.copy_from_slice(&self.buf[..8]);
        Ok(u64::from_le_bytes(bytes))
    }

    /// Read a u32 from the first 4 bytes of return data.
    #[inline]
    pub fn as_u32(&self) -> Result<u32, ProgramError> {
        if self.len < 4 {
            return Err(ProgramError::AccountDataTooSmall);
        }
        let mut bytes = [0u8; 4];
        bytes.copy_from_slice(&self.buf[..4]);
        Ok(u32::from_le_bytes(bytes))
    }
}

/// Retrieve return data from the most recent CPI call.
///
/// Returns `None` if no return data was set (length == 0).
#[inline]
pub fn get_return_data() -> Option<ReturnData> {
    #[allow(unused_mut)]
    let mut rd = ReturnData {
        buf: [0u8; MAX_RETURN_DATA],
        len: 0,
        program_id: Address::default(),
    };

    #[cfg(target_os = "solana")]
    {
        let actual_len = unsafe {
            crate::syscalls::sol_get_return_data(
                rd.buf.as_mut_ptr(),
                MAX_RETURN_DATA as u64,
                rd.program_id.0.as_mut_ptr(),
            )
        };
        rd.len = (actual_len as usize).min(MAX_RETURN_DATA);
    }

    #[cfg(not(target_os = "solana"))]
    {
        // Off-chain: no return data available.
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
pub fn invoke_and_read<'a, T: Projectable, const ACCOUNTS: usize>(
    instruction: &InstructionView,
    account_views: &[&crate::account_view::AccountView; ACCOUNTS],
    signers_seeds: &[Signer],
) -> Result<ReturnData, ProgramError> {
    crate::cpi::invoke_signed::<ACCOUNTS>(instruction, account_views, signers_seeds)?;

    get_return_data().ok_or(ProgramError::InvalidAccountData)
}
