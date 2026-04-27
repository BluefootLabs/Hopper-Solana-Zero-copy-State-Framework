//! # Hopper Property / Fuzz Tests
//!
//! Property-based and exhaustive tests for Hopper framework primitives.
//! These tests exercise invariants that must hold for all inputs -- not just
//! specific example values. They serve as a fuzz-light harness runnable
//! via `cargo test` without external tooling.
//!
//! ## Categories
//!
//! - **ABI wire types**: encode/decode roundtrip, endianness, alignment
//! - **Collections**: capacity bounds, ordering, count invariants
//! - **Header**: write/read integrity, sentinel detection
//! - **State diff**: capture/diff accuracy, truncation handling
//! - **Layout fingerprinting**: determinism, collision resistance
//! - **Realloc guard**: budget monotonicity, overflow safety

use hopper_core::abi::*;
use hopper_core::account::*;
use hopper_core::collections::{FixedVec, RingBuffer, BitSet};
use hopper_core::diff::StateSnapshot;

// =====================================================================
// ABI Wire Type Properties
// =====================================================================

/// All u64 values survive a roundtrip through WireU64.
#[test]
fn prop_wire_u64_roundtrip_exhaustive_boundaries() {
    let test_values: &[u64] = &[
        0, 1, 2, 127, 128, 255, 256,
        u16::MAX as u64, u32::MAX as u64,
        u64::MAX / 2, u64::MAX - 1, u64::MAX,
        0x0102030405060708,
        0xDEADBEEFCAFEBABE,
        0x8000000000000000,
    ];
    for &v in test_values {
        let wire = WireU64::new(v);
        assert_eq!(wire.get(), v, "WireU64 roundtrip failed for {v:#x}");
    }
}

/// WireU64 stores bytes in little-endian order.
#[test]
fn prop_wire_u64_is_little_endian() {
    let wire = WireU64::new(0x0807060504030201);
    let bytes: [u8; 8] = unsafe { core::mem::transmute(wire) };
    assert_eq!(bytes, [0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08]);
}

/// All u32 values survive roundtrip.
#[test]
fn prop_wire_u32_roundtrip_exhaustive_boundaries() {
    let test_values: &[u32] = &[
        0, 1, 127, 128, 255, 256,
        u16::MAX as u32, u32::MAX / 2, u32::MAX - 1, u32::MAX,
        0xDEADBEEF, 0x80000000,
    ];
    for &v in test_values {
        let wire = WireU32::new(v);
        assert_eq!(wire.get(), v, "WireU32 roundtrip failed for {v:#x}");
    }
}

/// All i64 values survive roundtrip, including negative.
#[test]
fn prop_wire_i64_roundtrip_signed() {
    let test_values: &[i64] = &[
        0, 1, -1, 127, -128, i64::MIN, i64::MAX,
        i64::MIN / 2, i64::MAX / 2,
        -0x0102030405060708,
    ];
    for &v in test_values {
        let wire = WireI64::new(v);
        assert_eq!(wire.get(), v, "WireI64 roundtrip failed for {v}");
    }
}

/// WireBool: exactly 0 maps to false, exactly 1 maps to true.
#[test]
fn prop_wire_bool_canonical_values() {
    let f = WireBool::new(false);
    let t = WireBool::new(true);
    assert!(!f.get());
    assert!(t.get());

    // Raw byte 0 -> false, byte 1 -> true
    let raw_false: [u8; 1] = [0];
    let raw_true: [u8; 1] = [1];
    let f2: WireBool = unsafe { core::mem::transmute(raw_false) };
    let t2: WireBool = unsafe { core::mem::transmute(raw_true) };
    assert!(!f2.get());
    assert!(t2.get());
}

/// All wire types are alignment-1.
#[test]
fn prop_all_wire_types_align_1() {
    assert_eq!(core::mem::align_of::<WireU16>(), 1);
    assert_eq!(core::mem::align_of::<WireU32>(), 1);
    assert_eq!(core::mem::align_of::<WireU64>(), 1);
    assert_eq!(core::mem::align_of::<WireU128>(), 1);
    assert_eq!(core::mem::align_of::<WireI16>(), 1);
    assert_eq!(core::mem::align_of::<WireI32>(), 1);
    assert_eq!(core::mem::align_of::<WireI64>(), 1);
    assert_eq!(core::mem::align_of::<WireI128>(), 1);
    assert_eq!(core::mem::align_of::<WireBool>(), 1);
    assert_eq!(core::mem::align_of::<AccountHeader>(), 1);
}

/// Wire type sizes match their declared widths.
#[test]
fn prop_wire_type_sizes_correct() {
    assert_eq!(core::mem::size_of::<WireU16>(), 2);
    assert_eq!(core::mem::size_of::<WireU32>(), 4);
    assert_eq!(core::mem::size_of::<WireU64>(), 8);
    assert_eq!(core::mem::size_of::<WireU128>(), 16);
    assert_eq!(core::mem::size_of::<WireI16>(), 2);
    assert_eq!(core::mem::size_of::<WireI32>(), 4);
    assert_eq!(core::mem::size_of::<WireI64>(), 8);
    assert_eq!(core::mem::size_of::<WireI128>(), 16);
    assert_eq!(core::mem::size_of::<WireBool>(), 1);
    assert_eq!(core::mem::size_of::<AccountHeader>(), HEADER_LEN);
}

// =====================================================================
// TypedAddress Properties
// =====================================================================

/// TypedAddress<T> is exactly 32 bytes, alignment 1.
#[test]
fn prop_typed_address_size_and_align() {
    assert_eq!(core::mem::size_of::<TypedAddress<Authority>>(), 32);
    assert_eq!(core::mem::align_of::<TypedAddress<Authority>>(), 1);
    assert_eq!(core::mem::size_of::<TypedAddress<Mint>>(), 32);
    assert_eq!(core::mem::size_of::<TypedAddress<Program>>(), 32);
}

/// TypedAddress zeroed() is all-zeros and is_zero() returns true.
#[test]
fn prop_typed_address_zero_invariant() {
    let addr: TypedAddress<Authority> = TypedAddress::zeroed();
    assert!(addr.is_zero());
    assert_eq!(addr.as_bytes(), &[0u8; 32]);
}

/// TypedAddress from_slice preserves bytes.
#[test]
fn prop_typed_address_from_slice_roundtrip() {
    let bytes: [u8; 32] = [
        1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16,
        17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32,
    ];
    let addr: TypedAddress<Authority> = TypedAddress::from_slice(&bytes);
    assert_eq!(addr.as_bytes(), &bytes);
    assert!(!addr.is_zero());
}

/// TypedAddress eq_bytes equivalence.
#[test]
fn prop_typed_address_eq_bytes() {
    let bytes = [42u8; 32];
    let addr: TypedAddress<Mint> = TypedAddress::from_slice(&bytes);
    assert!(addr.eq_bytes(&bytes));
    let other = [43u8; 32];
    assert!(!addr.eq_bytes(&other));
}

/// UntypedAddress can be created from TypedAddress.
#[test]
fn prop_untyped_address_cast() {
    let bytes = [7u8; 32];
    let typed: TypedAddress<Authority> = TypedAddress::from_slice(&bytes);
    let untyped: UntypedAddress = typed.untyped();
    assert_eq!(untyped.as_bytes(), &bytes);
}

// =====================================================================
// Account Header Properties
// =====================================================================

/// Write and read header roundtrip.
#[test]
fn prop_header_write_read_roundtrip() {
    let discs = [0u8, 1, 127, 254];
    let versions = [1u8, 2, 127, 255];
    let layout_ids: [[u8; 8]; 3] = [
        [0, 0, 0, 0, 0, 0, 0, 0],
        [1, 2, 3, 4, 5, 6, 7, 8],
        [0xFF, 0xFE, 0xFD, 0xFC, 0xFB, 0xFA, 0xF9, 0xF8],
    ];

    for &disc in &discs {
        for &ver in &versions {
            for layout_id in &layout_ids {
                let mut buf = [0u8; HEADER_LEN + 16]; // extra bytes
                zero_init(&mut buf);
                write_header(&mut buf, disc, ver, layout_id).unwrap();

                // Read back via overlay
                let header = unsafe { &*(buf.as_ptr() as *const AccountHeader) };
                assert_eq!(header.disc, disc, "disc mismatch");
                assert_eq!(header.version, ver, "version mismatch");
                assert_eq!(&header.layout_id, layout_id, "layout_id mismatch");
                assert_eq!(header.flags, [0, 0], "flags should be zero");
                assert_eq!(header.reserved, [0, 0, 0, 0], "reserved should be zero");
            }
        }
    }
}

/// Zero-init clears all bytes.
#[test]
fn prop_zero_init_clears_all() {
    let sizes = [16, 57, 128, 256, 1024];
    for size in sizes {
        let mut buf = vec![0xFFu8; size];
        zero_init(&mut buf);
        for (i, byte) in buf.iter().enumerate() {
            assert_eq!(*byte, 0, "byte {i} not zero after zero_init (size {size})");
        }
    }
}

/// Close sentinel is 0xFF at disc position.
#[test]
fn prop_close_sentinel_value() {
    assert_eq!(CLOSE_SENTINEL, 0xFF);
}

/// Header write fails on too-short buffer.
#[test]
fn prop_header_write_rejects_short_buffer() {
    let mut buf = [0u8; 8]; // too short (< 16)
    let result = write_header(&mut buf, 1, 1, &[0; 8]);
    assert!(result.is_err());
}

// =====================================================================
// FixedVec Properties
// =====================================================================

/// FixedVec: push/pop preserves LIFO ordering for all capacities.
#[test]
fn prop_fixed_vec_lifo_ordering() {
    for capacity in 1..=8 {
        let size = 4 + 8 * capacity;
        let mut buf = vec![0u8; size];
        let mut vec = FixedVec::<WireU64>::from_bytes(&mut buf).unwrap();

        // Push 1..=capacity
        for i in 1..=capacity {
            vec.push(WireU64::new(i as u64)).unwrap();
        }
        assert_eq!(vec.len(), capacity);

        // Pop should return in reverse order
        for i in (1..=capacity).rev() {
            let val = vec.pop().unwrap();
            assert_eq!(val.get(), i as u64);
        }
        assert!(vec.is_empty());
    }
}

/// FixedVec: push at capacity fails.
#[test]
fn prop_fixed_vec_capacity_enforced() {
    for capacity in 1..=4 {
        let size = 4 + 8 * capacity;
        let mut buf = vec![0u8; size];
        let mut vec = FixedVec::<WireU64>::from_bytes(&mut buf).unwrap();

        for i in 0..capacity {
            vec.push(WireU64::new(i as u64)).unwrap();
        }
        assert!(vec.is_full());
        assert!(vec.push(WireU64::new(99)).is_err());
    }
}

/// FixedVec: swap_remove maintains count invariant.
#[test]
fn prop_fixed_vec_swap_remove_count() {
    let mut buf = vec![0u8; 4 + 8 * 4];
    let mut vec = FixedVec::<WireU64>::from_bytes(&mut buf).unwrap();

    vec.push(WireU64::new(10)).unwrap();
    vec.push(WireU64::new(20)).unwrap();
    vec.push(WireU64::new(30)).unwrap();
    vec.push(WireU64::new(40)).unwrap();

    vec.swap_remove(1).unwrap();
    assert_eq!(vec.len(), 3);

    vec.swap_remove(0).unwrap();
    assert_eq!(vec.len(), 2);

    // Elements remaining should be accessible
    let _ = vec.get(0).unwrap();
    let _ = vec.get(1).unwrap();
    assert!(vec.get(2).is_err());
}

/// FixedVec: clear resets to empty.
#[test]
fn prop_fixed_vec_clear() {
    let mut buf = vec![0u8; 4 + 8 * 4];
    let mut vec = FixedVec::<WireU64>::from_bytes(&mut buf).unwrap();
    vec.push(WireU64::new(1)).unwrap();
    vec.push(WireU64::new(2)).unwrap();
    vec.clear();
    assert!(vec.is_empty());
    assert_eq!(vec.len(), 0);
}

// =====================================================================
// RingBuffer Properties
// =====================================================================

/// RingBuffer: wrapping preserves most recent N elements.
#[test]
fn prop_ring_buffer_preserves_last_n() {
    for capacity in 1..=5 {
        let el_size = 4; // WireU32
        let size = 8 + el_size * capacity;
        let mut buf = vec![0u8; size];
        let mut ring = RingBuffer::<WireU32>::from_bytes(&mut buf).unwrap();

        // Push 2x capacity elements
        let total = capacity * 2;
        for i in 0..total {
            ring.push(WireU32::new(i as u32)).unwrap();
        }

        // Ring should contain capacity elements
        let count = ring.count();
        assert!(count <= capacity, "ring count {count} > capacity {capacity}");

        // Latest should be the last pushed
        let latest = ring.latest().unwrap().get();
        assert_eq!(latest, (total - 1) as u32);
    }
}

// =====================================================================
// BitSet Properties
// =====================================================================

/// BitSet: set then get returns true for all bit positions.
#[test]
fn prop_bitset_set_get_all_positions() {
    let mut buf = [0u8; 8]; // 64 bits
    let mut bs = BitSet::from_bytes(&mut buf);

    for i in 0..64 {
        // Clear first
        let _ = bs.clear(i);
        assert!(!bs.get(i).unwrap());

        // Set
        bs.set(i).unwrap();
        assert!(bs.get(i).unwrap());
    }
}

/// BitSet: toggle works for all positions.
#[test]
fn prop_bitset_toggle_idempotency() {
    let mut buf = [0u8; 4]; // 32 bits
    let mut bs = BitSet::from_bytes(&mut buf);

    for i in 0..32 {
        // Start false
        assert!(!bs.get(i).unwrap());
        // Toggle to true
        bs.toggle(i).unwrap();
        assert!(bs.get(i).unwrap());
        // Toggle back to false
        bs.toggle(i).unwrap();
        assert!(!bs.get(i).unwrap());
    }
}

/// BitSet: count_ones matches manual count.
#[test]
fn prop_bitset_count_ones_accuracy() {
    let mut buf = [0u8; 4];
    let mut bs = BitSet::from_bytes(&mut buf);

    assert_eq!(bs.count_ones(), 0);

    // Set bits 0, 5, 10, 15, 20, 25, 30
    let positions = [0, 5, 10, 15, 20, 25, 30];
    for &pos in &positions {
        bs.set(pos).unwrap();
    }
    assert_eq!(bs.count_ones(), positions.len());
}

/// BitSet: out-of-bounds access returns error.
#[test]
fn prop_bitset_bounds_check() {
    let mut buf = [0u8; 2]; // 16 bits
    let mut bs = BitSet::from_bytes(&mut buf);

    assert!(bs.get(15).is_ok());
    assert!(bs.get(16).is_err());
    assert!(bs.set(16).is_err());
    assert!(bs.clear(16).is_err());
    assert!(bs.toggle(16).is_err());
}

// =====================================================================
// State Diff Properties
// =====================================================================

/// Identical data produces no changes.
#[test]
fn prop_state_diff_no_changes_on_identical() {
    let data = [0xAB_u8; 64];
    let snap = StateSnapshot::<64>::capture(&data);
    let diff = snap.diff(&data);
    assert!(!diff.has_changes());
    assert_eq!(diff.changed_byte_count(), 0);
}

/// Single byte change detected accurately.
#[test]
fn prop_state_diff_detects_single_byte() {
    let original = [0u8; 64];
    let snap = StateSnapshot::<64>::capture(&original);

    let mut modified = original;
    modified[31] = 0xFF;

    let diff = snap.diff(&modified);
    assert!(diff.has_changes());
    assert_eq!(diff.changed_byte_count(), 1);
}

/// All bytes changed.
#[test]
fn prop_state_diff_all_changed() {
    let original = [0u8; 32];
    let snap = StateSnapshot::<32>::capture(&original);

    let modified = [0xFF_u8; 32];
    let diff = snap.diff(&modified);
    assert!(diff.has_changes());
    assert_eq!(diff.changed_byte_count(), 32);
}

/// Truncated snapshot acknowledged.
#[test]
fn prop_state_snapshot_truncation() {
    let big_data = [0xAA_u8; 256];
    let snap = StateSnapshot::<64>::capture(&big_data);
    assert!(snap.was_truncated());
    assert_eq!(snap.len(), 64);
}

/// Non-truncated snapshot has exact length.
#[test]
fn prop_state_snapshot_exact_fit() {
    let data = [0xBB_u8; 32];
    let snap = StateSnapshot::<64>::capture(&data);
    assert!(!snap.was_truncated());
    assert_eq!(snap.len(), 32);
}

// =====================================================================
// Realloc Guard Properties
// =====================================================================

/// ReallocGuard budget is enforced monotonically via check_growth.
#[test]
fn prop_realloc_guard_budget_monotonic() {
    let mut guard = ReallocGuard::<8>::new(1024);

    // Register a slot with original size 100
    guard.register(0, 100).unwrap();

    // Growth within budget: 100 → 600 = delta 500, consumed 500
    assert!(guard.check_growth(0, 600).is_ok());
    guard.commit_growth(0, 600).unwrap();

    // Further growth within budget: 600 → 1100 = delta 500, consumed 1000
    assert!(guard.check_growth(0, 1100).is_ok());
    guard.commit_growth(0, 1100).unwrap();

    // Growth that exceeds budget: 1100 → 1200 = delta 100, consumed would be 1100 > 1024
    assert!(guard.check_growth(0, 1200).is_err());
}

/// ReallocGuard rejects unregistered slots.
#[test]
fn prop_realloc_guard_unregistered_slot() {
    let mut guard = ReallocGuard::<8>::new(1024);
    assert!(guard.commit_growth(5, 10).is_err());
}

/// ReallocGuard register bounds checking.
#[test]
fn prop_realloc_guard_register_bounds() {
    let mut guard = ReallocGuard::<4>::new(1024);
    // Slot 0 valid
    assert!(guard.register(0, 100).is_ok());
    // Out-of-bounds slot should fail
    assert!(guard.register(64, 100).is_err());
}

// =====================================================================
// Layout Fingerprint Properties
// =====================================================================

/// Layout ID is exactly 8 bytes.
#[test]
fn prop_layout_id_length() {
    // Compute a fingerprint manually using the same algorithm
    let input = b"hopper:v1:Test:1:field_a:WireU64:8,";
    let hash = sha2_const_stable::Sha256::new().update(input).finalize();
    let id: [u8; 8] = [
        hash[0], hash[1], hash[2], hash[3],
        hash[4], hash[5], hash[6], hash[7],
    ];
    assert_eq!(id.len(), 8);
}

/// Different field sets produce different layout IDs.
#[test]
fn prop_layout_id_collision_resistance() {
    let input_a = b"hopper:v1:Vault:1:balance:WireU64:8,";
    let input_b = b"hopper:v1:Vault:1:amount:WireU64:8,";

    let hash_a = sha2_const_stable::Sha256::new().update(input_a).finalize();
    let hash_b = sha2_const_stable::Sha256::new().update(input_b).finalize();

    let id_a: [u8; 8] = [hash_a[0], hash_a[1], hash_a[2], hash_a[3],
                          hash_a[4], hash_a[5], hash_a[6], hash_a[7]];
    let id_b: [u8; 8] = [hash_b[0], hash_b[1], hash_b[2], hash_b[3],
                          hash_b[4], hash_b[5], hash_b[6], hash_b[7]];

    assert_ne!(id_a, id_b, "Different field names must produce different layout IDs");
}

/// Same fields in different order produce different layout IDs.
#[test]
fn prop_layout_id_order_sensitive() {
    let input_a = b"hopper:v1:Test:1:alpha:WireU64:8,beta:WireU32:4,";
    let input_b = b"hopper:v1:Test:1:beta:WireU32:4,alpha:WireU64:8,";

    let hash_a = sha2_const_stable::Sha256::new().update(input_a).finalize();
    let hash_b = sha2_const_stable::Sha256::new().update(input_b).finalize();

    let id_a: [u8; 8] = [hash_a[0], hash_a[1], hash_a[2], hash_a[3],
                          hash_a[4], hash_a[5], hash_a[6], hash_a[7]];
    let id_b: [u8; 8] = [hash_b[0], hash_b[1], hash_b[2], hash_b[3],
                          hash_b[4], hash_b[5], hash_b[6], hash_b[7]];

    assert_ne!(id_a, id_b, "Field order must matter in layout ID generation");
}

/// Same definition is deterministic (idempotent).
#[test]
fn prop_layout_id_deterministic() {
    let input = b"hopper:v1:Vault:1:authority:TypedAddress < Authority >:32,balance:WireU64:8,bump:u8:1,";
    let hash1 = sha2_const_stable::Sha256::new().update(input).finalize();
    let hash2 = sha2_const_stable::Sha256::new().update(input).finalize();

    assert_eq!(hash1, hash2, "Same input must always produce same hash");
}

/// Version bump changes the layout ID.
#[test]
fn prop_layout_id_version_changes_id() {
    let input_v1 = b"hopper:v1:Vault:1:balance:WireU64:8,";
    let input_v2 = b"hopper:v1:Vault:2:balance:WireU64:8,";

    let hash_v1 = sha2_const_stable::Sha256::new().update(input_v1).finalize();
    let hash_v2 = sha2_const_stable::Sha256::new().update(input_v2).finalize();

    let id_v1: [u8; 8] = [hash_v1[0], hash_v1[1], hash_v1[2], hash_v1[3],
                           hash_v1[4], hash_v1[5], hash_v1[6], hash_v1[7]];
    let id_v2: [u8; 8] = [hash_v2[0], hash_v2[1], hash_v2[2], hash_v2[3],
                           hash_v2[4], hash_v2[5], hash_v2[6], hash_v2[7]];

    assert_ne!(id_v1, id_v2, "Version bump must change layout ID");
}

// =====================================================================
// Schema Migration Properties
// =====================================================================

/// Same manifest → no-op migration.
#[test]
fn prop_schema_same_manifest_noop() {
    use hopper_schema::*;
    let fields = &[
        FieldDescriptor { name: "balance", canonical_type: "WireU64", size: 8, offset: 16, intent: FieldIntent::Custom },
    ];
    let m = LayoutManifest {
        name: "Vault", version: 1, disc: 1,
        layout_id: [1, 2, 3, 4, 5, 6, 7, 8],
        total_size: 24, field_count: 1, fields,
    };
    assert!(!requires_migration(&m, &m));
    // Same manifest is NOT append-compatible (version not greater, layout_id not different)
    assert!(!is_append_compatible(&m, &m));
}

/// Added field → append-compatible.
#[test]
fn prop_schema_append_compatible() {
    use hopper_schema::*;
    let old_fields = &[
        FieldDescriptor { name: "balance", canonical_type: "WireU64", size: 8, offset: 16, intent: FieldIntent::Custom },
    ];
    let new_fields = &[
        FieldDescriptor { name: "balance", canonical_type: "WireU64", size: 8, offset: 16, intent: FieldIntent::Custom },
        FieldDescriptor { name: "extra", canonical_type: "WireU32", size: 4, offset: 24, intent: FieldIntent::Custom },
    ];
    let old = LayoutManifest {
        name: "Vault", version: 1, disc: 1,
        layout_id: [1, 2, 3, 4, 5, 6, 7, 8],
        total_size: 24, field_count: 1, fields: old_fields,
    };
    let new = LayoutManifest {
        name: "Vault", version: 2, disc: 1,
        layout_id: [8, 7, 6, 5, 4, 3, 2, 1],
        total_size: 28, field_count: 2, fields: new_fields,
    };
    assert!(is_append_compatible(&old, &new));
}

/// Changed field type → requires migration.
#[test]
fn prop_schema_changed_type_requires_migration() {
    use hopper_schema::*;
    let old_fields = &[
        FieldDescriptor { name: "balance", canonical_type: "WireU64", size: 8, offset: 16, intent: FieldIntent::Custom },
    ];
    let new_fields = &[
        FieldDescriptor { name: "balance", canonical_type: "WireU128", size: 16, offset: 16, intent: FieldIntent::Custom },
    ];
    let old = LayoutManifest {
        name: "Vault", version: 1, disc: 1,
        layout_id: [1, 2, 3, 4, 5, 6, 7, 8],
        total_size: 24, field_count: 1, fields: old_fields,
    };
    let new = LayoutManifest {
        name: "Vault", version: 2, disc: 1,
        layout_id: [9, 9, 9, 9, 9, 9, 9, 9],
        total_size: 32, field_count: 1, fields: new_fields,
    };
    assert!(requires_migration(&old, &new));
    // Coarse structural check passes (version bumped, size grew, layout_id differs)
    // but field-level comparison reveals the type change is NOT append-safe
    let report = compare_fields::<4>(&old, &new);
    assert!(!report.is_append_safe);
}

// =====================================================================
// Math Properties
// =====================================================================

/// Checked add detects overflow.
#[test]
fn prop_checked_add_overflow() {
    use hopper_core::math::checked_add;
    assert!(checked_add(u64::MAX, 1).is_err());
    assert!(checked_add(u64::MAX, u64::MAX).is_err());
    assert!(checked_add(u64::MAX - 1, 1).is_ok());
}

/// Checked mul detects overflow.
#[test]
fn prop_checked_mul_overflow() {
    use hopper_core::math::checked_mul;
    assert!(checked_mul(u64::MAX, 2).is_err());
    assert!(checked_mul(u64::MAX / 2 + 1, 2).is_err());
    assert!(checked_mul(u64::MAX / 2, 2).is_ok());
}

/// Checked sub detects underflow.
#[test]
fn prop_checked_sub_underflow() {
    use hopper_core::math::checked_sub;
    assert!(checked_sub(0, 1).is_err());
    assert!(checked_sub(5, 6).is_err());
    assert!(checked_sub(5, 5).is_ok());
}

// =====================================================================
// Segment Role Properties
// =====================================================================

use hopper_core::account::segment_role::{SegmentRole, SEG_ROLE_CORE, SEG_ROLE_EXTENSION, SEG_ROLE_JOURNAL, SEG_ROLE_INDEX, SEG_ROLE_CACHE, SEG_ROLE_AUDIT, SEG_ROLE_SHARD};

/// SegmentRole roundtrips through flags encoding.
#[test]
fn prop_segment_role_flags_roundtrip() {
    let roles = [
        SegmentRole::Core,
        SegmentRole::Extension,
        SegmentRole::Journal,
        SegmentRole::Index,
        SegmentRole::Cache,
        SegmentRole::Audit,
        SegmentRole::Shard,
        SegmentRole::Unclassified,
    ];
    for role in roles {
        let flags = role.into_flags(0);
        let decoded = SegmentRole::from_flags(flags);
        assert_eq!(decoded, role, "roundtrip failed for role {:?}", role.name());
    }
}

/// into_flags preserves lower 12 bits.
#[test]
fn prop_segment_role_preserves_lower_bits() {
    let lower_bits: &[u16] = &[0x000, 0x001, 0x007, 0x0FF, 0xFFF];
    let roles = [
        SegmentRole::Core, SegmentRole::Journal, SegmentRole::Cache, SegmentRole::Shard,
    ];
    for &bits in lower_bits {
        for role in roles {
            let flags = role.into_flags(bits);
            // Lower 12 bits preserved
            assert_eq!(flags & 0x0FFF, bits, "lower bits clobbered");
            // Role encoded in upper 4
            assert_eq!(SegmentRole::from_flags(flags), role, "role not encoded");
        }
    }
}

/// into_flags overwrites upper 4 bits even if already set.
#[test]
fn prop_segment_role_overwrites_existing_role() {
    let original_flags = SegmentRole::Audit.into_flags(0x007);
    let new_flags = SegmentRole::Cache.into_flags(original_flags);
    assert_eq!(SegmentRole::from_flags(new_flags), SegmentRole::Cache);
    assert_eq!(new_flags & 0x0FFF, 0x007);
}

/// Convenience constants match encoded values.
#[test]
fn prop_segment_role_constants() {
    assert_eq!(SEG_ROLE_CORE, SegmentRole::Core.into_flags(0));
    assert_eq!(SEG_ROLE_EXTENSION, SegmentRole::Extension.into_flags(0));
    assert_eq!(SEG_ROLE_JOURNAL, SegmentRole::Journal.into_flags(0));
    assert_eq!(SEG_ROLE_INDEX, SegmentRole::Index.into_flags(0));
    assert_eq!(SEG_ROLE_CACHE, SegmentRole::Cache.into_flags(0));
    assert_eq!(SEG_ROLE_AUDIT, SegmentRole::Audit.into_flags(0));
    assert_eq!(SEG_ROLE_SHARD, SegmentRole::Shard.into_flags(0));
}

/// must_preserve: Core and Audit must be preserved, others not.
#[test]
fn prop_segment_role_must_preserve() {
    assert!(SegmentRole::Core.must_preserve());
    assert!(SegmentRole::Audit.must_preserve());
    assert!(!SegmentRole::Extension.must_preserve());
    assert!(!SegmentRole::Journal.must_preserve());
    assert!(!SegmentRole::Index.must_preserve());
    assert!(!SegmentRole::Cache.must_preserve());
    assert!(!SegmentRole::Shard.must_preserve());
    assert!(!SegmentRole::Unclassified.must_preserve());
}

/// clearable_on_migration: Journal and Cache are clearable.
#[test]
fn prop_segment_role_clearable() {
    assert!(SegmentRole::Journal.clearable_on_migration());
    assert!(SegmentRole::Cache.clearable_on_migration());
    assert!(!SegmentRole::Core.clearable_on_migration());
    assert!(!SegmentRole::Audit.clearable_on_migration());
}

/// rebuildable: Index and Cache are rebuildable.
#[test]
fn prop_segment_role_rebuildable() {
    assert!(SegmentRole::Index.rebuildable());
    assert!(SegmentRole::Cache.rebuildable());
    assert!(!SegmentRole::Core.rebuildable());
    assert!(!SegmentRole::Journal.rebuildable());
}

/// append_only: Journal and Audit are append-only.
#[test]
fn prop_segment_role_append_only() {
    assert!(SegmentRole::Journal.is_append_only());
    assert!(SegmentRole::Audit.is_append_only());
    assert!(!SegmentRole::Core.is_append_only());
    assert!(!SegmentRole::Cache.is_append_only());
}

/// immutable_after_init: only Audit.
#[test]
fn prop_segment_role_immutable_after_init() {
    assert!(SegmentRole::Audit.is_immutable_after_init());
    assert!(!SegmentRole::Core.is_immutable_after_init());
    assert!(!SegmentRole::Journal.is_immutable_after_init());
}

/// requires_migration_copy is true only for Core and Audit.
#[test]
fn prop_segment_role_requires_migration_copy() {
    assert!(SegmentRole::Core.requires_migration_copy());
    assert!(SegmentRole::Audit.requires_migration_copy());
    assert!(!SegmentRole::Extension.requires_migration_copy());
    assert!(!SegmentRole::Journal.requires_migration_copy());
    assert!(!SegmentRole::Index.requires_migration_copy());
    assert!(!SegmentRole::Cache.requires_migration_copy());
    assert!(!SegmentRole::Shard.requires_migration_copy());
    assert!(!SegmentRole::Unclassified.requires_migration_copy());
}

/// is_safe_to_drop is true only for Cache.
#[test]
fn prop_segment_role_is_safe_to_drop() {
    assert!(SegmentRole::Cache.is_safe_to_drop());
    assert!(!SegmentRole::Core.is_safe_to_drop());
    assert!(!SegmentRole::Extension.is_safe_to_drop());
    assert!(!SegmentRole::Journal.is_safe_to_drop());
    assert!(!SegmentRole::Index.is_safe_to_drop());
    assert!(!SegmentRole::Audit.is_safe_to_drop());
    assert!(!SegmentRole::Shard.is_safe_to_drop());
    assert!(!SegmentRole::Unclassified.is_safe_to_drop());
}

/// from_flags with unknown upper bits maps to Unclassified.
#[test]
fn prop_segment_role_unknown_maps_to_unclassified() {
    for code in 8..=15u16 {
        let flags = code << 12;
        assert_eq!(
            SegmentRole::from_flags(flags),
            SegmentRole::Unclassified,
            "code {code} should map to Unclassified"
        );
    }
}

/// Role names are non-empty and lowercase.
#[test]
fn prop_segment_role_names_valid() {
    let roles = [
        SegmentRole::Core, SegmentRole::Extension, SegmentRole::Journal,
        SegmentRole::Index, SegmentRole::Cache, SegmentRole::Audit,
        SegmentRole::Shard, SegmentRole::Unclassified,
    ];
    for role in roles {
        let name = role.name();
        assert!(!name.is_empty());
        assert!(name.chars().all(|c| c.is_ascii_lowercase()), "name {name} not lowercase");
    }
}

// =====================================================================
// State Receipt Properties
// =====================================================================

use hopper_core::receipt::{StateReceipt, RECEIPT_SIZE};

/// Receipt begin captures initial state.
#[test]
fn prop_receipt_begin_initial_state() {
    let layout_id = [1, 2, 3, 4, 5, 6, 7, 8];
    let data = [0xAA_u8; 64];
    let receipt = StateReceipt::<64>::begin(&layout_id, &data);

    assert_eq!(receipt.layout_id, layout_id);
    assert!(!receipt.is_committed());
    assert!(!receipt.has_changes());
    assert_eq!(receipt.old_size, 64);
    assert_eq!(receipt.changed_bytes, 0);
    assert_eq!(receipt.changed_regions, 0);
}

/// Receipt commit detects no changes on identical data.
#[test]
fn prop_receipt_commit_no_changes() {
    let layout_id = [0; 8];
    let data = [0x55_u8; 32];
    let mut receipt = StateReceipt::<32>::begin(&layout_id, &data);
    receipt.commit(&data);

    assert!(receipt.is_committed());
    assert!(!receipt.has_changes());
    assert_eq!(receipt.changed_bytes, 0);
    assert_eq!(receipt.new_size, 32);
    assert!(!receipt.was_resized);
}

/// Receipt commit detects byte changes.
#[test]
fn prop_receipt_commit_detects_changes() {
    let layout_id = [0; 8];
    let data = [0u8; 32];
    let mut receipt = StateReceipt::<32>::begin(&layout_id, &data);

    let mut modified = data;
    modified[8] = 0xFF;
    modified[9] = 0xFF;
    modified[20] = 0x01;

    receipt.commit(&modified);
    assert!(receipt.is_committed());
    assert!(receipt.has_changes());
    assert_eq!(receipt.changed_bytes, 3);
    assert_eq!(receipt.new_size, 32);
}

/// Receipt detects resize.
#[test]
fn prop_receipt_detects_resize() {
    let layout_id = [0; 8];
    let data = [0u8; 32];
    let mut receipt = StateReceipt::<64>::begin(&layout_id, &data);

    let bigger = [0u8; 64];
    receipt.commit(&bigger);
    assert!(receipt.is_committed());
    assert!(receipt.was_resized);
    assert!(receipt.has_changes());
    assert_eq!(receipt.old_size, 32);
    assert_eq!(receipt.new_size, 64);
}

/// Receipt commit_with_fields tracks field-level changes.
#[test]
fn prop_receipt_field_tracking() {
    let layout_id = [0; 8];
    // Simulate a layout: [header:16][balance:8][count:4]
    let mut data = [0u8; 28];
    let mut receipt = StateReceipt::<28>::begin(&layout_id, &data);

    // Mutate the balance field (offset 16, size 8)
    data[16] = 0xFF;
    let fields: &[(&str, usize, usize)] = &[
        ("balance", 16, 8),
        ("count", 24, 4),
    ];
    receipt.commit_with_fields(&data, fields);

    assert!(receipt.is_committed());
    assert_eq!(receipt.changed_fields & 0x01, 1, "balance field not flagged");
    assert_eq!(receipt.changed_fields & 0x02, 0, "count field falsely flagged");
}

/// Receipt set_invariants records correctly.
#[test]
fn prop_receipt_invariants_tracking() {
    let layout_id = [0; 8];
    let data = [0u8; 16];
    let mut receipt = StateReceipt::<16>::begin(&layout_id, &data);
    receipt.commit(&data);

    receipt.set_invariants(true, 5);
    assert!(receipt.invariants_passed);
    assert_eq!(receipt.invariants_checked, 5);

    receipt.set_invariants(false, 3);
    assert!(!receipt.invariants_passed);
    assert_eq!(receipt.invariants_checked, 3);
}

/// Receipt set_cpi_invoked tracks CPI flag.
#[test]
fn prop_receipt_cpi_tracking() {
    let layout_id = [0; 8];
    let data = [0u8; 16];
    let mut receipt = StateReceipt::<16>::begin(&layout_id, &data);
    assert!(!receipt.cpi_invoked);
    receipt.set_cpi_invoked(true);
    assert!(receipt.cpi_invoked);
}

/// Receipt to_bytes wire format: layout_id at offset 0, flags at offset 32.
#[test]
fn prop_receipt_wire_format() {
    let layout_id = [0xA1, 0xA2, 0xA3, 0xA4, 0xA5, 0xA6, 0xA7, 0xA8];
    let data = [0u8; 32];
    let mut receipt = StateReceipt::<32>::begin(&layout_id, &data);

    let mut modified = data;
    modified[0] = 0xFF;
    receipt.commit(&modified);
    receipt.set_invariants(true, 2);
    receipt.set_cpi_invoked(true);

    let wire = receipt.to_bytes();
    assert_eq!(wire.len(), RECEIPT_SIZE);

    // layout_id at offset 0
    assert_eq!(&wire[0..8], &layout_id);
    // changed_fields at offset 8 (u64 LE)
    let cf = u64::from_le_bytes(wire[8..16].try_into().unwrap());
    assert_eq!(cf, 0); // commit (not commit_with_fields) doesn't set field mask
    // changed_bytes at offset 16 (u32 LE)
    let cb = u32::from_le_bytes(wire[16..20].try_into().unwrap());
    assert_eq!(cb, 1);
    // old_size at offset 22 (u32 LE)
    let os = u32::from_le_bytes(wire[22..26].try_into().unwrap());
    assert_eq!(os, 32);
    // new_size at offset 26 (u32 LE)
    let ns = u32::from_le_bytes(wire[26..30].try_into().unwrap());
    assert_eq!(ns, 32);
    // invariants_checked at offset 30 (u16 LE)
    let ic = u16::from_le_bytes(wire[30..32].try_into().unwrap());
    assert_eq!(ic, 2);
    // flags at offset 32
    let flags = wire[32];
    // was_resized: bit 0 = 0, invariants_passed: bit 1 = 1, cpi_invoked: bit 2 = 1, committed: bit 3 = 1
    assert_eq!(flags & (1 << 0), 0, "was_resized should be 0");
    assert_ne!(flags & (1 << 1), 0, "invariants_passed should be set");
    assert_ne!(flags & (1 << 2), 0, "cpi_invoked should be set");
    assert_ne!(flags & (1 << 3), 0, "committed should be set");
    // before_fingerprint at offset 33..41 (should be non-zero, data was all zeros)
    assert_ne!(&wire[33..41], &[0u8; 8], "before fingerprint should be populated");
    // after_fingerprint at offset 41..49 (should differ from before, data changed)
    assert_ne!(&wire[33..41], &wire[41..49], "fingerprints should differ when data changed");
}

/// Receipt to_bytes with field tracking includes changed_fields.
#[test]
fn prop_receipt_wire_format_with_fields() {
    let layout_id = [0; 8];
    let mut data = [0u8; 28];
    let mut receipt = StateReceipt::<28>::begin(&layout_id, &data);

    data[24] = 0x42; // mutate the second field
    let fields: &[(&str, usize, usize)] = &[
        ("balance", 16, 8),
        ("count", 24, 4),
    ];
    receipt.commit_with_fields(&data, fields);

    let wire = receipt.to_bytes();
    let cf = u64::from_le_bytes(wire[8..16].try_into().unwrap());
    assert_eq!(cf & 0x02, 0x02, "count field (bit 1) should be set");
    assert_eq!(cf & 0x01, 0x00, "balance field (bit 0) should not be set");
}

// =====================================================================
// Policy Capability Properties
// =====================================================================

use hopper_core::policy::*;

/// Empty CapabilitySet has no capabilities.
#[test]
fn prop_capability_set_empty() {
    let empty = CapabilitySet::new();
    assert_eq!(empty.bits(), 0);
    assert_eq!(empty.count(), 0);
    assert!(!empty.has(Capability::MutatesState));
    assert!(!empty.has(Capability::ReadsState));
}

// =====================================================================
// Math Properties
// =====================================================================

use hopper_core::math::*;

#[test]
fn prop_checked_mul_div_basic() {
    // (a * b) / c  with u128 intermediate
    assert_eq!(checked_mul_div(100, 200, 10).unwrap(), 2000);
    assert_eq!(checked_mul_div(0, 100, 1).unwrap(), 0);
    assert_eq!(checked_mul_div(u64::MAX, 1, 1).unwrap(), u64::MAX);
    // Divide by zero
    assert!(checked_mul_div(1, 1, 0).is_err());
}

#[test]
fn prop_checked_mul_div_no_overflow_for_large_tokens() {
    // Real DeFi scenario: 1_000_000_000 SOL (in lamports) * 1_000_000_000
    // This would overflow u64 with plain mul, but u128 intermediate handles it.
    let a: u64 = 1_000_000_000_000_000_000; // 1B SOL in lamports
    let b: u64 = 999_999_999;
    let c: u64 = 1_000_000_000;
    let result = checked_mul_div(a, b, c).unwrap();
    assert_eq!(result, 999_999_999_000_000_000);
}

#[test]
fn prop_checked_mul_div_ceil_rounds_up() {
    // 10 * 3 / 7 = 4.28... → floor 4, ceil 5
    assert_eq!(checked_mul_div(10, 3, 7).unwrap(), 4);
    assert_eq!(checked_mul_div_ceil(10, 3, 7).unwrap(), 5);
    // Exact division: ceil == floor
    assert_eq!(checked_mul_div(10, 2, 5).unwrap(), 4);
    assert_eq!(checked_mul_div_ceil(10, 2, 5).unwrap(), 4);
}

#[test]
fn prop_bps_of_matches_manual() {
    // 100 bps = 1%
    assert_eq!(bps_of(10_000, 100).unwrap(), 100);
    // 10000 bps = 100%
    assert_eq!(bps_of(10_000, 10_000).unwrap(), 10_000);
    // 25 bps = 0.25%
    assert_eq!(bps_of(1_000_000, 25).unwrap(), 2_500);
    // 0 amount
    assert_eq!(bps_of(0, 500).unwrap(), 0);
}

#[test]
fn prop_bps_of_ceil_never_rounds_to_zero() {
    // 1 * 1 / 10000 = 0.0001 → floor 0, ceil 1
    assert_eq!(bps_of(1, 1).unwrap(), 0);
    assert_eq!(bps_of_ceil(1, 1).unwrap(), 1);
}

#[test]
fn prop_scale_amount_identity() {
    assert_eq!(scale_amount(12345, 6, 6).unwrap(), 12345);
}

#[test]
fn prop_scale_amount_up_and_down() {
    // 6 → 9 decimals: multiply by 1000
    assert_eq!(scale_amount(1_000_000, 6, 9).unwrap(), 1_000_000_000);
    // 9 → 6 decimals: divide by 1000 (floor)
    assert_eq!(scale_amount(1_000_000_999, 9, 6).unwrap(), 1_000_000);
    // Ceiling variant rounds up
    assert_eq!(scale_amount_ceil(1_000_000_001, 9, 6).unwrap(), 1_000_001);
}

#[test]
fn prop_checked_pow_basic() {
    assert_eq!(checked_pow(2, 0).unwrap(), 1);
    assert_eq!(checked_pow(2, 10).unwrap(), 1024);
    assert_eq!(checked_pow(10, 18).unwrap(), 1_000_000_000_000_000_000);
    assert_eq!(checked_pow(1, u32::MAX).unwrap(), 1);
    // Overflow
    assert!(checked_pow(2, 64).is_err());
}

#[test]
fn prop_to_u64_bounds() {
    assert_eq!(to_u64(0u128).unwrap(), 0u64);
    assert_eq!(to_u64(u64::MAX as u128).unwrap(), u64::MAX);
    assert!(to_u64(u64::MAX as u128 + 1).is_err());
}

#[test]
fn prop_checked_div_ceil_basic() {
    assert_eq!(checked_div_ceil(10, 3).unwrap(), 4);
    assert_eq!(checked_div_ceil(9, 3).unwrap(), 3);
    assert_eq!(checked_div_ceil(0, 5).unwrap(), 0);
    assert!(checked_div_ceil(1, 0).is_err());
}

#[test]
fn prop_scale_bps_matches_bps_of() {
    // scale_bps(v, bps) should == bps_of(v, bps as u16) for small bps values
    for bps in [0u64, 1, 25, 100, 500, 10_000] {
        let amount = 1_000_000u64;
        assert_eq!(
            scale_bps(amount, bps).unwrap(),
            bps_of(amount, bps as u16).unwrap(),
            "mismatch at bps={bps}"
        );
    }
}

/// CapabilitySet::with adds exactly one bit.
#[test]
fn prop_capability_set_with() {
    let set = CapabilitySet::new()
        .with(Capability::MutatesState)
        .with(Capability::TouchesJournal);
    assert!(set.has(Capability::MutatesState));
    assert!(set.has(Capability::TouchesJournal));
    assert!(!set.has(Capability::ExternalCall));
    assert_eq!(set.count(), 2);
}

/// CapabilitySet::with is idempotent (adding same cap twice still 1 bit).
#[test]
fn prop_capability_set_idempotent() {
    let set = CapabilitySet::new()
        .with(Capability::CreatesAccount)
        .with(Capability::CreatesAccount);
    assert_eq!(set.count(), 1);
}

/// CapabilitySet::union combines all bits.
#[test]
fn prop_capability_set_union() {
    let a = CapabilitySet::new().with(Capability::MutatesState);
    let b = CapabilitySet::new().with(Capability::ExternalCall);
    let combined = a.union(b);
    assert!(combined.has(Capability::MutatesState));
    assert!(combined.has(Capability::ExternalCall));
    assert_eq!(combined.count(), 2);
}

/// CapabilitySet::is_subset_of works correctly.
#[test]
fn prop_capability_set_subset() {
    let full = CapabilitySet::new()
        .with(Capability::MutatesState)
        .with(Capability::TouchesJournal)
        .with(Capability::ExternalCall);
    let partial = CapabilitySet::new()
        .with(Capability::MutatesState)
        .with(Capability::TouchesJournal);
    let disjoint = CapabilitySet::new()
        .with(Capability::ClosesAccount);

    assert!(partial.is_subset_of(&full));
    assert!(!full.is_subset_of(&partial));
    assert!(!disjoint.is_subset_of(&full));
    assert!(CapabilitySet::new().is_subset_of(&full)); // empty is subset of everything
}

/// All Capability variants have unique bit masks.
#[test]
fn prop_capability_masks_unique() {
    let caps = [
        Capability::ReadsState,
        Capability::MutatesState,
        Capability::TouchesJournal,
        Capability::ExternalCall,
        Capability::MutatesTreasury,
        Capability::ReallocatesAccount,
        Capability::CreatesAccount,
        Capability::ClosesAccount,
        Capability::ModifiesAuthority,
        Capability::TransitionsState,
    ];
    for (i, a) in caps.iter().enumerate() {
        for (j, b) in caps.iter().enumerate() {
            if i != j {
                assert_ne!(a.mask(), b.mask(), "caps {i} and {j} have same mask");
            }
        }
    }
}

/// RequirementSet tracks requirements independently.
#[test]
fn prop_requirement_set_basic() {
    let set = RequirementSet::new()
        .with(PolicyRequirement::Authority)
        .with(PolicyRequirement::InvariantCheck);
    assert!(set.has(PolicyRequirement::Authority));
    assert!(set.has(PolicyRequirement::InvariantCheck));
    assert!(!set.has(PolicyRequirement::CpiGuard));
}

/// InstructionPolicy resolves correct requirements for given capabilities.
#[test]
fn prop_policy_resolve_basic() {
    let policy = InstructionPolicy::<4>::new()
        .when(Capability::MutatesState, PolicyRequirement::Authority)
        .when(Capability::MutatesState, PolicyRequirement::InvariantCheck)
        .when(Capability::ExternalCall, PolicyRequirement::CpiGuard);

    // Caps with MutatesState only
    let caps = CapabilitySet::new().with(Capability::MutatesState);
    let reqs = policy.resolve(&caps);
    assert!(reqs.has(PolicyRequirement::Authority));
    assert!(reqs.has(PolicyRequirement::InvariantCheck));
    assert!(!reqs.has(PolicyRequirement::CpiGuard));

    // Caps with ExternalCall only
    let caps2 = CapabilitySet::new().with(Capability::ExternalCall);
    let reqs2 = policy.resolve(&caps2);
    assert!(!reqs2.has(PolicyRequirement::Authority));
    assert!(reqs2.has(PolicyRequirement::CpiGuard));
}

/// InstructionPolicy with all caps triggers all requirements.
#[test]
fn prop_policy_resolve_all_caps() {
    let policy = InstructionPolicy::<3>::new()
        .when(Capability::MutatesState, PolicyRequirement::Authority)
        .when(Capability::TouchesJournal, PolicyRequirement::JournalCapacity)
        .when(Capability::ExternalCall, PolicyRequirement::CpiGuard);

    let all_caps = CapabilitySet::new()
        .with(Capability::MutatesState)
        .with(Capability::TouchesJournal)
        .with(Capability::ExternalCall);

    let reqs = policy.resolve(&all_caps);
    assert!(reqs.has(PolicyRequirement::Authority));
    assert!(reqs.has(PolicyRequirement::JournalCapacity));
    assert!(reqs.has(PolicyRequirement::CpiGuard));
}

/// InstructionPolicy with empty caps triggers no requirements.
#[test]
fn prop_policy_resolve_empty_caps() {
    let policy = InstructionPolicy::<2>::new()
        .when(Capability::MutatesState, PolicyRequirement::Authority)
        .when(Capability::ExternalCall, PolicyRequirement::CpiGuard);

    let empty = CapabilitySet::new();
    let reqs = policy.resolve(&empty);
    assert_eq!(reqs.bits(), 0);
}

/// InstructionPolicy rule_count tracks number of rules.
#[test]
fn prop_policy_rule_count() {
    let p0 = InstructionPolicy::<4>::new();
    assert_eq!(p0.rule_count(), 0);

    let p2 = InstructionPolicy::<4>::new()
        .when(Capability::MutatesState, PolicyRequirement::Authority)
        .when(Capability::ExternalCall, PolicyRequirement::CpiGuard);
    assert_eq!(p2.rule_count(), 2);
}
