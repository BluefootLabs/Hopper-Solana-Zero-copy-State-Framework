//! Touch-map v1 decode: hand-built wire vectors + the flags/attribution
//! semantics that force an INCONCLUSIVE verdict on a partial map.

use grillo_verifier::{
    decode_touch_map, DecodeError, TouchRecord, TOUCH_MAP_FLAG_OVERFLOWED, TOUCH_MAP_FLAG_SKIPPED,
    TOUCH_MAP_MAGIC, TOUCH_MAP_VERSION,
};

/// Build a wire-format touch map from `(slot, offset, size, write)` tuples,
/// exactly as the runtime's `encode_touch_map` and the `hopper tx explain`
/// tests do.
fn encode(records: &[(u8, u32, u32, bool)], flags: u8) -> Vec<u8> {
    let mut bytes = vec![
        TOUCH_MAP_MAGIC,
        TOUCH_MAP_VERSION,
        flags,
        records.len() as u8,
    ];
    for &(slot, offset, size, write) in records {
        bytes.push(slot);
        let packed = offset | if write { 0x8000_0000 } else { 0 };
        bytes.extend_from_slice(&packed.to_le_bytes());
        bytes.extend_from_slice(&size.to_le_bytes());
    }
    bytes
}

#[test]
fn decodes_a_synthetic_map_matching_hand_built_records() {
    let bytes = encode(&[(1, 114, 1, true), (1, 115, 8, true)], 0);
    let map = decode_touch_map(&bytes).unwrap();
    assert!(!map.overflowed);
    assert!(!map.skipped);
    assert!(!map.is_partial());
    assert_eq!(
        map.records,
        vec![
            TouchRecord {
                slot: 1,
                offset: 114,
                size: 1,
                write: true
            },
            TouchRecord {
                slot: 1,
                offset: 115,
                size: 8,
                write: true
            },
        ]
    );
    let writes: Vec<_> = map.writes().copied().collect();
    assert_eq!(writes.len(), 2, "both records are writes");
}

#[test]
fn decodes_an_empty_map() {
    let bytes = encode(&[], 0);
    assert_eq!(bytes, vec![0x7A, 0x01, 0x00, 0x00]);
    let map = decode_touch_map(&bytes).unwrap();
    assert!(map.records.is_empty());
    assert!(!map.is_partial());
}

#[test]
fn write_bit_and_offset_masking_are_independent() {
    // A read at the maximum 31-bit offset: bit31 clear, low 31 bits kept.
    let read = encode(&[(0, 0x7FFF_FFFF, 4, false)], 0);
    let m = decode_touch_map(&read).unwrap();
    assert_eq!(m.records[0].offset, 0x7FFF_FFFF);
    assert!(!m.records[0].write);

    // A write at a small offset: bit31 set, offset recovered by masking.
    let write = encode(&[(0, 5, 4, true)], 0);
    let m = decode_touch_map(&write).unwrap();
    assert_eq!(m.records[0].offset, 5);
    assert!(m.records[0].write);
}

#[test]
fn rejects_bad_magic_version_and_length() {
    let good = encode(&[(0, 8, 8, false)], 0);
    assert!(decode_touch_map(&good).is_ok());

    let mut bad_magic = good.clone();
    bad_magic[0] = 0x7B;
    assert_eq!(
        decode_touch_map(&bad_magic),
        Err(DecodeError::BadMagic(0x7B))
    );

    let mut bad_version = good.clone();
    bad_version[1] = 0x02;
    assert_eq!(
        decode_touch_map(&bad_version),
        Err(DecodeError::BadVersion(0x02))
    );

    // Exact-length equation: truncation and padding both fail (this is what
    // keeps Anchor event payloads out).
    assert!(matches!(
        decode_touch_map(&good[..good.len() - 1]),
        Err(DecodeError::LengthMismatch { .. })
    ));
    let mut padded = good.clone();
    padded.push(0);
    assert!(matches!(
        decode_touch_map(&padded),
        Err(DecodeError::LengthMismatch { .. })
    ));

    assert!(matches!(
        decode_touch_map(&[]),
        Err(DecodeError::TooShort { len: 0 })
    ));
    // A byte string that happens to start 0x7A 0x01 but isn't a map.
    assert!(decode_touch_map(b"\x7a\x01junk").is_err());
}

#[test]
fn overflowed_and_skipped_flags_mark_a_partial_map() {
    let overflow =
        decode_touch_map(&encode(&[(0, 0, 1, true)], TOUCH_MAP_FLAG_OVERFLOWED)).unwrap();
    assert!(overflow.overflowed);
    assert!(!overflow.skipped);
    assert!(overflow.is_partial(), "overflow => partial");

    let skip = decode_touch_map(&encode(&[(0, 0, 1, true)], TOUCH_MAP_FLAG_SKIPPED)).unwrap();
    assert!(skip.skipped);
    assert!(skip.is_partial(), "skip => partial");

    let both = decode_touch_map(&encode(
        &[(0, 0, 1, true)],
        TOUCH_MAP_FLAG_OVERFLOWED | TOUCH_MAP_FLAG_SKIPPED,
    ))
    .unwrap();
    assert!(both.overflowed && both.skipped && both.is_partial());
}
