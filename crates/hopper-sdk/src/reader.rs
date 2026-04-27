//! # Segment-aware partial account reader
//!
//! Clients usually want only a few fields of a large account, not the whole
//! blob. Hopper's schema knows every field's byte offset and canonical type,
//! so a client can pull exactly the bytes it needs and skip the rest. This is
//! the off-chain mirror of the on-chain segment-borrow idea: read narrow,
//! don't deserialize what you won't touch.
//!
//! Competitive framing:
//! - Quasar clients rely on Codama/Kinobi full deserialization.
//! - Anchor clients rely on Borsh full deserialization.
//! - Pinocchio has no client story at all.
//!
//! Here the reader verifies the layout_id first, then returns raw-typed
//! accessors for any named field, backed by the same offset tables the
//! on-chain side uses.

use hopper_schema::{FieldDescriptor, LayoutManifest};

use crate::fingerprint::{
    check_against_layout, FingerprintCheck, FingerprintError, LAYOUT_ID_OFFSET,
};

/// Errors produced by the segment-aware reader.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReaderError {
    /// Account data was shorter than the manifest's declared `total_size`.
    BufferTooShort {
        /// Required length.
        required: usize,
        /// Actual length.
        got: usize,
    },
    /// The header's layout_id did not match the expected layout's.
    LayoutMismatch {
        /// Expected fingerprint.
        expected: [u8; 8],
        /// Fingerprint actually on-chain.
        actual: [u8; 8],
    },
    /// Named field not found in layout.
    UnknownField,
    /// Field size on the wire does not match the caller's type width.
    SizeMismatch {
        /// Field wire size.
        wire: u16,
        /// Caller-requested size.
        requested: usize,
    },
    /// Fingerprint surface returned an error.
    Fingerprint(FingerprintError),
}

impl From<FingerprintError> for ReaderError {
    fn from(e: FingerprintError) -> Self {
        ReaderError::Fingerprint(e)
    }
}

/// Zero-copy segment-aware partial account reader.
///
/// Construct with [`SegmentReader::new`]. the layout_id is verified up front
/// so downstream `read_*` calls can be infallible w.r.t. identity.
#[derive(Debug)]
pub struct SegmentReader<'a> {
    bytes: &'a [u8],
    layout: &'a LayoutManifest,
}

impl<'a> SegmentReader<'a> {
    /// Bind a `LayoutManifest` to raw bytes, verifying the fingerprint.
    pub fn new(bytes: &'a [u8], layout: &'a LayoutManifest) -> Result<Self, ReaderError> {
        if bytes.len() < layout.total_size {
            return Err(ReaderError::BufferTooShort {
                required: layout.total_size,
                got: bytes.len(),
            });
        }
        match check_against_layout(bytes, layout)? {
            FingerprintCheck::Match => Ok(Self { bytes, layout }),
            FingerprintCheck::Mismatch { expected, actual } => {
                Err(ReaderError::LayoutMismatch { expected, actual })
            }
        }
    }

    /// Bind without re-checking the fingerprint. Only use if you've already
    /// verified identity upstream.
    ///
    /// # Safety
    /// The caller promises `bytes` is a `layout`-shaped blob.
    pub unsafe fn new_unchecked(bytes: &'a [u8], layout: &'a LayoutManifest) -> Self {
        Self { bytes, layout }
    }

    /// Access the raw account buffer.
    pub const fn bytes(&self) -> &'a [u8] {
        self.bytes
    }

    /// Layout manifest this reader was constructed against.
    pub const fn layout(&self) -> &'a LayoutManifest {
        self.layout
    }

    /// Look up a field descriptor by name.
    pub fn field(&self, name: &str) -> Option<&'a FieldDescriptor> {
        let mut i = 0;
        while i < self.layout.fields.len() {
            if bytes_eq(self.layout.fields[i].name, name) {
                return Some(&self.layout.fields[i]);
            }
            i += 1;
        }
        None
    }

    /// Absolute byte offset of a named field (accounts for the
    /// 12-byte Hopper header by using `offset` as field-start; since the
    /// manifest's `FieldDescriptor.offset` is already absolute under the
    /// framework's convention, this is a pass-through).
    pub fn offset_of(&self, name: &str) -> Option<usize> {
        self.field(name).map(|f| f.offset as usize)
    }

    /// Raw byte slice for a named field.
    pub fn read_raw(&self, name: &str) -> Result<&'a [u8], ReaderError> {
        let f = self.field(name).ok_or(ReaderError::UnknownField)?;
        let start = f.offset as usize;
        let end = start + f.size as usize;
        if end > self.bytes.len() {
            return Err(ReaderError::BufferTooShort {
                required: end,
                got: self.bytes.len(),
            });
        }
        Ok(&self.bytes[start..end])
    }

    /// Read a `u8` field.
    pub fn read_u8(&self, name: &str) -> Result<u8, ReaderError> {
        let raw = self.read_fixed(name, 1)?;
        Ok(raw[0])
    }

    /// Read a little-endian `u16`.
    pub fn read_u16(&self, name: &str) -> Result<u16, ReaderError> {
        let raw = self.read_fixed(name, 2)?;
        Ok(u16::from_le_bytes([raw[0], raw[1]]))
    }

    /// Read a little-endian `u32`.
    pub fn read_u32(&self, name: &str) -> Result<u32, ReaderError> {
        let raw = self.read_fixed(name, 4)?;
        Ok(u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]))
    }

    /// Read a little-endian `u64`.
    pub fn read_u64(&self, name: &str) -> Result<u64, ReaderError> {
        let raw = self.read_fixed(name, 8)?;
        Ok(u64::from_le_bytes([
            raw[0], raw[1], raw[2], raw[3], raw[4], raw[5], raw[6], raw[7],
        ]))
    }

    /// Read a little-endian `u128`.
    pub fn read_u128(&self, name: &str) -> Result<u128, ReaderError> {
        let raw = self.read_fixed(name, 16)?;
        let mut bytes = [0u8; 16];
        bytes.copy_from_slice(raw);
        Ok(u128::from_le_bytes(bytes))
    }

    /// Read a 32-byte Solana public key (Pubkey).
    pub fn read_pubkey(&self, name: &str) -> Result<[u8; 32], ReaderError> {
        let raw = self.read_fixed(name, 32)?;
        let mut out = [0u8; 32];
        out.copy_from_slice(raw);
        Ok(out)
    }

    /// Read the layout_id from the attached buffer.
    pub fn layout_id(&self) -> [u8; 8] {
        let mut id = [0u8; 8];
        id.copy_from_slice(&self.bytes[LAYOUT_ID_OFFSET..LAYOUT_ID_OFFSET + 8]);
        id
    }

    fn read_fixed(&self, name: &str, expect: usize) -> Result<&'a [u8], ReaderError> {
        let f = self.field(name).ok_or(ReaderError::UnknownField)?;
        if f.size as usize != expect {
            return Err(ReaderError::SizeMismatch {
                wire: f.size,
                requested: expect,
            });
        }
        let start = f.offset as usize;
        let end = start + expect;
        if end > self.bytes.len() {
            return Err(ReaderError::BufferTooShort {
                required: end,
                got: self.bytes.len(),
            });
        }
        Ok(&self.bytes[start..end])
    }
}

fn bytes_eq(a: &str, b: &str) -> bool {
    // Explicit equal-length scan so this compiles in no_std contexts without
    // pulling in str::eq internals.
    let a = a.as_bytes();
    let b = b.as_bytes();
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use hopper_schema::FieldIntent;

    const LAYOUT_ID: [u8; 8] = [0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF, 0x11, 0x22];

    fn fields() -> &'static [FieldDescriptor] {
        static F: [FieldDescriptor; 3] = [
            FieldDescriptor {
                name: "authority",
                canonical_type: "Pubkey",
                size: 32,
                offset: 16,
                intent: FieldIntent::Authority,
            },
            FieldDescriptor {
                name: "balance",
                canonical_type: "u64",
                size: 8,
                offset: 48,
                intent: FieldIntent::Balance,
            },
            FieldDescriptor {
                name: "bump",
                canonical_type: "u8",
                size: 1,
                offset: 56,
                intent: FieldIntent::Bump,
            },
        ];
        &F
    }

    fn manifest() -> LayoutManifest {
        LayoutManifest {
            name: "Vault",
            disc: 5,
            version: 1,
            layout_id: LAYOUT_ID,
            total_size: 80,
            field_count: 3,
            fields: fields(),
        }
    }

    fn blob() -> [u8; 80] {
        let mut b = [0u8; 80];
        b[0] = 5;
        b[1] = 1;
        b[LAYOUT_ID_OFFSET..LAYOUT_ID_OFFSET + 8].copy_from_slice(&LAYOUT_ID);
        // authority
        for i in 0..32 {
            b[16 + i] = i as u8;
        }
        // balance = 1_000_000
        b[48..56].copy_from_slice(&1_000_000u64.to_le_bytes());
        // bump
        b[56] = 253;
        b
    }

    #[test]
    fn binds_and_reads() {
        let m = manifest();
        let b = blob();
        let r = SegmentReader::new(&b, &m).unwrap();
        assert_eq!(r.read_u64("balance").unwrap(), 1_000_000);
        assert_eq!(r.read_u8("bump").unwrap(), 253);
        let pk = r.read_pubkey("authority").unwrap();
        assert_eq!(pk[0], 0);
        assert_eq!(pk[31], 31);
    }

    #[test]
    fn rejects_wrong_layout_id() {
        let m = manifest();
        let mut b = blob();
        b[LAYOUT_ID_OFFSET] ^= 0xFF;
        let err = SegmentReader::new(&b, &m).unwrap_err();
        assert!(matches!(err, ReaderError::LayoutMismatch { .. }));
    }

    #[test]
    fn rejects_too_short() {
        let m = manifest();
        let b = [0u8; 40];
        let err = SegmentReader::new(&b, &m).unwrap_err();
        assert!(matches!(
            err,
            ReaderError::BufferTooShort {
                required: 80,
                got: 40
            }
        ));
    }

    #[test]
    fn size_mismatch_detected() {
        let m = manifest();
        let b = blob();
        let r = SegmentReader::new(&b, &m).unwrap();
        assert!(matches!(
            r.read_u32("balance"),
            Err(ReaderError::SizeMismatch {
                wire: 8,
                requested: 4
            })
        ));
    }
}
