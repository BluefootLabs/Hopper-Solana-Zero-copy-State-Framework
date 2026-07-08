//! CPI return-data helpers.
//!
//! Solana return data is a 1 KiB byte channel from the most recent CPI. Hopper
//! keeps it stack-backed and exposes typed reads by value for `Pod` types.

use crate::{Address, Pod, ProgramError, ProgramResult};
use core::mem::MaybeUninit;

/// Maximum Solana return-data payload length.
pub const MAX_RETURN_DATA: usize = 1024;

/// Stack-backed snapshot of CPI return data.
///
/// The 1 KiB buffer is deliberately left uninitialized until the
/// `sol_get_return_data` syscall fills it; only the syscall-initialized prefix
/// (`data_len` bytes) is ever exposed to callers.
#[derive(Clone)]
pub struct ReturnData {
    program_id: Address,
    data: [MaybeUninit<u8>; MAX_RETURN_DATA],
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
        // SAFETY: `sol_get_return_data` initializes exactly
        // `min(actual_len, MAX_RETURN_DATA)` bytes of the buffer it was handed,
        // and `get_return_data` sets `data_len` to that same value (the test
        // constructor likewise writes `data_len` bytes before setting it), so
        // the first `data_len` bytes are always initialized `u8`s.
        // Fail-closed backstop for the invariant the SAFETY comment relies
        // on: `data_len` can never exceed the buffer capacity.
        debug_assert!(self.data_len <= MAX_RETURN_DATA);
        unsafe { core::slice::from_raw_parts(self.data.as_ptr() as *const u8, self.data_len) }
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
        // SAFETY: `T: Pod` is copyable from raw bytes, and the length check
        // above keeps the read inside the initialized `data_len`-byte prefix
        // exposed by `data()`. Use an unaligned read so the stack byte buffer
        // never imposes alignment requirements on callers.
        Ok(unsafe { core::ptr::read_unaligned(self.data().as_ptr() as *const T) })
    }
}

impl core::fmt::Debug for ReturnData {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        // Manual impl: deriving Debug would format the raw `MaybeUninit`
        // buffer; only the initialized prefix may be read.
        f.debug_struct("ReturnData")
            .field("program_id", &self.program_id)
            .field("data", &self.data())
            .field("data_len", &self.data_len)
            .field("actual_len", &self.actual_len)
            .finish()
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
///
/// The 1 KiB snapshot buffer is *not* zero-filled before the syscall — the
/// syscall initializes exactly the reported prefix, and `None` is returned
/// before any read when the runtime reports zero bytes. This is the bug class
/// behind Quasar #238/#234 (an `assume_init` over a buffer the syscall never
/// wrote, exposing uninitialized stack bytes as return data); Hopper's shape
/// is immune because uninitialized bytes can never escape: empty return data
/// short-circuits to `None`, and every accessor reads only the
/// syscall-initialized `data_len` prefix.
#[inline]
pub fn get_return_data() -> Option<ReturnData> {
    let mut snapshot = ReturnData {
        program_id: Address::default(),
        data: [const { MaybeUninit::uninit() }; MAX_RETURN_DATA],
        data_len: 0,
        actual_len: 0,
    };

    // SAFETY: Snapshot buffers are stack-allocated with the exact capacities
    // advertised to the runtime syscall; the data buffer may be uninitialized
    // because the syscall only writes (never reads) it.
    let actual_len = unsafe {
        crate::syscalls::sol_get_return_data(
            snapshot.data.as_mut_ptr() as *mut u8,
            MAX_RETURN_DATA as u64,
            snapshot.program_id.as_mut().as_mut_ptr(),
        )
    } as usize;

    if actual_len == 0 {
        // Nothing was written into the buffer: return before any field of the
        // snapshot's data can be observed. Off-chain the syscall stub reports
        // 0, so the uninitialized buffer never escapes there either.
        return None;
    }

    snapshot.actual_len = actual_len;
    snapshot.data_len = core::cmp::min(actual_len, MAX_RETURN_DATA);
    Some(snapshot)
}

#[cfg(test)]
impl ReturnData {
    /// Test-only constructor: builds a snapshot whose buffer prefix is fully
    /// initialized from `bytes`, mirroring what the syscall produces on-chain.
    fn test_snapshot(bytes: &[u8], program_id: Address) -> Self {
        assert!(bytes.len() <= MAX_RETURN_DATA);
        let mut data = [const { MaybeUninit::uninit() }; MAX_RETURN_DATA];
        for (dst, src) in data.iter_mut().zip(bytes) {
            dst.write(*src);
        }
        ReturnData {
            program_id,
            data,
            data_len: bytes.len(),
            actual_len: bytes.len(),
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
    fn try_set_return_data_rejects_oversized_payload() {
        let oversized = [0u8; MAX_RETURN_DATA + 1];
        assert!(try_set_return_data(&oversized).is_err());
    }

    #[test]
    fn return_data_reads_pod_by_value() {
        let snapshot = ReturnData::test_snapshot(&7u64.to_le_bytes(), Address::default());
        let raw = snapshot.read_pod::<[u8; 8]>().unwrap();
        assert_eq!(u64::from_le_bytes(raw), 7);
    }

    #[test]
    fn data_exposes_exactly_the_written_prefix() {
        let payload = [0xAB, 0xCD, 0xEF];
        let snapshot = ReturnData::test_snapshot(&payload, Address::default());
        assert_eq!(snapshot.data(), &payload);
        assert_eq!(snapshot.len(), payload.len());
        assert!(!snapshot.is_empty());
    }

    #[test]
    fn read_pod_never_reads_past_the_prefix() {
        let snapshot = ReturnData::test_snapshot(&[1, 2, 3], Address::default());
        assert!(matches!(
            snapshot.read_pod::<[u8; 4]>(),
            Err(ProgramError::AccountDataTooSmall)
        ));
        assert_eq!(snapshot.read_pod::<[u8; 3]>().unwrap(), [1, 2, 3]);
    }

    #[test]
    fn copy_to_copies_only_the_prefix() {
        let snapshot = ReturnData::test_snapshot(&[9, 8], Address::default());
        let mut dst = [0xFFu8; 4];
        snapshot.copy_to(&mut dst).unwrap();
        assert_eq!(dst, [9, 8, 0xFF, 0xFF]);

        let mut too_small = [0u8; 1];
        assert!(snapshot.copy_to(&mut too_small).is_err());
    }

    #[test]
    fn is_truncated_reflects_runtime_reported_length() {
        let mut snapshot = ReturnData::test_snapshot(&[0u8; 16], Address::default());
        assert!(!snapshot.is_truncated());
        snapshot.actual_len = MAX_RETURN_DATA + 512;
        assert!(snapshot.is_truncated());
        assert_eq!(snapshot.actual_len(), MAX_RETURN_DATA + 512);
        assert_eq!(snapshot.len(), 16);
    }

    #[test]
    fn debug_prints_only_the_initialized_prefix() {
        let snapshot = ReturnData::test_snapshot(&[1, 2], Address::default());
        let rendered = std::format!("{snapshot:?}");
        assert!(rendered.contains("data: [1, 2]"));
        assert!(rendered.contains("data_len: 2"));
    }
}
