//! End-to-end test for `#[hopper::state(compact, dynamic_tail = T)]` and
//! `#[hopper::state(compact, raw_tail = true)]`: the 1-byte-header analogue of
//! the hybrid tail (`hybrid_tail_integration.rs`).
//!
//! A compact dynamic account has the wire shape `[disc:u8][fixed_head][tail]`
//! with **no** 16-byte universal header. The fixed head stays zero-copy; the
//! tail round-trips a typed (or raw) payload through the same
//! offset-parameterized tail runtime the headered path uses, only anchored at
//! `COMPACT_LEN` instead of `LEN`.

#![cfg(feature = "proc-macros")]
// These tests assert derived associated consts; the constant value of the
// assertion is precisely what is under test.
#![allow(clippy::assertions_on_constants)]

use hopper::__runtime::{write_tail_payload, ProgramError, TailCodec};
use hopper::prelude::*;

/// Fixed compact head: authority + counter. The dynamic tail carries
/// protocol metadata. Wire shape: `[disc:u8][authority:32][counter:8][tail]`.
#[derive(Copy, Clone)]
#[hopper::state(compact, disc = 7, version = 1, dynamic_tail = MarketTail)]
#[repr(C)]
pub struct CompactMarket {
    pub authority: [u8; 32],
    pub counter: WireU64,
}

/// A custom `TailCodec` payload (proves user-authored codecs slot into the
/// compact path with no macro changes).
#[derive(Debug, PartialEq, Eq)]
pub struct MarketTail {
    pub epoch: u32,
    pub flags: u8,
}

impl TailCodec for MarketTail {
    const MAX_ENCODED_LEN: usize =
        <u32 as TailCodec>::MAX_ENCODED_LEN + <u8 as TailCodec>::MAX_ENCODED_LEN;

    fn encode(&self, out: &mut [u8]) -> Result<usize, ProgramError> {
        let mut cursor = 0usize;
        cursor += self.epoch.encode(&mut out[cursor..])?;
        cursor += self.flags.encode(&mut out[cursor..])?;
        Ok(cursor)
    }

    fn decode(input: &[u8]) -> Result<(Self, usize), ProgramError> {
        let (epoch, n1) = u32::decode(input)?;
        let (flags, n2) = u8::decode(&input[n1..])?;
        Ok((MarketTail { epoch, flags }, n1 + n2))
    }
}

#[test]
fn compact_dynamic_tail_sits_right_after_one_byte_disc_and_head() {
    // Fixed head = 32 + 8 = 40. Compact prefix = 1 (disc) + 40 = 41.
    assert!(CompactMarket::HAS_DYNAMIC_TAIL);
    assert_eq!(CompactMarket::BODY_SIZE, 40);
    assert_eq!(CompactMarket::COMPACT_LEN, 41);
    // The tail prefix is at 41, NOT at 16 + 40 = 56: 15 bytes saved vs headered.
    assert_eq!(CompactMarket::TAIL_PREFIX_OFFSET, 41);
    assert_eq!(16 + 40 - CompactMarket::TAIL_PREFIX_OFFSET, 15);
}

#[test]
fn compact_market_implements_compact_dynamic_layout() {
    use hopper::account::CompactDynamicLayout;
    assert_eq!(<CompactMarket as CompactDynamicLayout>::DISC, 7);
    assert_eq!(<CompactMarket as CompactDynamicLayout>::FIXED_HEAD_SIZE, 40);
    assert_eq!(<CompactMarket as CompactDynamicLayout>::MIN_LEN, 41);
    assert_eq!(<CompactMarket as CompactDynamicLayout>::TAIL_OFFSET, 41);

    // validate_compact_dynamic accepts an oversized (tailed) buffer that the
    // fixed-length validator would reject.
    let mut buf = [0u8; 41 + 4 + 8];
    buf[0] = 7;
    assert!(<CompactMarket as CompactDynamicLayout>::validate_compact_dynamic(&buf).is_ok());
    buf[0] = 8; // wrong disc
    assert!(matches!(
        <CompactMarket as CompactDynamicLayout>::validate_compact_dynamic(&buf),
        Err(ProgramError::InvalidAccountData)
    ));
}

#[test]
fn compact_tail_write_then_read_roundtrips_typed_payload() {
    let mut data = [0u8; CompactMarket::space_for_tail(32)];
    data[0] = CompactMarket::DISC;
    let meta = MarketTail {
        epoch: 42,
        flags: 0b1010_0001,
    };
    let written = CompactMarket::tail_write(&mut data, &meta).unwrap();
    assert_eq!(written, 5); // 4 (u32 epoch) + 1 (u8 flags)
    assert_eq!(CompactMarket::tail_len(&data).unwrap(), 5);
    assert_eq!(CompactMarket::tail_read(&data).unwrap(), meta);

    // The length prefix and payload land immediately after the fixed head.
    let off = CompactMarket::TAIL_PREFIX_OFFSET;
    assert_eq!(
        u32::from_le_bytes(data[off..off + 4].try_into().unwrap()),
        5
    );
    assert_eq!(
        u32::from_le_bytes(data[off + 4..off + 8].try_into().unwrap()),
        42
    );
}

#[test]
fn compact_space_for_tail_is_disc_head_prefix_capacity() {
    // 1 (disc) + 40 (head) + 4 (prefix) + 32 (capacity) = 77.
    assert_eq!(CompactMarket::space_for_tail(32), 77);
}

/// Raw-tail compact variant: `[disc][head][u32 len][raw bytes]`, no field-level
/// tail prefix on the payload.
#[derive(Copy, Clone)]
#[hopper::state(compact, disc = 9, version = 1, raw_tail = true)]
#[repr(C)]
pub struct CompactBlob {
    pub owner: [u8; 32],
}

#[test]
fn compact_raw_tail_exposes_payload_slice() {
    assert!(CompactBlob::HAS_DYNAMIC_TAIL);
    assert!(CompactBlob::HAS_RAW_DYNAMIC_TAIL);
    assert_eq!(CompactBlob::COMPACT_LEN, 33);
    assert_eq!(CompactBlob::TAIL_PREFIX_OFFSET, 33);

    let mut data = [0u8; CompactBlob::space_for_tail(8)];
    data[0] = CompactBlob::DISC;
    // Write a raw payload through the runtime helper at the compact tail offset.
    let payload = [1u8, 2, 3, 0xFF];
    write_tail_payload(&mut data, CompactBlob::TAIL_PREFIX_OFFSET, &payload).unwrap();

    assert_eq!(CompactBlob::tail_len(&data).unwrap(), 4);
    assert_eq!(CompactBlob::tail_payload(&data).unwrap(), &payload);
}

/// A fixed compact layout still reports `HAS_DYNAMIC_TAIL = false`.
#[derive(Copy, Clone)]
#[hopper::state(compact, disc = 11, version = 1)]
#[repr(C)]
pub struct CompactFixed {
    pub balance: WireU64,
}

#[test]
fn fixed_compact_layout_reports_no_dynamic_tail() {
    assert!(!CompactFixed::HAS_DYNAMIC_TAIL);
}

/// The dynamic tail is part of layout identity: the same fixed head with a
/// different tail type must produce a different layout fingerprint.
#[derive(Copy, Clone)]
#[hopper::state(compact, disc = 7, version = 1, dynamic_tail = OtherTail)]
#[repr(C)]
pub struct CompactMarketOtherTail {
    pub authority: [u8; 32],
    pub counter: WireU64,
}

#[derive(Debug, PartialEq, Eq)]
pub struct OtherTail {
    pub value: u64,
}

impl TailCodec for OtherTail {
    const MAX_ENCODED_LEN: usize = <u64 as TailCodec>::MAX_ENCODED_LEN;
    fn encode(&self, out: &mut [u8]) -> Result<usize, ProgramError> {
        self.value.encode(out)
    }
    fn decode(input: &[u8]) -> Result<(Self, usize), ProgramError> {
        let (value, n) = u64::decode(input)?;
        Ok((OtherTail { value }, n))
    }
}

#[test]
fn distinct_tail_types_yield_distinct_layout_ids() {
    // Same fixed head, same disc, different tail type -> different fingerprint.
    assert_ne!(CompactMarket::LAYOUT_ID, CompactMarketOtherTail::LAYOUT_ID);
}
