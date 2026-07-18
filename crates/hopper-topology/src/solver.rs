use core::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::commitment::PLAN_DOMAIN;
use crate::{
    sha256_hex, BucketAssignment, CandidateKind, CandidateMetrics, CandidatePlan, CertificateGrade,
    LogicalAccess, LogicalAtom, PhysicalAccount, PlacementGroup, ProvenanceKind, Ratio,
    RejectedCandidate, SelectionPolicy, TopologyCertificate, TopologyPlan, TransactionClass,
    WorkloadProfile, PLAN_SCHEMA_V1, PROFILE_SCHEMA_V1,
};

const ROUTE_DOMAIN: &[u8] = b"hopper.topology.rendezvous.v0.1\0";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TopologyError {
    message: String,
}

impl TopologyError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for TopologyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for TopologyError {}

#[derive(Clone)]
struct CandidateSpec {
    id: String,
    kind: CandidateKind,
    shard_count: Option<u16>,
}

#[derive(Clone)]
struct GroupInternal {
    id: String,
    atoms: Vec<String>,
    sharded: bool,
}

struct CandidateInternal {
    public: CandidatePlan,
    atom_groups: BTreeMap<String, String>,
    sharded_groups: BTreeSet<String>,
    bucket_shards: BTreeMap<u16, u16>,
}

#[derive(Default)]
struct LockSet {
    reads: BTreeSet<String>,
    writes: BTreeSet<String>,
}

enum Evaluation {
    Feasible(CandidatePlan),
    Rejected(RejectedCandidate),
}

/// Map one fixed virtual bucket to a physical shard using rendezvous hashing.
///
/// Scores for existing shard IDs do not change when the last shard is added.
/// Consequently, a bucket either stays put or moves to the new shard; it never
/// moves between two old shards. This limits remapping without a mutable
/// routing heuristic. Bucket weights intentionally do not alter identity.
pub fn route_bucket(routing_seed: &str, bucket: u16, shard_count: u16) -> u16 {
    assert!(shard_count > 0, "shard_count must be non-zero");
    let mut best_shard = 0u16;
    let mut best_score = 0u64;
    for shard in 0..shard_count {
        let mut hasher = Sha256::new();
        hasher.update(ROUTE_DOMAIN);
        hasher.update((routing_seed.len() as u64).to_le_bytes());
        hasher.update(routing_seed.as_bytes());
        hasher.update(bucket.to_le_bytes());
        hasher.update(shard.to_le_bytes());
        let digest = hasher.finalize();
        let score = u64::from_be_bytes(digest[..8].try_into().expect("eight digest bytes"));
        if shard == 0 || score > best_score {
            best_score = score;
            best_shard = shard;
        }
    }
    best_shard
}

/// Validate, normalize, solve, and certify one workload profile.
pub fn solve(profile: WorkloadProfile) -> Result<TopologyPlan, TopologyError> {
    let profile = normalize_and_validate(profile)?;
    let input_bytes = serde_json::to_vec(&profile)
        .map_err(|error| TopologyError::new(format!("serialize normalized profile: {error}")))?;
    let input_commitment = sha256_hex(b"hopper.topology.profile.v0.1\0", &input_bytes);
    let hot_floor = routing_bucket_floor(&profile)?;

    let mut specs = vec![CandidateSpec {
        id: "current".to_string(),
        kind: CandidateKind::Current,
        shard_count: None,
    }];
    if profile.candidate_space.include_vertical {
        specs.push(CandidateSpec {
            id: "vertical".to_string(),
            kind: CandidateKind::Vertical,
            shard_count: None,
        });
    }
    for count in &profile.candidate_space.shard_counts {
        if profile.candidate_space.include_horizontal {
            specs.push(CandidateSpec {
                id: format!("horizontal-{count}"),
                kind: CandidateKind::Horizontal,
                shard_count: Some(*count),
            });
        }
        if profile.candidate_space.include_hybrid {
            specs.push(CandidateSpec {
                id: format!("hybrid-{count}"),
                kind: CandidateKind::Hybrid,
                shard_count: Some(*count),
            });
        }
    }
    specs.sort_by(|a, b| a.id.cmp(&b.id));

    let mut feasible = Vec::new();
    let mut rejected = Vec::new();
    for spec in specs {
        match evaluate_candidate(&profile, spec)? {
            Evaluation::Feasible(candidate) => feasible.push(candidate),
            Evaluation::Rejected(candidate) => rejected.push(candidate),
        }
    }
    feasible.sort_by(|a, b| a.id.cmp(&b.id));
    rejected.sort_by(|a, b| a.id.cmp(&b.id));

    let pareto_candidate_ids = pareto_frontier(&feasible);
    let chosen_candidate_id = choose_candidate(&feasible, &pareto_candidate_ids, profile.policy)
        .map(|candidate| candidate.id.clone());

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct PlanCommitmentView<'a> {
        schema: &'a str,
        input_commitment: &'a str,
        selected_policy: SelectionPolicy,
        routing_bucket_collision_floor: Ratio,
        feasible_candidates: &'a [CandidatePlan],
        pareto_candidate_ids: &'a [String],
        chosen_candidate_id: &'a Option<String>,
        rejected_candidates: &'a [RejectedCandidate],
    }

    let view = PlanCommitmentView {
        schema: PLAN_SCHEMA_V1,
        input_commitment: &input_commitment,
        selected_policy: profile.policy,
        routing_bucket_collision_floor: hot_floor,
        feasible_candidates: &feasible,
        pareto_candidate_ids: &pareto_candidate_ids,
        chosen_candidate_id: &chosen_candidate_id,
        rejected_candidates: &rejected,
    };
    let plan_bytes = serde_json::to_vec(&view)
        .map_err(|error| TopologyError::new(format!("serialize plan commitment: {error}")))?;
    let plan_commitment = sha256_hex(PLAN_DOMAIN, &plan_bytes);
    let certificate = certificate_for(
        &profile,
        &input_commitment,
        &plan_commitment,
        chosen_candidate_id.is_some(),
    );

    Ok(TopologyPlan {
        schema: PLAN_SCHEMA_V1.to_string(),
        input_commitment,
        plan_commitment,
        selected_policy: profile.policy,
        routing_bucket_collision_floor: hot_floor,
        feasible_candidates: feasible,
        pareto_candidate_ids,
        chosen_candidate_id,
        rejected_candidates: rejected,
        certificate,
    })
}

pub(crate) fn normalize_and_validate(
    mut profile: WorkloadProfile,
) -> Result<WorkloadProfile, TopologyError> {
    if profile.schema != PROFILE_SCHEMA_V1 {
        return Err(TopologyError::new(format!(
            "unsupported profile schema `{}` (expected `{PROFILE_SCHEMA_V1}`)",
            profile.schema
        )));
    }
    require_text("programId", &profile.program_id)?;
    require_text("manifestCommitment", &profile.manifest_commitment)?;
    require_text("provenance.sourceId", &profile.provenance.source_id)?;
    require_text("cluster.genesisHash", &profile.cluster.genesis_hash)?;
    require_text("cluster.featureSet", &profile.cluster.feature_set)?;
    require_text(
        "candidateSpace.routingSeed",
        &profile.candidate_space.routing_seed,
    )?;

    if profile.cluster.max_transaction_account_locks == 0
        || profile.cluster.max_transaction_bytes == 0
        || profile.cluster.max_account_data_bytes == 0
        || profile.cluster.additional_account_message_bytes == 0
    {
        return Err(TopologyError::new(
            "cluster limits and additionalAccountMessageBytes must be non-zero",
        ));
    }
    if profile.budgets.max_physical_accounts == 0 {
        return Err(TopologyError::new(
            "budgets.maxPhysicalAccounts must be non-zero",
        ));
    }
    if profile.atoms.is_empty() {
        return Err(TopologyError::new("profile must declare at least one atom"));
    }
    if profile.virtual_buckets.is_empty() {
        return Err(TopologyError::new(
            "profile must declare at least one fixed virtual bucket",
        ));
    }
    if profile.transactions.is_empty() {
        return Err(TopologyError::new(
            "profile must declare at least one correlated transaction class",
        ));
    }

    profile.atoms.sort_by(|a, b| a.id.cmp(&b.id));
    for window in profile.atoms.windows(2) {
        if window[0].id == window[1].id {
            return Err(TopologyError::new(format!(
                "duplicate atom id `{}`",
                window[0].id
            )));
        }
    }
    for atom in &profile.atoms {
        require_text("atom.id", &atom.id)?;
        require_text("atom.currentAccount", &atom.current_account)?;
        if atom.size_bytes == 0 {
            return Err(TopologyError::new(format!(
                "atom `{}` has zero sizeBytes",
                atom.id
            )));
        }
    }

    profile.virtual_buckets.sort_by_key(|bucket| bucket.id);
    for window in profile.virtual_buckets.windows(2) {
        if window[0].id == window[1].id {
            return Err(TopologyError::new(format!(
                "duplicate virtual bucket {}",
                window[0].id
            )));
        }
    }
    for bucket in &profile.virtual_buckets {
        if bucket.weight == 0 {
            return Err(TopologyError::new(format!(
                "virtual bucket {} has zero weight",
                bucket.id
            )));
        }
    }

    profile.candidate_space.shard_counts.sort_unstable();
    profile.candidate_space.shard_counts.dedup();
    for count in &profile.candidate_space.shard_counts {
        if *count < 2 {
            return Err(TopologyError::new(
                "candidate shard counts must be at least two",
            ));
        }
    }
    // A shard count exceeding the fixed virtual-bucket count is NOT malformed
    // input — it is a per-candidate infeasibility (a shard would receive no
    // traffic) that must not fail the whole solve, so `current` and `vertical`
    // placements still resolve. It is rejected with a reason during candidate
    // evaluation instead.
    if (profile.candidate_space.include_horizontal || profile.candidate_space.include_hybrid)
        && profile.candidate_space.shard_counts.is_empty()
    {
        return Err(TopologyError::new(
            "horizontal/hybrid search requires at least one shard count",
        ));
    }

    profile.transactions.sort_by(|a, b| a.id.cmp(&b.id));
    for window in profile.transactions.windows(2) {
        if window[0].id == window[1].id {
            return Err(TopologyError::new(format!(
                "duplicate transaction class id `{}`",
                window[0].id
            )));
        }
    }

    let atom_ids: BTreeSet<&str> = profile.atoms.iter().map(|atom| atom.id.as_str()).collect();
    let bucket_ids: BTreeSet<u16> = profile
        .virtual_buckets
        .iter()
        .map(|bucket| bucket.id)
        .collect();
    for transaction in &mut profile.transactions {
        require_text("transaction.id", &transaction.id)?;
        if transaction.weight == 0 {
            return Err(TopologyError::new(format!(
                "transaction class `{}` has zero weight",
                transaction.id
            )));
        }
        transaction.reads.sort();
        transaction.reads.dedup();
        transaction.writes.sort();
        transaction.writes.dedup();
        let writes: BTreeSet<LogicalAccess> = transaction.writes.iter().cloned().collect();
        transaction.reads.retain(|read| !writes.contains(read));
        if transaction.reads.is_empty() && transaction.writes.is_empty() {
            return Err(TopologyError::new(format!(
                "transaction class `{}` has no logical accesses",
                transaction.id
            )));
        }
        for access in transaction.reads.iter().chain(&transaction.writes) {
            if !atom_ids.contains(access.atom.as_str()) {
                return Err(TopologyError::new(format!(
                    "transaction `{}` references unknown atom `{}`",
                    transaction.id, access.atom
                )));
            }
            if !bucket_ids.contains(&access.bucket) {
                return Err(TopologyError::new(format!(
                    "transaction `{}` references unknown virtual bucket {}",
                    transaction.id, access.bucket
                )));
            }
        }
    }

    checked_total_weight(&profile.transactions)?;
    total_logical_bytes(&profile)?;
    Ok(profile)
}

fn require_text(field: &str, value: &str) -> Result<(), TopologyError> {
    if value.trim().is_empty() {
        return Err(TopologyError::new(format!("{field} must not be empty")));
    }
    Ok(())
}

fn checked_total_weight(transactions: &[TransactionClass]) -> Result<u128, TopologyError> {
    transactions.iter().try_fold(0u128, |total, transaction| {
        total
            .checked_add(transaction.weight as u128)
            .ok_or_else(|| TopologyError::new("transaction weight sum overflow"))
    })
}

fn atom_logical_bytes(profile: &WorkloadProfile, atom: &LogicalAtom) -> Result<u64, TopologyError> {
    if atom.shardable {
        atom.size_bytes
            .checked_mul(profile.virtual_buckets.len() as u64)
            .ok_or_else(|| TopologyError::new(format!("logical size overflow for `{}`", atom.id)))
    } else {
        Ok(atom.size_bytes)
    }
}

fn total_logical_bytes(profile: &WorkloadProfile) -> Result<u64, TopologyError> {
    profile.atoms.iter().try_fold(0u64, |total, atom| {
        total
            .checked_add(atom_logical_bytes(profile, atom)?)
            .ok_or_else(|| TopologyError::new("total logical byte size overflow"))
    })
}

fn routing_bucket_floor(profile: &WorkloadProfile) -> Result<Ratio, TopologyError> {
    let mut total = 0u128;
    let mut squares = 0u128;
    for bucket in &profile.virtual_buckets {
        let weight = bucket.weight as u128;
        total = total
            .checked_add(weight)
            .ok_or_else(|| TopologyError::new("virtual bucket weight sum overflow"))?;
        squares = squares
            .checked_add(weight * weight)
            .ok_or_else(|| TopologyError::new("virtual bucket square sum overflow"))?;
    }
    let denominator = total
        .checked_mul(total)
        .ok_or_else(|| TopologyError::new("virtual bucket denominator overflow"))?;
    Ok(Ratio::new(squares, denominator))
}

fn candidate_groups(
    profile: &WorkloadProfile,
    spec: &CandidateSpec,
) -> Result<Vec<GroupInternal>, Vec<String>> {
    let mut current: BTreeMap<String, Vec<&LogicalAtom>> = BTreeMap::new();
    for atom in &profile.atoms {
        current
            .entry(atom.current_account.clone())
            .or_default()
            .push(atom);
    }

    match spec.kind {
        CandidateKind::Current => Ok(current
            .into_iter()
            .map(|(account, atoms)| GroupInternal {
                id: format!("current:{account}"),
                atoms: atoms.into_iter().map(|atom| atom.id.clone()).collect(),
                sharded: false,
            })
            .collect()),
        CandidateKind::Vertical => Ok(profile
            .atoms
            .iter()
            .map(|atom| GroupInternal {
                id: format!("atom:{}", atom.id),
                atoms: vec![atom.id.clone()],
                sharded: false,
            })
            .collect()),
        CandidateKind::Horizontal => {
            let mut groups = Vec::new();
            let mut reasons = Vec::new();
            let mut sharded = 0usize;
            for (account, atoms) in current {
                let shardable = atoms.iter().filter(|atom| atom.shardable).count();
                if shardable != 0 && shardable != atoms.len() {
                    reasons.push(format!(
                        "current account `{account}` mixes shardable and singleton atoms; horizontal duplication would be unsound"
                    ));
                    continue;
                }
                let is_sharded = shardable == atoms.len();
                if is_sharded {
                    sharded += 1;
                }
                groups.push(GroupInternal {
                    id: format!("current:{account}"),
                    atoms: atoms.into_iter().map(|atom| atom.id.clone()).collect(),
                    sharded: is_sharded,
                });
            }
            if sharded == 0 {
                reasons.push("no current placement group is explicitly shardable".to_string());
            }
            if reasons.is_empty() {
                Ok(groups)
            } else {
                Err(reasons)
            }
        }
        CandidateKind::Hybrid => {
            if !profile.atoms.iter().any(|atom| atom.shardable) {
                return Err(vec!["no logical atom is explicitly shardable".to_string()]);
            }
            Ok(profile
                .atoms
                .iter()
                .map(|atom| GroupInternal {
                    id: format!("atom:{}", atom.id),
                    atoms: vec![atom.id.clone()],
                    sharded: atom.shardable,
                })
                .collect())
        }
    }
}

fn evaluate_candidate(
    profile: &WorkloadProfile,
    spec: CandidateSpec,
) -> Result<Evaluation, TopologyError> {
    let groups = match candidate_groups(profile, &spec) {
        Ok(groups) => groups,
        Err(mut reasons) => {
            reasons.sort();
            reasons.dedup();
            return Ok(Evaluation::Rejected(RejectedCandidate {
                id: spec.id,
                kind: spec.kind,
                shard_count: spec.shard_count,
                reasons,
                metrics: None,
            }));
        }
    };

    // A sharded candidate provisions `shard_count` physical shards. Routing
    // fewer fixed buckets than shards would leave a shard permanently empty,
    // so the candidate is infeasible for this workload — rejected with a
    // reason rather than emitted as a dead-account layout. (Counts below two
    // are malformed input, rejected during normalization.)
    if let Some(count) = spec.shard_count {
        if groups.iter().any(|group| group.sharded)
            && count as usize > profile.virtual_buckets.len()
        {
            return Ok(Evaluation::Rejected(RejectedCandidate {
                id: spec.id,
                kind: spec.kind,
                shard_count: spec.shard_count,
                reasons: vec![format!(
                    "shard count {count} exceeds fixed virtual bucket count {}; a shard would receive no traffic",
                    profile.virtual_buckets.len()
                )],
                metrics: None,
            }));
        }
    }

    let mut atom_groups = BTreeMap::new();
    let mut sharded_groups = BTreeSet::new();
    for group in &groups {
        for atom in &group.atoms {
            atom_groups.insert(atom.clone(), group.id.clone());
        }
        if group.sharded {
            sharded_groups.insert(group.id.clone());
        }
    }

    let mut bucket_shards = BTreeMap::new();
    if let Some(count) = spec.shard_count {
        for bucket in &profile.virtual_buckets {
            bucket_shards.insert(
                bucket.id,
                route_bucket(&profile.candidate_space.routing_seed, bucket.id, count),
            );
        }
    }
    let bucket_assignments: Vec<BucketAssignment> = bucket_shards
        .iter()
        .map(|(bucket, shard)| BucketAssignment {
            bucket: *bucket,
            shard: *shard,
        })
        .collect();

    let atom_by_id: BTreeMap<&str, &LogicalAtom> = profile
        .atoms
        .iter()
        .map(|atom| (atom.id.as_str(), atom))
        .collect();
    let mut physical_layout = Vec::new();
    for group in &groups {
        if group.sharded {
            let count = spec.shard_count.expect("sharded candidates carry a count");
            for shard in 0..count {
                let bucket_count = bucket_shards
                    .values()
                    .filter(|candidate| **candidate == shard)
                    .count() as u64;
                let mut bytes = profile.cluster.account_header_bytes as u64;
                for atom_id in &group.atoms {
                    let atom = atom_by_id[atom_id.as_str()];
                    let atom_bytes = atom
                        .size_bytes
                        .checked_mul(bucket_count)
                        .ok_or_else(|| TopologyError::new("sharded account size overflow"))?;
                    bytes = bytes
                        .checked_add(atom_bytes)
                        .ok_or_else(|| TopologyError::new("sharded account size overflow"))?;
                }
                physical_layout.push(PhysicalAccount {
                    id: format!("{}#shard-{shard}", group.id),
                    group: group.id.clone(),
                    shard: Some(shard),
                    data_bytes: bytes,
                });
            }
        } else {
            let mut bytes = profile.cluster.account_header_bytes as u64;
            for atom_id in &group.atoms {
                let atom = atom_by_id[atom_id.as_str()];
                bytes = bytes
                    .checked_add(atom_logical_bytes(profile, atom)?)
                    .ok_or_else(|| TopologyError::new("account size overflow"))?;
            }
            physical_layout.push(PhysicalAccount {
                id: group.id.clone(),
                group: group.id.clone(),
                shard: None,
                data_bytes: bytes,
            });
        }
    }
    physical_layout.sort_by(|a, b| a.id.cmp(&b.id));

    let public_groups = groups
        .iter()
        .map(|group| PlacementGroup {
            id: group.id.clone(),
            atoms: group.atoms.clone(),
            sharded: group.sharded,
        })
        .collect();
    let mut internal = CandidateInternal {
        public: CandidatePlan {
            id: spec.id.clone(),
            kind: spec.kind,
            shard_count: spec.shard_count,
            groups: public_groups,
            bucket_assignments,
            physical_layout,
            metrics: CandidateMetrics {
                weighted_lock_conflict: Ratio::ZERO,
                max_transaction_account_locks: 0,
                max_transaction_bytes: 0,
                physical_accounts: 0,
                total_account_data_bytes: 0,
                total_storage_bytes: 0,
                total_rent_lamports: 0,
                migration_bytes_upper_bound: 0,
            },
        },
        atom_groups,
        sharded_groups,
        bucket_shards,
    };
    internal.public.metrics = metrics_for(profile, &internal)?;

    let mut reasons = feasibility_reasons(profile, &internal.public.metrics);
    for account in &internal.public.physical_layout {
        if account.data_bytes > profile.cluster.max_account_data_bytes {
            reasons.push(format!(
                "physical account `{}` has {} data bytes, exceeding pinned account limit {}",
                account.id, account.data_bytes, profile.cluster.max_account_data_bytes
            ));
        }
    }
    reasons.sort();
    reasons.dedup();
    if reasons.is_empty() {
        Ok(Evaluation::Feasible(internal.public))
    } else {
        Ok(Evaluation::Rejected(RejectedCandidate {
            id: spec.id,
            kind: spec.kind,
            shard_count: spec.shard_count,
            reasons,
            metrics: Some(internal.public.metrics),
        }))
    }
}

fn physical_account_for(
    candidate: &CandidateInternal,
    access: &LogicalAccess,
) -> Result<String, TopologyError> {
    let group = candidate.atom_groups.get(&access.atom).ok_or_else(|| {
        TopologyError::new(format!("candidate omitted logical atom `{}`", access.atom))
    })?;
    if candidate.sharded_groups.contains(group) {
        let shard = candidate.bucket_shards.get(&access.bucket).ok_or_else(|| {
            TopologyError::new(format!(
                "candidate omitted virtual bucket {}",
                access.bucket
            ))
        })?;
        Ok(format!("{group}#shard-{shard}"))
    } else {
        Ok(group.clone())
    }
}

fn lock_set_for(
    candidate: &CandidateInternal,
    transaction: &TransactionClass,
) -> Result<LockSet, TopologyError> {
    let mut locks = LockSet::default();
    for read in &transaction.reads {
        locks.reads.insert(physical_account_for(candidate, read)?);
    }
    for write in &transaction.writes {
        let account = physical_account_for(candidate, write)?;
        locks.reads.remove(&account);
        locks.writes.insert(account);
    }
    Ok(locks)
}

fn lock_sets_conflict(a: &LockSet, b: &LockSet) -> bool {
    a.writes
        .iter()
        .any(|account| b.writes.contains(account) || b.reads.contains(account))
        || b.writes.iter().any(|account| a.reads.contains(account))
}

fn metrics_for(
    profile: &WorkloadProfile,
    candidate: &CandidateInternal,
) -> Result<CandidateMetrics, TopologyError> {
    let mut lock_sets = Vec::with_capacity(profile.transactions.len());
    let mut max_locks = 0u32;
    let mut max_bytes = 0u32;
    for transaction in &profile.transactions {
        let locks = lock_set_for(candidate, transaction)?;
        let state_accounts = locks.reads.len() + locks.writes.len();
        let account_count = (transaction.fixed_account_locks as u32)
            .checked_add(state_accounts as u32)
            .ok_or_else(|| TopologyError::new("transaction account count overflow"))?;
        let added_bytes = (state_accounts as u32)
            .checked_mul(profile.cluster.additional_account_message_bytes as u32)
            .ok_or_else(|| TopologyError::new("transaction message size overflow"))?;
        let message_bytes = (transaction.fixed_message_bytes as u32)
            .checked_add(added_bytes)
            .ok_or_else(|| TopologyError::new("transaction message size overflow"))?;
        max_locks = max_locks.max(account_count);
        max_bytes = max_bytes.max(message_bytes);
        lock_sets.push(locks);
    }

    let total_weight = checked_total_weight(&profile.transactions)?;
    let denominator = total_weight
        .checked_mul(total_weight)
        .ok_or_else(|| TopologyError::new("conflict denominator overflow"))?;
    let mut conflict_weight = 0u128;
    for (i, left) in profile.transactions.iter().enumerate() {
        for (j, right) in profile.transactions.iter().enumerate() {
            if lock_sets_conflict(&lock_sets[i], &lock_sets[j]) {
                let pair = (left.weight as u128) * (right.weight as u128);
                conflict_weight = conflict_weight
                    .checked_add(pair)
                    .ok_or_else(|| TopologyError::new("conflict weight overflow"))?;
            }
        }
    }

    let mut data_bytes = 0u64;
    let mut storage_bytes = 0u64;
    let mut rent_lamports = 0u64;
    for account in &candidate.public.physical_layout {
        data_bytes = data_bytes
            .checked_add(account.data_bytes)
            .ok_or_else(|| TopologyError::new("total account data overflow"))?;
        storage_bytes = storage_bytes
            .checked_add(account.data_bytes)
            .and_then(|value| {
                value.checked_add(profile.cluster.account_storage_overhead_bytes as u64)
            })
            .ok_or_else(|| TopologyError::new("total storage size overflow"))?;
        let charged_bytes = account
            .data_bytes
            .checked_add(profile.cluster.rent_exempt_base_bytes as u64)
            .ok_or_else(|| TopologyError::new("rent byte size overflow"))?;
        let account_rent = charged_bytes
            .checked_mul(profile.cluster.rent_lamports_per_byte)
            .ok_or_else(|| TopologyError::new("rent calculation overflow"))?;
        rent_lamports = rent_lamports
            .checked_add(account_rent)
            .ok_or_else(|| TopologyError::new("total rent overflow"))?;
    }
    let physical_accounts = u32::try_from(candidate.public.physical_layout.len())
        .map_err(|_| TopologyError::new("physical account count exceeds u32"))?;
    let migration_bytes = if candidate.public.kind == CandidateKind::Current {
        0
    } else {
        total_logical_bytes(profile)?
    };

    Ok(CandidateMetrics {
        weighted_lock_conflict: Ratio::new(conflict_weight, denominator),
        max_transaction_account_locks: max_locks,
        max_transaction_bytes: max_bytes,
        physical_accounts,
        total_account_data_bytes: data_bytes,
        total_storage_bytes: storage_bytes,
        total_rent_lamports: rent_lamports,
        migration_bytes_upper_bound: migration_bytes,
    })
}

fn feasibility_reasons(profile: &WorkloadProfile, metrics: &CandidateMetrics) -> Vec<String> {
    let mut reasons = Vec::new();
    if metrics.max_transaction_account_locks > profile.cluster.max_transaction_account_locks as u32
    {
        reasons.push(format!(
            "max transaction account locks {} exceeds pinned cluster limit {}",
            metrics.max_transaction_account_locks, profile.cluster.max_transaction_account_locks
        ));
    }
    if metrics.max_transaction_bytes > profile.cluster.max_transaction_bytes as u32 {
        reasons.push(format!(
            "max transaction bytes {} exceeds pinned message limit {}",
            metrics.max_transaction_bytes, profile.cluster.max_transaction_bytes
        ));
    }
    if metrics.physical_accounts > profile.budgets.max_physical_accounts {
        reasons.push(format!(
            "physical account count {} exceeds project budget {}",
            metrics.physical_accounts, profile.budgets.max_physical_accounts
        ));
    }
    if metrics.total_rent_lamports > profile.budgets.max_total_rent_lamports {
        reasons.push(format!(
            "rent {} exceeds project budget {} lamports",
            metrics.total_rent_lamports, profile.budgets.max_total_rent_lamports
        ));
    }
    if metrics.migration_bytes_upper_bound > profile.budgets.max_migration_bytes {
        reasons.push(format!(
            "migration upper bound {} exceeds project budget {} bytes",
            metrics.migration_bytes_upper_bound, profile.budgets.max_migration_bytes
        ));
    }
    reasons
}

fn dominates(left: &CandidateMetrics, right: &CandidateMetrics) -> bool {
    let dimensions = [
        left.weighted_lock_conflict
            .compare(right.weighted_lock_conflict),
        left.max_transaction_account_locks
            .cmp(&right.max_transaction_account_locks),
        left.max_transaction_bytes.cmp(&right.max_transaction_bytes),
        left.physical_accounts.cmp(&right.physical_accounts),
        left.total_rent_lamports.cmp(&right.total_rent_lamports),
        left.migration_bytes_upper_bound
            .cmp(&right.migration_bytes_upper_bound),
    ];
    dimensions.iter().all(|order| *order != Ordering::Greater)
        && dimensions.iter().any(|order| *order == Ordering::Less)
}

fn pareto_frontier(candidates: &[CandidatePlan]) -> Vec<String> {
    let mut ids = Vec::new();
    for candidate in candidates {
        let dominated = candidates
            .iter()
            .any(|other| other.id != candidate.id && dominates(&other.metrics, &candidate.metrics));
        if !dominated {
            ids.push(candidate.id.clone());
        }
    }
    ids.sort();
    ids
}

fn choose_candidate<'a>(
    candidates: &'a [CandidatePlan],
    pareto_ids: &[String],
    policy: SelectionPolicy,
) -> Option<&'a CandidatePlan> {
    let pareto: BTreeSet<&str> = pareto_ids.iter().map(String::as_str).collect();
    candidates
        .iter()
        .filter(|candidate| pareto.contains(candidate.id.as_str()))
        .min_by(|a, b| compare_for_policy(a, b, policy))
}

fn compare_for_policy(
    left: &CandidatePlan,
    right: &CandidatePlan,
    policy: SelectionPolicy,
) -> Ordering {
    let conflict = || {
        left.metrics
            .weighted_lock_conflict
            .compare(right.metrics.weighted_lock_conflict)
    };
    let locks = || {
        left.metrics
            .max_transaction_account_locks
            .cmp(&right.metrics.max_transaction_account_locks)
    };
    let bytes = || {
        left.metrics
            .max_transaction_bytes
            .cmp(&right.metrics.max_transaction_bytes)
    };
    let accounts = || {
        left.metrics
            .physical_accounts
            .cmp(&right.metrics.physical_accounts)
    };
    let rent = || {
        left.metrics
            .total_rent_lamports
            .cmp(&right.metrics.total_rent_lamports)
    };
    let migration = || {
        left.metrics
            .migration_bytes_upper_bound
            .cmp(&right.metrics.migration_bytes_upper_bound)
    };
    let orders = match policy {
        SelectionPolicy::ThroughputFirst => [
            conflict(),
            locks(),
            bytes(),
            rent(),
            migration(),
            accounts(),
        ],
        SelectionPolicy::CapitalFirst => [
            rent(),
            accounts(),
            conflict(),
            locks(),
            bytes(),
            migration(),
        ],
        SelectionPolicy::MinimalMigration => [
            migration(),
            conflict(),
            rent(),
            accounts(),
            locks(),
            bytes(),
        ],
    };
    for order in orders {
        if order != Ordering::Equal {
            return order;
        }
    }
    left.id.cmp(&right.id)
}

fn certificate_for(
    profile: &WorkloadProfile,
    input_commitment: &str,
    plan_commitment: &str,
    feasible: bool,
) -> TopologyCertificate {
    let mut downgrade_reasons = Vec::new();
    let grade = if !feasible {
        downgrade_reasons.push("no candidate satisfied every pinned constraint".to_string());
        CertificateGrade::Infeasible
    } else if !profile.completeness.is_complete() {
        if !profile.completeness.transaction_accounts_complete {
            downgrade_reasons.push("transaction account metas are incomplete".to_string());
        }
        if !profile.completeness.reads_complete {
            downgrade_reasons.push("logical read coverage is incomplete".to_string());
        }
        if !profile.completeness.writes_complete {
            downgrade_reasons.push("logical write coverage is incomplete".to_string());
        }
        if !profile.completeness.cpi_frames_complete {
            downgrade_reasons.push("nested CPI frame coverage is incomplete".to_string());
        }
        if !profile.completeness.remaining_accounts_resolved {
            downgrade_reasons.push("remaining-account roles are unresolved".to_string());
        }
        if profile.completeness.partial_records != 0 {
            downgrade_reasons.push(format!(
                "{} partial profile record(s) were reported",
                profile.completeness.partial_records
            ));
        }
        CertificateGrade::Incomplete
    } else {
        match profile.provenance.kind {
            ProvenanceKind::ConfirmedCluster => CertificateGrade::Certified,
            ProvenanceKind::DeterministicReplay => CertificateGrade::ReplayValidated,
            ProvenanceKind::HostSimulation => CertificateGrade::Observed,
            ProvenanceKind::Synthetic => {
                downgrade_reasons
                    .push("synthetic workloads cannot receive Certified grade".to_string());
                CertificateGrade::Exploratory
            }
        }
    };

    TopologyCertificate {
        grade,
        input_commitment: input_commitment.to_string(),
        plan_commitment: plan_commitment.to_string(),
        downgrade_reasons,
        claims: vec![
            "candidate metrics use exact account-level R/W and W/W conflict semantics for the modeled logical atoms".to_string(),
            "the emitted profile and plan commitments are deterministic after semantic normalization".to_string(),
            "feasibility was checked against the pinned limits and explicit project budgets in the input".to_string(),
        ],
        nonclaims: vec![
            "not a prediction of validator throughput, latency, fees, or scheduler ordering".to_string(),
            "not a proof that a generated migration or application-level semantic rewrite is correct".to_string(),
            "does not model conflicts among fixed or foreign accounts whose identities were not supplied as logical atoms".to_string(),
            "the routing-bucket collision floor assumes independently drawn single-bucket traffic and is not a universal lower bound for multi-key transactions".to_string(),
        ],
    }
}

#[cfg(test)]
mod ratio_tests {
    use super::*;

    #[test]
    fn ratio_comparison_does_not_cross_multiply() {
        assert_eq!(Ratio::new(1, 3).compare(Ratio::new(2, 5)), Ordering::Less);
        assert_eq!(Ratio::new(9, 12).compare(Ratio::new(3, 4)), Ordering::Equal);
        assert_eq!(
            Ratio::new(u128::MAX - 1, u128::MAX).compare(Ratio::new(1, 2)),
            Ordering::Greater
        );
    }
}
