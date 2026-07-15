//! Decoder for the Hopper **touch-map wire format v1** — the public,
//! versioned record a Hopper program emits (one `sol_log_data` segment)
//! describing every `(account, offset, size, R/W)` range an instruction
//! touched.
//!
//! This is a byte-for-byte reimplementation of the decode that already
//! ships in `hopper tx explain`
//! (`tools/hopper-cli/src/cmd/tx_explain.rs`), the generated TypeScript
//! client, and the runtime's own tests. The format is documented in
//! `hopper-runtime/src/segment_borrow.rs`:
//!
//! ```text
//! byte 0        magic   = 0x7A
//! byte 1        version = 0x01
//! byte 2        flags   bit0 = overflowed (log truncated, map is PARTIAL)
//!                       bit1 = skipped    (a record could not be represented)
//! byte 3        count   number of records (each 9 bytes)
//! then per record (9 bytes):
//!   +0          slot    u8  account index into the instruction's account list
//!   +1..+5      packed  u32 LE; bit31 = write, low 31 bits = byte offset
//!   +5..+9      size    u32 LE
//! ```
//!
//! A decoder MUST verify magic, version, AND the exact-length equation
//! `len == 4 + 9 * count` — together these keep unrelated `Program data:`
//! payloads (Anchor events, receipts) from ever being misread as a map.

/// Magic byte of the touch-map wire format v1 (`'z'`).
pub const TOUCH_MAP_MAGIC: u8 = 0x7A;
/// Touch-map wire format version.
pub const TOUCH_MAP_VERSION: u8 = 0x01;
/// Flags bit0: the on-chain touch log overflowed; the map is partial.
pub const TOUCH_MAP_FLAG_OVERFLOWED: u8 = 1 << 0;
/// Flags bit1: the encoder skipped at least one record it could not
/// represent (unmappable address, or offset beyond 31 bits).
pub const TOUCH_MAP_FLAG_SKIPPED: u8 = 1 << 1;
/// Fixed header length (magic, version, flags, count).
pub const TOUCH_MAP_HEADER_LEN: usize = 4;
/// Encoded length of one touch record.
pub const TOUCH_MAP_RECORD_LEN: usize = 9;
/// Capacity of the on-chain touch log; overflow truncates at this count.
pub const MAX_TOUCH_RECORDS: usize = 32;

/// One decoded touch record: a single `(account, offset, size, R/W)` range
/// the instruction touched.
///
/// `slot` is the account's index in the instruction's account list — the
/// same index a manifest [`RangeContract`](grillo_manifest::RangeContract)
/// and the runtime `Context` use, so the three join directly.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TouchRecord {
    /// Account slot index into the instruction's account list.
    pub slot: u8,
    /// Byte offset within the account data.
    pub offset: u32,
    /// Byte length of the touched range.
    pub size: u32,
    /// Write access (`true`) or read access (`false`).
    pub write: bool,
}

impl TouchRecord {
    /// Exclusive end offset, widened to `u64` so it cannot overflow.
    pub fn end(&self) -> u64 {
        self.offset as u64 + self.size as u64
    }
}

/// A decoded touch map, with its honesty flags preserved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TouchMap {
    /// The touch log overflowed [`MAX_TOUCH_RECORDS`]; the map is a partial
    /// view of the instruction's effects.
    pub overflowed: bool,
    /// The encoder dropped at least one touched range it could not
    /// represent; the map is partial.
    pub skipped: bool,
    /// The decoded records, in first-touch order.
    pub records: Vec<TouchRecord>,
}

impl TouchMap {
    /// A partial map (`overflowed` or `skipped`) does NOT enumerate the
    /// instruction's complete effect set, so byte attribution against it is
    /// impossible — a verifier must return `INCONCLUSIVE`, never `PASS`.
    pub fn is_partial(&self) -> bool {
        self.overflowed || self.skipped
    }

    /// The WRITE records only (the `acquired` write-set).
    pub fn writes(&self) -> impl Iterator<Item = &TouchRecord> {
        self.records.iter().filter(|r| r.write)
    }
}

/// Why a byte slice is not a valid touch-map v1 payload.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecodeError {
    /// Fewer than the 4-byte header.
    TooShort { len: usize },
    /// First byte was not [`TOUCH_MAP_MAGIC`].
    BadMagic(u8),
    /// Second byte was not [`TOUCH_MAP_VERSION`].
    BadVersion(u8),
    /// The exact-length equation `len == 4 + 9 * count` did not hold. This
    /// is what keeps foreign `Program data:` payloads out.
    LengthMismatch {
        declared_count: usize,
        expected_len: usize,
        actual_len: usize,
    },
}

impl core::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            DecodeError::TooShort { len } => {
                write!(
                    f,
                    "touch map too short: {len} bytes (need >= {TOUCH_MAP_HEADER_LEN})"
                )
            }
            DecodeError::BadMagic(b) => {
                write!(
                    f,
                    "bad touch-map magic: 0x{b:02x} (expected 0x{TOUCH_MAP_MAGIC:02x})"
                )
            }
            DecodeError::BadVersion(b) => {
                write!(
                    f,
                    "bad touch-map version: 0x{b:02x} (expected 0x{TOUCH_MAP_VERSION:02x})"
                )
            }
            DecodeError::LengthMismatch {
                declared_count,
                expected_len,
                actual_len,
            } => write!(
                f,
                "touch-map length mismatch: count {declared_count} implies {expected_len} bytes, \
                 got {actual_len}"
            ),
        }
    }
}

impl std::error::Error for DecodeError {}

/// Decode a touch-map `sol_log_data` payload. Returns an error unless the
/// magic, version, and exact-length equation all hold.
pub fn decode_touch_map(bytes: &[u8]) -> Result<TouchMap, DecodeError> {
    if bytes.len() < TOUCH_MAP_HEADER_LEN {
        return Err(DecodeError::TooShort { len: bytes.len() });
    }
    if bytes[0] != TOUCH_MAP_MAGIC {
        return Err(DecodeError::BadMagic(bytes[0]));
    }
    if bytes[1] != TOUCH_MAP_VERSION {
        return Err(DecodeError::BadVersion(bytes[1]));
    }
    let flags = bytes[2];
    let count = bytes[3] as usize;
    let expected_len = TOUCH_MAP_HEADER_LEN + count * TOUCH_MAP_RECORD_LEN;
    if bytes.len() != expected_len {
        return Err(DecodeError::LengthMismatch {
            declared_count: count,
            expected_len,
            actual_len: bytes.len(),
        });
    }

    let mut records = Vec::with_capacity(count);
    for i in 0..count {
        let base = TOUCH_MAP_HEADER_LEN + i * TOUCH_MAP_RECORD_LEN;
        // Infallible slicing: the length equation above guarantees these
        // ranges are in bounds.
        let packed = u32::from_le_bytes(bytes[base + 1..base + 5].try_into().unwrap());
        let size = u32::from_le_bytes(bytes[base + 5..base + 9].try_into().unwrap());
        records.push(TouchRecord {
            slot: bytes[base],
            offset: packed & 0x7FFF_FFFF,
            size,
            write: packed & 0x8000_0000 != 0,
        });
    }

    Ok(TouchMap {
        overflowed: flags & TOUCH_MAP_FLAG_OVERFLOWED != 0,
        skipped: flags & TOUCH_MAP_FLAG_SKIPPED != 0,
        records,
    })
}
