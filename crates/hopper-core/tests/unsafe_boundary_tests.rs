//! # Unsafe Boundary Tests
//!
//! Exhaustive tests for every unsafe boundary in Hopper:
//! - Pod/FixedLayout overlay casts with wrong-size, truncated, oversized data
//! - VerifiedAccount rejection of undersized buffers
//! - Header version / fingerprint / discriminator mismatch detection
//! - Segment descriptor boundary conditions
//! - Overlay-at out-of-bounds rejection
//! - Unchecked cast contracts

extern crate alloc;

use hopper_core::account::{
    pod_from_bytes, pod_from_bytes_mut, pod_read, pod_write,
    VerifiedAccount, VerifiedAccountMut,
    AccountHeader, HEADER_LEN,
    FixedLayout, Pod,
    SegmentDescriptor, SegmentTable, SEGMENT_DESC_SIZE,
};
use hopper_core::abi::*;
use hopper_runtime::error::ProgramError;

// =====================================================================
// Test fixtures
// =====================================================================

/// A tiny Pod struct for testing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(C)]
struct TinyPod {
    a: u8,
    b: u8,
}

const _: () = assert!(core::mem::size_of::<TinyPod>() == 2);
const _: () = assert!(core::mem::align_of::<TinyPod>() == 1);

unsafe impl ::hopper_runtime::__hopper_native::bytemuck::Zeroable for TinyPod {}
unsafe impl ::hopper_runtime::__hopper_native::bytemuck::Pod for TinyPod {}
unsafe impl Pod for TinyPod {}
impl FixedLayout for TinyPod {
    const SIZE: usize = 2;
}

/// A 32-byte Pod struct matching a typical layout body.
#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(C)]
struct MockLayout {
    header: [u8; 16],  // simulated header area
    balance: [u8; 8],
    owner: [u8; 8],
}

const _: () = assert!(core::mem::size_of::<MockLayout>() == 32);
const _: () = assert!(core::mem::align_of::<MockLayout>() == 1);

unsafe impl ::hopper_runtime::__hopper_native::bytemuck::Zeroable for MockLayout {}
unsafe impl ::hopper_runtime::__hopper_native::bytemuck::Pod for MockLayout {}
unsafe impl Pod for MockLayout {}
impl FixedLayout for MockLayout {
    const SIZE: usize = 32;
}

// =====================================================================
// pod_from_bytes boundary tests
// =====================================================================

#[test]
fn pod_from_bytes_exact_size() {
    let data = [0xAA, 0xBB];
    let result: Result<&TinyPod, ProgramError> = pod_from_bytes(&data);
    assert!(result.is_ok());
    let val = result.unwrap();
    assert_eq!(val.a, 0xAA);
    assert_eq!(val.b, 0xBB);
}

#[test]
fn pod_from_bytes_oversized_succeeds() {
    let data = [0x11, 0x22, 0x33, 0x44, 0x55];
    let result: Result<&TinyPod, ProgramError> = pod_from_bytes(&data);
    assert!(result.is_ok());
    let val = result.unwrap();
    assert_eq!(val.a, 0x11);
    assert_eq!(val.b, 0x22);
}

#[test]
fn pod_from_bytes_undersized_rejects() {
    let data = [0xFF];
    let result: Result<&TinyPod, ProgramError> = pod_from_bytes(&data);
    assert!(result.is_err());
}

#[test]
fn pod_from_bytes_empty_rejects() {
    let data: [u8; 0] = [];
    let result: Result<&TinyPod, ProgramError> = pod_from_bytes(&data);
    assert!(result.is_err());
}

#[test]
fn pod_from_bytes_mut_undersized_rejects() {
    let mut data = [0xFFu8];
    let result: Result<&mut TinyPod, ProgramError> = pod_from_bytes_mut(&mut data);
    assert!(result.is_err());
}

#[test]
fn pod_from_bytes_mut_exact_size() {
    let mut data = [0x01, 0x02];
    let result: Result<&mut TinyPod, ProgramError> = pod_from_bytes_mut(&mut data);
    assert!(result.is_ok());
    let val = result.unwrap();
    val.a = 0xFF;
    assert_eq!(data[0], 0xFF);
}

// =====================================================================
// pod_read / pod_write boundary tests
// =====================================================================

#[test]
fn pod_read_undersized_rejects() {
    let data = [0xAA];
    let result: Result<TinyPod, ProgramError> = pod_read(&data);
    assert!(result.is_err());
}

#[test]
fn pod_read_exact_roundtrips() {
    let data = [0xDE, 0xAD];
    let val: TinyPod = pod_read(&data).unwrap();
    assert_eq!(val.a, 0xDE);
    assert_eq!(val.b, 0xAD);
}

#[test]
fn pod_write_undersized_rejects() {
    let mut data = [0u8; 1];
    let val = TinyPod { a: 1, b: 2 };
    let result = pod_write(&mut data, &val);
    assert!(result.is_err());
}

#[test]
fn pod_write_exact_roundtrips() {
    let mut data = [0u8; 2];
    let val = TinyPod { a: 0xCA, b: 0xFE };
    pod_write(&mut data, &val).unwrap();
    assert_eq!(data, [0xCA, 0xFE]);
}

// =====================================================================
// VerifiedAccount boundary tests
// =====================================================================

#[test]
fn verified_account_undersized_rejects() {
    let data = [0u8; 31]; // MockLayout needs 32
    let result = VerifiedAccount::<MockLayout>::new(&data);
    assert!(result.is_err());
}

#[test]
fn verified_account_exact_size_succeeds() {
    let data = [0u8; 32];
    let result = VerifiedAccount::<MockLayout>::new(&data);
    assert!(result.is_ok());
}

#[test]
fn verified_account_oversized_succeeds() {
    let data = [0u8; 128];
    let result = VerifiedAccount::<MockLayout>::new(&data);
    assert!(result.is_ok());
}

#[test]
fn verified_account_get_reads_correct_data() {
    let mut data = [0u8; 32];
    data[16] = 0x42; // balance[0]
    let va = VerifiedAccount::<MockLayout>::new(&data).unwrap();
    let layout = va.get();
    assert_eq!(layout.balance[0], 0x42);
}

#[test]
fn verified_account_mut_undersized_rejects() {
    let mut data = [0u8; 31];
    let result = VerifiedAccountMut::<MockLayout>::new(&mut data);
    assert!(result.is_err());
}

#[test]
fn verified_account_mut_write_reflects() {
    let mut data = [0u8; 32];
    {
        let mut va = VerifiedAccountMut::<MockLayout>::new(&mut data).unwrap();
        va.get_mut().balance = [0xFF; 8];
    }
    assert_eq!(data[16..24], [0xFF; 8]);
}

// =====================================================================
// Overlay-at boundary tests
// =====================================================================

#[test]
fn overlay_at_valid_offset_succeeds() {
    let data = [0u8; 64];
    let va = VerifiedAccount::<MockLayout>::new(&data).unwrap();
    let result = va.overlay_at::<TinyPod>(60);
    assert!(result.is_ok());
}

#[test]
fn overlay_at_out_of_bounds_rejects() {
    let data = [0u8; 64];
    let va = VerifiedAccount::<MockLayout>::new(&data).unwrap();
    // TinyPod needs 2 bytes at offset 63 → end = 65 > 64
    let result = va.overlay_at::<TinyPod>(63);
    assert!(result.is_err());
}

#[test]
fn overlay_at_exact_end_succeeds() {
    let data = [0u8; 64];
    let va = VerifiedAccount::<MockLayout>::new(&data).unwrap();
    // TinyPod needs 2 bytes at offset 62 → end = 64 == 64
    let result = va.overlay_at::<TinyPod>(62);
    assert!(result.is_ok());
}

#[test]
fn overlay_at_usize_max_rejects_no_panic() {
    let data = [0u8; 64];
    let va = VerifiedAccount::<MockLayout>::new(&data).unwrap();
    // offset = usize::MAX should trigger overflow check, not panic
    let result = va.overlay_at::<TinyPod>(usize::MAX);
    assert!(result.is_err());
}

#[test]
fn overlay_at_mut_out_of_bounds_rejects() {
    let mut data = [0u8; 64];
    let mut va = VerifiedAccountMut::<MockLayout>::new(&mut data).unwrap();
    let result = va.overlay_at_mut::<TinyPod>(63);
    assert!(result.is_err());
}

#[test]
fn overlay_at_mut_valid_writes_through() {
    let mut data = [0u8; 64];
    {
        let mut va = VerifiedAccountMut::<MockLayout>::new(&mut data).unwrap();
        let tiny = va.overlay_at_mut::<TinyPod>(60).unwrap();
        tiny.a = 0xAA;
        tiny.b = 0xBB;
    }
    assert_eq!(data[60], 0xAA);
    assert_eq!(data[61], 0xBB);
}

// =====================================================================
// AccountHeader boundary tests
// =====================================================================

#[test]
fn header_from_undersized_buffer_rejects() {
    let data = [0u8; 15]; // need 16
    let result = pod_from_bytes::<AccountHeader>(&data);
    assert!(result.is_err());
}

#[test]
fn header_roundtrip_all_fields() {
    let hdr = AccountHeader::new(1, 2, 0x0304, [0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5, 0xA6, 0xA7]);
    assert_eq!(hdr.disc, 1);
    assert_eq!(hdr.version, 2);
    assert_eq!(hdr.flags_u16(), 0x0304);
    assert_eq!(hdr.layout_id, [0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5, 0xA6, 0xA7]);
    assert_eq!(hdr.reserved, [0; 4]);
}

#[test]
fn header_disc_mismatch_detection() {
    let hdr1 = AccountHeader::new(1, 1, 0, [0; 8]);
    let hdr2 = AccountHeader::new(2, 1, 0, [0; 8]);
    assert_ne!(hdr1.disc, hdr2.disc);
}

#[test]
fn header_version_mismatch_detection() {
    let hdr1 = AccountHeader::new(1, 1, 0, [0xAA; 8]);
    let hdr2 = AccountHeader::new(1, 2, 0, [0xAA; 8]);
    assert_eq!(hdr1.disc, hdr2.disc);
    assert_ne!(hdr1.version, hdr2.version);
}

#[test]
fn header_fingerprint_mismatch_detection() {
    let hdr1 = AccountHeader::new(1, 1, 0, [0x11; 8]);
    let hdr2 = AccountHeader::new(1, 1, 0, [0x22; 8]);
    assert_eq!(hdr1.disc, hdr2.disc);
    assert_eq!(hdr1.version, hdr2.version);
    assert_ne!(hdr1.layout_id, hdr2.layout_id);
}

#[test]
fn header_wire_layout_matches_expected() {
    let hdr = AccountHeader::new(0xDD, 0x03, 0x1234, [1, 2, 3, 4, 5, 6, 7, 8]);
    let bytes: &[u8] = unsafe {
        core::slice::from_raw_parts(
            &hdr as *const AccountHeader as *const u8,
            HEADER_LEN,
        )
    };
    assert_eq!(bytes[0], 0xDD);       // disc
    assert_eq!(bytes[1], 0x03);       // version
    assert_eq!(bytes[2], 0x34);       // flags LE low
    assert_eq!(bytes[3], 0x12);       // flags LE high
    assert_eq!(&bytes[4..12], &[1, 2, 3, 4, 5, 6, 7, 8]); // layout_id
    assert_eq!(&bytes[12..16], &[0, 0, 0, 0]); // reserved
}

// =====================================================================
// Segment descriptor boundary tests
// =====================================================================

#[test]
fn segment_table_undersized_rejects() {
    let data = [0u8; 11]; // need 12 for 1 segment
    let result = SegmentTable::from_bytes(&data, 1);
    assert!(result.is_err());
}

#[test]
fn segment_table_exact_size_succeeds() {
    let data = [0u8; 12];
    let result = SegmentTable::from_bytes(&data, 1);
    assert!(result.is_ok());
    assert_eq!(result.unwrap().segment_count(), 1);
}

#[test]
fn segment_table_descriptor_oob_rejects() {
    let data = [0u8; 24]; // 2 segments
    let tbl = SegmentTable::from_bytes(&data, 2).unwrap();
    assert!(tbl.descriptor(0).is_ok());
    assert!(tbl.descriptor(1).is_ok());
    assert!(tbl.descriptor(2).is_err()); // out of bounds
}

#[test]
fn segment_descriptor_roundtrip() {
    let mut bytes = [0u8; SEGMENT_DESC_SIZE];
    // Write offset = 100, count = 5, capacity = 10, element_size = 8, flags = 1
    bytes[0..4].copy_from_slice(&100u32.to_le_bytes());
    bytes[4..6].copy_from_slice(&5u16.to_le_bytes());
    bytes[6..8].copy_from_slice(&10u16.to_le_bytes());
    bytes[8..10].copy_from_slice(&8u16.to_le_bytes());
    bytes[10..12].copy_from_slice(&1u16.to_le_bytes());

    let desc: &SegmentDescriptor = pod_from_bytes(&bytes).unwrap();
    assert_eq!(desc.offset(), 100);
    assert_eq!(desc.count(), 5);
    assert_eq!(desc.capacity(), 10);
    assert_eq!(desc.element_size(), 8);
    assert_eq!(desc.flags(), 1);
    assert_eq!(desc.data_len(), 40); // 5 * 8
    assert_eq!(desc.allocated_len(), 80); // 10 * 8
    assert!(!desc.is_full());
}

#[test]
fn segment_descriptor_is_full_when_count_equals_capacity() {
    let mut bytes = [0u8; SEGMENT_DESC_SIZE];
    bytes[0..4].copy_from_slice(&0u32.to_le_bytes());
    bytes[4..6].copy_from_slice(&10u16.to_le_bytes()); // count = 10
    bytes[6..8].copy_from_slice(&10u16.to_le_bytes()); // capacity = 10
    bytes[8..10].copy_from_slice(&4u16.to_le_bytes());
    bytes[10..12].copy_from_slice(&0u16.to_le_bytes());

    let desc: &SegmentDescriptor = pod_from_bytes(&bytes).unwrap();
    assert!(desc.is_full());
}

#[test]
fn segment_table_zero_count_valid() {
    let data = [0u8; 0]; // empty
    let result = SegmentTable::from_bytes(&data, 0);
    assert!(result.is_ok());
    assert_eq!(result.unwrap().segment_count(), 0);
}

// =====================================================================
// Wire type boundary tests
// =====================================================================

#[test]
fn wire_u64_all_bits_roundtrip() {
    for &v in &[0u64, 1, u64::MAX, 0x8000000000000000, 0xDEADBEEFCAFEBABE] {
        let w = WireU64::new(v);
        assert_eq!(w.get(), v);
    }
}

#[test]
fn wire_u32_all_bits_roundtrip() {
    for &v in &[0u32, 1, u32::MAX, 0x80000000, 0xDEADBEEF] {
        let w = WireU32::new(v);
        assert_eq!(w.get(), v);
    }
}

#[test]
fn wire_u16_all_bits_roundtrip() {
    for &v in &[0u16, 1, u16::MAX, 0x8000, 0xCAFE] {
        let w = WireU16::new(v);
        assert_eq!(w.get(), v);
    }
}

#[test]
fn wire_i64_negative_roundtrip() {
    for &v in &[0i64, -1, i64::MIN, i64::MAX, -0x7FFFFFFFFFFFFFFF] {
        let w = WireI64::new(v);
        assert_eq!(w.get(), v);
    }
}

#[test]
fn wire_bool_roundtrip() {
    let t = WireBool::new(true);
    let f = WireBool::new(false);
    assert!(t.get());
    assert!(!f.get());
}

#[test]
fn wire_u64_is_little_endian_raw() {
    let w = WireU64::new(0x0102030405060708);
    assert_eq!(*w.as_bytes(), [0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01]);
}

#[test]
fn wire_u64_pod_from_bytes_undersized() {
    let data = [0u8; 7]; // need 8
    let result = pod_from_bytes::<WireU64>(&data);
    assert!(result.is_err());
}

#[test]
fn wire_u64_pod_from_bytes_exact() {
    let data = 42u64.to_le_bytes();
    let result = pod_from_bytes::<WireU64>(&data);
    assert!(result.is_ok());
    assert_eq!(result.unwrap().get(), 42);
}

// =====================================================================
// Unchecked cast contract tests
// =====================================================================

#[test]
fn unchecked_cast_matches_checked_cast() {
    let data = [0xDE, 0xAD, 0xBE, 0xEF];
    let checked: &TinyPod = pod_from_bytes(&data).unwrap();
    let unchecked: &TinyPod = unsafe { hopper_core::account::cast_unchecked(&data) };
    assert_eq!(checked.a, unchecked.a);
    assert_eq!(checked.b, unchecked.b);
}

#[test]
fn unchecked_cast_mut_matches_checked() {
    let mut data1 = [0xCA, 0xFE, 0x00, 0x00];
    let mut data2 = [0xCA, 0xFE, 0x00, 0x00];
    let checked: &mut TinyPod = pod_from_bytes_mut(&mut data1).unwrap();
    checked.a = 0xFF;
    let unchecked: &mut TinyPod = unsafe { hopper_core::account::cast_unchecked_mut(&mut data2) };
    unchecked.a = 0xFF;
    assert_eq!(data1[0], data2[0]);
    assert_eq!(data1[1], data2[1]);
}
