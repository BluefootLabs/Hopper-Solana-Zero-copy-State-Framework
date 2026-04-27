//! # Compatibility Regression Tests
//!
//! Ensures schema compatibility functions remain correct as the codebase
//! evolves. Tests cover:
//! - Append-safe additions detected correctly
//! - Forbidden field reorder / rename / resize rejected
//! - Field removal detected as breaking
//! - Field type change detected as breaking
//! - compare_fields report accuracy
//! - is_backward_readable correctness
//! - requires_migration correctness
//! - Receipt wire format roundtrip (encode → decode equivalence)
//! - Receipt enrichment (phase, compat_impact, migration_flags) roundtrip

extern crate alloc;

use hopper_schema::{
    FieldDescriptor, FieldIntent, LayoutManifest, FieldCompat,
    compare_fields, is_append_compatible, requires_migration,
    is_backward_readable,
};
use hopper_core::receipt::{
    StateReceipt, DecodedReceipt, RECEIPT_SIZE,
    Phase, CompatImpact,
};

// =====================================================================
// Manifest test helpers
// =====================================================================

const FIELD_A: FieldDescriptor = FieldDescriptor {
    name: "alpha",
    canonical_type: "u64",
    size: 8,
    offset: 16,
    intent: FieldIntent::Custom,
};

const FIELD_B: FieldDescriptor = FieldDescriptor {
    name: "beta",
    canonical_type: "u64",
    size: 8,
    offset: 24,
    intent: FieldIntent::Custom,
};

const FIELD_C: FieldDescriptor = FieldDescriptor {
    name: "gamma",
    canonical_type: "u32",
    size: 4,
    offset: 32,
    intent: FieldIntent::Custom,
};

const FIELD_B_RENAMED: FieldDescriptor = FieldDescriptor {
    name: "beta_v2",
    canonical_type: "u64",
    size: 8,
    offset: 24,
    intent: FieldIntent::Custom,
};

const FIELD_B_RESIZED: FieldDescriptor = FieldDescriptor {
    name: "beta",
    canonical_type: "u128",
    size: 16,
    offset: 24,
    intent: FieldIntent::Custom,
};

fn make_manifest(
    name: &'static str,
    disc: u8,
    version: u8,
    layout_id: [u8; 8],
    total_size: usize,
    fields: &'static [FieldDescriptor],
) -> LayoutManifest {
    LayoutManifest {
        name,
        disc,
        version,
        layout_id,
        total_size,
        field_count: fields.len(),
        fields,
    }
}

// =====================================================================
// Append-safe addition tests
// =====================================================================

static V1_FIELDS: [FieldDescriptor; 2] = [FIELD_A, FIELD_B];
static V2_FIELDS_APPENDED: [FieldDescriptor; 3] = [FIELD_A, FIELD_B, FIELD_C];

#[test]
fn append_safe_addition_detected() {
    let v1 = make_manifest("Test", 1, 1, [0xAA; 8], 32, &V1_FIELDS);
    let v2 = make_manifest("Test", 1, 2, [0xBB; 8], 36, &V2_FIELDS_APPENDED);
    assert!(is_append_compatible(&v1, &v2));
}

#[test]
fn append_safe_report_marks_added_field() {
    let v1 = make_manifest("Test", 1, 1, [0xAA; 8], 32, &V1_FIELDS);
    let v2 = make_manifest("Test", 1, 2, [0xBB; 8], 36, &V2_FIELDS_APPENDED);
    let report = compare_fields::<8>(&v1, &v2);
    assert!(report.is_append_safe());
    assert_eq!(report.len(), 3);
    assert_eq!(report.get(0).unwrap().status, FieldCompat::Identical);
    assert_eq!(report.get(1).unwrap().status, FieldCompat::Identical);
    assert_eq!(report.get(2).unwrap().status, FieldCompat::Added);
    assert_eq!(report.get(2).unwrap().name, "gamma");
}

#[test]
fn identical_manifests_are_not_append_compatible() {
    // Same version + same layout_id means no change happened
    let v1 = make_manifest("Test", 1, 1, [0xAA; 8], 32, &V1_FIELDS);
    let v1_copy = make_manifest("Test", 1, 1, [0xAA; 8], 32, &V1_FIELDS);
    // is_append_compatible requires version bump & different layout_id
    assert!(!is_append_compatible(&v1, &v1_copy));
}

// =====================================================================
// Forbidden field reorder / rename
// =====================================================================

static V2_FIELDS_RENAMED: [FieldDescriptor; 2] = [FIELD_A, FIELD_B_RENAMED];

#[test]
fn field_rename_detected_as_changed() {
    let v1 = make_manifest("Test", 1, 1, [0xAA; 8], 32, &V1_FIELDS);
    let v2 = make_manifest("Test", 1, 2, [0xBB; 8], 32, &V2_FIELDS_RENAMED);
    let report = compare_fields::<8>(&v1, &v2);
    assert!(!report.is_append_safe());
    assert_eq!(report.get(1).unwrap().status, FieldCompat::Changed);
}

#[test]
fn field_rename_not_append_compatible() {
    let v1 = make_manifest("Test", 1, 1, [0xAA; 8], 32, &V1_FIELDS);
    let v2 = make_manifest("Test", 1, 2, [0xBB; 8], 32, &V2_FIELDS_RENAMED);
    // Even though disc matches and version bumped, name change = not safe
    // (is_append_compatible checks size >= old, version >, different layout_id, same disc)
    // It doesn't check fields, it's structural. But field report says not safe.
    let report = compare_fields::<8>(&v1, &v2);
    assert!(!report.is_append_safe());
}

// =====================================================================
// Field removal (breaking)
// =====================================================================

static V2_FIELDS_REMOVED: [FieldDescriptor; 1] = [FIELD_A];

#[test]
fn field_removal_detected_as_removed() {
    let v1 = make_manifest("Test", 1, 1, [0xAA; 8], 32, &V1_FIELDS);
    let v2 = make_manifest("Test", 1, 2, [0xBB; 8], 24, &V2_FIELDS_REMOVED);
    let report = compare_fields::<8>(&v1, &v2);
    assert!(!report.is_append_safe());
    assert_eq!(report.len(), 2);
    assert_eq!(report.get(0).unwrap().status, FieldCompat::Identical);
    assert_eq!(report.get(1).unwrap().status, FieldCompat::Removed);
    assert_eq!(report.get(1).unwrap().name, "beta");
}

#[test]
fn field_removal_requires_migration() {
    let v1 = make_manifest("Test", 1, 1, [0xAA; 8], 32, &V1_FIELDS);
    let v2 = make_manifest("Test", 1, 2, [0xBB; 8], 24, &V2_FIELDS_REMOVED);
    assert!(requires_migration(&v1, &v2));
}

// =====================================================================
// Field type/size change (breaking)
// =====================================================================

static V2_FIELDS_RESIZED: [FieldDescriptor; 2] = [FIELD_A, FIELD_B_RESIZED];

#[test]
fn field_type_change_detected_as_changed() {
    let v1 = make_manifest("Test", 1, 1, [0xAA; 8], 32, &V1_FIELDS);
    let v2 = make_manifest("Test", 1, 2, [0xBB; 8], 40, &V2_FIELDS_RESIZED);
    let report = compare_fields::<8>(&v1, &v2);
    assert!(!report.is_append_safe());
    assert_eq!(report.get(1).unwrap().status, FieldCompat::Changed);
}

#[test]
fn field_type_change_requires_migration() {
    let v1 = make_manifest("Test", 1, 1, [0xAA; 8], 32, &V1_FIELDS);
    let v2 = make_manifest("Test", 1, 2, [0xBB; 8], 40, &V2_FIELDS_RESIZED);
    assert!(requires_migration(&v1, &v2));
}

// =====================================================================
// Backward readability
// =====================================================================

#[test]
fn append_only_is_backward_readable() {
    let v1 = make_manifest("Test", 1, 1, [0xAA; 8], 32, &V1_FIELDS);
    let v2 = make_manifest("Test", 1, 2, [0xBB; 8], 36, &V2_FIELDS_APPENDED);
    assert!(is_backward_readable(&v1, &v2));
}

#[test]
fn field_change_is_not_backward_readable() {
    let v1 = make_manifest("Test", 1, 1, [0xAA; 8], 32, &V1_FIELDS);
    let v2 = make_manifest("Test", 1, 2, [0xBB; 8], 40, &V2_FIELDS_RESIZED);
    assert!(!is_backward_readable(&v1, &v2));
}

// =====================================================================
// compare_fields edge cases
// =====================================================================

static EMPTY_FIELDS: [FieldDescriptor; 0] = [];

#[test]
fn compare_empty_to_nonempty() {
    let empty = make_manifest("E", 1, 1, [0; 8], 16, &EMPTY_FIELDS);
    let full = make_manifest("F", 1, 2, [1; 8], 32, &V1_FIELDS);
    let report = compare_fields::<8>(&empty, &full);
    assert!(report.is_append_safe()); // adding fields to empty is append-safe
    assert_eq!(report.len(), 2);
    assert_eq!(report.get(0).unwrap().status, FieldCompat::Added);
    assert_eq!(report.get(1).unwrap().status, FieldCompat::Added);
}

#[test]
fn compare_nonempty_to_empty() {
    let full = make_manifest("F", 1, 1, [0; 8], 32, &V1_FIELDS);
    let empty = make_manifest("E", 1, 2, [1; 8], 16, &EMPTY_FIELDS);
    let report = compare_fields::<8>(&full, &empty);
    assert!(!report.is_append_safe()); // removing all fields is breaking
    assert_eq!(report.len(), 2);
    assert_eq!(report.get(0).unwrap().status, FieldCompat::Removed);
    assert_eq!(report.get(1).unwrap().status, FieldCompat::Removed);
}

#[test]
fn compare_empty_to_empty() {
    let e1 = make_manifest("A", 1, 1, [0; 8], 16, &EMPTY_FIELDS);
    let e2 = make_manifest("B", 1, 1, [0; 8], 16, &EMPTY_FIELDS);
    let report = compare_fields::<8>(&e1, &e2);
    assert!(report.is_append_safe());
    assert_eq!(report.len(), 0);
}

#[test]
fn compare_fields_const_n_truncation() {
    // If N < total fields, we lose some entries but don't panic
    let v1 = make_manifest("Test", 1, 1, [0xAA; 8], 32, &V1_FIELDS);
    let v2 = make_manifest("Test", 1, 2, [0xBB; 8], 36, &V2_FIELDS_APPENDED);
    // N=2 means we can only hold 2 of 3 entries
    let report = compare_fields::<2>(&v1, &v2);
    assert_eq!(report.len(), 2);
}

// =====================================================================
// Receipt wire format roundtrip
// =====================================================================

#[test]
fn receipt_encode_decode_roundtrip() {
    let data = [0xABu8; 64];
    let layout_id = [0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88];
    let mut receipt = StateReceipt::<64>::begin(&layout_id, &data);

    // Mutate some bytes
    let mut mutated = data;
    mutated[20] = 0xFF;
    mutated[21] = 0xFE;

    receipt.commit(&mutated);
    receipt.set_invariants(true, 5);
    receipt.set_policy_flags(0xCAFE);
    receipt.set_cpi_count(2);
    receipt.set_journal_appends(3);
    receipt.set_phase(Phase::Init);
    receipt.set_validation_bundle_id(42);
    receipt.set_compat_impact(CompatImpact::Append);
    receipt.set_migration_flags(0b101);

    let wire = receipt.to_bytes();
    assert_eq!(wire.len(), RECEIPT_SIZE);

    let decoded = DecodedReceipt::from_bytes(&wire).unwrap();
    assert_eq!(decoded.layout_id, layout_id);
    assert!(decoded.committed);
    assert!(decoded.invariants_passed);
    assert_eq!(decoded.invariants_checked, 5);
    assert_eq!(decoded.policy_flags, 0xCAFE);
    assert_eq!(decoded.cpi_count, 2);
    assert_eq!(decoded.journal_appends, 3);
    assert_eq!(decoded.phase, Phase::Init as u8);
    assert_eq!(decoded.validation_bundle_id, 42);
    assert_eq!(decoded.compat_impact, CompatImpact::Append as u8);
    assert_eq!(decoded.migration_flags, 0b101);
    assert!(decoded.cpi_invoked);
}

#[test]
fn receipt_decode_too_short_returns_none() {
    let data = [0u8; 63]; // need 64
    assert!(DecodedReceipt::from_bytes(&data).is_none());
}

#[test]
fn receipt_no_mutation_fingerprint_unchanged() {
    let data = [0x42u8; 32];
    let layout_id = [0; 8];
    let mut receipt = StateReceipt::<32>::begin(&layout_id, &data);
    receipt.commit(&data); // same data
    assert!(!receipt.fingerprint_changed());
    assert!(!receipt.has_changes());
}

#[test]
fn receipt_resize_detected() {
    let data = [0u8; 32];
    let layout_id = [0; 8];
    let mut receipt = StateReceipt::<32>::begin(&layout_id, &data);
    let bigger = [0u8; 48];
    receipt.commit(&bigger);
    assert!(receipt.was_resized);
    assert!(receipt.has_changes());
    assert_eq!(receipt.old_size, 32);
    assert_eq!(receipt.new_size, 48);
}

// =====================================================================
// Receipt enrichment roundtrip
// =====================================================================

#[test]
fn phase_enum_roundtrip() {
    for tag in 0u8..=5 {
        let phase = Phase::from_tag(tag);
        match tag {
            0 => assert_eq!(phase, Phase::Update),
            1 => assert_eq!(phase, Phase::Init),
            2 => assert_eq!(phase, Phase::Close),
            3 => assert_eq!(phase, Phase::Migrate),
            4 => assert_eq!(phase, Phase::ReadOnly),
            _ => assert_eq!(phase, Phase::Update), // unknown → Update
        }
    }
}

#[test]
fn compat_impact_enum_roundtrip() {
    for tag in 0u8..=4 {
        let impact = CompatImpact::from_tag(tag);
        match tag {
            0 => assert_eq!(impact, CompatImpact::None),
            1 => assert_eq!(impact, CompatImpact::Append),
            2 => assert_eq!(impact, CompatImpact::Migration),
            3 => assert_eq!(impact, CompatImpact::Breaking),
            _ => assert_eq!(impact, CompatImpact::None), // unknown → None
        }
    }
}

#[test]
fn phase_name_coverage() {
    assert_eq!(Phase::Update.name(), "Update");
    assert_eq!(Phase::Init.name(), "Init");
    assert_eq!(Phase::Close.name(), "Close");
    assert_eq!(Phase::Migrate.name(), "Migrate");
    assert_eq!(Phase::ReadOnly.name(), "ReadOnly");
}

#[test]
fn compat_impact_name_coverage() {
    assert_eq!(CompatImpact::None.name(), "none");
    assert_eq!(CompatImpact::Append.name(), "append");
    assert_eq!(CompatImpact::Migration.name(), "migration");
    assert_eq!(CompatImpact::Breaking.name(), "breaking");
}

#[test]
fn receipt_segment_mask_roundtrip() {
    let data = [0u8; 64];
    let layout_id = [0; 8];
    let mut receipt = StateReceipt::<64>::begin(&layout_id, &data);

    let mut mutated = data;
    mutated[20..24].copy_from_slice(&[0xFF; 4]);

    let segments = [(16usize, 8usize), (24, 8), (32, 8)];
    receipt.commit_with_segments(&mutated, &segments);

    // Segment 0 (offset 16, size 8) includes bytes 20-24 → changed
    assert_ne!(receipt.segment_changed_mask & 0x01, 0);

    let wire = receipt.to_bytes();
    let decoded = DecodedReceipt::from_bytes(&wire).unwrap();
    assert_eq!(decoded.segment_changed_mask, receipt.segment_changed_mask);
}

#[test]
fn receipt_field_mask_roundtrip() {
    let data = [0u8; 32];
    let layout_id = [0; 8];
    let mut receipt = StateReceipt::<32>::begin(&layout_id, &data);

    let mut mutated = data;
    mutated[8] = 0xFF; // change field at offset 8

    let fields = [("alpha", 0usize, 8usize), ("beta", 8, 8), ("gamma", 16, 8)];
    receipt.commit_with_fields(&mutated, &fields);

    // Field 1 (beta, offset 8) changed
    assert_ne!(receipt.changed_fields & (1 << 1), 0);
    // Field 0 (alpha, offset 0) unchanged
    assert_eq!(receipt.changed_fields & (1 << 0), 0);

    let wire = receipt.to_bytes();
    let decoded = DecodedReceipt::from_bytes(&wire).unwrap();
    assert_eq!(decoded.changed_fields, receipt.changed_fields);
}

// =====================================================================
// All wire format bytes are accounted for
// =====================================================================

#[test]
fn receipt_wire_format_reserved_byte_is_zero() {
    // Verify byte 63 (reserved) stays zero even with all fields set
    let layout_id = [0xFF; 8];
    let data = [0x42u8; 128];
    let mut receipt = StateReceipt::<128>::begin(&layout_id, &data);

    let mut mutated = [0x99u8; 128];
    mutated[0..8].copy_from_slice(&[0xFF; 8]);

    receipt.commit(&mutated);
    receipt.changed_fields = u64::MAX;
    receipt.set_invariants(true, u16::MAX);
    receipt.set_cpi_count(255);
    receipt.set_policy_flags(u32::MAX);
    receipt.set_journal_appends(u16::MAX);
    receipt.set_phase(Phase::Migrate);
    receipt.set_validation_bundle_id(u16::MAX);
    receipt.set_compat_impact(CompatImpact::Breaking);
    receipt.set_migration_flags(0xFF);

    let wire = receipt.to_bytes();

    // Bytes 69..72 are the reserved tail of the 72-byte receipt format.
    // Byte 63 is now `failed_invariant_idx` (defaults to FAILED_INVARIANT_NONE
    // = 0xFF when no invariant failed).
    assert_eq!(&wire[69..72], &[0u8; 3], "reserved trailing bytes must be zero");

    // Known non-computed fields at specific offsets must be non-zero
    assert_ne!(&wire[0..8], &[0u8; 8], "layout_id should be non-zero");
    assert_ne!(&wire[8..16], &[0u8; 8], "changed_fields should be non-zero");
    assert_ne!(wire[32], 0, "flags byte should be non-zero (committed + invariants_passed + cpi)");
    assert_ne!(&wire[51..55], &[0u8; 4], "policy_flags should be non-zero");
    assert_ne!(&wire[55..57], &[0u8; 2], "journal_appends should be non-zero");
    assert_ne!(wire[57], 0, "cpi_count should be non-zero");
    assert_ne!(wire[58], 0, "phase should be non-zero (Migrate=3)");
    assert_ne!(&wire[59..61], &[0u8; 2], "validation_bundle_id should be non-zero");
    assert_ne!(wire[61], 0, "compat_impact should be non-zero (Breaking=3)");
    assert_ne!(wire[62], 0, "migration_flags should be non-zero");
}

// =====================================================================
// ReceiptExplain enrichment
// =====================================================================

#[test]
fn receipt_explain_with_segment_roles() {
    let layout_id = [1u8; 8];
    let data = [0u8; 64];
    let mut receipt = StateReceipt::<64>::begin(&layout_id, &data);

    let mut mutated = data;
    mutated[16] = 0xFF;
    receipt.commit(&mutated);

    let decoded = DecodedReceipt::from_bytes(&receipt.to_bytes()).unwrap();
    let explain = decoded.explain()
        .with_policy_name("TreasuryWrite")
        .with_segment_role(0, "core")
        .with_segment_role(1, "journal");

    assert_eq!(explain.policy_name, "TreasuryWrite");
    assert_eq!(explain.segment_role_names[0], "core");
    assert_eq!(explain.segment_role_names[1], "journal");
    assert_eq!(explain.segment_role_count, 2);
    assert_eq!(explain.segment_role_names[2], ""); // unset
}

#[test]
fn receipt_explain_summary_multi_segment() {
    let layout_id = [1u8; 8];
    let data = [0u8; 64];
    let mut receipt = StateReceipt::<64>::begin(&layout_id, &data);
    receipt.changed_fields = 0b11; // 2 fields changed
    receipt.segment_changed_mask = 0b11; // 2 segments

    let mut mutated = data;
    mutated[16] = 0xFF;
    receipt.commit(&mutated);

    let decoded = DecodedReceipt::from_bytes(&receipt.to_bytes()).unwrap();
    let explain = decoded.explain();

    // Summary should mention multiple segments
    let summary = explain.summary();
    assert!(
        summary.contains("multiple segments") || summary.contains("mutated"),
        "summary '{}' should describe mutation",
        summary
    );
}
