//! # Hopper Schema
//!
//! Schema export, ABI fingerprinting, decode tooling, and program management
//! primitives for the Hopper framework.
//!
//! Provides:
//! - Layout manifest generation (JSON-compatible metadata)
//! - Layout fingerprint computation and comparison
//! - Schema diff detection for migration safety
//! - Field-level compatibility checking
//! - Account header decoding and inspection
//! - Segment registry inspection
//! - Manifest registries for multi-layout programs
//! - Program manifests for Hopper Manager (full program schema)
//! - Field-level account decoding for account inspection
//! - Segment migration analysis and migration planning

#![no_std]

pub mod accounts;
pub mod clientgen;
pub mod codama;

use hopper_core::account::HEADER_LEN;
use hopper_core::field_map::FieldInfo;
use hopper_runtime::{AccountView, LayoutContract};
use hopper_runtime::layout::LayoutInfo;
use core::fmt;

// Re-export receipt types for CLI consumers
pub use hopper_core::receipt::{CompatImpact, DecodedReceipt, ReceiptExplain, Phase, ReceiptNarrative, NarrativeRisk};
// Re-export policy types for CLI consumers
pub use hopper_core::policy::PolicyClass;

// ---------------------------------------------------------------------------
// On-chain manifest storage constants
// ---------------------------------------------------------------------------

/// PDA seed for on-chain Hopper manifest accounts.
///
/// Programs store their manifest JSON at:
///   `find_program_address(&[MANIFEST_SEED], &program_id)`
///
/// This deterministic address allows any tool to discover a program's
/// schema knowing only the program ID.
pub const MANIFEST_SEED: &[u8] = b"hopper:manifest";

/// 8-byte magic discriminator at the start of a manifest account.
pub const MANIFEST_MAGIC: [u8; 8] = *b"HOPRMNFT";

/// On-chain manifest account header size (bytes).
///
/// Layout:
///   [0..8]   magic       — `MANIFEST_MAGIC` discriminator
///   [8..12]  version     — u32 LE wire format version (currently 1)
///   [12..16] data_len    — u32 LE byte count of the JSON payload
///   [16..17] compression — u8 (0 = raw JSON, 1 = zlib-deflate)
///   [17..20] reserved    — 3 padding bytes
///   [20..]   payload     — JSON data (raw or compressed)
pub const MANIFEST_HEADER_LEN: usize = 20;

/// Current manifest wire format version.
pub const MANIFEST_VERSION: u32 = 1;

/// Compression tag: no compression (raw JSON).
pub const MANIFEST_COMPRESS_NONE: u8 = 0;
/// Compression tag: zlib-deflate compressed JSON.
pub const MANIFEST_COMPRESS_ZLIB: u8 = 1;

/// Semantic intent of a layout field.
///
/// Enables auto-generated UI, receipt explanations, invariant validation,
/// and client SDKs to understand *what* each field means -- not just its type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum FieldIntent {
    /// Token/SOL balance (lamports or token amount).
    Balance = 0,
    /// Public key that controls this account.
    Authority = 1,
    /// Unix timestamp (seconds since epoch).
    Timestamp = 2,
    /// Monotonic counter (nonce, sequence number).
    Counter = 3,
    /// Array/collection index or offset.
    Index = 4,
    /// Basis-point value (e.g. fee rate, slippage tolerance).
    BasisPoints = 5,
    /// Boolean flag stored as a byte.
    Flag = 6,
    /// Public key reference to another account.
    Address = 7,
    /// Hash or fingerprint (layout_id, merkle root, etc.).
    Hash = 8,
    /// PDA seed component stored on-chain.
    PDASeed = 9,
    /// Layout or schema version number.
    Version = 10,
    /// PDA bump seed.
    Bump = 11,
    /// Cryptographic nonce (distinct from monotonic counter).
    Nonce = 12,
    /// Token supply or mint total.
    Supply = 13,
    /// Rate limit, cap, or ceiling value.
    Limit = 14,
    /// Multisig or governance threshold.
    Threshold = 15,
    /// Owner identity (distinct from authority -- may be non-signer).
    Owner = 16,
    /// Delegated authority (can act on behalf of owner).
    Delegate = 17,
    /// State machine status / lifecycle stage.
    Status = 18,
    /// Application-specific field with no standard semantic.
    Custom = 255,
}

impl FieldIntent {
    /// Human-readable name for display.
    pub fn name(self) -> &'static str {
        match self {
            Self::Balance => "balance",
            Self::Authority => "authority",
            Self::Timestamp => "timestamp",
            Self::Counter => "counter",
            Self::Index => "index",
            Self::BasisPoints => "basis_points",
            Self::Flag => "flag",
            Self::Address => "address",
            Self::Hash => "hash",
            Self::PDASeed => "pda_seed",
            Self::Version => "version",
            Self::Bump => "bump",
            Self::Nonce => "nonce",
            Self::Supply => "supply",
            Self::Limit => "limit",
            Self::Threshold => "threshold",
            Self::Owner => "owner",
            Self::Delegate => "delegate",
            Self::Status => "status",
            Self::Custom => "custom",
        }
    }

    /// Whether this field represents a monetary amount that should be
    /// tracked for conservation invariants.
    pub fn is_monetary(self) -> bool {
        matches!(self, Self::Balance | Self::BasisPoints | Self::Supply)
    }

    /// Whether this field is an identity reference (authority, owner, delegate, or address).
    pub fn is_identity(self) -> bool {
        matches!(self, Self::Authority | Self::Address | Self::Owner | Self::Delegate)
    }

    /// Whether this field is authority-sensitive (mutations require signer verification).
    pub fn is_authority_sensitive(self) -> bool {
        matches!(self, Self::Authority | Self::Owner | Self::Delegate)
    }

    /// Whether this field is immutable after initialization (bump, PDA seed, version seeds).
    pub fn is_init_only(self) -> bool {
        matches!(self, Self::PDASeed | Self::Bump)
    }

    /// Whether this field represents a governance or access-control parameter.
    pub fn is_governance(self) -> bool {
        matches!(self, Self::Threshold | Self::Limit | Self::Status)
    }
}

impl fmt::Display for FieldIntent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

// ---------------------------------------------------------------------------
// Mutation Class -- how a layout behaves under mutation
// ---------------------------------------------------------------------------

/// Classification of how a layout or segment behaves when mutated.
///
/// Enables Hopper to reason about mutation risk, receipt expectations,
/// and policy requirements at the type level rather than guessing from bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum MutationClass {
    /// No writes allowed. Read-only overlay.
    ReadOnly = 0,
    /// New entries appended; existing data never modified.
    AppendOnly = 1,
    /// Existing fields modified in-place; no size change.
    InPlace = 2,
    /// Account may be resized (realloc) during mutation.
    Resizing = 3,
    /// Mutation touches authority, owner, or delegate fields.
    AuthoritySensitive = 4,
    /// Mutation affects balances, supply, or other financial fields.
    Financial = 5,
    /// Mutation changes state machine status or lifecycle stage.
    StateTransition = 6,
}

impl MutationClass {
    /// Human-readable name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::ReadOnly => "read-only",
            Self::AppendOnly => "append-only",
            Self::InPlace => "in-place",
            Self::Resizing => "resizing",
            Self::AuthoritySensitive => "authority-sensitive",
            Self::Financial => "financial",
            Self::StateTransition => "state-transition",
        }
    }

    /// Whether this class involves any writes.
    pub const fn is_mutating(self) -> bool {
        !matches!(self, Self::ReadOnly)
    }

    /// Whether this class requires a state snapshot for receipt generation.
    pub const fn requires_snapshot(self) -> bool {
        !matches!(self, Self::ReadOnly)
    }

    /// Whether this class typically needs authority verification.
    pub const fn requires_authority(self) -> bool {
        matches!(self, Self::AuthoritySensitive | Self::Financial | Self::Resizing | Self::StateTransition)
    }
}

impl fmt::Display for MutationClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

// ---------------------------------------------------------------------------
// Layout Behavior -- operational metadata for a layout
// ---------------------------------------------------------------------------

/// Describes how a layout behaves under mutation, what policy it expects,
/// and what receipt profile it should produce.
///
/// Attach this to a layout manifest to give Hopper's lint engine, Manager,
/// and receipt system enough context to validate mutations semantically.
#[derive(Clone, Copy, Debug)]
pub struct LayoutBehavior {
    /// Whether any instruction mutating this layout requires a signer.
    pub requires_signer: bool,
    /// Whether mutations affect balance/financial fields.
    pub affects_balance: bool,
    /// Whether mutations affect authority/owner/delegate fields.
    pub affects_authority: bool,
    /// Primary mutation class for this layout.
    pub mutation_class: MutationClass,
}

impl LayoutBehavior {
    /// A read-only layout that should never be mutated.
    pub const READ_ONLY: Self = Self {
        requires_signer: false,
        affects_balance: false,
        affects_authority: false,
        mutation_class: MutationClass::ReadOnly,
    };

    /// Default behavior for a standard mutable account.
    pub const STANDARD: Self = Self {
        requires_signer: true,
        affects_balance: false,
        affects_authority: false,
        mutation_class: MutationClass::InPlace,
    };

    /// Behavior for a treasury/vault layout that manages balances.
    pub const FINANCIAL: Self = Self {
        requires_signer: true,
        affects_balance: true,
        affects_authority: false,
        mutation_class: MutationClass::Financial,
    };

    /// Behavior for an append-only journal or audit log.
    pub const APPEND_ONLY: Self = Self {
        requires_signer: true,
        affects_balance: false,
        affects_authority: false,
        mutation_class: MutationClass::AppendOnly,
    };
}

impl fmt::Display for LayoutBehavior {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "mutation={}", self.mutation_class)?;
        if self.requires_signer { write!(f, " signer")?; }
        if self.affects_balance { write!(f, " balance")?; }
        if self.affects_authority { write!(f, " authority")?; }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Layout Stability Grade -- evolution safety scoring
// ---------------------------------------------------------------------------

/// How safe it is to evolve a layout over time.
///
/// Computed from field intents, segment roles, and mutation classes to help
/// builders understand whether their layout design invites future migration
/// pain or stays safely extensible.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum LayoutStabilityGrade {
    /// Layout is safe to extend indefinitely (append-only fields, stable core).
    Stable = 0,
    /// Layout is actively evolving but changes are managed.
    Evolving = 1,
    /// Layout has fields or segments that make migration risky.
    MigrationSensitive = 2,
    /// Layout design makes future evolution dangerous. Refactor recommended.
    UnsafeToEvolve = 3,
}

impl LayoutStabilityGrade {
    /// Human-readable name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Stable => "stable",
            Self::Evolving => "evolving",
            Self::MigrationSensitive => "migration-sensitive",
            Self::UnsafeToEvolve => "unsafe-to-evolve",
        }
    }

    /// Compute the stability grade for a layout manifest.
    ///
    /// Heuristic: counts authority-sensitive fields, financial fields,
    /// and checks whether field ordering invites migration pain.
    pub fn compute(manifest: &LayoutManifest) -> Self {
        let mut authority_count = 0u16;
        let mut financial_count = 0u16;
        let mut init_only_count = 0u16;
        let mut has_custom = false;

        let mut i = 0;
        while i < manifest.field_count {
            let intent = manifest.fields[i].intent;
            if intent.is_authority_sensitive() {
                authority_count += 1;
            }
            if intent.is_monetary() {
                financial_count += 1;
            }
            if intent.is_init_only() {
                init_only_count += 1;
            }
            if matches!(intent, FieldIntent::Custom) {
                has_custom = true;
            }
            i += 1;
        }

        // If the layout has many authority-sensitive or financial fields
        // interleaved with generic fields, it's harder to evolve safely.
        if authority_count > 2 && financial_count > 2 {
            return Self::UnsafeToEvolve;
        }
        if authority_count > 1 || financial_count > 2 {
            return Self::MigrationSensitive;
        }
        if has_custom && manifest.field_count > 8 {
            return Self::Evolving;
        }
        // Init-only fields (PDA seeds, bumps) anchor the layout and
        // make append-only extension safer.
        if init_only_count > 0 {
            return Self::Stable;
        }
        Self::Evolving
    }
}

impl fmt::Display for LayoutStabilityGrade {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// A field descriptor in a layout manifest.
#[derive(Clone, Copy, Debug)]
pub struct FieldDescriptor {
    /// Field name (static str).
    pub name: &'static str,
    /// Canonical type name.
    pub canonical_type: &'static str,
    /// Byte size.
    pub size: u16,
    /// Byte offset from start of struct.
    pub offset: u16,
    /// Semantic intent (what the field *means*, not just its type).
    pub intent: FieldIntent,
}

/// A layout manifest describing an account type.
#[derive(Clone, Copy, Debug)]
pub struct LayoutManifest {
    /// Layout name.
    pub name: &'static str,
    /// Discriminator byte.
    pub disc: u8,
    /// Version byte.
    pub version: u8,
    /// Layout ID (8-byte fingerprint).
    pub layout_id: [u8; 8],
    /// Total byte size including header.
    pub total_size: usize,
    /// Number of fields (not counting header).
    pub field_count: usize,
    /// Field descriptors (static slice). Empty for legacy manifests.
    pub fields: &'static [FieldDescriptor],
}

// -- Layout Fingerprint v2 --

/// Extended layout fingerprint combining wire-level and semantic identity.
///
/// The **wire hash** matches the 8-byte `layout_id` stored in the account header
/// and captures the raw byte layout (field sizes, offsets, types).
///
/// The **semantic hash** additionally folds in field intents, enabling detection
/// of semantic changes that don't alter the wire format (e.g. reinterpreting a
/// `u64` from `Balance` to `Timestamp`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LayoutFingerprint {
    /// Wire-layout hash (matches the header layout_id).
    pub wire_hash: [u8; 8],
    /// Semantic hash incorporating field intents, names, and roles.
    pub semantic_hash: [u8; 8],
}

impl LayoutFingerprint {
    /// Compute a fingerprint from a layout manifest.
    ///
    /// `wire_hash` is taken directly from `manifest.layout_id`.
    /// `semantic_hash` is a deterministic FNV-1a-64 over field names, types,
    /// sizes, offsets, and intents.
    pub const fn from_manifest(manifest: &LayoutManifest) -> Self {
        Self {
            wire_hash: manifest.layout_id,
            semantic_hash: Self::compute_semantic(manifest.fields),
        }
    }

    /// Whether both wire and semantic fingerprints match.
    pub const fn is_identical(&self, other: &Self) -> bool {
        let mut i = 0;
        while i < 8 {
            if self.wire_hash[i] != other.wire_hash[i] { return false; }
            if self.semantic_hash[i] != other.semantic_hash[i] { return false; }
            i += 1;
        }
        true
    }

    /// Whether wire layout matches but semantics differ.
    ///
    /// This detects reinterpretation: same bytes on the wire, different meaning.
    pub const fn wire_matches_but_semantics_differ(&self, other: &Self) -> bool {
        let mut wire_eq = true;
        let mut sem_eq = true;
        let mut i = 0;
        while i < 8 {
            if self.wire_hash[i] != other.wire_hash[i] { wire_eq = false; }
            if self.semantic_hash[i] != other.semantic_hash[i] { sem_eq = false; }
            i += 1;
        }
        wire_eq && !sem_eq
    }

    /// FNV-1a-64 over field descriptors including intents.
    const fn compute_semantic(fields: &[FieldDescriptor]) -> [u8; 8] {
        const FNV_OFFSET: u64 = 0xcbf29ce484222325;
        const FNV_PRIME: u64 = 0x00000100000001B3;

        let mut hash = FNV_OFFSET;
        let mut i = 0;
        while i < fields.len() {
            // Mix in name bytes
            let name = fields[i].name.as_bytes();
            let mut j = 0;
            while j < name.len() {
                hash ^= name[j] as u64;
                hash = hash.wrapping_mul(FNV_PRIME);
                j += 1;
            }
            // Mix in type bytes
            let ty = fields[i].canonical_type.as_bytes();
            j = 0;
            while j < ty.len() {
                hash ^= ty[j] as u64;
                hash = hash.wrapping_mul(FNV_PRIME);
                j += 1;
            }
            // Mix in size, offset
            hash ^= fields[i].size as u64;
            hash = hash.wrapping_mul(FNV_PRIME);
            hash ^= fields[i].offset as u64;
            hash = hash.wrapping_mul(FNV_PRIME);
            // Mix in intent discriminant (the semantic component)
            hash ^= fields[i].intent as u8 as u64;
            hash = hash.wrapping_mul(FNV_PRIME);
            i += 1;
        }
        hash.to_le_bytes()
    }
}

impl fmt::Display for LayoutFingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "wire=")?;
        let mut i = 0;
        while i < 8 {
            let _ = write!(f, "{:02x}", self.wire_hash[i]);
            i += 1;
        }
        write!(f, " sem=")?;
        i = 0;
        while i < 8 {
            let _ = write!(f, "{:02x}", self.semantic_hash[i]);
            i += 1;
        }
        Ok(())
    }
}

// -- Compatibility Checking --

/// Check if two layout manifests are append-compatible.
///
/// Returns `true` if `newer` is a strict superset of `older`:
/// - Same discriminator
/// - `newer.version > older.version`
/// - `newer.total_size >= older.total_size`
/// - Different layout IDs (proving the change)
#[inline]
pub fn is_append_compatible(older: &LayoutManifest, newer: &LayoutManifest) -> bool {
    older.disc == newer.disc
        && newer.version > older.version
        && newer.total_size >= older.total_size
        && older.layout_id != newer.layout_id
}

/// Check if migration is required between two manifests.
///
/// Migration is required when:
/// - Same discriminator but different layout IDs
/// - The newer layout is NOT append-compatible (fields changed, not just appended)
#[inline]
pub fn requires_migration(older: &LayoutManifest, newer: &LayoutManifest) -> bool {
    older.disc == newer.disc && older.layout_id != newer.layout_id
}

/// Check if accounts written by `newer` can still be parsed by code expecting `older`.
///
/// Backward-readable means:
/// - Same discriminator
/// - All fields in `older` exist in `newer` with the same name, type, and size
/// - No fields were reordered (shared prefix is intact)
///
/// This is useful for progressive rollouts: if backward-readable, both V(N) and
/// V(N+1) code can coexist, reading each other's accounts (V(N) ignores extra
/// fields at the end).
///
/// Note: This does NOT mean no migration is needed -- the layout_id will still
/// differ, so strict loaders will reject the data. This checks the *wire-level*
/// compatibility of the prefix.
#[inline]
pub fn is_backward_readable(older: &LayoutManifest, newer: &LayoutManifest) -> bool {
    if older.disc != newer.disc {
        return false;
    }
    // All older fields must exist in newer at the same positions with same types.
    if newer.field_count < older.field_count {
        return false;
    }
    let mut i = 0;
    while i < older.field_count {
        let old_f = &older.fields[i];
        // Newer must have a field at the same index with matching name, type, size.
        if i >= newer.field_count {
            return false;
        }
        let new_f = &newer.fields[i];
        if !const_str_eq(old_f.name, new_f.name)
            || !const_str_eq(old_f.canonical_type, new_f.canonical_type)
            || old_f.size != new_f.size
        {
            return false;
        }
        i += 1;
    }
    // Newer may have extra fields at the end -- that's fine for backward reading.
    true
}

// -- Compatibility Verdict --

/// Unified compatibility verdict between two layout versions.
///
/// Replaces ad-hoc boolean checks with a single, ranked classification.
/// The ordering is from least disruptive to most disruptive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompatibilityVerdict {
    /// Layouts are byte-identical (same layout_id).
    Identical,
    /// Same wire layout (field count, sizes, offsets, types all match)
    /// but `layout_id` differs — semantic metadata changed (e.g. field
    /// intent, layout name). Safe to read; no migration needed.
    WireCompatible,
    /// Same discriminator, old field prefix intact in new layout.
    /// Old readers can still parse new accounts (they ignore the tail).
    /// Covers both strict append (new fields only at the end) and
    /// prefix-preserving changes. No forced migration required.
    AppendSafe,
    /// Breaking change: field types changed, fields removed, or prefix
    /// altered. Full migration required before deploying new code.
    MigrationRequired,
    /// Different discriminators — these are fundamentally different types.
    Incompatible,
}

impl CompatibilityVerdict {
    /// Compute the verdict for a version transition.
    #[inline]
    pub fn between(older: &LayoutManifest, newer: &LayoutManifest) -> Self {
        if older.layout_id == newer.layout_id {
            return Self::Identical;
        }
        if older.disc != newer.disc {
            return Self::Incompatible;
        }
        let backward = is_backward_readable(older, newer);
        // Wire-compatible: identical wire layout but layout_id differs
        // (only semantic metadata changed, e.g. field intent).
        if backward
            && older.field_count == newer.field_count
            && older.total_size == newer.total_size
        {
            return Self::WireCompatible;
        }
        if backward {
            Self::AppendSafe
        } else {
            Self::MigrationRequired
        }
    }

    /// Human-readable name.
    #[inline]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Identical => "identical",
            Self::WireCompatible => "wire-compatible",
            Self::AppendSafe => "append-safe",
            Self::MigrationRequired => "migration-required",
            Self::Incompatible => "incompatible",
        }
    }

    /// Whether the transition is safe without any migration.
    #[inline]
    pub const fn is_safe(self) -> bool {
        matches!(self, Self::Identical | Self::WireCompatible | Self::AppendSafe)
    }

    /// Whether old readers can still parse accounts written by the new layout.
    #[inline]
    pub const fn is_backward_readable(self) -> bool {
        matches!(self, Self::Identical | Self::WireCompatible | Self::AppendSafe)
    }

    /// Whether a migration instruction is required.
    #[inline]
    pub const fn requires_migration(self) -> bool {
        matches!(self, Self::MigrationRequired)
    }

    /// Refine a verdict using segment-role information.
    ///
    /// The base `between()` is field-level only. When a segmented account
    /// has role metadata, this method can soften or escalate:
    ///
    /// * A `MigrationRequired` verdict is softened to `AppendSafe` when
    ///   **all** changed segments are clearable or rebuildable (Cache,
    ///   Index, Journal). Core / Audit / Extension changes stay breaking.
    ///
    /// * An `AppendSafe` verdict is escalated to `MigrationRequired` when
    ///   **any** modified segment is immutable-after-init (Audit).
    pub fn refine_with_roles<const N: usize>(
        self,
        report: &SegmentMigrationReport<N>,
    ) -> Self {
        match self {
            Self::MigrationRequired => {
                // If every segment that must change is clearable or
                // rebuildable, the migration is effectively append-safe.
                let mut i = 0;
                let mut all_soft = true;
                while i < report.count {
                    let adv = &report.advice[i];
                    if adv.must_preserve && !adv.clearable && !adv.rebuildable {
                        // At least one hard segment -- stay breaking.
                        all_soft = false;
                        break;
                    }
                    i += 1;
                }
                if all_soft && report.count > 0 { Self::AppendSafe } else { self }
            }
            Self::AppendSafe => {
                // Escalate if any immutable (Audit) segment was touched.
                let mut i = 0;
                while i < report.count {
                    if report.advice[i].immutable {
                        return Self::MigrationRequired;
                    }
                    i += 1;
                }
                self
            }
            _ => self,
        }
    }
}

impl fmt::Display for CompatibilityVerdict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

// ---------------------------------------------------------------------------
// Compatibility Explain -- human-readable verdict context
// ---------------------------------------------------------------------------

/// A structured, human-readable explanation of a compatibility verdict.
///
/// Goes beyond the raw verdict to tell operators *why* a transition is
/// safe or dangerous, what segments are involved, and what action is needed.
pub struct CompatibilityExplain {
    /// The computed verdict.
    pub verdict: CompatibilityVerdict,
    /// Fields that were added in the newer layout.
    pub added_fields: [&'static str; 16],
    /// Number of valid entries in `added_fields`.
    pub added_count: u8,
    /// Fields that were removed in the newer layout (breaking).
    pub removed_fields: [&'static str; 16],
    /// Number of valid entries in `removed_fields`.
    pub removed_count: u8,
    /// Fields that were changed (type or size mismatch).
    pub changed_fields: [&'static str; 16],
    /// Number of valid entries in `changed_fields`.
    pub changed_count: u8,
    /// Whether the semantic hash changed (meaning shifted even if wire is the same).
    pub semantic_drift: bool,
    /// One-line human-readable summary.
    pub summary: &'static str,
}

impl CompatibilityExplain {
    /// Generate a full explanation from two layout manifests.
    pub fn between(older: &LayoutManifest, newer: &LayoutManifest) -> Self {
        let verdict = CompatibilityVerdict::between(older, newer);

        let mut added = [""; 16];
        let mut added_n = 0u8;
        let mut removed = [""; 16];
        let mut removed_n = 0u8;
        let mut changed = [""; 16];
        let mut changed_n = 0u8;

        // Directly iterate manifest fields so we keep the 'static lifetime.
        let shared = if older.field_count < newer.field_count {
            older.field_count
        } else {
            newer.field_count
        };

        let mut i = 0;
        while i < shared {
            let old_f = &older.fields[i];
            let new_f = &newer.fields[i];
            let name_eq = const_str_eq(old_f.name, new_f.name);
            let type_eq = const_str_eq(old_f.canonical_type, new_f.canonical_type);
            let size_eq = old_f.size == new_f.size;
            if !(name_eq && type_eq && size_eq) {
                if (changed_n as usize) < 16 {
                    changed[changed_n as usize] = old_f.name;
                    changed_n += 1;
                }
            }
            i += 1;
        }
        // Fields only in newer (added).
        while i < newer.field_count {
            if (added_n as usize) < 16 {
                added[added_n as usize] = newer.fields[i].name;
                added_n += 1;
            }
            i += 1;
        }
        // Fields only in older (removed).
        let mut j = shared;
        while j < older.field_count {
            if (removed_n as usize) < 16 {
                removed[removed_n as usize] = older.fields[j].name;
                removed_n += 1;
            }
            j += 1;
        }

        let fp_old = LayoutFingerprint::from_manifest(older);
        let fp_new = LayoutFingerprint::from_manifest(newer);
        let semantic_drift = fp_old.wire_matches_but_semantics_differ(&fp_new);

        let summary = match verdict {
            CompatibilityVerdict::Identical => "Layouts are byte-identical. No action needed.",
            CompatibilityVerdict::WireCompatible => {
                if semantic_drift {
                    "Wire layout matches but field semantics changed. Review field intents."
                } else {
                    "Wire layout matches with metadata-only changes. Safe to deploy."
                }
            }
            CompatibilityVerdict::AppendSafe => "New fields appended at the end. Old readers still work.",
            CompatibilityVerdict::MigrationRequired => "Breaking field changes. Migration instruction required before deploy.",
            CompatibilityVerdict::Incompatible => "Different discriminators. These are unrelated account types.",
        };

        Self {
            verdict,
            added_fields: added,
            added_count: added_n,
            removed_fields: removed,
            removed_count: removed_n,
            changed_fields: changed,
            changed_count: changed_n,
            semantic_drift,
            summary,
        }
    }
}

impl fmt::Display for CompatibilityExplain {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Verdict: {} ({})", self.verdict.name(), self.summary)?;
        if self.added_count > 0 {
            write!(f, "  Added:")?;
            let mut i = 0;
            while i < self.added_count as usize {
                write!(f, " {}", self.added_fields[i])?;
                i += 1;
            }
            writeln!(f)?;
        }
        if self.removed_count > 0 {
            write!(f, "  Removed:")?;
            let mut i = 0;
            while i < self.removed_count as usize {
                write!(f, " {}", self.removed_fields[i])?;
                i += 1;
            }
            writeln!(f)?;
        }
        if self.changed_count > 0 {
            write!(f, "  Changed:")?;
            let mut i = 0;
            while i < self.changed_count as usize {
                write!(f, " {}", self.changed_fields[i])?;
                i += 1;
            }
            writeln!(f)?;
        }
        if self.semantic_drift {
            writeln!(f, "  Warning: semantic drift detected (wire matches but meaning changed)")?;
        }
        Ok(())
    }
}

/// Field-level compatibility result.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FieldCompat {
    /// Field exists in both versions with identical type and size.
    Identical,
    /// Field exists in both but type or size changed (breaking).
    Changed,
    /// Field was added in the newer version (append-safe).
    Added,
    /// Field was removed in the newer version (breaking).
    Removed,
}

/// Compare two manifests field-by-field.
///
/// Returns up to `N` field comparison results. Each entry is
/// (field_name, FieldCompat). Checks that shared prefix fields are
/// identical and classifies remaining fields as Added or Removed.
#[inline]
pub fn compare_fields<'a, const N: usize>(
    older: &'a LayoutManifest,
    newer: &'a LayoutManifest,
) -> FieldCompatReport<'a, N> {
    let mut report = FieldCompatReport {
        entries: [FieldCompatEntry { name: "", status: FieldCompat::Identical }; N],
        count: 0,
        is_append_safe: true,
    };

    // Check shared prefix
    let shared = if older.field_count < newer.field_count {
        older.field_count
    } else {
        newer.field_count
    };

    let mut i = 0;
    while i < shared && report.count < N {
        let old_f = &older.fields[i];
        let new_f = &newer.fields[i];

        let name_eq = const_str_eq(old_f.name, new_f.name);
        let type_eq = const_str_eq(old_f.canonical_type, new_f.canonical_type);
        let size_eq = old_f.size == new_f.size;

        let status = if name_eq && type_eq && size_eq {
            FieldCompat::Identical
        } else {
            report.is_append_safe = false;
            FieldCompat::Changed
        };

        report.entries[report.count] = FieldCompatEntry {
            name: old_f.name,
            status,
        };
        report.count += 1;
        i += 1;
    }

    // Fields only in newer (added)
    while i < newer.field_count && report.count < N {
        report.entries[report.count] = FieldCompatEntry {
            name: newer.fields[i].name,
            status: FieldCompat::Added,
        };
        report.count += 1;
        i += 1;
    }

    // Fields only in older (removed -- breaking)
    let mut j = shared;
    while j < older.field_count && report.count < N {
        report.entries[report.count] = FieldCompatEntry {
            name: older.fields[j].name,
            status: FieldCompat::Removed,
        };
        report.count += 1;
        report.is_append_safe = false;
        j += 1;
    }

    report
}

/// A single field compatibility entry.
#[derive(Clone, Copy)]
pub struct FieldCompatEntry<'a> {
    pub name: &'a str,
    pub status: FieldCompat,
}

/// Result of a field-level compatibility comparison.
pub struct FieldCompatReport<'a, const N: usize> {
    pub entries: [FieldCompatEntry<'a>; N],
    pub count: usize,
    /// True if all changes are append-only (no mutations or removals).
    pub is_append_safe: bool,
}

impl<'a, const N: usize> FieldCompatReport<'a, N> {
    /// Number of field comparison entries.
    #[inline(always)]
    pub fn len(&self) -> usize {
        self.count
    }

    /// Whether the report has no entries.
    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Get entry by index.
    #[inline(always)]
    pub fn get(&self, i: usize) -> Option<&FieldCompatEntry<'a>> {
        if i < self.count { Some(&self.entries[i]) } else { None }
    }

    /// Whether the schema change is append-safe (no breaking changes).
    #[inline(always)]
    pub fn is_append_safe(&self) -> bool {
        self.is_append_safe
    }

    /// Count fields with a specific status.
    #[inline]
    pub fn count_status(&self, status: FieldCompat) -> usize {
        let mut n = 0;
        let mut i = 0;
        while i < self.count {
            if self.entries[i].status == status {
                n += 1;
            }
            i += 1;
        }
        n
    }
}

// -- Account Header Decoder --

/// Decoded account header for inspection/tooling.
#[derive(Clone, Copy)]
pub struct DecodedHeader {
    pub disc: u8,
    pub version: u8,
    pub flags: u16,
    pub layout_id: [u8; 8],
    pub reserved: [u8; 4],
}

/// Decode an account header from raw bytes.
///
/// Works on any data that starts with a 16-byte Hopper header.
/// Does not validate -- just reads the bytes.
#[inline]
pub fn decode_header(data: &[u8]) -> Option<DecodedHeader> {
    if data.len() < HEADER_LEN {
        return None;
    }
    Some(DecodedHeader {
        disc: data[0],
        version: data[1],
        flags: u16::from_le_bytes([data[2], data[3]]),
        layout_id: [data[4], data[5], data[6], data[7], data[8], data[9], data[10], data[11]],
        reserved: [data[12], data[13], data[14], data[15]],
    })
}

/// Try to identify which manifest matches an account's header.
///
/// Scans a list of manifests for matching disc + layout_id.
/// Returns the index and manifest if found.
#[inline]
pub fn identify_account<'a>(
    data: &[u8],
    manifests: &'a [LayoutManifest],
) -> Option<(usize, &'a LayoutManifest)> {
    let header = decode_header(data)?;
    let mut i = 0;
    while i < manifests.len() {
        let m = &manifests[i];
        if m.disc == header.disc && m.layout_id == header.layout_id {
            return Some((i, m));
        }
        i += 1;
    }
    None
}

// -- Segment Inspector --

/// Decoded segment entry for inspection.
#[derive(Clone, Copy)]
pub struct DecodedSegment {
    pub id: [u8; 4],
    pub offset: u32,
    pub size: u32,
    pub flags: u16,
    pub version: u8,
}

/// Decode segment entries from a segmented account.
///
/// Returns up to `N` segments. Works on raw account data that
/// starts with a 16-byte header followed by the segment registry.
#[inline]
pub fn decode_segments<const N: usize>(data: &[u8]) -> Option<(usize, [DecodedSegment; N])> {
    let registry_start = HEADER_LEN;
    if data.len() < registry_start + 4 {
        return None;
    }

    let count = u16::from_le_bytes([data[registry_start], data[registry_start + 1]]) as usize;
    if count > N {
        return None;
    }

    let entries_start = registry_start + 4;
    let mut segments = [DecodedSegment {
        id: [0; 4], offset: 0, size: 0, flags: 0, version: 0,
    }; N];

    let mut i = 0;
    while i < count {
        let off = entries_start + i * 16;
        if off + 16 > data.len() {
            return None;
        }
        segments[i] = DecodedSegment {
            id: [data[off], data[off + 1], data[off + 2], data[off + 3]],
            offset: u32::from_le_bytes([data[off + 4], data[off + 5], data[off + 6], data[off + 7]]),
            size: u32::from_le_bytes([data[off + 8], data[off + 9], data[off + 10], data[off + 11]]),
            flags: u16::from_le_bytes([data[off + 12], data[off + 13]]),
            version: data[off + 14],
        };
        i += 1;
    }

    Some((count, segments))
}

// -- Manifest Registry --

/// A static registry of all layout manifests for a program.
///
/// Pass this to `identify_account` or CLI tooling to decode
/// arbitrary accounts from a program.
///
/// ```ignore
/// const MANIFESTS: ManifestRegistry<3> = ManifestRegistry::new(&[
///     VAULT_MANIFEST,
///     POOL_MANIFEST,
///     POSITION_MANIFEST,
/// ]);
///
/// if let Some((idx, manifest)) = MANIFESTS.identify(data) {
///     // Found matching layout
/// }
/// ```
pub struct ManifestRegistry<const N: usize> {
    manifests: [Option<LayoutManifest>; N],
    count: usize,
}

impl<const N: usize> ManifestRegistry<N> {
    /// Create an empty registry.
    #[inline(always)]
    pub const fn empty() -> Self {
        Self {
            manifests: [None; N],
            count: 0,
        }
    }

    /// Create a registry from a slice of manifests.
    #[inline]
    pub const fn from_slice(manifests: &[LayoutManifest]) -> Self {
        let mut reg = Self::empty();
        let mut i = 0;
        while i < manifests.len() && i < N {
            reg.manifests[i] = Some(manifests[i]);
            reg.count += 1;
            i += 1;
        }
        reg
    }

    /// Number of registered manifests.
    #[inline(always)]
    pub const fn len(&self) -> usize {
        self.count
    }

    /// Whether the registry has no manifests.
    #[inline(always)]
    pub const fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Try to identify an account from header data.
    #[inline]
    pub fn identify(&self, data: &[u8]) -> Option<(usize, &LayoutManifest)> {
        let header = decode_header(data)?;
        let mut i = 0;
        while i < self.count {
            if let Some(m) = &self.manifests[i] {
                if m.disc == header.disc && m.layout_id == header.layout_id {
                    return Some((i, m));
                }
            }
            i += 1;
        }
        None
    }

    /// Get a manifest by index.
    #[inline]
    pub fn get(&self, index: usize) -> Option<&LayoutManifest> {
        if index < self.count {
            self.manifests[index].as_ref()
        } else {
            None
        }
    }

    /// Find a manifest by discriminator.
    #[inline]
    pub fn find_by_disc(&self, disc: u8) -> Option<&LayoutManifest> {
        let mut i = 0;
        while i < self.count {
            if let Some(m) = &self.manifests[i] {
                if m.disc == disc {
                    return Some(m);
                }
            }
            i += 1;
        }
        None
    }

    /// Find a manifest by layout_id.
    #[inline]
    pub fn find_by_layout_id(&self, layout_id: &[u8; 8]) -> Option<&LayoutManifest> {
        let mut i = 0;
        while i < self.count {
            if let Some(m) = &self.manifests[i] {
                if &m.layout_id == layout_id {
                    return Some(m);
                }
            }
            i += 1;
        }
        None
    }
}

// -- Helpers --

/// String equality check without std.
#[inline]
fn const_str_eq(a: &str, b: &str) -> bool {
    let a = a.as_bytes();
    let b = b.as_bytes();
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

// -- Migration Planner --

/// Migration policy for a version transition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MigrationPolicy {
    /// No changes -- manifests are identical.
    NoOp,
    /// Append-only: new fields added at end. Existing data valid as-is.
    /// Just update header (version + layout_id). No data movement needed.
    AppendOnly,
    /// Migration required: field types/sizes changed, or fields removed.
    /// Must allocate new account, copy compatible prefix, zero-init new region.
    RequiresMigration,
    /// Incompatible: different discriminators or fundamental layout mismatch.
    Incompatible,
}

/// A step in the migration plan.
#[derive(Clone, Copy)]
pub struct MigrationStep<'a> {
    /// Step type.
    pub action: MigrationAction,
    /// Target field name (if field-specific).
    pub field: &'a str,
    /// Byte offset for this step.
    pub offset: u16,
    /// Byte count for this step.
    pub size: u16,
}

/// Migration action type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MigrationAction {
    /// Copy bytes from old account at (offset, size) to new account.
    CopyPrefix,
    /// Zero-initialize new region at (offset, size).
    ZeroInit,
    /// Update the account header (version + layout_id).
    UpdateHeader,
    /// Realloc the account to new size (grows in place if possible).
    Realloc,
}

/// A generated migration plan between two layout versions.
///
/// Contains ordered steps to transform V(N) account data to V(N+1).
/// All steps are stack-allocated to stay `no_std`/`no_alloc`.
pub struct MigrationPlan<'a, const N: usize> {
    pub policy: MigrationPolicy,
    pub steps: [MigrationStep<'a>; N],
    pub step_count: usize,
    /// Old total size.
    pub old_size: usize,
    /// New total size.
    pub new_size: usize,
    /// How many bytes must be copied from old to new.
    pub copy_bytes: usize,
    /// How many bytes are newly zero-initialized.
    pub zero_bytes: usize,
    /// Whether V(N) code can still parse V(N+1) accounts (prefix-compatible).
    pub backward_readable: bool,
}

impl<'a, const N: usize> MigrationPlan<'a, N> {
    /// Generate a migration plan from two manifests.
    ///
    /// Analyzes the field-level diff and produces an ordered list of
    /// concrete steps (copy prefix, zero-init new fields, update header).
    pub fn generate(
        older: &'a LayoutManifest,
        newer: &'a LayoutManifest,
    ) -> Self {
        let mut plan = Self {
            policy: MigrationPolicy::NoOp,
            steps: [MigrationStep {
                action: MigrationAction::CopyPrefix,
                field: "",
                offset: 0,
                size: 0,
            }; N],
            step_count: 0,
            old_size: older.total_size,
            new_size: newer.total_size,
            copy_bytes: 0,
            zero_bytes: 0,
            backward_readable: is_backward_readable(older, newer),
        };

        // Same layout -- no-op
        if older.layout_id == newer.layout_id {
            plan.policy = MigrationPolicy::NoOp;
            return plan;
        }

        // Different discriminators -- incompatible
        if older.disc != newer.disc {
            plan.policy = MigrationPolicy::Incompatible;
            return plan;
        }

        // Field-level analysis
        let report = compare_fields::<32>(older, newer);

        if !report.is_append_safe {
            plan.policy = MigrationPolicy::RequiresMigration;
        } else {
            plan.policy = MigrationPolicy::AppendOnly;
        }

        // Step 1: Copy the compatible prefix (all Identical fields)
        let mut compatible_end: u16 = HEADER_LEN as u16;
        let mut i = 0;
        while i < report.count {
            if report.entries[i].status == FieldCompat::Identical {
                // Find matching field in older to get offset+size
                let mut j = 0;
                while j < older.field_count {
                    if const_str_eq(older.fields[j].name, report.entries[i].name) {
                        let field_end = older.fields[j].offset + older.fields[j].size;
                        if field_end > compatible_end {
                            compatible_end = field_end;
                        }
                        break;
                    }
                    j += 1;
                }
            }
            i += 1;
        }

        if compatible_end > HEADER_LEN as u16 && plan.step_count < N {
            let copy_size = compatible_end - HEADER_LEN as u16;
            plan.steps[plan.step_count] = MigrationStep {
                action: MigrationAction::CopyPrefix,
                field: "",
                offset: HEADER_LEN as u16,
                size: copy_size,
            };
            plan.copy_bytes = copy_size as usize;
            plan.step_count += 1;
        }

        // Step 2: Realloc if size changed
        if newer.total_size != older.total_size && plan.step_count < N {
            plan.steps[plan.step_count] = MigrationStep {
                action: MigrationAction::Realloc,
                field: "",
                offset: 0,
                size: newer.total_size as u16,
            };
            plan.step_count += 1;
        }

        // Step 3: Zero-init added fields
        i = 0;
        while i < report.count {
            if report.entries[i].status == FieldCompat::Added && plan.step_count < N {
                // Find in newer manifest
                let mut j = 0;
                while j < newer.field_count {
                    if const_str_eq(newer.fields[j].name, report.entries[i].name) {
                        plan.steps[plan.step_count] = MigrationStep {
                            action: MigrationAction::ZeroInit,
                            field: report.entries[i].name,
                            offset: newer.fields[j].offset,
                            size: newer.fields[j].size,
                        };
                        plan.zero_bytes += newer.fields[j].size as usize;
                        plan.step_count += 1;
                        break;
                    }
                    j += 1;
                }
            }
            i += 1;
        }

        // Step 4: Update header
        if plan.step_count < N {
            plan.steps[plan.step_count] = MigrationStep {
                action: MigrationAction::UpdateHeader,
                field: "",
                offset: 0,
                size: HEADER_LEN as u16,
            };
            plan.step_count += 1;
        }

        plan
    }

    /// Number of steps in the plan.
    #[inline(always)]
    pub fn len(&self) -> usize {
        self.step_count
    }

    /// Whether the plan has no steps.
    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.step_count == 0
    }

    /// Whether this plan requires data movement (non-trivial migration).
    #[inline(always)]
    pub fn requires_data_copy(&self) -> bool {
        self.policy == MigrationPolicy::RequiresMigration
    }

    /// Get step by index.
    #[inline(always)]
    pub fn step(&self, i: usize) -> Option<&MigrationStep<'a>> {
        if i < self.step_count { Some(&self.steps[i]) } else { None }
    }

    /// Iterator-style: iterate steps with index.
    #[inline]
    pub fn for_each_step<F: FnMut(usize, &MigrationStep<'a>)>(&self, mut f: F) {
        let mut i = 0;
        while i < self.step_count {
            f(i, &self.steps[i]);
            i += 1;
        }
    }
}

// -- Segment-Role-Aware Migration Advice --

/// Segment role classification for migration (mirrors hopper-core SegmentRole).
///
/// This is a schema-level copy so hopper-schema can reason about roles without
/// depending on internal details of hopper-core's segment module.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum SegmentRoleHint {
    Core = 0,
    Extension = 1,
    Journal = 2,
    Index = 3,
    Cache = 4,
    Audit = 5,
    Shard = 6,
    Unclassified = 7,
}

impl SegmentRoleHint {
    /// Decode role from the upper 4 bits of a segment flags field.
    #[inline(always)]
    pub fn from_flags(flags: u16) -> Self {
        match (flags >> 12) & 0x0F {
            0 => Self::Core,
            1 => Self::Extension,
            2 => Self::Journal,
            3 => Self::Index,
            4 => Self::Cache,
            5 => Self::Audit,
            6 => Self::Shard,
            _ => Self::Unclassified,
        }
    }

    /// Human-readable name.
    #[inline(always)]
    pub fn name(self) -> &'static str {
        match self {
            Self::Core => "Core",
            Self::Extension => "Extension",
            Self::Journal => "Journal",
            Self::Index => "Index",
            Self::Cache => "Cache",
            Self::Audit => "Audit",
            Self::Shard => "Shard",
            Self::Unclassified => "Unclassified",
        }
    }

    /// Whether data in this segment must survive migration unchanged.
    #[inline(always)]
    pub fn must_preserve(self) -> bool {
        matches!(self, Self::Core | Self::Extension | Self::Audit | Self::Shard)
    }

    /// Whether the segment can be zeroed and rebuilt from other on-chain state.
    #[inline(always)]
    pub fn is_rebuildable(self) -> bool {
        matches!(self, Self::Index | Self::Cache)
    }

    /// Whether the segment can be cleared during migration.
    #[inline(always)]
    pub fn is_clearable(self) -> bool {
        matches!(self, Self::Journal | Self::Index | Self::Cache)
    }

    /// Whether the segment is append-only (no in-place mutations).
    #[inline(always)]
    pub fn is_append_only(self) -> bool {
        matches!(self, Self::Journal | Self::Audit)
    }

    /// Whether the segment is immutable after initialization (Audit logs).
    #[inline(always)]
    pub fn is_immutable(self) -> bool {
        matches!(self, Self::Audit)
    }

    /// Whether this segment's data must be copied during migration.
    ///
    /// Core and Audit segments contain irreplaceable state that cannot
    /// be rebuilt or cleared — their bytes must survive migration intact.
    #[inline(always)]
    pub fn requires_migration_copy(self) -> bool {
        matches!(self, Self::Core | Self::Audit)
    }

    /// Whether this segment can be safely dropped (zeroed) without data loss.
    ///
    /// Cache segments hold derived/computed values that can be rebuilt
    /// from other on-chain state. Dropping them is always safe.
    #[inline(always)]
    pub fn is_safe_to_drop(self) -> bool {
        matches!(self, Self::Cache)
    }
}

/// Migration advice for a single segment.
#[derive(Clone, Copy)]
pub struct SegmentAdvice {
    /// Segment ID bytes.
    pub id: [u8; 4],
    /// Byte size.
    pub size: u32,
    /// Decoded role.
    pub role: SegmentRoleHint,
    /// Must be preserved across migration (data cannot be lost).
    pub must_preserve: bool,
    /// Can be cleared and rebuilt from other data.
    pub clearable: bool,
    /// Can be rebuilt from on-chain state (index, cache).
    pub rebuildable: bool,
    /// Append-only: only new entries allowed, no mutations.
    pub append_only: bool,
    /// Immutable after init (Audit segments).
    pub immutable: bool,
}

/// Segment-level migration report for a segmented account.
///
/// Analyzes each segment's role and produces per-segment migration advice.
/// This lets the migration planner tell you which segments are safe to clear,
/// which must be preserved, and which can be rebuilt from other data.
pub struct SegmentMigrationReport<const N: usize> {
    pub advice: [SegmentAdvice; N],
    pub count: usize,
    /// Total bytes in segments that must be preserved.
    pub preserve_bytes: u32,
    /// Total bytes in segments that can be cleared.
    pub clearable_bytes: u32,
    /// Total bytes in segments that can be rebuilt.
    pub rebuildable_bytes: u32,
}

impl<const N: usize> SegmentMigrationReport<N> {
    /// Analyze decoded segments and produce migration advice per segment.
    pub fn analyze(segments: &[DecodedSegment], count: usize) -> Self {
        let mut report = Self {
            advice: [SegmentAdvice {
                id: [0; 4],
                size: 0,
                role: SegmentRoleHint::Unclassified,
                must_preserve: false,
                clearable: false,
                rebuildable: false,
                append_only: false,
                immutable: false,
            }; N],
            count: 0,
            preserve_bytes: 0,
            clearable_bytes: 0,
            rebuildable_bytes: 0,
        };

        let mut i = 0;
        while i < count && i < N {
            let seg = &segments[i];
            let role = SegmentRoleHint::from_flags(seg.flags);

            report.advice[i] = SegmentAdvice {
                id: seg.id,
                size: seg.size,
                role,
                must_preserve: role.must_preserve(),
                clearable: role.is_clearable(),
                rebuildable: role.is_rebuildable(),
                append_only: role.is_append_only(),
                immutable: role.is_immutable(),
            };

            if role.must_preserve() {
                report.preserve_bytes += seg.size;
            }
            if role.is_clearable() {
                report.clearable_bytes += seg.size;
            }
            if role.is_rebuildable() {
                report.rebuildable_bytes += seg.size;
            }

            report.count += 1;
            i += 1;
        }

        report
    }

    /// Number of segments that must be preserved during migration.
    pub fn must_preserve_count(&self) -> usize {
        let mut n = 0;
        let mut i = 0;
        while i < self.count {
            if self.advice[i].must_preserve { n += 1; }
            i += 1;
        }
        n
    }

    /// Number of segments that can be safely cleared during migration.
    pub fn clearable_count(&self) -> usize {
        let mut n = 0;
        let mut i = 0;
        while i < self.count {
            if self.advice[i].clearable { n += 1; }
            i += 1;
        }
        n
    }
}

impl<const N: usize> fmt::Display for SegmentMigrationReport<N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Segment Migration Advice ({} segments):", self.count)?;
        let mut i = 0;
        while i < self.count {
            let a = &self.advice[i];
            write!(f, "  [{}] {} ({} bytes):", i, a.role.name(), a.size)?;
            if a.must_preserve {
                write!(f, " MUST-PRESERVE")?;
            }
            if a.clearable {
                write!(f, " clearable")?;
            }
            if a.rebuildable {
                write!(f, " rebuildable")?;
            }
            if a.append_only {
                write!(f, " append-only")?;
            }
            if a.immutable {
                write!(f, " immutable")?;
            }
            writeln!(f)?;
            i += 1;
        }
        writeln!(f, "  preserve={} bytes, clearable={} bytes, rebuildable={} bytes",
            self.preserve_bytes, self.clearable_bytes, self.rebuildable_bytes)?;
        Ok(())
    }
}

// -- Inspection Surfaces --
//
// `core::fmt::Display` implementations for decoded types.
// These provide human-readable output for CLI tooling, indexers,
// and debugging without requiring `std`.

impl fmt::Display for DecodedHeader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Header {{ disc: {}, ver: {}, flags: 0x{:04x}, layout_id: ",
            self.disc, self.version, self.flags,
        )?;
        write_hex(f, &self.layout_id)?;
        write!(f, " }}")
    }
}

impl fmt::Debug for DecodedHeader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "DecodedHeader {{ disc: {}, version: {}, flags: 0x{:04x}, layout_id: ",
            self.disc, self.version, self.flags,
        )?;
        write_hex(f, &self.layout_id)?;
        write!(f, ", reserved: ")?;
        write_hex(f, &self.reserved)?;
        write!(f, " }}")
    }
}

impl fmt::Display for DecodedSegment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Segment {{ id: ")?;
        write_hex(f, &self.id)?;
        write!(
            f,
            ", offset: {}, size: {}, flags: 0x{:04x}, ver: {} }}",
            self.offset, self.size, self.flags, self.version,
        )
    }
}

impl fmt::Debug for DecodedSegment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DecodedSegment {{ id: ")?;
        write_hex(f, &self.id)?;
        write!(
            f,
            ", offset: {}, size: {}, flags: 0x{:04x}, version: {} }}",
            self.offset, self.size, self.flags, self.version,
        )
    }
}

impl fmt::Display for FieldCompat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FieldCompat::Identical => write!(f, "identical"),
            FieldCompat::Changed => write!(f, "changed"),
            FieldCompat::Added => write!(f, "added"),
            FieldCompat::Removed => write!(f, "removed"),
        }
    }
}

impl fmt::Display for MigrationPolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MigrationPolicy::NoOp => write!(f, "no-op"),
            MigrationPolicy::AppendOnly => write!(f, "append-only"),
            MigrationPolicy::RequiresMigration => write!(f, "requires-migration"),
            MigrationPolicy::Incompatible => write!(f, "incompatible"),
        }
    }
}

impl fmt::Display for MigrationAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MigrationAction::CopyPrefix => write!(f, "copy-prefix"),
            MigrationAction::ZeroInit => write!(f, "zero-init"),
            MigrationAction::UpdateHeader => write!(f, "update-header"),
            MigrationAction::Realloc => write!(f, "realloc"),
        }
    }
}

impl<'a> fmt::Display for MigrationStep<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} @ offset={}, size={}", self.action, self.offset, self.size)?;
        if !self.field.is_empty() {
            write!(f, " (field: {})", self.field)?;
        }
        Ok(())
    }
}

impl<'a, const N: usize> fmt::Display for MigrationPlan<'a, N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "MigrationPlan ({}):", self.policy)?;
        writeln!(f, "  old_size={}, new_size={}", self.old_size, self.new_size)?;
        writeln!(f, "  copy={} bytes, zero={} bytes", self.copy_bytes, self.zero_bytes)?;
        let mut i = 0;
        while i < self.step_count {
            writeln!(f, "  step {}: {}", i, self.steps[i])?;
            i += 1;
        }
        Ok(())
    }
}

impl fmt::Display for LayoutManifest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "{} v{} (disc={}, size={})", self.name, self.version, self.disc, self.total_size)?;
        write!(f, "  layout_id: ")?;
        write_hex(f, &self.layout_id)?;
        writeln!(f)?;
        let mut i = 0;
        while i < self.field_count {
            let field = &self.fields[i];
            writeln!(
                f,
                "  [{:>3}..{:>3}] {} : {} ({} bytes)",
                field.offset,
                field.offset + field.size,
                field.name,
                field.canonical_type,
                field.size,
            )?;
            i += 1;
        }
        Ok(())
    }
}

/// Decode an account header and format it for display.
///
/// Returns `None` if data is too short.
pub fn format_header(data: &[u8]) -> Option<DecodedHeader> {
    decode_header(data)
}

/// Decode segments and return a displayable segment map string.
///
/// Returns `None` if data doesn't contain a valid segment registry.
pub fn format_segment_map<const N: usize>(data: &[u8]) -> Option<SegmentMap<N>> {
    let (count, segments) = decode_segments::<N>(data)?;
    Some(SegmentMap { count, segments })
}

/// A decoded segment map for display.
pub struct SegmentMap<const N: usize> {
    pub count: usize,
    pub segments: [DecodedSegment; N],
}

impl<const N: usize> fmt::Display for SegmentMap<N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Segment Map ({} segments):", self.count)?;
        let reg_end = HEADER_LEN + 4 + self.count * 16;
        writeln!(f, "  [  0..{:>3}] Header", HEADER_LEN)?;
        writeln!(f, "  [{:>3}..{:>3}] Registry", HEADER_LEN, reg_end)?;
        let mut i = 0;
        while i < self.count {
            let seg = &self.segments[i];
            let end = seg.offset + seg.size;
            write!(f, "  [{:>3}..{:>3}] Segment {} (id=", seg.offset, end, i)?;
            write_hex(f, &seg.id)?;
            writeln!(f, ", {} bytes, v{})", seg.size, seg.version)?;
            i += 1;
        }
        Ok(())
    }
}

/// Write bytes as hex to a formatter (no_std compatible).
fn write_hex(f: &mut fmt::Formatter<'_>, bytes: &[u8]) -> fmt::Result {
    for b in bytes {
        write!(f, "{:02x}", b)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Program Manifest -- full program schema for Manager / tooling
// ---------------------------------------------------------------------------

/// An account entry in an instruction's account list.
#[derive(Clone, Copy)]
pub struct AccountEntry {
    /// Account name.
    pub name: &'static str,
    /// Whether the account is writable.
    pub writable: bool,
    /// Whether the account is a signer.
    pub signer: bool,
    /// Optional layout reference name (for typed accounts).
    pub layout_ref: &'static str,
}

/// An argument descriptor for an instruction.
#[derive(Clone, Copy)]
pub struct ArgDescriptor {
    /// Argument name.
    pub name: &'static str,
    /// Canonical type name.
    pub canonical_type: &'static str,
    /// Byte size.
    pub size: u16,
}

/// An instruction descriptor in a program manifest.
#[derive(Clone, Copy)]
pub struct InstructionDescriptor {
    /// Instruction name.
    pub name: &'static str,
    /// Discriminator tag.
    pub tag: u8,
    /// Arguments.
    pub args: &'static [ArgDescriptor],
    /// Accounts.
    pub accounts: &'static [AccountEntry],
    /// Capability names.
    pub capabilities: &'static [&'static str],
    /// Policy pack name (empty if custom).
    pub policy_pack: &'static str,
    /// Whether this instruction emits a receipt.
    pub receipt_expected: bool,
}

/// An event descriptor in a program manifest.
#[derive(Clone, Copy)]
pub struct EventDescriptor {
    /// Event name.
    pub name: &'static str,
    /// Event discriminator tag.
    pub tag: u8,
    /// Event fields.
    pub fields: &'static [FieldDescriptor],
}

/// A policy descriptor in a program manifest.
#[derive(Clone, Copy)]
pub struct PolicyDescriptor {
    /// Policy pack name.
    pub name: &'static str,
    /// Capability names this policy covers.
    pub capabilities: &'static [&'static str],
    /// Requirement names this policy triggers.
    pub requirements: &'static [&'static str],
    /// Invariant names this policy checks.
    pub invariants: &'static [&'static str],
    /// Receipt profile expected when this policy is active.
    pub receipt_profile: &'static str,
}

/// Extended per-layout metadata for the manifest.
///
/// Carries richer operational metadata that Hopper Manager, CLI, and
/// migration tooling use beyond what `LayoutManifest` provides.
#[derive(Clone, Copy)]
pub struct LayoutMetadata {
    /// Layout name (must match corresponding `LayoutManifest.name`).
    pub name: &'static str,
    /// Segment role descriptors (for segmented accounts).
    pub segment_roles: &'static [&'static str],
    /// Whether append-only changes to this layout are always safe.
    pub append_safe: bool,
    /// Whether changes require an explicit migration instruction.
    pub migration_required: bool,
    /// Whether derived data (index/cache segments) can be rebuilt from core.
    pub rebuildable: bool,
    /// Policy pack name that governs writes to this layout.
    pub policy_pack: &'static str,
    /// Invariant pack names that must hold for this layout.
    pub invariant_pack: &'static [&'static str],
    /// Receipt profile name for mutations on this layout.
    pub receipt_profile: &'static str,
    /// Execution phases this layout participates in.
    pub phase_requirements: &'static [&'static str],
    /// Trust profile label (e.g. "verified", "trusted", "unchecked").
    pub trust_profile: &'static str,
    /// Hints for Hopper Manager rendering.
    pub manager_hints: &'static [&'static str],
}

/// A compatibility pair describing a known upgrade path.
#[derive(Clone, Copy)]
pub struct CompatibilityPair {
    /// Old layout name.
    pub from_layout: &'static str,
    /// Old version.
    pub from_version: u8,
    /// New layout name.
    pub to_layout: &'static str,
    /// New version.
    pub to_version: u8,
    /// Migration policy.
    pub policy: MigrationPolicy,
    /// Whether backward reading is supported.
    pub backward_readable: bool,
}

/// A full program manifest for Hopper Manager and tooling.
///
/// This is the **rich internal schema** that powers `hopper manager`,
/// compatibility checking, migration planning, receipt rendering, and
/// CLI inspection. It is intentionally richer than the public IDL --
/// the manifest carries operational metadata that tools need but
/// external consumers do not.
///
/// ## Truth hierarchy
///
/// ```text
/// ProgramManifest  ⊃  ProgramIdl  ⊃  CodamaProjection
///       (rich)         (public)         (interop)
/// ```
#[derive(Clone, Copy)]
pub struct ProgramManifest {
    /// Program name.
    pub name: &'static str,
    /// Program version string.
    pub version: &'static str,
    /// Program description.
    pub description: &'static str,
    /// Layout manifests for all account types.
    pub layouts: &'static [LayoutManifest],
    /// Extended per-layout operational metadata.
    pub layout_metadata: &'static [LayoutMetadata],
    /// Instruction descriptors.
    pub instructions: &'static [InstructionDescriptor],
    /// Event descriptors.
    pub events: &'static [EventDescriptor],
    /// Policy descriptors.
    pub policies: &'static [PolicyDescriptor],
    /// Known upgrade paths between layout versions.
    pub compatibility_pairs: &'static [CompatibilityPair],
    /// Tooling / rendering hints for Manager.
    pub tooling_hints: &'static [&'static str],
    /// Context (instruction account struct) descriptors.
    pub contexts: &'static [crate::accounts::ContextDescriptor],
}

// ---------------------------------------------------------------------------
// Program IDL -- public schema subset
// ---------------------------------------------------------------------------

/// PDA seed hint for an instruction account.
#[derive(Clone, Copy)]
pub struct PdaSeedHint {
    /// Seed kind: "literal", "account", "arg".
    pub kind: &'static str,
    /// Seed value or reference name.
    pub value: &'static str,
}

/// IDL account entry with optional PDA metadata.
#[derive(Clone, Copy)]
pub struct IdlAccountEntry {
    /// Account name.
    pub name: &'static str,
    /// Whether the account is writable.
    pub writable: bool,
    /// Whether the account is a signer.
    pub signer: bool,
    /// Optional layout reference name.
    pub layout_ref: &'static str,
    /// PDA seed hints (empty if not a PDA).
    pub pda_seeds: &'static [PdaSeedHint],
}

/// IDL instruction descriptor.
#[derive(Clone, Copy)]
pub struct IdlInstructionDescriptor {
    /// Instruction name.
    pub name: &'static str,
    /// Discriminator tag.
    pub tag: u8,
    /// Arguments.
    pub args: &'static [ArgDescriptor],
    /// Accounts with PDA metadata.
    pub accounts: &'static [IdlAccountEntry],
}

/// A public-facing IDL for a Hopper program.
///
/// Contains only what external consumers (clients, explorers, SDKs) need.
/// Does NOT contain internal policy logic, migration planner hints,
/// trust internals, or unsafe metadata.
///
/// Generated from (and strictly a subset of) `ProgramManifest`.
#[derive(Clone, Copy)]
pub struct ProgramIdl {
    /// Program name.
    pub name: &'static str,
    /// Program version string.
    pub version: &'static str,
    /// Program description.
    pub description: &'static str,
    /// Instructions with args, accounts, PDA hints.
    pub instructions: &'static [IdlInstructionDescriptor],
    /// Account layout summaries (name, disc, version, layout_id, size, fields).
    pub accounts: &'static [LayoutManifest],
    /// Event descriptors.
    pub events: &'static [EventDescriptor],
    /// Optional layout_id fingerprints per account.
    pub fingerprints: &'static [([u8; 8], &'static str)],
}

impl ProgramIdl {
    /// Create an empty IDL.
    pub const fn empty() -> Self {
        Self {
            name: "",
            version: "",
            description: "",
            instructions: &[],
            accounts: &[],
            events: &[],
            fingerprints: &[],
        }
    }

    /// Number of instructions.
    pub const fn instruction_count(&self) -> usize {
        self.instructions.len()
    }

    /// Number of account types.
    pub const fn account_count(&self) -> usize {
        self.accounts.len()
    }

    /// Find an instruction by name.
    pub fn find_instruction(&self, name: &str) -> Option<&IdlInstructionDescriptor> {
        let mut i = 0;
        while i < self.instructions.len() {
            if const_str_eq(self.instructions[i].name, name) {
                return Some(&self.instructions[i]);
            }
            i += 1;
        }
        None
    }

    /// Find an account layout by name.
    pub fn find_account(&self, name: &str) -> Option<&LayoutManifest> {
        let mut i = 0;
        while i < self.accounts.len() {
            if const_str_eq(self.accounts[i].name, name) {
                return Some(&self.accounts[i]);
            }
            i += 1;
        }
        None
    }
}

impl fmt::Display for ProgramIdl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "IDL: {} {}", self.name, self.version)?;
        if !self.description.is_empty() {
            writeln!(f, "  {}", self.description)?;
        }
        writeln!(f)?;
        writeln!(f, "Instructions ({}):", self.instructions.len())?;
        for ix in self.instructions.iter() {
            write!(f, "  {:>2}  {:16} args={} accounts={}",
                ix.tag, ix.name, ix.args.len(), ix.accounts.len())?;
            writeln!(f)?;
        }
        writeln!(f)?;
        writeln!(f, "Accounts ({}):", self.accounts.len())?;
        for a in self.accounts.iter() {
            write!(f, "  {:16} disc={} v{} {} bytes  id=",
                a.name, a.disc, a.version, a.total_size)?;
            write_hex(f, &a.layout_id)?;
            writeln!(f)?;
        }
        if !self.events.is_empty() {
            writeln!(f)?;
            writeln!(f, "Events ({}):", self.events.len())?;
            for e in self.events.iter() {
                writeln!(f, "  {:>2}  {:16} fields={}", e.tag, e.name, e.fields.len())?;
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Codama Projection -- ecosystem interop subset
// ---------------------------------------------------------------------------

/// Codama-compatible instruction descriptor.
///
/// Only the fields needed for Codama/Kinobi IDL generation.
#[derive(Clone, Copy)]
pub struct CodamaInstruction {
    pub name: &'static str,
    pub discriminator: u8,
    pub args: &'static [ArgDescriptor],
    pub accounts: &'static [IdlAccountEntry],
}

/// Codama-compatible account descriptor.
#[derive(Clone, Copy)]
pub struct CodamaAccount {
    pub name: &'static str,
    pub discriminator: u8,
    pub size: usize,
    pub fields: &'static [FieldDescriptor],
}

/// Codama-compatible event descriptor.
#[derive(Clone, Copy)]
pub struct CodamaEvent {
    pub name: &'static str,
    pub discriminator: u8,
    pub fields: &'static [FieldDescriptor],
}

/// Codama-compatible projection of a Hopper program.
///
/// This is a **bridge**, not a prison. It maps the clean public subset
/// of a Hopper program into a shape that Codama/Kinobi tooling can consume.
///
/// Does NOT include: internal policy logic, migration planner hints,
/// trust internals, unsafe metadata, segment roles, or manager hints.
///
/// ## Layering
///
/// ```text
/// ProgramManifest = rich truth
/// ProgramIdl      = public schema
/// CodamaProjection = compatibility projection
/// ```
#[derive(Clone, Copy)]
pub struct CodamaProjection {
    /// Program name.
    pub name: &'static str,
    /// Program version.
    pub version: &'static str,
    /// Instructions (public subset).
    pub instructions: &'static [CodamaInstruction],
    /// Account types (public subset).
    pub accounts: &'static [CodamaAccount],
    /// Events (public subset).
    pub events: &'static [CodamaEvent],
}

impl CodamaProjection {
    /// Create an empty projection.
    pub const fn empty() -> Self {
        Self {
            name: "",
            version: "",
            instructions: &[],
            accounts: &[],
            events: &[],
        }
    }
}

impl fmt::Display for CodamaProjection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Codama: {} {}", self.name, self.version)?;
        writeln!(f)?;
        writeln!(f, "Instructions ({}):", self.instructions.len())?;
        for ix in self.instructions.iter() {
            writeln!(f, "  {:>2}  {:16} args={} accounts={}",
                ix.discriminator, ix.name, ix.args.len(), ix.accounts.len())?;
        }
        writeln!(f)?;
        writeln!(f, "Accounts ({}):", self.accounts.len())?;
        for a in self.accounts.iter() {
            writeln!(f, "  {:16} disc={} {} bytes fields={}",
                a.name, a.discriminator, a.size, a.fields.len())?;
        }
        if !self.events.is_empty() {
            writeln!(f)?;
            writeln!(f, "Events ({}):", self.events.len())?;
            for e in self.events.iter() {
                writeln!(f, "  {:>2}  {:16} fields={}", e.discriminator, e.name, e.fields.len())?;
            }
        }
        Ok(())
    }
}

impl ProgramManifest {
    /// Create an empty program manifest.
    pub const fn empty() -> Self {
        Self {
            name: "",
            version: "",
            description: "",
            layouts: &[],
            layout_metadata: &[],
            instructions: &[],
            events: &[],
            policies: &[],
            compatibility_pairs: &[],
            tooling_hints: &[],
            contexts: &[],
        }
    }

    /// Number of layouts.
    pub const fn layout_count(&self) -> usize {
        self.layouts.len()
    }

    /// Number of instructions.
    pub const fn instruction_count(&self) -> usize {
        self.instructions.len()
    }

    /// Find a layout by discriminator.
    pub fn find_layout_by_disc(&self, disc: u8) -> Option<&LayoutManifest> {
        let mut i = 0;
        while i < self.layouts.len() {
            if self.layouts[i].disc == disc {
                return Some(&self.layouts[i]);
            }
            i += 1;
        }
        None
    }

    /// Find a layout by layout_id fingerprint.
    pub fn find_layout_by_id(&self, layout_id: &[u8; 8]) -> Option<&LayoutManifest> {
        let mut i = 0;
        while i < self.layouts.len() {
            if self.layouts[i].layout_id == *layout_id {
                return Some(&self.layouts[i]);
            }
            i += 1;
        }
        None
    }

    /// Find a layout that matches the given account data header.
    pub fn identify_from_data(&self, data: &[u8]) -> Option<&LayoutManifest> {
        let header = decode_header(data)?;
        // Try layout_id match first (strongest)
        if let Some(m) = self.find_layout_by_id(&header.layout_id) {
            return Some(m);
        }
        // Fall back to disc match
        self.find_layout_by_disc(header.disc)
    }

    /// Find an instruction by tag.
    pub fn find_instruction(&self, tag: u8) -> Option<&InstructionDescriptor> {
        let mut i = 0;
        while i < self.instructions.len() {
            if self.instructions[i].tag == tag {
                return Some(&self.instructions[i]);
            }
            i += 1;
        }
        None
    }

    /// Find a policy by name.
    pub fn find_policy(&self, name: &str) -> Option<&PolicyDescriptor> {
        let mut i = 0;
        while i < self.policies.len() {
            if self.policies[i].name == name {
                return Some(&self.policies[i]);
            }
            i += 1;
        }
        None
    }

    /// Find extended layout metadata by layout name.
    pub fn find_layout_metadata(&self, name: &str) -> Option<&LayoutMetadata> {
        let mut i = 0;
        while i < self.layout_metadata.len() {
            if const_str_eq(self.layout_metadata[i].name, name) {
                return Some(&self.layout_metadata[i]);
            }
            i += 1;
        }
        None
    }

    /// Find a compatibility pair for an upgrade path.
    pub fn find_compat_pair(
        &self,
        from_name: &str,
        from_ver: u8,
        to_name: &str,
        to_ver: u8,
    ) -> Option<&CompatibilityPair> {
        let mut i = 0;
        while i < self.compatibility_pairs.len() {
            let cp = &self.compatibility_pairs[i];
            if const_str_eq(cp.from_layout, from_name)
                && cp.from_version == from_ver
                && const_str_eq(cp.to_layout, to_name)
                && cp.to_version == to_ver
            {
                return Some(cp);
            }
            i += 1;
        }
        None
    }
}

impl fmt::Display for ProgramManifest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Program: {} {}", self.name, self.version)?;
        if !self.description.is_empty() {
            writeln!(f, "  {}", self.description)?;
        }
        writeln!(f)?;

        writeln!(f, "Layouts ({}):", self.layouts.len())?;
        for m in self.layouts.iter() {
            write!(f, "  {:16} v{}  disc={}  {} bytes  fingerprint=",
                m.name, m.version, m.disc, m.total_size)?;
            write_hex(f, &m.layout_id)?;
            // Show extended metadata if available
            if let Some(meta) = self.find_layout_metadata(m.name) {
                if !meta.trust_profile.is_empty() {
                    write!(f, "  trust={}", meta.trust_profile)?;
                }
                if meta.append_safe {
                    write!(f, "  append-safe")?;
                }
                if meta.migration_required {
                    write!(f, "  migration-required")?;
                }
            }
            writeln!(f)?;
        }
        writeln!(f)?;

        writeln!(f, "Instructions ({}):", self.instructions.len())?;
        for ix in self.instructions.iter() {
            write!(f, "  {:>2}  {:16} accounts={}",
                ix.tag, ix.name, ix.accounts.len())?;
            if !ix.capabilities.is_empty() {
                write!(f, "  caps=")?;
                for (j, c) in ix.capabilities.iter().enumerate() {
                    if j > 0 { write!(f, ",")?; }
                    write!(f, "{}", c)?;
                }
            }
            if ix.receipt_expected {
                write!(f, "  receipt=yes")?;
            }
            writeln!(f)?;
        }
        writeln!(f)?;

        if !self.policies.is_empty() {
            writeln!(f, "Policies ({}):", self.policies.len())?;
            for p in self.policies.iter() {
                write!(f, "  {:24}", p.name)?;
                for (j, r) in p.requirements.iter().enumerate() {
                    if j > 0 { write!(f, " + ")?; }
                    write!(f, "{}", r)?;
                }
                if !p.receipt_profile.is_empty() {
                    write!(f, "  receipt={}", p.receipt_profile)?;
                }
                writeln!(f)?;
            }
            writeln!(f)?;
        }

        if !self.events.is_empty() {
            writeln!(f, "Events ({}):", self.events.len())?;
            for e in self.events.iter() {
                writeln!(f, "  {:>2}  {:16} fields={}",
                    e.tag, e.name, e.fields.len())?;
            }
            writeln!(f)?;
        }

        if !self.compatibility_pairs.is_empty() {
            writeln!(f, "Compatibility ({}):", self.compatibility_pairs.len())?;
            for cp in self.compatibility_pairs.iter() {
                writeln!(f, "  {} v{} -> {} v{}  {}{}",
                    cp.from_layout, cp.from_version,
                    cp.to_layout, cp.to_version,
                    cp.policy,
                    if cp.backward_readable { "  backward-readable" } else { "" },
                )?;
            }
        }

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Field-Level Account Decoder
// ---------------------------------------------------------------------------

/// A decoded field value from account data.
pub struct DecodedField<'a> {
    /// Field name.
    pub name: &'a str,
    /// Canonical type.
    pub canonical_type: &'a str,
    /// Raw bytes of this field.
    pub raw: &'a [u8],
    /// Offset in account data.
    pub offset: u16,
    /// Size in bytes.
    pub size: u16,
}

impl<'a> DecodedField<'a> {
    /// Format the field value as a human-readable string.
    ///
    /// Recognizes common Hopper wire types and formats them appropriately.
    pub fn format_value(&self, buf: &mut [u8]) -> usize {
        match self.canonical_type {
            "WireU64" | "LeU64" if self.raw.len() >= 8 => {
                let v = u64::from_le_bytes([
                    self.raw[0], self.raw[1], self.raw[2], self.raw[3],
                    self.raw[4], self.raw[5], self.raw[6], self.raw[7],
                ]);
                format_u64(v, buf)
            }
            "WireU32" | "LeU32" if self.raw.len() >= 4 => {
                let v = u32::from_le_bytes([
                    self.raw[0], self.raw[1], self.raw[2], self.raw[3],
                ]) as u64;
                format_u64(v, buf)
            }
            "WireU16" | "LeU16" if self.raw.len() >= 2 => {
                let v = u16::from_le_bytes([self.raw[0], self.raw[1]]) as u64;
                format_u64(v, buf)
            }
            "WireBool" | "LeBool" if !self.raw.is_empty() => {
                if self.raw[0] != 0 {
                    let len = 4usize.min(buf.len());
                    buf[..len].copy_from_slice(&b"true"[..len]);
                    len
                } else {
                    let len = 5usize.min(buf.len());
                    buf[..len].copy_from_slice(&b"false"[..len]);
                    len
                }
            }
            "u8" if self.raw.len() == 1 => {
                format_u64(self.raw[0] as u64, buf)
            }
            _ if self.size == 32 => {
                // Likely an address/pubkey -- show as hex
                format_hex_truncated(self.raw, buf)
            }
            _ => {
                format_hex_truncated(self.raw, buf)
            }
        }
    }
}

/// Decode all fields of an account against a layout manifest.
///
/// Returns the number of fields decoded (up to N).
pub fn decode_account_fields<'a, const N: usize>(
    data: &'a [u8],
    manifest: &'a LayoutManifest,
) -> (usize, [Option<DecodedField<'a>>; N]) {
    let mut fields: [Option<DecodedField<'a>>; N] = [const { None }; N];
    let count = manifest.field_count.min(N);
    let mut i = 0;
    while i < count {
        let fd = &manifest.fields[i];
        let start = fd.offset as usize;
        let end = start + fd.size as usize;
        if end <= data.len() {
            fields[i] = Some(DecodedField {
                name: fd.name,
                canonical_type: fd.canonical_type,
                raw: &data[start..end],
                offset: fd.offset,
                size: fd.size,
            });
        }
        i += 1;
    }
    (count, fields)
}

/// Format a u64 as decimal into a byte buffer. Returns bytes written.
fn format_u64(mut v: u64, buf: &mut [u8]) -> usize {
    if v == 0 {
        if !buf.is_empty() {
            buf[0] = b'0';
            return 1;
        }
        return 0;
    }
    // Write digits in reverse
    let mut tmp = [0u8; 20];
    let mut pos = 0;
    while v > 0 && pos < 20 {
        tmp[pos] = b'0' + (v % 10) as u8;
        v /= 10;
        pos += 1;
    }
    let len = pos.min(buf.len());
    let mut i = 0;
    while i < len {
        buf[i] = tmp[pos - 1 - i];
        i += 1;
    }
    len
}

/// Format bytes as hex, truncated to fit buffer. Shows first 8 + "..." if long.
fn format_hex_truncated(bytes: &[u8], buf: &mut [u8]) -> usize {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let max_bytes = if bytes.len() > 8 { 8 } else { bytes.len() };
    let mut pos = 0;
    // "0x" prefix
    if buf.len() >= 2 {
        buf[0] = b'0';
        buf[1] = b'x';
        pos = 2;
    }
    let mut i = 0;
    while i < max_bytes && pos + 1 < buf.len() {
        buf[pos] = HEX[(bytes[i] >> 4) as usize];
        buf[pos + 1] = HEX[(bytes[i] & 0xf) as usize];
        pos += 2;
        i += 1;
    }
    if bytes.len() > 8 && pos + 3 <= buf.len() {
        buf[pos] = b'.';
        buf[pos + 1] = b'.';
        buf[pos + 2] = b'.';
        pos += 3;
    }
    pos
}

// ---------------------------------------------------------------------------
// On-Chain Schema Pointer
// ---------------------------------------------------------------------------

/// On-chain account that points to a Hopper program's schema.
///
/// Stored at PDA `["hopper-schema", program_id]`. Contains hashes of
/// the manifest, IDL, and Codama projection, plus a URI to fetch the
/// full manifest. See `docs/ONCHAIN_SCHEMA_PUBLICATION.md`.
///
/// ## Wire layout (294 bytes payload + 16 bytes header = 310 bytes)
///
/// ```text
/// [0..16]    Hopper header (disc=255, ver=1)
/// [16..18]   schema_version   u16 LE
/// [18..20]   pointer_flags    u16 LE
/// [20..52]   manifest_hash    [u8; 32]
/// [52..84]   idl_hash         [u8; 32]
/// [84..116]  codama_hash      [u8; 32]
/// [116..118] uri_len          u16 LE
/// [118..310] uri              [u8; 192]
/// ```
#[repr(C)]
#[derive(Clone, Copy)]
pub struct HopperSchemaPointer {
    /// Schema format version (currently 1).
    pub schema_version: u16,
    /// Feature flags (HAS_MANIFEST, HAS_IDL, HAS_CODAMA, HAS_URI, ...).
    pub pointer_flags: u16,
    /// SHA-256 of the manifest JSON.
    pub manifest_hash: [u8; 32],
    /// SHA-256 of the IDL JSON.
    pub idl_hash: [u8; 32],
    /// SHA-256 of the Codama projection JSON.
    pub codama_hash: [u8; 32],
    /// Length of the URI string (0..192).
    pub uri_len: u16,
    /// UTF-8 URI pointing to the manifest (padded with zeros).
    pub uri: [u8; 192],
}

impl HopperSchemaPointer {
    /// Reserved discriminator for schema pointer accounts.
    pub const DISC: u8 = 255;

    /// Total payload size (excluding Hopper header).
    pub const PAYLOAD_LEN: usize = 2 + 2 + 32 + 32 + 32 + 2 + 192; // 294

    /// Total account size including Hopper header.
    pub const ACCOUNT_LEN: usize = HEADER_LEN + Self::PAYLOAD_LEN; // 310

    /// PDA seed prefix.
    pub const PDA_SEED: &'static [u8] = b"hopper-schema";

    // Flag bits
    pub const FLAG_HAS_MANIFEST: u16 = 0x0001;
    pub const FLAG_HAS_IDL: u16 = 0x0002;
    pub const FLAG_HAS_CODAMA: u16 = 0x0004;
    pub const FLAG_HAS_URI: u16 = 0x0008;
    pub const FLAG_URI_IS_IPFS: u16 = 0x0010;
    pub const FLAG_URI_IS_ARWEAVE: u16 = 0x0020;

    /// Get the URI as a string slice.
    pub fn uri_str(&self) -> &str {
        let len = (self.uri_len as usize).min(192);
        // SAFETY: We validate UTF-8 at read time.
        core::str::from_utf8(&self.uri[..len]).unwrap_or("")
    }

    /// Check if a specific flag is set.
    #[inline(always)]
    pub fn has_flag(&self, flag: u16) -> bool {
        self.pointer_flags & flag != 0
    }
}

// ---------------------------------------------------------------------------
// Semantic Lint Engine -- catch suspicious state patterns at build time
// ---------------------------------------------------------------------------

/// A semantic lint warning produced by analyzing field intents, mutation
/// classes, and policy against a layout manifest.
#[derive(Clone, Copy, Debug)]
pub struct SemanticLint {
    /// Lint severity.
    pub severity: LintSeverity,
    /// Short machine-readable code.
    pub code: &'static str,
    /// Human-readable warning message.
    pub message: &'static str,
    /// Field name involved (empty if layout-wide).
    pub field: &'static str,
}

/// Lint severity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum LintSeverity {
    /// Informational note.
    Info = 0,
    /// Potential issue worth reviewing.
    Warning = 1,
    /// Likely correctness or security issue.
    Error = 2,
}

impl LintSeverity {
    /// Human-readable label.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Warning => "warning",
            Self::Error => "error",
        }
    }
}

/// Run semantic lints against a layout manifest and its behavior.
///
/// Returns the number of lint warnings produced (up to N).
pub fn lint_layout<const N: usize>(
    manifest: &LayoutManifest,
    behavior: &LayoutBehavior,
) -> (usize, [SemanticLint; N]) {
    let mut lints = [SemanticLint {
        severity: LintSeverity::Info,
        code: "",
        message: "",
        field: "",
    }; N];
    let mut count = 0usize;

    let mut i = 0;
    while i < manifest.field_count {
        let field = &manifest.fields[i];

        // Authority field mutated without signer requirement
        if field.intent.is_authority_sensitive()
            && behavior.mutation_class.is_mutating()
            && !behavior.requires_signer
        {
            if count < N {
                lints[count] = SemanticLint {
                    severity: LintSeverity::Error,
                    code: "E001",
                    message: "Authority-sensitive field in mutable layout without signer requirement",
                    field: field.name,
                };
                count += 1;
            }
        }

        // Financial field mutated without financial mutation class
        if field.intent.is_monetary()
            && behavior.mutation_class.is_mutating()
            && !matches!(behavior.mutation_class, MutationClass::Financial)
        {
            if count < N {
                lints[count] = SemanticLint {
                    severity: LintSeverity::Warning,
                    code: "W001",
                    message: "Monetary field in layout without financial mutation class",
                    field: field.name,
                };
                count += 1;
            }
        }

        // Init-only field (PDA seed, bump) in a layout that isn't read-only
        if field.intent.is_init_only()
            && behavior.mutation_class.is_mutating()
            && !matches!(behavior.mutation_class, MutationClass::AppendOnly)
        {
            if count < N {
                lints[count] = SemanticLint {
                    severity: LintSeverity::Warning,
                    code: "W002",
                    message: "Init-only field (PDA seed or bump) in mutable layout. Consider making read-only or append-only.",
                    field: field.name,
                };
                count += 1;
            }
        }

        i += 1;
    }

    // Layout-wide lints

    // Mutable layout with no signer
    if behavior.mutation_class.is_mutating() && !behavior.requires_signer {
        if count < N {
            lints[count] = SemanticLint {
                severity: LintSeverity::Warning,
                code: "W003",
                message: "Mutable layout does not require signer. Verify this is intentional.",
                field: "",
            };
            count += 1;
        }
    }

    // Financial impact without balance tracking
    if behavior.affects_balance {
        let mut has_balance = false;
        let mut j = 0;
        while j < manifest.field_count {
            if manifest.fields[j].intent.is_monetary() {
                has_balance = true;
            }
            j += 1;
        }
        if !has_balance && count < N {
            lints[count] = SemanticLint {
                severity: LintSeverity::Warning,
                code: "W004",
                message: "Layout behavior declares affects_balance but no monetary fields found",
                field: "",
            };
            count += 1;
        }
    }

    (count, lints)
}

/// Run policy-aware semantic lints.
///
/// Complements `lint_layout` with cross-cutting checks between layout
/// behavior and policy classification. Call after `lint_layout` and merge
/// the results.
pub fn lint_policy<const N: usize>(
    behavior: &LayoutBehavior,
    policy: PolicyClass,
) -> (usize, [SemanticLint; N]) {
    let mut lints = [SemanticLint {
        severity: LintSeverity::Info,
        code: "",
        message: "",
        field: "",
    }; N];
    let mut count = 0usize;

    // Financial mutation class without financial policy class
    if matches!(behavior.mutation_class, MutationClass::Financial)
        && !matches!(policy, PolicyClass::Financial)
    {
        if count < N {
            lints[count] = SemanticLint {
                severity: LintSeverity::Warning,
                code: "W005",
                message: "Financial mutation class but policy class is not Financial",
                field: "",
            };
            count += 1;
        }
    }

    // Financial policy class without financial mutation class
    if matches!(policy, PolicyClass::Financial)
        && !matches!(behavior.mutation_class, MutationClass::Financial)
    {
        if count < N {
            lints[count] = SemanticLint {
                severity: LintSeverity::Warning,
                code: "W006",
                message: "Financial policy class but mutation class is not Financial",
                field: "",
            };
            count += 1;
        }
    }

    (count, lints)
}

impl fmt::Display for SemanticLint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}: {}", self.severity.name(), self.code, self.message)?;
        if !self.field.is_empty() {
            write!(f, " (field: {})", self.field)?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Protocol Operating Profile -- machine-readable program behavior map
// ---------------------------------------------------------------------------

/// A machine-readable summary of a program's operational characteristics.
///
/// Generated from a `ProgramManifest` to give auditors, dashboards, explorers,
/// and operator tools a meaningful map of how the program behaves.
pub struct OperatingProfile {
    /// Fields classified as financial (balance, supply, basis_points).
    pub financial_fields: [&'static str; 16],
    /// Number of valid financial field entries.
    pub financial_count: u8,
    /// Fields classified as authority surfaces (authority, owner, delegate).
    pub authority_surfaces: [&'static str; 16],
    /// Number of valid authority surface entries.
    pub authority_count: u8,
    /// Segments that are append-only.
    pub append_only_segments: [&'static str; 8],
    /// Number of valid append-only segment entries.
    pub append_only_count: u8,
    /// Segments sensitive to migration.
    pub migration_sensitive: [&'static str; 8],
    /// Number of valid migration-sensitive entries.
    pub migration_sensitive_count: u8,
    /// Layout stability grades per layout.
    pub stability_grades: [(& 'static str, LayoutStabilityGrade); 8],
    /// Number of valid stability grade entries.
    pub stability_count: u8,
    /// Whether the program has any financial operations.
    pub has_financial_ops: bool,
    /// Whether the program has any CPI-invoking instructions.
    pub has_cpi_ops: bool,
    /// Whether the program has migration paths defined.
    pub has_migration_paths: bool,
    /// Whether the program emits receipts.
    pub has_receipts: bool,
}

impl OperatingProfile {
    /// Generate an operating profile from a program manifest.
    pub fn from_manifest(manifest: &ProgramManifest) -> Self {
        let mut profile = Self {
            financial_fields: [""; 16],
            financial_count: 0,
            authority_surfaces: [""; 16],
            authority_count: 0,
            append_only_segments: [""; 8],
            append_only_count: 0,
            migration_sensitive: [""; 8],
            migration_sensitive_count: 0,
            stability_grades: [("", LayoutStabilityGrade::Stable); 8],
            stability_count: 0,
            has_financial_ops: false,
            has_cpi_ops: false,
            has_migration_paths: !manifest.compatibility_pairs.is_empty(),
            has_receipts: false,
        };

        // Scan layouts for field intents
        let mut li = 0;
        while li < manifest.layouts.len() {
            let layout = &manifest.layouts[li];

            // Stability grade
            if (profile.stability_count as usize) < 8 {
                profile.stability_grades[profile.stability_count as usize] =
                    (layout.name, LayoutStabilityGrade::compute(layout));
                profile.stability_count += 1;
            }

            let mut fi = 0;
            while fi < layout.field_count {
                let field = &layout.fields[fi];
                if field.intent.is_monetary() && (profile.financial_count as usize) < 16 {
                    profile.financial_fields[profile.financial_count as usize] = field.name;
                    profile.financial_count += 1;
                }
                if field.intent.is_authority_sensitive() && (profile.authority_count as usize) < 16 {
                    profile.authority_surfaces[profile.authority_count as usize] = field.name;
                    profile.authority_count += 1;
                }
                fi += 1;
            }
            li += 1;
        }

        // Scan layout metadata for segment info
        let mut mi = 0;
        while mi < manifest.layout_metadata.len() {
            let meta = &manifest.layout_metadata[mi];
            let mut si = 0;
            while si < meta.segment_roles.len() {
                let role_name = meta.segment_roles[si];
                if (const_str_eq(role_name, "Journal") || const_str_eq(role_name, "Audit"))
                    && (profile.append_only_count as usize) < 8
                {
                    profile.append_only_segments[profile.append_only_count as usize] = role_name;
                    profile.append_only_count += 1;
                }
                if const_str_eq(role_name, "Core")
                    && (profile.migration_sensitive_count as usize) < 8
                {
                    profile.migration_sensitive[profile.migration_sensitive_count as usize] = meta.name;
                    profile.migration_sensitive_count += 1;
                }
                si += 1;
            }
            mi += 1;
        }

        // Scan instructions for capabilities
        let mut ii = 0;
        while ii < manifest.instructions.len() {
            let ix = &manifest.instructions[ii];
            if ix.receipt_expected {
                profile.has_receipts = true;
            }
            let mut ci = 0;
            while ci < ix.capabilities.len() {
                if const_str_eq(ix.capabilities[ci], "MutatesTreasury") {
                    profile.has_financial_ops = true;
                }
                if const_str_eq(ix.capabilities[ci], "ExternalCall") {
                    profile.has_cpi_ops = true;
                }
                ci += 1;
            }
            ii += 1;
        }

        profile
    }
}

impl fmt::Display for OperatingProfile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Operating Profile:")?;

        if self.financial_count > 0 {
            write!(f, "  Financial fields:")?;
            let mut i = 0;
            while i < self.financial_count as usize {
                write!(f, " {}", self.financial_fields[i])?;
                i += 1;
            }
            writeln!(f)?;
        }

        if self.authority_count > 0 {
            write!(f, "  Authority surfaces:")?;
            let mut i = 0;
            while i < self.authority_count as usize {
                write!(f, " {}", self.authority_surfaces[i])?;
                i += 1;
            }
            writeln!(f)?;
        }

        if self.append_only_count > 0 {
            write!(f, "  Append-only segments:")?;
            let mut i = 0;
            while i < self.append_only_count as usize {
                write!(f, " {}", self.append_only_segments[i])?;
                i += 1;
            }
            writeln!(f)?;
        }

        if self.stability_count > 0 {
            writeln!(f, "  Stability grades:")?;
            let mut i = 0;
            while i < self.stability_count as usize {
                let (name, grade) = self.stability_grades[i];
                writeln!(f, "    {}: {}", name, grade.name())?;
                i += 1;
            }
        }

        write!(f, "  Features:")?;
        if self.has_financial_ops { write!(f, " financial")?; }
        if self.has_cpi_ops { write!(f, " cpi")?; }
        if self.has_migration_paths { write!(f, " migration")?; }
        if self.has_receipts { write!(f, " receipts")?; }
        writeln!(f)?;

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Expanded IDL -- policies, compat, receipts, segments, field intents
// ---------------------------------------------------------------------------

/// Extended IDL with full Hopper-native sections.
///
/// This is the **complete** program schema for Hopper-aware tools. It extends
/// `ProgramIdl` with policies, compatibility, receipts, segments, field intents,
/// and an operating profile.
pub struct HopperIdl {
    /// Base IDL (instructions, accounts, events).
    pub base: ProgramIdl,
    /// Policy descriptors.
    pub policies: &'static [PolicyDescriptor],
    /// Known upgrade paths.
    pub compatibility: &'static [CompatibilityPair],
    /// Receipt profiles keyed by name.
    pub receipt_profiles: &'static [ReceiptProfile],
    /// Segment metadata.
    pub segment_metadata: &'static [IdlSegmentDescriptor],
    /// Context (instruction account struct) descriptors.
    pub contexts: &'static [crate::accounts::ContextDescriptor],
}

/// A receipt profile describing what a receipt for a given mutation type looks like.
#[derive(Clone, Copy)]
pub struct ReceiptProfile {
    /// Profile name (e.g. "default-mutation", "treasury-write").
    pub name: &'static str,
    /// Expected phase.
    pub expected_phase: &'static str,
    /// Whether balance changes are expected.
    pub expects_balance_change: bool,
    /// Whether authority changes are expected.
    pub expects_authority_change: bool,
    /// Whether journal appends are expected.
    pub expects_journal_append: bool,
    /// Minimum expected changed fields.
    pub min_changed_fields: u8,
}

impl fmt::Display for ReceiptProfile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}(phase={}", self.name, self.expected_phase)?;
        if self.expects_balance_change { write!(f, " balance")?; }
        if self.expects_authority_change { write!(f, " authority")?; }
        if self.expects_journal_append { write!(f, " journal")?; }
        if self.min_changed_fields > 0 {
            write!(f, " min_fields={}", self.min_changed_fields)?;
        }
        write!(f, ")")
    }
}

/// Segment metadata for inclusion in the IDL.
#[derive(Clone, Copy)]
pub struct IdlSegmentDescriptor {
    /// Segment name.
    pub name: &'static str,
    /// Role name (Core, Extension, Journal, etc.).
    pub role: &'static str,
    /// Whether the segment is append-only.
    pub append_only: bool,
    /// Whether the segment is rebuildable from other data.
    pub rebuildable: bool,
    /// Whether the segment must survive migration.
    pub must_preserve: bool,
}

impl fmt::Display for IdlSegmentDescriptor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}(role={}", self.name, self.role)?;
        if self.append_only { write!(f, " append-only")?; }
        if self.rebuildable { write!(f, " rebuildable")?; }
        if self.must_preserve { write!(f, " must-preserve")?; }
        write!(f, ")")
    }
}

impl HopperIdl {
    /// Create an empty extended IDL.
    pub const fn empty() -> Self {
        Self {
            base: ProgramIdl::empty(),
            policies: &[],
            compatibility: &[],
            receipt_profiles: &[],
            segment_metadata: &[],
            contexts: &[],
        }
    }
}

impl fmt::Display for HopperIdl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.base)?;

        if !self.policies.is_empty() {
            writeln!(f)?;
            writeln!(f, "Policies ({}):", self.policies.len())?;
            for p in self.policies.iter() {
                write!(f, "  {:24}", p.name)?;
                for (j, r) in p.requirements.iter().enumerate() {
                    if j > 0 { write!(f, " + ")?; }
                    write!(f, "{}", r)?;
                }
                writeln!(f)?;
            }
        }

        if !self.compatibility.is_empty() {
            writeln!(f)?;
            writeln!(f, "Compatibility ({}):", self.compatibility.len())?;
            for cp in self.compatibility.iter() {
                writeln!(f, "  {} v{} -> {} v{}  {}",
                    cp.from_layout, cp.from_version,
                    cp.to_layout, cp.to_version,
                    cp.policy,
                )?;
            }
        }

        if !self.receipt_profiles.is_empty() {
            writeln!(f)?;
            writeln!(f, "Receipt Profiles ({}):", self.receipt_profiles.len())?;
            for rp in self.receipt_profiles.iter() {
                writeln!(f, "  {:24} phase={} balance={} authority={} journal={}",
                    rp.name, rp.expected_phase,
                    rp.expects_balance_change, rp.expects_authority_change,
                    rp.expects_journal_append,
                )?;
            }
        }

        if !self.segment_metadata.is_empty() {
            writeln!(f)?;
            writeln!(f, "Segments ({}):", self.segment_metadata.len())?;
            for s in self.segment_metadata.iter() {
                write!(f, "  {:16} role={}", s.name, s.role)?;
                if s.append_only { write!(f, " append-only")?; }
                if s.rebuildable { write!(f, " rebuildable")?; }
                if s.must_preserve { write!(f, " must-preserve")?; }
                writeln!(f)?;
            }
        }

        if !self.contexts.is_empty() {
            writeln!(f)?;
            writeln!(f, "Contexts ({}):", self.contexts.len())?;
            for ctx in self.contexts.iter() {
                write!(f, "  {}", ctx)?;
            }
        }

        Ok(())
    }
}

// ═══════════════════════════════════════════════════════════════════════
//  SchemaExport -- bridge from LayoutContract + FieldMap to schema
// ═══════════════════════════════════════════════════════════════════════

/// Minimal manager-readable metadata for a Hopper layout.
#[derive(Clone, Copy, Debug)]
pub struct ManagerMetadata {
    /// Runtime header/layout identity.
    pub layout: LayoutInfo,
    /// Field-level wire map.
    pub fields: &'static [FieldInfo],
}

/// Unified schema payload tying runtime identity to rich manifest metadata.
#[derive(Clone, Copy, Debug)]
pub struct SchemaBundle {
    pub manager: ManagerMetadata,
    pub manifest: LayoutManifest,
}

/// Trait for layout types that can export their full schema information.
///
/// This creates a single source of truth linking runtime layout contracts
/// (discriminator, version, layout_id, size) with field-level metadata
/// (names, offsets, sizes). The exported information powers:
///
/// - Manager metadata (on-chain or off-chain program inspection)
/// - IDL generation (Codama, Hopper IDL, client SDKs)
/// - Schema diff and migration safety checking
/// - Client code generation with typed field access
///
/// Implementors provide both a runtime-facing view (`layout_info`, `field_map`)
/// and a higher-level schema manifest for richer tooling.
pub trait SchemaExport: LayoutContract {
    /// Runtime header/layout identity.
    #[inline(always)]
    fn layout_info() -> LayoutInfo {
        <Self as LayoutContract>::layout_info_static()
    }

    /// Field-level wire map used by manager and client tooling.
    #[inline(always)]
    fn field_map() -> &'static [FieldInfo] {
        <Self as LayoutContract>::fields()
    }

    /// Combined runtime metadata payload for manager-facing inspection.
    #[inline(always)]
    fn manager_metadata() -> ManagerMetadata {
        ManagerMetadata {
            layout: Self::layout_info(),
            fields: Self::field_map(),
        }
    }

    /// Combined runtime and manifest metadata payload.
    #[inline(always)]
    fn schema_bundle() -> SchemaBundle {
        SchemaBundle {
            manager: Self::manager_metadata(),
            manifest: Self::layout_manifest(),
        }
    }

    /// Rich schema manifest for diffing, linting, and client generation.
    fn layout_manifest() -> LayoutManifest;
}

/// Bridge from a live `AccountView` to the schema bundle of a concrete layout type.
pub trait AccountSchemaExt {
    /// Return manager metadata if the account header matches `T`.
    fn manager_metadata_for<T: SchemaExport>(&self) -> Option<ManagerMetadata>;

    /// Return the full schema bundle if the account header matches `T`.
    fn schema_bundle_for<T: SchemaExport>(&self) -> Option<SchemaBundle>;
}

impl AccountSchemaExt for AccountView {
    #[inline]
    fn manager_metadata_for<T: SchemaExport>(&self) -> Option<ManagerMetadata> {
        let info = self.layout_info()?;
        if info.matches::<T>() {
            Some(T::manager_metadata())
        } else {
            None
        }
    }

    #[inline]
    fn schema_bundle_for<T: SchemaExport>(&self) -> Option<SchemaBundle> {
        let info = self.layout_info()?;
        if info.matches::<T>() {
            Some(T::schema_bundle())
        } else {
            None
        }
    }
}

// -- Migration Plan Tests --

#[cfg(test)]
mod tests {
    use super::*;

    const V1_FIELDS: &[FieldDescriptor] = &[
        FieldDescriptor { name: "authority", canonical_type: "[u8;32]", size: 32, offset: 16, intent: FieldIntent::Custom },
        FieldDescriptor { name: "balance", canonical_type: "WireU64", size: 8, offset: 48, intent: FieldIntent::Custom },
    ];

    const V2_FIELDS: &[FieldDescriptor] = &[
        FieldDescriptor { name: "authority", canonical_type: "[u8;32]", size: 32, offset: 16, intent: FieldIntent::Custom },
        FieldDescriptor { name: "balance", canonical_type: "WireU64", size: 8, offset: 48, intent: FieldIntent::Custom },
        FieldDescriptor { name: "bump", canonical_type: "u8", size: 1, offset: 56, intent: FieldIntent::Custom },
    ];

    const V1_MANIFEST: LayoutManifest = LayoutManifest {
        name: "Vault",
        disc: 1,
        version: 1,
        layout_id: [1, 2, 3, 4, 5, 6, 7, 8],
        total_size: 56,
        field_count: 2,
        fields: V1_FIELDS,
    };

    const V2_MANIFEST: LayoutManifest = LayoutManifest {
        name: "Vault",
        disc: 1,
        version: 2,
        layout_id: [10, 20, 30, 40, 50, 60, 70, 80],
        total_size: 57,
        field_count: 3,
        fields: V2_FIELDS,
    };

    #[test]
    fn no_op_for_identical() {
        let plan = MigrationPlan::<16>::generate(&V1_MANIFEST, &V1_MANIFEST);
        assert_eq!(plan.policy, MigrationPolicy::NoOp);
        assert_eq!(plan.step_count, 0);
    }

    #[test]
    fn append_only_migration() {
        let plan = MigrationPlan::<16>::generate(&V1_MANIFEST, &V2_MANIFEST);
        assert_eq!(plan.policy, MigrationPolicy::AppendOnly);
        assert!(plan.step_count >= 3); // copy + realloc + zero-init + header
        assert_eq!(plan.old_size, 56);
        assert_eq!(plan.new_size, 57);
        assert!(plan.copy_bytes > 0);
        assert!(plan.zero_bytes > 0);

        // First step should be CopyPrefix
        assert_eq!(plan.steps[0].action, MigrationAction::CopyPrefix);
        // Should have a ZeroInit for the "bump" field
        let mut found_zero = false;
        let mut i = 0;
        while i < plan.step_count {
            if plan.steps[i].action == MigrationAction::ZeroInit {
                assert_eq!(plan.steps[i].field, "bump");
                assert_eq!(plan.steps[i].size, 1);
                found_zero = true;
            }
            i += 1;
        }
        assert!(found_zero);
    }

    #[test]
    fn incompatible_different_disc() {
        let other = LayoutManifest {
            disc: 99,
            ..V2_MANIFEST
        };
        let plan = MigrationPlan::<16>::generate(&V1_MANIFEST, &other);
        assert_eq!(plan.policy, MigrationPolicy::Incompatible);
    }

    #[test]
    fn breaking_change_detected() {
        let changed_fields: &[FieldDescriptor] = &[
            FieldDescriptor { name: "authority", canonical_type: "WireU64", size: 8, offset: 16, intent: FieldIntent::Custom },
            FieldDescriptor { name: "balance", canonical_type: "WireU64", size: 8, offset: 24, intent: FieldIntent::Custom },
        ];
        let breaking = LayoutManifest {
            name: "Vault",
            disc: 1,
            version: 2,
            layout_id: [99; 8],
            total_size: 32,
            field_count: 2,
            fields: changed_fields,
        };
        let plan = MigrationPlan::<16>::generate(&V1_MANIFEST, &breaking);
        assert_eq!(plan.policy, MigrationPolicy::RequiresMigration);
    }

    // -----------------------------------------------------------------------
    // CompatibilityVerdict tests
    // -----------------------------------------------------------------------

    #[test]
    fn verdict_identical() {
        let v = CompatibilityVerdict::between(&V1_MANIFEST, &V1_MANIFEST);
        assert_eq!(v, CompatibilityVerdict::Identical);
        assert!(v.is_safe());
        assert!(v.is_backward_readable());
        assert!(!v.requires_migration());
    }

    #[test]
    fn verdict_append_safe() {
        let v = CompatibilityVerdict::between(&V1_MANIFEST, &V2_MANIFEST);
        assert_eq!(v, CompatibilityVerdict::AppendSafe);
        assert!(v.is_safe());
        assert!(v.is_backward_readable());
        assert!(!v.requires_migration());
    }

    #[test]
    fn verdict_migration_required() {
        let changed_fields: &[FieldDescriptor] = &[
            FieldDescriptor { name: "authority", canonical_type: "WireU64", size: 8, offset: 16, intent: FieldIntent::Custom },
            FieldDescriptor { name: "balance", canonical_type: "WireU64", size: 8, offset: 24, intent: FieldIntent::Custom },
        ];
        let breaking = LayoutManifest {
            name: "Vault",
            disc: 1,
            version: 2,
            layout_id: [99; 8],
            total_size: 32,
            field_count: 2,
            fields: changed_fields,
        };
        let v = CompatibilityVerdict::between(&V1_MANIFEST, &breaking);
        assert_eq!(v, CompatibilityVerdict::MigrationRequired);
        assert!(!v.is_safe());
        assert!(!v.is_backward_readable());
        assert!(v.requires_migration());
    }

    #[test]
    fn verdict_wire_compatible() {
        // Same disc, same fields (count + prefix), same total_size, but different layout_id.
        let semantic_variant = LayoutManifest {
            layout_id: [77; 8], // different layout_id
            ..V1_MANIFEST        // same disc, fields, total_size
        };
        let v = CompatibilityVerdict::between(&V1_MANIFEST, &semantic_variant);
        assert_eq!(v, CompatibilityVerdict::WireCompatible);
        assert!(v.is_safe());
        assert!(v.is_backward_readable());
        assert!(!v.requires_migration());
    }

    #[test]
    fn verdict_incompatible() {
        let other = LayoutManifest { disc: 99, ..V2_MANIFEST };
        let v = CompatibilityVerdict::between(&V1_MANIFEST, &other);
        assert_eq!(v, CompatibilityVerdict::Incompatible);
        assert!(!v.is_safe());
    }

    #[test]
    fn verdict_names() {
        assert_eq!(CompatibilityVerdict::Identical.name(), "identical");
        assert_eq!(CompatibilityVerdict::WireCompatible.name(), "wire-compatible");
        assert_eq!(CompatibilityVerdict::AppendSafe.name(), "append-safe");
        assert_eq!(CompatibilityVerdict::MigrationRequired.name(), "migration-required");
        assert_eq!(CompatibilityVerdict::Incompatible.name(), "incompatible");
    }

    #[test]
    fn segment_advice_core_must_preserve() {
        let segs = [DecodedSegment {
            id: [1, 0, 0, 0],
            offset: 36,
            size: 100,
            flags: 0x0000, // Core = upper 4 bits 0
            version: 1,
        }];
        let report = SegmentMigrationReport::<4>::analyze(&segs, 1);
        assert_eq!(report.count, 1);
        assert_eq!(report.advice[0].role, SegmentRoleHint::Core);
        assert!(report.advice[0].must_preserve);
        assert!(!report.advice[0].clearable);
        assert_eq!(report.preserve_bytes, 100);
    }

    #[test]
    fn segment_advice_journal_clearable() {
        let segs = [DecodedSegment {
            id: [2, 0, 0, 0],
            offset: 136,
            size: 256,
            flags: 0x2000, // Journal = upper 4 bits 2
            version: 1,
        }];
        let report = SegmentMigrationReport::<4>::analyze(&segs, 1);
        assert_eq!(report.advice[0].role, SegmentRoleHint::Journal);
        assert!(report.advice[0].clearable);
        assert!(report.advice[0].append_only);
        assert!(!report.advice[0].must_preserve);
        assert_eq!(report.clearable_bytes, 256);
    }

    #[test]
    fn segment_advice_cache_rebuildable() {
        let segs = [DecodedSegment {
            id: [3, 0, 0, 0],
            offset: 400,
            size: 64,
            flags: 0x4000, // Cache = upper 4 bits 4
            version: 1,
        }];
        let report = SegmentMigrationReport::<4>::analyze(&segs, 1);
        assert_eq!(report.advice[0].role, SegmentRoleHint::Cache);
        assert!(report.advice[0].clearable);
        assert!(report.advice[0].rebuildable);
    }

    #[test]
    fn segment_advice_audit_immutable() {
        let segs = [DecodedSegment {
            id: [4, 0, 0, 0],
            offset: 200,
            size: 32,
            flags: 0x5000, // Audit = upper 4 bits 5
            version: 1,
        }];
        let report = SegmentMigrationReport::<4>::analyze(&segs, 1);
        assert_eq!(report.advice[0].role, SegmentRoleHint::Audit);
        assert!(report.advice[0].must_preserve);
        assert!(report.advice[0].immutable);
        assert!(report.advice[0].append_only);
        assert!(!report.advice[0].clearable);
    }

    #[test]
    fn segment_advice_mixed_report() {
        let segs = [
            DecodedSegment { id: [1, 0, 0, 0], offset: 36, size: 100, flags: 0x0000, version: 1 },
            DecodedSegment { id: [2, 0, 0, 0], offset: 136, size: 200, flags: 0x2000, version: 1 },
            DecodedSegment { id: [3, 0, 0, 0], offset: 336, size: 64, flags: 0x4000, version: 1 },
        ];
        let report = SegmentMigrationReport::<8>::analyze(&segs, 3);
        assert_eq!(report.count, 3);
        assert_eq!(report.must_preserve_count(), 1);
        assert_eq!(report.clearable_count(), 2);
        assert_eq!(report.preserve_bytes, 100);
        assert_eq!(report.clearable_bytes, 264);
        assert_eq!(report.rebuildable_bytes, 64);
    }

    #[test]
    fn segment_role_hint_requires_migration_copy() {
        assert!(SegmentRoleHint::Core.requires_migration_copy());
        assert!(SegmentRoleHint::Audit.requires_migration_copy());
        assert!(!SegmentRoleHint::Extension.requires_migration_copy());
        assert!(!SegmentRoleHint::Journal.requires_migration_copy());
        assert!(!SegmentRoleHint::Index.requires_migration_copy());
        assert!(!SegmentRoleHint::Cache.requires_migration_copy());
        assert!(!SegmentRoleHint::Shard.requires_migration_copy());
    }

    #[test]
    fn segment_role_hint_is_safe_to_drop() {
        assert!(SegmentRoleHint::Cache.is_safe_to_drop());
        assert!(!SegmentRoleHint::Core.is_safe_to_drop());
        assert!(!SegmentRoleHint::Extension.is_safe_to_drop());
        assert!(!SegmentRoleHint::Journal.is_safe_to_drop());
        assert!(!SegmentRoleHint::Index.is_safe_to_drop());
        assert!(!SegmentRoleHint::Audit.is_safe_to_drop());
        assert!(!SegmentRoleHint::Shard.is_safe_to_drop());
    }

    // -----------------------------------------------------------------------
    // Program Manifest tests
    // -----------------------------------------------------------------------

    static PM_LAYOUTS: &[LayoutManifest] = &[
        LayoutManifest {
            name: "Vault",
            disc: 1,
            version: 1,
            layout_id: [1, 2, 3, 4, 5, 6, 7, 8],
            total_size: 57,
            field_count: 0,
            fields: &[],
        },
        LayoutManifest {
            name: "Config",
            disc: 2,
            version: 1,
            layout_id: [8, 7, 6, 5, 4, 3, 2, 1],
            total_size: 43,
            field_count: 0,
            fields: &[],
        },
    ];

    static PM_INSTRUCTIONS: &[InstructionDescriptor] = &[
        InstructionDescriptor {
            name: "deposit",
            tag: 1,
            args: &[],
            accounts: &[],
            capabilities: &["MutatesState"],
            policy_pack: "TREASURY_WRITE",
            receipt_expected: true,
        },
        InstructionDescriptor {
            name: "withdraw",
            tag: 2,
            args: &[],
            accounts: &[],
            capabilities: &["MutatesState", "TransfersTokens"],
            policy_pack: "TREASURY_WRITE",
            receipt_expected: true,
        },
    ];

    static PM_POLICIES: &[PolicyDescriptor] = &[
        PolicyDescriptor {
            name: "TREASURY_WRITE",
            capabilities: &["MutatesState"],
            requirements: &["SignerAuthority"],
            invariants: &[],
            receipt_profile: "default-mutation",
        },
    ];

    #[test]
    fn program_manifest_find_layout_by_disc() {
        let prog = ProgramManifest {
            name: "test",
            version: "0.1.0",
            description: "",
            layouts: PM_LAYOUTS,
            layout_metadata: &[],
            instructions: &[],
            events: &[],
            policies: &[],
            compatibility_pairs: &[],
            tooling_hints: &[],
            contexts: &[],
        };
        assert_eq!(prog.layout_count(), 2);
        assert!(prog.find_layout_by_disc(1).is_some());
        assert_eq!(prog.find_layout_by_disc(1).unwrap().name, "Vault");
        assert!(prog.find_layout_by_disc(2).is_some());
        assert!(prog.find_layout_by_disc(3).is_none());
    }

    #[test]
    fn program_manifest_find_layout_by_id() {
        let prog = ProgramManifest {
            name: "test",
            version: "0.1.0",
            description: "",
            layouts: PM_LAYOUTS,
            layout_metadata: &[],
            instructions: &[],
            events: &[],
            policies: &[],
            compatibility_pairs: &[],
            tooling_hints: &[],
            contexts: &[],
        };
        let id = [1, 2, 3, 4, 5, 6, 7, 8];
        assert!(prog.find_layout_by_id(&id).is_some());
        let bad_id = [0, 0, 0, 0, 0, 0, 0, 0];
        assert!(prog.find_layout_by_id(&bad_id).is_none());
    }

    #[test]
    fn program_manifest_identify_from_data() {
        static ID_LAYOUTS: &[LayoutManifest] = &[
            LayoutManifest {
                name: "Vault",
                disc: 1,
                version: 1,
                layout_id: [0x10, 0x20, 0x30, 0x40, 0x50, 0x60, 0x70, 0x80],
                total_size: 57,
                field_count: 0,
                fields: &[],
            },
        ];
        let prog = ProgramManifest {
            name: "test",
            version: "0.1.0",
            description: "",
            layouts: ID_LAYOUTS,
            layout_metadata: &[],
            instructions: &[],
            events: &[],
            policies: &[],
            compatibility_pairs: &[],
            tooling_hints: &[],
            contexts: &[],
        };
        // Build a 16-byte header: disc=1, version=1, flags=0, layout_id
        let mut data = [0u8; 57];
        data[0] = 1; // disc
        data[1] = 1; // version
        data[4..12].copy_from_slice(&[0x10, 0x20, 0x30, 0x40, 0x50, 0x60, 0x70, 0x80]);
        let result = prog.identify_from_data(&data);
        assert!(result.is_some());
        assert_eq!(result.unwrap().name, "Vault");
    }

    #[test]
    fn program_manifest_find_instruction() {
        let prog = ProgramManifest {
            name: "test",
            version: "0.1.0",
            description: "",
            layouts: &[],
            layout_metadata: &[],
            instructions: PM_INSTRUCTIONS,
            events: &[],
            policies: &[],
            compatibility_pairs: &[],
            tooling_hints: &[],
            contexts: &[],
        };
        assert_eq!(prog.instruction_count(), 2);
        assert_eq!(prog.find_instruction(1).unwrap().name, "deposit");
        assert_eq!(prog.find_instruction(2).unwrap().name, "withdraw");
        assert!(prog.find_instruction(3).is_none());
    }

    #[test]
    fn program_manifest_find_policy() {
        let prog = ProgramManifest {
            name: "test",
            version: "0.1.0",
            description: "",
            layouts: &[],
            layout_metadata: &[],
            instructions: &[],
            events: &[],
            policies: PM_POLICIES,
            compatibility_pairs: &[],
            tooling_hints: &[],
            contexts: &[],
        };
        assert!(prog.find_policy("TREASURY_WRITE").is_some());
        assert!(prog.find_policy("NONEXISTENT").is_none());
    }

    #[test]
    fn decode_account_fields_basic() {
        static DECODE_FIELDS: &[FieldDescriptor] = &[
            FieldDescriptor { name: "balance", canonical_type: "WireU64", size: 8, offset: 16, intent: FieldIntent::Custom },
            FieldDescriptor { name: "bump", canonical_type: "u8", size: 1, offset: 24, intent: FieldIntent::Custom },
        ];
        static DECODE_MANIFEST: LayoutManifest = LayoutManifest {
            name: "Test",
            disc: 1,
            version: 1,
            layout_id: [0; 8],
            total_size: 25,
            field_count: 2,
            fields: DECODE_FIELDS,
        };
        let mut data = [0u8; 25];
        let balance_bytes = 1000u64.to_le_bytes();
        data[16..24].copy_from_slice(&balance_bytes);
        data[24] = 254;

        let (count, decoded) = decode_account_fields::<8>(&data, &DECODE_MANIFEST);
        assert_eq!(count, 2);
        assert!(decoded[0].is_some());
        assert_eq!(decoded[0].as_ref().unwrap().name, "balance");
        assert!(decoded[1].is_some());
        assert_eq!(decoded[1].as_ref().unwrap().name, "bump");
        assert_eq!(decoded[1].as_ref().unwrap().raw[0], 254);
    }

    #[test]
    fn decoded_field_format_wire_u64() {
        let raw = 42u64.to_le_bytes();
        let field = DecodedField {
            name: "balance",
            canonical_type: "WireU64",
            raw: &raw,
            offset: 16,
            size: 8,
        };
        let mut buf = [0u8; 32];
        let len = field.format_value(&mut buf);
        assert_eq!(&buf[..len], b"42");
    }

    #[test]
    fn decoded_field_format_wire_u32() {
        let raw = 65535u32.to_le_bytes();
        let field = DecodedField {
            name: "count",
            canonical_type: "WireU32",
            raw: &raw,
            offset: 0,
            size: 4,
        };
        let mut buf = [0u8; 32];
        let len = field.format_value(&mut buf);
        assert_eq!(&buf[..len], b"65535");
    }

    #[test]
    fn decoded_field_format_bool() {
        let raw_true = [1u8];
        let field = DecodedField {
            name: "frozen",
            canonical_type: "WireBool",
            raw: &raw_true,
            offset: 0,
            size: 1,
        };
        let mut buf = [0u8; 32];
        let len = field.format_value(&mut buf);
        assert_eq!(&buf[..len], b"true");

        let raw_false = [0u8];
        let field2 = DecodedField {
            name: "frozen",
            canonical_type: "WireBool",
            raw: &raw_false,
            offset: 0,
            size: 1,
        };
        let len = field2.format_value(&mut buf);
        assert_eq!(&buf[..len], b"false");
    }

    #[test]
    fn decoded_field_format_address() {
        let raw = [0xABu8; 32];
        let field = DecodedField {
            name: "authority",
            canonical_type: "[u8;32]",
            raw: &raw,
            offset: 0,
            size: 32,
        };
        let mut buf = [0u8; 64];
        let len = field.format_value(&mut buf);
        let s = core::str::from_utf8(&buf[..len]).unwrap();
        assert!(s.starts_with("0x"));
        assert!(s.ends_with("..."));
    }

    #[test]
    fn format_u64_basic() {
        let mut buf = [0u8; 32];
        let len = super::format_u64(12345, &mut buf);
        assert_eq!(&buf[..len], b"12345");

        let len = super::format_u64(0, &mut buf);
        assert_eq!(&buf[..len], b"0");

        let len = super::format_u64(u64::MAX, &mut buf);
        let expected = b"18446744073709551615";
        assert_eq!(&buf[..len], &expected[..]);
    }

    #[test]
    fn format_hex_truncated_short() {
        let mut buf = [0u8; 64];
        let len = super::format_hex_truncated(&[0xAB, 0xCD], &mut buf);
        assert_eq!(&buf[..len], b"0xabcd");
    }

    #[test]
    fn format_hex_truncated_long() {
        let mut buf = [0u8; 64];
        let data = [0xFFu8; 32];
        let len = super::format_hex_truncated(&data, &mut buf);
        let s = core::str::from_utf8(&buf[..len]).unwrap();
        assert!(s.starts_with("0x"));
        assert!(s.ends_with("..."));
        assert_eq!(len, 21); // 0x + 16 hex chars + ...
    }

    #[test]
    fn program_manifest_display() {
        let prog = ProgramManifest {
            name: "test_program",
            version: "0.1.0",
            description: "A test",
            layouts: PM_LAYOUTS,
            layout_metadata: &[],
            instructions: PM_INSTRUCTIONS,
            events: &[],
            policies: PM_POLICIES,
            compatibility_pairs: &[],
            tooling_hints: &[],
            contexts: &[],
        };
        extern crate alloc;
        use alloc::format;
        let s = format!("{}", prog);
        assert!(s.contains("test_program"));
        assert!(s.contains("Vault"));
        assert!(s.contains("deposit"));
        assert!(s.contains("MutatesState"));
        assert!(s.contains("TREASURY_WRITE"));
        assert!(s.contains("SignerAuthority"));
    }

    #[test]
    fn program_manifest_empty() {
        let prog = ProgramManifest::empty();
        assert_eq!(prog.layout_count(), 0);
        assert_eq!(prog.instruction_count(), 0);
        assert!(prog.find_layout_by_disc(0).is_none());
        assert!(prog.find_instruction(0).is_none());
        assert!(prog.identify_from_data(&[0u8; 16]).is_none());
    }

    // -----------------------------------------------------------------------
    // Malformed input torture tests
    // -----------------------------------------------------------------------

    #[test]
    fn decode_header_empty_buffer() {
        assert!(decode_header(&[]).is_none());
    }

    #[test]
    fn decode_header_one_byte() {
        assert!(decode_header(&[0xFF]).is_none());
    }

    #[test]
    fn decode_header_fifteen_bytes() {
        assert!(decode_header(&[0u8; 15]).is_none());
    }

    #[test]
    fn decode_header_exact_sixteen() {
        let h = decode_header(&[0u8; 16]);
        assert!(h.is_some());
        let h = h.unwrap();
        assert_eq!(h.disc, 0);
        assert_eq!(h.version, 0);
    }

    #[test]
    fn decode_header_large_buffer() {
        let data = [0xABu8; 1024];
        let h = decode_header(&data).unwrap();
        assert_eq!(h.disc, 0xAB);
        assert_eq!(h.version, 0xAB);
    }

    #[test]
    fn decode_segments_too_short() {
        // Needs header (16) + registry header (4) minimum
        assert!(decode_segments::<8>(&[0u8; 19]).is_none());
    }

    #[test]
    fn decode_segments_zero_count() {
        // 16 header + 4 registry header with count=0
        let mut data = [0u8; 20];
        data[16] = 0; // count low byte
        data[17] = 0; // count high byte
        let result = decode_segments::<8>(&data);
        assert!(result.is_some());
        let (n, _) = result.unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn compare_fields_identical_empty() {
        let a = LayoutManifest { name: "A", disc: 1, version: 1, layout_id: [0; 8], total_size: 16, field_count: 0, fields: &[] };
        let b = LayoutManifest { name: "B", disc: 1, version: 1, layout_id: [0; 8], total_size: 16, field_count: 0, fields: &[] };
        let report = compare_fields::<8>(&a, &b);
        assert_eq!(report.count, 0);
        assert!(report.is_append_safe);
    }

    static SINGLE_FIELD: &[FieldDescriptor] = &[
        FieldDescriptor { name: "x", canonical_type: "u8", size: 1, offset: 16, intent: FieldIntent::Custom },
    ];

    #[test]
    fn compare_fields_all_removed() {
        let a = LayoutManifest { name: "A", disc: 1, version: 1, layout_id: [1; 8], total_size: 17, field_count: 1, fields: SINGLE_FIELD };
        let b = LayoutManifest { name: "B", disc: 1, version: 2, layout_id: [2; 8], total_size: 16, field_count: 0, fields: &[] };
        let report = compare_fields::<8>(&a, &b);
        assert_eq!(report.count, 1);
        assert!(!report.is_append_safe);
    }

    static OLD_TYPE_FIELD: &[FieldDescriptor] = &[
        FieldDescriptor { name: "x", canonical_type: "u8", size: 1, offset: 16, intent: FieldIntent::Custom },
    ];
    static NEW_TYPE_FIELD: &[FieldDescriptor] = &[
        FieldDescriptor { name: "x", canonical_type: "u16", size: 2, offset: 16, intent: FieldIntent::Custom },
    ];

    #[test]
    fn compare_fields_type_change_detected() {
        let a = LayoutManifest { name: "A", disc: 1, version: 1, layout_id: [1; 8], total_size: 17, field_count: 1, fields: OLD_TYPE_FIELD };
        let b = LayoutManifest { name: "B", disc: 1, version: 2, layout_id: [2; 8], total_size: 18, field_count: 1, fields: NEW_TYPE_FIELD };
        let report = compare_fields::<8>(&a, &b);
        assert_eq!(report.entries[0].status, FieldCompat::Changed);
        assert!(!report.is_append_safe);
    }

    #[test]
    fn verdict_different_disc_is_incompatible() {
        let a = LayoutManifest { name: "A", disc: 1, version: 1, layout_id: [1; 8], total_size: 16, field_count: 0, fields: &[] };
        let b = LayoutManifest { name: "B", disc: 2, version: 1, layout_id: [2; 8], total_size: 16, field_count: 0, fields: &[] };
        assert_eq!(CompatibilityVerdict::between(&a, &b), CompatibilityVerdict::Incompatible);
    }

    #[test]
    fn verdict_same_id_is_identical() {
        let a = LayoutManifest { name: "A", disc: 1, version: 1, layout_id: [9; 8], total_size: 16, field_count: 0, fields: &[] };
        assert_eq!(CompatibilityVerdict::between(&a, &a), CompatibilityVerdict::Identical);
    }

    #[test]
    fn compatibility_explain_between_identical() {
        let a = LayoutManifest { name: "A", disc: 1, version: 1, layout_id: [9; 8], total_size: 16, field_count: 0, fields: &[] };
        let exp = CompatibilityExplain::between(&a, &a);
        assert_eq!(exp.verdict, CompatibilityVerdict::Identical);
        assert_eq!(exp.added_count, 0);
        assert_eq!(exp.removed_count, 0);
        assert!(!exp.semantic_drift);
    }

    static APPEND_OLD: &[FieldDescriptor] = &[
        FieldDescriptor { name: "a", canonical_type: "u8", size: 1, offset: 16, intent: FieldIntent::Custom },
    ];
    static APPEND_NEW: &[FieldDescriptor] = &[
        FieldDescriptor { name: "a", canonical_type: "u8", size: 1, offset: 16, intent: FieldIntent::Custom },
        FieldDescriptor { name: "b", canonical_type: "u8", size: 1, offset: 17, intent: FieldIntent::Custom },
    ];

    #[test]
    fn compatibility_explain_append_counts_fields() {
        let older = LayoutManifest { name: "T", disc: 1, version: 1, layout_id: [1; 8], total_size: 17, field_count: 1, fields: APPEND_OLD };
        let newer = LayoutManifest { name: "T", disc: 1, version: 2, layout_id: [2; 8], total_size: 18, field_count: 2, fields: APPEND_NEW };
        let exp = CompatibilityExplain::between(&older, &newer);
        assert_eq!(exp.verdict, CompatibilityVerdict::AppendSafe);
        assert_eq!(exp.added_count, 1);
        assert_eq!(exp.added_fields[0], "b");
    }

    #[test]
    fn layout_fingerprint_deterministic() {
        let m = LayoutManifest { name: "X", disc: 1, version: 1, layout_id: [5; 8], total_size: 16, field_count: 0, fields: &[] };
        let fp1 = LayoutFingerprint::from_manifest(&m);
        let fp2 = LayoutFingerprint::from_manifest(&m);
        assert_eq!(fp1.wire_hash, fp2.wire_hash);
        assert_eq!(fp1.semantic_hash, fp2.semantic_hash);
    }

    static FP_CUSTOM: &[FieldDescriptor] = &[
        FieldDescriptor { name: "x", canonical_type: "u8", size: 1, offset: 16, intent: FieldIntent::Custom },
    ];
    static FP_BALANCE: &[FieldDescriptor] = &[
        FieldDescriptor { name: "x", canonical_type: "u8", size: 1, offset: 16, intent: FieldIntent::Balance },
    ];

    #[test]
    fn layout_fingerprint_differs_on_intent_change() {
        let m1 = LayoutManifest { name: "T", disc: 1, version: 1, layout_id: [1; 8], total_size: 17, field_count: 1, fields: FP_CUSTOM };
        let m2 = LayoutManifest { name: "T", disc: 1, version: 1, layout_id: [1; 8], total_size: 17, field_count: 1, fields: FP_BALANCE };
        let fp1 = LayoutFingerprint::from_manifest(&m1);
        let fp2 = LayoutFingerprint::from_manifest(&m2);
        assert_eq!(fp1.wire_hash, fp2.wire_hash);
        assert_ne!(fp1.semantic_hash, fp2.semantic_hash);
    }

    static LINT_AUTH_FIELD: &[FieldDescriptor] = &[
        FieldDescriptor { name: "auth", canonical_type: "[u8;32]", size: 32, offset: 16, intent: FieldIntent::Authority },
    ];

    #[test]
    fn lint_layout_authority_without_signer() {
        let m = LayoutManifest { name: "T", disc: 1, version: 1, layout_id: [0; 8], total_size: 48, field_count: 1, fields: LINT_AUTH_FIELD };
        // Use a mutating behavior WITHOUT signer to trigger E001
        let behavior = LayoutBehavior { requires_signer: false, affects_balance: false, affects_authority: true, mutation_class: MutationClass::InPlace };
        let (n, lints) = lint_layout::<8>(&m, &behavior);
        assert!(n >= 1);
        assert_eq!(lints[0].code, "E001");
    }

    #[test]
    fn lint_layout_clean_passes() {
        let m = LayoutManifest { name: "T", disc: 1, version: 1, layout_id: [0; 8], total_size: 48, field_count: 1, fields: LINT_AUTH_FIELD };
        let behavior = LayoutBehavior { requires_signer: true, affects_balance: false, affects_authority: true, mutation_class: MutationClass::AuthoritySensitive };
        let (n, _) = lint_layout::<8>(&m, &behavior);
        assert_eq!(n, 0);
    }

    #[test]
    fn mutation_class_properties() {
        assert!(!MutationClass::ReadOnly.is_mutating());
        assert!(MutationClass::InPlace.is_mutating());
        assert!(MutationClass::Financial.requires_snapshot());
        assert!(MutationClass::AuthoritySensitive.requires_authority());
        assert!(!MutationClass::AppendOnly.requires_authority());
    }

    static SEED_FIELD: &[FieldDescriptor] = &[
        FieldDescriptor { name: "seed", canonical_type: "[u8;32]", size: 32, offset: 16, intent: FieldIntent::PDASeed },
    ];

    #[test]
    fn layout_stability_grade_stable_with_init_only() {
        let m = LayoutManifest { name: "T", disc: 1, version: 1, layout_id: [0; 8], total_size: 48, field_count: 1, fields: SEED_FIELD };
        assert_eq!(LayoutStabilityGrade::compute(&m), LayoutStabilityGrade::Stable);
    }

    #[test]
    fn layout_stability_grade_evolving_with_custom() {
        let m = LayoutManifest { name: "T", disc: 1, version: 1, layout_id: [0; 8], total_size: 17, field_count: 1, fields: SINGLE_FIELD };
        assert_eq!(LayoutStabilityGrade::compute(&m), LayoutStabilityGrade::Evolving);
    }

    static GRADE_HEAVY: &[FieldDescriptor] = &[
        FieldDescriptor { name: "auth1", canonical_type: "[u8;32]", size: 32, offset: 16, intent: FieldIntent::Authority },
        FieldDescriptor { name: "auth2", canonical_type: "[u8;32]", size: 32, offset: 48, intent: FieldIntent::Owner },
        FieldDescriptor { name: "auth3", canonical_type: "[u8;32]", size: 32, offset: 80, intent: FieldIntent::Delegate },
        FieldDescriptor { name: "bal1", canonical_type: "WireU64", size: 8, offset: 112, intent: FieldIntent::Balance },
        FieldDescriptor { name: "bal2", canonical_type: "WireU64", size: 8, offset: 120, intent: FieldIntent::Supply },
        FieldDescriptor { name: "bal3", canonical_type: "WireU64", size: 8, offset: 128, intent: FieldIntent::Balance },
    ];

    #[test]
    fn layout_stability_grade_unsafe_to_evolve_heavy() {
        let m = LayoutManifest { name: "T", disc: 1, version: 1, layout_id: [0; 8], total_size: 136, field_count: 6, fields: GRADE_HEAVY };
        let grade = LayoutStabilityGrade::compute(&m);
        assert_eq!(grade, LayoutStabilityGrade::UnsafeToEvolve);
    }

    #[test]
    fn field_intent_new_variants_coverage() {
        assert_eq!(FieldIntent::PDASeed.name(), "pda_seed");
        assert_eq!(FieldIntent::Version.name(), "version");
        assert_eq!(FieldIntent::Bump.name(), "bump");
        assert_eq!(FieldIntent::Status.name(), "status");
        assert!(FieldIntent::Owner.is_authority_sensitive());
        assert!(FieldIntent::Delegate.is_authority_sensitive());
        assert!(FieldIntent::Threshold.is_governance());
        assert!(FieldIntent::Bump.is_init_only());
        assert!(FieldIntent::PDASeed.is_init_only());
        assert!(FieldIntent::Supply.is_monetary());
    }

    #[test]
    fn refine_verdict_softens_with_rebuildable_segments() {
        let advice = [
            SegmentAdvice {
                id: [0; 4], size: 100, role: SegmentRoleHint::Cache,
                must_preserve: false, clearable: true, rebuildable: true,
                append_only: false, immutable: false,
            },
            SegmentAdvice {
                id: [0; 4], size: 0, role: SegmentRoleHint::Unclassified,
                must_preserve: false, clearable: false, rebuildable: false,
                append_only: false, immutable: false,
            },
        ];
        let report = SegmentMigrationReport {
            advice,
            count: 1,
            preserve_bytes: 0,
            clearable_bytes: 100,
            rebuildable_bytes: 100,
        };
        let refined = CompatibilityVerdict::MigrationRequired.refine_with_roles(&report);
        assert_eq!(refined, CompatibilityVerdict::AppendSafe);
    }

    #[test]
    fn refine_verdict_escalates_with_immutable_segment() {
        let advice = [SegmentAdvice {
            id: [0; 4], size: 50, role: SegmentRoleHint::Audit,
            must_preserve: true, clearable: false, rebuildable: false,
            append_only: true, immutable: true,
        }];
        let report = SegmentMigrationReport {
            advice,
            count: 1,
            preserve_bytes: 50,
            clearable_bytes: 0,
            rebuildable_bytes: 0,
        };
        let refined = CompatibilityVerdict::AppendSafe.refine_with_roles(&report);
        assert_eq!(refined, CompatibilityVerdict::MigrationRequired);
    }

    #[test]
    fn lint_policy_financial_mismatch() {
        let behavior = LayoutBehavior {
            requires_signer: true,
            affects_balance: true,
            affects_authority: false,
            mutation_class: MutationClass::Financial,
        };
        let (n, lints) = lint_policy::<8>(&behavior, PolicyClass::Write);
        assert!(n >= 1);
        assert_eq!(lints[0].code, "W005");
    }

    #[test]
    fn lint_policy_reverse_mismatch() {
        let behavior = LayoutBehavior {
            requires_signer: true,
            affects_balance: false,
            affects_authority: false,
            mutation_class: MutationClass::InPlace,
        };
        let (n, lints) = lint_policy::<8>(&behavior, PolicyClass::Financial);
        assert!(n >= 1);
        assert_eq!(lints[0].code, "W006");
    }

    #[test]
    fn lint_policy_clean_when_aligned() {
        let behavior = LayoutBehavior {
            requires_signer: true,
            affects_balance: true,
            affects_authority: false,
            mutation_class: MutationClass::Financial,
        };
        let (n, _) = lint_policy::<8>(&behavior, PolicyClass::Financial);
        assert_eq!(n, 0);
    }

    #[test]
    fn display_field_intent() {
        extern crate alloc;
        use alloc::format;
        assert_eq!(format!("{}", FieldIntent::Balance), "balance");
        assert_eq!(format!("{}", FieldIntent::Authority), "authority");
    }

    #[test]
    fn display_mutation_class() {
        extern crate alloc;
        use alloc::format;
        assert_eq!(format!("{}", MutationClass::Financial), "financial");
        assert_eq!(format!("{}", MutationClass::ReadOnly), "read-only");
    }

    #[test]
    fn display_layout_stability_grade() {
        extern crate alloc;
        use alloc::format;
        assert_eq!(format!("{}", LayoutStabilityGrade::Stable), "stable");
        assert_eq!(format!("{}", LayoutStabilityGrade::UnsafeToEvolve), "unsafe-to-evolve");
    }

    #[test]
    fn display_compatibility_verdict() {
        extern crate alloc;
        use alloc::format;
        assert_eq!(format!("{}", CompatibilityVerdict::Identical), "identical");
        assert_eq!(format!("{}", CompatibilityVerdict::MigrationRequired), "migration-required");
    }

    #[test]
    fn display_layout_fingerprint() {
        extern crate alloc;
        use alloc::format;
        let fp = LayoutFingerprint { wire_hash: [0xAB, 0xCD, 0, 0, 0, 0, 0, 0], semantic_hash: [0, 0, 0, 0, 0, 0, 0xFF, 0x01] };
        let s = format!("{}", fp);
        assert!(s.starts_with("wire=abcd"));
        assert!(s.contains("sem="));
        assert!(s.ends_with("ff01"));
    }

    #[test]
    fn display_receipt_profile() {
        extern crate alloc;
        use alloc::format;
        let rp = ReceiptProfile {
            name: "test",
            expected_phase: "Mutate",
            expects_balance_change: true,
            expects_authority_change: false,
            expects_journal_append: false,
            min_changed_fields: 2,
        };
        let s = format!("{}", rp);
        assert!(s.contains("test"));
        assert!(s.contains("Mutate"));
        assert!(s.contains("balance"));
        assert!(s.contains("min_fields=2"));
    }

    #[test]
    fn display_idl_segment_descriptor() {
        extern crate alloc;
        use alloc::format;
        let sd = IdlSegmentDescriptor {
            name: "core",
            role: "Core",
            append_only: false,
            rebuildable: false,
            must_preserve: true,
        };
        let s = format!("{}", sd);
        assert!(s.contains("core"));
        assert!(s.contains("Core"));
        assert!(s.contains("must-preserve"));
        assert!(!s.contains("append-only"));
    }
}
