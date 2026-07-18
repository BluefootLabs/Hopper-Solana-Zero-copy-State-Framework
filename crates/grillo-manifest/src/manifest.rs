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

/// One invocation-parametric byte-range rule from a Hopper manifest.
///
/// The rule narrows the conservative static column envelope to exactly one
/// fixed-size cell selected by an instruction argument.  The runtime enforces
/// the same formula before granting a write lease:
///
/// ```text
/// selected_start = base_offset + selector * stride
/// selected_range = [selected_start, selected_start + cell_size)
/// ```
///
/// Grillo must resolve these rules for the concrete invocation before it can
/// issue a byte-precise verdict.  Treating the static envelope as authority
/// would silently authorize neighboring cells that the program itself denies.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParametricRangeContract {
    /// Position of the governed account in the instruction account list.
    #[serde(rename = "accountIndex")]
    pub account_index: u8,
    /// Absolute byte offset of cell zero.
    #[serde(rename = "baseOffset")]
    pub base_offset: u32,
    /// Byte distance between consecutive cells.
    pub stride: u32,
    /// Writable byte length of the selected cell.
    #[serde(rename = "cellSize")]
    pub cell_size: u32,
    /// Number of selectable cells.
    pub count: u32,
    /// Compact selector position used by the runtime policy.
    #[serde(rename = "argumentIndex")]
    pub argument_index: u8,
    /// Stable instruction-argument name used by framework-neutral resolvers.
    #[serde(rename = "argument")]
    pub argument_name: String,
    /// Stable layout segment/field name for diagnostics.
    #[serde(rename = "segment")]
    pub segment_name: String,
}

impl ParametricRangeContract {
    /// Exclusive end of the entire governed column envelope.
    pub fn envelope_end(&self) -> Option<u64> {
        let last = self.stride as u64 * self.count.checked_sub(1)? as u64;
        (self.base_offset as u64)
            .checked_add(last)?
            .checked_add(self.cell_size as u64)
    }

    /// Resolve the selected cell, returning `None` for an invalid selector or
    /// arithmetic overflow.
    pub fn selected_range(&self, selector: u32) -> Option<RangeContract> {
        if selector >= self.count || self.cell_size == 0 {
            return None;
        }
        let offset = (self.base_offset as u64)
            .checked_add((self.stride as u64).checked_mul(selector as u64)?)?;
        let end = offset.checked_add(self.cell_size as u64)?;
        if offset > u32::MAX as u64 || end > u32::MAX as u64 {
            return None;
        }
        Some(RangeContract {
            account_index: self.account_index,
            offset: offset as u32,
            size: self.cell_size,
        })
    }
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

/// Wire encoding of one instruction argument, narrowed from Hopper's
/// manifest descriptor.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ArgEncodingContract {
    /// Exactly `size` bytes.
    Fixed,
    /// A little-endian `u16` element count followed by encoded elements.
    BoundedVec,
    /// A little-endian `u16` byte count followed by UTF-8 bytes.
    BoundedString,
}

impl Default for ArgEncodingContract {
    fn default() -> Self {
        Self::Fixed
    }
}

/// Instruction-argument metadata needed to decode parametric selectors from
/// the real invocation payload.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArgContract {
    /// Stable argument name.
    pub name: String,
    /// Canonical Hopper/Rust type spelling.
    #[serde(rename = "type")]
    pub canonical_type: String,
    /// Fixed size or maximum encoded size, depending on [`encoding`](Self::encoding).
    pub size: u16,
    /// Wire encoding shape.
    #[serde(default)]
    pub encoding: ArgEncodingContract,
    /// Maximum element/byte count for bounded encodings.
    #[serde(rename = "maxLen", default)]
    pub max_len: Option<u16>,
    /// Encoded byte size of one bounded-vector element.
    #[serde(rename = "elementSize", default)]
    pub element_size: Option<u16>,
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
    /// Invocation-parametric exact-cell rules (manifest key
    /// `parametricWriteRanges`).  A non-empty set MUST be resolved against the
    /// concrete instruction arguments before verification; the static
    /// [`authorized`](Self::authorized) ranges are only conservative column
    /// envelopes in those regions.
    #[serde(rename = "parametricWriteRanges", default)]
    pub parametric: Vec<ParametricRangeContract>,
    /// Account indices permitted to have their lamports mutated. Carries
    /// authority only when [`mutation_complete`](Self::mutation_complete)
    /// is `true`.
    #[serde(rename = "lamportAccounts", default)]
    pub lamport_accounts: Vec<u8>,
    /// Ordered wire arguments.  Grillo uses these descriptors to locate and
    /// decode exact-cell selectors from the actual invocation payload rather
    /// than trusting caller-supplied values.
    #[serde(default)]
    pub args: Vec<ArgContract>,
    /// The instruction's positional account list.
    #[serde(default)]
    pub accounts: Vec<AccountRole>,
}

/// Read-only view shared by unresolved and invocation-resolved effect
/// contracts.
///
/// Keeping the verifier generic over this trait prevents a caller from having
/// to copy a resolved contract back into the manifest's unresolved shape.
pub trait MutationContractView {
    /// Whether the byte-range contract is complete and enforced.
    fn strict_writes(&self) -> bool;
    /// Whether the lamport dimension is also complete.
    fn mutation_complete(&self) -> bool;
    /// Effective authorized byte ranges.
    fn authorized_ranges(&self) -> &[RangeContract];
    /// Account indices permitted to mutate lamports.
    fn lamport_accounts(&self) -> &[u8];
    /// Whether this is still a conservative contract that must be resolved
    /// against invocation arguments before it can authorize bytes.
    fn requires_resolution(&self) -> bool {
        false
    }

    /// Whether `account_index` is authorized to mutate lamports.
    fn authorizes_lamports(&self, account_index: u8) -> bool {
        self.mutation_complete() && self.lamport_accounts().contains(&account_index)
    }
}

impl MutationContractView for InstructionContract {
    fn strict_writes(&self) -> bool {
        self.strict_writes
    }

    fn mutation_complete(&self) -> bool {
        self.mutation_complete
    }

    fn authorized_ranges(&self) -> &[RangeContract] {
        &self.authorized
    }

    fn lamport_accounts(&self) -> &[u8] {
        &self.lamport_accounts
    }

    fn requires_resolution(&self) -> bool {
        !self.parametric.is_empty()
    }
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
    fn parses_parametric_rules_and_selector_wire_metadata() {
        let json = r#"{
            "name": "p",
            "version": "1.0.0",
            "instructions": [{
                "name": "claim",
                "tag": 4,
                "args": [
                    { "name": "slot", "type": "u16", "size": 2, "encoding": "fixed" }
                ],
                "accounts": [
                    { "name": "shard", "writable": true, "signer": false }
                ],
                "strictWrites": true,
                "writeRanges": [
                    { "accountIndex": 0, "offset": 100, "size": 20 }
                ],
                "parametricWriteRanges": [{
                    "accountIndex": 0,
                    "baseOffset": 100,
                    "stride": 1,
                    "cellSize": 1,
                    "count": 20,
                    "argumentIndex": 0,
                    "argument": "slot",
                    "segment": "statuses"
                }]
            }]
        }"#;
        let manifest = MutationManifest::from_json(json).unwrap();
        let claim = manifest.instruction("claim").unwrap();
        assert_eq!(claim.args[0].encoding, ArgEncodingContract::Fixed);
        assert_eq!(claim.parametric.len(), 1);
        assert_eq!(claim.parametric[0].argument_name, "slot");
        assert_eq!(claim.parametric[0].envelope_end(), Some(120));
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
