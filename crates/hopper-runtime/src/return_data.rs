//! CPI return-data helpers.
//!
//! Solana return data is a 1 KiB byte channel from the most recent CPI. Hopper
//! keeps it stack-backed and exposes typed reads by value for `Pod` types.

use crate::{Address, Pod, ProgramError, ProgramResult};

/// Maximum Solana return-data payload length.
pub const MAX_RETURN_DATA: usize = 1024;

/// Stack-backed snapshot of CPI return data.
#[derive(Clone, Debug)]
pub struct ReturnData {
    program_id: Address,
    data: [u8; MAX_RETURN_DATA],
    data_len: usize,
    actual_len: usize,
}

impl ReturnData {
    /// Program id that set this return data.
    #[inline(always)]
    pub const fn program_id(&self) -> &Address {
        &self.program_id
    }

    /// Bytes copied into this snapshot.
    #[inline(always)]
    pub fn data(&self) -> &[u8] {
        &self.data[..self.data_len]
    }

    /// Copied byte length.
    #[inline(always)]
    pub const fn len(&self) -> usize {
        self.data_len
    }

    /// Runtime-reported byte length before truncation to Hopper's stack buffer.
    #[inline(always)]
    pub const fn actual_len(&self) -> usize {
        self.actual_len
    }

    /// Whether no bytes were copied.
    #[inline(always)]
    pub const fn is_empty(&self) -> bool {
        self.data_len == 0
    }

    /// Whether the runtime reported more bytes than Hopper copied.
    #[inline(always)]
    pub const fn is_truncated(&self) -> bool {
        self.actual_len > self.data_len
    }

    /// Copy return bytes into `dst`.
    #[inline]
    pub fn copy_to(&self, dst: &mut [u8]) -> ProgramResult {
        crate::memory::copy_bytes(dst, self.data())
    }

    /// Read a `Pod` value from the first bytes of the return-data payload.
    #[inline]
    pub fn read_pod<T: Pod>(&self) -> Result<T, ProgramError> {
        let size = core::mem::size_of::<T>();
        if self.data_len < size {
            return Err(ProgramError::AccountDataTooSmall);
        }
        // SAFETY: `T: Pod` is copyable from raw bytes. Use an unaligned read so
        // the stack byte buffer never imposes alignment requirements on callers.
        Ok(unsafe { core::ptr::read_unaligned(self.data.as_ptr() as *const T) })
    }
}

/// Set return data for this instruction.
#[inline(always)]
pub fn set_return_data(data: &[u8]) {
    // SAFETY: `data` is a valid byte slice for its full length.
    unsafe {
        crate::syscalls::sol_set_return_data(data.as_ptr(), data.len() as u64);
    }
}

/// Set return data, rejecting payloads larger than Solana's 1 KiB limit.
#[inline]
pub fn try_set_return_data(data: &[u8]) -> ProgramResult {
    if data.len() > MAX_RETURN_DATA {
        return Err(ProgramError::InvalidArgument);
    }
    set_return_data(data);
    Ok(())
}

/// Read return data from the most recent CPI.
#[inline]
pub fn get_return_data() -> Option<ReturnData> {
    let mut snapshot = ReturnData {
        program_id: Address::default(),
        data: [0u8; MAX_RETURN_DATA],
        data_len: 0,
        actual_len: 0,
    };

    // SAFETY: Snapshot buffers are stack-allocated with the exact capacities
    // advertised to the runtime syscall.
    let actual_len = unsafe {
        crate::syscalls::sol_get_return_data(
            snapshot.data.as_mut_ptr(),
            MAX_RETURN_DATA as u64,
            snapshot.program_id.as_mut().as_mut_ptr(),
        )
    } as usize;

    if actual_len == 0 {
        return None;
    }

    snapshot.actual_len = actual_len;
    snapshot.data_len = core::cmp::min(actual_len, MAX_RETURN_DATA);
    Some(snapshot)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offchain_get_return_data_is_none() {
        assert!(get_return_data().is_none());
    }

    #[test]
    fn try_set_return_data_rejects_oversized_payload() {
        let oversized = [0u8; MAX_RETURN_DATA + 1];
        assert!(try_set_return_data(&oversized).is_err());
    }

    #[test]
    fn return_data_reads_pod_by_value() {
        let mut data = [0u8; MAX_RETURN_DATA];
        data[..8].copy_from_slice(&7u64.to_le_bytes());
        let snapshot = ReturnData {
            program_id: Address::default(),
            data,
            data_len: 8,
            actual_len: 8,
        };
        assert_eq!(snapshot.read_pod::<u64>().unwrap(), 7);
    }
}
