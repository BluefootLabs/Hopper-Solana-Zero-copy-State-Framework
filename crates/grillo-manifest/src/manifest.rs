//! Parsed view of a Hopper program manifest, narrowed to the **mutation
//! contract**: for each instruction, which byte ranges of which accounts
//! it is authorized to write, and (when declared) which accounts may have
//! their lamports moved.
//!
//! This parses the REAL `hopper.manifest.json` that
//! `hopper compile --emit manifest` emits (renderer:
//! `hopper_schema::codama::ManifestJson`). It does not define a rival
//! schema — the field names below (`strictWrites`, `writeRanges`,
//! `accountIndex`, `mutationComplete`, `lamportAccounts`) are exactly the
//! keys that renderer writes.

use serde::{Deserialize, Serialize};

/// One authorized byte-range write permission on a single account.
///
/// `account_index` is the account's position in the instruction's account
/// list — the same index the runtime `Context` and the touch map's `slot`
/// use, so a verifier can join the three directly.
///
/// A whole-account write permission is published as `offset = 0,
/// size = u32::MAX`; use [`end`](RangeContract::end), which widens to
/// `u64`, to avoid overflow when computing the exclusive end.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RangeContract {
    /// Position of the target account in the instruction's account list.
    #[serde(rename = "accountIndex")]
    pub account_index: u8,
    /// Byte offset of the authorized range within the account data.
    pub offset: u32,
    /// Byte length of the authorized range.
    pub size: u32,
}

impl RangeContract {
    /// Exclusive end offset, widened to `u64` so a `u32::MAX`-sized
    /// whole-account range cannot overflow.
    pub fn end(&self) -> u64 {
        self.offset as u64 + self.size as u64
    }

    /// Whether byte `offset` falls inside this range.
    pub fn contains_byte(&self, offset: u32) -> bool {
        (offset as u64) >= self.offset as u64 && (offset as u64) < self.end()
    }
}

/// The role an account plays in an instruction, from the manifest's
/// positional account list.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountRole {
    /// Declared account name.
    pub name: String,
    /// Whether the account is declared writable at the Sealevel level.
    #[serde(default)]
    pub writable: bool,
    /// Whether the account is a required signer.
    #[serde(default)]
    pub signer: bool,
}

/// The mutation contract for one instruction.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstructionContract {
    /// Instruction name.
    pub name: String,
    /// Instruction discriminator tag.
    pub tag: u8,
    /// Whether the context was compiled with `strict_writes`: the
    /// [`authorized`](Self::authorized) ranges are then the COMPLETE,
    /// enforced data-write surface. When `false`, the ranges carry no
    /// authority and a verifier cannot conclude anything about the byte
    /// write set (the pre-`strict_writes` contract).
    #[serde(rename = "strictWrites")]
    pub strict_writes: bool,
    /// Whether the write set covers BOTH mutation dimensions — data byte
    /// ranges AND lamport balances. Only `true` when the context declared
    /// `strict_writes` + `lamports(...)`; the manifest omits the key
    /// otherwise, so an older manifest is never mistaken for a completeness
    /// claim.
    #[serde(rename = "mutationComplete", default)]
    pub mutation_complete: bool,
    /// The authorized byte-range write set (manifest key `writeRanges`).
    #[serde(rename = "writeRanges", default)]
    pub authorized: Vec<RangeContract>,
    /// Account indices permitted to have their lamports mutated. Carries
    /// authority only when [`mutation_complete`](Self::mutation_complete)
    /// is `true`.
    #[serde(rename = "lamportAccounts", default)]
    pub lamport_accounts: Vec<u8>,
    /// The instruction's positional account list.
    #[serde(default)]
    pub accounts: Vec<AccountRole>,
}

impl InstructionContract {
    /// Name of the account at `index` in this instruction's account list.
    pub fn account_name(&self, index: u8) -> Option<&str> {
        self.accounts.get(index as usize).map(|a| a.name.as_str())
    }

    /// Whether `index` is authorized to have its lamports mutated. Always
    /// `false` unless the instruction is mutation-complete.
    pub fn authorizes_lamports(&self, index: u8) -> bool {
        self.mutation_complete && self.lamport_accounts.contains(&index)
    }

    /// The authorized ranges targeting `account_index`.
    pub fn authorized_for(&self, account_index: u8) -> impl Iterator<Item = &RangeContract> {
        self.authorized
            .iter()
            .filter(move |r| r.account_index == account_index)
    }
}

/// A Hopper program manifest, parsed down to its mutation contract.
///
/// Construct with [`MutationManifest::from_json`], which parses the real
/// emitted JSON and fails loudly on a manifest that predates the byte-range
/// mutation contract (see [`ParseError`]).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MutationManifest {
    /// Program name (manifest key `name`).
    #[serde(rename = "name")]
    pub program_name: String,
    /// Program version (manifest key `version`).
    #[serde(rename = "version")]
    pub program_version: String,
    /// Per-instruction mutation contracts, in manifest order.
    #[serde(default)]
    pub instructions: Vec<InstructionContract>,
}

/// Why a manifest could not be parsed into a [`MutationManifest`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParseError {
    /// The input was not valid JSON, or a value had the wrong shape/type
    /// for the mutation-contract fields.
    Json(String),
    /// The JSON parsed, but it is not a mutation-contract manifest Grillo
    /// understands — most commonly a manifest that predates the byte-range
    /// write contract (no per-instruction `strictWrites` field). Grillo
    /// refuses it rather than silently treating undeclared writes as
    /// unconstrained.
    UnsupportedManifest(String),
}

impl core::fmt::Display for ParseError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ParseError::Json(msg) => write!(f, "manifest JSON error: {msg}"),
            ParseError::UnsupportedManifest(msg) => write!(f, "unsupported manifest: {msg}"),
        }
    }
}

impl std::error::Error for ParseError {}

impl MutationManifest {
    /// Parse a real `hopper.manifest.json` string into a mutation-contract
    /// view.
    ///
    /// # Version gate
    ///
    /// The Hopper manifest carries no explicit *format*-version field — only
    /// a program `version`. Grillo therefore gates on a structural marker
    /// the codama renderer emits UNCONDITIONALLY for every instruction:
    /// `strictWrites`. A manifest whose instructions lack it predates the
    /// byte-range mutation contract, and Grillo returns
    /// [`ParseError::UnsupportedManifest`] rather than guess. (This is the
    /// one documented delta from the task brief, which anticipated an
    /// explicit version field.)
    pub fn from_json(json: &str) -> Result<Self, ParseError> {
        let value: serde_json::Value =
            serde_json::from_str(json).map_err(|e| ParseError::Json(e.to_string()))?;

        let Some(instructions) = value.get("instructions").and_then(|v| v.as_array()) else {
            return Err(ParseError::UnsupportedManifest(
                "manifest has no `instructions` array; not a Hopper program manifest".to_string(),
            ));
        };
        for ix in instructions {
            if ix.get("strictWrites").is_none() {
                let name = ix
                    .get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or("<unnamed>");
                return Err(ParseError::UnsupportedManifest(format!(
                    "instruction `{name}` has no `strictWrites` field; this manifest predates \
                     the Hopper byte-range mutation contract — regenerate it with \
                     `hopper compile --emit manifest`"
                )));
            }
        }

        serde_json::from_value(value).map_err(|e| ParseError::Json(e.to_string()))
    }

    /// Look up an instruction's contract by name.
    pub fn instruction(&self, name: &str) -> Option<&InstructionContract> {
        self.instructions.iter().find(|i| i.name == name)
    }

    /// Look up an instruction's contract by discriminator tag.
    pub fn instruction_by_tag(&self, tag: u8) -> Option<&InstructionContract> {
        self.instructions.iter().find(|i| i.tag == tag)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn range_end_widens_and_does_not_overflow() {
        let whole = RangeContract {
            account_index: 0,
            offset: 0,
            size: u32::MAX,
        };
        assert_eq!(whole.end(), u32::MAX as u64);
        assert!(whole.contains_byte(0));
        assert!(whole.contains_byte(u32::MAX - 1));

        let r = RangeContract {
            account_index: 1,
            offset: 114,
            size: 1,
        };
        assert!(r.contains_byte(114));
        assert!(!r.contains_byte(115));
    }

    #[test]
    fn legacy_manifest_without_strict_writes_is_refused() {
        // Shape of the pre-mutation-contract manifest: an instruction with
        // no `strictWrites` key. Grillo must fail loudly, not treat the
        // undeclared write set as unconstrained.
        let legacy = r#"{
            "name": "legacy",
            "version": "0.1.0",
            "instructions": [
                { "name": "increment", "tag": 0, "accounts": [] }
            ]
        }"#;
        let err = MutationManifest::from_json(legacy).unwrap_err();
        match err {
            ParseError::UnsupportedManifest(msg) => {
                assert!(
                    msg.contains("increment"),
                    "message names the instruction: {msg}"
                );
                assert!(
                    msg.contains("strictWrites"),
                    "message names the gate: {msg}"
                );
            }
            other => panic!("expected UnsupportedManifest, got {other:?}"),
        }
    }

    #[test]
    fn non_manifest_json_is_refused() {
        let err = MutationManifest::from_json(r#"{"foo": 1}"#).unwrap_err();
        assert!(matches!(err, ParseError::UnsupportedManifest(_)));
    }

    #[test]
    fn malformed_json_is_a_json_error() {
        let err = MutationManifest::from_json("{not json").unwrap_err();
        assert!(matches!(err, ParseError::Json(_)));
    }

    #[test]
    fn parses_a_minimal_strict_writes_instruction() {
        let json = r#"{
            "name": "p",
            "version": "1.0.0",
            "instructions": [
                {
                    "name": "pause",
                    "tag": 1,
                    "accounts": [
                        { "name": "admin", "writable": false, "signer": true },
                        { "name": "config", "writable": true, "signer": false, "layoutRef": "Config" }
                    ],
                    "strictWrites": true,
                    "writeRanges": [
                        { "account": "config", "accountIndex": 1, "offset": 114, "size": 1 },
                        { "account": "config", "accountIndex": 1, "offset": 115, "size": 8 }
                    ]
                }
            ]
        }"#;
        let m = MutationManifest::from_json(json).unwrap();
        let ix = m.instruction("pause").unwrap();
        assert!(ix.strict_writes);
        assert!(!ix.mutation_complete);
        assert_eq!(
            ix.authorized,
            vec![
                RangeContract {
                    account_index: 1,
                    offset: 114,
                    size: 1
                },
                RangeContract {
                    account_index: 1,
                    offset: 115,
                    size: 8
                },
            ]
        );
        assert_eq!(ix.account_name(1), Some("config"));
        assert!(
            !ix.authorizes_lamports(1),
            "not mutation-complete => no lamport authority"
        );
    }

    #[test]
    fn ignores_unknown_manifest_fields() {
        // The real manifest carries layouts/events/policies/receiptSchema/…
        // that Grillo does not model; they must be ignored, not rejected.
        let json = r#"{
            "name": "p",
            "version": "1.0.0",
            "description": "x",
            "layouts": [ { "name": "L", "disc": 1, "unknown": 7 } ],
            "policies": [],
            "receiptSchema": { "size": 72 },
            "instructions": [
                { "name": "i", "tag": 0, "args": [], "capabilities": [],
                  "policyPack": "", "receiptExpected": false,
                  "strictWrites": false, "writeRanges": [] }
            ]
        }"#;
        let m = MutationManifest::from_json(json).unwrap();
        assert_eq!(m.program_name, "p");
        assert_eq!(m.instructions.len(), 1);
        assert!(!m.instructions[0].strict_writes);
    }
}
