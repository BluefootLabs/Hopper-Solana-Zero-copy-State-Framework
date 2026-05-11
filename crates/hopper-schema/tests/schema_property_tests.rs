use hopper_schema::{
    compare_fields, is_append_compatible, requires_migration, FieldDescriptor, FieldIntent,
    LayoutManifest,
};

/// Same manifest -> no-op migration.
#[test]
fn prop_schema_same_manifest_noop() {
    let fields = &[FieldDescriptor {
        name: "balance",
        canonical_type: "WireU64",
        size: 8,
        offset: 16,
        intent: FieldIntent::Custom,
    }];
    let manifest = LayoutManifest {
        name: "Vault",
        version: 1,
        disc: 1,
        layout_id: [1, 2, 3, 4, 5, 6, 7, 8],
        total_size: 24,
        field_count: 1,
        fields,
    };
    assert!(!requires_migration(&manifest, &manifest));
    assert!(!is_append_compatible(&manifest, &manifest));
}

/// Added field -> append-compatible.
#[test]
fn prop_schema_append_compatible() {
    let old_fields = &[FieldDescriptor {
        name: "balance",
        canonical_type: "WireU64",
        size: 8,
        offset: 16,
        intent: FieldIntent::Custom,
    }];
    let new_fields = &[
        FieldDescriptor {
            name: "balance",
            canonical_type: "WireU64",
            size: 8,
            offset: 16,
            intent: FieldIntent::Custom,
        },
        FieldDescriptor {
            name: "extra",
            canonical_type: "WireU32",
            size: 4,
            offset: 24,
            intent: FieldIntent::Custom,
        },
    ];
    let old = LayoutManifest {
        name: "Vault",
        version: 1,
        disc: 1,
        layout_id: [1, 2, 3, 4, 5, 6, 7, 8],
        total_size: 24,
        field_count: 1,
        fields: old_fields,
    };
    let new = LayoutManifest {
        name: "Vault",
        version: 2,
        disc: 1,
        layout_id: [8, 7, 6, 5, 4, 3, 2, 1],
        total_size: 28,
        field_count: 2,
        fields: new_fields,
    };
    assert!(is_append_compatible(&old, &new));
}

/// Changed field type -> requires migration.
#[test]
fn prop_schema_changed_type_requires_migration() {
    let old_fields = &[FieldDescriptor {
        name: "balance",
        canonical_type: "WireU64",
        size: 8,
        offset: 16,
        intent: FieldIntent::Custom,
    }];
    let new_fields = &[FieldDescriptor {
        name: "balance",
        canonical_type: "WireU128",
        size: 16,
        offset: 16,
        intent: FieldIntent::Custom,
    }];
    let old = LayoutManifest {
        name: "Vault",
        version: 1,
        disc: 1,
        layout_id: [1, 2, 3, 4, 5, 6, 7, 8],
        total_size: 24,
        field_count: 1,
        fields: old_fields,
    };
    let new = LayoutManifest {
        name: "Vault",
        version: 2,
        disc: 1,
        layout_id: [9, 9, 9, 9, 9, 9, 9, 9],
        total_size: 32,
        field_count: 1,
        fields: new_fields,
    };
    assert!(requires_migration(&old, &new));
    let report = compare_fields::<4>(&old, &new);
    assert!(!report.is_append_safe);
}
