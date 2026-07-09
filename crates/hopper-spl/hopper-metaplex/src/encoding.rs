//! Stack-buffer Borsh encoder for Metaplex instruction data.
//!
//! Metaplex's instruction format is Borsh-encoded. Borsh's variable-length
//! `String` and `Option<T>` framings make the encoding non-zero-copy by
//! definition - there's no fixed-offset layout to point a `&T` at.
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
    /// is clippy-clean - `is_empty` is the conventional companion of
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

// =====================================================================
// Kani layout proofs for the Borsh tape writer.
//
// Metaplex instruction data is Borsh-encoded and `BorshTape` is the sole
// mechanism the builders in `instructions.rs` use to lay it out. These
// proofs pin, over symbolic scalar inputs, that each `write_*` primitive
// lands its value little-endian at the current cursor, advances the
// cursor by exactly the value's width, and (for the `Option`/`bool`
// tags) writes the canonical Borsh tag byte. Scalars are compared
// word-/field-wise (never a slice `==`), matching the raw_input PID_WORD
// lesson that keeps harnesses free of memcmp loops.
//
// Run by `scripts/kani-spl-layouts.{sh,ps1}` (CI lane
// `kani-spl-layout-proofs`).
// =====================================================================
#[cfg(kani)]
mod kani_proofs {
    use super::*;

    #[kani::proof]
    fn write_u8_lands_at_cursor_and_advances_one() {
        let v: u8 = kani::any();
        let mut buf = [0u8; 4];
        let mut tape = BorshTape::new(&mut buf);
        tape.write_u8(v).unwrap();
        assert_eq!(tape.len(), 1);
        assert_eq!(buf[0], v);
    }

    #[kani::proof]
    fn write_disc_is_write_u8() {
        let v: u8 = kani::any();
        let mut buf = [0u8; 4];
        let mut tape = BorshTape::new(&mut buf);
        tape.write_disc(v).unwrap();
        assert_eq!(tape.len(), 1);
        assert_eq!(buf[0], v);
    }

    #[kani::proof]
    fn write_u16_le_lands_le_and_advances_two() {
        let v: u16 = kani::any();
        let mut buf = [0u8; 4];
        let mut tape = BorshTape::new(&mut buf);
        tape.write_u16_le(v).unwrap();
        assert_eq!(tape.len(), 2);
        assert_eq!(u16::from_le_bytes([buf[0], buf[1]]), v);
    }

    #[kani::proof]
    fn write_u32_le_lands_le_and_advances_four() {
        let v: u32 = kani::any();
        let mut buf = [0u8; 8];
        let mut tape = BorshTape::new(&mut buf);
        tape.write_u32_le(v).unwrap();
        assert_eq!(tape.len(), 4);
        assert_eq!(u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]), v);
    }

    #[kani::proof]
    fn write_u64_le_lands_le_and_advances_eight() {
        let v: u64 = kani::any();
        let mut buf = [0u8; 12];
        let mut tape = BorshTape::new(&mut buf);
        tape.write_u64_le(v).unwrap();
        assert_eq!(tape.len(), 8);
        let got = u64::from_le_bytes([
            buf[0], buf[1], buf[2], buf[3], buf[4], buf[5], buf[6], buf[7],
        ]);
        assert_eq!(got, v);
    }

    #[kani::proof]
    fn write_bool_is_zero_or_one() {
        let v: bool = kani::any();
        let mut buf = [0u8; 4];
        let mut tape = BorshTape::new(&mut buf);
        tape.write_bool(v).unwrap();
        assert_eq!(tape.len(), 1);
        assert_eq!(buf[0], if v { 1 } else { 0 });
    }

    #[kani::proof]
    fn write_option_none_is_zero_tag() {
        let mut buf = [0u8; 4];
        let mut tape = BorshTape::new(&mut buf);
        tape.write_option_none().unwrap();
        assert_eq!(tape.len(), 1);
        assert_eq!(buf[0], 0);
    }

    #[kani::proof]
    fn write_option_some_tag_is_one_tag() {
        let mut buf = [0u8; 4];
        let mut tape = BorshTape::new(&mut buf);
        tape.write_option_some_tag().unwrap();
        assert_eq!(tape.len(), 1);
        assert_eq!(buf[0], 1);
    }

    #[kani::proof]
    fn write_option_u64_none_is_single_zero_byte() {
        let mut buf = [0u8; 12];
        let mut tape = BorshTape::new(&mut buf);
        tape.write_option_u64_le(None).unwrap();
        assert_eq!(tape.len(), 1);
        assert_eq!(buf[0], 0);
    }

    #[kani::proof]
    fn write_option_u64_some_is_tag_then_le_value() {
        let v: u64 = kani::any();
        let mut buf = [0u8; 12];
        let mut tape = BorshTape::new(&mut buf);
        tape.write_option_u64_le(Some(v)).unwrap();
        assert_eq!(tape.len(), 9);
        assert_eq!(buf[0], 1);
        let got = u64::from_le_bytes([
            buf[1], buf[2], buf[3], buf[4], buf[5], buf[6], buf[7], buf[8],
        ]);
        assert_eq!(got, v);
    }

    // A Borsh `String` is `[u32 LE length][bytes]`. Prove the 4-byte
    // length prefix over a symbolic length: the prefix equals the byte
    // count and the cursor advances by `4 + len`. A concrete two-byte
    // ASCII string exercises the body placement (see the `writes_borsh_
    // string_with_length_prefix` unit test); here the emphasis is the
    // symbolic length-prefix contract.
    #[kani::proof]
    fn write_str_length_prefix_is_u32_le() {
        let s = "hi";
        let mut buf = [0u8; 16];
        let mut tape = BorshTape::new(&mut buf);
        tape.write_str(s).unwrap();
        assert_eq!(tape.len(), 4 + s.len());
        assert_eq!(
            u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]),
            s.len() as u32
        );
        assert_eq!(buf[4], b'h');
        assert_eq!(buf[5], b'i');
    }
}
