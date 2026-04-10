//! Account reader with header-aware field access.

use hopper_runtime::error::ProgramError;
use super::header::{AccountHeader, HEADER_LEN};
use super::cursor::SliceCursor;
use super::pod::pod_from_bytes;

/// Header-aware read-only account reader.
///
/// Provides typed access to the header fields and a `SliceCursor` for the body.
pub struct AccountReader<'a> {
    data: &'a [u8],
}

impl<'a> AccountReader<'a> {
    /// Create a reader, validating minimum header size.
    #[inline(always)]
    pub fn new(data: &'a [u8]) -> Result<Self, ProgramError> {
        if data.len() < HEADER_LEN {
            return Err(ProgramError::AccountDataTooSmall);
        }
        Ok(Self { data })
    }

    /// Create a reader with full header validation.
    #[inline(always)]
    pub fn new_checked(
        data: &'a [u8],
        disc: u8,
        min_version: u8,
        layout_id: &[u8; 8],
    ) -> Result<Self, ProgramError> {
        super::header::check_header(data, disc, min_version, layout_id)?;
        Ok(Self { data })
    }

    /// The underlying header, zero-copy.
    #[inline(always)]
    pub fn header(&self) -> &AccountHeader {
        // SAFETY: We validated data.len() >= HEADER_LEN in constructor.
        // AccountHeader has align_of == 1 and size == HEADER_LEN.
        unsafe { &*(self.data.as_ptr() as *const AccountHeader) }
    }

    /// Discriminator byte.
    #[inline(always)]
    pub fn discriminator(&self) -> u8 {
        self.data[0]
    }

    /// Version byte.
    #[inline(always)]
    pub fn version(&self) -> u8 {
        self.data[1]
    }

    /// Flags as u16.
    #[inline(always)]
    pub fn flags(&self) -> u16 {
        u16::from_le_bytes([self.data[2], self.data[3]])
    }

    /// Layout ID (8 bytes).
    #[inline(always)]
    pub fn layout_id(&self) -> [u8; 8] {
        let mut id = [0u8; 8];
        id.copy_from_slice(&self.data[4..12]);
        id
    }

    /// Body data after the header as a cursor.
    #[inline(always)]
    pub fn body(&self) -> SliceCursor<'a> {
        SliceCursor::new(&self.data[HEADER_LEN..])
    }

    /// Raw body bytes after the header.
    #[inline(always)]
    pub fn body_bytes(&self) -> &'a [u8] {
        &self.data[HEADER_LEN..]
    }

    /// Entire raw data including header.
    #[inline(always)]
    pub fn raw(&self) -> &'a [u8] {
        self.data
    }

    /// Read a u64 at a specific offset from the start of data.
    #[inline(always)]
    pub fn u64_at(&self, offset: usize) -> Result<u64, ProgramError> {
        if offset + 8 > self.data.len() {
            return Err(ProgramError::InvalidAccountData);
        }
        Ok(u64::from_le_bytes([
            self.data[offset],
            self.data[offset + 1],
            self.data[offset + 2],
            self.data[offset + 3],
            self.data[offset + 4],
            self.data[offset + 5],
            self.data[offset + 6],
            self.data[offset + 7],
        ]))
    }

    /// Read a 32-byte address at a specific offset.
    #[inline(always)]
    pub fn address_at(&self, offset: usize) -> Result<&'a [u8; 32], ProgramError> {
        if offset + 32 > self.data.len() {
            return Err(ProgramError::InvalidAccountData);
        }
        // SAFETY: Checked bounds. [u8; 32] has alignment 1.
        Ok(unsafe { &*(self.data.as_ptr().add(offset) as *const [u8; 32]) })
    }

    /// Overlay a Pod type at a specific offset.
    #[inline(always)]
    pub fn overlay_at<T: super::Pod + super::FixedLayout>(
        &self,
        offset: usize,
    ) -> Result<&'a T, ProgramError> {
        if offset > self.data.len() {
            return Err(ProgramError::InvalidAccountData);
        }
        pod_from_bytes::<T>(&self.data[offset..])
    }
}
