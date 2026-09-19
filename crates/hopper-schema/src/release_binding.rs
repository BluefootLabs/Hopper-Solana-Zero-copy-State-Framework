//! Versioned release-binary binding for Hopper program interfaces.
//!
//! [`interface_commitment`] hashes the executable declaration carried by a
//! [`ProgramManifest`](crate::ProgramManifest): program name/version, account
//! layouts, instruction wire data and account metas, events, policy contracts,
//! and the context fields represented by `ProgramManifest`. The encoding is semantic,
//! ordered, and unambiguously framed, so JSON whitespace, key order, and
//! accepted field aliases do not affect the result.
//!
//! The commitment deliberately excludes descriptive and measured metadata:
//! program descriptions, compute-unit estimates, layout-manager annotations,
//! compatibility plans, and tooling hints. These exclusions make this an
//! executable-interface commitment, not a byte-for-byte manifest-file hash.

use crate::{
    accounts::{AccountLifecycle, ContextDescriptor},
    ArgEncoding, EventDescriptor, FieldDescriptor, InstructionDescriptor, LayoutManifest,
    PolicyDescriptor, ProgramManifest,
};
use hopper_runtime::sha256::ConstSha256;

/// Magic bytes that identify a Hopper release-interface binding record.
pub const RELEASE_BINDING_MAGIC: [u8; 16] = *b"HOPPER_ABI_V1\0\0\0";

/// Binary format version encoded in every binding record.
pub const RELEASE_BINDING_FORMAT_VERSION: u16 = 1;

/// Hash algorithm identifier for the canonical SHA-256 byte stream.
pub const RELEASE_BINDING_HASH_SHA256: u8 = 1;

/// Fixed byte length of a version-1 release binding record.
pub const RELEASE_BINDING_RECORD_LEN: usize = 56;

/// Byte offset of the 32-byte interface commitment in a binding record.
pub const RELEASE_BINDING_COMMITMENT_OFFSET: usize = 24;

const COMMITMENT_DOMAIN: &[u8] = b"hopper:release-interface:v1";

/// Compute the canonical manifest-projected interface commitment.
///
/// This function is `const` so `hopper::program_manifest!`
/// can place the result directly in the compiled ELF without a build script.
pub const fn interface_commitment(manifest: &ProgramManifest) -> [u8; 32] {
    let mut state = ConstSha256::new().update(COMMITMENT_DOMAIN);
    state = field_str(state, b"program.name", manifest.name);
    state = field_str(state, b"program.version", manifest.version);

    state = field_u64(state, b"layouts.len", manifest.layouts.len() as u64);
    let mut i = 0;
    while i < manifest.layouts.len() {
        state = feed_layout(state, &manifest.layouts[i]);
        i += 1;
    }

    state = field_u64(
        state,
        b"instructions.len",
        manifest.instructions.len() as u64,
    );
    i = 0;
    while i < manifest.instructions.len() {
        let instruction = &manifest.instructions[i];
        state = feed_instruction(
            state,
            instruction,
            matching_context(instruction, manifest.contexts),
        );
        i += 1;
    }

    state = field_u64(state, b"events.len", manifest.events.len() as u64);
    i = 0;
    while i < manifest.events.len() {
        state = feed_event(state, &manifest.events[i]);
        i += 1;
    }

    state = field_u64(state, b"policies.len", manifest.policies.len() as u64);
    i = 0;
    while i < manifest.policies.len() {
        state = feed_policy(state, &manifest.policies[i]);
        i += 1;
    }

    state = field_u64(state, b"contexts.len", manifest.contexts.len() as u64);
    i = 0;
    while i < manifest.contexts.len() {
        state = feed_context(state, &manifest.contexts[i]);
        i += 1;
    }
    state.finalize()
}

/// Build the fixed version-1 ELF record for a program manifest.
pub const fn release_binding_record(
    manifest: &ProgramManifest,
) -> [u8; RELEASE_BINDING_RECORD_LEN] {
    release_binding_record_from_commitment(interface_commitment(manifest))
}

/// Build the fixed version-1 ELF record from a previously computed commitment.
///
/// Generated programs use this form so the compiler evaluates a large
/// manifest commitment only once.
pub const fn release_binding_record_from_commitment(
    commitment: [u8; 32],
) -> [u8; RELEASE_BINDING_RECORD_LEN] {
    let mut out = [0u8; RELEASE_BINDING_RECORD_LEN];
    let mut i = 0;
    while i < RELEASE_BINDING_MAGIC.len() {
        out[i] = RELEASE_BINDING_MAGIC[i];
        i += 1;
    }
    let version = RELEASE_BINDING_FORMAT_VERSION.to_le_bytes();
    out[16] = version[0];
    out[17] = version[1];
    out[18] = RELEASE_BINDING_HASH_SHA256;
    out[19] = 0;
    let record_len = (RELEASE_BINDING_RECORD_LEN as u32).to_le_bytes();
    out[20] = record_len[0];
    out[21] = record_len[1];
    out[22] = record_len[2];
    out[23] = record_len[3];
    i = 0;
    while i < commitment.len() {
        out[RELEASE_BINDING_COMMITMENT_OFFSET + i] = commitment[i];
        i += 1;
    }
    out
}

const fn feed_layout(mut state: ConstSha256, layout: &LayoutManifest) -> ConstSha256 {
    state = field_str(state, b"layout.name", layout.name);
    state = field_u8(state, b"layout.disc", layout.disc);
    state = field_u8(state, b"layout.version", layout.version);
    state = field_bytes(state, b"layout.layout_id", &layout.layout_id);
    state = field_u64(state, b"layout.total_size", layout.total_size as u64);
    // Preserve the established v1 stream byte-for-byte for fixed layouts.
    // Dynamic-tail metadata did not exist in the original v1 schema, so true
    // is encoded as an optional, typed extension. The 0xb2 bool chunk is
    // unambiguous before the following 0xb6 field-count chunk.
    if layout.has_dynamic_tail {
        state = field_bool(state, b"layout.has_dynamic_tail", true);
    }
    state = field_u64(state, b"layout.field_count", layout.field_count as u64);
    feed_fields(state, b"layout.fields.len", layout.fields)
}

const fn feed_fields(
    mut state: ConstSha256,
    count_label: &[u8],
    fields: &[FieldDescriptor],
) -> ConstSha256 {
    state = field_u64(state, count_label, fields.len() as u64);
    let mut i = 0;
    while i < fields.len() {
        let field = &fields[i];
        state = field_str(state, b"field.name", field.name);
        state = field_str(state, b"field.type", field.canonical_type);
        state = field_u16(state, b"field.size", field.size);
        state = field_u16(state, b"field.offset", field.offset);
        state = field_u8(state, b"field.intent", field.intent as u8);
        i += 1;
    }
    state
}

const fn feed_instruction(
    mut state: ConstSha256,
    instruction: &InstructionDescriptor,
    matching_context: Option<&ContextDescriptor>,
) -> ConstSha256 {
    state = field_str(state, b"instruction.name", instruction.name);
    state = field_u8(state, b"instruction.tag", instruction.tag);
    state = field_bytes(
        state,
        b"instruction.discriminator",
        instruction.discriminator,
    );

    state = field_u64(
        state,
        b"instruction.args.len",
        instruction.args.len() as u64,
    );
    let mut i = 0;
    while i < instruction.args.len() {
        let arg = &instruction.args[i];
        state = field_str(state, b"arg.name", arg.name);
        state = field_str(state, b"arg.type", arg.canonical_type);
        state = field_u16(state, b"arg.size", arg.size);
        match arg.encoding {
            ArgEncoding::Fixed => {
                state = field_u8(state, b"arg.encoding", 0);
            }
            ArgEncoding::BoundedVec {
                max_len,
                element_size,
            } => {
                state = field_u8(state, b"arg.encoding", 1);
                state = field_u16(state, b"arg.max_len", max_len);
                state = field_u16(state, b"arg.element_size", element_size);
            }
            ArgEncoding::BoundedString { max_len } => {
                state = field_u8(state, b"arg.encoding", 2);
                state = field_u16(state, b"arg.max_len", max_len);
            }
        }
        i += 1;
    }

    state = field_u64(
        state,
        b"instruction.accounts.len",
        instruction.accounts.len() as u64,
    );
    i = 0;
    while i < instruction.accounts.len() {
        let account = &instruction.accounts[i];
        state = field_str(state, b"instruction.account.name", account.name);
        state = field_bool(state, b"instruction.account.writable", account.writable);
        state = field_bool(state, b"instruction.account.signer", account.signer);
        state = field_str(state, b"instruction.account.layout_ref", account.layout_ref);
        let seeds_in_context = match matching_context {
            Some(context) if i < context.accounts.len() => {
                string_slices_equal(account.seeds, context.accounts[i].seeds)
            }
            _ => false,
        };
        state = field_bool(
            state,
            b"instruction.account.seeds_in_context",
            seeds_in_context,
        );
        if !seeds_in_context {
            state = feed_strings(state, b"instruction.account.seeds.len", account.seeds);
        }
        i += 1;
    }

    match instruction.remaining_accounts {
        Some(remaining) => {
            state = field_bool(state, b"instruction.remaining.present", true);
            state = field_u16(state, b"instruction.remaining.max", remaining.max);
        }
        None => {
            state = field_bool(state, b"instruction.remaining.present", false);
        }
    }
    state = feed_strings(
        state,
        b"instruction.capabilities.len",
        instruction.capabilities,
    );
    state = field_str(state, b"instruction.policy_pack", instruction.policy_pack);
    state = field_bool(
        state,
        b"instruction.receipt_expected",
        instruction.receipt_expected,
    );
    state = field_bool(
        state,
        b"instruction.strict_writes",
        instruction.strict_writes,
    );
    state = feed_write_ranges(
        state,
        b"instruction.write_ranges.len",
        instruction.write_ranges,
    );
    state = feed_parametric_ranges(
        state,
        b"instruction.parametric_write_ranges.len",
        instruction.parametric_write_ranges,
    );
    state = field_bool(
        state,
        b"instruction.mutation_complete",
        instruction.mutation_complete,
    );
    field_bytes(
        state,
        b"instruction.lamport_accounts",
        instruction.lamport_accounts,
    )
}

const fn matching_context<'a>(
    instruction: &InstructionDescriptor,
    contexts: &'a [ContextDescriptor],
) -> Option<&'a ContextDescriptor> {
    let mut only_match = usize::MAX;
    let mut match_count = 0usize;
    let mut i = 0;
    while i < contexts.len() {
        let context = &contexts[i];
        if context_matches_instruction(instruction, context) {
            if canonical_name_equal(instruction.name, context.name) {
                return Some(context);
            }
            only_match = i;
            match_count += 1;
        }
        i += 1;
    }
    if match_count == 1 {
        Some(&contexts[only_match])
    } else {
        None
    }
}

const fn context_matches_instruction(
    instruction: &InstructionDescriptor,
    context: &ContextDescriptor,
) -> bool {
    if instruction.accounts.len() != context.accounts.len() {
        return false;
    }
    let mut i = 0;
    while i < instruction.accounts.len() {
        let instruction_account = &instruction.accounts[i];
        let context_account = &context.accounts[i];
        if !string_equal(instruction_account.name, context_account.name)
            || instruction_account.writable != context_account.writable
            || instruction_account.signer != context_account.signer
            || (!instruction_account.layout_ref.is_empty()
                && !context_account.layout_ref.is_empty()
                && !string_equal(instruction_account.layout_ref, context_account.layout_ref))
        {
            return false;
        }
        i += 1;
    }
    true
}

const fn string_slices_equal(left: &[&str], right: &[&str]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut i = 0;
    while i < left.len() {
        if !string_equal(left[i], right[i]) {
            return false;
        }
        i += 1;
    }
    true
}

const fn string_equal(left: &str, right: &str) -> bool {
    let left = left.as_bytes();
    let right = right.as_bytes();
    if left.len() != right.len() {
        return false;
    }
    let mut i = 0;
    while i < left.len() {
        if left[i] != right[i] {
            return false;
        }
        i += 1;
    }
    true
}

const fn canonical_name_equal(left: &str, right: &str) -> bool {
    let left = left.as_bytes();
    let right = right.as_bytes();
    let mut left_index = 0;
    let mut right_index = 0;
    loop {
        while left_index < left.len() && !ascii_alphanumeric(left[left_index]) {
            left_index += 1;
        }
        while right_index < right.len() && !ascii_alphanumeric(right[right_index]) {
            right_index += 1;
        }
        if left_index == left.len() || right_index == right.len() {
            return left_index == left.len() && right_index == right.len();
        }
        if ascii_lower(left[left_index]) != ascii_lower(right[right_index]) {
            return false;
        }
        left_index += 1;
        right_index += 1;
    }
}

const fn ascii_alphanumeric(byte: u8) -> bool {
    (byte >= b'0' && byte <= b'9')
        || (byte >= b'A' && byte <= b'Z')
        || (byte >= b'a' && byte <= b'z')
}

const fn ascii_lower(byte: u8) -> u8 {
    if byte >= b'A' && byte <= b'Z' {
        byte + (b'a' - b'A')
    } else {
        byte
    }
}

const fn feed_event(mut state: ConstSha256, event: &EventDescriptor) -> ConstSha256 {
    state = field_str(state, b"event.name", event.name);
    state = field_u8(state, b"event.tag", event.tag);
    feed_fields(state, b"event.fields.len", event.fields)
}

const fn feed_policy(mut state: ConstSha256, policy: &PolicyDescriptor) -> ConstSha256 {
    state = field_str(state, b"policy.name", policy.name);
    state = feed_strings(state, b"policy.capabilities.len", policy.capabilities);
    state = feed_strings(state, b"policy.requirements.len", policy.requirements);
    state = feed_strings(state, b"policy.invariants.len", policy.invariants);
    field_str(state, b"policy.receipt_profile", policy.receipt_profile)
}

const fn feed_context(mut state: ConstSha256, context: &ContextDescriptor) -> ConstSha256 {
    state = field_str(state, b"context.name", context.name);
    state = field_u64(
        state,
        b"context.accounts.len",
        context.accounts.len() as u64,
    );
    let mut i = 0;
    while i < context.accounts.len() {
        let account = &context.accounts[i];
        state = field_str(state, b"context.account.name", account.name);
        state = field_str(state, b"context.account.kind", account.kind);
        state = field_bool(state, b"context.account.writable", account.writable);
        state = field_bool(state, b"context.account.signer", account.signer);
        state = field_str(state, b"context.account.layout_ref", account.layout_ref);
        state = field_str(state, b"context.account.policy_ref", account.policy_ref);
        state = feed_strings(state, b"context.account.seeds.len", account.seeds);
        state = field_bool(state, b"context.account.optional", account.optional);
        state = field_u8(
            state,
            b"context.account.lifecycle",
            lifecycle_tag(account.lifecycle),
        );
        state = field_str(state, b"context.account.payer", account.payer);
        state = field_u32(state, b"context.account.init_space", account.init_space);
        state = feed_strings(state, b"context.account.has_one.len", account.has_one);
        state = field_str(
            state,
            b"context.account.expected_address",
            account.expected_address,
        );
        state = field_str(
            state,
            b"context.account.expected_owner",
            account.expected_owner,
        );
        i += 1;
    }
    state = feed_strings(state, b"context.policies.len", context.policies);
    state = field_bool(
        state,
        b"context.receipts_expected",
        context.receipts_expected,
    );
    state = feed_strings(
        state,
        b"context.mutation_classes.len",
        context.mutation_classes,
    );
    state = field_bool(state, b"context.strict_writes", context.strict_writes);
    state = feed_write_ranges(state, b"context.write_ranges.len", context.write_ranges);
    state = feed_parametric_ranges(
        state,
        b"context.parametric_write_ranges.len",
        context.parametric_write_ranges,
    );
    state = field_bool(
        state,
        b"context.mutation_complete",
        context.mutation_complete,
    );
    field_bytes(state, b"context.lamport_accounts", context.lamport_accounts)
}

const fn feed_write_ranges(
    mut state: ConstSha256,
    count_label: &[u8],
    ranges: &[crate::WriteRange],
) -> ConstSha256 {
    state = field_u64(state, count_label, ranges.len() as u64);
    let mut i = 0;
    while i < ranges.len() {
        let range = &ranges[i];
        state = field_u8(state, b"write_range.account_index", range.account_index);
        state = field_u32(state, b"write_range.offset", range.offset);
        state = field_u32(state, b"write_range.size", range.size);
        i += 1;
    }
    state
}

const fn feed_parametric_ranges(
    mut state: ConstSha256,
    count_label: &[u8],
    ranges: &[crate::ParametricWriteRange],
) -> ConstSha256 {
    state = field_u64(state, count_label, ranges.len() as u64);
    let mut i = 0;
    while i < ranges.len() {
        let range = &ranges[i];
        state = field_u8(
            state,
            b"parametric_range.account_index",
            range.account_index,
        );
        state = field_u32(state, b"parametric_range.base_offset", range.base_offset);
        state = field_u32(state, b"parametric_range.stride", range.stride);
        state = field_u32(state, b"parametric_range.cell_size", range.cell_size);
        state = field_u32(state, b"parametric_range.count", range.count);
        state = field_u8(
            state,
            b"parametric_range.argument_index",
            range.argument_index,
        );
        state = field_str(
            state,
            b"parametric_range.argument_name",
            range.argument_name,
        );
        state = field_str(state, b"parametric_range.segment_name", range.segment_name);
        i += 1;
    }
    state
}

const fn feed_strings(mut state: ConstSha256, count_label: &[u8], values: &[&str]) -> ConstSha256 {
    state = field_u64(state, count_label, values.len() as u64);
    let mut i = 0;
    while i < values.len() {
        state = field_str(state, b"string.value", values[i]);
        i += 1;
    }
    state
}

const fn lifecycle_tag(value: AccountLifecycle) -> u8 {
    match value {
        AccountLifecycle::Existing => 0,
        AccountLifecycle::Init => 1,
        AccountLifecycle::InitIfNeeded => 2,
        AccountLifecycle::Realloc => 3,
        AccountLifecycle::Close => 4,
    }
}

// The v1 stream identifies fields by their fixed schema order and the typed
// chunk tag. Labels remain at call sites to make the encoder reviewable, but
// are not repeated in the hashed bytes; this keeps large program manifests
// below rustc's Solana-target const-evaluation limit without weakening the
// typed framing.
const fn field_str(state: ConstSha256, _label: &[u8], value: &str) -> ConstSha256 {
    chunk(state, 0xb1, value.as_bytes())
}

const fn field_bool(state: ConstSha256, _label: &[u8], value: bool) -> ConstSha256 {
    fixed_chunk(state, 0xb2, &[if value { 1 } else { 0 }])
}

const fn field_u8(state: ConstSha256, _label: &[u8], value: u8) -> ConstSha256 {
    fixed_chunk(state, 0xb3, &[value])
}

const fn field_u16(state: ConstSha256, _label: &[u8], value: u16) -> ConstSha256 {
    fixed_chunk(state, 0xb4, &value.to_le_bytes())
}

const fn field_u32(state: ConstSha256, _label: &[u8], value: u32) -> ConstSha256 {
    fixed_chunk(state, 0xb5, &value.to_le_bytes())
}

const fn field_u64(state: ConstSha256, _label: &[u8], value: u64) -> ConstSha256 {
    fixed_chunk(state, 0xb6, &value.to_le_bytes())
}

const fn field_bytes(state: ConstSha256, _label: &[u8], value: &[u8]) -> ConstSha256 {
    chunk(state, 0xb7, value)
}

const fn chunk(state: ConstSha256, kind: u8, value: &[u8]) -> ConstSha256 {
    state
        .update(&[kind])
        .update(&(value.len() as u64).to_le_bytes())
        .update(value)
}

const fn fixed_chunk(state: ConstSha256, kind: u8, value: &[u8]) -> ConstSha256 {
    state.update(&[kind]).update(value)
}

#[cfg(test)]
mod tests {
    extern crate alloc;

    use super::*;
    use crate::{AccountEntry, ArgDescriptor, FieldIntent, RemainingAccountsDescriptor};

    static FIELDS: &[FieldDescriptor] = &[FieldDescriptor {
        name: "balance",
        canonical_type: "WireU64",
        size: 8,
        offset: 16,
        intent: FieldIntent::Balance,
    }];
    static LAYOUTS: &[LayoutManifest] = &[LayoutManifest {
        name: "Vault",
        disc: 7,
        version: 1,
        layout_id: [9; 8],
        total_size: 24,
        has_dynamic_tail: false,
        field_count: 1,
        fields: FIELDS,
    }];
    static ARGS: &[ArgDescriptor] = &[ArgDescriptor {
        name: "amount",
        canonical_type: "u64",
        size: 8,
        encoding: ArgEncoding::Fixed,
    }];
    static ACCOUNTS: &[AccountEntry] = &[AccountEntry {
        name: "vault",
        writable: true,
        signer: false,
        layout_ref: "Vault",
        seeds: &[],
    }];
    static INSTRUCTIONS: &[InstructionDescriptor] = &[InstructionDescriptor {
        name: "deposit",
        tag: 3,
        discriminator: &[3, 4],
        args: ARGS,
        accounts: ACCOUNTS,
        remaining_accounts: Some(RemainingAccountsDescriptor { max: 2 }),
        capabilities: &[],
        policy_pack: "",
        receipt_expected: false,
        strict_writes: false,
        write_ranges: &[],
        parametric_write_ranges: &[],
        mutation_complete: false,
        lamport_accounts: &[],
        cu_estimate: 1_000,
    }];
    static POLICIES: &[PolicyDescriptor] = &[PolicyDescriptor {
        name: "vault-write",
        capabilities: &["MutatesState"],
        requirements: &["SignerAuthority"],
        invariants: &["conservation"],
        receipt_profile: "balance-change",
    }];
    static MANIFEST: ProgramManifest = ProgramManifest {
        name: "binding-test",
        version: "1.2.3",
        description: "not executable",
        layouts: LAYOUTS,
        layout_metadata: &[],
        instructions: INSTRUCTIONS,
        events: &[],
        policies: POLICIES,
        compatibility_pairs: &[],
        tooling_hints: &[],
        contexts: &[],
    };

    #[test]
    fn record_has_versioned_header_and_exact_commitment() {
        let record = release_binding_record(&MANIFEST);
        assert_eq!(&record[..16], &RELEASE_BINDING_MAGIC);
        assert_eq!(u16::from_le_bytes([record[16], record[17]]), 1);
        assert_eq!(record[18], RELEASE_BINDING_HASH_SHA256);
        assert_eq!(record[19], 0);
        assert_eq!(
            u32::from_le_bytes([record[20], record[21], record[22], record[23]]) as usize,
            RELEASE_BINDING_RECORD_LEN
        );
        assert_eq!(
            &record[RELEASE_BINDING_COMMITMENT_OFFSET..],
            &interface_commitment(&MANIFEST)
        );
    }

    #[test]
    fn version_one_commitment_encoding_is_stable() {
        assert_eq!(
            interface_commitment(&MANIFEST),
            [
                9, 132, 224, 136, 218, 106, 105, 75, 236, 254, 133, 124, 98, 206, 158, 236, 25,
                148, 80, 147, 171, 36, 224, 41, 239, 68, 82, 67, 21, 3, 184, 229,
            ]
        );
    }

    #[test]
    fn executable_interface_changes_change_the_commitment() {
        let original = interface_commitment(&MANIFEST);

        let mut renamed = MANIFEST;
        renamed.name = "other-program";
        assert_ne!(original, interface_commitment(&renamed));

        static OTHER_INSTRUCTIONS: &[InstructionDescriptor] = &[InstructionDescriptor {
            discriminator: &[3, 5],
            ..INSTRUCTIONS[0]
        }];
        let mut changed_instruction = MANIFEST;
        changed_instruction.instructions = OTHER_INSTRUCTIONS;
        assert_ne!(original, interface_commitment(&changed_instruction));

        static OTHER_ACCOUNTS: &[AccountEntry] = &[AccountEntry {
            writable: false,
            ..ACCOUNTS[0]
        }];
        static OTHER_META_INSTRUCTIONS: &[InstructionDescriptor] = &[InstructionDescriptor {
            accounts: OTHER_ACCOUNTS,
            ..INSTRUCTIONS[0]
        }];
        let mut changed_meta = MANIFEST;
        changed_meta.instructions = OTHER_META_INSTRUCTIONS;
        assert_ne!(original, interface_commitment(&changed_meta));

        static SEEDED_ACCOUNTS: &[AccountEntry] = &[AccountEntry {
            seeds: &["b\"vault\"", "authority"],
            ..ACCOUNTS[0]
        }];
        static SEEDED_INSTRUCTIONS: &[InstructionDescriptor] = &[InstructionDescriptor {
            accounts: SEEDED_ACCOUNTS,
            ..INSTRUCTIONS[0]
        }];
        let mut changed_seed_contract = MANIFEST;
        changed_seed_contract.instructions = SEEDED_INSTRUCTIONS;
        assert_ne!(original, interface_commitment(&changed_seed_contract));

        static OTHER_POLICIES: &[PolicyDescriptor] = &[PolicyDescriptor {
            requirements: &["PdaAuthority"],
            ..POLICIES[0]
        }];
        let mut changed_policy = MANIFEST;
        changed_policy.policies = OTHER_POLICIES;
        assert_ne!(original, interface_commitment(&changed_policy));

        let dynamic_layout = LayoutManifest {
            has_dynamic_tail: true,
            ..LAYOUTS[0]
        };
        let mut changed_size_policy = MANIFEST;
        changed_size_policy.layouts =
            alloc::boxed::Box::leak(alloc::boxed::Box::new([dynamic_layout]));
        assert_ne!(original, interface_commitment(&changed_size_policy));
    }

    #[test]
    fn non_executable_release_metadata_is_not_in_the_commitment() {
        let original = interface_commitment(&MANIFEST);
        let mut changed = MANIFEST;
        changed.description = "edited release copy";
        static OTHER_INSTRUCTIONS: &[InstructionDescriptor] = &[InstructionDescriptor {
            cu_estimate: 99_999,
            ..INSTRUCTIONS[0]
        }];
        changed.instructions = OTHER_INSTRUCTIONS;
        changed.tooling_hints = &["manager-only"];
        assert_eq!(original, interface_commitment(&changed));
    }
}
