use hopper_topology::{
    route_bucket, solve, CandidatePlan, CandidateSpace, CertificateGrade, ClusterConstraints,
    Completeness, LogicalAccess, LogicalAtom, ProjectBudgets, Provenance, ProvenanceKind, Ratio,
    SelectionPolicy, TransactionClass, VirtualBucket, WorkloadProfile, PROFILE_SCHEMA_V1,
};

fn access(atom: &str, bucket: u16) -> LogicalAccess {
    LogicalAccess {
        atom: atom.to_string(),
        bucket,
    }
}

fn transaction(
    id: &str,
    reads: Vec<LogicalAccess>,
    writes: Vec<LogicalAccess>,
) -> TransactionClass {
    TransactionClass {
        id: id.to_string(),
        weight: 1,
        fixed_account_locks: 2,
        fixed_message_bytes: 100,
        reads,
        writes,
    }
}

fn base_profile(
    atoms: Vec<LogicalAtom>,
    buckets: Vec<VirtualBucket>,
    transactions: Vec<TransactionClass>,
) -> WorkloadProfile {
    WorkloadProfile {
        schema: PROFILE_SCHEMA_V1.to_string(),
        program_id: "Loom111111111111111111111111111111111111".to_string(),
        manifest_commitment: "11".repeat(32),
        provenance: Provenance {
            kind: ProvenanceKind::ConfirmedCluster,
            source_id: "fixture".to_string(),
            first_slot: Some(10),
            last_slot: Some(20),
        },
        completeness: Completeness {
            transaction_accounts_complete: true,
            reads_complete: true,
            writes_complete: true,
            cpi_frames_complete: true,
            remaining_accounts_resolved: true,
            partial_records: 0,
        },
        cluster: ClusterConstraints {
            genesis_hash: "genesis-fixture".to_string(),
            observed_slot: 20,
            feature_set: "fixture-features".to_string(),
            max_transaction_account_locks: 64,
            max_transaction_bytes: 1_232,
            max_account_data_bytes: 10_485_760,
            account_header_bytes: 16,
            account_storage_overhead_bytes: 64,
            rent_exempt_base_bytes: 128,
            rent_lamports_per_byte: 1,
            additional_account_message_bytes: 33,
        },
        budgets: ProjectBudgets {
            max_physical_accounts: 1_000,
            max_total_rent_lamports: u64::MAX,
            max_migration_bytes: u64::MAX,
        },
        policy: SelectionPolicy::ThroughputFirst,
        candidate_space: CandidateSpace {
            routing_seed: "loom-test".to_string(),
            shard_counts: vec![2],
            include_vertical: true,
            include_horizontal: true,
            include_hybrid: true,
        },
        atoms,
        virtual_buckets: buckets,
        transactions,
    }
}

fn atom(id: &str, current: &str, shardable: bool) -> LogicalAtom {
    LogicalAtom {
        id: id.to_string(),
        size_bytes: 8,
        current_account: current.to_string(),
        shardable,
    }
}

fn candidate<'a>(plan: &'a hopper_topology::TopologyPlan, id: &str) -> &'a CandidatePlan {
    plan.feasible_candidates
        .iter()
        .find(|candidate| candidate.id == id)
        .unwrap_or_else(|| panic!("missing feasible candidate {id}"))
}

#[test]
fn account_lock_semantics_distinguish_rr_wr_and_ww() {
    let atoms = vec![atom("cell", "shared", false)];
    let buckets = vec![VirtualBucket { id: 0, weight: 1 }];

    let rr = solve(base_profile(
        atoms.clone(),
        buckets.clone(),
        vec![
            transaction("a", vec![access("cell", 0)], vec![]),
            transaction("b", vec![access("cell", 0)], vec![]),
        ],
    ))
    .unwrap();
    assert_eq!(
        candidate(&rr, "current").metrics.weighted_lock_conflict,
        Ratio::ZERO
    );

    let wr = solve(base_profile(
        atoms.clone(),
        buckets.clone(),
        vec![
            transaction("reader", vec![access("cell", 0)], vec![]),
            transaction("writer", vec![], vec![access("cell", 0)]),
        ],
    ))
    .unwrap();
    assert_eq!(
        candidate(&wr, "current").metrics.weighted_lock_conflict,
        Ratio::new(3, 4)
    );

    let ww = solve(base_profile(
        atoms,
        buckets,
        vec![
            transaction("a", vec![], vec![access("cell", 0)]),
            transaction("b", vec![], vec![access("cell", 0)]),
        ],
    ))
    .unwrap();
    assert_eq!(
        candidate(&ww, "current").metrics.weighted_lock_conflict,
        Ratio::new(1, 1)
    );
}

#[test]
fn exact_cells_in_one_account_still_conflict() {
    let profile = base_profile(
        vec![
            atom("left-cell", "same-account", false),
            atom("right-cell", "same-account", false),
        ],
        vec![VirtualBucket { id: 0, weight: 1 }],
        vec![
            transaction("left", vec![], vec![access("left-cell", 0)]),
            transaction("right", vec![], vec![access("right-cell", 0)]),
        ],
    );
    let plan = solve(profile).unwrap();
    assert_eq!(
        candidate(&plan, "current").metrics.weighted_lock_conflict,
        Ratio::new(1, 1)
    );
    assert_eq!(
        candidate(&plan, "vertical").metrics.weighted_lock_conflict,
        Ratio::new(1, 2)
    );
}

#[test]
fn uniform_one_key_per_shard_has_one_over_k_conflict() {
    const K: u16 = 4;
    let mut representative = [None; K as usize];
    for bucket in 0..=u16::MAX {
        let shard = route_bucket("loom-test", bucket, K) as usize;
        representative[shard].get_or_insert(bucket);
        if representative.iter().all(Option::is_some) {
            break;
        }
    }
    let bucket_ids: Vec<u16> = representative
        .into_iter()
        .map(|bucket| bucket.expect("each shard has a representative"))
        .collect();
    let buckets = bucket_ids
        .iter()
        .map(|id| VirtualBucket { id: *id, weight: 1 })
        .collect();
    let transactions = bucket_ids
        .iter()
        .enumerate()
        .map(|(index, bucket)| {
            transaction(
                &format!("bucket-{index}"),
                vec![],
                vec![access("records", *bucket)],
            )
        })
        .collect();
    let mut profile = base_profile(
        vec![atom("records", "monolith", true)],
        buckets,
        transactions,
    );
    profile.candidate_space.shard_counts = vec![K];
    let plan = solve(profile).unwrap();
    assert_eq!(
        candidate(&plan, "horizontal-4")
            .metrics
            .weighted_lock_conflict,
        Ratio::new(1, K as u128)
    );
}

#[test]
fn hot_key_floor_is_reported_as_an_exact_rational() {
    let profile = base_profile(
        vec![atom("records", "monolith", true)],
        vec![
            VirtualBucket { id: 3, weight: 9 },
            VirtualBucket { id: 8, weight: 1 },
        ],
        vec![transaction("write", vec![], vec![access("records", 3)])],
    );
    let plan = solve(profile).unwrap();
    assert_eq!(plan.routing_bucket_collision_floor, Ratio::new(41, 50));
}

#[test]
fn vertical_split_exposes_parallelism_and_account_tradeoff() {
    let profile = base_profile(
        vec![
            atom("price", "market", false),
            atom("queue", "market", false),
        ],
        vec![VirtualBucket { id: 0, weight: 1 }],
        vec![
            transaction("price", vec![], vec![access("price", 0)]),
            transaction("queue", vec![], vec![access("queue", 0)]),
            transaction("both", vec![], vec![access("price", 0), access("queue", 0)]),
        ],
    );
    let plan = solve(profile).unwrap();
    let current = candidate(&plan, "current");
    let vertical = candidate(&plan, "vertical");
    assert_eq!(current.metrics.weighted_lock_conflict, Ratio::new(1, 1));
    assert!(vertical
        .metrics
        .weighted_lock_conflict
        .compare(current.metrics.weighted_lock_conflict)
        .is_lt());
    assert_eq!(current.metrics.max_transaction_account_locks, 3);
    assert_eq!(vertical.metrics.max_transaction_account_locks, 4);
}

#[test]
fn rent_and_account_budgets_reject_vertical_split() {
    let mut profile = base_profile(
        vec![atom("a", "one", false), atom("b", "one", false)],
        vec![VirtualBucket { id: 0, weight: 1 }],
        vec![transaction(
            "both",
            vec![],
            vec![access("a", 0), access("b", 0)],
        )],
    );
    profile.candidate_space.include_horizontal = false;
    profile.candidate_space.include_hybrid = false;
    profile.budgets.max_physical_accounts = 1;
    profile.budgets.max_total_rent_lamports = 200;
    let plan = solve(profile).unwrap();
    assert!(plan.feasible_candidates.iter().any(|c| c.id == "current"));
    let rejected = plan
        .rejected_candidates
        .iter()
        .find(|candidate| candidate.id == "vertical")
        .expect("vertical rejected");
    assert!(rejected
        .reasons
        .iter()
        .any(|reason| reason.contains("physical account count")));
    assert!(rejected
        .reasons
        .iter()
        .any(|reason| reason.contains("rent")));
}

#[test]
fn normalized_input_order_has_identical_commitments() {
    let mut first = base_profile(
        vec![atom("a", "one", true), atom("b", "one", true)],
        vec![
            VirtualBucket { id: 0, weight: 1 },
            VirtualBucket { id: 1, weight: 1 },
        ],
        vec![
            transaction("alpha", vec![access("b", 1), access("a", 0)], vec![]),
            transaction("beta", vec![], vec![access("a", 1)]),
        ],
    );
    let mut second = first.clone();
    second.atoms.reverse();
    second.virtual_buckets.reverse();
    second.transactions.reverse();
    second.transactions[1].reads.reverse();
    first.candidate_space.shard_counts = vec![2, 2];
    second.candidate_space.shard_counts = vec![2];

    let a = solve(first).unwrap();
    let b = solve(second).unwrap();
    assert_eq!(a.input_commitment, b.input_commitment);
    assert_eq!(a.plan_commitment, b.plan_commitment);
}

#[test]
fn adding_a_shard_only_moves_buckets_to_the_new_shard() {
    let mut moved = 0usize;
    let mut stayed = 0usize;
    for bucket in 0..512u16 {
        let before = route_bucket("stable-seed", bucket, 2);
        let after = route_bucket("stable-seed", bucket, 3);
        if before == after {
            stayed += 1;
        } else {
            moved += 1;
            assert_eq!(after, 2, "a bucket may only move to the added shard");
        }
    }
    assert!(moved > 0, "fixture must exercise remapping");
    assert!(stayed > 0, "adding one shard must not remap every bucket");
}

#[test]
fn incomplete_and_synthetic_profiles_cannot_be_certified() {
    let make = || {
        base_profile(
            vec![atom("a", "one", false)],
            vec![VirtualBucket { id: 0, weight: 1 }],
            vec![transaction("write", vec![], vec![access("a", 0)])],
        )
    };
    let mut incomplete = make();
    incomplete.completeness.reads_complete = false;
    assert_eq!(
        solve(incomplete).unwrap().certificate.grade,
        CertificateGrade::Incomplete
    );

    let mut synthetic = make();
    synthetic.provenance.kind = ProvenanceKind::Synthetic;
    assert_eq!(
        solve(synthetic).unwrap().certificate.grade,
        CertificateGrade::Exploratory
    );
}

#[test]
fn strict_json_rejects_unknown_fields() {
    let profile = base_profile(
        vec![atom("a", "one", false)],
        vec![VirtualBucket { id: 0, weight: 1 }],
        vec![transaction("write", vec![], vec![access("a", 0)])],
    );
    let mut value = serde_json::to_value(profile).unwrap();
    value
        .as_object_mut()
        .unwrap()
        .insert("mispelledLimit".to_string(), serde_json::json!(7));
    assert!(serde_json::from_value::<WorkloadProfile>(value).is_err());
}
