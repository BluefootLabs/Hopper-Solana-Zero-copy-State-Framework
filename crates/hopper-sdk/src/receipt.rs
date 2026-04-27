//! # Receipt decoder
//!
//! The Hopper receipt is a **fixed 72-byte wire format** that the program
//! emits at the end of a mutating instruction. The exact offsets are
//! authoritative in `hopper-core::receipt::StateReceipt::to_bytes`; this
//! module mirrors that layout bit-for-bit so off-chain consumers can
//! decode receipts without linking the on-chain crate.
//!
//! A 64-byte legacy receipt (pre-0.2) is accepted for backwards
//! compatibility; the failure-payload fields are then populated with
//! defaults (no failure recorded).
//!
//! ## Wire layout (authoritative)
//!
//! | off | sz | field                  | type    |
//! |-----|----|------------------------|---------|
//! |   0 |  8 | layout_id              | [u8;8]  |
//! |   8 |  8 | changed_fields         | u64 LE  |
//! |  16 |  4 | changed_bytes          | u32 LE  |
//! |  20 |  2 | changed_regions        | u16 LE  |
//! |  22 |  4 | old_size               | u32 LE  |
//! |  26 |  4 | new_size               | u32 LE  |
//! |  30 |  2 | invariants_checked     | u16 LE  |
//! |  32 |  1 | flags                  | bitfield|
//! |  33 |  8 | before_fingerprint     | [u8;8]  |
//! |  41 |  8 | after_fingerprint      | [u8;8]  |
//! |  49 |  2 | segment_changed_mask   | u16 LE  |
//! |  51 |  4 | policy_flags           | u32 LE  |
//! |  55 |  2 | journal_appends        | u16 LE  |
//! |  57 |  1 | cpi_count              | u8      |
//! |  58 |  1 | phase                  | u8      |
//! |  59 |  2 | validation_bundle_id   | u16 LE  |
//! |  61 |  1 | compat_impact          | u8      |
//! |  62 |  1 | migration_flags        | u8      |
//! |  63 |  1 | failed_invariant_idx   | u8      |
//! |  64 |  4 | failed_error_code      | u32 LE  |
//! |  68 |  1 | failure_stage          | u8      |
//! |  69 |  3 | reserved               | zero    |
//!
//! Flags byte:
//! - bit 0: was_resized
//! - bit 1: invariants_passed
//! - bit 2: cpi_invoked
//! - bit 3: committed
//! - bit 4: had_failure

/// Fixed byte length of a Hopper receipt on the wire.
pub const RECEIPT_SIZE: usize = 72;

/// Legacy receipt byte length (pre-0.2). Accepted at parse time with the
/// failure payload defaulted to "no failure recorded".
pub const RECEIPT_SIZE_LEGACY: usize = 64;

/// Sentinel value for `failed_invariant_idx` meaning "no invariant was
/// associated with the failure".
pub const FAILED_INVARIANT_NONE: u8 = 0xFF;

/// Receipt parse error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceiptError {
    /// Input was shorter than `RECEIPT_SIZE_LEGACY` bytes.
    TooShort {
        /// Actual input length.
        got: usize,
    },
    /// Reserved trailing region was non-zero. likely corrupt or stale.
    ReservedNonZero,
    /// `phase` byte is outside the documented enum range (0..=4).
    InvalidPhase(u8),
    /// `compat_impact` byte is outside the documented enum range (0..=3).
    InvalidCompatImpact(u8),
    /// `failure_stage` byte is outside the documented enum range (0..=5).
    InvalidFailureStage(u8),
}

/// Execution phase a receipt was captured in.
///
/// Mirrors `hopper-core::receipt::Phase` exactly so consumers never
/// need to link on-chain crates just to read a receipt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
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
    fn from_u8(v: u8) -> Option<Self> {
        Some(match v {
            0 => Phase::Update,
            1 => Phase::Init,
            2 => Phase::Close,
            3 => Phase::Migrate,
            4 => Phase::ReadOnly,
            _ => return None,
        })
    }

    /// Short human-readable name.
    pub const fn name(self) -> &'static str {
        match self {
            Phase::Update => "update",
            Phase::Init => "init",
            Phase::Close => "close",
            Phase::Migrate => "migrate",
            Phase::ReadOnly => "readonly",
        }
    }
}

/// Compatibility impact class of the mutation carried by this receipt.
/// Mirrors `hopper-core::receipt::CompatImpact`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum CompatImpact {
    /// No wire-level change; readers at the prior layout still work.
    None = 0,
    /// Append-only change; readers ignoring new fields still work.
    Append = 1,
    /// Full migration required.
    Migration = 2,
    /// Breaking change.
    Breaking = 3,
}

impl CompatImpact {
    fn from_u8(v: u8) -> Option<Self> {
        Some(match v {
            0 => CompatImpact::None,
            1 => CompatImpact::Append,
            2 => CompatImpact::Migration,
            3 => CompatImpact::Breaking,
            _ => return None,
        })
    }

    /// Short human-readable name.
    pub const fn name(self) -> &'static str {
        match self {
            CompatImpact::None => "none",
            CompatImpact::Append => "append",
            CompatImpact::Migration => "migration",
            CompatImpact::Breaking => "breaking",
        }
    }
}

/// Stage at which a failure was recorded on a receipt.
///
/// Mirrors `hopper-core::receipt::FailureStage`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum FailureStage {
    /// No failure (receipt committed cleanly).
    None = 0,
    /// Failed during account/context validation (pre-handler).
    Validation = 1,
    /// Failed inside the instruction handler before any invariant.
    Handler = 2,
    /// Failed inside an invariant check.
    Invariant = 3,
    /// Failed during the post-handler receipt commit/emit path.
    Post = 4,
    /// Failed inside a close guard / teardown routine.
    Teardown = 5,
}

impl FailureStage {
    fn from_u8(v: u8) -> Option<Self> {
        Some(match v {
            0 => FailureStage::None,
            1 => FailureStage::Validation,
            2 => FailureStage::Handler,
            3 => FailureStage::Invariant,
            4 => FailureStage::Post,
            5 => FailureStage::Teardown,
            _ => return None,
        })
    }

    /// Short human-readable name.
    pub const fn name(self) -> &'static str {
        match self {
            FailureStage::None => "none",
            FailureStage::Validation => "validation",
            FailureStage::Handler => "handler",
            FailureStage::Invariant => "invariant",
            FailureStage::Post => "post",
            FailureStage::Teardown => "teardown",
        }
    }
}

/// Raw wire receipt buffer.
///
/// Stores at least the legacy 64-byte receipt; the extra 8 bytes of the
/// 0.2+ format live in the tail. This is primarily useful when the
/// consumer wants to treat the receipt as an opaque blob for storage.
#[derive(Debug, Clone, Copy)]
pub struct ReceiptWire(pub [u8; RECEIPT_SIZE]);

impl ReceiptWire {
    /// Copy the first `RECEIPT_SIZE` bytes of `buf` into a new `ReceiptWire`.
    ///
    /// If `buf` is only `RECEIPT_SIZE_LEGACY` bytes, the tail is zero-filled
    /// (meaning "no failure recorded").
    pub fn from_slice(buf: &[u8]) -> Result<Self, ReceiptError> {
        if buf.len() < RECEIPT_SIZE_LEGACY {
            return Err(ReceiptError::TooShort { got: buf.len() });
        }
        let mut bytes = [0u8; RECEIPT_SIZE];
        let n = core::cmp::min(buf.len(), RECEIPT_SIZE);
        bytes[..n].copy_from_slice(&buf[..n]);
        Ok(Self(bytes))
    }
}

/// A fully decoded receipt in host-endian Rust types. Use this in indexers,
/// receipt explorers, and receipt-aware UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecodedReceipt {
    /// Layout identifier of the account this receipt was produced for.
    pub layout_id: [u8; 8],
    /// Bitmask of field indices that changed. Up to 64 fields.
    pub changed_fields: u64,
    /// Total changed bytes.
    pub changed_bytes: u32,
    /// Number of disjoint changed regions.
    pub changed_regions: u16,
    /// Size before mutation.
    pub old_size: u32,
    /// Size after mutation.
    pub new_size: u32,
    /// Number of invariants evaluated.
    pub invariants_checked: u16,
    /// Whether the account was reallocated.
    pub was_resized: bool,
    /// Whether all invariants passed.
    pub invariants_passed: bool,
    /// Whether a CPI was invoked during this frame.
    pub cpi_invoked: bool,
    /// Whether the frame was committed (`false` = rolled back / dry run).
    pub committed: bool,
    /// Whether a failure was recorded (populates `failed_*` fields).
    pub had_failure: bool,
    /// Fingerprint of the pre-mutation state (8 bytes, mixer-derived).
    pub before_fingerprint: [u8; 8],
    /// Fingerprint of the post-mutation state.
    pub after_fingerprint: [u8; 8],
    /// Bitmask of segment indices touched (up to 16).
    pub segment_changed_mask: u16,
    /// Policy flags bitmask.
    pub policy_flags: u32,
    /// Number of journal entries appended.
    pub journal_appends: u16,
    /// Count of CPIs.
    pub cpi_count: u8,
    /// Execution phase at which the receipt was sealed.
    pub phase: Phase,
    /// Identifier of the validation bundle used.
    pub validation_bundle_id: u16,
    /// Compatibility class of the mutation.
    pub compat_impact: CompatImpact,
    /// Bitmask of migration-related flags.
    pub migration_flags: u8,
    /// Invariant index for the failure (`FAILED_INVARIANT_NONE` when none).
    pub failed_invariant_idx: u8,
    /// User error code for the failing check (`0` when none).
    pub failed_error_code: u32,
    /// Stage at which the failure occurred.
    pub failure_stage: FailureStage,
}

impl DecodedReceipt {
    /// Parse a 72-byte wire receipt.
    ///
    /// Accepts a 64-byte legacy receipt as a fallback: in that case the
    /// failure-payload fields default to "no failure recorded".
    pub fn parse(buf: &[u8]) -> Result<Self, ReceiptError> {
        if buf.len() < RECEIPT_SIZE_LEGACY {
            return Err(ReceiptError::TooShort { got: buf.len() });
        }

        let mut layout_id = [0u8; 8];
        layout_id.copy_from_slice(&buf[0..8]);

        let changed_fields = u64::from_le_bytes([
            buf[8], buf[9], buf[10], buf[11], buf[12], buf[13], buf[14], buf[15],
        ]);
        let changed_bytes = u32::from_le_bytes([buf[16], buf[17], buf[18], buf[19]]);
        let changed_regions = u16::from_le_bytes([buf[20], buf[21]]);
        let old_size = u32::from_le_bytes([buf[22], buf[23], buf[24], buf[25]]);
        let new_size = u32::from_le_bytes([buf[26], buf[27], buf[28], buf[29]]);
        let invariants_checked = u16::from_le_bytes([buf[30], buf[31]]);

        let flags = buf[32];
        let was_resized = flags & (1 << 0) != 0;
        let invariants_passed = flags & (1 << 1) != 0;
        let cpi_invoked = flags & (1 << 2) != 0;
        let committed = flags & (1 << 3) != 0;
        let had_failure = flags & (1 << 4) != 0;

        let mut before_fingerprint = [0u8; 8];
        before_fingerprint.copy_from_slice(&buf[33..41]);
        let mut after_fingerprint = [0u8; 8];
        after_fingerprint.copy_from_slice(&buf[41..49]);

        let segment_changed_mask = u16::from_le_bytes([buf[49], buf[50]]);
        let policy_flags = u32::from_le_bytes([buf[51], buf[52], buf[53], buf[54]]);
        let journal_appends = u16::from_le_bytes([buf[55], buf[56]]);
        let cpi_count = buf[57];
        let phase = Phase::from_u8(buf[58]).ok_or(ReceiptError::InvalidPhase(buf[58]))?;
        let validation_bundle_id = u16::from_le_bytes([buf[59], buf[60]]);
        let compat_impact =
            CompatImpact::from_u8(buf[61]).ok_or(ReceiptError::InvalidCompatImpact(buf[61]))?;
        let migration_flags = buf[62];

        // Failure payload. When the caller only has a legacy 64-byte
        // receipt, default everything to "no failure" rather than fail
        // the parse. old producers never emitted this slot.
        let (failed_invariant_idx, failed_error_code, failure_stage) = if buf.len() >= RECEIPT_SIZE
        {
            // Reserved bytes (69..72) must be zero; producers always
            // zero-pad. A non-zero byte here signals wire drift and
            // should surface to the caller.
            let mut i = 69usize;
            while i < RECEIPT_SIZE {
                if buf[i] != 0 {
                    return Err(ReceiptError::ReservedNonZero);
                }
                i += 1;
            }
            let idx = buf[63];
            let code = u32::from_le_bytes([buf[64], buf[65], buf[66], buf[67]]);
            let stage =
                FailureStage::from_u8(buf[68]).ok_or(ReceiptError::InvalidFailureStage(buf[68]))?;
            (idx, code, stage)
        } else {
            (FAILED_INVARIANT_NONE, 0u32, FailureStage::None)
        };

        Ok(Self {
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
            had_failure,
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
            failed_invariant_idx,
            failed_error_code,
            failure_stage,
        })
    }

    /// Iterate the indices of fields that changed.
    pub fn changed_field_indices(&self) -> ChangedFieldIter {
        ChangedFieldIter {
            mask: self.changed_fields,
            idx: 0,
        }
    }

    /// Iterate the indices of segments that were touched.
    pub fn changed_segment_indices(&self) -> ChangedSegmentIter {
        ChangedSegmentIter {
            mask: self.segment_changed_mask,
            idx: 0,
        }
    }

    /// Whether any state was actually modified.
    pub const fn is_mutation(&self) -> bool {
        self.committed && (self.changed_bytes > 0 || self.was_resized)
    }

    /// Whether this receipt is safe to treat as a *read-through* receipt.
    pub const fn is_readonly(&self) -> bool {
        self.committed
            && !self.was_resized
            && self.changed_bytes == 0
            && !self.cpi_invoked
            && self.journal_appends == 0
    }

    /// Size delta in bytes (post minus pre).
    pub const fn size_delta(&self) -> i64 {
        (self.new_size as i64) - (self.old_size as i64)
    }
}

/// Iterator over indices of fields that changed according to the receipt.
pub struct ChangedFieldIter {
    mask: u64,
    idx: u32,
}

impl Iterator for ChangedFieldIter {
    type Item = u32;
    fn next(&mut self) -> Option<u32> {
        while self.idx < 64 {
            let cur = self.idx;
            let bit = 1u64 << cur;
            self.idx += 1;
            if self.mask & bit != 0 {
                return Some(cur);
            }
        }
        None
    }
}

/// Iterator over indices of segments that changed.
pub struct ChangedSegmentIter {
    mask: u16,
    idx: u32,
}

impl Iterator for ChangedSegmentIter {
    type Item = u32;
    fn next(&mut self) -> Option<u32> {
        while self.idx < 16 {
            let cur = self.idx;
            let bit = 1u16 << cur;
            self.idx += 1;
            if self.mask & bit != 0 {
                return Some(cur);
            }
        }
        None
    }
}

#[cfg(feature = "narrate")]
pub mod narrative {
    //! Human-readable receipt narration.
    //!
    //! Turns a `DecodedReceipt` plus its matching `LayoutManifest` and
    //! optional `ErrorRegistry` into a sentence an indexer or UI can
    //! display without needing to know Solana or Hopper semantics.
    //!
    //! **The invariant→name lookup is the payoff of the provable-safety
    //! chain.** When the receipt reports `had_failure=true` with a
    //! populated `failed_error_code`, the narrator cross-references the
    //! program's `ErrorRegistry` to render:
    //!
    //! ```text
    //! Execution aborted at invariant stage: Invariant `balance_nonzero` failed (code 0x1001).
    //! ```
    //!
    //! without requiring any per-program hand-written mapping code.

    use super::{DecodedReceipt, FailureStage};
    use alloc::string::{String, ToString};
    use alloc::vec::Vec;
    use hopper_schema::{ErrorRegistry, LayoutManifest};

    /// Structured narrative ready for rendering.
    #[derive(Debug, Clone)]
    pub struct ReceiptNarrative {
        /// Root sentence.
        pub summary: String,
        /// Per-field change lines.
        pub field_changes: Vec<String>,
        /// Flags (resized, CPI, journal, migration).
        pub flags: Vec<String>,
        /// Severity bucket: "info" | "notice" | "warn" | "error".
        pub severity: &'static str,
        /// If the receipt carries a failure, the rendered "Invariant X
        /// failed" sentence the operator should see first.
        pub failure_line: Option<String>,
    }

    /// Convert a decoded receipt into a narrative using optional layout
    /// and error registries. Without them, indices and raw codes are used.
    pub struct Narrator<'a> {
        /// Optional layout manifest. If provided, field names replace indices.
        pub layout: Option<&'a LayoutManifest>,
        /// Optional error registry. If provided, failing codes are
        /// rendered as "Invariant `x` failed" instead of "code 0xNNNN".
        pub errors: Option<&'a ErrorRegistry>,
    }

    impl<'a> Narrator<'a> {
        /// Build a narrator with only a layout manifest.
        pub const fn with_layout(layout: &'a LayoutManifest) -> Self {
            Self {
                layout: Some(layout),
                errors: None,
            }
        }

        /// Build a narrator with both a layout and error registry.
        pub const fn with_all(layout: &'a LayoutManifest, errors: &'a ErrorRegistry) -> Self {
            Self {
                layout: Some(layout),
                errors: Some(errors),
            }
        }

        /// Build a `ReceiptNarrative` from a decoded receipt.
        pub fn narrate(&self, r: &DecodedReceipt) -> ReceiptNarrative {
            // Render failure first because it dominates the story.
            let failure_line = if r.had_failure {
                Some(render_failure(r, self.errors))
            } else {
                None
            };

            let mut field_changes = Vec::new();
            for idx in r.changed_field_indices() {
                let name = self
                    .layout
                    .and_then(|m| m.fields.get(idx as usize))
                    .map(|f| f.name.to_string())
                    .unwrap_or_else(|| format!("field[{}]", idx));
                field_changes.push(name);
            }

            let mut flags = Vec::new();
            if r.was_resized {
                flags.push(format!(
                    "resized {} → {} bytes (Δ {})",
                    r.old_size,
                    r.new_size,
                    r.size_delta()
                ));
            }
            if r.cpi_invoked {
                flags.push(format!("invoked {} CPI(s)", r.cpi_count));
            }
            if r.journal_appends > 0 {
                flags.push(format!("appended {} journal entr(ies)", r.journal_appends));
            }
            if r.migration_flags != 0 {
                flags.push(format!("migration flags = 0x{:02x}", r.migration_flags));
            }

            let (summary, severity) = summarize(r, &field_changes, failure_line.as_deref());

            ReceiptNarrative {
                summary,
                field_changes,
                flags,
                severity,
                failure_line,
            }
        }
    }

    /// Format the failure line for a receipt that records one.
    ///
    /// Uses the registry to promote raw error codes to invariant names
    /// when possible. Falls back to "error code 0xNNNN" otherwise.
    fn render_failure(r: &DecodedReceipt, errors: Option<&ErrorRegistry>) -> String {
        let stage_label = r.failure_stage.name();
        // Prefer invariant name via registry lookup.
        if let Some(reg) = errors {
            if let Some(desc) = reg.find_by_code(r.failed_error_code) {
                if !desc.invariant.is_empty() {
                    return format!(
                        "Execution aborted at {} stage: invariant `{}` failed \
                         ({}::{} = 0x{:x}).",
                        stage_label, desc.invariant, reg.enum_name, desc.name, desc.code,
                    );
                }
                return format!(
                    "Execution aborted at {} stage: {}::{} (code 0x{:x}).",
                    stage_label, reg.enum_name, desc.name, desc.code,
                );
            }
        }
        // No registry available. render raw code.
        if r.failure_stage == FailureStage::Invariant
            && r.failed_invariant_idx != super::FAILED_INVARIANT_NONE
        {
            format!(
                "Execution aborted at invariant stage: invariant #{} failed (code 0x{:x}).",
                r.failed_invariant_idx, r.failed_error_code
            )
        } else {
            format!(
                "Execution aborted at {} stage: error code 0x{:x}.",
                stage_label, r.failed_error_code
            )
        }
    }

    fn summarize(
        r: &DecodedReceipt,
        changed: &[String],
        failure_line: Option<&str>,
    ) -> (String, &'static str) {
        if let Some(line) = failure_line {
            return (line.to_string(), "error");
        }
        if !r.committed {
            return (
                format!(
                    "Frame rolled back in phase '{}' (invariants {}/{}).",
                    r.phase.name(),
                    if r.invariants_passed {
                        "passed"
                    } else {
                        "failed"
                    },
                    r.invariants_checked
                ),
                "warn",
            );
        }
        if r.is_readonly() {
            return (
                format!(
                    "Read-through committed at phase '{}'; no state mutated.",
                    r.phase.name()
                ),
                "info",
            );
        }
        let names = if changed.is_empty() {
            "no named fields".to_string()
        } else if changed.len() <= 3 {
            changed.join(", ")
        } else {
            format!("{} and {} more", changed[..3].join(", "), changed.len() - 3)
        };
        let severity = if r.compat_impact as u8 >= super::CompatImpact::Migration as u8 {
            "warn"
        } else if r.compat_impact as u8 >= super::CompatImpact::Append as u8 {
            "notice"
        } else {
            "info"
        };
        (
            format!(
                "Committed at phase '{}': mutated {} ({} byte{}, {} region{}), compat={}.",
                r.phase.name(),
                names,
                r.changed_bytes,
                if r.changed_bytes == 1 { "" } else { "s" },
                r.changed_regions,
                if r.changed_regions == 1 { "" } else { "s" },
                r.compat_impact.name(),
            ),
            severity,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_wire() -> [u8; RECEIPT_SIZE] {
        let mut b = [0u8; RECEIPT_SIZE];
        b[0..8].copy_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8]); // layout_id
        b[8..16].copy_from_slice(&(0b1011u64).to_le_bytes()); // changed_fields
        b[16..20].copy_from_slice(&16u32.to_le_bytes()); // changed_bytes
        b[20..22].copy_from_slice(&2u16.to_le_bytes()); // changed_regions
        b[22..26].copy_from_slice(&128u32.to_le_bytes()); // old_size
        b[26..30].copy_from_slice(&128u32.to_le_bytes()); // new_size
        b[30..32].copy_from_slice(&3u16.to_le_bytes()); // invariants_checked
                                                        // flags: invariants_passed | committed
        b[32] = (1 << 1) | (1 << 3);
        b[33..41].copy_from_slice(&[0xAA, 0xBB, 0xCC, 0xDD, 0x00, 0x00, 0x00, 0x00]);
        b[41..49].copy_from_slice(&[0x11, 0x22, 0x33, 0x44, 0x00, 0x00, 0x00, 0x00]);
        b[49..51].copy_from_slice(&0b10u16.to_le_bytes()); // seg mask
        b[51..55].copy_from_slice(&0x42u32.to_le_bytes()); // policy_flags
        b[55..57].copy_from_slice(&0u16.to_le_bytes()); // journal_appends
        b[57] = 0; // cpi_count
        b[58] = 0; // phase = Update
        b[59..61].copy_from_slice(&7u16.to_le_bytes()); // validation_bundle_id
        b[61] = 0; // compat_impact = None (Breaking not used here)
        b[62] = 0; // migration_flags
        b[63] = FAILED_INVARIANT_NONE; // no invariant failure
        b[64..68].copy_from_slice(&0u32.to_le_bytes()); // failed_error_code
        b[68] = 0; // failure_stage = None
                   // 69..72 reserved (zero)
        b
    }

    #[test]
    fn parses_valid_wire() {
        let wire = sample_wire();
        let r = DecodedReceipt::parse(&wire).expect("should parse");
        assert_eq!(r.phase, Phase::Update);
        assert!(r.committed);
        assert!(r.invariants_passed);
        assert_eq!(r.changed_fields, 0b1011);
        assert_eq!(r.changed_bytes, 16);
        assert_eq!(r.compat_impact, CompatImpact::None);
        assert_eq!(r.validation_bundle_id, 7);
        assert!(!r.had_failure);
        assert_eq!(r.failed_error_code, 0);
        assert_eq!(r.failed_invariant_idx, FAILED_INVARIANT_NONE);
        assert!(!r.is_readonly());
        // changed_bytes=16 + committed → receipt represents a real mutation.
        assert!(r.is_mutation());
    }

    #[test]
    fn rejects_short() {
        let buf = [0u8; 32];
        assert!(matches!(
            DecodedReceipt::parse(&buf),
            Err(ReceiptError::TooShort { got: 32 })
        ));
    }

    #[test]
    fn accepts_legacy_64_byte_receipt() {
        let wire = sample_wire();
        let legacy = &wire[..RECEIPT_SIZE_LEGACY];
        let r = DecodedReceipt::parse(legacy).expect("should parse legacy");
        assert!(!r.had_failure);
        assert_eq!(r.failed_invariant_idx, FAILED_INVARIANT_NONE);
        assert_eq!(r.failed_error_code, 0);
        assert_eq!(r.failure_stage, FailureStage::None);
    }

    #[test]
    fn decodes_invariant_failure() {
        let mut wire = sample_wire();
        // Clear invariants_passed, set had_failure.
        wire[32] = (1 << 3) | (1 << 4); // committed | had_failure
        wire[63] = 0x02; // invariant idx 2
        wire[64..68].copy_from_slice(&0x1001u32.to_le_bytes()); // code
        wire[68] = 3; // FailureStage::Invariant
        let r = DecodedReceipt::parse(&wire).expect("should parse failure");
        assert!(r.had_failure);
        assert!(!r.invariants_passed);
        assert_eq!(r.failed_invariant_idx, 0x02);
        assert_eq!(r.failed_error_code, 0x1001);
        assert_eq!(r.failure_stage, FailureStage::Invariant);
    }

    #[test]
    fn rejects_reserved_nonzero() {
        let mut wire = sample_wire();
        wire[70] = 1; // poison reserved
        assert!(matches!(
            DecodedReceipt::parse(&wire),
            Err(ReceiptError::ReservedNonZero)
        ));
    }

    #[test]
    fn changed_field_iter_enumerates_bits() {
        let wire = sample_wire();
        let r = DecodedReceipt::parse(&wire).unwrap();
        let indices: alloc::vec::Vec<u32> = r.changed_field_indices().collect();
        assert_eq!(indices, alloc::vec![0u32, 1u32, 3u32]);
    }

    #[test]
    fn changed_segment_iter_enumerates_bits() {
        let wire = sample_wire();
        let r = DecodedReceipt::parse(&wire).unwrap();
        let indices: alloc::vec::Vec<u32> = r.changed_segment_indices().collect();
        assert_eq!(indices, alloc::vec![1u32]);
    }
}
