//! # Overlay Equivalence Tests
//!
//! Verifies that every access path to the same data produces identical
//! byte-level results:
//! - `pod_from_bytes` vs `pod_read` (reference vs value)
//! - `VerifiedAccount::get()` vs `pod_from_bytes` on same buffer
//! - `overlay_at` vs manual `pod_from_bytes` at offset
//! - `cast_unchecked` vs `pod_from_bytes` (checked vs unchecked parity)
//! - Mutable overlay write-through equivalence

extern crate alloc;

use hopper_core::account::{
    pod_from_bytes, pod_from_bytes_mut, pod_read, pod_write,
    VerifiedAccount, VerifiedAccountMut,
    AccountHeader, HEADER_LEN,
    FixedLayout, Pod,
};
use hopper_core::abi::*;

// =====================================================================
// Test fixtures
// =====================================================================

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(C)]
struct SmallPod {
    x: u8,
    y: u8,
    z: u8,
    w: u8,
}

const _: () = assert!(core::mem::size_of::<SmallPod>() == 4);
const _: () = assert!(core::mem::align_of::<SmallPod>() == 1);

unsafe impl ::hopper_runtime::__hopper_native::bytemuck::Zeroable for SmallPod {}
unsafe impl ::hopper_runtime::__hopper_native::bytemuck::Pod for SmallPod {}
unsafe impl Pod for SmallPod {}
impl FixedLayout for SmallPod {
    const SIZE: usize = 4;
}

/// A layout with known wire fields for parity testing.
#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(C)]
struct TestLayout {
    alpha: [u8; 8],
    beta: [u8; 8],
    gamma: [u8; 4],
    delta: [u8; 4],
}

const _: () = assert!(core::mem::size_of::<TestLayout>() == 24);
const _: () = assert!(core::mem::align_of::<TestLayout>() == 1);

unsafe impl ::hopper_runtime::__hopper_native::bytemuck::Zeroable for TestLayout {}
unsafe impl ::hopper_runtime::__hopper_native::bytemuck::Pod for TestLayout {}
unsafe impl Pod for TestLayout {}
impl FixedLayout for TestLayout {
    const SIZE: usize = 24;
}

// =====================================================================
// pod_from_bytes vs pod_read equivalence
// =====================================================================

#[test]
fn ref_vs_value_read_equivalence() {
    let data = [0xDE, 0xAD, 0xBE, 0xEF];
    let by_ref: &SmallPod = pod_from_bytes(&data).unwrap();
    let by_val: SmallPod = pod_read(&data).unwrap();
    assert_eq!(by_ref.x, by_val.x);
    assert_eq!(by_ref.y, by_val.y);
    assert_eq!(by_ref.z, by_val.z);
    assert_eq!(by_ref.w, by_val.w);
}

#[test]
fn ref_vs_value_read_all_zeros() {
    let data = [0u8; 4];
    let by_ref: &SmallPod = pod_from_bytes(&data).unwrap();
    let by_val: SmallPod = pod_read(&data).unwrap();
    assert_eq!(*by_ref, by_val);
}

#[test]
fn ref_vs_value_read_all_ones() {
    let data = [0xFF; 4];
    let by_ref: &SmallPod = pod_from_bytes(&data).unwrap();
    let by_val: SmallPod = pod_read(&data).unwrap();
    assert_eq!(*by_ref, by_val);
}

// =====================================================================
// VerifiedAccount::get() vs raw pod_from_bytes parity
// =====================================================================

#[test]
fn verified_get_matches_raw_pod() {
    let mut data = [0u8; 24];
    // Fill with recognizable pattern
    for (i, b) in data.iter_mut().enumerate() {
        *b = (i as u8).wrapping_mul(0x37);
    }
    let va = VerifiedAccount::<TestLayout>::new(&data).unwrap();
    let via_verified = va.get();
    let via_raw: &TestLayout = pod_from_bytes(&data).unwrap();

    assert_eq!(via_verified.alpha, via_raw.alpha);
    assert_eq!(via_verified.beta, via_raw.beta);
    assert_eq!(via_verified.gamma, via_raw.gamma);
    assert_eq!(via_verified.delta, via_raw.delta);
}

#[test]
fn verified_mut_get_matches_raw_pod() {
    let mut data = [0u8; 24];
    for (i, b) in data.iter_mut().enumerate() {
        *b = (i as u8) ^ 0xAA;
    }
    let va = VerifiedAccountMut::<TestLayout>::new(&mut data).unwrap();
    let via_verified = va.get();
    // Compare field by field since we can't borrow data again
    let expected_alpha: [u8; 8] = core::array::from_fn(|i| (i as u8) ^ 0xAA);
    assert_eq!(via_verified.alpha, expected_alpha);
}

// =====================================================================
// overlay_at vs manual pod_from_bytes at offset parity
// =====================================================================

#[test]
fn overlay_at_matches_manual_slice_pod() {
    let mut data = [0u8; 128];
    // Write a SmallPod at offset 40
    data[40] = 0x11;
    data[41] = 0x22;
    data[42] = 0x33;
    data[43] = 0x44;

    let va = VerifiedAccount::<TestLayout>::new(&data).unwrap();
    let via_overlay: &SmallPod = va.overlay_at(40).unwrap();
    let via_manual: &SmallPod = pod_from_bytes(&data[40..]).unwrap();

    assert_eq!(via_overlay.x, via_manual.x);
    assert_eq!(via_overlay.y, via_manual.y);
    assert_eq!(via_overlay.z, via_manual.z);
    assert_eq!(via_overlay.w, via_manual.w);
}

#[test]
fn overlay_at_mut_write_matches_pod_write() {
    let mut data1 = [0u8; 128];
    let mut data2 = [0u8; 128];

    // Method 1: overlay_at_mut
    {
        let mut va = VerifiedAccountMut::<TestLayout>::new(&mut data1).unwrap();
        let pod = va.overlay_at_mut::<SmallPod>(40).unwrap();
        pod.x = 0xAA;
        pod.y = 0xBB;
        pod.z = 0xCC;
        pod.w = 0xDD;
    }

    // Method 2: pod_write
    {
        let val = SmallPod { x: 0xAA, y: 0xBB, z: 0xCC, w: 0xDD };
        pod_write(&mut data2[40..], &val).unwrap();
    }

    assert_eq!(data1[40..44], data2[40..44]);
}

// =====================================================================
// cast_unchecked vs pod_from_bytes parity
// =====================================================================

#[test]
fn unchecked_matches_checked_reference() {
    let data = [0x01, 0x02, 0x03, 0x04, 0x05, 0x06];
    let checked: &SmallPod = pod_from_bytes(&data).unwrap();
    let unchecked: &SmallPod = unsafe { hopper_core::account::cast_unchecked(&data) };
    assert_eq!(checked.x, unchecked.x);
    assert_eq!(checked.y, unchecked.y);
    assert_eq!(checked.z, unchecked.z);
    assert_eq!(checked.w, unchecked.w);
}

#[test]
fn unchecked_mut_matches_checked_mut_write() {
    let mut data1 = [0u8; 8];
    let mut data2 = [0u8; 8];

    // Via checked
    {
        let p: &mut SmallPod = pod_from_bytes_mut(&mut data1).unwrap();
        p.x = 0xAA;
        p.y = 0xBB;
    }

    // Via unchecked
    {
        let p: &mut SmallPod = unsafe { hopper_core::account::cast_unchecked_mut(&mut data2) };
        p.x = 0xAA;
        p.y = 0xBB;
    }

    assert_eq!(data1[..4], data2[..4]);
}

// =====================================================================
// Wire type overlay equivalence
// =====================================================================

#[test]
fn wire_u64_overlay_vs_raw_bytes() {
    let value = 0xDEADBEEFCAFEBABEu64;
    let expected_bytes = value.to_le_bytes();
    let wire = WireU64::new(value);
    assert_eq!(*wire.as_bytes(), expected_bytes);
    assert_eq!(wire.get(), value);
}

#[test]
fn wire_u32_overlay_vs_raw_bytes() {
    let value = 0xDEADBEEFu32;
    let expected_bytes = value.to_le_bytes();
    let wire = WireU32::new(value);
    assert_eq!(*wire.as_bytes(), expected_bytes);
    assert_eq!(wire.get(), value);
}

#[test]
fn wire_i64_overlay_vs_raw_bytes() {
    let value = -1i64;
    let expected_bytes = value.to_le_bytes();
    let wire = WireI64::new(value);
    assert_eq!(*wire.as_bytes(), expected_bytes);
    assert_eq!(wire.get(), value);
}

#[test]
fn wire_bool_overlay_nonzero_is_true() {
    // Any nonzero byte should read as true
    for nonzero in 1u8..=255 {
        let data = [nonzero];
        let w: &WireBool = pod_from_bytes(&data).unwrap();
        assert!(w.get(), "byte {nonzero} should be truthy");
    }
}

#[test]
fn wire_bool_overlay_zero_is_false() {
    let data = [0u8];
    let w: &WireBool = pod_from_bytes(&data).unwrap();
    assert!(!w.get());
}

// =====================================================================
// AccountHeader overlay equivalence
// =====================================================================

#[test]
fn header_overlay_matches_constructor() {
    let constructed = AccountHeader::new(5, 3, 0xABCD, [1, 2, 3, 4, 5, 6, 7, 8]);

    // Serialize to bytes
    let bytes: [u8; HEADER_LEN] = unsafe {
        core::mem::transmute(constructed)
    };

    // Overlay back
    let overlaid: &AccountHeader = pod_from_bytes(&bytes).unwrap();

    assert_eq!(overlaid.disc, constructed.disc);
    assert_eq!(overlaid.version, constructed.version);
    assert_eq!(overlaid.flags, constructed.flags);
    assert_eq!(overlaid.layout_id, constructed.layout_id);
    assert_eq!(overlaid.reserved, constructed.reserved);
}

#[test]
fn header_pod_write_then_read_parity() {
    let hdr = AccountHeader::new(7, 2, 0x1234, [0xAA; 8]);
    let mut buf = [0u8; 32];
    pod_write(&mut buf, &hdr).unwrap();

    let read_back: &AccountHeader = pod_from_bytes(&buf).unwrap();
    assert_eq!(read_back.disc, 7);
    assert_eq!(read_back.version, 2);
    assert_eq!(read_back.flags_u16(), 0x1234);
    assert_eq!(read_back.layout_id, [0xAA; 8]);
}

// =====================================================================
// ABI roundtrip: all wire types through pod path
// =====================================================================

#[test]
fn all_wire_types_pod_roundtrip() {
    // WireU64
    {
        let v = WireU64::new(0x0102030405060708);
        let mut buf = [0u8; 16];
        pod_write(&mut buf, &v).unwrap();
        let back: WireU64 = pod_read(&buf).unwrap();
        assert_eq!(back.get(), v.get());
    }
    // WireU32
    {
        let v = WireU32::new(0xDEADBEEF);
        let mut buf = [0u8; 8];
        pod_write(&mut buf, &v).unwrap();
        let back: WireU32 = pod_read(&buf).unwrap();
        assert_eq!(back.get(), v.get());
    }
    // WireU16
    {
        let v = WireU16::new(0xCAFE);
        let mut buf = [0u8; 4];
        pod_write(&mut buf, &v).unwrap();
        let back: WireU16 = pod_read(&buf).unwrap();
        assert_eq!(back.get(), v.get());
    }
    // WireI64
    {
        let v = WireI64::new(-999_999_999);
        let mut buf = [0u8; 16];
        pod_write(&mut buf, &v).unwrap();
        let back: WireI64 = pod_read(&buf).unwrap();
        assert_eq!(back.get(), v.get());
    }
    // WireBool
    {
        let v = WireBool::new(true);
        let mut buf = [0u8; 4];
        pod_write(&mut buf, &v).unwrap();
        let back: WireBool = pod_read(&buf).unwrap();
        assert_eq!(back.get(), v.get());
    }
}
