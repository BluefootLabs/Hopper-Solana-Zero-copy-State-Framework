use core::cmp::Ordering;

use serde::{Deserialize, Serialize};

/// The only profile schema accepted by this release.
pub const PROFILE_SCHEMA_V1: &str = "hopper.topology.profile.v0.1";
/// The plan schema emitted by this release.
pub const PLAN_SCHEMA_V1: &str = "hopper.topology.plan.v0.1";

/// One exact non-negative rational metric.
///
/// Conflict probabilities stay rational so input ordering, platform floating
/// point, and formatting cannot perturb a plan commitment.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Ratio {
    pub numerator: u128,
    pub denominator: u128,
}

impl Ratio {
    pub const ZERO: Self = Self {
        numerator: 0,
        denominator: 1,
    };

    pub fn new(numerator: u128, denominator: u128) -> Self {
        debug_assert!(denominator != 0);
        if numerator == 0 {
            return Self::ZERO;
        }
        let divisor = gcd(numerator, denominator);
        Self {
            numerator: numerator / divisor,
            denominator: denominator / divisor,
        }
    }

    /// Compare two fractions without cross-multiplication overflow.
    pub fn compare(self, other: Self) -> Ordering {
        compare_fractions(
            self.numerator,
            self.denominator,
            other.numerator,
            other.denominator,
        )
    }
}

fn gcd(mut a: u128, mut b: u128) -> u128 {
    while b != 0 {
        let next = a % b;
        a = b;
        b = next;
    }
    a
}

fn compare_fractions(mut an: u128, mut ad: u128, mut bn: u128, mut bd: u128) -> Ordering {
    let mut reverse = false;
    loop {
        let aq = an / ad;
        let bq = bn / bd;
        if aq != bq {
            return if reverse { bq.cmp(&aq) } else { aq.cmp(&bq) };
        }

        let ar = an % ad;
        let br = bn % bd;
        match (ar == 0, br == 0) {
            (true, true) => return Ordering::Equal,
            (true, false) => {
                return if reverse {
                    Ordering::Greater
                } else {
                    Ordering::Less
                }
            }
            (false, true) => {
                return if reverse {
                    Ordering::Less
                } else {
                    Ordering::Greater
                }
            }
            (false, false) => {
                an = ad;
                ad = ar;
                bn = bd;
                bd = br;
                reverse = !reverse;
            }
        }
    }
}

/// Provenance of the correlated transaction classes in a profile.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Provenance {
    pub kind: ProvenanceKind,
    /// Stable dataset, capture, or replay identifier.
    pub source_id: String,
    pub first_slot: Option<u64>,
    pub last_slot: Option<u64>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ProvenanceKind {
    ConfirmedCluster,
    DeterministicReplay,
    HostSimulation,
    Synthetic,
}

/// Coverage facts are explicit rather than inferred from the source label.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Completeness {
    pub transaction_accounts_complete: bool,
    pub reads_complete: bool,
    pub writes_complete: bool,
    pub cpi_frames_complete: bool,
    pub remaining_accounts_resolved: bool,
    pub partial_records: u64,
}

impl Completeness {
    pub fn is_complete(&self) -> bool {
        self.transaction_accounts_complete
            && self.reads_complete
            && self.writes_complete
            && self.cpi_frames_complete
            && self.remaining_accounts_resolved
            && self.partial_records == 0
    }
}

/// A pinned target-cluster and transaction-encoding cost model.
///
/// Loom never assumes that account-lock limits, rent, or address encoding are
/// identical across clusters or feature sets.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ClusterConstraints {
    pub genesis_hash: String,
    pub observed_slot: u64,
    pub feature_set: String,
    pub max_transaction_account_locks: u16,
    pub max_transaction_bytes: u16,
    pub max_account_data_bytes: u64,
    /// Per-account bytes owned by the program before logical atom data.
    pub account_header_bytes: u32,
    /// Runtime/database overhead reported separately from account data.
    pub account_storage_overhead_bytes: u32,
    /// Bytes added by the target rent formula before multiplying by the rate.
    pub rent_exempt_base_bytes: u32,
    /// Effective rent-exempt lamports per charged byte at `observed_slot`.
    pub rent_lamports_per_byte: u64,
    /// Explicit message-model increment for each topology-controlled account.
    pub additional_account_message_bytes: u16,
}

/// User/project feasibility ceilings, kept separate from cluster facts.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectBudgets {
    pub max_physical_accounts: u32,
    pub max_total_rent_lamports: u64,
    pub max_migration_bytes: u64,
}

/// One logical state atom that may be separated vertically.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LogicalAtom {
    pub id: String,
    /// If `shardable`, bytes occupied by this atom in one virtual bucket.
    /// Otherwise, its total bytes in the singleton logical state.
    pub size_bytes: u64,
    /// Physical account holding the atom in the measured baseline.
    pub current_account: String,
    /// Whether the author explicitly permits routing this atom by bucket.
    pub shardable: bool,
}

/// One fixed routing bucket. Keys map to these buckets outside Loom; Loom maps
/// the fixed bucket IDs to physical shards.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VirtualBucket {
    pub id: u16,
    /// Observed/specified routing weight used only for the hot-key floor.
    pub weight: u64,
}

/// An exact logical access. Correlation is preserved by its enclosing
/// [`TransactionClass`].
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LogicalAccess {
    pub atom: String,
    pub bucket: u16,
}

/// A weighted, correlated transaction class.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TransactionClass {
    pub id: String,
    pub weight: u64,
    /// Non-topology account locks, counted for feasibility but not assumed to
    /// have known identities for conflict analysis.
    pub fixed_account_locks: u16,
    /// Message bytes not changed by a topology candidate.
    pub fixed_message_bytes: u16,
    pub reads: Vec<LogicalAccess>,
    pub writes: Vec<LogicalAccess>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SelectionPolicy {
    ThroughputFirst,
    CapitalFirst,
    MinimalMigration,
}

/// Candidate-space controls. Shard counts are explicit so CI evaluates the
/// same finite search after upgrades.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CandidateSpace {
    pub routing_seed: String,
    pub shard_counts: Vec<u16>,
    pub include_vertical: bool,
    pub include_horizontal: bool,
    pub include_hybrid: bool,
}

/// Complete input to the v0.1 solver.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkloadProfile {
    pub schema: String,
    pub program_id: String,
    pub manifest_commitment: String,
    pub provenance: Provenance,
    pub completeness: Completeness,
    pub cluster: ClusterConstraints,
    pub budgets: ProjectBudgets,
    pub policy: SelectionPolicy,
    pub candidate_space: CandidateSpace,
    pub atoms: Vec<LogicalAtom>,
    pub virtual_buckets: Vec<VirtualBucket>,
    pub transactions: Vec<TransactionClass>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CandidateKind {
    Current,
    Vertical,
    Horizontal,
    Hybrid,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlacementGroup {
    pub id: String,
    pub atoms: Vec<String>,
    pub sharded: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BucketAssignment {
    pub bucket: u16,
    pub shard: u16,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PhysicalAccount {
    pub id: String,
    pub group: String,
    pub shard: Option<u16>,
    pub data_bytes: u64,
}

/// Metrics are separate dimensions. No mixed-unit score is emitted.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CandidateMetrics {
    /// Probability that two independently drawn modeled transactions have an
    /// account-level R/W or W/W conflict.
    pub weighted_lock_conflict: Ratio,
    pub max_transaction_account_locks: u32,
    pub max_transaction_bytes: u32,
    pub physical_accounts: u32,
    pub total_account_data_bytes: u64,
    pub total_storage_bytes: u64,
    pub total_rent_lamports: u64,
    /// Conservative upper bound: every logical byte moved for non-current
    /// candidates.
    pub migration_bytes_upper_bound: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CandidatePlan {
    pub id: String,
    pub kind: CandidateKind,
    pub shard_count: Option<u16>,
    pub groups: Vec<PlacementGroup>,
    pub bucket_assignments: Vec<BucketAssignment>,
    pub physical_layout: Vec<PhysicalAccount>,
    pub metrics: CandidateMetrics,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RejectedCandidate {
    pub id: String,
    pub kind: CandidateKind,
    pub shard_count: Option<u16>,
    pub reasons: Vec<String>,
    pub metrics: Option<CandidateMetrics>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CertificateGrade {
    Certified,
    ReplayValidated,
    Observed,
    Exploratory,
    Incomplete,
    Infeasible,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TopologyCertificate {
    pub grade: CertificateGrade,
    pub input_commitment: String,
    pub plan_commitment: String,
    pub downgrade_reasons: Vec<String>,
    pub claims: Vec<String>,
    pub nonclaims: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TopologyPlan {
    pub schema: String,
    pub input_commitment: String,
    pub plan_commitment: String,
    pub selected_policy: SelectionPolicy,
    pub routing_bucket_collision_floor: Ratio,
    pub feasible_candidates: Vec<CandidatePlan>,
    pub pareto_candidate_ids: Vec<String>,
    pub chosen_candidate_id: Option<String>,
    pub rejected_candidates: Vec<RejectedCandidate>,
    pub certificate: TopologyCertificate,
}
