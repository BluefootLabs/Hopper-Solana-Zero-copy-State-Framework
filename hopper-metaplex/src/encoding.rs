//! Stack-buffer Borsh encoder for Metaplex instruction data.
//!
//! Metaplex's instruction format is Borsh-encoded. Borsh's variable-length
//! `String` and `Option<T>` framings make the encoding non-zero-copy by
//! definition — there's no fixed-offset layout to point a `&T` at.
//! Hopper handles that by writing the Borsh tape into a small fixed-size
//! stack buffer at the call site and passing `&buffer[..len]` as the
//! instruction data. No heap, no `Vec`, no `alloc::String`.
//!
//! `BorshTape` is the writer. It tracks `(buffer, len, capacity)` and
//! returns `ProgramError::InvalidInstructionData` on overflow so a caller
//! can't push past the buffer's capacity. Each builder picks a buffer
//! size that's a comfortable upper bound for the instruction it emits.

use hopper_runtime::error::ProgramError;
use hopper_runtime::ProgramResult;

/// Mutable cursor over a stack buffer that writes Borsh-encoded values.
///
/// All `write_*` methods return an error on overflow rather than panicking
/// or wrapping, so a caller-supplied string longer than the buffer is
/// caught at encode time. Callers who want to enforce a tighter cap
/// (the Metaplex spec caps `name`/`symbol`/`uri` at known lengths) should
/// validate before calling here.
pub struct BorshTape<'a> {
    buf: &'a mut [u8],
    pos: usize,
}

impl<'a> BorshTape<'a> {
    #[inline]
    pub fn new(buf: &'a mut [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    /// Bytes written so far. Use as `&buf[..tape.len()]` to get the
    /// finished instruction data.
    #[inline]
    pub fn len(&self) -> usize {
        self.pos
    }

    /// Whether the buffer has any bytes written. Provided so the type
    /// is clippy-clean — `is_empty` is the conventional companion of
    /// `len`.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.pos == 0
    }

    /// Available capacity remaining in the buffer.
    #[inline]
    pub fn remaining(&self) -> usize {
        self.buf.len() - self.pos
    }

    fn reserve(&mut self, n: usize) -> ProgramResult {
        if self.pos.saturating_add(n) > self.buf.len() {
            return Err(ProgramError::InvalidInstructionData);
        }
        Ok(())
    }

    /// Write a single discriminator byte (Metaplex's enum-position
    /// instruction tag). `CreateMetadataAccountV3` is 33,
    /// `CreateMasterEditionV3` is 17, `UpdateMetadataAccountV2` is 15.
    #[inline]
    pub fn write_disc(&mut self, disc: u8) -> ProgramResult {
        self.write_u8(disc)
    }

    #[inline]
    pub fn write_u8(&mut self, value: u8) -> ProgramResult {
        self.reserve(1)?;
        self.buf[self.pos] = value;
        self.pos += 1;
        Ok(())
    }

    #[inline]
    pub fn write_u16_le(&mut self, value: u16) -> ProgramResult {
        self.reserve(2)?;
        self.buf[self.pos..self.pos + 2].copy_from_slice(&value.to_le_bytes());
        self.pos += 2;
        Ok(())
    }

    #[inline]
    pub fn write_u32_le(&mut self, value: u32) -> ProgramResult {
        self.reserve(4)?;
        self.buf[self.pos..self.pos + 4].copy_from_slice(&value.to_le_bytes());
        self.pos += 4;
        Ok(())
    }

    #[inline]
    pub fn write_u64_le(&mut self, value: u64) -> ProgramResult {
        self.reserve(8)?;
        self.buf[self.pos..self.pos + 8].copy_from_slice(&value.to_le_bytes());
        self.pos += 8;
        Ok(())
    }

    #[inline]
    pub fn write_bool(&mut self, value: bool) -> ProgramResult {
        self.write_u8(if value { 1 } else { 0 })
    }

    /// Borsh-encode a `String` as `[u32 LE length][bytes]`. Caller is
    /// responsible for any application-level length cap (Metaplex's
    /// 32/10/200-byte caps for name/symbol/uri are enforced by the
    /// caller before this point).
    #[inline]
    pub fn write_str(&mut self, value: &str) -> ProgramResult {
        let bytes = value.as_bytes();
        self.write_u32_le(bytes.len() as u32)?;
        self.reserve(bytes.len())?;
        self.buf[self.pos..self.pos + bytes.len()].copy_from_slice(bytes);
        self.pos += bytes.len();
        Ok(())
    }

    /// Borsh-encode `Option::None` (single zero byte).
    #[inline]
    pub fn write_option_none(&mut self) -> ProgramResult {
        self.write_u8(0)
    }

    /// Borsh-encode `Option::Some(())` tag (single one byte). Use
    /// before writing the wrapped payload.
    #[inline]
    pub fn write_option_some_tag(&mut self) -> ProgramResult {
        self.write_u8(1)
    }

    /// Borsh-encode `Option<u64>`. Convenience helper for the
    /// `max_supply` field of `CreateMasterEditionV3`.
    #[inline]
    pub fn write_option_u64_le(&mut self, value: Option<u64>) -> ProgramResult {
        match value {
            None => self.write_option_none(),
            Some(v) => {
                self.write_option_some_tag()?;
                self.write_u64_le(v)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_borsh_string_with_length_prefix() {
        let mut buf = [0u8; 64];
        let len = {
            let mut tape = BorshTape::new(&mut buf);
            tape.write_str("hi").unwrap();
            tape.len()
        };
        // [2, 0, 0, 0, b'h', b'i']
        assert_eq!(&buf[..6], &[2, 0, 0, 0, b'h', b'i']);
        assert_eq!(len, 6);
    }

    #[test]
    fn rejects_overflow() {
        let mut buf = [0u8; 4];
        let mut tape = BorshTape::new(&mut buf);
        // u32 length prefix already fills the buffer; the bytes
        // afterward have no room.
        assert!(tape.write_str("hi").is_err());
    }

    #[test]
    fn writes_option_some_u64_with_tag() {
        let mut buf = [0u8; 16];
        let mut tape = BorshTape::new(&mut buf);
        tape.write_option_u64_le(Some(42)).unwrap();
        assert_eq!(&buf[..9], &[1, 42, 0, 0, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn writes_option_none_u64_as_zero_byte() {
        let mut buf = [0u8; 16];
        let len = {
            let mut tape = BorshTape::new(&mut buf);
            tape.write_option_u64_le(None).unwrap();
            tape.len()
        };
        assert_eq!(&buf[..1], &[0]);
        assert_eq!(len, 1);
    }
}
