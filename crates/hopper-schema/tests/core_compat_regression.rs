use hopper_schema::{
    compare_fields, is_append_compatible, is_backward_readable, requires_migration, FieldCompat,
    FieldDescriptor, FieldIntent, LayoutManifest, MigrationPlan, MigrationPolicy,
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

static VAULT_V1_FIELDS: &[FieldDescriptor] = &[
    FieldDescriptor {
        name: "authority",
        canonical_type: "[u8;32]",
        size: 32,
        offset: 16,
        intent: FieldIntent::Custom,
    },
    FieldDescriptor {
        name: "balance",
        canonical_type: "WireU64",
        size: 8,
        offset: 48,
        intent: FieldIntent::Custom,
    },
    FieldDescriptor {
        name: "bump",
        canonical_type: "u8",
        size: 1,
        offset: 56,
        intent: FieldIntent::Custom,
    },
];

static VAULT_V2_FIELDS: &[FieldDescriptor] = &[
    FieldDescriptor {
        name: "authority",
        canonical_type: "[u8;32]",
        size: 32,
        offset: 16,
        intent: FieldIntent::Custom,
    },
    FieldDescriptor {
        name: "balance",
        canonical_type: "WireU64",
        size: 8,
        offset: 48,
        intent: FieldIntent::Custom,
    },
    FieldDescriptor {
        name: "bump",
        canonical_type: "u8",
        size: 1,
        offset: 56,
        intent: FieldIntent::Custom,
    },
    FieldDescriptor {
        name: "fee_bps",
        canonical_type: "WireU16",
        size: 2,
        offset: 57,
        intent: FieldIntent::Custom,
    },
];

static VAULT_V2_CHANGED_FIELDS: &[FieldDescriptor] = &[
    FieldDescriptor {
        name: "authority",
        canonical_type: "[u8;32]",
        size: 32,
        offset: 16,
        intent: FieldIntent::Custom,
    },
    FieldDescriptor {
        name: "balance",
        canonical_type: "WireU128",
        size: 16,
        offset: 48,
        intent: FieldIntent::Custom,
    },
    FieldDescriptor {
        name: "bump",
        canonical_type: "u8",
        size: 1,
        offset: 64,
        intent: FieldIntent::Custom,
    },
];

#[test]
fn migration_plan_noop_identical() {
    let v1 = make_manifest("Vault", 1, 1, [1; 8], 57, VAULT_V1_FIELDS);
    let v1_copy = make_manifest("Vault", 1, 1, [1; 8], 57, VAULT_V1_FIELDS);

    let plan = MigrationPlan::<16>::generate(&v1, &v1_copy);
    assert!(matches!(plan.policy, MigrationPolicy::NoOp));
    assert!(plan.is_empty());
}

#[test]
fn migration_plan_append_only() {
    let v1 = make_manifest("Vault", 1, 1, [1; 8], 57, VAULT_V1_FIELDS);
    let v2 = make_manifest("Vault", 1, 2, [2; 8], 59, VAULT_V2_FIELDS);

    assert!(is_append_compatible(&v1, &v2));

    let plan = MigrationPlan::<16>::generate(&v1, &v2);
    assert!(matches!(plan.policy, MigrationPolicy::AppendOnly));
    assert_eq!(plan.old_size, 57);
    assert_eq!(plan.new_size, 59);
}

#[test]
fn migration_plan_requires_migration_on_field_change() {
    let v1 = make_manifest("Vault", 1, 1, [1; 8], 57, VAULT_V1_FIELDS);
    let v2 = make_manifest("Vault", 1, 2, [3; 8], 65, VAULT_V2_CHANGED_FIELDS);

    assert!(requires_migration(&v1, &v2));

    let plan = MigrationPlan::<16>::generate(&v1, &v2);
    assert!(matches!(plan.policy, MigrationPolicy::RequiresMigration));
}

#[test]
fn migration_plan_incompatible_different_disc() {
    let v1 = make_manifest("Vault", 1, 1, [1; 8], 57, VAULT_V1_FIELDS);
    let v2 = make_manifest("Pool", 2, 1, [2; 8], 100, VAULT_V1_FIELDS);

    let plan = MigrationPlan::<16>::generate(&v1, &v2);
    assert!(matches!(plan.policy, MigrationPolicy::Incompatible));
}

#[test]
fn compare_fields_identical() {
    let v1 = make_manifest("Vault", 1, 1, [1; 8], 57, VAULT_V1_FIELDS);
    let v1_copy = make_manifest("Vault", 1, 1, [1; 8], 57, VAULT_V1_FIELDS);

    let report = compare_fields::<8>(&v1, &v1_copy);
    assert!(report.is_append_safe);
    for i in 0..report.len() {
        if let Some(entry) = report.get(i) {
            assert_eq!(entry.status, FieldCompat::Identical);
        }
    }
}

#[test]
fn compare_fields_added() {
    let v1 = make_manifest("Vault", 1, 1, [1; 8], 57, VAULT_V1_FIELDS);
    let v2 = make_manifest("Vault", 1, 2, [2; 8], 59, VAULT_V2_FIELDS);

    let report = compare_fields::<8>(&v1, &v2);
    assert!(report.is_append_safe);
    assert_eq!(report.count_status(FieldCompat::Added), 1);
    assert_eq!(report.count_status(FieldCompat::Identical), 3);
}

#[test]
fn compare_fields_changed() {
    let v1 = make_manifest("Vault", 1, 1, [1; 8], 57, VAULT_V1_FIELDS);
    let v2 = make_manifest("Vault", 1, 2, [3; 8], 65, VAULT_V2_CHANGED_FIELDS);

    let report = compare_fields::<8>(&v1, &v2);
    assert!(!report.is_append_safe);
    assert!(report.count_status(FieldCompat::Changed) > 0);
}

#[test]
fn layout_fingerprint_different_for_different_field_order() {
    let fields_ab: &[FieldDescriptor] = &[
        FieldDescriptor {
            name: "alpha",
            canonical_type: "WireU64",
            size: 8,
            offset: 16,
            intent: FieldIntent::Custom,
        },
        FieldDescriptor {
            name: "beta",
            canonical_type: "WireU32",
            size: 4,
            offset: 24,
            intent: FieldIntent::Custom,
        },
    ];
    let fields_ba: &[FieldDescriptor] = &[
        FieldDescriptor {
            name: "beta",
            canonical_type: "WireU32",
            size: 4,
            offset: 16,
            intent: FieldIntent::Custom,
        },
        FieldDescriptor {
            name: "alpha",
            canonical_type: "WireU64",
            size: 8,
            offset: 20,
            intent: FieldIntent::Custom,
        },
    ];

    let m_ab = make_manifest("Test", 1, 1, [0xAA; 8], 28, leak_fields(fields_ab));
    let m_ba = make_manifest("Test", 1, 1, [0xBB; 8], 28, leak_fields(fields_ba));

    let report = compare_fields::<8>(&m_ab, &m_ba);
    assert!(
        !report.is_append_safe
            || report.count_status(FieldCompat::Changed) > 0
            || report.count_status(FieldCompat::Added) > 0
            || report.count_status(FieldCompat::Removed) > 0
    );
}

fn leak_fields(fields: &[FieldDescriptor]) -> &'static [FieldDescriptor] {
    Box::leak(fields.to_vec().into_boxed_slice())
}

#[test]
fn layout_fingerprint_append_only_detection() {
    let v1_fields: &[FieldDescriptor] = &[
        FieldDescriptor {
            name: "a",
            canonical_type: "WireU64",
            size: 8,
            offset: 16,
            intent: FieldIntent::Custom,
        },
        FieldDescriptor {
            name: "b",
            canonical_type: "WireU32",
            size: 4,
            offset: 24,
            intent: FieldIntent::Custom,
        },
    ];
    let v2_fields: &[FieldDescriptor] = &[
        FieldDescriptor {
            name: "a",
            canonical_type: "WireU64",
            size: 8,
            offset: 16,
            intent: FieldIntent::Custom,
        },
        FieldDescriptor {
            name: "b",
            canonical_type: "WireU32",
            size: 4,
            offset: 24,
            intent: FieldIntent::Custom,
        },
        FieldDescriptor {
            name: "c",
            canonical_type: "WireU16",
            size: 2,
            offset: 28,
            intent: FieldIntent::Custom,
        },
    ];

    let v1 = make_manifest("Layout", 1, 1, [1; 8], 28, leak_fields(v1_fields));
    let v2 = make_manifest("Layout", 1, 2, [2; 8], 30, leak_fields(v2_fields));

    let report = compare_fields::<8>(&v1, &v2);
    assert!(report.is_append_safe);
    assert_eq!(report.count_status(FieldCompat::Added), 1);
    assert_eq!(report.count_status(FieldCompat::Identical), 2);
}

#[test]
fn layout_fingerprint_removal_breaks_append_safety() {
    let v1_fields: &[FieldDescriptor] = &[
        FieldDescriptor {
            name: "a",
            canonical_type: "WireU64",
            size: 8,
            offset: 16,
            intent: FieldIntent::Custom,
        },
        FieldDescriptor {
            name: "b",
            canonical_type: "WireU32",
            size: 4,
            offset: 24,
            intent: FieldIntent::Custom,
        },
    ];
    let v2_fields: &[FieldDescriptor] = &[FieldDescriptor {
        name: "a",
        canonical_type: "WireU64",
        size: 8,
        offset: 16,
        intent: FieldIntent::Custom,
    }];

    let v1 = make_manifest("Layout", 1, 1, [1; 8], 28, leak_fields(v1_fields));
    let v2 = make_manifest("Layout", 1, 2, [2; 8], 24, leak_fields(v2_fields));

    let report = compare_fields::<8>(&v1, &v2);
    assert!(!report.is_append_safe);
    assert!(report.count_status(FieldCompat::Removed) > 0);
}

#[test]
fn backward_readable_identical() {
    let v1 = make_manifest("Vault", 1, 1, [1; 8], 57, VAULT_V1_FIELDS);
    let v1_copy = make_manifest("Vault", 1, 1, [1; 8], 57, VAULT_V1_FIELDS);
    assert!(is_backward_readable(&v1, &v1_copy));
}

#[test]
fn backward_readable_append_only() {
    let v1 = make_manifest("Vault", 1, 1, [1; 8], 57, VAULT_V1_FIELDS);
    let v2 = make_manifest("Vault", 1, 2, [2; 8], 59, VAULT_V2_FIELDS);
    assert!(is_backward_readable(&v1, &v2));
}

#[test]
fn backward_readable_false_when_field_type_changed() {
    let v1 = make_manifest("Vault", 1, 1, [1; 8], 57, VAULT_V1_FIELDS);
    let v2 = make_manifest("Vault", 1, 2, [3; 8], 65, VAULT_V2_CHANGED_FIELDS);
    assert!(!is_backward_readable(&v1, &v2));
}

#[test]
fn backward_readable_false_different_disc() {
    let v1 = make_manifest("Vault", 1, 1, [1; 8], 57, VAULT_V1_FIELDS);
    let v2 = make_manifest("Pool", 2, 1, [2; 8], 100, VAULT_V1_FIELDS);
    assert!(!is_backward_readable(&v1, &v2));
}

#[test]
fn backward_readable_false_fewer_fields_in_newer() {
    let v2 = make_manifest("Vault", 1, 2, [2; 8], 59, VAULT_V2_FIELDS);
    let v1 = make_manifest("Vault", 1, 1, [1; 8], 57, VAULT_V1_FIELDS);
    assert!(!is_backward_readable(&v2, &v1));
}

#[test]
fn migration_plan_backward_readable_populated() {
    let v1 = make_manifest("Vault", 1, 1, [1; 8], 57, VAULT_V1_FIELDS);
    let v2 = make_manifest("Vault", 1, 2, [2; 8], 59, VAULT_V2_FIELDS);
    let plan = MigrationPlan::<16>::generate(&v1, &v2);
    assert!(plan.backward_readable);

    let v2_changed = make_manifest("Vault", 1, 2, [3; 8], 65, VAULT_V2_CHANGED_FIELDS);
    let plan2 = MigrationPlan::<16>::generate(&v1, &v2_changed);
    assert!(!plan2.backward_readable);
}
