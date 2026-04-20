//! Hybrid serialization tail for `#[hopper::state(dynamic_tail = T)]`.
//!
//! Closes Hopper Safety Audit innovation I5 ("Hybrid serialization").
//! The rationale from the audit (page 14):
//!
//! > Lets Hopper own the fixed-layout hot path while still supporting a
//! > dynamic tail for vectors, strings, and optional metadata.
//!
//! # Wire format
//!
//! After the layout's fixed body (offset `TYPE_OFFSET + WIRE_SIZE`), the
//! tail is encoded as:
//!
//! ```text
//! [ len: u32 LE ] [ payload: len bytes ]
//! ```
//!
//! The fixed-body fast path remains fully zero-copy. code that never
//! touches the tail pays zero overhead. Tail access is explicit
//! (`tail_read::<T>()` / `tail_write::<T>()`), which is why the tail
//! is **not** zero-copy: the typed representation is reconstructed on
//! read and serialized on write.
//!
//! # Canonical tail encoding (`TailCodec`)
//!
//! `TailCodec` is a minimal Borsh-subset serializer:
//!
//! * integers: native little-endian
//! * `[u8; N]`: raw bytes, fixed width
//! * `Vec<u8>` / byte slices: u32 LE length prefix + bytes
//! * strings: u32 LE length prefix + UTF-8
//! * `Option<T>`: 1-byte tag (0 = None, 1 = Some) + inner payload
//!
//! Programs that need richer types (custom structs, `Vec<T>`) implement
//! `TailCodec` themselves; the framework does not force a derive.

use crate::error::ProgramError;

/// Canonical serializer for dynamic-tail payloads.
///
/// Implementations encode into a caller-provided buffer and decode
/// from a caller-provided slice, returning the byte count consumed
/// in both directions. Byte counts drive the length-prefix handling
/// inside `#[hopper::state]`'s generated tail accessors. the
/// encoding must be deterministic and bidirectional.
pub trait TailCodec: Sized {
    /// Upper bound on the encoded size. Used by generated helpers to
    /// verify the account has enough room before invoking `encode`.
    /// Implementors should pick the smallest valid bound. Hopper
    /// uses this to pre-size reallocs.
    const MAX_ENCODED_LEN: usize;

    /// Serialize `self` into `out`. Returns the number of bytes
    /// written (always `<= MAX_ENCODED_LEN`). Fails with
    /// `AccountDataTooSmall` when `out.len() < encoded_len`.
    fn encode(&self, out: &mut [u8]) -> Result<usize, ProgramError>;

    /// Deserialize from `input`. Returns `(value, bytes_consumed)`.
    /// Fails with `InvalidAccountData` on malformed encoding.
    fn decode(input: &[u8]) -> Result<(Self, usize), ProgramError>;
}

// ── Primitive impls (little-endian, fixed width) ────────────────────

impl TailCodec for u8 {
    const MAX_ENCODED_LEN: usize = 1;
    #[inline]
    fn encode(&self, out: &mut [u8]) -> Result<usize, ProgramError> {
        if out.is_empty() {
            return Err(ProgramError::AccountDataTooSmall);
        }
        out[0] = *self;
        Ok(1)
    }
    #[inline]
    fn decode(input: &[u8]) -> Result<(Self, usize), ProgramError> {
        input
            .first()
            .copied()
            .map(|b| (b, 1))
            .ok_or(ProgramError::InvalidAccountData)
    }
}

macro_rules! tail_codec_int {
    ( $( $ty:ty : $n:expr ),+ $(,)? ) => {
        $(
            impl TailCodec for $ty {
                const MAX_ENCODED_LEN: usize = $n;
                #[inline]
                fn encode(&self, out: &mut [u8]) -> Result<usize, ProgramError> {
                    if out.len() < $n {
                        return Err(ProgramError::AccountDataTooSmall);
                    }
                    out[..$n].copy_from_slice(&self.to_le_bytes());
                    Ok($n)
                }
                #[inline]
                fn decode(input: &[u8]) -> Result<(Self, usize), ProgramError> {
                    if input.len() < $n {
                        return Err(ProgramError::InvalidAccountData);
                    }
                    let mut bytes = [0u8; $n];
                    bytes.copy_from_slice(&input[..$n]);
                    Ok((Self::from_le_bytes(bytes), $n))
                }
            }
        )+
    };
}

tail_codec_int! {
    u16: 2, u32: 4, u64: 8, u128: 16,
    i16: 2, i32: 4, i64: 8, i128: 16,
}

// `bool` as 1 byte (0 = false, 1 = true; anything else rejected).
impl TailCodec for bool {
    const MAX_ENCODED_LEN: usize = 1;
    #[inline]
    fn encode(&self, out: &mut [u8]) -> Result<usize, ProgramError> {
        if out.is_empty() {
            return Err(ProgramError::AccountDataTooSmall);
        }
        out[0] = if *self { 1 } else { 0 };
        Ok(1)
    }
    #[inline]
    fn decode(input: &[u8]) -> Result<(Self, usize), ProgramError> {
        match input.first().copied() {
            Some(0) => Ok((false, 1)),
            Some(1) => Ok((true, 1)),
            _ => Err(ProgramError::InvalidAccountData),
        }
    }
}

// `[u8; N]`. raw fixed-width bytes.
impl<const N: usize> TailCodec for [u8; N] {
    const MAX_ENCODED_LEN: usize = N;
    #[inline]
    fn encode(&self, out: &mut [u8]) -> Result<usize, ProgramError> {
        if out.len() < N {
            return Err(ProgramError::AccountDataTooSmall);
        }
        out[..N].copy_from_slice(self);
        Ok(N)
    }
    #[inline]
    fn decode(input: &[u8]) -> Result<(Self, usize), ProgramError> {
        if input.len() < N {
            return Err(ProgramError::InvalidAccountData);
        }
        let mut out = [0u8; N];
        out.copy_from_slice(&input[..N]);
        Ok((out, N))
    }
}

// `Option<T>`. 1-byte tag + inner payload when present.
impl<T: TailCodec> TailCodec for Option<T> {
    const MAX_ENCODED_LEN: usize = 1 + T::MAX_ENCODED_LEN;
    #[inline]
    fn encode(&self, out: &mut [u8]) -> Result<usize, ProgramError> {
        if out.is_empty() {
            return Err(ProgramError::AccountDataTooSmall);
        }
        match self {
            None => {
                out[0] = 0;
                Ok(1)
            }
            Some(inner) => {
                out[0] = 1;
                let written = inner.encode(&mut out[1..])?;
                Ok(1 + written)
            }
        }
    }
    #[inline]
    fn decode(input: &[u8]) -> Result<(Self, usize), ProgramError> {
        match input.first().copied() {
            Some(0) => Ok((None, 1)),
            Some(1) => {
                let (inner, n) = T::decode(&input[1..])?;
                Ok((Some(inner), 1 + n))
            }
            _ => Err(ProgramError::InvalidAccountData),
        }
    }
}

// ── Framework helpers used by `#[hopper::state(dynamic_tail = T)]` ──

/// Read the tail's u32-LE length prefix.
///
/// `body_end` is the byte offset immediately after the layout's fixed
/// body (i.e. `TYPE_OFFSET + WIRE_SIZE` for a layout with no header
/// beyond the 16-byte Hopper prefix, otherwise `HEADER_LEN +
/// WIRE_SIZE`). Returns `AccountDataTooSmall` if the account has
/// fewer than 4 tail bytes available.
#[inline]
pub fn read_tail_len(data: &[u8], body_end: usize) -> Result<u32, ProgramError> {
    let end = body_end
        .checked_add(4)
        .ok_or(ProgramError::AccountDataTooSmall)?;
    if data.len() < end {
        return Err(ProgramError::AccountDataTooSmall);
    }
    let mut bytes = [0u8; 4];
    bytes.copy_from_slice(&data[body_end..end]);
    Ok(u32::from_le_bytes(bytes))
}

/// Return a slice referencing just the tail payload bytes (excluding
/// the 4-byte length prefix). Length-bounded by the u32 prefix.
#[inline]
pub fn tail_payload(data: &[u8], body_end: usize) -> Result<&[u8], ProgramError> {
    let len = read_tail_len(data, body_end)? as usize;
    let start = body_end + 4;
    let end = start.checked_add(len).ok_or(ProgramError::InvalidAccountData)?;
    if data.len() < end {
        return Err(ProgramError::InvalidAccountData);
    }
    Ok(&data[start..end])
}

/// Decode the tail as `T: TailCodec`, checking that the encoded length
/// exactly matches the u32 prefix. Extra bytes beyond `T`'s decode
/// are a malformed-encoding signal.
#[inline]
pub fn read_tail<T: TailCodec>(
    data: &[u8],
    body_end: usize,
) -> Result<T, ProgramError> {
    let payload = tail_payload(data, body_end)?;
    let (value, consumed) = T::decode(payload)?;
    if consumed != payload.len() {
        return Err(ProgramError::InvalidAccountData);
    }
    Ok(value)
}

/// Encode `tail` into the account's tail slot, rewriting the u32
/// length prefix. Returns `AccountDataTooSmall` when the existing
/// account byte buffer can't fit the encoded payload. in that case
/// the caller should `realloc` first.
#[inline]
pub fn write_tail<T: TailCodec>(
    data: &mut [u8],
    body_end: usize,
    tail: &T,
) -> Result<usize, ProgramError> {
    let prefix_end = body_end
        .checked_add(4)
        .ok_or(ProgramError::AccountDataTooSmall)?;
    if data.len() < prefix_end {
        return Err(ProgramError::AccountDataTooSmall);
    }
    let written = tail.encode(&mut data[prefix_end..])?;
    if written > u32::MAX as usize {
        return Err(ProgramError::InvalidAccountData);
    }
    data[body_end..prefix_end].copy_from_slice(&(written as u32).to_le_bytes());
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn u32_roundtrip() {
        let mut buf = [0u8; 8];
        let n = 0xDEAD_BEEFu32.encode(&mut buf).unwrap();
        assert_eq!(n, 4);
        let (back, consumed) = u32::decode(&buf).unwrap();
        assert_eq!(consumed, 4);
        assert_eq!(back, 0xDEAD_BEEF);
    }

    #[test]
    fn u64_roundtrip() {
        let mut buf = [0u8; 8];
        0x0123_4567_89AB_CDEFu64.encode(&mut buf).unwrap();
        let (back, _) = u64::decode(&buf).unwrap();
        assert_eq!(back, 0x0123_4567_89AB_CDEF);
    }

    #[test]
    fn bool_encode_decode() {
        let mut buf = [0u8; 1];
        true.encode(&mut buf).unwrap();
        assert_eq!(buf[0], 1);
        assert_eq!(bool::decode(&buf).unwrap(), (true, 1));
        false.encode(&mut buf).unwrap();
        assert_eq!(buf[0], 0);
        assert_eq!(bool::decode(&buf).unwrap(), (false, 1));
    }

    #[test]
    fn bool_rejects_garbage() {
        let buf = [2u8];
        assert!(bool::decode(&buf).is_err());
    }

    #[test]
    fn byte_array_roundtrip() {
        let src: [u8; 8] = *b"HOPPER!!";
        let mut buf = [0u8; 16];
        let n = src.encode(&mut buf).unwrap();
        assert_eq!(n, 8);
        let (back, consumed) = <[u8; 8]>::decode(&buf).unwrap();
        assert_eq!(consumed, 8);
        assert_eq!(back, src);
    }

    #[test]
    fn option_none_encodes_to_one_byte() {
        let mut buf = [0u8; 16];
        let n = Option::<u64>::None.encode(&mut buf).unwrap();
        assert_eq!(n, 1);
        assert_eq!(buf[0], 0);
        let (back, c) = <Option<u64>>::decode(&buf).unwrap();
        assert_eq!(back, None);
        assert_eq!(c, 1);
    }

    #[test]
    fn option_some_includes_inner_payload() {
        let mut buf = [0u8; 16];
        let n = Option::<u64>::Some(0xAAAA_BBBB_CCCC_DDDD).encode(&mut buf).unwrap();
        assert_eq!(n, 9);
        assert_eq!(buf[0], 1);
        let (back, c) = <Option<u64>>::decode(&buf).unwrap();
        assert_eq!(back, Some(0xAAAA_BBBB_CCCC_DDDD));
        assert_eq!(c, 9);
    }

    #[test]
    fn option_rejects_invalid_tag() {
        let buf = [7u8, 0, 0, 0, 0, 0, 0, 0, 0];
        assert!(<Option<u64>>::decode(&buf).is_err());
    }

    #[test]
    fn tail_length_prefix_roundtrip() {
        // Simulate an account body: 16-byte "header" + 8-byte body +
        // 4-byte length prefix + tail bytes. body_end = 24.
        let mut data = [0u8; 64];
        let body_end = 24usize;
        let tail_value: u64 = 0x1234_5678_9ABC_DEF0;
        let written = write_tail(&mut data, body_end, &tail_value).unwrap();
        assert_eq!(written, 8);
        let read_len = read_tail_len(&data, body_end).unwrap();
        assert_eq!(read_len, 8);
        let back: u64 = read_tail::<u64>(&data, body_end).unwrap();
        assert_eq!(back, tail_value);
    }

    #[test]
    fn tail_decode_rejects_excess_payload() {
        // If the tail encodes as 4 bytes but the length prefix claims
        // 8, the decode must refuse rather than silently succeed.
        let mut data = [0u8; 32];
        // body_end = 16; prefix says 8 bytes; payload is u32 (4 bytes) +
        // garbage (4 bytes). Decoding as u32 leaves 4 bytes unconsumed
        // which is caught by `read_tail`.
        let body_end = 16usize;
        data[body_end..body_end + 4].copy_from_slice(&8u32.to_le_bytes());
        // Fill payload with something that decodes as u32=0x11223344
        // and then trailing garbage.
        data[body_end + 4..body_end + 8].copy_from_slice(&0x1122_3344u32.to_le_bytes());
        data[body_end + 8..body_end + 12].copy_from_slice(&0xFFu32.to_le_bytes());
        // u32 decodes 4 bytes but prefix claims 8. expect error.
        let result = read_tail::<u32>(&data, body_end);
        assert!(result.is_err());
    }

    #[test]
    fn tail_bounds_check_on_truncated_buffer() {
        let data = [0u8; 10];
        assert!(read_tail_len(&data, 16).is_err());
        assert!(tail_payload(&data, 16).is_err());
    }

    #[test]
    fn max_encoded_len_matches_actual_encode_size() {
        let mut buf = [0u8; 32];
        assert_eq!(0u32.encode(&mut buf).unwrap(), u32::MAX_ENCODED_LEN);
        assert_eq!(0u64.encode(&mut buf).unwrap(), u64::MAX_ENCODED_LEN);
        assert_eq!(true.encode(&mut buf).unwrap(), bool::MAX_ENCODED_LEN);
        assert_eq!([0u8; 7].encode(&mut buf).unwrap(), <[u8; 7]>::MAX_ENCODED_LEN);
        assert_eq!(Option::<u32>::None.encode(&mut buf).unwrap(), 1);
        assert_eq!(
            Option::<u32>::Some(0).encode(&mut buf).unwrap(),
            <Option<u32>>::MAX_ENCODED_LEN
        );
    }
}
