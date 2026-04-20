//! End-to-end test for `#[hopper::state(dynamic_tail = T)]`. audit
//! innovation I5 (hybrid serialization).
//!
//! The declared layout stays fully zero-copy for its fixed body while
//! gaining `tail_len`, `tail_read`, and `tail_write` helpers that
//! round-trip a typed dynamic payload through the length-prefixed tail
//! slot defined in `hopper_runtime::tail`.

#![cfg(feature = "proc-macros")]

use hopper::__runtime::{ProgramError, TailCodec};
use hopper::prelude::*;

/// Fixed-body layout: authority + counter, nothing else in the hot
/// path. The dynamic tail carries optional protocol-specific metadata
/// encoded via the `TailCodec` Borsh-subset.
#[hopper::state(disc = 99, version = 1, dynamic_tail = VaultMetadata)]
#[derive(Copy, Clone)]
#[repr(C)]
pub struct MetadataVault {
    pub authority: [u8; 32],
    pub counter: WireU64,
}

/// A custom `TailCodec` payload. Real protocols would typically
/// stick to the primitive impls in `hopper_runtime::tail`; this
/// test exists to prove that user-authored codecs slot in without
/// macro changes.
#[derive(Debug, PartialEq, Eq)]
pub struct VaultMetadata {
    pub epoch: u32,
    pub paused: bool,
    pub operator_tag: [u8; 8],
}

impl TailCodec for VaultMetadata {
    const MAX_ENCODED_LEN: usize =
        <u32 as TailCodec>::MAX_ENCODED_LEN
            + <bool as TailCodec>::MAX_ENCODED_LEN
            + <[u8; 8] as TailCodec>::MAX_ENCODED_LEN;

    fn encode(&self, out: &mut [u8]) -> Result<usize, ProgramError> {
        let mut cursor = 0usize;
        cursor += self.epoch.encode(&mut out[cursor..])?;
        cursor += self.paused.encode(&mut out[cursor..])?;
        cursor += self.operator_tag.encode(&mut out[cursor..])?;
        Ok(cursor)
    }

    fn decode(input: &[u8]) -> Result<(Self, usize), ProgramError> {
        let (epoch, n1) = u32::decode(input)?;
        let (paused, n2) = bool::decode(&input[n1..])?;
        let (operator_tag, n3) = <[u8; 8]>::decode(&input[n1 + n2..])?;
        Ok((
            VaultMetadata {
                epoch,
                paused,
                operator_tag,
            },
            n1 + n2 + n3,
        ))
    }
}

#[test]
fn layout_reports_has_dynamic_tail_true() {
    assert!(MetadataVault::HAS_DYNAMIC_TAIL);
}

#[test]
fn tail_prefix_offset_sits_after_fixed_body() {
    // Layout body is 32 + 8 = 40 bytes; add the 16-byte header = 56.
    // That's where the u32 length prefix lives.
    assert_eq!(
        MetadataVault::TAIL_PREFIX_OFFSET,
        MetadataVault::LEN
    );
    assert_eq!(MetadataVault::TAIL_PREFIX_OFFSET, 16 + 32 + 8);
}

#[test]
fn tail_write_then_read_roundtrips_typed_payload() {
    // Simulate an account buffer big enough for header + body + tail.
    let mut data = [0u8; 128];
    let meta = VaultMetadata {
        epoch: 42,
        paused: true,
        operator_tag: *b"hopperxx",
    };
    let written = MetadataVault::tail_write(&mut data, &meta).unwrap();
    // 4 (u32) + 1 (bool) + 8 (array) = 13 bytes
    assert_eq!(written, 13);
    // Length prefix reflects the written count.
    assert_eq!(MetadataVault::tail_len(&data).unwrap(), 13);
    // Round-trip decode matches the original.
    let back = MetadataVault::tail_read(&data).unwrap();
    assert_eq!(back, meta);
}

#[test]
fn tail_len_on_freshly_zeroed_buffer_is_zero() {
    let data = [0u8; 128];
    // A zeroed account has length prefix = 0 (no tail payload yet).
    assert_eq!(MetadataVault::tail_len(&data).unwrap(), 0);
}

#[test]
fn tail_read_rejects_truncated_buffer() {
    // Buffer too small for even the length prefix.
    let data = [0u8; 32];
    let err = MetadataVault::tail_len(&data).unwrap_err();
    assert!(matches!(err, ProgramError::AccountDataTooSmall));
}

#[test]
fn tail_write_updates_prefix_then_payload() {
    let mut data = [0u8; 128];
    let meta = VaultMetadata {
        epoch: 0xDEAD_BEEF,
        paused: false,
        operator_tag: [0x11; 8],
    };
    MetadataVault::tail_write(&mut data, &meta).unwrap();
    let prefix_off = MetadataVault::TAIL_PREFIX_OFFSET;
    // Prefix bytes should encode the written length as LE u32.
    let mut prefix_bytes = [0u8; 4];
    prefix_bytes.copy_from_slice(&data[prefix_off..prefix_off + 4]);
    assert_eq!(u32::from_le_bytes(prefix_bytes), 13);
    // First 4 payload bytes should be the epoch LE.
    let mut epoch_bytes = [0u8; 4];
    epoch_bytes.copy_from_slice(&data[prefix_off + 4..prefix_off + 8]);
    assert_eq!(u32::from_le_bytes(epoch_bytes), 0xDEAD_BEEF);
}

#[test]
fn tail_rewrite_overwrites_previous_payload() {
    let mut data = [0u8; 128];
    let first = VaultMetadata {
        epoch: 1,
        paused: false,
        operator_tag: [1u8; 8],
    };
    MetadataVault::tail_write(&mut data, &first).unwrap();
    let second = VaultMetadata {
        epoch: 999,
        paused: true,
        operator_tag: [9u8; 8],
    };
    MetadataVault::tail_write(&mut data, &second).unwrap();
    assert_eq!(MetadataVault::tail_read(&data).unwrap(), second);
}

/// Confirmation that layouts *without* `dynamic_tail = ...` still
/// expose a `HAS_DYNAMIC_TAIL = false` const for branching logic.
#[hopper::state(disc = 100, version = 1)]
#[derive(Copy, Clone)]
#[repr(C)]
pub struct PlainVault {
    pub counter: WireU64,
}

#[test]
fn plain_layout_reports_has_dynamic_tail_false() {
    assert!(!PlainVault::HAS_DYNAMIC_TAIL);
}
