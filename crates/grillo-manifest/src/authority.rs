//! Upgrade authority diff between two Hopper program manifests.
//!
//! A program upgrade can change bytecode without changing its interface, and
//! it can change its interface without anyone noticing that the new release
//! may now do more than the old one. This module answers one narrow question
//! for an upgrade reviewer:
//!
//! > Does the new manifest grant any instruction more authority than the old
//! > one did?
//!
//! "Authority" is everything a Hopper manifest declares about what a caller
//! must prove and what a handler may mutate:
//!
//! - which instructions exist (matched by their exact discriminator bytes);
//! - which accounts must sign and which are writable at the Sealevel level;
//! - the enforced byte-range write set under `strict_writes`, compared per
//!   layout field so a layout shift is not misread as a new permission;
//! - exact-cell parametric rules that narrow a column to one selected cell;
//! - the lamport permission set when the context is mutation-complete;
//! - the remaining-account ceiling;
//! - per-account context constraints: PDA seeds, `has_one` relations,
//!   expected owner and address (including CPI program bindings), account
//!   kind, optionality, lifecycle (init, realloc, close), and policy
//!   references.
//!
//! Every difference is classified as [`AuthorityImpact::Widened`],
//! [`AuthorityImpact::Narrowed`], [`AuthorityImpact::Review`] (changed in a
//! way that cannot be ordered, such as a PDA seed swap or a different CPI
//! program), or [`AuthorityImpact::Info`]. The report binds both inputs by a
//! SHA-256 digest of their canonical JSON, so an approved widening can be
//! checked into review and re-validated against exactly that manifest pair.
//!
//! Scope: this compares declarations. It does not inspect bytecode, prove
//! that either handler honors its manifest, or cover constraints a manifest
//! does not carry. Pair it with `hopper verify --release` (ELF binding) and
//! Grillo evidence verification (observed effects) for the other two legs.
//!
//! ```
//! use grillo_manifest::authority::{AuthorityDiff, AuthorityVerdict};
//!
//! let old = r#"{ "name": "p", "version": "1.0.0", "instructions": [
//!   { "name": "pause", "tag": 1, "strictWrites": true,
//!     "accounts": [ { "name": "admin", "signer": true },
//!                   { "name": "config", "writable": true } ],
//!     "writeRanges": [ { "accountIndex": 1, "offset": 114, "size": 1 } ] } ] }"#;
//! // The upgrade drops the admin signature requirement.
//! let new = old.replace(r#""name": "admin", "signer": true"#, r#""name": "admin""#);
//!
//! let report = AuthorityDiff::between_json(old, &new).unwrap();
//! assert_eq!(report.verdict(), AuthorityVerdict::Widened);
//! assert!(report.findings.iter().any(|f| f.code == "signer_dropped"));
//! ```

use std::collections::BTreeMap;
use std::fmt::Write as _;

use serde::{Deserialize, Serialize};

use crate::manifest::{MutationManifest, ParametricRangeContract, ParseError, RangeContract};
use crate::sha256::sha256;

/// Report schema identifier. Bump when the finding codes or matching rules
/// change meaning, so an approval file from an older encoder is refused.
pub const AUTHORITY_DIFF_SCHEMA: &str = "grillo.authority-diff.v1";

/// Direction of one authority change.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthorityImpact {
    /// The new manifest permits something the old one refused.
    Widened,
    /// A declared constraint changed in a way that has no order (for example
    /// different PDA seeds or a different expected CPI program). A reviewer
    /// has to decide.
    Review,
    /// The new manifest is strictly more restrictive here.
    Narrowed,
    /// Informational: no authority change, but worth showing (renames).
    Info,
}

impl AuthorityImpact {
    /// Short uppercase label used by the text renderer.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Widened => "WIDENED",
            Self::Review => "REVIEW",
            Self::Narrowed => "NARROWED",
            Self::Info => "INFO",
        }
    }
}

/// One classified difference between the two manifests.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct AuthorityFinding {
    /// Direction of the change.
    pub impact: AuthorityImpact,
    /// Instruction name in the new manifest (old name for removals).
    pub instruction: String,
    /// Account role the finding is about, when it is account-scoped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
    /// Stable machine code, for example `signer_dropped`.
    pub code: String,
    /// Human-readable detail. Byte ranges are half-open `[start, end)`.
    pub detail: String,
}

/// Overall result of an authority diff.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthorityVerdict {
    /// No finding widens authority and none needs review.
    NotWidened,
    /// At least one finding widens authority.
    Widened,
    /// Nothing widens outright, but at least one change needs review.
    Review,
}

impl AuthorityVerdict {
    /// Uppercase label used by the text renderer and CLIs.
    pub const fn label(self) -> &'static str {
        match self {
            Self::NotWidened => "NOT WIDENED",
            Self::Widened => "WIDENED",
            Self::Review => "REVIEW",
        }
    }
}

/// The full authority diff between an old and a new manifest.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorityReport {
    /// Report schema identifier ([`AUTHORITY_DIFF_SCHEMA`]).
    pub schema: String,
    /// Old program name and version, as declared.
    pub old_program: String,
    /// New program name and version, as declared.
    pub new_program: String,
    /// SHA-256 over the canonical JSON of the old manifest (hex).
    pub old_manifest_digest: String,
    /// SHA-256 over the canonical JSON of the new manifest (hex).
    pub new_manifest_digest: String,
    /// Findings in a deterministic order: widened first.
    pub findings: Vec<AuthorityFinding>,
}

/// Why an approval file does not cover a report.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ApprovalError {
    /// The approval was produced by a different report schema.
    Schema(String),
    /// The approval was issued for a different old or new manifest.
    DigestMismatch {
        /// Which side differs (`old` or `new`).
        side: &'static str,
        /// Digest recorded in the approval.
        approved: String,
        /// Digest of the manifest being checked.
        actual: String,
    },
    /// Widened or review findings that the approval does not list.
    Unapproved(Vec<AuthorityFinding>),
}

impl core::fmt::Display for ApprovalError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Schema(schema) => write!(
                f,
                "approval uses schema `{schema}`, expected `{AUTHORITY_DIFF_SCHEMA}`"
            ),
            Self::DigestMismatch {
                side,
                approved,
                actual,
            } => write!(
                f,
                "approval was issued for a different {side} manifest (approved {approved}, \
                 actual {actual})"
            ),
            Self::Unapproved(findings) => {
                write!(f, "{} unapproved authority change(s)", findings.len())
            }
        }
    }
}

impl std::error::Error for ApprovalError {}

impl AuthorityReport {
    /// Overall verdict. Widened dominates review.
    pub fn verdict(&self) -> AuthorityVerdict {
        if self
            .findings
            .iter()
            .any(|f| f.impact == AuthorityImpact::Widened)
        {
            AuthorityVerdict::Widened
        } else if self
            .findings
            .iter()
            .any(|f| f.impact == AuthorityImpact::Review)
        {
            AuthorityVerdict::Review
        } else {
            AuthorityVerdict::NotWidened
        }
    }

    /// Findings of one impact class.
    pub fn with_impact(&self, impact: AuthorityImpact) -> impl Iterator<Item = &AuthorityFinding> {
        self.findings.iter().filter(move |f| f.impact == impact)
    }

    /// Check that `approved` (a previously reviewed report) covers this one.
    ///
    /// The approval must use the same schema and name the same old and new
    /// manifest digests, and every widened or review finding here must appear
    /// verbatim in it. Narrowed and informational findings never need
    /// approval. An approval cannot be replayed onto a different manifest
    /// pair because both digests are part of the check.
    pub fn check_approval(&self, approved: &AuthorityReport) -> Result<(), ApprovalError> {
        if approved.schema != AUTHORITY_DIFF_SCHEMA {
            return Err(ApprovalError::Schema(approved.schema.clone()));
        }
        if approved.old_manifest_digest != self.old_manifest_digest {
            return Err(ApprovalError::DigestMismatch {
                side: "old",
                approved: approved.old_manifest_digest.clone(),
                actual: self.old_manifest_digest.clone(),
            });
        }
        if approved.new_manifest_digest != self.new_manifest_digest {
            return Err(ApprovalError::DigestMismatch {
                side: "new",
                approved: approved.new_manifest_digest.clone(),
                actual: self.new_manifest_digest.clone(),
            });
        }
        let unapproved: Vec<AuthorityFinding> = self
            .findings
            .iter()
            .filter(|f| matches!(f.impact, AuthorityImpact::Widened | AuthorityImpact::Review))
            .filter(|f| !approved.findings.contains(f))
            .cloned()
            .collect();
        if unapproved.is_empty() {
            Ok(())
        } else {
            Err(ApprovalError::Unapproved(unapproved))
        }
    }

    /// Serialize the report as pretty JSON (the approval file format).
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("authority report serializes")
    }

    /// Parse a report previously written by [`to_json`](Self::to_json).
    pub fn from_json(json: &str) -> Result<Self, ParseError> {
        serde_json::from_str(json).map_err(|e| ParseError::Json(e.to_string()))
    }

    /// Plain-text rendering for terminals and CI logs.
    pub fn render(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(
            out,
            "authority diff: {} -> {}",
            self.old_program, self.new_program
        );
        let _ = writeln!(out, "  old manifest sha256 {}", self.old_manifest_digest);
        let _ = writeln!(out, "  new manifest sha256 {}", self.new_manifest_digest);
        if self.findings.is_empty() {
            let _ = writeln!(out, "  no authority differences");
        }
        for finding in &self.findings {
            let scope = match &finding.account {
                Some(account) => format!("{}.{}", finding.instruction, account),
                None => finding.instruction.clone(),
            };
            let _ = writeln!(
                out,
                "  {:<8} {:<34} {:<28} {}",
                finding.impact.label(),
                scope,
                finding.code,
                finding.detail
            );
        }
        let count = |impact| self.with_impact(impact).count();
        let _ = writeln!(
            out,
            "verdict: {} ({} widened, {} review, {} narrowed, {} info)",
            self.verdict().label(),
            count(AuthorityImpact::Widened),
            count(AuthorityImpact::Review),
            count(AuthorityImpact::Narrowed),
            count(AuthorityImpact::Info),
        );
        out
    }
}

/// Entry point for computing authority diffs.
pub struct AuthorityDiff;

impl AuthorityDiff {
    /// Diff two `hopper.manifest.json` documents.
    ///
    /// Both inputs must pass the same version gate as
    /// [`MutationManifest::from_json`]: a manifest that predates the
    /// byte-range mutation contract is refused rather than compared as if its
    /// writes were unconstrained.
    pub fn between_json(old_json: &str, new_json: &str) -> Result<AuthorityReport, ParseError> {
        MutationManifest::from_json(old_json)?;
        MutationManifest::from_json(new_json)?;
        let old_value: serde_json::Value =
            serde_json::from_str(old_json).map_err(|e| ParseError::Json(e.to_string()))?;
        let new_value: serde_json::Value =
            serde_json::from_str(new_json).map_err(|e| ParseError::Json(e.to_string()))?;
        let old: Doc = serde_json::from_value(old_value.clone())
            .map_err(|e| ParseError::Json(e.to_string()))?;
        let new: Doc = serde_json::from_value(new_value.clone())
            .map_err(|e| ParseError::Json(e.to_string()))?;

        let mut findings = Vec::new();
        diff_program(&old, &new, &mut findings);
        findings.sort();
        findings.dedup();

        Ok(AuthorityReport {
            schema: AUTHORITY_DIFF_SCHEMA.to_string(),
            old_program: format!("{} v{}", old.name, old.version),
            new_program: format!("{} v{}", new.name, new.version),
            old_manifest_digest: hex(&sha256(&canonical_json(&old_value))),
            new_manifest_digest: hex(&sha256(&canonical_json(&new_value))),
            findings,
        })
    }
}

// ---------------------------------------------------------------------------
// Manifest document model (only what authority depends on).
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct Doc {
    name: String,
    version: String,
    #[serde(default)]
    layouts: Vec<DocLayout>,
    instructions: Vec<DocInstruction>,
    #[serde(default)]
    contexts: Vec<DocContext>,
}

#[derive(Deserialize)]
struct DocLayout {
    name: String,
    #[serde(rename = "layoutId", default)]
    layout_id: serde_json::Value,
    #[serde(default)]
    fields: Vec<DocField>,
}

#[derive(Deserialize)]
struct DocField {
    name: String,
    offset: u32,
    size: u32,
}

#[derive(Deserialize)]
struct DocInstruction {
    name: String,
    tag: u8,
    #[serde(rename = "discriminatorBytes", default)]
    discriminator: Vec<u8>,
    #[serde(default)]
    accounts: Vec<DocAccount>,
    #[serde(rename = "remainingAccountsMax", default)]
    remaining_accounts_max: Option<u16>,
    #[serde(rename = "strictWrites")]
    strict_writes: bool,
    #[serde(rename = "mutationComplete", default)]
    mutation_complete: bool,
    #[serde(rename = "lamportAccounts", default)]
    lamport_accounts: Vec<u8>,
    #[serde(rename = "writeRanges", default)]
    write_ranges: Vec<RangeContract>,
    #[serde(rename = "parametricWriteRanges", default)]
    parametric: Vec<ParametricRangeContract>,
}

impl DocInstruction {
    fn key(&self) -> Vec<u8> {
        if self.discriminator.is_empty() {
            vec![self.tag]
        } else {
            self.discriminator.clone()
        }
    }

    fn index_of(&self, name: &str) -> Option<u8> {
        self.accounts
            .iter()
            .position(|a| a.name == name)
            .and_then(|i| u8::try_from(i).ok())
    }
}

#[derive(Deserialize)]
struct DocAccount {
    name: String,
    #[serde(default)]
    writable: bool,
    #[serde(default)]
    signer: bool,
    #[serde(rename = "layoutRef", default)]
    layout_ref: Option<String>,
}

#[derive(Deserialize)]
struct DocContext {
    name: String,
    /// Handler names the program macro bound to this context.
    #[serde(default)]
    instructions: Vec<String>,
    #[serde(default)]
    accounts: Vec<DocContextAccount>,
}

#[derive(Deserialize)]
struct DocContextAccount {
    name: String,
    #[serde(default)]
    kind: String,
    #[serde(default)]
    writable: bool,
    #[serde(default)]
    signer: bool,
    #[serde(rename = "layoutRef", default)]
    layout_ref: Option<String>,
    #[serde(rename = "policyRef", default)]
    policy_ref: Option<String>,
    #[serde(default)]
    seeds: Vec<String>,
    #[serde(default)]
    optional: bool,
    #[serde(default)]
    lifecycle: Option<String>,
    #[serde(default)]
    payer: Option<String>,
    #[serde(rename = "hasOne", default)]
    has_one: Vec<String>,
    #[serde(rename = "expectedAddress", default)]
    expected_address: Option<String>,
    #[serde(rename = "expectedOwner", default)]
    expected_owner: Option<String>,
}

impl DocContextAccount {
    fn lifecycle(&self) -> &str {
        self.lifecycle.as_deref().unwrap_or("existing")
    }
}

// ---------------------------------------------------------------------------
// Program and instruction comparison.
// ---------------------------------------------------------------------------

struct Sink<'a> {
    findings: &'a mut Vec<AuthorityFinding>,
    instruction: String,
}

impl Sink<'_> {
    fn push(&mut self, impact: AuthorityImpact, account: Option<&str>, code: &str, detail: String) {
        self.findings.push(AuthorityFinding {
            impact,
            instruction: self.instruction.clone(),
            account: account.map(str::to_string),
            code: code.to_string(),
            detail,
        });
    }
}

fn diff_program(old: &Doc, new: &Doc, findings: &mut Vec<AuthorityFinding>) {
    if old.name != new.name {
        findings.push(AuthorityFinding {
            impact: AuthorityImpact::Info,
            instruction: "*".to_string(),
            account: None,
            code: "program_renamed".to_string(),
            detail: format!("`{}` -> `{}`", old.name, new.name),
        });
    }

    let old_by_key: BTreeMap<Vec<u8>, &DocInstruction> =
        old.instructions.iter().map(|ix| (ix.key(), ix)).collect();
    let new_by_key: BTreeMap<Vec<u8>, &DocInstruction> =
        new.instructions.iter().map(|ix| (ix.key(), ix)).collect();

    for (key, new_ix) in &new_by_key {
        let mut sink = Sink {
            findings,
            instruction: new_ix.name.clone(),
        };
        match old_by_key.get(key) {
            Some(old_ix) => diff_instruction(old, new, old_ix, new_ix, &mut sink),
            None => {
                let rebound = old.instructions.iter().find(|o| o.name == new_ix.name);
                let mut detail = format!(
                    "new entry point, discriminator {}; {}",
                    fmt_bytes(key),
                    summarize_authority(new_ix)
                );
                if let Some(previous) = rebound {
                    let _ = write!(
                        detail,
                        "; same name was previously bound to {}",
                        fmt_bytes(&previous.key())
                    );
                }
                sink.push(AuthorityImpact::Widened, None, "instruction_added", detail);
            }
        }
    }
    for (key, old_ix) in &old_by_key {
        if !new_by_key.contains_key(key) {
            findings.push(AuthorityFinding {
                impact: AuthorityImpact::Narrowed,
                instruction: old_ix.name.clone(),
                account: None,
                code: "instruction_removed".to_string(),
                detail: format!("discriminator {} no longer dispatches", fmt_bytes(key)),
            });
        }
    }
}

fn summarize_authority(ix: &DocInstruction) -> String {
    let writable: Vec<&str> = ix
        .accounts
        .iter()
        .filter(|a| a.writable)
        .map(|a| a.name.as_str())
        .collect();
    let signers: Vec<&str> = ix
        .accounts
        .iter()
        .filter(|a| a.signer)
        .map(|a| a.name.as_str())
        .collect();
    format!(
        "writable [{}], signers [{}], strict_writes {}",
        writable.join(", "),
        signers.join(", "),
        ix.strict_writes
    )
}

fn diff_instruction(
    old_doc: &Doc,
    new_doc: &Doc,
    old: &DocInstruction,
    new: &DocInstruction,
    sink: &mut Sink<'_>,
) {
    use AuthorityImpact::*;

    if old.name != new.name {
        sink.push(
            Info,
            None,
            "instruction_renamed",
            format!("`{}` -> `{}` at the same discriminator", old.name, new.name),
        );
    }

    match (old.strict_writes, new.strict_writes) {
        (true, false) => sink.push(
            Widened,
            None,
            "strict_writes_removed",
            "byte-range enforcement dropped; every writable account's data is now unbounded"
                .to_string(),
        ),
        (false, true) => sink.push(
            Narrowed,
            None,
            "strict_writes_added",
            "data writes are now limited to declared byte ranges".to_string(),
        ),
        _ => {}
    }

    match (old.mutation_complete, new.mutation_complete) {
        (true, false) => sink.push(
            Widened,
            None,
            "lamport_contract_removed",
            "lamport permissions are no longer declared; any writable account's lamports may move"
                .to_string(),
        ),
        (false, true) => sink.push(
            Narrowed,
            None,
            "lamport_contract_added",
            "lamport mutation is now limited to declared accounts".to_string(),
        ),
        (true, true) => diff_lamports(old, new, sink),
        (false, false) => {}
    }

    match (old.remaining_accounts_max, new.remaining_accounts_max) {
        (None, Some(max)) => sink.push(
            Widened,
            None,
            "remaining_accounts_added",
            format!("now accepts up to {max} variadic accounts"),
        ),
        (Some(a), Some(b)) if b > a => sink.push(
            Widened,
            None,
            "remaining_accounts_raised",
            format!("variadic ceiling {a} -> {b}"),
        ),
        (Some(a), Some(b)) if b < a => sink.push(
            Narrowed,
            None,
            "remaining_accounts_lowered",
            format!("variadic ceiling {a} -> {b}"),
        ),
        (Some(a), None) => sink.push(
            Narrowed,
            None,
            "remaining_accounts_removed",
            format!("variadic suffix (max {a}) no longer accepted"),
        ),
        _ => {}
    }

    let old_ctx = matching_context(old, &old_doc.contexts);
    let new_ctx = matching_context(new, &new_doc.contexts);
    if old_ctx.is_some() && new_ctx.is_none() {
        sink.push(
            Review,
            None,
            "context_constraints_unavailable",
            "the old manifest published this instruction's context constraints; the new one \
             does not, so seed, relation, owner, and lifecycle changes cannot be compared"
                .to_string(),
        );
    }

    for new_account in &new.accounts {
        let name = new_account.name.as_str();
        let new_ctx_account = context_account(new_ctx, name);
        let Some(old_account) = old.accounts.iter().find(|a| a.name == name) else {
            let is_program = new_ctx_account.is_some_and(|c| c.kind.starts_with("Program"));
            if is_program {
                sink.push(
                    Widened,
                    Some(name),
                    "cpi_program_added",
                    format!(
                        "new program account `{}`{}",
                        new_ctx_account.map(|c| c.kind.as_str()).unwrap_or(""),
                        new_ctx_account
                            .and_then(|c| c.expected_address.as_deref())
                            .map(|a| format!(" bound to {a}"))
                            .unwrap_or_default()
                    ),
                );
            } else if new_account.writable {
                sink.push(
                    Widened,
                    Some(name),
                    "writable_account_added",
                    data_authority(new, name, new_doc).describe_new(),
                );
            } else if new_account.signer {
                sink.push(
                    Narrowed,
                    Some(name),
                    "signer_account_added",
                    "a new required signer".to_string(),
                );
            } else {
                sink.push(
                    Info,
                    Some(name),
                    "readonly_account_added",
                    "a new read-only, non-signer account".to_string(),
                );
            }
            continue;
        };

        match (old_account.signer, new_account.signer) {
            (true, false) => sink.push(
                Widened,
                Some(name),
                "signer_dropped",
                "signature no longer required".to_string(),
            ),
            (false, true) => sink.push(
                Narrowed,
                Some(name),
                "signer_required",
                "signature now required".to_string(),
            ),
            _ => {}
        }
        match (old_account.writable, new_account.writable) {
            (false, true) => sink.push(
                Widened,
                Some(name),
                "became_writable",
                "read-only account is now writable".to_string(),
            ),
            (true, false) => sink.push(
                Narrowed,
                Some(name),
                "became_readonly",
                "writable account is now read-only".to_string(),
            ),
            _ => {}
        }

        diff_data_authority(old_doc, new_doc, old, new, name, sink);
        diff_parametric(old, new, name, sink);
        if let (Some(old_c), Some(new_c)) = (context_account(old_ctx, name), new_ctx_account) {
            diff_context_account(old_c, new_c, sink);
        }
    }

    for old_account in &old.accounts {
        let name = old_account.name.as_str();
        if new.accounts.iter().any(|a| a.name == name) {
            continue;
        }
        if old_account.signer {
            sink.push(
                Widened,
                Some(name),
                "signer_account_removed",
                "a required signer is no longer part of the instruction".to_string(),
            );
        } else {
            sink.push(
                Narrowed,
                Some(name),
                "account_removed",
                if old_account.writable {
                    "writable account no longer passed".to_string()
                } else {
                    "read-only account no longer passed".to_string()
                },
            );
        }
    }
}

fn diff_lamports(old: &DocInstruction, new: &DocInstruction, sink: &mut Sink<'_>) {
    let names = |ix: &DocInstruction| -> Vec<String> {
        let mut out: Vec<String> = ix
            .lamport_accounts
            .iter()
            .map(|i| {
                ix.accounts
                    .get(*i as usize)
                    .map(|a| a.name.clone())
                    .unwrap_or_else(|| format!("#{i}"))
            })
            .collect();
        out.sort();
        out.dedup();
        out
    };
    let old_names = names(old);
    let new_names = names(new);
    for name in new_names.iter().filter(|n| !old_names.contains(n)) {
        sink.push(
            AuthorityImpact::Widened,
            Some(name.as_str()),
            "lamport_permission_added",
            "lamports may now be mutated".to_string(),
        );
    }
    for name in old_names.iter().filter(|n| !new_names.contains(n)) {
        sink.push(
            AuthorityImpact::Narrowed,
            Some(name.as_str()),
            "lamport_permission_removed",
            "lamports may no longer be mutated".to_string(),
        );
    }
}

// ---------------------------------------------------------------------------
// Data write authority.
// ---------------------------------------------------------------------------

enum DataAuthority<'a> {
    /// The account is read-only at the Sealevel level.
    None,
    /// Only these byte intervals may be written through tracked access.
    Ranges {
        intervals: Vec<(u64, u64)>,
        layout: Option<&'a DocLayout>,
    },
    /// Writable without a byte-range contract: the whole account.
    Unbounded,
}

impl DataAuthority<'_> {
    fn describe_new(&self) -> String {
        match self {
            DataAuthority::None => "not writable".to_string(),
            DataAuthority::Unbounded => "writable with unbounded data authority".to_string(),
            DataAuthority::Ranges { intervals, .. } if intervals.is_empty() => {
                "writable, no data ranges (lamports or lifecycle only)".to_string()
            }
            DataAuthority::Ranges { intervals, layout } => {
                format!("writable bytes {}", describe(intervals, *layout))
            }
        }
    }
}

fn data_authority<'a>(ix: &DocInstruction, name: &str, doc: &'a Doc) -> DataAuthority<'a> {
    let Some(account) = ix.accounts.iter().find(|a| a.name == name) else {
        return DataAuthority::None;
    };
    if !account.writable {
        return DataAuthority::None;
    }
    if !ix.strict_writes {
        return DataAuthority::Unbounded;
    }
    let index = ix.index_of(name);
    let intervals = normalize(
        ix.write_ranges
            .iter()
            .filter(|r| Some(r.account_index) == index)
            .map(|r| (r.offset as u64, r.end()))
            .collect(),
    );
    let layout = account
        .layout_ref
        .as_deref()
        .and_then(|l| doc.layouts.iter().find(|layout| layout.name == l));
    DataAuthority::Ranges { intervals, layout }
}

fn diff_data_authority(
    old_doc: &Doc,
    new_doc: &Doc,
    old: &DocInstruction,
    new: &DocInstruction,
    name: &str,
    sink: &mut Sink<'_>,
) {
    use AuthorityImpact::*;
    let before = data_authority(old, name, old_doc);
    let after = data_authority(new, name, new_doc);
    match (&before, &after) {
        (DataAuthority::Unbounded, DataAuthority::Unbounded)
        | (DataAuthority::None, DataAuthority::None) => {}
        (_, DataAuthority::Unbounded) => sink.push(
            Widened,
            Some(name),
            "data_authority_unbounded",
            "account data is now writable without a byte-range contract".to_string(),
        ),
        (DataAuthority::Unbounded, _) => sink.push(
            Narrowed,
            Some(name),
            "data_authority_bounded",
            format!("was unbounded, now {}", after.describe_new()),
        ),
        (
            DataAuthority::None,
            DataAuthority::Ranges {
                intervals,
                layout: new_layout,
            },
        ) => {
            if !intervals.is_empty() {
                sink.push(
                    Widened,
                    Some(name),
                    "write_range_widened",
                    format!("gains {}", describe(intervals, *new_layout)),
                );
            }
        }
        (
            DataAuthority::Ranges {
                intervals,
                layout: old_layout,
            },
            DataAuthority::None,
        ) => {
            if !intervals.is_empty() {
                sink.push(
                    Narrowed,
                    Some(name),
                    "write_range_narrowed",
                    format!("loses {}", describe(intervals, *old_layout)),
                );
            }
        }
        (
            DataAuthority::Ranges {
                intervals: old_iv,
                layout: old_layout,
            },
            DataAuthority::Ranges {
                intervals: new_iv,
                layout: new_layout,
            },
        ) => {
            let (gained, lost) = match (old_layout, new_layout) {
                (Some(ol), Some(nl)) => field_relative_delta(old_iv, ol, new_iv, nl),
                _ => (
                    describe_opt(&subtract(new_iv, old_iv), *new_layout),
                    describe_opt(&subtract(old_iv, new_iv), *old_layout),
                ),
            };
            if let Some(gained) = gained {
                sink.push(
                    Widened,
                    Some(name),
                    "write_range_widened",
                    format!("gains {gained}"),
                );
            }
            if let Some(lost) = lost {
                sink.push(
                    Narrowed,
                    Some(name),
                    "write_range_narrowed",
                    format!("loses {lost}"),
                );
            }
            if let (Some(ol), Some(nl)) = (old_layout, new_layout) {
                if ol.name == nl.name && ol.layout_id != nl.layout_id {
                    sink.push(
                        Info,
                        Some(name),
                        "layout_changed",
                        format!(
                            "layout `{}` changed identity; ranges were compared per field",
                            nl.name
                        ),
                    );
                }
            }
        }
    }
}

/// Compare two range sets in field space so that a field that moved to a new
/// offset keeps the same authority, then compare the bytes outside every
/// declared field (header and padding) by absolute offset.
fn field_relative_delta(
    old_iv: &[(u64, u64)],
    old_layout: &DocLayout,
    new_iv: &[(u64, u64)],
    new_layout: &DocLayout,
) -> (Option<String>, Option<String>) {
    let mut gained = Vec::new();
    let mut lost = Vec::new();

    let relative = |iv: &[(u64, u64)], field: &DocField| -> Vec<(u64, u64)> {
        let start = field.offset as u64;
        let end = start + field.size as u64;
        intersect(iv, &[(start, end)])
            .into_iter()
            .map(|(a, b)| (a - start, b - start))
            .collect()
    };
    let label = |field: &DocField, rel: &[(u64, u64)]| -> String {
        if rel == [(0, field.size as u64)] {
            format!("`{}`", field.name)
        } else {
            let parts: Vec<String> = rel.iter().map(|(a, b)| format!("[{a}, {b})")).collect();
            format!("`{}` bytes {}", field.name, parts.join(" "))
        }
    };

    for field in &new_layout.fields {
        let new_rel = relative(new_iv, field);
        let old_rel = old_layout
            .fields
            .iter()
            .find(|f| f.name == field.name)
            .map(|f| relative(old_iv, f))
            .unwrap_or_default();
        let g = subtract(&new_rel, &old_rel);
        if !g.is_empty() {
            gained.push(label(field, &g));
        }
    }
    for field in &old_layout.fields {
        let old_rel = relative(old_iv, field);
        let new_rel = new_layout
            .fields
            .iter()
            .find(|f| f.name == field.name)
            .map(|f| relative(new_iv, f))
            .unwrap_or_default();
        let l = subtract(&old_rel, &new_rel);
        if !l.is_empty() {
            lost.push(label(field, &l));
        }
    }

    let outside = |iv: &[(u64, u64)], layout: &DocLayout| -> Vec<(u64, u64)> {
        let covered = normalize(
            layout
                .fields
                .iter()
                .map(|f| (f.offset as u64, f.offset as u64 + f.size as u64))
                .collect(),
        );
        subtract(iv, &covered)
    };
    let old_out = outside(old_iv, old_layout);
    let new_out = outside(new_iv, new_layout);
    let first_field = |layout: &DocLayout| layout.fields.iter().map(|f| f.offset as u64).min();
    let unmapped = |iv: &[(u64, u64)], layout: &DocLayout| -> Vec<String> {
        iv.iter()
            .map(|&(a, b)| match first_field(layout) {
                Some(first) if b <= first => format!("header bytes [{a}, {b})"),
                _ => format!("unmapped bytes {}", fmt_interval(a, b)),
            })
            .collect()
    };
    gained.extend(unmapped(&subtract(&new_out, &old_out), new_layout));
    lost.extend(unmapped(&subtract(&old_out, &new_out), old_layout));

    let join = |v: Vec<String>| (!v.is_empty()).then(|| v.join(", "));
    (join(gained), join(lost))
}

fn describe_opt(iv: &[(u64, u64)], layout: Option<&DocLayout>) -> Option<String> {
    (!iv.is_empty()).then(|| describe(iv, layout))
}

/// Render intervals, naming the layout fields they cover when known.
fn describe(iv: &[(u64, u64)], layout: Option<&DocLayout>) -> String {
    let bytes: Vec<String> = iv.iter().map(|&(a, b)| fmt_interval(a, b)).collect();
    let Some(layout) = layout else {
        return format!("bytes {}", bytes.join(" "));
    };
    let fields: Vec<&str> = layout
        .fields
        .iter()
        .filter(|f| {
            let start = f.offset as u64;
            let end = start + f.size as u64;
            !intersect(iv, &[(start, end)]).is_empty()
        })
        .map(|f| f.name.as_str())
        .collect();
    if fields.is_empty() {
        format!("bytes {}", bytes.join(" "))
    } else {
        format!("bytes {} ({})", bytes.join(" "), fields.join(", "))
    }
}

fn fmt_interval(a: u64, b: u64) -> String {
    if a == 0 && b >= u32::MAX as u64 {
        "[whole account]".to_string()
    } else {
        format!("[{a}, {b})")
    }
}

// ---------------------------------------------------------------------------
// Exact-cell parametric rules.
// ---------------------------------------------------------------------------

fn diff_parametric(old: &DocInstruction, new: &DocInstruction, name: &str, sink: &mut Sink<'_>) {
    use AuthorityImpact::*;
    if !old.strict_writes || !new.strict_writes {
        // Without a strict contract on both sides the static comparison above
        // already reports the dominant change.
        return;
    }
    let (Some(old_index), Some(new_index)) = (old.index_of(name), new.index_of(name)) else {
        return;
    };
    let old_rules: Vec<&ParametricRangeContract> = old
        .parametric
        .iter()
        .filter(|r| r.account_index == old_index)
        .collect();
    let new_rules: Vec<&ParametricRangeContract> = new
        .parametric
        .iter()
        .filter(|r| r.account_index == new_index)
        .collect();
    let new_static = normalize(
        new.write_ranges
            .iter()
            .filter(|r| r.account_index == new_index)
            .map(|r| (r.offset as u64, r.end()))
            .collect(),
    );

    for rule in &old_rules {
        match new_rules
            .iter()
            .find(|r| r.segment_name == rule.segment_name)
        {
            None => {
                let still_static = rule.envelope_end().is_some_and(|end| {
                    !intersect(&new_static, &[(rule.base_offset as u64, end)]).is_empty()
                });
                if still_static {
                    sink.push(
                        Widened,
                        Some(name),
                        "exact_cell_rule_removed",
                        format!(
                            "`{}` was one selected cell per call; the whole column is now \
                             writable",
                            rule.segment_name
                        ),
                    );
                }
            }
            Some(next) => {
                if next.count > rule.count || next.cell_size > rule.cell_size {
                    sink.push(
                        Widened,
                        Some(name),
                        "exact_cell_rule_widened",
                        format!(
                            "`{}` cells {} x {} bytes -> {} x {} bytes",
                            rule.segment_name,
                            rule.count,
                            rule.cell_size,
                            next.count,
                            next.cell_size
                        ),
                    );
                } else if next.count < rule.count || next.cell_size < rule.cell_size {
                    sink.push(
                        Narrowed,
                        Some(name),
                        "exact_cell_rule_narrowed",
                        format!(
                            "`{}` cells {} x {} bytes -> {} x {} bytes",
                            rule.segment_name,
                            rule.count,
                            rule.cell_size,
                            next.count,
                            next.cell_size
                        ),
                    );
                }
                if next.argument_name != rule.argument_name
                    || next.argument_index != rule.argument_index
                {
                    sink.push(
                        Review,
                        Some(name),
                        "exact_cell_selector_changed",
                        format!(
                            "`{}` selector `{}` (#{}) -> `{}` (#{})",
                            rule.segment_name,
                            rule.argument_name,
                            rule.argument_index,
                            next.argument_name,
                            next.argument_index
                        ),
                    );
                }
            }
        }
    }
    for rule in &new_rules {
        if !old_rules
            .iter()
            .any(|r| r.segment_name == rule.segment_name)
        {
            sink.push(
                Narrowed,
                Some(name),
                "exact_cell_rule_added",
                format!(
                    "`{}` narrowed to one selected cell per call",
                    rule.segment_name
                ),
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Context constraints.
// ---------------------------------------------------------------------------

fn canonical_identifier(value: &str) -> String {
    value
        .bytes()
        .filter(u8::is_ascii_alphanumeric)
        .map(|b| b.to_ascii_lowercase() as char)
        .collect()
}

fn context_matches(ix: &DocInstruction, ctx: &DocContext) -> bool {
    ctx.accounts.len() == ix.accounts.len()
        && ix.accounts.iter().zip(&ctx.accounts).all(|(a, c)| {
            a.name == c.name
                && a.writable == c.writable
                && a.signer == c.signer
                && match (&a.layout_ref, &c.layout_ref) {
                    (Some(x), Some(y)) => x == y,
                    _ => true,
                }
        })
}

/// Same rule as `hopper fuzz`: a context that names the instruction among
/// the handlers bound to it wins outright; otherwise prefer the structurally
/// matching context whose name equals the instruction's, and last accept a
/// unique structural match.
fn matching_context<'a>(ix: &DocInstruction, contexts: &'a [DocContext]) -> Option<&'a DocContext> {
    if let Some(declared) = contexts
        .iter()
        .find(|c| c.instructions.iter().any(|n| n == &ix.name))
    {
        return Some(declared);
    }
    let wanted = canonical_identifier(&ix.name);
    if let Some(named) = contexts
        .iter()
        .filter(|c| context_matches(ix, c))
        .find(|c| canonical_identifier(&c.name) == wanted)
    {
        return Some(named);
    }
    let mut candidates = contexts.iter().filter(|c| context_matches(ix, c));
    let only = candidates.next()?;
    candidates.next().is_none().then_some(only)
}

fn context_account<'a>(ctx: Option<&'a DocContext>, name: &str) -> Option<&'a DocContextAccount> {
    ctx?.accounts.iter().find(|a| a.name == name)
}

fn is_unchecked_kind(kind: &str) -> bool {
    kind.contains("Unchecked") || kind == "AccountView" || kind == "AccountInfo"
}

fn diff_optional_binding(
    old: Option<&str>,
    new: Option<&str>,
    codes: (&str, &str, &str),
    what: &str,
    account: &str,
    sink: &mut Sink<'_>,
) {
    let (removed, changed, added) = codes;
    match (old, new) {
        (Some(a), None) => sink.push(
            AuthorityImpact::Widened,
            Some(account),
            removed,
            format!("{what} {a} is no longer enforced"),
        ),
        (Some(a), Some(b)) if a != b => sink.push(
            AuthorityImpact::Review,
            Some(account),
            changed,
            format!("{what} {a} -> {b}"),
        ),
        (None, Some(b)) => sink.push(
            AuthorityImpact::Narrowed,
            Some(account),
            added,
            format!("{what} {b} is now enforced"),
        ),
        _ => {}
    }
}

fn diff_context_account(old: &DocContextAccount, new: &DocContextAccount, sink: &mut Sink<'_>) {
    use AuthorityImpact::*;
    let name = new.name.as_str();

    match (old.seeds.is_empty(), new.seeds.is_empty()) {
        (false, true) => sink.push(
            Widened,
            Some(name),
            "pda_binding_removed",
            format!(
                "address was derived from seeds [{}]; any address is now accepted",
                old.seeds.join(", ")
            ),
        ),
        (true, false) => sink.push(
            Narrowed,
            Some(name),
            "pda_binding_added",
            format!("address now derived from seeds [{}]", new.seeds.join(", ")),
        ),
        (false, false) if old.seeds != new.seeds => sink.push(
            Review,
            Some(name),
            "pda_seeds_changed",
            format!(
                "seeds [{}] -> [{}]",
                old.seeds.join(", "),
                new.seeds.join(", ")
            ),
        ),
        _ => {}
    }

    for relation in old.has_one.iter().filter(|r| !new.has_one.contains(r)) {
        sink.push(
            Widened,
            Some(name),
            "has_one_removed",
            format!("no longer required to match `{relation}`"),
        );
    }
    for relation in new.has_one.iter().filter(|r| !old.has_one.contains(r)) {
        sink.push(
            Narrowed,
            Some(name),
            "has_one_added",
            format!("now required to match `{relation}`"),
        );
    }

    let program = new.kind.starts_with("Program") || old.kind.starts_with("Program");
    diff_optional_binding(
        old.expected_address.as_deref(),
        new.expected_address.as_deref(),
        if program {
            (
                "program_binding_removed",
                "cpi_program_changed",
                "program_binding_added",
            )
        } else {
            (
                "address_binding_removed",
                "address_binding_changed",
                "address_binding_added",
            )
        },
        if program { "program id" } else { "address" },
        name,
        sink,
    );
    diff_optional_binding(
        old.expected_owner.as_deref(),
        new.expected_owner.as_deref(),
        ("owner_check_removed", "owner_changed", "owner_check_added"),
        "owner",
        name,
        sink,
    );
    diff_optional_binding(
        old.policy_ref.as_deref(),
        new.policy_ref.as_deref(),
        ("policy_removed", "policy_changed", "policy_added"),
        "policy",
        name,
        sink,
    );

    if old.kind != new.kind {
        if is_unchecked_kind(&new.kind) && !is_unchecked_kind(&old.kind) {
            sink.push(
                Widened,
                Some(name),
                "type_check_removed",
                format!("`{}` -> `{}`", old.kind, new.kind),
            );
        } else if is_unchecked_kind(&old.kind) && !is_unchecked_kind(&new.kind) {
            sink.push(
                Narrowed,
                Some(name),
                "type_check_added",
                format!("`{}` -> `{}`", old.kind, new.kind),
            );
        } else {
            sink.push(
                Review,
                Some(name),
                "account_kind_changed",
                format!("`{}` -> `{}`", old.kind, new.kind),
            );
        }
    }

    match (old.optional, new.optional) {
        (false, true) => sink.push(
            Widened,
            Some(name),
            "became_optional",
            "account may now be omitted".to_string(),
        ),
        (true, false) => sink.push(
            Narrowed,
            Some(name),
            "became_required",
            "account is now required".to_string(),
        ),
        _ => {}
    }

    let (before, after) = (old.lifecycle(), new.lifecycle());
    if before != after {
        let impact = match (before, after) {
            ("existing", _) => Widened,
            (_, "existing") => Narrowed,
            _ => Review,
        };
        sink.push(
            impact,
            Some(name),
            "lifecycle_changed",
            format!("`{before}` -> `{after}`"),
        );
    }

    if old.payer != new.payer && old.payer.is_some() && new.payer.is_some() {
        sink.push(
            Review,
            Some(name),
            "payer_changed",
            format!(
                "`{}` -> `{}`",
                old.payer.as_deref().unwrap_or(""),
                new.payer.as_deref().unwrap_or("")
            ),
        );
    }
}

// ---------------------------------------------------------------------------
// Interval arithmetic over half-open `[start, end)` byte ranges.
// ---------------------------------------------------------------------------

fn normalize(mut iv: Vec<(u64, u64)>) -> Vec<(u64, u64)> {
    iv.retain(|(a, b)| a < b);
    iv.sort_unstable();
    let mut out: Vec<(u64, u64)> = Vec::with_capacity(iv.len());
    for (a, b) in iv {
        match out.last_mut() {
            Some(last) if a <= last.1 => last.1 = last.1.max(b),
            _ => out.push((a, b)),
        }
    }
    out
}

fn subtract(a: &[(u64, u64)], b: &[(u64, u64)]) -> Vec<(u64, u64)> {
    let b = normalize(b.to_vec());
    let mut out = Vec::new();
    for &(mut start, end) in &normalize(a.to_vec()) {
        for &(bs, be) in &b {
            if be <= start || bs >= end {
                continue;
            }
            if bs > start {
                out.push((start, bs));
            }
            start = start.max(be);
            if start >= end {
                break;
            }
        }
        if start < end {
            out.push((start, end));
        }
    }
    out
}

fn intersect(a: &[(u64, u64)], b: &[(u64, u64)]) -> Vec<(u64, u64)> {
    let mut out = Vec::new();
    for &(a0, a1) in a {
        for &(b0, b1) in b {
            let start = a0.max(b0);
            let end = a1.min(b1);
            if start < end {
                out.push((start, end));
            }
        }
    }
    normalize(out)
}

// ---------------------------------------------------------------------------
// Encoding helpers.
// ---------------------------------------------------------------------------

fn fmt_bytes(bytes: &[u8]) -> String {
    let parts: Vec<String> = bytes.iter().map(u8::to_string).collect();
    format!("[{}]", parts.join(", "))
}

fn hex(bytes: &[u8; 32]) -> String {
    let mut out = String::with_capacity(64);
    for b in bytes {
        let _ = write!(out, "{b:02x}");
    }
    out
}

/// Canonical JSON: object keys sorted, no insignificant whitespace, numbers
/// and strings as serde_json prints them. Independent of whether some other
/// crate in the build enabled serde_json's `preserve_order` feature.
fn canonical_json(value: &serde_json::Value) -> Vec<u8> {
    fn write(value: &serde_json::Value, out: &mut String) {
        match value {
            serde_json::Value::Object(map) => {
                let mut keys: Vec<&String> = map.keys().collect();
                keys.sort();
                out.push('{');
                for (i, key) in keys.into_iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    out.push_str(&serde_json::Value::String(key.clone()).to_string());
                    out.push(':');
                    write(&map[key], out);
                }
                out.push('}');
            }
            serde_json::Value::Array(items) => {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write(item, out);
                }
                out.push(']');
            }
            other => out.push_str(&other.to_string()),
        }
    }
    let mut out = String::new();
    write(value, &mut out);
    out.into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = r#"{
      "name": "vault", "version": "1.0.0",
      "layouts": [ { "name": "Config", "layoutId": "aa", "fields": [
          { "name": "admin", "offset": 16, "size": 32 },
          { "name": "fee_bps", "offset": 48, "size": 2 },
          { "name": "paused", "offset": 50, "size": 1 } ] } ],
      "instructions": [
        { "name": "pause", "tag": 1, "discriminatorBytes": [1], "strictWrites": true,
          "accounts": [ { "name": "admin", "signer": true },
                        { "name": "config", "writable": true, "layoutRef": "Config" } ],
          "writeRanges": [ { "accountIndex": 1, "offset": 50, "size": 1 } ] }
      ],
      "contexts": [
        { "name": "Pause", "accounts": [
          { "name": "admin", "kind": "Signer", "signer": true, "seeds": [], "hasOne": [] },
          { "name": "config", "kind": "Config", "writable": true, "layoutRef": "Config",
            "seeds": ["CONFIG_SEED"], "hasOne": ["admin"] } ] }
      ]
    }"#;

    fn diff(new: &str) -> AuthorityReport {
        AuthorityDiff::between_json(BASE, new).unwrap()
    }

    fn codes(report: &AuthorityReport, impact: AuthorityImpact) -> Vec<String> {
        report.with_impact(impact).map(|f| f.code.clone()).collect()
    }

    #[test]
    fn identical_manifests_are_not_widened() {
        let report = diff(BASE);
        assert!(report.findings.is_empty(), "{}", report.render());
        assert_eq!(report.verdict(), AuthorityVerdict::NotWidened);
        assert_eq!(report.old_manifest_digest, report.new_manifest_digest);
    }

    #[test]
    fn whitespace_does_not_change_the_digest() {
        let compact: String = BASE.split_whitespace().collect::<Vec<_>>().join(" ");
        let report = diff(&compact);
        assert_eq!(report.old_manifest_digest, report.new_manifest_digest);
    }

    #[test]
    fn widening_a_field_range_is_named_by_field() {
        let new = BASE.replace(
            r#"{ "accountIndex": 1, "offset": 50, "size": 1 }"#,
            r#"{ "accountIndex": 1, "offset": 48, "size": 3 }"#,
        );
        let report = diff(&new);
        assert_eq!(report.verdict(), AuthorityVerdict::Widened);
        let finding = report
            .findings
            .iter()
            .find(|f| f.code == "write_range_widened")
            .unwrap();
        assert_eq!(finding.account.as_deref(), Some("config"));
        assert!(finding.detail.contains("`fee_bps`"), "{}", finding.detail);
        assert!(!finding.detail.contains("paused"), "{}", finding.detail);
    }

    #[test]
    fn a_field_moving_offset_keeps_its_authority() {
        // Insert a new field before `paused`: every offset shifts, but the
        // handler may still write exactly `paused`.
        let new = BASE
            .replace(
                r#"{ "name": "paused", "offset": 50, "size": 1 }"#,
                r#"{ "name": "flags", "offset": 50, "size": 4 },
                   { "name": "paused", "offset": 54, "size": 1 }"#,
            )
            .replace(r#""layoutId": "aa""#, r#""layoutId": "bb""#)
            .replace(
                r#"{ "accountIndex": 1, "offset": 50, "size": 1 }"#,
                r#"{ "accountIndex": 1, "offset": 54, "size": 1 }"#,
            );
        let report = diff(&new);
        assert_eq!(
            report.verdict(),
            AuthorityVerdict::NotWidened,
            "{}",
            report.render()
        );
        assert_eq!(codes(&report, AuthorityImpact::Info), ["layout_changed"]);
    }

    #[test]
    fn header_write_is_labelled() {
        let new = BASE.replace(
            r#""writeRanges": [ { "accountIndex": 1, "offset": 50, "size": 1 } ]"#,
            r#""writeRanges": [ { "accountIndex": 1, "offset": 50, "size": 1 },
                                { "accountIndex": 1, "offset": 0, "size": 8 } ]"#,
        );
        let report = diff(&new);
        let finding = report
            .findings
            .iter()
            .find(|f| f.code == "write_range_widened")
            .unwrap();
        assert!(
            finding.detail.contains("header bytes [0, 8)"),
            "{}",
            finding.detail
        );
    }

    #[test]
    fn dropping_strict_writes_is_unbounded() {
        let new = BASE.replace(r#""strictWrites": true"#, r#""strictWrites": false"#);
        let report = diff(&new);
        let widened = codes(&report, AuthorityImpact::Widened);
        assert!(widened.contains(&"strict_writes_removed".to_string()));
        assert!(widened.contains(&"data_authority_unbounded".to_string()));
    }

    #[test]
    fn context_constraint_removal_widens() {
        let new = BASE
            .replace(r#""seeds": ["CONFIG_SEED"]"#, r#""seeds": []"#)
            .replace(r#""hasOne": ["admin"]"#, r#""hasOne": []"#);
        let report = diff(&new);
        let widened = codes(&report, AuthorityImpact::Widened);
        assert_eq!(widened, ["has_one_removed", "pda_binding_removed"]);
    }

    #[test]
    fn seed_swap_needs_review() {
        let new = BASE.replace(r#""seeds": ["CONFIG_SEED"]"#, r#""seeds": ["OTHER"]"#);
        let report = diff(&new);
        assert_eq!(report.verdict(), AuthorityVerdict::Review);
        assert_eq!(
            codes(&report, AuthorityImpact::Review),
            ["pda_seeds_changed"]
        );
    }

    #[test]
    fn removing_the_signer_account_widens() {
        let new = BASE
            .replace(r#"{ "name": "admin", "signer": true },"#, "")
            .replace(r#""accountIndex": 1"#, r#""accountIndex": 0"#);
        let report = diff(&new);
        let widened = codes(&report, AuthorityImpact::Widened);
        assert!(
            widened.contains(&"signer_account_removed".to_string()),
            "{}",
            report.render()
        );
        // Authority is compared by role, so the index shift is not a
        // spurious widening of `config`.
        assert!(!widened.contains(&"write_range_widened".to_string()));
    }

    #[test]
    fn new_instruction_and_rebinding_are_reported() {
        let new = BASE.replace(
            r#""discriminatorBytes": [1]"#,
            r#""discriminatorBytes": [9]"#,
        );
        let report = diff(&new);
        let added = report
            .findings
            .iter()
            .find(|f| f.code == "instruction_added")
            .unwrap();
        assert!(
            added.detail.contains("previously bound to [1]"),
            "{}",
            added.detail
        );
        assert_eq!(
            codes(&report, AuthorityImpact::Narrowed),
            ["instruction_removed"]
        );
    }

    #[test]
    fn exact_cell_rule_removal_widens() {
        let with_rule = r#"{ "name": "p", "version": "1", "instructions": [
          { "name": "claim", "tag": 4, "strictWrites": true,
            "accounts": [ { "name": "shard", "writable": true } ],
            "writeRanges": [ { "accountIndex": 0, "offset": 100, "size": 20 } ],
            "parametricWriteRanges": [ { "accountIndex": 0, "baseOffset": 100, "stride": 1,
              "cellSize": 1, "count": 20, "argumentIndex": 0, "argument": "slot",
              "segment": "statuses" } ] } ] }"#;
        let without = with_rule.replace(
            r#""parametricWriteRanges": [ { "accountIndex": 0, "baseOffset": 100, "stride": 1,
              "cellSize": 1, "count": 20, "argumentIndex": 0, "argument": "slot",
              "segment": "statuses" } ]"#,
            r#""parametricWriteRanges": []"#,
        );
        let report = AuthorityDiff::between_json(with_rule, &without).unwrap();
        assert_eq!(
            codes(&report, AuthorityImpact::Widened),
            ["exact_cell_rule_removed"]
        );
        let back = AuthorityDiff::between_json(&without, with_rule).unwrap();
        assert_eq!(back.verdict(), AuthorityVerdict::NotWidened);
    }

    #[test]
    fn lamport_contract_changes() {
        let complete = BASE.replace(
            r#""strictWrites": true,"#,
            r#""strictWrites": true, "mutationComplete": true, "lamportAccounts": [],"#,
        );
        let adds = complete.replace(r#""lamportAccounts": []"#, r#""lamportAccounts": [1]"#);
        let report = AuthorityDiff::between_json(&complete, &adds).unwrap();
        assert_eq!(
            codes(&report, AuthorityImpact::Widened),
            ["lamport_permission_added"]
        );
        let dropped = AuthorityDiff::between_json(&complete, BASE).unwrap();
        assert_eq!(
            codes(&dropped, AuthorityImpact::Widened),
            ["lamport_contract_removed"]
        );
    }

    #[test]
    fn approval_binds_digests_and_findings() {
        let new = BASE.replace(r#""hasOne": ["admin"]"#, r#""hasOne": []"#);
        let report = diff(&new);
        let approval = AuthorityReport::from_json(&report.to_json()).unwrap();
        assert_eq!(report.check_approval(&approval), Ok(()));

        // The same approval cannot be replayed onto a different new manifest.
        let other = diff(&new.replace(r#""seeds": ["CONFIG_SEED"]"#, r#""seeds": []"#));
        assert!(matches!(
            other.check_approval(&approval),
            Err(ApprovalError::DigestMismatch { side: "new", .. })
        ));

        // An approval listing nothing leaves the widening unapproved.
        let mut empty = approval.clone();
        empty.findings.clear();
        assert!(matches!(
            report.check_approval(&empty),
            Err(ApprovalError::Unapproved(ref f)) if f.len() == 1
        ));
    }

    #[test]
    fn legacy_manifests_are_refused() {
        let legacy = r#"{ "name": "l", "version": "0", "instructions": [
            { "name": "i", "tag": 0, "accounts": [] } ] }"#;
        assert!(matches!(
            AuthorityDiff::between_json(legacy, BASE),
            Err(ParseError::UnsupportedManifest(_))
        ));
    }

    #[test]
    fn interval_arithmetic() {
        assert_eq!(normalize(vec![(5, 7), (0, 2), (1, 3)]), [(0, 3), (5, 7)]);
        assert_eq!(
            subtract(&[(0, 10)], &[(2, 4), (6, 8)]),
            [(0, 2), (4, 6), (8, 10)]
        );
        assert_eq!(subtract(&[(0, 4)], &[(0, 10)]), []);
        assert_eq!(intersect(&[(0, 10)], &[(5, 20)]), [(5, 10)]);
    }
}
