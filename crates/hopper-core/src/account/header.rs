//! 16-byte account header.
//!
//! Wire format (all fields little-endian where multi-byte):
//! ```text
//! ----------------------------------------------------------------
//! - byte 0  - byte 1  - bytes 2-3- bytes 4-11       - bytes 12-15-
//! ----------------------------------------------------------------
//! - DISC    - VERSION - FLAGS    - LAYOUT_ID (8)    - RESERVED   -
//! - u8      - u8      - u16 LE   - [u8; 8] SHA-256  - [u8; 4]    -
//! ----------------------------------------------------------------
//! ```
//!
//! The layout_id is the first 8 bytes of:
//! `SHA-256("hopper:v1:" + name + ":" + version + ":" + canonical_field_string)`
//!
//! Canonical field string: `"field_name:canonical_type:size,"` per field with trailing comma.
//! Field order is declaration order.

use hopper_runtime::error::ProgramError;

/// Header length in bytes.
pub const HEADER_LEN: usize = 16;

/// Header format version. Bump only if the header wire format itself changes.
pub const HEADER_FORMAT: u8 = 1;

// Offsets within the header
const DISC_OFFSET: usize = 0;
const VERSION_OFFSET: usize = 1;
const FLAGS_OFFSET: usize = 2;
const LAYOUT_ID_OFFSET: usize = 4;
const RESERVED_OFFSET: usize = 12;

/// The 16-byte account header, overlay-safe.
#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct AccountHeader {
    pub disc: u8,
    pub version: u8,
    pub flags: [u8; 2],
    pub layout_id: [u8; 8],
    pub reserved: [u8; 4],
}

const _: () = assert!(core::mem::size_of::<AccountHeader>() == HEADER_LEN);
const _: () = assert!(core::mem::align_of::<AccountHeader>() == 1);

// SAFETY: #[repr(C)] of all-byte fields, all bit patterns valid.
unsafe impl super::Pod for AccountHeader {}

impl super::FixedLayout for AccountHeader {
    const SIZE: usize = HEADER_LEN;
}

impl AccountHeader {
    /// Create a new header.
    #[inline(always)]
    pub const fn new(disc: u8, version: u8, flags: u16, layout_id: [u8; 8]) -> Self {
        Self {
            disc,
            version,
            flags: flags.to_le_bytes(),
            layout_id,
            reserved: [0; 4],
        }
    }

    /// Read the flags as a `u16`.
    #[inline(always)]
    pub const fn flags_u16(&self) -> u16 {
        u16::from_le_bytes(self.flags)
    }
}

/// Write a complete header to the beginning of `data`.
///
/// # Precondition
/// `data.len() >= HEADER_LEN` and data should be zero-initialized first.
#[inline(always)]
pub fn write_header(
    data: &mut [u8],
    disc: u8,
    version: u8,
    layout_id: &[u8; 8],
) -> Result<(), ProgramError> {
    if data.len() < HEADER_LEN {
        return Err(ProgramError::AccountDataTooSmall);
    }
    data[DISC_OFFSET] = disc;
    data[VERSION_OFFSET] = version;
    data[FLAGS_OFFSET..FLAGS_OFFSET + 2].copy_from_slice(&0u16.to_le_bytes());
    data[LAYOUT_ID_OFFSET..LAYOUT_ID_OFFSET + 8].copy_from_slice(layout_id);
    data[RESERVED_OFFSET..RESERVED_OFFSET + 4].copy_from_slice(&[0u8; 4]);
    Ok(())
}

/// Validate the header against expected values.
#[inline(always)]
pub fn check_header(
    data: &[u8],
    expected_disc: u8,
    min_version: u8,
    layout_id: &[u8; 8],
) -> Result<(), ProgramError> {
    if data.len() < HEADER_LEN {
        return Err(ProgramError::AccountDataTooSmall);
    }
    if data[DISC_OFFSET] != expected_disc {
        return Err(ProgramError::InvalidAccountData);
    }
    if data[VERSION_OFFSET] < min_version {
        return Err(ProgramError::InvalidAccountData);
    }
    if data[LAYOUT_ID_OFFSET..LAYOUT_ID_OFFSET + 8] != *layout_id {
        return Err(ProgramError::InvalidAccountData);
    }
    Ok(())
}

/// Read the discriminator byte from raw account data.
///
/// This is a standalone utility for callers that need to peek at the
/// disc without constructing an `AccountReader`. Returns the first
/// byte or `AccountDataTooSmall` if `data` is empty.
#[inline(always)]
pub fn read_discriminator(data: &[u8]) -> Result<u8, ProgramError> {
    data.first().copied().ok_or(ProgramError::AccountDataTooSmall)
}

/// Read the version byte.
#[inline(always)]
pub fn read_version(data: &[u8]) -> Result<u8, ProgramError> {
    if data.len() < 2 {
        return Err(ProgramError::AccountDataTooSmall);
    }
    Ok(data[VERSION_OFFSET])
}

/// Read the flags field as `u16`.
#[inline(always)]
pub fn read_header_flags(data: &[u8]) -> Result<u16, ProgramError> {
    if data.len() < 4 {
        return Err(ProgramError::AccountDataTooSmall);
    }
    Ok(u16::from_le_bytes([data[FLAGS_OFFSET], data[FLAGS_OFFSET + 1]]))
}

/// Read the 8-byte layout_id.
#[inline(always)]
pub fn read_layout_id(data: &[u8]) -> Result<[u8; 8], ProgramError> {
    if data.len() < 12 {
        return Err(ProgramError::AccountDataTooSmall);
    }
    let mut id = [0u8; 8];
    id.copy_from_slice(&data[LAYOUT_ID_OFFSET..LAYOUT_ID_OFFSET + 8]);
    Ok(id)
}
