//! Invocation resolution for parametric mutation contracts.
//!
//! A Hopper manifest publishes conservative static write ranges plus exact-cell
//! rules selected by instruction arguments.  This module binds those rules to
//! the REAL wire payload and produces the effective authorization set:
//!
//! ```text
//! selected cells U (static authorized bytes - every governed envelope)
//! ```
//!
//! Resolution is deliberately fail-closed.  Malformed descriptors, missing or
//! truncated arguments, overlapping envelopes, selectors outside their cell
//! count, and envelopes not covered by the static contract are all errors.

use core::fmt;

use crate::manifest::{
    ArgContract, ArgEncodingContract, InstructionContract, MutationContractView,
    ParametricRangeContract, RangeContract,
};
use crate::sha256::sha256;

/// One selector value bound from an invocation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedSelector {
    /// Compact selector index used by Hopper's runtime write policy.
    pub argument_index: u8,
    /// Stable instruction-argument name.
    pub name: String,
    /// Decoded unsigned selector value.
    pub value: u32,
}

/// A concrete instruction effect contract after all parametric rules have
/// been bound to invocation arguments.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedInstructionContract {
    /// Instruction name.
    name: String,
    /// One-byte manifest tag.
    tag: u8,
    /// Whether the data write set is complete.
    strict_writes: bool,
    /// Whether the lamport dimension is complete.
    mutation_complete: bool,
    /// Effective, invocation-specific authorized byte ranges.
    authorized: Vec<RangeContract>,
    /// Accounts permitted to mutate lamports.
    lamport_accounts: Vec<u8>,
    /// Concrete selector values that produced this contract.
    selectors: Vec<ResolvedSelector>,
    /// Commitment to the unresolved instruction contract, including all
    /// parametric rules and wire argument descriptors.
    source_commitment: [u8; 32],
}

impl MutationContractView for ResolvedInstructionContract {
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
}

impl ResolvedInstructionContract {
    /// Instruction name pinned by the source contract.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// One-byte manifest tag pinned by the source contract.
    pub fn tag(&self) -> u8 {
        self.tag
    }

    /// Whether the data write set is complete.
    pub fn strict_writes(&self) -> bool {
        self.strict_writes
    }

    /// Whether the lamport permission set is complete.
    pub fn mutation_complete(&self) -> bool {
        self.mutation_complete
    }

    /// Effective invocation-specific authorized ranges.
    pub fn authorized_ranges(&self) -> &[RangeContract] {
        &self.authorized
    }

    /// Accounts permitted to mutate lamports.
    pub fn lamport_accounts(&self) -> &[u8] {
        &self.lamport_accounts
    }

    /// Selector values used to resolve this invocation.
    pub fn selectors(&self) -> &[ResolvedSelector] {
        &self.selectors
    }

    /// Commitment to the unresolved source contract.
    pub fn source_commitment(&self) -> [u8; 32] {
        self.source_commitment
    }

    /// Stable SHA-256 certificate for this concrete invocation effect.
    ///
    /// The source commitment pins the published rule set.  The selector list
    /// and resolved ranges pin the invocation-specific result, so changing a
    /// slot or any exact-cell rule changes this digest.
    pub fn commitment(&self) -> [u8; 32] {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"grillo.resolved-effect.v2");
        bytes.extend_from_slice(&self.source_commitment);
        bytes.extend_from_slice(&(self.name.len() as u32).to_le_bytes());
        bytes.extend_from_slice(self.name.as_bytes());
        bytes.push(self.tag);
        bytes.push(self.strict_writes as u8);
        bytes.push(self.mutation_complete as u8);
        bytes.extend_from_slice(&(self.lamport_accounts.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&self.lamport_accounts);
        bytes.extend_from_slice(&(self.selectors.len() as u32).to_le_bytes());
        for selector in &self.selectors {
            bytes.push(selector.argument_index);
            bytes.extend_from_slice(&(selector.name.len() as u32).to_le_bytes());
            bytes.extend_from_slice(selector.name.as_bytes());
            bytes.extend_from_slice(&selector.value.to_le_bytes());
        }
        bytes.extend_from_slice(&(self.authorized.len() as u32).to_le_bytes());
        for range in &self.authorized {
            bytes.push(range.account_index);
            bytes.extend_from_slice(&range.offset.to_le_bytes());
            bytes.extend_from_slice(&range.size.to_le_bytes());
        }
        sha256(&bytes)
    }

    /// Effective ranges for one positional instruction account.
    pub fn authorized_for(&self, account_index: u8) -> impl Iterator<Item = &RangeContract> {
        self.authorized
            .iter()
            .filter(move |range| range.account_index == account_index)
    }
}

/// Why an invocation-specific effect contract could not be resolved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResolveError {
    /// A parametric selector has no matching argument descriptor.
    MissingArgumentDescriptor { argument: String },
    /// The instruction payload ended while decoding an argument.
    TruncatedArgument { argument: String },
    /// An argument descriptor is internally inconsistent.
    InvalidArgumentDescriptor {
        argument: String,
        reason: &'static str,
    },
    /// Exact-cell selectors are intentionally restricted to fixed unsigned
    /// integer wire types so off-chain and on-chain casts cannot diverge.
    UnsupportedSelectorType { argument: String, ty: String },
    /// A decoded selector is outside the declared cell count.
    SelectorOutOfRange {
        argument: String,
        value: u32,
        count: u32,
    },
    /// A parametric rule is malformed or overflows its offset arithmetic.
    InvalidRule {
        segment: String,
        reason: &'static str,
    },
    /// Two rules on the same account govern overlapping envelopes.  Hopper's
    /// runtime gives the first rule precedence, so accepting a set union here
    /// would be unsound.
    OverlappingRules {
        first_segment: String,
        second_segment: String,
    },
    /// A rule could broaden authority beyond the conservative static set.
    EnvelopeNotStaticallyAuthorized { account_index: u8, segment: String },
    /// Caller supplied fewer already-decoded compact selectors than the rule
    /// references.
    MissingSelectorValue {
        argument: String,
        argument_index: u8,
    },
    /// Two rules disagree about the name/index identity of one selector.
    InconsistentSelectorIdentity {
        argument: String,
        argument_index: u8,
    },
}

impl fmt::Display for ResolveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingArgumentDescriptor { argument } => {
                write!(f, "no wire descriptor for parametric argument `{argument}`")
            }
            Self::TruncatedArgument { argument } => {
                write!(f, "instruction payload is truncated at argument `{argument}`")
            }
            Self::InvalidArgumentDescriptor { argument, reason } => {
                write!(f, "invalid descriptor for argument `{argument}`: {reason}")
            }
            Self::UnsupportedSelectorType { argument, ty } => write!(
                f,
                "parametric argument `{argument}` has unsupported type `{ty}`; expected u8, u16, or u32"
            ),
            Self::SelectorOutOfRange {
                argument,
                value,
                count,
            } => write!(
                f,
                "parametric argument `{argument}` selects cell {value}, but count is {count}"
            ),
            Self::InvalidRule { segment, reason } => {
                write!(f, "invalid parametric rule for `{segment}`: {reason}")
            }
            Self::OverlappingRules {
                first_segment,
                second_segment,
            } => write!(
                f,
                "parametric envelopes `{first_segment}` and `{second_segment}` overlap"
            ),
            Self::EnvelopeNotStaticallyAuthorized {
                account_index,
                segment,
            } => write!(
                f,
                "parametric envelope `{segment}` on account {account_index} is not covered by the static contract"
            ),
            Self::MissingSelectorValue {
                argument,
                argument_index,
            } => write!(
                f,
                "missing selector {argument_index} for parametric argument `{argument}`"
            ),
            Self::InconsistentSelectorIdentity {
                argument,
                argument_index,
            } => write!(
                f,
                "parametric selector {argument_index} and name `{argument}` are inconsistent across rules"
            ),
        }
    }
}

impl std::error::Error for ResolveError {}

impl InstructionContract {
    /// Resolve exact-cell effects from the actual instruction **argument
    /// payload** (the bytes after the instruction discriminator).
    ///
    /// Hopper manifests currently expose a one-byte `tag`, not the complete
    /// discriminator byte string used by imported multi-byte ABIs.  Accepting
    /// the post-discriminator payload keeps this API exact for both cases: the
    /// transaction/IDL decoder that selected this instruction also knows the
    /// discriminator length and passes the remaining bytes here.
    pub fn resolve_effects(
        &self,
        argument_payload: &[u8],
    ) -> Result<ResolvedInstructionContract, ResolveError> {
        let named = decode_selector_values(&self.args, &self.parametric, argument_payload)?;
        self.resolve_with(|rule| {
            named
                .iter()
                .find(|(name, _)| name == &rule.argument_name)
                .map(|(_, value)| *value)
        })
    }

    /// Resolve from selector values already decoded in Hopper's compact
    /// parametric-index order.
    ///
    /// This is useful for SVM integrations that already decoded handler args;
    /// independent transaction verifiers should prefer [`resolve_effects`]
    /// so the certificate is bound directly to wire bytes.
    pub fn resolve_effects_with_selectors(
        &self,
        selectors: &[u32],
    ) -> Result<ResolvedInstructionContract, ResolveError> {
        self.resolve_with(|rule| selectors.get(rule.argument_index as usize).copied())
    }

    fn resolve_with(
        &self,
        mut selector_for: impl FnMut(&ParametricRangeContract) -> Option<u32>,
    ) -> Result<ResolvedInstructionContract, ResolveError> {
        validate_rules(self)?;

        let mut effective = self.authorized.clone();
        let mut selected_ranges = Vec::with_capacity(self.parametric.len());
        let mut selectors: Vec<ResolvedSelector> = Vec::new();

        for rule in &self.parametric {
            let value = selector_for(rule).ok_or_else(|| ResolveError::MissingSelectorValue {
                argument: rule.argument_name.clone(),
                argument_index: rule.argument_index,
            })?;
            let selected =
                rule.selected_range(value)
                    .ok_or_else(|| ResolveError::SelectorOutOfRange {
                        argument: rule.argument_name.clone(),
                        value,
                        count: rule.count,
                    })?;

            if let Some(existing) = selectors.iter().find(|selector| {
                selector.argument_index == rule.argument_index
                    || selector.name == rule.argument_name
            }) {
                if existing.argument_index != rule.argument_index
                    || existing.name != rule.argument_name
                    || existing.value != value
                {
                    return Err(ResolveError::InconsistentSelectorIdentity {
                        argument: rule.argument_name.clone(),
                        argument_index: rule.argument_index,
                    });
                }
            } else {
                selectors.push(ResolvedSelector {
                    argument_index: rule.argument_index,
                    name: rule.argument_name.clone(),
                    value,
                });
            }
            selected_ranges.push(selected);
        }

        // Parametric rules take authority over every byte in their envelopes;
        // remove those bytes from every static grant before adding only the
        // selected cells back.
        for rule in &self.parametric {
            let envelope_end = rule
                .envelope_end()
                .ok_or_else(|| ResolveError::InvalidRule {
                    segment: rule.segment_name.clone(),
                    reason: "column envelope overflows",
                })?;
            let mut next = Vec::with_capacity(effective.len() + 1);
            for range in effective {
                subtract_envelope(
                    &range,
                    rule.account_index,
                    rule.base_offset as u64,
                    envelope_end,
                    &mut next,
                );
            }
            effective = next;
        }
        effective.extend(selected_ranges);
        effective.sort_by_key(|range| (range.account_index, range.offset, range.size));
        selectors.sort_by_key(|selector| selector.argument_index);

        Ok(ResolvedInstructionContract {
            name: self.name.clone(),
            tag: self.tag,
            strict_writes: self.strict_writes,
            mutation_complete: self.mutation_complete,
            authorized: effective,
            lamport_accounts: self.lamport_accounts.clone(),
            selectors,
            source_commitment: self.commitment(),
        })
    }
}

fn validate_rules(contract: &InstructionContract) -> Result<(), ResolveError> {
    for rule in &contract.parametric {
        if rule.count == 0 {
            return Err(ResolveError::InvalidRule {
                segment: rule.segment_name.clone(),
                reason: "cell count is zero",
            });
        }
        if rule.cell_size == 0 {
            return Err(ResolveError::InvalidRule {
                segment: rule.segment_name.clone(),
                reason: "cell size is zero",
            });
        }
        if rule.count > 1 && rule.stride < rule.cell_size {
            return Err(ResolveError::InvalidRule {
                segment: rule.segment_name.clone(),
                reason: "cell stride is smaller than cell size",
            });
        }
        let end = rule
            .envelope_end()
            .ok_or_else(|| ResolveError::InvalidRule {
                segment: rule.segment_name.clone(),
                reason: "column envelope overflows",
            })?;
        if end > u32::MAX as u64 {
            return Err(ResolveError::InvalidRule {
                segment: rule.segment_name.clone(),
                reason: "column envelope exceeds the u32 account address space",
            });
        }
        let static_cover: Vec<(u64, u64)> = contract
            .authorized
            .iter()
            .filter(|range| range.account_index == rule.account_index)
            .map(|range| (range.offset as u64, range.end()))
            .collect();
        if first_gap(rule.base_offset as u64, end, &static_cover).is_some() {
            return Err(ResolveError::EnvelopeNotStaticallyAuthorized {
                account_index: rule.account_index,
                segment: rule.segment_name.clone(),
            });
        }
    }

    for left_index in 0..contract.parametric.len() {
        let left = &contract.parametric[left_index];
        let left_end = left
            .envelope_end()
            .ok_or_else(|| ResolveError::InvalidRule {
                segment: left.segment_name.clone(),
                reason: "column envelope overflows",
            })?;
        for right in &contract.parametric[left_index + 1..] {
            if left.account_index != right.account_index {
                continue;
            }
            let right_end = right
                .envelope_end()
                .ok_or_else(|| ResolveError::InvalidRule {
                    segment: right.segment_name.clone(),
                    reason: "column envelope overflows",
                })?;
            if (left.base_offset as u64) < right_end && (right.base_offset as u64) < left_end {
                return Err(ResolveError::OverlappingRules {
                    first_segment: left.segment_name.clone(),
                    second_segment: right.segment_name.clone(),
                });
            }
        }
    }

    // One compact runtime selector index must name exactly one wire arg, and
    // one wire arg must map to exactly one compact index.
    for (index, left) in contract.parametric.iter().enumerate() {
        for right in &contract.parametric[index + 1..] {
            if (left.argument_index == right.argument_index)
                != (left.argument_name == right.argument_name)
            {
                return Err(ResolveError::InconsistentSelectorIdentity {
                    argument: right.argument_name.clone(),
                    argument_index: right.argument_index,
                });
            }
        }
    }
    Ok(())
}

fn decode_selector_values(
    args: &[ArgContract],
    rules: &[ParametricRangeContract],
    payload: &[u8],
) -> Result<Vec<(String, u32)>, ResolveError> {
    for (index, left) in args.iter().enumerate() {
        if args[index + 1..]
            .iter()
            .any(|right| right.name == left.name)
        {
            return Err(ResolveError::InvalidArgumentDescriptor {
                argument: left.name.clone(),
                reason: "duplicate argument name",
            });
        }
    }

    for rule in rules {
        if !args.iter().any(|arg| arg.name == rule.argument_name) {
            return Err(ResolveError::MissingArgumentDescriptor {
                argument: rule.argument_name.clone(),
            });
        }
    }

    let mut selector_names: Vec<&str> = Vec::new();
    for rule in rules {
        if !selector_names.contains(&rule.argument_name.as_str()) {
            selector_names.push(&rule.argument_name);
        }
    }

    let mut cursor = 0usize;
    let mut decoded = Vec::new();
    for arg in args {
        // Everything after the final selector is irrelevant to effect
        // resolution.  Stopping here also avoids pretending we can skip an
        // arbitrary bounded-vector element codec from max-size metadata.
        if decoded.len() == selector_names.len() {
            break;
        }
        let selected = rules.iter().any(|rule| rule.argument_name == arg.name);
        match arg.encoding {
            ArgEncodingContract::Fixed => {
                if arg.size == 0 {
                    return Err(ResolveError::InvalidArgumentDescriptor {
                        argument: arg.name.clone(),
                        reason:
                            "cannot locate a selector after a zero-width or unknown fixed encoding",
                    });
                }
                let end = cursor.checked_add(arg.size as usize).ok_or_else(|| {
                    ResolveError::InvalidArgumentDescriptor {
                        argument: arg.name.clone(),
                        reason: "fixed size overflows",
                    }
                })?;
                let bytes =
                    payload
                        .get(cursor..end)
                        .ok_or_else(|| ResolveError::TruncatedArgument {
                            argument: arg.name.clone(),
                        })?;
                if selected {
                    let value = decode_unsigned_selector(arg, bytes)?;
                    decoded.push((arg.name.clone(), value));
                }
                cursor = end;
            }
            ArgEncodingContract::BoundedVec => {
                if selected {
                    return Err(ResolveError::UnsupportedSelectorType {
                        argument: arg.name.clone(),
                        ty: arg.canonical_type.clone(),
                    });
                }
                // `elementSize` is a MAX encoded size, not necessarily an
                // exact stride (nested bounded codecs may consume less).  The
                // current manifest cannot locate a later selector soundly, so
                // fail closed instead of advancing by `len * elementSize`.
                return Err(ResolveError::InvalidArgumentDescriptor {
                    argument: arg.name.clone(),
                    reason: "cannot locate a later selector after boundedVec without an exact element encoding",
                });
            }
            ArgEncodingContract::BoundedString => {
                if selected {
                    return Err(ResolveError::UnsupportedSelectorType {
                        argument: arg.name.clone(),
                        ty: arg.canonical_type.clone(),
                    });
                }
                let (len, after_len) = read_u16_len(payload, cursor, &arg.name)?;
                let max_len =
                    arg.max_len
                        .ok_or_else(|| ResolveError::InvalidArgumentDescriptor {
                            argument: arg.name.clone(),
                            reason: "bounded string has no maxLen",
                        })?;
                if len > max_len {
                    return Err(ResolveError::InvalidArgumentDescriptor {
                        argument: arg.name.clone(),
                        reason: "bounded string length exceeds maxLen",
                    });
                }
                cursor = after_len.checked_add(len as usize).ok_or_else(|| {
                    ResolveError::InvalidArgumentDescriptor {
                        argument: arg.name.clone(),
                        reason: "bounded string length overflows",
                    }
                })?;
                if cursor > payload.len() {
                    return Err(ResolveError::TruncatedArgument {
                        argument: arg.name.clone(),
                    });
                }
            }
        }
    }
    Ok(decoded)
}

fn decode_unsigned_selector(arg: &ArgContract, bytes: &[u8]) -> Result<u32, ResolveError> {
    match (arg.canonical_type.as_str(), bytes) {
        ("u8", [value]) => Ok(*value as u32),
        ("u16", [a, b]) => Ok(u16::from_le_bytes([*a, *b]) as u32),
        ("u32", [a, b, c, d]) => Ok(u32::from_le_bytes([*a, *b, *c, *d])),
        _ => Err(ResolveError::UnsupportedSelectorType {
            argument: arg.name.clone(),
            ty: arg.canonical_type.clone(),
        }),
    }
}

fn read_u16_len(
    payload: &[u8],
    cursor: usize,
    argument: &str,
) -> Result<(u16, usize), ResolveError> {
    let bytes = payload
        .get(cursor..cursor.saturating_add(2))
        .ok_or_else(|| ResolveError::TruncatedArgument {
            argument: argument.to_string(),
        })?;
    Ok((u16::from_le_bytes([bytes[0], bytes[1]]), cursor + 2))
}

fn subtract_envelope(
    range: &RangeContract,
    account_index: u8,
    envelope_start: u64,
    envelope_end: u64,
    out: &mut Vec<RangeContract>,
) {
    if range.account_index != account_index
        || range.end() <= envelope_start
        || range.offset as u64 >= envelope_end
    {
        out.push(*range);
        return;
    }

    let range_start = range.offset as u64;
    let range_end = range.end();
    if range_start < envelope_start {
        out.push(RangeContract {
            account_index,
            offset: range.offset,
            size: (envelope_start - range_start) as u32,
        });
    }
    if range_end > envelope_end {
        out.push(RangeContract {
            account_index,
            offset: envelope_end as u32,
            size: (range_end - envelope_end) as u32,
        });
    }
}

fn first_gap(start: u64, end: u64, cover: &[(u64, u64)]) -> Option<u64> {
    let mut cursor = start;
    while cursor < end {
        let mut furthest = cursor;
        for &(cover_start, cover_end) in cover {
            if cover_start <= cursor && cursor < cover_end && cover_end > furthest {
                furthest = cover_end;
            }
        }
        if furthest == cursor {
            return Some(cursor);
        }
        cursor = furthest;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{AccountRole, ArgContract, ArgEncodingContract};

    fn contract() -> InstructionContract {
        InstructionContract {
            name: "settle".to_string(),
            tag: 7,
            strict_writes: true,
            mutation_complete: false,
            authorized: vec![RangeContract {
                account_index: 1,
                offset: 90,
                size: 60,
            }],
            parametric: vec![ParametricRangeContract {
                account_index: 1,
                base_offset: 100,
                stride: 4,
                cell_size: 4,
                count: 10,
                argument_index: 0,
                argument_name: "slot".to_string(),
                segment_name: "statuses".to_string(),
            }],
            lamport_accounts: vec![],
            args: vec![ArgContract {
                name: "slot".to_string(),
                canonical_type: "u16".to_string(),
                size: 2,
                encoding: ArgEncodingContract::Fixed,
                max_len: None,
                element_size: None,
            }],
            accounts: vec![AccountRole {
                name: "shard".to_string(),
                writable: true,
                signer: false,
            }],
        }
    }

    #[test]
    fn payload_resolves_static_minus_envelope_plus_selected_cell() {
        let resolved = contract().resolve_effects(&3u16.to_le_bytes()).unwrap();
        assert_eq!(
            resolved.authorized_ranges(),
            vec![
                RangeContract {
                    account_index: 1,
                    offset: 90,
                    size: 10,
                },
                RangeContract {
                    account_index: 1,
                    offset: 112,
                    size: 4,
                },
                RangeContract {
                    account_index: 1,
                    offset: 140,
                    size: 10,
                },
            ]
        );
        assert!(resolved
            .authorized_ranges()
            .iter()
            .any(|r| r.contains_byte(112)));
        assert!(!resolved
            .authorized_ranges()
            .iter()
            .any(|r| r.contains_byte(116)));
    }

    #[test]
    fn selector_changes_resolved_commitment() {
        let a = contract().resolve_effects(&3u16.to_le_bytes()).unwrap();
        let b = contract().resolve_effects(&4u16.to_le_bytes()).unwrap();
        assert_ne!(a.commitment(), b.commitment());
    }

    #[test]
    fn every_verifier_relevant_resolved_field_is_committed() {
        let resolved = contract().resolve_effects(&3u16.to_le_bytes()).unwrap();
        let baseline = resolved.commitment();

        let mut altered = resolved.clone();
        altered.mutation_complete = true;
        altered.lamport_accounts.push(1);
        assert_ne!(baseline, altered.commitment());

        let mut altered = resolved.clone();
        altered.strict_writes = false;
        assert_ne!(baseline, altered.commitment());
    }

    #[test]
    fn malformed_selector_payload_fails_closed() {
        assert!(matches!(
            contract().resolve_effects(&[3]),
            Err(ResolveError::TruncatedArgument { .. })
        ));
        assert!(matches!(
            contract().resolve_effects(&10u16.to_le_bytes()),
            Err(ResolveError::SelectorOutOfRange { .. })
        ));
    }

    #[test]
    fn overlapping_envelopes_are_rejected() {
        let mut c = contract();
        let mut second = c.parametric[0].clone();
        second.base_offset = 108;
        second.segment_name = "revisions".to_string();
        c.parametric.push(second);
        assert!(matches!(
            c.resolve_effects(&3u16.to_le_bytes()),
            Err(ResolveError::OverlappingRules { .. })
        ));
    }

    #[test]
    fn envelope_must_be_statically_declared() {
        let mut c = contract();
        c.authorized[0] = RangeContract {
            account_index: 1,
            offset: 0,
            size: 50,
        };
        assert!(matches!(
            c.resolve_effects(&3u16.to_le_bytes()),
            Err(ResolveError::EnvelopeNotStaticallyAuthorized { .. })
        ));
    }

    #[test]
    fn bounded_vector_before_selector_fails_closed_without_exact_element_shape() {
        let mut c = contract();
        c.args.insert(
            0,
            ArgContract {
                name: "route".to_string(),
                canonical_type: "HopperVec<u8, 8>".to_string(),
                size: 10,
                encoding: ArgEncodingContract::BoundedVec,
                max_len: Some(8),
                element_size: Some(1),
            },
        );
        let payload = [2, 0, 0xaa, 0xbb, 4, 0];
        assert!(matches!(
            c.resolve_effects(&payload),
            Err(ResolveError::InvalidArgumentDescriptor { .. })
        ));
    }

    #[test]
    fn unknown_fixed_argument_before_selector_fails_closed() {
        let mut c = contract();
        c.args.insert(
            0,
            ArgContract {
                name: "custom".to_string(),
                canonical_type: "CustomCodec".to_string(),
                size: 0,
                encoding: ArgEncodingContract::Fixed,
                max_len: None,
                element_size: None,
            },
        );
        assert!(matches!(
            c.resolve_effects(&[0xaa, 3, 0]),
            Err(ResolveError::InvalidArgumentDescriptor { argument, .. })
                if argument == "custom"
        ));
    }

    #[test]
    fn duplicate_argument_names_fail_closed() {
        let mut c = contract();
        c.args.insert(0, c.args[0].clone());
        assert!(matches!(
            c.resolve_effects(&[3, 0, 3, 0]),
            Err(ResolveError::InvalidArgumentDescriptor { reason, .. })
                if reason == "duplicate argument name"
        ));
    }
}
