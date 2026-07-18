//! A stable commitment to a program's mutation contract.
//!
//! The commitment is `SHA-256` over a canonical, deterministic byte
//! encoding of the mutation contract (stable field order, little-endian
//! integers). Two manifests with an identical mutation contract hash equal;
//! ANY change to a range, a parametric selector rule, its wire-decoding
//! metadata, a flag, a lamport permission, or an instruction's identity
//! changes the hash. It is deliberately independent of manifest
//! cosmetics — account display names/roles, layouts, and program description
//! — so a commitment pins exactly the identity and wire inputs needed to
//! resolve "what bytes and lamports this program may move."

use crate::manifest::{ArgEncodingContract, InstructionContract, MutationManifest};
use crate::sha256::sha256;

/// Domain-separation tag + version for the canonical encoding. Bump the
/// trailing version if the encoding ever changes so commitments produced by
/// different encoders can never silently collide.
const COMMIT_DOMAIN: &[u8] = b"grillo.mutation-contract.v2";

impl InstructionContract {
    /// Append this instruction's canonical mutation-contract encoding to
    /// `out`.
    ///
    /// Layout (all integers little-endian):
    ///
    /// ```text
    /// u32  name length      | name bytes (utf-8)
    /// u8   tag
    /// u8   strict_writes    (0/1)
    /// u8   mutation_complete (0/1)
    /// u32  argument descriptor count
    ///   repeated: name | canonical type | size | encoding | bounded metadata
    /// u32  authorized range count
    ///   repeated: u8 account_index | u32 offset | u32 size
    /// u32  parametric rule count
    ///   repeated: every numeric field | argument name | segment name
    /// u32  lamport account count
    ///   repeated: u8 account_index
    /// ```
    pub fn encode_canonical(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&(self.name.len() as u32).to_le_bytes());
        out.extend_from_slice(self.name.as_bytes());
        out.push(self.tag);
        out.push(self.strict_writes as u8);
        out.push(self.mutation_complete as u8);

        out.extend_from_slice(&(self.args.len() as u32).to_le_bytes());
        for arg in &self.args {
            encode_string(out, &arg.name);
            encode_string(out, &arg.canonical_type);
            out.extend_from_slice(&arg.size.to_le_bytes());
            out.push(match arg.encoding {
                ArgEncodingContract::Fixed => 0,
                ArgEncodingContract::BoundedVec => 1,
                ArgEncodingContract::BoundedString => 2,
            });
            encode_optional_u16(out, arg.max_len);
            encode_optional_u16(out, arg.element_size);
        }

        out.extend_from_slice(&(self.authorized.len() as u32).to_le_bytes());
        for r in &self.authorized {
            out.push(r.account_index);
            out.extend_from_slice(&r.offset.to_le_bytes());
            out.extend_from_slice(&r.size.to_le_bytes());
        }

        out.extend_from_slice(&(self.parametric.len() as u32).to_le_bytes());
        for rule in &self.parametric {
            out.push(rule.account_index);
            out.extend_from_slice(&rule.base_offset.to_le_bytes());
            out.extend_from_slice(&rule.stride.to_le_bytes());
            out.extend_from_slice(&rule.cell_size.to_le_bytes());
            out.extend_from_slice(&rule.count.to_le_bytes());
            out.push(rule.argument_index);
            encode_string(out, &rule.argument_name);
            encode_string(out, &rule.segment_name);
        }

        out.extend_from_slice(&(self.lamport_accounts.len() as u32).to_le_bytes());
        for idx in &self.lamport_accounts {
            out.push(*idx);
        }
    }

    /// `SHA-256` commitment over just this instruction's contract (domain
    /// separated, so it never collides with a whole-manifest commitment).
    pub fn commitment(&self) -> [u8; 32] {
        let mut out = Vec::new();
        out.extend_from_slice(COMMIT_DOMAIN);
        out.extend_from_slice(b".instruction");
        self.encode_canonical(&mut out);
        sha256(&out)
    }
}

fn encode_string(out: &mut Vec<u8>, value: &str) {
    out.extend_from_slice(&(value.len() as u32).to_le_bytes());
    out.extend_from_slice(value.as_bytes());
}

fn encode_optional_u16(out: &mut Vec<u8>, value: Option<u16>) {
    match value {
        Some(value) => {
            out.push(1);
            out.extend_from_slice(&value.to_le_bytes());
        }
        None => out.push(0),
    }
}

impl MutationManifest {
    /// Canonical, deterministic byte encoding of the whole program's
    /// mutation contract: the domain tag, the instruction count, then each
    /// instruction's [`encode_canonical`](InstructionContract::encode_canonical)
    /// in manifest order.
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(COMMIT_DOMAIN);
        out.extend_from_slice(&(self.instructions.len() as u32).to_le_bytes());
        for ix in &self.instructions {
            ix.encode_canonical(&mut out);
        }
        out
    }

    /// `SHA-256` commitment over [`canonical_bytes`](Self::canonical_bytes).
    pub fn commitment(&self) -> [u8; 32] {
        sha256(&self.canonical_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{
        AccountRole, ArgContract, ArgEncodingContract, ParametricRangeContract, RangeContract,
    };

    fn sample() -> MutationManifest {
        MutationManifest {
            program_name: "p".to_string(),
            program_version: "1.0.0".to_string(),
            instructions: vec![InstructionContract {
                name: "pause".to_string(),
                tag: 1,
                strict_writes: true,
                mutation_complete: false,
                authorized: vec![
                    RangeContract {
                        account_index: 1,
                        offset: 114,
                        size: 1,
                    },
                    RangeContract {
                        account_index: 1,
                        offset: 115,
                        size: 8,
                    },
                ],
                parametric: vec![ParametricRangeContract {
                    account_index: 1,
                    base_offset: 115,
                    stride: 8,
                    cell_size: 8,
                    count: 1,
                    argument_index: 0,
                    argument_name: "slot".to_string(),
                    segment_name: "revision".to_string(),
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
                accounts: vec![
                    AccountRole {
                        name: "admin".to_string(),
                        writable: false,
                        signer: true,
                    },
                    AccountRole {
                        name: "config".to_string(),
                        writable: true,
                        signer: false,
                    },
                ],
            }],
        }
    }

    #[test]
    fn commitment_is_deterministic() {
        assert_eq!(sample().commitment(), sample().commitment());
    }

    #[test]
    fn changing_a_range_size_changes_the_commitment() {
        let base = sample().commitment();
        let mut m = sample();
        m.instructions[0].authorized[1].size = 9;
        assert_ne!(base, m.commitment(), "a range change must change the hash");
    }

    #[test]
    fn changing_a_range_offset_changes_the_commitment() {
        let base = sample().commitment();
        let mut m = sample();
        m.instructions[0].authorized[0].offset = 113;
        assert_ne!(base, m.commitment());
    }

    #[test]
    fn flipping_strict_writes_changes_the_commitment() {
        let base = sample().commitment();
        let mut m = sample();
        m.instructions[0].strict_writes = false;
        assert_ne!(base, m.commitment(), "a flag change must change the hash");
    }

    #[test]
    fn flipping_mutation_complete_changes_the_commitment() {
        let base = sample().commitment();
        let mut m = sample();
        m.instructions[0].mutation_complete = true;
        assert_ne!(base, m.commitment());
    }

    #[test]
    fn adding_a_lamport_permission_changes_the_commitment() {
        let base = sample().commitment();
        let mut m = sample();
        m.instructions[0].lamport_accounts.push(1);
        assert_ne!(base, m.commitment());
    }

    #[test]
    fn changing_any_parametric_rule_field_changes_the_commitment() {
        let base = sample().commitment();

        let mut changed = sample();
        changed.instructions[0].parametric[0].stride += 1;
        assert_ne!(base, changed.commitment());

        let mut changed = sample();
        changed.instructions[0].parametric[0].argument_name = "other".to_string();
        assert_ne!(base, changed.commitment());

        let mut changed = sample();
        changed.instructions[0].parametric[0].segment_name = "other".to_string();
        assert_ne!(base, changed.commitment());
    }

    #[test]
    fn changing_selector_wire_metadata_changes_the_commitment() {
        let base = sample().commitment();
        let mut changed = sample();
        changed.instructions[0].args[0].canonical_type = "u32".to_string();
        changed.instructions[0].args[0].size = 4;
        assert_ne!(base, changed.commitment());
    }

    #[test]
    fn reordering_ranges_changes_the_commitment() {
        // The commitment pins the manifest's DECLARED order; reordering the
        // same two ranges is a different published contract.
        let base = sample().commitment();
        let mut m = sample();
        m.instructions[0].authorized.swap(0, 1);
        assert_ne!(base, m.commitment());
    }

    #[test]
    fn cosmetic_changes_do_not_change_the_commitment() {
        // Contract-only: program name/version, account display names, and
        // account signer/writable flags are NOT part of the mutation
        // contract, so changing them leaves the commitment identical.
        let base = sample().commitment();
        let mut m = sample();
        m.program_name = "renamed-program".to_string();
        m.program_version = "9.9.9".to_string();
        m.instructions[0].accounts[1].name = "cfg".to_string();
        m.instructions[0].accounts[0].writable = true;
        assert_eq!(
            base,
            m.commitment(),
            "cosmetic metadata must not move the commitment"
        );
    }

    #[test]
    fn instruction_commitment_differs_from_manifest_commitment() {
        // Domain separation: a one-instruction manifest's commitment must
        // not equal that instruction's standalone commitment.
        let m = sample();
        assert_ne!(m.commitment(), m.instructions[0].commitment());
    }
}
