//! State Receipts -- structured mutation summaries.
//!
//! A `StateReceipt` captures a complete record of what happened during
//! an instruction's execution: which fields changed, what the before/after
//! fingerprints were, which invariants ran, which capabilities were active,
//! and how many CPI calls or journal appends occurred.
//!
//! ## Use Cases
//!
//! - **Audit trails**: Emit receipts as events for off-chain indexing
//! - **Test assertions**: Verify exact mutation footprint in tests
//! - **Post-mutation validation**: Feed receipt to invariant checks
//! - **Debugging**: Log receipts during development
//! - **CLI inspection**: Decode receipt bytes with `hopper receipt`
//!
//! ## Usage
//!
//! ```ignore
//! // Before mutation
//! let mut receipt = StateReceipt::<8>::begin(
//!     &layout_id,
//!     account_data,
//! );
//!
//! // ... mutations happen ...
//!
//! // After mutation
//! receipt.commit(account_data);
//! receipt.set_invariants(true, 3);
//! receipt.set_policy_flags(DEPOSIT_CAPS.bits());
//! receipt.set_cpi_count(1);
//! receipt.set_journal_appends(2);
//!
//! // Emit as event
//! emit_slices(&[&receipt.to_bytes()]);
//! ```

use crate::diff::StateSnapshot;

/// Maximum fields tracked in a receipt's changed-field bitmask.
pub const MAX_RECEIPT_FIELDS: usize = 64;

/// FNV-1a 64-bit fingerprint of a byte slice.
///
/// Not cryptographic. Used for fast before/after change detection.
#[inline]
fn fnv1a_fingerprint(data: &[u8]) -> [u8; 8] {
    let mut hash: u64 = 0xcbf29ce484222325;
    for &b in data {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash.to_le_bytes()
}

/// A structured record of a state mutation.
///
/// `SNAP_SIZE` is the maximum snapshot size (stack-allocated).
pub struct StateReceipt<const SNAP_SIZE: usize> {
    /// Layout ID of the account being mutated.
    pub layout_id: [u8; 8],
    /// Before-snapshot.
    snapshot: StateSnapshot<SNAP_SIZE>,
    /// Field-level change bitmask (bit N = field N changed).
    pub changed_fields: u64,
    /// Number of bytes that changed.
    pub changed_bytes: usize,
    /// Number of changed regions (contiguous runs).
    pub changed_regions: usize,
    /// Whether the account was resized.
    pub was_resized: bool,
    /// Old account data length.
    pub old_size: usize,
    /// New account data length.
    pub new_size: usize,
    /// Whether all invariants passed after mutation.
    pub invariants_passed: bool,
    /// Number of invariants checked.
    pub invariants_checked: u16,
    /// Whether CPI was invoked during the instruction.
    pub cpi_invoked: bool,
    /// Whether the receipt has been committed (post-mutation data provided).
    committed: bool,
    /// FNV-1a fingerprint of the data before mutation.
    pub before_fingerprint: [u8; 8],
    /// FNV-1a fingerprint of the data after mutation (set on commit).
    pub after_fingerprint: [u8; 8],
    /// Bitmask of which segments were touched (bit N = segment N changed).
    pub segment_changed_mask: u16,
    /// CapabilitySet bits describing what this instruction does.
    pub policy_flags: u32,
    /// Number of journal entries appended during this instruction.
    pub journal_appends: u16,
    /// Number of CPI calls made during this instruction.
    pub cpi_count: u8,
    /// Instruction phase tag (see [`Phase`]).
    pub phase: u8,
    /// Validation bundle identifier (program-defined).
    pub validation_bundle_id: u16,
    /// Compatibility impact of this mutation (see [`CompatImpact`]).
    pub compat_impact: u8,
    /// Migration flags (bit 0 = triggered, bit 1 = realloc, bit 2 = schema bump).
    pub migration_flags: u8,
}

/// Instruction execution phase encoded in a receipt.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Normal update / mutation.
    Update = 0,
    /// Account initialization.
    Init = 1,
    /// Account close / deletion.
    Close = 2,
    /// Migration to a new layout version.
    Migrate = 3,
    /// Read-only / view (no mutation expected).
    ReadOnly = 4,
}

impl Phase {
    /// Convert from raw tag.
    #[inline(always)]
    pub fn from_tag(tag: u8) -> Self {
        match tag {
            1 => Self::Init,
            2 => Self::Close,
            3 => Self::Migrate,
            4 => Self::ReadOnly,
            _ => Self::Update,
        }
    }

    /// Human-readable name.
    #[inline(always)]
    pub fn name(self) -> &'static str {
        match self {
            Self::Update => "Update",
            Self::Init => "Init",
            Self::Close => "Close",
            Self::Migrate => "Migrate",
            Self::ReadOnly => "ReadOnly",
        }
    }
}

/// Compatibility impact level encoded in a receipt.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompatImpact {
    /// No compatibility impact.
    None = 0,
    /// Append-only growth — backward readable.
    Append = 1,
    /// Full migration required.
    Migration = 2,
    /// Breaking change.
    Breaking = 3,
}

impl CompatImpact {
    /// Convert from raw tag.
    #[inline(always)]
    pub fn from_tag(tag: u8) -> Self {
        match tag {
            1 => Self::Append,
            2 => Self::Migration,
            3 => Self::Breaking,
            _ => Self::None,
        }
    }

    /// Human-readable name.
    #[inline(always)]
    pub fn name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Append => "append",
            Self::Migration => "migration",
            Self::Breaking => "breaking",
        }
    }
}

impl<const SNAP_SIZE: usize> StateReceipt<SNAP_SIZE> {
    /// Begin recording a state receipt.
    ///
    /// Captures a before-snapshot and fingerprint of the account data.
    #[inline]
    pub fn begin(layout_id: &[u8; 8], data: &[u8]) -> Self {
        Self {
            layout_id: *layout_id,
            snapshot: StateSnapshot::capture(data),
            changed_fields: 0,
            changed_bytes: 0,
            changed_regions: 0,
            was_resized: false,
            old_size: data.len(),
            new_size: data.len(),
            invariants_passed: false,
            invariants_checked: 0,
            cpi_invoked: false,
            committed: false,
            before_fingerprint: fnv1a_fingerprint(data),
            after_fingerprint: [0; 8],
            segment_changed_mask: 0,
            policy_flags: 0,
            journal_appends: 0,
            cpi_count: 0,
            phase: Phase::Update as u8,
            validation_bundle_id: 0,
            compat_impact: CompatImpact::None as u8,
            migration_flags: 0,
        }
    }

    /// Commit the receipt by providing post-mutation data.
    ///
    /// Computes the diff and after-fingerprint.
    #[inline]
    pub fn commit(&mut self, current_data: &[u8]) {
        let diff = self.snapshot.diff(current_data);
        self.changed_bytes = diff.changed_byte_count();
        self.was_resized = diff.was_resized();
        self.new_size = current_data.len();

        let regions = diff.changed_regions::<16>();
        self.changed_regions = regions.len();

        self.after_fingerprint = fnv1a_fingerprint(current_data);
        self.committed = true;
    }

    /// Commit with field-level tracking.
    ///
    /// `fields` is `(name, offset, size)` per layout field.
    /// Sets the `changed_fields` bitmask based on which fields actually changed.
    #[inline]
    pub fn commit_with_fields(
        &mut self,
        current_data: &[u8],
        fields: &[(&str, usize, usize)],
    ) {
        self.commit(current_data);
        self.changed_fields = crate::diff::field_diff_mask(
            self.snapshot.data(),
            current_data,
            fields,
        );
    }

    /// Commit with segment-level tracking.
    ///
    /// `segments` is `(offset, size)` per segment in the account.
    /// Sets `segment_changed_mask` based on which segments have byte changes.
    #[inline]
    pub fn commit_with_segments(
        &mut self,
        current_data: &[u8],
        segments: &[(usize, usize)],
    ) {
        self.commit(current_data);
        let snap_data = self.snapshot.data();
        let mut mask: u16 = 0;
        let compare_len = if snap_data.len() < current_data.len() {
            snap_data.len()
        } else {
            current_data.len()
        };
        for (i, &(offset, size)) in segments.iter().enumerate() {
            if i >= 16 {
                break;
            }
            let end = offset + size;
            if end <= compare_len {
                if snap_data[offset..end] != current_data[offset..end] {
                    mask |= 1 << i;
                }
            } else if offset < compare_len {
                // Partial overlap: segment extends beyond one of the buffers
                mask |= 1 << i;
            } else if self.was_resized {
                // Segment entirely in new region
                mask |= 1 << i;
            }
        }
        self.segment_changed_mask = mask;
    }

    /// Set invariant results.
    #[inline(always)]
    pub fn set_invariants(&mut self, passed: bool, checked: u16) {
        self.invariants_passed = passed;
        self.invariants_checked = checked;
    }

    /// Set invariant pass status (convenience).
    #[inline(always)]
    pub fn set_invariants_passed(&mut self, passed: bool) {
        self.invariants_passed = passed;
    }

    /// Mark that CPI was invoked during this instruction.
    #[inline(always)]
    pub fn set_cpi_invoked(&mut self, invoked: bool) {
        self.cpi_invoked = invoked;
    }

    /// Set the number of CPI calls made. Also sets `cpi_invoked` if count > 0.
    #[inline(always)]
    pub fn set_cpi_count(&mut self, count: u8) {
        self.cpi_count = count;
        self.cpi_invoked = count > 0;
    }

    /// Set the policy/capability flags for this instruction.
    ///
    /// Pass `CapabilitySet::bits()` to record which capabilities were active.
    #[inline(always)]
    pub fn set_policy_flags(&mut self, flags: u32) {
        self.policy_flags = flags;
    }

    /// Set the number of journal entries appended during this instruction.
    #[inline(always)]
    pub fn set_journal_appends(&mut self, count: u16) {
        self.journal_appends = count;
    }

    /// Set the instruction phase.
    #[inline(always)]
    pub fn set_phase(&mut self, phase: Phase) {
        self.phase = phase as u8;
    }

    /// Set the validation bundle identifier.
    #[inline(always)]
    pub fn set_validation_bundle_id(&mut self, id: u16) {
        self.validation_bundle_id = id;
    }

    /// Set the compatibility impact level.
    #[inline(always)]
    pub fn set_compat_impact(&mut self, impact: CompatImpact) {
        self.compat_impact = impact as u8;
    }

    /// Set migration flags (bit 0 = triggered, bit 1 = realloc, bit 2 = schema bump).
    #[inline(always)]
    pub fn set_migration_flags(&mut self, flags: u8) {
        self.migration_flags = flags;
    }

    /// Whether the receipt has been committed.
    #[inline(always)]
    pub fn is_committed(&self) -> bool {
        self.committed
    }

    /// Whether any data actually changed.
    #[inline(always)]
    pub fn has_changes(&self) -> bool {
        self.changed_bytes > 0 || self.was_resized
    }

    /// Whether the before and after fingerprints differ.
    #[inline(always)]
    pub fn fingerprint_changed(&self) -> bool {
        self.before_fingerprint != self.after_fingerprint
    }

    /// Serialize the receipt summary into a fixed-size byte array.
    ///
    /// Wire format (64 bytes):
    /// ```text
    /// [layout_id: 8 bytes]                  //  0.. 8
    /// [changed_fields: 8 bytes (u64 LE)]    //  8..16
    /// [changed_bytes: 4 bytes (u32 LE)]     // 16..20
    /// [changed_regions: 2 bytes (u16 LE)]   // 20..22
    /// [old_size: 4 bytes (u32 LE)]          // 22..26
    /// [new_size: 4 bytes (u32 LE)]          // 26..30
    /// [invariants_checked: 2 bytes (u16 LE)]// 30..32
    /// [flags: 1 byte]                       // 32
    ///   bit 0: was_resized
    ///   bit 1: invariants_passed
    ///   bit 2: cpi_invoked
    ///   bit 3: committed
    /// [before_fingerprint: 8 bytes]         // 33..41
    /// [after_fingerprint: 8 bytes]          // 41..49
    /// [segment_changed_mask: 2 bytes (u16)] // 49..51
    /// [policy_flags: 4 bytes (u32 LE)]      // 51..55
    /// [journal_appends: 2 bytes (u16 LE)]   // 55..57
    /// [cpi_count: 1 byte]                   // 57
    /// [phase: 1 byte]                         // 58
    /// [validation_bundle_id: 2 bytes (u16)]   // 59..61
    /// [compat_impact: 1 byte]                 // 61
    /// [migration_flags: 1 byte]               // 62
    /// [_reserved: 1 byte]                     // 63
    /// ```
    #[inline]
    pub fn to_bytes(&self) -> [u8; RECEIPT_SIZE] {
        let mut out = [0u8; RECEIPT_SIZE];
        // layout_id
        out[0..8].copy_from_slice(&self.layout_id);
        // changed_fields
        out[8..16].copy_from_slice(&self.changed_fields.to_le_bytes());
        // changed_bytes
        out[16..20].copy_from_slice(&(self.changed_bytes as u32).to_le_bytes());
        // changed_regions
        out[20..22].copy_from_slice(&(self.changed_regions as u16).to_le_bytes());
        // old_size
        out[22..26].copy_from_slice(&(self.old_size as u32).to_le_bytes());
        // new_size
        out[26..30].copy_from_slice(&(self.new_size as u32).to_le_bytes());
        // invariants_checked
        out[30..32].copy_from_slice(&self.invariants_checked.to_le_bytes());
        // flags
        let mut flags: u8 = 0;
        if self.was_resized { flags |= 1 << 0; }
        if self.invariants_passed { flags |= 1 << 1; }
        if self.cpi_invoked { flags |= 1 << 2; }
        if self.committed { flags |= 1 << 3; }
        out[32] = flags;
        // before_fingerprint
        out[33..41].copy_from_slice(&self.before_fingerprint);
        // after_fingerprint
        out[41..49].copy_from_slice(&self.after_fingerprint);
        // segment_changed_mask
        out[49..51].copy_from_slice(&self.segment_changed_mask.to_le_bytes());
        // policy_flags
        out[51..55].copy_from_slice(&self.policy_flags.to_le_bytes());
        // journal_appends
        out[55..57].copy_from_slice(&self.journal_appends.to_le_bytes());
        // cpi_count
        out[57] = self.cpi_count;
        // phase
        out[58] = self.phase;
        // validation_bundle_id
        out[59..61].copy_from_slice(&self.validation_bundle_id.to_le_bytes());
        // compat_impact
        out[61] = self.compat_impact;
        // migration_flags
        out[62] = self.migration_flags;
        out
    }
}

/// Receipt summary size in bytes.
pub const RECEIPT_SIZE: usize = 64;

/// Decoded receipt from wire bytes. Useful for CLI and off-chain tooling.
pub struct DecodedReceipt {
    pub layout_id: [u8; 8],
    pub changed_fields: u64,
    pub changed_bytes: u32,
    pub changed_regions: u16,
    pub old_size: u32,
    pub new_size: u32,
    pub invariants_checked: u16,
    pub was_resized: bool,
    pub invariants_passed: bool,
    pub cpi_invoked: bool,
    pub committed: bool,
    pub before_fingerprint: [u8; 8],
    pub after_fingerprint: [u8; 8],
    pub segment_changed_mask: u16,
    pub policy_flags: u32,
    pub journal_appends: u16,
    pub cpi_count: u8,
    pub phase: u8,
    pub validation_bundle_id: u16,
    pub compat_impact: u8,
    pub migration_flags: u8,
}

impl DecodedReceipt {
    /// Decode a receipt from its 64-byte wire representation.
    ///
    /// Returns `None` if the slice is too short.
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < RECEIPT_SIZE {
            return None;
        }
        let mut layout_id = [0u8; 8];
        layout_id.copy_from_slice(&bytes[0..8]);
        let changed_fields = u64::from_le_bytes([
            bytes[8], bytes[9], bytes[10], bytes[11],
            bytes[12], bytes[13], bytes[14], bytes[15],
        ]);
        let changed_bytes = u32::from_le_bytes([
            bytes[16], bytes[17], bytes[18], bytes[19],
        ]);
        let changed_regions = u16::from_le_bytes([bytes[20], bytes[21]]);
        let old_size = u32::from_le_bytes([
            bytes[22], bytes[23], bytes[24], bytes[25],
        ]);
        let new_size = u32::from_le_bytes([
            bytes[26], bytes[27], bytes[28], bytes[29],
        ]);
        let invariants_checked = u16::from_le_bytes([bytes[30], bytes[31]]);
        let flags = bytes[32];
        let was_resized = flags & (1 << 0) != 0;
        let invariants_passed = flags & (1 << 1) != 0;
        let cpi_invoked = flags & (1 << 2) != 0;
        let committed = flags & (1 << 3) != 0;

        let mut before_fingerprint = [0u8; 8];
        before_fingerprint.copy_from_slice(&bytes[33..41]);
        let mut after_fingerprint = [0u8; 8];
        after_fingerprint.copy_from_slice(&bytes[41..49]);
        let segment_changed_mask = u16::from_le_bytes([bytes[49], bytes[50]]);
        let policy_flags = u32::from_le_bytes([
            bytes[51], bytes[52], bytes[53], bytes[54],
        ]);
        let journal_appends = u16::from_le_bytes([bytes[55], bytes[56]]);
        let cpi_count = bytes[57];
        let phase = bytes[58];
        let validation_bundle_id = u16::from_le_bytes([bytes[59], bytes[60]]);
        let compat_impact = bytes[61];
        let migration_flags = bytes[62];

        Some(Self {
            layout_id,
            changed_fields,
            changed_bytes,
            changed_regions,
            old_size,
            new_size,
            invariants_checked,
            was_resized,
            invariants_passed,
            cpi_invoked,
            committed,
            before_fingerprint,
            after_fingerprint,
            segment_changed_mask,
            policy_flags,
            journal_appends,
            cpi_count,
            phase,
            validation_bundle_id,
            compat_impact,
            migration_flags,
        })
    }

    /// Whether data actually changed according to this receipt.
    #[inline(always)]
    pub fn has_changes(&self) -> bool {
        self.changed_bytes > 0 || self.was_resized
    }

    /// Whether before/after fingerprints differ.
    #[inline(always)]
    pub fn fingerprint_changed(&self) -> bool {
        self.before_fingerprint != self.after_fingerprint
    }

    /// Resolve the `phase` byte to a [`Phase`] enum.
    #[inline(always)]
    pub fn phase_enum(&self) -> Phase {
        Phase::from_tag(self.phase)
    }

    /// Resolve the `compat_impact` byte to a [`CompatImpact`] enum.
    #[inline(always)]
    pub fn compat_impact_enum(&self) -> CompatImpact {
        CompatImpact::from_tag(self.compat_impact)
    }

    /// Return a structured human-readable explanation of this receipt.
    ///
    /// This is the "operator UX" layer—every numeric field gets a semantic
    /// label so tools, dashboards, and CLI output can show meaningful text
    /// instead of raw bytes.
    pub fn explain(&self) -> ReceiptExplain {
        let phase = self.phase_enum();
        let compat = self.compat_impact_enum();

        let mutation_desc = if !self.has_changes() {
            "No mutations detected"
        } else if self.was_resized {
            "Account was resized"
        } else {
            "Account data modified in-place"
        };

        let integrity_desc = if !self.committed {
            "Receipt was NOT committed (incomplete)"
        } else if self.invariants_passed && self.invariants_checked > 0 {
            "All invariants passed"
        } else if self.invariants_checked > 0 {
            "INVARIANT VIOLATION detected"
        } else {
            "No invariants checked"
        };

        let cpi_desc = if self.cpi_invoked {
            "CPI was invoked during execution"
        } else {
            "No CPI calls"
        };

        ReceiptExplain {
            phase_name: phase.name(),
            compat_label: compat.name(),
            policy_name: "unknown",
            mutation_summary: mutation_desc,
            integrity_summary: integrity_desc,
            cpi_summary: cpi_desc,
            changed_field_count: self.changed_fields.count_ones() as u16,
            segment_count: self.segment_changed_mask.count_ones() as u8,
            fingerprint_changed: self.fingerprint_changed(),
            segment_role_names: [""; 8],
            segment_role_count: 0,
        }
    }
}

/// Human-readable explanation of a decoded receipt.
///
/// Produced by [`DecodedReceipt::explain()`]. Every field is a semantic
/// label, not a raw number—designed for operator dashboards, CLI output,
/// and audit logs.
pub struct ReceiptExplain {
    /// Phase name ("Update", "Init", "Close", "Migrate", "ReadOnly").
    pub phase_name: &'static str,
    /// Compatibility impact label ("None", "Append", "Migration", "Breaking").
    pub compat_label: &'static str,
    /// Policy pack name that governed this instruction ("unknown" when not embedded).
    pub policy_name: &'static str,
    /// One-sentence description of what mutation occurred.
    pub mutation_summary: &'static str,
    /// One-sentence description of invariant check result.
    pub integrity_summary: &'static str,
    /// One-sentence CPI summary.
    pub cpi_summary: &'static str,
    /// Number of individual fields changed (popcount of changed_fields mask).
    pub changed_field_count: u16,
    /// Number of segments that were modified.
    pub segment_count: u8,
    /// Whether the before/after fingerprints differ.
    pub fingerprint_changed: bool,
    /// Role names for modified segments (up to 8). Use `segment_role_count`
    /// to know how many entries are valid.
    pub segment_role_names: [&'static str; 8],
    /// Number of valid entries in `segment_role_names`.
    pub segment_role_count: u8,
}

impl ReceiptExplain {
    /// Return a copy with the given policy name injected.
    ///
    /// The receipt wire format does not carry the policy pack name—only a
    /// bitmask of flags. Call this after constructing an explain from the
    /// decoded receipt when you know which policy pack governed the
    /// instruction (e.g. from the program manifest).
    #[inline]
    pub const fn with_policy_name(mut self, name: &'static str) -> Self {
        self.policy_name = name;
        self
    }

    /// Inject a segment role name at the given index.
    ///
    /// Call once per modified segment, using the `SegmentRole::name()`
    /// output for each bit set in `segment_changed_mask`. This enriches
    /// the explain with human-readable role labels.
    #[inline]
    pub const fn with_segment_role(mut self, idx: u8, name: &'static str) -> Self {
        if (idx as usize) < 8 {
            self.segment_role_names[idx as usize] = name;
            if idx >= self.segment_role_count {
                self.segment_role_count = idx + 1;
            }
        }
        self
    }

    /// One-line human-readable summary combining phase, mutation,
    /// and integrity status.
    #[inline]
    pub const fn summary(&self) -> &'static str {
        // Phase + mutation + integrity condensed into a single static label.
        // Because we're no_std we return the most descriptive static string
        // based on the phase and mutation state.
        if !crate::const_str_eq(self.phase_name, "Update")
            && !crate::const_str_eq(self.phase_name, "Init")
            && !crate::const_str_eq(self.phase_name, "Close")
            && !crate::const_str_eq(self.phase_name, "Migrate")
        {
            return "Read-only operation, no state changes";
        }
        if crate::const_str_eq(self.phase_name, "Init") {
            return "Account initialized";
        }
        if crate::const_str_eq(self.phase_name, "Close") {
            return "Account closed";
        }
        if crate::const_str_eq(self.phase_name, "Migrate") {
            if self.fingerprint_changed {
                return "Migration applied, layout fingerprint updated";
            }
            return "Migration applied";
        }
        // Update phase — provide more detail based on what changed
        if !self.fingerprint_changed && self.changed_field_count == 0 {
            return "Update executed with no observable state changes";
        }
        if self.fingerprint_changed && self.changed_field_count > 0 && self.segment_count > 1 {
            return "State mutated across multiple segments with fingerprint change";
        }
        if self.fingerprint_changed && self.changed_field_count > 0 {
            return "State mutated with fingerprint change";
        }
        if self.fingerprint_changed {
            return "Fingerprint changed without field-level mutations";
        }
        if self.changed_field_count > 0 && self.segment_count > 1 {
            return "State mutated across multiple segments";
        }
        if self.changed_field_count > 0 {
            return "State mutated";
        }
        "Update completed"
    }
}

// ---------------------------------------------------------------------------
// Receipt Narrative -- auto-generated human explanations
// ---------------------------------------------------------------------------

/// An auto-generated human-readable narrative describing a mutation.
///
/// Built from the receipt explain plus optional field intents, policy class,
/// and segment roles. This is the "operator artifact" layer that turns
/// binary receipt data into sentences an operator can actually read.
pub struct ReceiptNarrative {
    /// Sentence fragments describing what happened. Up to 8 fragments.
    pub fragments: [&'static str; 8],
    /// Number of valid fragments.
    pub count: u8,
    /// Overall risk level of this mutation.
    pub risk_level: NarrativeRisk,
}

/// Risk level for a receipt narrative.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum NarrativeRisk {
    /// No observable state changes.
    None = 0,
    /// Standard mutation, nothing unusual.
    Low = 1,
    /// Mutation touches authority or financial fields.
    Medium = 2,
    /// Migration, resize, or integrity violation detected.
    High = 3,
    /// Invariant failure or uncommitted receipt.
    Critical = 4,
}

impl NarrativeRisk {
    /// Human-readable label.
    pub const fn name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Critical => "critical",
        }
    }
}

impl core::fmt::Display for NarrativeRisk {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.name())
    }
}

impl ReceiptNarrative {
    /// Generate a narrative from a receipt explanation.
    ///
    /// Produces a sequence of human-readable fragments describing the
    /// mutation, along with a risk assessment.
    pub fn from_explain(explain: &ReceiptExplain) -> Self {
        let mut frags: [&'static str; 8] = [""; 8];
        let mut n = 0u8;
        let mut risk = NarrativeRisk::None;

        // Phase description
        let phase_frag = match explain.phase_name {
            "Init" => "Account was initialized.",
            "Close" => "Account was closed.",
            "Migrate" => "Migration was applied to the account.",
            "ReadOnly" => "Read-only operation executed.",
            _ => "State mutation executed.",
        };
        if n < 8 { frags[n as usize] = phase_frag; n += 1; }

        // Mutation details
        if explain.changed_field_count > 0 {
            risk = NarrativeRisk::Low;
            if explain.segment_count > 1 {
                if n < 8 { frags[n as usize] = "Changes span multiple segments."; n += 1; }
            }
        }

        // Fingerprint change
        if explain.fingerprint_changed {
            if n < 8 { frags[n as usize] = "Layout fingerprint changed."; n += 1; }
            if risk as u8 == NarrativeRisk::Low as u8 {
                risk = NarrativeRisk::Medium;
            }
        }

        // Compatibility impact
        match explain.compat_label {
            "Append" => {
                if n < 8 { frags[n as usize] = "Append-safe extension applied."; n += 1; }
            }
            "Migration" => {
                if n < 8 { frags[n as usize] = "Migration-level change detected."; n += 1; }
                risk = NarrativeRisk::High;
            }
            "Breaking" => {
                if n < 8 { frags[n as usize] = "Breaking compatibility change."; n += 1; }
                risk = NarrativeRisk::High;
            }
            _ => {}
        }

        // CPI
        if !crate::const_str_eq(explain.cpi_summary, "No CPI calls") {
            if n < 8 { frags[n as usize] = "Cross-program invocation occurred."; n += 1; }
        }

        // Integrity
        if crate::const_str_eq(explain.integrity_summary, "INVARIANT VIOLATION detected") {
            if n < 8 { frags[n as usize] = "INVARIANT VIOLATION: post-mutation checks failed."; n += 1; }
            risk = NarrativeRisk::Critical;
        }
        if crate::const_str_eq(explain.integrity_summary, "Receipt was NOT committed (incomplete)") {
            if n < 8 { frags[n as usize] = "Receipt was not committed. Mutation may be incomplete."; n += 1; }
            risk = NarrativeRisk::Critical;
        }

        // Segment roles
        if explain.segment_role_count > 0 {
            let mut i = 0u8;
            while i < explain.segment_role_count && i < 8 {
                let role = explain.segment_role_names[i as usize];
                if crate::const_str_eq(role, "audit") || crate::const_str_eq(role, "Audit") {
                    if n < 8 { frags[n as usize] = "Audit segment was touched."; n += 1; }
                }
                i += 1;
            }
        }

        // Phase-level risk escalation
        if crate::const_str_eq(explain.phase_name, "Migrate") {
            if (risk as u8) < NarrativeRisk::High as u8 {
                risk = NarrativeRisk::High;
            }
        }

        Self {
            fragments: frags,
            count: n,
            risk_level: risk,
        }
    }
}

impl core::fmt::Display for ReceiptNarrative {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let mut i = 0u8;
        while i < self.count {
            if i > 0 {
                write!(f, " ")?;
            }
            write!(f, "{}", self.fragments[i as usize])?;
            i += 1;
        }
        write!(f, " [risk: {}]", self.risk_level.name())
    }
}
