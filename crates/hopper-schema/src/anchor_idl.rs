//! Current Solana/Anchor IDL projection.
//!
//! Anchor 0.30 replaced the legacy IDL shape with specification version
//! `0.1.0`. The current shape requires a program address, stores program
//! identity under `metadata`, uses `writable` and `signer` account flags,
//! and keeps account/event discriminators separate from entries in `types`.
//!
//! Hopper publishes its actual wire discriminators. It does not synthesize
//! Anchor's default `sha256("global:<name>")[..8]` instruction discriminator
//! or `sha256("account:<name>")[..8]` account discriminator. Current Anchor
//! supports custom, variable-length discriminators, so a Hopper one-byte or
//! multi-byte prefix can be represented without changing the program ABI.
//! The full-manifest projection preserves a one-to-eight-byte instruction
//! prefix. The smaller `ProgramIdl` model carries only its legacy one-byte
//! tag and publishes that byte without padding. Headered accounts publish the
//! two-byte `[discriminator, version]` prefix that Hopper actually validates;
//! compact accounts publish their one-byte discriminator.
//!
//! Hopper account bodies use an offset-aware zero-copy layout rather than
//! Anchor's default Borsh codec. Account type definitions therefore declare
//! a custom `hopper-zero-copy-v1` serialization. Consumers can build the
//! instructions present in this projection, but must use a Hopper-aware
//! account decoder. Instructions containing Hopper's `u16`-length bounded
//! vector or string encoding are deliberately omitted: Anchor IDL v0.1.0 has
//! no way to describe that wire codec, and advertising it as a Borsh `vec` or
//! `string` would generate invalid transaction data.
//!
//! Formatting emits a marked partial projection when declarations cannot be
//! represented: ambiguous discriminator prefixes are omitted, as is PDA
//! metadata whose seed source cannot be resolved without inventing bytes or a
//! reference. Such output carries machine-readable top-level `docs` markers.
//! The corresponding `validate` methods, and CLI publication/export paths that
//! call them, reject partial projections.
//!
//! The formatter is allocation-free and remains compatible with `no_std`.

use core::fmt;

use crate::{
    classify_seed, AccountEntry, ArgDescriptor, ConstantDescriptor, EventDescriptor,
    FieldDescriptor, IdlAccountEntry, IdlInstructionDescriptor, InstructionDescriptor,
    LayoutManifest, ProgramIdl, ProgramManifest, SeedPart,
};

const IDL_SPEC_VERSION: &str = "0.1.0";
const ACCOUNT_SERIALIZATION: &str = "hopper-zero-copy-v1";
const EVENT_SERIALIZATION: &str = "hopper-event-v1";
const HOPPER_HEADER_SIZE: usize = 16;

// ---------------------------------------------------------------------------
// JSON and identifier helpers
// ---------------------------------------------------------------------------

fn write_json_char(f: &mut fmt::Formatter<'_>, c: char) -> fmt::Result {
    match c {
        '"' => write!(f, "\\\""),
        '\\' => write!(f, "\\\\"),
        '\n' => write!(f, "\\n"),
        '\r' => write!(f, "\\r"),
        '\t' => write!(f, "\\t"),
        '\u{08}' => write!(f, "\\b"),
        '\u{0c}' => write!(f, "\\f"),
        c if c <= '\u{1f}' => write!(f, "\\u{:04x}", c as u32),
        _ => write!(f, "{}", c),
    }
}

fn write_json_str(f: &mut fmt::Formatter<'_>, value: &str) -> fmt::Result {
    write!(f, "\"")?;
    for c in value.chars() {
        write_json_char(f, c)?;
    }
    write!(f, "\"")
}

/// Anchor 0.30 and newer serialize instruction, account-meta, argument, and
/// field identifiers as camelCase. Program names remain unchanged.
fn write_camel_json_str(f: &mut fmt::Formatter<'_>, value: &str) -> fmt::Result {
    write!(f, "\"")?;
    let mut uppercase_next = false;
    for c in value.chars() {
        if c == '_' {
            uppercase_next = true;
            continue;
        }
        let c = if uppercase_next {
            uppercase_next = false;
            c.to_ascii_uppercase()
        } else {
            c
        };
        write_json_char(f, c)?;
    }
    write!(f, "\"")
}

fn write_indent(f: &mut fmt::Formatter<'_>, level: usize) -> fmt::Result {
    for _ in 0..level {
        write!(f, "  ")?;
    }
    Ok(())
}

fn identifier_eq(left: &str, right: &str) -> bool {
    let mut left = left.bytes().filter(|b| *b != b'_');
    let mut right = right.bytes().filter(|b| *b != b'_');
    loop {
        match (left.next(), right.next()) {
            (Some(a), Some(b)) if a.eq_ignore_ascii_case(&b) => {}
            (None, None) => return true,
            _ => return false,
        }
    }
}

fn generic_inner<'a>(value: &'a str, name: &str) -> Option<&'a str> {
    let rest = value.strip_prefix(name)?.trim_start();
    let inner = rest.strip_prefix('<')?.strip_suffix('>')?;
    Some(inner.trim())
}

fn parse_array_type(value: &str) -> Option<(&str, &str)> {
    let inner = value.trim().strip_prefix('[')?.strip_suffix(']')?;
    let (element, length) = inner.rsplit_once(';')?;
    Some((element.trim(), length.trim()))
}

fn parse_decimal(value: &str) -> Option<usize> {
    let mut parsed = 0usize;
    let mut saw_digit = false;
    for byte in value.bytes() {
        if byte == b'_' {
            continue;
        }
        if !byte.is_ascii_digit() {
            return None;
        }
        saw_digit = true;
        parsed = parsed
            .checked_mul(10)?
            .checked_add((byte - b'0') as usize)?;
    }
    saw_digit.then_some(parsed)
}

fn primitive_width(canonical: &str) -> Option<usize> {
    match canonical.trim() {
        "u8" | "i8" | "bool" | "WireU8" | "WireI8" | "WireBool" => Some(1),
        "u16" | "i16" | "WireU16" | "WireI16" => Some(2),
        "u32" | "i32" | "f32" | "WireU32" | "WireI32" => Some(4),
        "u64" | "i64" | "f64" | "WireU64" | "WireI64" => Some(8),
        "u128" | "i128" | "WireU128" | "WireI128" => Some(16),
        "u256" | "i256" | "WireU256" | "WireI256" | "Pubkey" | "Address" | "UntypedAddress"
        | "pubkey" => Some(32),
        value if generic_inner(value, "TypedAddress").is_some() => Some(32),
        _ => None,
    }
}

fn fixed_type_width(canonical: &str) -> Option<usize> {
    let canonical = canonical.trim();
    if let Some(width) = primitive_width(canonical) {
        return Some(width);
    }
    let (element, length) = parse_array_type(canonical)?;
    fixed_type_width(element)?.checked_mul(parse_decimal(length)?)
}

fn resolved_array_length(canonical: &str, exact_size: Option<usize>) -> Option<(&str, usize)> {
    let (element, declared_length) = parse_array_type(canonical)?;
    if let Some(length) = parse_decimal(declared_length) {
        return Some((element, length));
    }

    let element_width = fixed_type_width(element)?;
    let exact_size = exact_size?;
    if element_width == 0 || exact_size % element_width != 0 {
        return None;
    }
    Some((element, exact_size / element_width))
}

fn fixed_arg_is_anchor_encodable(arg: &ArgDescriptor) -> bool {
    if arg.encoding != crate::ArgEncoding::Fixed {
        return false;
    }

    let canonical = arg.canonical_type.trim();
    if canonical.is_empty()
        || canonical == "bytes"
        || canonical == "string"
        || generic_inner(canonical, "Option").is_some()
        || generic_inner(canonical, "Vec").is_some()
        || generic_inner(canonical, "BoundedVec").is_some()
        || generic_inner(canonical, "HopperVec").is_some()
        || generic_inner(canonical, "BoundedString").is_some()
        || generic_inner(canonical, "HopperString").is_some()
    {
        return false;
    }

    if let Some(width) = fixed_type_width(canonical) {
        return width == usize::from(arg.size);
    }
    if let Some((element, length)) = resolved_array_length(canonical, Some(arg.size.into())) {
        return fixed_type_width(element)
            .and_then(|width| width.checked_mul(length))
            .is_some_and(|width| width == usize::from(arg.size));
    }

    // A hand-authored fixed descriptor can still be projected safely as raw
    // bytes. Unlike an unresolved `defined` reference, this makes callers
    // supply exactly the declared wire width.
    arg.size != 0
}

fn instruction_is_anchor_encodable(args: &[ArgDescriptor]) -> bool {
    args.iter().all(fixed_arg_is_anchor_encodable)
}

fn first_unencodable_arg(args: &[ArgDescriptor]) -> Option<&ArgDescriptor> {
    args.iter().find(|arg| !fixed_arg_is_anchor_encodable(arg))
}

/// Project a canonical Hopper type into the current IDL type grammar.
///
/// `exact_size` resolves const-named fixed arrays. Unknown fixed-width values
/// become raw byte arrays instead of dangling `defined` references.
fn write_anchor_type(
    f: &mut fmt::Formatter<'_>,
    canonical: &str,
    exact_size: Option<usize>,
) -> fmt::Result {
    let canonical = canonical.trim();
    if canonical.is_empty() {
        return write!(f, "{{ \"array\": [\"u8\", {}] }}", exact_size.unwrap_or(0));
    }

    match canonical {
        "u8" | "u16" | "u32" | "u64" | "u128" | "u256" | "i8" | "i16" | "i32" | "i64" | "i128"
        | "i256" | "f32" | "f64" | "bool" | "bytes" | "string" => {
            return write_json_str(f, canonical);
        }
        "Pubkey" | "Address" | "pubkey" => return write_json_str(f, "pubkey"),
        _ => {}
    }

    if generic_inner(canonical, "TypedAddress").is_some() {
        return write_json_str(f, "pubkey");
    }

    if let Some(wire) = canonical.strip_prefix("Wire") {
        let primitive = match wire {
            "U8" => Some("u8"),
            "U16" => Some("u16"),
            "U32" => Some("u32"),
            "U64" => Some("u64"),
            "U128" => Some("u128"),
            "U256" => Some("u256"),
            "I8" => Some("i8"),
            "I16" => Some("i16"),
            "I32" => Some("i32"),
            "I64" => Some("i64"),
            "I128" => Some("i128"),
            "I256" => Some("i256"),
            "Bool" => Some("bool"),
            _ => None,
        };
        if let Some(primitive) = primitive {
            return write_json_str(f, primitive);
        }
    }

    if let Some((element, length)) = resolved_array_length(canonical, exact_size) {
        write!(f, "{{ \"array\": [")?;
        write_anchor_type(f, element, fixed_type_width(element))?;
        return write!(f, ", {}] }}", length);
    }

    if let Some(inner) = generic_inner(canonical, "Option") {
        write!(f, "{{ \"option\": ")?;
        write_anchor_type(f, inner, None)?;
        return write!(f, " }}");
    }

    if let Some(inner) = generic_inner(canonical, "Vec") {
        write!(f, "{{ \"vec\": ")?;
        write_anchor_type(f, inner, None)?;
        return write!(f, " }}");
    }

    write!(f, "{{ \"array\": [\"u8\", {}] }}", exact_size.unwrap_or(0))
}

fn write_discriminator(f: &mut fmt::Formatter<'_>, discriminator: &[u8]) -> fmt::Result {
    write!(f, "[")?;
    for (index, byte) in discriminator.iter().enumerate() {
        if index != 0 {
            write!(f, ", ")?;
        }
        write!(f, "{}", byte)?;
    }
    write!(f, "]")
}

fn write_tag_discriminator(f: &mut fmt::Formatter<'_>, tag: u8) -> fmt::Result {
    write_discriminator(f, core::slice::from_ref(&tag))
}

fn discriminator_prefixes_collide(left: &[u8], right: &[u8]) -> bool {
    left.starts_with(right) || right.starts_with(left)
}

fn idl_instruction_discriminator(instruction: &IdlInstructionDescriptor) -> &[u8] {
    core::slice::from_ref(&instruction.tag)
}

fn manifest_instruction_discriminator(instruction: &InstructionDescriptor) -> &[u8] {
    if instruction.discriminator.is_empty() {
        core::slice::from_ref(&instruction.tag)
    } else {
        instruction.discriminator
    }
}

fn idl_instruction_discriminators_collide(
    left: &IdlInstructionDescriptor,
    right: &IdlInstructionDescriptor,
) -> bool {
    discriminator_prefixes_collide(
        idl_instruction_discriminator(left),
        idl_instruction_discriminator(right),
    )
}

fn manifest_instruction_discriminators_collide(
    left: &InstructionDescriptor,
    right: &InstructionDescriptor,
) -> bool {
    discriminator_prefixes_collide(
        manifest_instruction_discriminator(left),
        manifest_instruction_discriminator(right),
    )
}

fn first_idl_instruction_discriminator_collision(
    instructions: &[IdlInstructionDescriptor],
) -> Option<(&IdlInstructionDescriptor, &IdlInstructionDescriptor)> {
    for (index, left) in instructions.iter().enumerate() {
        if let Some(right) = instructions[index + 1..]
            .iter()
            .find(|right| idl_instruction_discriminators_collide(left, right))
        {
            return Some((left, right));
        }
    }
    None
}

fn first_manifest_instruction_discriminator_collision(
    instructions: &[InstructionDescriptor],
) -> Option<(&InstructionDescriptor, &InstructionDescriptor)> {
    for (index, left) in instructions.iter().enumerate() {
        if let Some(right) = instructions[index + 1..]
            .iter()
            .find(|right| manifest_instruction_discriminators_collide(left, right))
        {
            return Some((left, right));
        }
    }
    None
}

fn idl_instruction_discriminator_is_ambiguous(
    instructions: &[IdlInstructionDescriptor],
    index: usize,
) -> bool {
    instructions.iter().enumerate().any(|(other_index, other)| {
        other_index != index && idl_instruction_discriminators_collide(&instructions[index], other)
    })
}

fn manifest_instruction_discriminator_is_ambiguous(
    instructions: &[InstructionDescriptor],
    index: usize,
) -> bool {
    instructions.iter().enumerate().any(|(other_index, other)| {
        other_index != index
            && manifest_instruction_discriminators_collide(&instructions[index], other)
    })
}

// ---------------------------------------------------------------------------
// PDA projection
// ---------------------------------------------------------------------------

fn write_literal_bytes(f: &mut fmt::Formatter<'_>, value: &str) -> fmt::Result {
    write!(f, "[")?;
    for (index, byte) in value.bytes().enumerate() {
        if index != 0 {
            write!(f, ", ")?;
        }
        write!(f, "{}", byte)?;
    }
    write!(f, "]")
}

fn reference_root(value: &str) -> &str {
    value.split_once('.').map_or(value, |(root, _)| root).trim()
}

fn base58_digit(byte: u8) -> Option<u8> {
    match byte {
        b'1'..=b'9' => Some(byte - b'1'),
        b'A'..=b'H' => Some(byte - b'A' + 9),
        b'J'..=b'N' => Some(byte - b'J' + 17),
        b'P'..=b'Z' => Some(byte - b'P' + 22),
        b'a'..=b'k' => Some(byte - b'a' + 33),
        b'm'..=b'z' => Some(byte - b'm' + 44),
        _ => None,
    }
}

/// Validate a canonical Solana address without adding an allocator or a
/// base58 dependency to this `no_std` schema crate.
fn is_base58_pubkey(value: &str) -> bool {
    if value.is_empty() {
        return false;
    }

    let mut decoded = [0u8; 32];
    for byte in value.bytes() {
        let Some(digit) = base58_digit(byte) else {
            return false;
        };
        let mut carry = u32::from(digit);
        for slot in decoded.iter_mut().rev() {
            carry += u32::from(*slot) * 58;
            *slot = (carry & 0xff) as u8;
            carry >>= 8;
        }
        if carry != 0 {
            return false;
        }
    }

    let leading_zeroes = value.bytes().take_while(|byte| *byte == b'1').count();
    let significant_bytes = decoded
        .iter()
        .position(|byte| *byte != 0)
        .map_or(0, |first| decoded.len() - first);
    leading_zeroes + significant_bytes == decoded.len()
}

fn idl_seed_supported(
    seed: &crate::PdaSeedHint,
    accounts: &[IdlAccountEntry],
    args: &[ArgDescriptor],
) -> bool {
    match seed.kind {
        "literal" | "const" => true,
        "account" => accounts
            .iter()
            .any(|account| identifier_eq(account.name, reference_root(seed.value))),
        "arg" => args
            .iter()
            .any(|arg| identifier_eq(arg.name, reference_root(seed.value))),
        _ => false,
    }
}

fn write_idl_seed(f: &mut fmt::Formatter<'_>, seed: &crate::PdaSeedHint) -> fmt::Result {
    match seed.kind {
        "literal" | "const" => {
            write!(f, "{{ \"kind\": \"const\", \"value\": ")?;
            write_literal_bytes(f, seed.value)?;
            write!(f, " }}")
        }
        "account" => {
            write!(f, "{{ \"kind\": \"account\", \"path\": ")?;
            write_camel_json_str(f, seed.value)?;
            write!(f, " }}")
        }
        "arg" => {
            write!(f, "{{ \"kind\": \"arg\", \"path\": ")?;
            write_camel_json_str(f, seed.value)?;
            write!(f, " }}")
        }
        _ => Err(fmt::Error),
    }
}

fn write_idl_pda(f: &mut fmt::Formatter<'_>, seeds: &[crate::PdaSeedHint]) -> fmt::Result {
    write!(f, "{{ \"seeds\": [")?;
    for (index, seed) in seeds.iter().enumerate() {
        if index != 0 {
            write!(f, ", ")?;
        }
        write_idl_seed(f, seed)?;
    }
    write!(f, "] }}")
}

fn manifest_seeds_supported(
    seeds: &[&str],
    accounts: &[AccountEntry],
    args: &[ArgDescriptor],
) -> bool {
    seeds
        .iter()
        .all(|seed| manifest_seed_supported(seed, accounts, args))
}

fn manifest_seed_supported(seed: &str, accounts: &[AccountEntry], args: &[ArgDescriptor]) -> bool {
    match classify_seed(seed) {
        // `classify_seed` deliberately preserves literal source text for
        // general client generators. This projection may only copy literals
        // whose source spelling is already their exact UTF-8 byte sequence;
        // otherwise an escape such as `\\x00` would derive a different PDA.
        SeedPart::Literal(value) => !value.contains('\\'),
        SeedPart::Account(path) => accounts
            .iter()
            .any(|account| account.name == reference_root(path)),
        // Solana IDL argument seeds use the argument's normal Borsh encoding.
        // Hopper's integer encoding is little endian, so a big-endian source
        // expression cannot be projected as the same `kind: "arg"` seed.
        SeedPart::Arg(path) => {
            !seed.contains(".to_be_bytes()")
                && seed.contains(".to_le_bytes()")
                && args.iter().any(|arg| arg.name == reference_root(path))
        }
        SeedPart::Unknown(_) => false,
    }
}

fn first_unsupported_idl_pda_seed(
    instructions: &[IdlInstructionDescriptor],
) -> Option<(
    &IdlInstructionDescriptor,
    &IdlAccountEntry,
    &crate::PdaSeedHint,
)> {
    for instruction in instructions {
        for account in instruction.accounts {
            if let Some(seed) = account
                .pda_seeds
                .iter()
                .find(|seed| !idl_seed_supported(seed, instruction.accounts, instruction.args))
            {
                return Some((instruction, account, seed));
            }
        }
    }
    None
}

fn first_unsupported_manifest_pda_seed(
    instructions: &[InstructionDescriptor],
) -> Option<(&InstructionDescriptor, &AccountEntry, &'static str)> {
    for instruction in instructions {
        for account in instruction.accounts {
            if let Some(seed) =
                account.seeds.iter().copied().find(|seed| {
                    !manifest_seed_supported(seed, instruction.accounts, instruction.args)
                })
            {
                return Some((instruction, account, seed));
            }
        }
    }
    None
}

fn first_unsupported_manifest_expected_address(
    manifest: &ProgramManifest,
) -> Option<(&InstructionDescriptor, &AccountEntry, &'static str)> {
    for instruction in manifest.instructions {
        for (index, account) in instruction.accounts.iter().enumerate() {
            let address = find_context_account(manifest, instruction.name, index, account.name)
                .map(|context| context.expected_address)
                .unwrap_or("");
            if !address.is_empty() && !is_base58_pubkey(address) {
                return Some((instruction, account, address));
            }
        }
    }
    None
}

fn write_manifest_seed(f: &mut fmt::Formatter<'_>, seed: &str) -> fmt::Result {
    match classify_seed(seed) {
        SeedPart::Literal(value) => {
            write!(f, "{{ \"kind\": \"const\", \"value\": ")?;
            write_literal_bytes(f, value)?;
            write!(f, " }}")
        }
        SeedPart::Account(path) => {
            write!(f, "{{ \"kind\": \"account\", \"path\": ")?;
            write_camel_json_str(f, path)?;
            write!(f, " }}")
        }
        SeedPart::Arg(path) => {
            write!(f, "{{ \"kind\": \"arg\", \"path\": ")?;
            write_camel_json_str(f, path)?;
            write!(f, " }}")
        }
        SeedPart::Unknown(_) => Err(fmt::Error),
    }
}

fn write_manifest_pda(f: &mut fmt::Formatter<'_>, seeds: &[&str]) -> fmt::Result {
    write!(f, "{{ \"seeds\": [")?;
    for (index, seed) in seeds.iter().enumerate() {
        if index != 0 {
            write!(f, ", ")?;
        }
        write_manifest_seed(f, seed)?;
    }
    write!(f, "] }}")
}

// ---------------------------------------------------------------------------
// Instruction projection
// ---------------------------------------------------------------------------

fn write_instruction_args(
    f: &mut fmt::Formatter<'_>,
    args: &[ArgDescriptor],
    indent: usize,
) -> fmt::Result {
    if args.is_empty() {
        return write!(f, "[]");
    }

    writeln!(f, "[")?;
    for (index, arg) in args.iter().enumerate() {
        write_indent(f, indent + 1)?;
        write!(f, "{{ \"name\": ")?;
        write_camel_json_str(f, arg.name)?;
        write!(f, ", \"type\": ")?;
        write_anchor_type(f, arg.canonical_type, Some(arg.size.into()))?;
        write!(f, " }}")?;
        if index + 1 == args.len() {
            writeln!(f)?;
        } else {
            writeln!(f, ",")?;
        }
    }
    write_indent(f, indent)?;
    write!(f, "]")
}

fn write_idl_instruction_accounts(
    f: &mut fmt::Formatter<'_>,
    accounts: &[IdlAccountEntry],
    args: &[ArgDescriptor],
    indent: usize,
) -> fmt::Result {
    if accounts.is_empty() {
        return write!(f, "[]");
    }

    writeln!(f, "[")?;
    for (index, account) in accounts.iter().enumerate() {
        write_indent(f, indent + 1)?;
        write!(f, "{{ \"name\": ")?;
        write_camel_json_str(f, account.name)?;
        if account.writable {
            write!(f, ", \"writable\": true")?;
        }
        if account.signer {
            write!(f, ", \"signer\": true")?;
        }
        if !account.pda_seeds.is_empty()
            && account
                .pda_seeds
                .iter()
                .all(|seed| idl_seed_supported(seed, accounts, args))
        {
            write!(f, ", \"pda\": ")?;
            write_idl_pda(f, account.pda_seeds)?;
        }
        write!(f, " }}")?;
        if index + 1 == accounts.len() {
            writeln!(f)?;
        } else {
            writeln!(f, ",")?;
        }
    }
    write_indent(f, indent)?;
    write!(f, "]")
}

fn find_context_account<'a>(
    manifest: &'a ProgramManifest,
    instruction_name: &str,
    account_index: usize,
    account_name: &str,
) -> Option<&'a crate::accounts::ContextAccountDescriptor> {
    let context = manifest
        .contexts
        .iter()
        .find(|context| identifier_eq(context.name, instruction_name))?;

    match context.accounts.get(account_index) {
        Some(account) if identifier_eq(account.name, account_name) => Some(account),
        _ => context
            .accounts
            .iter()
            .find(|account| identifier_eq(account.name, account_name)),
    }
}

fn write_manifest_instruction_accounts(
    f: &mut fmt::Formatter<'_>,
    manifest: &ProgramManifest,
    instruction_name: &str,
    accounts: &[AccountEntry],
    args: &[ArgDescriptor],
    indent: usize,
) -> fmt::Result {
    if accounts.is_empty() {
        return write!(f, "[]");
    }

    writeln!(f, "[")?;
    for (index, account) in accounts.iter().enumerate() {
        let context = find_context_account(manifest, instruction_name, index, account.name);
        write_indent(f, indent + 1)?;
        write!(f, "{{ \"name\": ")?;
        write_camel_json_str(f, account.name)?;
        if account.writable {
            write!(f, ", \"writable\": true")?;
        }
        if account.signer {
            write!(f, ", \"signer\": true")?;
        }
        if context.is_some_and(|context| context.optional) {
            write!(f, ", \"optional\": true")?;
        }
        if let Some(address) = context
            .map(|context| context.expected_address)
            .filter(|address| is_base58_pubkey(address))
        {
            write!(f, ", \"address\": ")?;
            write_json_str(f, address)?;
        }
        if !account.seeds.is_empty() && manifest_seeds_supported(account.seeds, accounts, args) {
            write!(f, ", \"pda\": ")?;
            write_manifest_pda(f, account.seeds)?;
        }
        if let Some(relations) = context
            .map(|context| context.has_one)
            .filter(|relations| !relations.is_empty())
        {
            write!(f, ", \"relations\": [")?;
            for (relation_index, relation) in relations.iter().enumerate() {
                if relation_index != 0 {
                    write!(f, ", ")?;
                }
                write_camel_json_str(f, relation)?;
            }
            write!(f, "]")?;
        }
        write!(f, " }}")?;
        if index + 1 == accounts.len() {
            writeln!(f)?;
        } else {
            writeln!(f, ",")?;
        }
    }
    write_indent(f, indent)?;
    write!(f, "]")
}

fn write_idl_instructions(
    f: &mut fmt::Formatter<'_>,
    instructions: &[IdlInstructionDescriptor],
) -> fmt::Result {
    let safe_count = instructions
        .iter()
        .enumerate()
        .filter(|(index, instruction)| {
            instruction_is_anchor_encodable(instruction.args)
                && !idl_instruction_discriminator_is_ambiguous(instructions, *index)
        })
        .count();
    if safe_count == 0 {
        return writeln!(f, "  \"instructions\": [],");
    }

    writeln!(f, "  \"instructions\": [")?;
    for (output_index, (_, instruction)) in instructions
        .iter()
        .enumerate()
        .filter(|(index, instruction)| {
            instruction_is_anchor_encodable(instruction.args)
                && !idl_instruction_discriminator_is_ambiguous(instructions, *index)
        })
        .enumerate()
    {
        write_indent(f, 2)?;
        writeln!(f, "{{")?;
        write_indent(f, 3)?;
        write!(f, "\"name\": ")?;
        write_camel_json_str(f, instruction.name)?;
        writeln!(f, ",")?;
        write_indent(f, 3)?;
        write!(f, "\"discriminator\": ")?;
        write_tag_discriminator(f, instruction.tag)?;
        writeln!(f, ",")?;
        write_indent(f, 3)?;
        write!(f, "\"accounts\": ")?;
        write_idl_instruction_accounts(f, instruction.accounts, instruction.args, 3)?;
        writeln!(f, ",")?;
        write_indent(f, 3)?;
        write!(f, "\"args\": ")?;
        write_instruction_args(f, instruction.args, 3)?;
        writeln!(f)?;
        write_indent(f, 2)?;
        write!(f, "}}")?;
        if output_index + 1 == safe_count {
            writeln!(f)?;
        } else {
            writeln!(f, ",")?;
        }
    }
    writeln!(f, "  ],")
}

fn write_manifest_instructions(
    f: &mut fmt::Formatter<'_>,
    manifest: &ProgramManifest,
) -> fmt::Result {
    let safe_count = manifest
        .instructions
        .iter()
        .enumerate()
        .filter(|(index, instruction)| {
            instruction_is_anchor_encodable(instruction.args)
                && instruction.remaining_accounts.is_none()
                && !manifest_instruction_discriminator_is_ambiguous(manifest.instructions, *index)
        })
        .count();
    if safe_count == 0 {
        return writeln!(f, "  \"instructions\": [],");
    }

    writeln!(f, "  \"instructions\": [")?;
    for (output_index, (_, instruction)) in manifest
        .instructions
        .iter()
        .enumerate()
        .filter(|(index, instruction)| {
            instruction_is_anchor_encodable(instruction.args)
                && instruction.remaining_accounts.is_none()
                && !manifest_instruction_discriminator_is_ambiguous(manifest.instructions, *index)
        })
        .enumerate()
    {
        write_indent(f, 2)?;
        writeln!(f, "{{")?;
        write_indent(f, 3)?;
        write!(f, "\"name\": ")?;
        write_camel_json_str(f, instruction.name)?;
        writeln!(f, ",")?;
        write_indent(f, 3)?;
        write!(f, "\"discriminator\": ")?;
        if instruction.discriminator.is_empty() {
            write_tag_discriminator(f, instruction.tag)?;
        } else {
            write_discriminator(f, instruction.discriminator)?;
        }
        writeln!(f, ",")?;
        write_indent(f, 3)?;
        write!(f, "\"accounts\": ")?;
        write_manifest_instruction_accounts(
            f,
            manifest,
            instruction.name,
            instruction.accounts,
            instruction.args,
            3,
        )?;
        writeln!(f, ",")?;
        write_indent(f, 3)?;
        write!(f, "\"args\": ")?;
        write_instruction_args(f, instruction.args, 3)?;
        writeln!(f)?;
        write_indent(f, 2)?;
        write!(f, "}}")?;
        if output_index + 1 == safe_count {
            writeln!(f)?;
        } else {
            writeln!(f, ",")?;
        }
    }
    writeln!(f, "  ],")
}

// ---------------------------------------------------------------------------
// Account, event, and type projection
// ---------------------------------------------------------------------------

fn layout_is_compact(layout: &LayoutManifest) -> bool {
    // Keep this in lockstep with clientgen.rs. Macro-generated compact layouts
    // start their first field at account offset 1; an empty compact layout is
    // shorter than the universal 16-byte header. Headered layouts start their
    // fields at offset 16 or later.
    layout.fields.iter().map(|field| field.offset).min() == Some(1)
        || layout.total_size < HOPPER_HEADER_SIZE
}

fn account_discriminators_collide(left: &LayoutManifest, right: &LayoutManifest) -> bool {
    if left.disc != right.disc {
        return false;
    }

    // A compact one-byte prefix collides with every account beginning with the
    // same byte. Two headered layouts remain distinguishable by version.
    layout_is_compact(left) || layout_is_compact(right) || left.version == right.version
}

fn first_account_discriminator_collision(
    layouts: &[LayoutManifest],
) -> Option<(&LayoutManifest, &LayoutManifest)> {
    for (index, left) in layouts.iter().enumerate() {
        if let Some(right) = layouts[index + 1..]
            .iter()
            .find(|right| account_discriminators_collide(left, right))
        {
            return Some((left, right));
        }
    }
    None
}

fn event_discriminators_collide(left: &EventDescriptor, right: &EventDescriptor) -> bool {
    discriminator_prefixes_collide(
        core::slice::from_ref(&left.tag),
        core::slice::from_ref(&right.tag),
    )
}

fn first_event_discriminator_collision(
    events: &[EventDescriptor],
) -> Option<(&EventDescriptor, &EventDescriptor)> {
    for (index, left) in events.iter().enumerate() {
        if let Some(right) = events[index + 1..]
            .iter()
            .find(|right| event_discriminators_collide(left, right))
        {
            return Some((left, right));
        }
    }
    None
}

fn event_discriminator_is_ambiguous(events: &[EventDescriptor], index: usize) -> bool {
    events.iter().enumerate().any(|(other_index, other)| {
        other_index != index && event_discriminators_collide(&events[index], other)
    })
}

fn write_account_discriminator(f: &mut fmt::Formatter<'_>, layout: &LayoutManifest) -> fmt::Result {
    if layout_is_compact(layout) {
        write_tag_discriminator(f, layout.disc)
    } else {
        write_discriminator(f, &[layout.disc, layout.version])
    }
}

fn write_account_declarations(
    f: &mut fmt::Formatter<'_>,
    layouts: &[LayoutManifest],
) -> fmt::Result {
    if layouts.is_empty() {
        return writeln!(f, "  \"accounts\": [],");
    }
    if first_account_discriminator_collision(layouts).is_some() {
        // Callers that require a complete projection should call `validate`
        // before formatting. Keeping the account namespace empty here is the
        // safe fallback for legacy Display callers: no account decoder can be
        // selected using an ambiguous prefix.
        return writeln!(f, "  \"accounts\": [],");
    }

    writeln!(f, "  \"accounts\": [")?;
    for (index, layout) in layouts.iter().enumerate() {
        write!(f, "    {{ \"name\": ")?;
        write_json_str(f, layout.name)?;
        write!(f, ", \"discriminator\": ")?;
        write_account_discriminator(f, layout)?;
        write!(f, " }}")?;
        if index + 1 == layouts.len() {
            writeln!(f)?;
        } else {
            writeln!(f, ",")?;
        }
    }
    writeln!(f, "  ],")
}

fn write_event_declarations(f: &mut fmt::Formatter<'_>, events: &[EventDescriptor]) -> fmt::Result {
    let safe_count = events
        .iter()
        .enumerate()
        .filter(|(index, _)| !event_discriminator_is_ambiguous(events, *index))
        .count();
    if safe_count == 0 {
        return writeln!(f, "  \"events\": [],");
    }

    writeln!(f, "  \"events\": [")?;
    for (output_index, (_, event)) in events
        .iter()
        .enumerate()
        .filter(|(index, _)| !event_discriminator_is_ambiguous(events, *index))
        .enumerate()
    {
        write!(f, "    {{ \"name\": ")?;
        write_json_str(f, event.name)?;
        write!(f, ", \"discriminator\": ")?;
        write_tag_discriminator(f, event.tag)?;
        write!(f, " }}")?;
        if output_index + 1 == safe_count {
            writeln!(f)?;
        } else {
            writeln!(f, ",")?;
        }
    }
    writeln!(f, "  ],")
}

fn write_fields(
    f: &mut fmt::Formatter<'_>,
    fields: &[FieldDescriptor],
    indent: usize,
) -> fmt::Result {
    if fields.is_empty() {
        return write!(f, "[]");
    }

    writeln!(f, "[")?;
    for (index, field) in fields.iter().enumerate() {
        write_indent(f, indent + 1)?;
        write!(f, "{{ \"name\": ")?;
        write_camel_json_str(f, field.name)?;
        write!(f, ", \"type\": ")?;
        write_anchor_type(f, field.canonical_type, Some(field.size.into()))?;
        write!(f, " }}")?;
        if index + 1 == fields.len() {
            writeln!(f)?;
        } else {
            writeln!(f, ",")?;
        }
    }
    write_indent(f, indent)?;
    write!(f, "]")
}

fn write_type_definition(
    f: &mut fmt::Formatter<'_>,
    name: &str,
    serialization: &str,
    fields: &[FieldDescriptor],
    indent: usize,
) -> fmt::Result {
    write_indent(f, indent)?;
    writeln!(f, "{{")?;
    write_indent(f, indent + 1)?;
    write!(f, "\"name\": ")?;
    write_json_str(f, name)?;
    writeln!(f, ",")?;
    write_indent(f, indent + 1)?;
    write!(f, "\"serialization\": {{ \"custom\": ")?;
    write_json_str(f, serialization)?;
    writeln!(f, " }},")?;
    write_indent(f, indent + 1)?;
    writeln!(f, "\"type\": {{")?;
    write_indent(f, indent + 2)?;
    writeln!(f, "\"kind\": \"struct\",")?;
    write_indent(f, indent + 2)?;
    write!(f, "\"fields\": ")?;
    write_fields(f, fields, indent + 2)?;
    writeln!(f)?;
    write_indent(f, indent + 1)?;
    writeln!(f, "}}")?;
    write_indent(f, indent)?;
    write!(f, "}}")
}

fn write_types(
    f: &mut fmt::Formatter<'_>,
    layouts: &[LayoutManifest],
    events: &[EventDescriptor],
) -> fmt::Result {
    let safe_event_count = events
        .iter()
        .enumerate()
        .filter(|(index, _)| !event_discriminator_is_ambiguous(events, *index))
        .count();
    if layouts.is_empty() && safe_event_count == 0 {
        return writeln!(f, "  \"types\": [],");
    }

    writeln!(f, "  \"types\": [")?;
    for (index, layout) in layouts.iter().enumerate() {
        write_type_definition(f, layout.name, ACCOUNT_SERIALIZATION, layout.fields, 2)?;
        if index + 1 == layouts.len() && safe_event_count == 0 {
            writeln!(f)?;
        } else {
            writeln!(f, ",")?;
        }
    }
    for (output_index, (_, event)) in events
        .iter()
        .enumerate()
        .filter(|(index, _)| !event_discriminator_is_ambiguous(events, *index))
        .enumerate()
    {
        write_type_definition(f, event.name, EVENT_SERIALIZATION, event.fields, 2)?;
        if output_index + 1 == safe_event_count {
            writeln!(f)?;
        } else {
            writeln!(f, ",")?;
        }
    }
    writeln!(f, "  ],")
}

fn write_constants(f: &mut fmt::Formatter<'_>, constants: &[ConstantDescriptor]) -> fmt::Result {
    if constants.is_empty() {
        return writeln!(f, "  \"constants\": []");
    }

    writeln!(f, "  \"constants\": [")?;
    for (index, constant) in constants.iter().enumerate() {
        write!(f, "    {{ \"name\": ")?;
        write_json_str(f, constant.name)?;
        write!(f, ", \"type\": ")?;
        write_anchor_type(f, constant.ty, None)?;
        write!(f, ", \"value\": ")?;
        write_json_str(f, constant.value)?;
        write!(f, " }}")?;
        if index + 1 == constants.len() {
            writeln!(f)?;
        } else {
            writeln!(f, ",")?;
        }
    }
    writeln!(f, "  ]")
}

fn write_preamble(
    f: &mut fmt::Formatter<'_>,
    address: &str,
    name: &str,
    version: &str,
    description: &str,
) -> fmt::Result {
    writeln!(f, "{{")?;
    write!(f, "  \"address\": ")?;
    write_json_str(f, address)?;
    writeln!(f, ",")?;
    writeln!(f, "  \"metadata\": {{")?;
    write!(f, "    \"name\": ")?;
    write_json_str(f, name)?;
    writeln!(f, ",")?;
    write!(f, "    \"version\": ")?;
    write_json_str(f, version)?;
    writeln!(f, ",")?;
    write!(f, "    \"spec\": ")?;
    write_json_str(f, IDL_SPEC_VERSION)?;
    if description.is_empty() {
        writeln!(f)?;
    } else {
        writeln!(f, ",")?;
        write!(f, "    \"description\": ")?;
        write_json_str(f, description)?;
        writeln!(f)?;
    }
    writeln!(f, "  }},")
}

fn finish_idl(f: &mut fmt::Formatter<'_>) -> fmt::Result {
    write!(f, "}}")
}

#[derive(Clone, Copy, Default)]
struct ProjectionIssues {
    has_unencodable_instruction: bool,
    has_unrepresentable_remaining_accounts: bool,
    has_unrepresentable_dynamic_tail: bool,
    has_ambiguous_instruction_discriminator: bool,
    has_ambiguous_account_discriminator: bool,
    has_ambiguous_event_discriminator: bool,
    has_unresolved_pda_seed: bool,
    has_unresolved_expected_address: bool,
}

fn write_projection_status(f: &mut fmt::Formatter<'_>, issues: ProjectionIssues) -> fmt::Result {
    let ProjectionIssues {
        has_unencodable_instruction,
        has_unrepresentable_remaining_accounts,
        has_unrepresentable_dynamic_tail,
        has_ambiguous_instruction_discriminator,
        has_ambiguous_account_discriminator,
        has_ambiguous_event_discriminator,
        has_unresolved_pda_seed,
        has_unresolved_expected_address,
    } = issues;
    if !has_unencodable_instruction
        && !has_unrepresentable_remaining_accounts
        && !has_unrepresentable_dynamic_tail
        && !has_ambiguous_instruction_discriminator
        && !has_ambiguous_account_discriminator
        && !has_ambiguous_event_discriminator
        && !has_unresolved_pda_seed
        && !has_unresolved_expected_address
    {
        return Ok(());
    }

    writeln!(f, "  \"docs\": [")?;
    write!(f, "    \"hopper:anchor-idl-projection:partial\"")?;
    if has_unencodable_instruction {
        writeln!(f, ",")?;
        write!(
            f,
            "    \"hopper:anchor-idl-projection:omits-unsupported-instruction-encodings\""
        )?;
    }
    if has_unrepresentable_remaining_accounts {
        writeln!(f, ",")?;
        write!(
            f,
            "    \"hopper:anchor-idl-projection:omits-remaining-account-contracts\""
        )?;
    }
    if has_unrepresentable_dynamic_tail {
        writeln!(f, ",")?;
        write!(
            f,
            "    \"hopper:anchor-idl-projection:omits-dynamic-tail-contracts\""
        )?;
    }
    if has_ambiguous_instruction_discriminator {
        writeln!(f, ",")?;
        write!(
            f,
            "    \"hopper:anchor-idl-projection:omits-ambiguous-instruction-discriminators\""
        )?;
    }
    if has_ambiguous_account_discriminator {
        writeln!(f, ",")?;
        write!(
            f,
            "    \"hopper:anchor-idl-projection:omits-ambiguous-account-discriminators\""
        )?;
    }
    if has_ambiguous_event_discriminator {
        writeln!(f, ",")?;
        write!(
            f,
            "    \"hopper:anchor-idl-projection:omits-ambiguous-event-discriminators\""
        )?;
    }
    if has_unresolved_pda_seed {
        writeln!(f, ",")?;
        write!(
            f,
            "    \"hopper:anchor-idl-projection:omits-unresolved-pda-seeds\""
        )?;
    }
    if has_unresolved_expected_address {
        writeln!(f, ",")?;
        write!(
            f,
            "    \"hopper:anchor-idl-projection:omits-unresolved-account-addresses\""
        )?;
    }
    writeln!(f)?;
    writeln!(f, "  ],")
}

/// A reason a complete Hopper manifest cannot be represented by Anchor IDL
/// v0.1.0 without changing its wire meaning.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnchorIdlProjectionError {
    /// A dynamic account tail has no field/type descriptor in the canonical
    /// Solana IDL v0.1.0 projection, so emitting only its fixed prefix would be
    /// lossy.
    UnsupportedDynamicTail {
        /// Layout carrying the unrepresentable tail.
        layout: &'static str,
    },
    /// The instruction contains an argument whose Hopper encoding has no
    /// byte-equivalent Anchor IDL type.
    UnsupportedArgumentEncoding {
        /// Instruction containing the unsupported argument.
        instruction: &'static str,
        /// First unsupported argument.
        argument: &'static str,
    },
    /// The instruction accepts a bounded variadic account suffix. Solana IDL
    /// v0.1.0 cannot encode Hopper's remaining-account ceiling or grammar.
    UnsupportedRemainingAccounts {
        /// Instruction containing the variadic suffix contract.
        instruction: &'static str,
    },
    /// Two instruction declarations would use equal or prefix-overlapping
    /// effective discriminators, making dispatch ambiguous to an IDL client.
    AmbiguousInstructionDiscriminator {
        /// First colliding instruction.
        first: &'static str,
        /// Second colliding instruction.
        second: &'static str,
    },
    /// Two account declarations would use equal or prefix-overlapping
    /// discriminators. Anchor rejects these because decoding would be
    /// ambiguous.
    AmbiguousAccountDiscriminator {
        /// First colliding layout.
        first: &'static str,
        /// Second colliding layout.
        second: &'static str,
    },
    /// Two event declarations would use equal or prefix-overlapping
    /// discriminators, making event decoding ambiguous.
    AmbiguousEventDiscriminator {
        /// First colliding event.
        first: &'static str,
        /// Second colliding event.
        second: &'static str,
    },
    /// A non-empty PDA seed list cannot be projected without inventing a
    /// constant value or a reference to an instruction account or argument.
    UnsupportedPdaSeed {
        /// Instruction containing the PDA account.
        instruction: &'static str,
        /// Account whose PDA metadata cannot be projected.
        account: &'static str,
        /// First unsupported or unresolved source expression.
        seed: &'static str,
    },
    /// A pinned account address is source syntax rather than a resolved
    /// 32-byte base58 public key.
    UnsupportedAccountAddress {
        /// Instruction containing the pinned account.
        instruction: &'static str,
        /// Account whose address could not be resolved.
        account: &'static str,
        /// Unresolved source spelling.
        address: &'static str,
    },
}

impl fmt::Display for AnchorIdlProjectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedDynamicTail { layout } => write!(
                f,
                "account layout `{layout}` has a dynamic tail that Solana IDL v0.1.0 cannot represent losslessly"
            ),
            Self::UnsupportedArgumentEncoding {
                instruction,
                argument,
            } => write!(
                f,
                "instruction `{instruction}` argument `{argument}` uses a Hopper wire encoding that Anchor IDL v0.1.0 cannot represent"
            ),
            Self::UnsupportedRemainingAccounts { instruction } => write!(
                f,
                "instruction `{instruction}` has a Hopper remaining-account contract that Solana IDL v0.1.0 cannot represent"
            ),
            Self::AmbiguousInstructionDiscriminator { first, second } => write!(
                f,
                "instructions `{first}` and `{second}` have ambiguous Anchor discriminator prefixes"
            ),
            Self::AmbiguousAccountDiscriminator { first, second } => write!(
                f,
                "account layouts `{first}` and `{second}` have ambiguous Anchor discriminator prefixes"
            ),
            Self::AmbiguousEventDiscriminator { first, second } => write!(
                f,
                "events `{first}` and `{second}` have ambiguous Anchor discriminator prefixes"
            ),
            Self::UnsupportedPdaSeed {
                instruction,
                account,
                seed,
            } => write!(
                f,
                "instruction `{instruction}` account `{account}` PDA seed `{seed}` cannot be resolved as an Anchor constant, account, or argument seed"
            ),
            Self::UnsupportedAccountAddress {
                instruction,
                account,
                address,
            } => write!(
                f,
                "instruction `{instruction}` account `{account}` address `{address}` is not a resolved 32-byte base58 Solana address"
            ),
        }
    }
}

fn validate_idl_instruction_discriminators(
    instructions: &[IdlInstructionDescriptor],
) -> Result<(), AnchorIdlProjectionError> {
    if let Some((first, second)) = first_idl_instruction_discriminator_collision(instructions) {
        return Err(
            AnchorIdlProjectionError::AmbiguousInstructionDiscriminator {
                first: first.name,
                second: second.name,
            },
        );
    }
    Ok(())
}

fn validate_manifest_instruction_discriminators(
    instructions: &[InstructionDescriptor],
) -> Result<(), AnchorIdlProjectionError> {
    if let Some((first, second)) = first_manifest_instruction_discriminator_collision(instructions)
    {
        return Err(
            AnchorIdlProjectionError::AmbiguousInstructionDiscriminator {
                first: first.name,
                second: second.name,
            },
        );
    }
    Ok(())
}

fn validate_account_discriminators(
    layouts: &[LayoutManifest],
) -> Result<(), AnchorIdlProjectionError> {
    if let Some((first, second)) = first_account_discriminator_collision(layouts) {
        return Err(AnchorIdlProjectionError::AmbiguousAccountDiscriminator {
            first: first.name,
            second: second.name,
        });
    }
    Ok(())
}

fn validate_event_discriminators(
    events: &[EventDescriptor],
) -> Result<(), AnchorIdlProjectionError> {
    if let Some((first, second)) = first_event_discriminator_collision(events) {
        return Err(AnchorIdlProjectionError::AmbiguousEventDiscriminator {
            first: first.name,
            second: second.name,
        });
    }
    Ok(())
}

/// Validate that a [`ProgramIdl`] has a complete, unambiguous Anchor IDL
/// v0.1.0 projection.
///
/// Formatting remains fail-closed for legacy callers: unsupported
/// instructions, declarations with ambiguous discriminator prefixes, and
/// unresolved PDA metadata are omitted. The top-level `docs` array is marked
/// `hopper:anchor-idl-projection:partial` and includes a reason marker.
/// Deployment and publication tools should call this function first and
/// reject an error instead of publishing a partial projection.
pub fn validate_program_idl_projection(idl: &ProgramIdl) -> Result<(), AnchorIdlProjectionError> {
    for instruction in idl.instructions {
        if let Some(argument) = first_unencodable_arg(instruction.args) {
            return Err(AnchorIdlProjectionError::UnsupportedArgumentEncoding {
                instruction: instruction.name,
                argument: argument.name,
            });
        }
    }
    validate_idl_instruction_discriminators(idl.instructions)?;
    validate_account_discriminators(idl.accounts)?;
    validate_event_discriminators(idl.events)?;
    if let Some((instruction, account, seed)) = first_unsupported_idl_pda_seed(idl.instructions) {
        return Err(AnchorIdlProjectionError::UnsupportedPdaSeed {
            instruction: instruction.name,
            account: account.name,
            seed: seed.value,
        });
    }
    Ok(())
}

/// Validate that a [`ProgramManifest`] has a complete, unambiguous Anchor IDL
/// v0.1.0 projection.
///
/// See [`validate_program_idl_projection`] for the fail-closed formatting
/// behavior and the publication requirement.
pub fn validate_program_manifest_projection(
    manifest: &ProgramManifest,
) -> Result<(), AnchorIdlProjectionError> {
    if let Some(layout) = manifest
        .layouts
        .iter()
        .find(|layout| layout.has_dynamic_tail)
    {
        return Err(AnchorIdlProjectionError::UnsupportedDynamicTail {
            layout: layout.name,
        });
    }
    for instruction in manifest.instructions {
        if let Some(argument) = first_unencodable_arg(instruction.args) {
            return Err(AnchorIdlProjectionError::UnsupportedArgumentEncoding {
                instruction: instruction.name,
                argument: argument.name,
            });
        }
        if instruction.remaining_accounts.is_some() {
            return Err(AnchorIdlProjectionError::UnsupportedRemainingAccounts {
                instruction: instruction.name,
            });
        }
    }
    validate_manifest_instruction_discriminators(manifest.instructions)?;
    validate_account_discriminators(manifest.layouts)?;
    validate_event_discriminators(manifest.events)?;
    if let Some((instruction, account, seed)) =
        first_unsupported_manifest_pda_seed(manifest.instructions)
    {
        return Err(AnchorIdlProjectionError::UnsupportedPdaSeed {
            instruction: instruction.name,
            account: account.name,
            seed,
        });
    }
    if let Some((instruction, account, address)) =
        first_unsupported_manifest_expected_address(manifest)
    {
        return Err(AnchorIdlProjectionError::UnsupportedAccountAddress {
            instruction: instruction.name,
            account: account.name,
            address,
        });
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Public formatter wrappers
// ---------------------------------------------------------------------------

/// Emit a Solana IDL v0.1.0 / Anchor-compatible projection from a [`ProgramIdl`].
///
/// `address` is the expected base58 program address supplied by the caller.
/// Formatting does not query RPC or prove deployment; Hopper manifests
/// deliberately do not bind a build artifact to one deployment address.
pub struct AnchorIdlJson<'a> {
    /// Public Hopper IDL projection.
    pub idl: &'a ProgramIdl,
    /// Expected base58 program address supplied by the caller.
    pub address: &'a str,
}

impl AnchorIdlJson<'_> {
    /// Reject a partial or ambiguous Anchor projection before publication.
    pub fn validate(&self) -> Result<(), AnchorIdlProjectionError> {
        validate_program_idl_projection(self.idl)
    }
}

impl fmt::Display for AnchorIdlJson<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_preamble(
            f,
            self.address,
            self.idl.name,
            self.idl.version,
            self.idl.description,
        )?;
        write_projection_status(
            f,
            ProjectionIssues {
                has_unencodable_instruction: self
                    .idl
                    .instructions
                    .iter()
                    .any(|instruction| !instruction_is_anchor_encodable(instruction.args)),
                has_ambiguous_instruction_discriminator:
                    first_idl_instruction_discriminator_collision(self.idl.instructions).is_some(),
                has_ambiguous_account_discriminator: first_account_discriminator_collision(
                    self.idl.accounts,
                )
                .is_some(),
                has_ambiguous_event_discriminator: first_event_discriminator_collision(
                    self.idl.events,
                )
                .is_some(),
                has_unresolved_pda_seed: first_unsupported_idl_pda_seed(self.idl.instructions)
                    .is_some(),
                ..ProjectionIssues::default()
            },
        )?;
        write_idl_instructions(f, self.idl.instructions)?;
        write_account_declarations(f, self.idl.accounts)?;
        write_event_declarations(f, self.idl.events)?;
        writeln!(f, "  \"errors\": [],")?;
        write_types(f, self.idl.accounts, self.idl.events)?;
        write_constants(f, &[])?;
        finish_idl(f)
    }
}

/// Emit the current IDL shape from a [`ProgramIdl`] plus public constants.
pub struct AnchorIdlWithConstants<'a> {
    /// Public Hopper IDL projection.
    pub idl: &'a ProgramIdl,
    /// Base58 program address for this deployment.
    pub address: &'a str,
    /// Constants to include in the IDL.
    pub constants: &'a [ConstantDescriptor],
}

impl AnchorIdlWithConstants<'_> {
    /// Reject a partial or ambiguous Anchor projection before publication.
    pub fn validate(&self) -> Result<(), AnchorIdlProjectionError> {
        validate_program_idl_projection(self.idl)
    }
}

impl fmt::Display for AnchorIdlWithConstants<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_preamble(
            f,
            self.address,
            self.idl.name,
            self.idl.version,
            self.idl.description,
        )?;
        write_projection_status(
            f,
            ProjectionIssues {
                has_unencodable_instruction: self
                    .idl
                    .instructions
                    .iter()
                    .any(|instruction| !instruction_is_anchor_encodable(instruction.args)),
                has_ambiguous_instruction_discriminator:
                    first_idl_instruction_discriminator_collision(self.idl.instructions).is_some(),
                has_ambiguous_account_discriminator: first_account_discriminator_collision(
                    self.idl.accounts,
                )
                .is_some(),
                has_ambiguous_event_discriminator: first_event_discriminator_collision(
                    self.idl.events,
                )
                .is_some(),
                has_unresolved_pda_seed: first_unsupported_idl_pda_seed(self.idl.instructions)
                    .is_some(),
                ..ProjectionIssues::default()
            },
        )?;
        write_idl_instructions(f, self.idl.instructions)?;
        write_account_declarations(f, self.idl.accounts)?;
        write_event_declarations(f, self.idl.events)?;
        writeln!(f, "  \"errors\": [],")?;
        write_types(f, self.idl.accounts, self.idl.events)?;
        write_constants(f, self.constants)?;
        finish_idl(f)
    }
}

/// Emit a Solana IDL v0.1.0 / Anchor-compatible projection from a full
/// [`ProgramManifest`].
pub struct AnchorIdlFromManifest<'a> {
    /// Source manifest.
    pub manifest: &'a ProgramManifest,
    /// Expected base58 program address supplied by the caller.
    pub address: &'a str,
}

impl AnchorIdlFromManifest<'_> {
    /// Reject a partial or ambiguous Anchor projection before publication.
    pub fn validate(&self) -> Result<(), AnchorIdlProjectionError> {
        validate_program_manifest_projection(self.manifest)
    }
}

impl fmt::Display for AnchorIdlFromManifest<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_preamble(
            f,
            self.address,
            self.manifest.name,
            self.manifest.version,
            self.manifest.description,
        )?;
        write_projection_status(
            f,
            ProjectionIssues {
                has_unencodable_instruction: self
                    .manifest
                    .instructions
                    .iter()
                    .any(|instruction| !instruction_is_anchor_encodable(instruction.args)),
                has_unrepresentable_remaining_accounts: self
                    .manifest
                    .instructions
                    .iter()
                    .any(|instruction| instruction.remaining_accounts.is_some()),
                has_unrepresentable_dynamic_tail: self
                    .manifest
                    .layouts
                    .iter()
                    .any(|layout| layout.has_dynamic_tail),
                has_ambiguous_instruction_discriminator:
                    first_manifest_instruction_discriminator_collision(self.manifest.instructions)
                        .is_some(),
                has_ambiguous_account_discriminator: first_account_discriminator_collision(
                    self.manifest.layouts,
                )
                .is_some(),
                has_ambiguous_event_discriminator: first_event_discriminator_collision(
                    self.manifest.events,
                )
                .is_some(),
                has_unresolved_pda_seed: first_unsupported_manifest_pda_seed(
                    self.manifest.instructions,
                )
                .is_some(),
                has_unresolved_expected_address: first_unsupported_manifest_expected_address(
                    self.manifest,
                )
                .is_some(),
            },
        )?;
        write_manifest_instructions(f, self.manifest)?;
        write_account_declarations(f, self.manifest.layouts)?;
        write_event_declarations(f, self.manifest.events)?;
        writeln!(f, "  \"errors\": [],")?;
        write_types(f, self.manifest.layouts, self.manifest.events)?;
        write_constants(f, &[])?;
        finish_idl(f)
    }
}

/// Emit the current IDL shape from a manifest plus public constants.
pub struct AnchorIdlFromManifestWithConstants<'a> {
    /// Source manifest.
    pub manifest: &'a ProgramManifest,
    /// Base58 program address for this deployment.
    pub address: &'a str,
    /// Constants to include in the IDL.
    pub constants: &'a [ConstantDescriptor],
}

impl AnchorIdlFromManifestWithConstants<'_> {
    /// Reject a partial or ambiguous Anchor projection before publication.
    pub fn validate(&self) -> Result<(), AnchorIdlProjectionError> {
        validate_program_manifest_projection(self.manifest)
    }
}

impl fmt::Display for AnchorIdlFromManifestWithConstants<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_preamble(
            f,
            self.address,
            self.manifest.name,
            self.manifest.version,
            self.manifest.description,
        )?;
        write_projection_status(
            f,
            ProjectionIssues {
                has_unencodable_instruction: self
                    .manifest
                    .instructions
                    .iter()
                    .any(|instruction| !instruction_is_anchor_encodable(instruction.args)),
                has_unrepresentable_remaining_accounts: self
                    .manifest
                    .instructions
                    .iter()
                    .any(|instruction| instruction.remaining_accounts.is_some()),
                has_unrepresentable_dynamic_tail: self
                    .manifest
                    .layouts
                    .iter()
                    .any(|layout| layout.has_dynamic_tail),
                has_ambiguous_instruction_discriminator:
                    first_manifest_instruction_discriminator_collision(self.manifest.instructions)
                        .is_some(),
                has_ambiguous_account_discriminator: first_account_discriminator_collision(
                    self.manifest.layouts,
                )
                .is_some(),
                has_ambiguous_event_discriminator: first_event_discriminator_collision(
                    self.manifest.events,
                )
                .is_some(),
                has_unresolved_pda_seed: first_unsupported_manifest_pda_seed(
                    self.manifest.instructions,
                )
                .is_some(),
                has_unresolved_expected_address: first_unsupported_manifest_expected_address(
                    self.manifest,
                )
                .is_some(),
            },
        )?;
        write_manifest_instructions(f, self.manifest)?;
        write_account_declarations(f, self.manifest.layouts)?;
        write_event_declarations(f, self.manifest.events)?;
        writeln!(f, "  \"errors\": [],")?;
        write_types(f, self.manifest.layouts, self.manifest.events)?;
        write_constants(f, self.constants)?;
        finish_idl(f)
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use crate::{
        AccountEntry, ArgDescriptor, ArgEncoding, FieldIntent, IdlAccountEntry,
        IdlInstructionDescriptor, InstructionDescriptor, LayoutManifest, ProgramManifest,
    };
    use std::format;

    const ADDRESS: &str = "11111111111111111111111111111111";

    static EMPTY_IDL: ProgramIdl = ProgramIdl {
        name: "empty_program",
        version: "0.3.0",
        description: "",
        instructions: &[],
        accounts: &[],
        events: &[],
        fingerprints: &[],
    };

    static FIELDS: &[FieldDescriptor] = &[
        FieldDescriptor {
            name: "authority_key",
            canonical_type: "TypedAddress < Authority >",
            size: 32,
            offset: 16,
            intent: FieldIntent::Custom,
        },
        FieldDescriptor {
            name: "amount",
            canonical_type: "WireU64",
            size: 8,
            offset: 48,
            intent: FieldIntent::Custom,
        },
    ];

    static LAYOUTS: &[LayoutManifest] = &[LayoutManifest {
        name: "Vault",
        disc: 4,
        version: 1,
        layout_id: [90; 8],
        total_size: 56,
        has_dynamic_tail: false,
        field_count: 2,
        fields: FIELDS,
    }];

    static ARGS: &[ArgDescriptor] = &[ArgDescriptor {
        name: "deposit_amount",
        canonical_type: "u64",
        size: 8,
        encoding: ArgEncoding::Fixed,
    }];

    static IDL_SEEDS: &[crate::PdaSeedHint] = &[
        crate::PdaSeedHint {
            kind: "literal",
            value: "vault",
        },
        crate::PdaSeedHint {
            kind: "account",
            value: "authority_key",
        },
    ];

    static IDL_ACCOUNTS: &[IdlAccountEntry] = &[
        IdlAccountEntry {
            name: "authority_key",
            writable: false,
            signer: true,
            layout_ref: "",
            pda_seeds: &[],
        },
        IdlAccountEntry {
            name: "vault_account",
            writable: true,
            signer: false,
            layout_ref: "Vault",
            pda_seeds: IDL_SEEDS,
        },
    ];

    static IDL_INSTRUCTIONS: &[IdlInstructionDescriptor] = &[IdlInstructionDescriptor {
        name: "make_deposit",
        tag: 3,
        args: ARGS,
        accounts: IDL_ACCOUNTS,
    }];

    static PROGRAM_IDL: ProgramIdl = ProgramIdl {
        name: "test_program",
        version: "0.3.0",
        description: "Current IDL",
        instructions: IDL_INSTRUCTIONS,
        accounts: LAYOUTS,
        events: &[],
        fingerprints: &[],
    };

    static MANIFEST_ACCOUNTS: &[AccountEntry] = &[
        AccountEntry {
            name: "authority_key",
            writable: false,
            signer: true,
            layout_ref: "",
            seeds: &[],
        },
        AccountEntry {
            name: "vault_account",
            writable: true,
            signer: false,
            layout_ref: "Vault",
            seeds: &["b\"vault\"", "authority_key.key().as_ref()"],
        },
    ];

    static MANIFEST_INSTRUCTIONS: &[InstructionDescriptor] = &[InstructionDescriptor {
        name: "make_deposit",
        tag: 9,
        discriminator: &[9, 8, 7],
        args: ARGS,
        accounts: MANIFEST_ACCOUNTS,
        remaining_accounts: None,
        capabilities: &[],
        policy_pack: "",
        receipt_expected: false,
        strict_writes: false,
        write_ranges: &[],
        parametric_write_ranges: &[],
        mutation_complete: false,
        lamport_accounts: &[],
        cu_estimate: 0,
    }];

    static MANIFEST_CONTEXT_ACCOUNTS: &[crate::accounts::ContextAccountDescriptor] =
        &[crate::accounts::ContextAccountDescriptor {
            name: "authority_key",
            kind: "Signer",
            writable: false,
            signer: true,
            layout_ref: "",
            policy_ref: "",
            seeds: &[],
            optional: false,
            lifecycle: crate::accounts::AccountLifecycle::Existing,
            payer: "",
            init_space: 0,
            has_one: &[],
            expected_address: ADDRESS,
            expected_owner: "",
        }];

    static MANIFEST_CONTEXTS: &[crate::accounts::ContextDescriptor] =
        &[crate::accounts::ContextDescriptor {
            name: "make_deposit",
            accounts: MANIFEST_CONTEXT_ACCOUNTS,
            policies: &[],
            receipts_expected: false,
            mutation_classes: &[],
            strict_writes: false,
            write_ranges: &[],
            parametric_write_ranges: &[],
            mutation_complete: false,
            lamport_accounts: &[],
        }];

    static MANIFEST: ProgramManifest = ProgramManifest {
        name: "test_program",
        version: "0.3.0",
        description: "Current IDL",
        layouts: LAYOUTS,
        layout_metadata: &[],
        instructions: MANIFEST_INSTRUCTIONS,
        events: &[],
        policies: &[],
        compatibility_pairs: &[],
        tooling_hints: &[],
        contexts: MANIFEST_CONTEXTS,
    };

    #[test]
    fn empty_idl_matches_current_top_level_shape_exactly() {
        let rendered = format!(
            "{}",
            AnchorIdlJson {
                idl: &EMPTY_IDL,
                address: ADDRESS,
            }
        );
        assert_eq!(
            rendered,
            concat!(
                "{\n",
                "  \"address\": \"11111111111111111111111111111111\",\n",
                "  \"metadata\": {\n",
                "    \"name\": \"empty_program\",\n",
                "    \"version\": \"0.3.0\",\n",
                "    \"spec\": \"0.1.0\"\n",
                "  },\n",
                "  \"instructions\": [],\n",
                "  \"accounts\": [],\n",
                "  \"events\": [],\n",
                "  \"errors\": [],\n",
                "  \"types\": [],\n",
                "  \"constants\": []\n",
                "}"
            )
        );
    }

    #[test]
    fn current_idl_uses_current_names_types_and_custom_wire_discriminators() {
        assert!(validate_program_idl_projection(&PROGRAM_IDL).is_ok());
        let rendered = format!(
            "{}",
            AnchorIdlJson {
                idl: &PROGRAM_IDL,
                address: ADDRESS,
            }
        );

        assert!(rendered.contains("\"name\": \"makeDeposit\""));
        assert!(rendered.contains("\"discriminator\": [3]"));
        assert!(rendered.contains("\"name\": \"authorityKey\", \"signer\": true"));
        assert!(rendered.contains("\"name\": \"vaultAccount\", \"writable\": true"));
        assert!(!rendered.contains("isMut"));
        assert!(!rendered.contains("isSigner"));
        assert!(!rendered.contains("publicKey"));
        assert!(rendered.contains("\"type\": \"pubkey\""));
        assert!(rendered.contains("\"name\": \"Vault\", \"discriminator\": [4, 1]"));
        assert!(rendered.contains("\"serialization\": { \"custom\": \"hopper-zero-copy-v1\" }"));
        assert!(rendered.contains(
            "\"pda\": { \"seeds\": [{ \"kind\": \"const\", \"value\": [118, 97, 117, 108, 116] }, { \"kind\": \"account\", \"path\": \"authorityKey\" }] }"
        ));
    }

    #[test]
    fn manifest_dynamic_tail_is_rejected_as_lossy_solana_idl() {
        let dynamic_layout = LayoutManifest {
            has_dynamic_tail: true,
            ..LAYOUTS[0]
        };
        let dynamic_manifest = ProgramManifest {
            layouts: std::boxed::Box::leak(std::boxed::Box::new([dynamic_layout])),
            ..MANIFEST
        };
        let projection = AnchorIdlFromManifest {
            manifest: &dynamic_manifest,
            address: ADDRESS,
        };

        assert_eq!(
            projection.validate(),
            Err(AnchorIdlProjectionError::UnsupportedDynamicTail { layout: "Vault" })
        );
        assert!(format!("{}", projection)
            .contains("hopper:anchor-idl-projection:omits-dynamic-tail-contracts"));
    }

    #[test]
    fn manifest_emits_the_exact_multi_byte_instruction_prefix() {
        assert!(validate_program_manifest_projection(&MANIFEST).is_ok());
        let rendered = format!(
            "{}",
            AnchorIdlFromManifest {
                manifest: &MANIFEST,
                address: ADDRESS,
            }
        );

        assert!(rendered.contains("\"discriminator\": [9, 8, 7]"));
        assert!(!rendered.contains("\"discriminator\": [9, 0"));
        assert!(rendered.contains(
            "\"name\": \"authorityKey\", \"signer\": true, \"address\": \"11111111111111111111111111111111\""
        ));
        assert!(rendered.contains("\"name\": \"Vault\", \"discriminator\": [4, 1]"));
        assert!(rendered.contains(
            "\"pda\": { \"seeds\": [{ \"kind\": \"const\", \"value\": [118, 97, 117, 108, 116] }, { \"kind\": \"account\", \"path\": \"authorityKey\" }] }"
        ));
    }

    #[test]
    fn manifest_rejects_an_unresolved_expected_address() {
        static CONTEXT_ACCOUNTS: &[crate::accounts::ContextAccountDescriptor] =
            &[crate::accounts::ContextAccountDescriptor {
                name: "authority_key",
                kind: "Signer",
                writable: false,
                signer: true,
                layout_ref: "",
                policy_ref: "",
                seeds: &[],
                optional: false,
                lifecycle: crate::accounts::AccountLifecycle::Existing,
                payer: "",
                init_space: 0,
                has_one: &[],
                expected_address: "EXPECTED_AUTHORITY",
                expected_owner: "",
            }];
        static CONTEXTS: &[crate::accounts::ContextDescriptor] =
            &[crate::accounts::ContextDescriptor {
                name: "make_deposit",
                accounts: CONTEXT_ACCOUNTS,
                policies: &[],
                receipts_expected: false,
                mutation_classes: &[],
                strict_writes: false,
                write_ranges: &[],
                parametric_write_ranges: &[],
                mutation_complete: false,
                lamport_accounts: &[],
            }];
        static UNRESOLVED_ADDRESS_MANIFEST: ProgramManifest = ProgramManifest {
            name: "unresolved_address",
            version: "0.3.0",
            description: "",
            layouts: LAYOUTS,
            layout_metadata: &[],
            instructions: MANIFEST_INSTRUCTIONS,
            events: &[],
            policies: &[],
            compatibility_pairs: &[],
            tooling_hints: &[],
            contexts: CONTEXTS,
        };

        let projection = AnchorIdlFromManifest {
            manifest: &UNRESOLVED_ADDRESS_MANIFEST,
            address: ADDRESS,
        };
        assert_eq!(
            projection.validate(),
            Err(AnchorIdlProjectionError::UnsupportedAccountAddress {
                instruction: "make_deposit",
                account: "authority_key",
                address: "EXPECTED_AUTHORITY",
            })
        );

        let rendered = format!("{}", projection);
        assert!(!rendered.contains("\"address\": \"EXPECTED_AUTHORITY\""));
        assert!(
            rendered.contains("hopper:anchor-idl-projection:omits-unresolved-account-addresses")
        );
        assert!(is_base58_pubkey(ADDRESS));
        assert!(!is_base58_pubkey("1111111111111111111111111111111"));
        assert!(!is_base58_pubkey("111111111111111111111111111111111"));
    }

    #[test]
    fn manifest_does_not_invent_an_account_seed_for_a_symbolic_constant() {
        static ACCOUNTS: &[AccountEntry] = &[AccountEntry {
            name: "config",
            writable: true,
            signer: false,
            layout_ref: "Vault",
            seeds: &["CONFIG_SEED"],
        }];
        static INSTRUCTIONS: &[InstructionDescriptor] = &[InstructionDescriptor {
            name: "initialize_config",
            tag: 1,
            discriminator: &[1],
            args: &[],
            accounts: ACCOUNTS,
            remaining_accounts: None,
            capabilities: &[],
            policy_pack: "",
            receipt_expected: false,
            strict_writes: false,
            write_ranges: &[],
            parametric_write_ranges: &[],
            mutation_complete: false,
            lamport_accounts: &[],
            cu_estimate: 0,
        }];
        static SYMBOLIC_SEED_MANIFEST: ProgramManifest = ProgramManifest {
            name: "symbolic_seed",
            version: "0.3.0",
            description: "",
            layouts: LAYOUTS,
            layout_metadata: &[],
            instructions: INSTRUCTIONS,
            events: &[],
            policies: &[],
            compatibility_pairs: &[],
            tooling_hints: &[],
            contexts: &[],
        };

        let projection = AnchorIdlFromManifest {
            manifest: &SYMBOLIC_SEED_MANIFEST,
            address: ADDRESS,
        };
        assert_eq!(
            projection.validate(),
            Err(AnchorIdlProjectionError::UnsupportedPdaSeed {
                instruction: "initialize_config",
                account: "config",
                seed: "CONFIG_SEED",
            })
        );
        let rendered = format!("{}", projection);

        assert!(rendered.contains("\"name\": \"config\", \"writable\": true"));
        assert!(!rendered.contains("\"pda\""));
        assert!(!rendered.contains("CONFIGSEED"));
        assert!(rendered.contains("hopper:anchor-idl-projection:partial"));
        assert!(rendered.contains("hopper:anchor-idl-projection:omits-unresolved-pda-seeds"));
    }

    #[test]
    fn manifest_pda_seed_projection_requires_byte_equivalent_spellings() {
        assert!(manifest_seed_supported(
            "deposit_amount.to_le_bytes()",
            MANIFEST_ACCOUNTS,
            ARGS,
        ));
        assert!(!manifest_seed_supported(
            "deposit_amount.to_be_bytes()",
            MANIFEST_ACCOUNTS,
            ARGS,
        ));
        assert!(manifest_seed_supported(
            r#"b"vault""#,
            MANIFEST_ACCOUNTS,
            ARGS,
        ));
        assert!(!manifest_seed_supported(
            r#"b"vault\x00""#,
            MANIFEST_ACCOUNTS,
            ARGS,
        ));
    }

    #[test]
    fn idl_rejects_and_marks_an_unresolved_pda_seed() {
        static SEEDS: &[crate::PdaSeedHint] = &[crate::PdaSeedHint {
            kind: "account",
            value: "CONFIG_SEED",
        }];
        static ACCOUNTS: &[IdlAccountEntry] = &[IdlAccountEntry {
            name: "config",
            writable: true,
            signer: false,
            layout_ref: "Vault",
            pda_seeds: SEEDS,
        }];
        static INSTRUCTIONS: &[IdlInstructionDescriptor] = &[IdlInstructionDescriptor {
            name: "initialize_config",
            tag: 1,
            args: &[],
            accounts: ACCOUNTS,
        }];
        static IDL: ProgramIdl = ProgramIdl {
            name: "symbolic_seed",
            version: "0.3.0",
            description: "",
            instructions: INSTRUCTIONS,
            accounts: &[],
            events: &[],
            fingerprints: &[],
        };

        let projection = AnchorIdlJson {
            idl: &IDL,
            address: ADDRESS,
        };
        assert_eq!(
            projection.validate(),
            Err(AnchorIdlProjectionError::UnsupportedPdaSeed {
                instruction: "initialize_config",
                account: "config",
                seed: "CONFIG_SEED",
            })
        );

        let rendered = format!("{}", projection);
        assert!(rendered.contains("\"name\": \"config\", \"writable\": true"));
        assert!(!rendered.contains("\"pda\""));
        assert!(rendered.contains("hopper:anchor-idl-projection:omits-unresolved-pda-seeds"));
    }

    #[test]
    fn manifest_rejects_equal_or_prefix_overlapping_effective_instruction_discriminators() {
        static INSTRUCTIONS: &[InstructionDescriptor] = &[
            InstructionDescriptor {
                name: "short_prefix",
                tag: 1,
                discriminator: &[],
                args: &[],
                accounts: &[],
                remaining_accounts: None,
                capabilities: &[],
                policy_pack: "",
                receipt_expected: false,
                strict_writes: false,
                write_ranges: &[],
                parametric_write_ranges: &[],
                mutation_complete: false,
                lamport_accounts: &[],
                cu_estimate: 0,
            },
            InstructionDescriptor {
                name: "long_prefix",
                tag: 1,
                discriminator: &[1, 2],
                args: &[],
                accounts: &[],
                remaining_accounts: None,
                capabilities: &[],
                policy_pack: "",
                receipt_expected: false,
                strict_writes: false,
                write_ranges: &[],
                parametric_write_ranges: &[],
                mutation_complete: false,
                lamport_accounts: &[],
                cu_estimate: 0,
            },
            InstructionDescriptor {
                name: "safe_handler",
                tag: 2,
                discriminator: &[2],
                args: &[],
                accounts: &[],
                remaining_accounts: None,
                capabilities: &[],
                policy_pack: "",
                receipt_expected: false,
                strict_writes: false,
                write_ranges: &[],
                parametric_write_ranges: &[],
                mutation_complete: false,
                lamport_accounts: &[],
                cu_estimate: 0,
            },
        ];
        static MANIFEST: ProgramManifest = ProgramManifest {
            name: "ambiguous_instructions",
            version: "0.3.0",
            description: "",
            layouts: &[],
            layout_metadata: &[],
            instructions: INSTRUCTIONS,
            events: &[],
            policies: &[],
            compatibility_pairs: &[],
            tooling_hints: &[],
            contexts: &[],
        };

        let projection = AnchorIdlFromManifest {
            manifest: &MANIFEST,
            address: ADDRESS,
        };
        assert_eq!(
            projection.validate(),
            Err(
                AnchorIdlProjectionError::AmbiguousInstructionDiscriminator {
                    first: "short_prefix",
                    second: "long_prefix",
                }
            )
        );

        let rendered = format!("{}", projection);
        assert!(!rendered.contains("shortPrefix"));
        assert!(!rendered.contains("longPrefix"));
        assert!(rendered.contains("\"name\": \"safeHandler\""));
        assert!(rendered
            .contains("hopper:anchor-idl-projection:omits-ambiguous-instruction-discriminators"));
    }

    #[test]
    fn idl_rejects_duplicate_instruction_tags() {
        static INSTRUCTIONS: &[IdlInstructionDescriptor] = &[
            IdlInstructionDescriptor {
                name: "first_handler",
                tag: 7,
                args: &[],
                accounts: &[],
            },
            IdlInstructionDescriptor {
                name: "second_handler",
                tag: 7,
                args: &[],
                accounts: &[],
            },
        ];
        static IDL: ProgramIdl = ProgramIdl {
            name: "duplicate_tags",
            version: "0.3.0",
            description: "",
            instructions: INSTRUCTIONS,
            accounts: &[],
            events: &[],
            fingerprints: &[],
        };

        let projection = AnchorIdlJson {
            idl: &IDL,
            address: ADDRESS,
        };
        assert_eq!(
            projection.validate(),
            Err(
                AnchorIdlProjectionError::AmbiguousInstructionDiscriminator {
                    first: "first_handler",
                    second: "second_handler",
                }
            )
        );
        let rendered = format!("{}", projection);
        assert!(rendered.contains("\"instructions\": []"));
        assert!(rendered
            .contains("hopper:anchor-idl-projection:omits-ambiguous-instruction-discriminators"));
    }

    #[test]
    fn duplicate_event_tags_are_rejected_and_omitted() {
        static EVENTS: &[EventDescriptor] = &[
            EventDescriptor {
                name: "FirstEvent",
                tag: 12,
                fields: &[],
            },
            EventDescriptor {
                name: "SecondEvent",
                tag: 12,
                fields: &[],
            },
            EventDescriptor {
                name: "SafeEvent",
                tag: 13,
                fields: &[],
            },
        ];
        static IDL: ProgramIdl = ProgramIdl {
            name: "duplicate_events",
            version: "0.3.0",
            description: "",
            instructions: &[],
            accounts: &[],
            events: EVENTS,
            fingerprints: &[],
        };

        let projection = AnchorIdlJson {
            idl: &IDL,
            address: ADDRESS,
        };
        assert_eq!(
            projection.validate(),
            Err(AnchorIdlProjectionError::AmbiguousEventDiscriminator {
                first: "FirstEvent",
                second: "SecondEvent",
            })
        );

        let rendered = format!("{}", projection);
        assert!(!rendered.contains("FirstEvent"));
        assert!(!rendered.contains("SecondEvent"));
        assert!(rendered.contains("\"name\": \"SafeEvent\", \"discriminator\": [13]"));
        assert!(
            rendered.contains("hopper:anchor-idl-projection:omits-ambiguous-event-discriminators")
        );
    }

    #[test]
    fn manifest_remaining_account_contract_is_rejected_and_omitted() {
        static INSTRUCTIONS: &[InstructionDescriptor] = &[
            InstructionDescriptor {
                name: "execute_route",
                tag: 6,
                discriminator: &[6],
                args: &[],
                accounts: &[],
                remaining_accounts: Some(crate::RemainingAccountsDescriptor { max: 32 }),
                capabilities: &[],
                policy_pack: "",
                receipt_expected: false,
                strict_writes: false,
                write_ranges: &[],
                parametric_write_ranges: &[],
                mutation_complete: false,
                lamport_accounts: &[],
                cu_estimate: 0,
            },
            InstructionDescriptor {
                name: "fixed_accounts_only",
                tag: 7,
                discriminator: &[7],
                args: &[],
                accounts: &[],
                remaining_accounts: None,
                capabilities: &[],
                policy_pack: "",
                receipt_expected: false,
                strict_writes: false,
                write_ranges: &[],
                parametric_write_ranges: &[],
                mutation_complete: false,
                lamport_accounts: &[],
                cu_estimate: 0,
            },
        ];
        static MANIFEST: ProgramManifest = ProgramManifest {
            name: "remaining_accounts",
            version: "0.3.0",
            description: "",
            layouts: &[],
            layout_metadata: &[],
            instructions: INSTRUCTIONS,
            events: &[],
            policies: &[],
            compatibility_pairs: &[],
            tooling_hints: &[],
            contexts: &[],
        };

        let projection = AnchorIdlFromManifest {
            manifest: &MANIFEST,
            address: ADDRESS,
        };
        assert_eq!(
            projection.validate(),
            Err(AnchorIdlProjectionError::UnsupportedRemainingAccounts {
                instruction: "execute_route",
            })
        );

        let rendered = format!("{}", projection);
        assert!(!rendered.contains("executeRoute"));
        assert!(rendered.contains("\"name\": \"fixedAccountsOnly\""));
        assert!(rendered.contains("hopper:anchor-idl-projection:omits-remaining-account-contracts"));
    }

    #[test]
    fn bounded_args_are_omitted_while_fixed_symbolic_arrays_resolve_exactly() {
        static ROUTE_ARGS: &[ArgDescriptor] = &[
            ArgDescriptor {
                name: "route_data",
                canonical_type: "HopperVec<u8,MAX_ROUTE_DATA>",
                size: 514,
                encoding: ArgEncoding::BoundedVec {
                    max_len: 512,
                    element_size: 1,
                },
            },
            ArgDescriptor {
                name: "route_meta_flags",
                canonical_type: "[u8;MAX_ROUTE_ACCOUNTS]",
                size: 32,
                encoding: ArgEncoding::Fixed,
            },
        ];
        static FLAGS_ARGS: &[ArgDescriptor] = &[ArgDescriptor {
            name: "route_meta_flags",
            canonical_type: "[u8;MAX_ROUTE_ACCOUNTS]",
            size: 32,
            encoding: ArgEncoding::Fixed,
        }];
        static INSTRUCTIONS: &[IdlInstructionDescriptor] = &[
            IdlInstructionDescriptor {
                name: "execute_intent",
                tag: 6,
                args: ROUTE_ARGS,
                accounts: &[],
            },
            IdlInstructionDescriptor {
                name: "set_route_flags",
                tag: 7,
                args: FLAGS_ARGS,
                accounts: &[],
            },
        ];
        static MANIFEST_INSTRUCTIONS: &[InstructionDescriptor] = &[InstructionDescriptor {
            name: "execute_intent",
            tag: 6,
            discriminator: &[6],
            args: ROUTE_ARGS,
            accounts: &[],
            remaining_accounts: None,
            capabilities: &[],
            policy_pack: "",
            receipt_expected: false,
            strict_writes: false,
            write_ranges: &[],
            parametric_write_ranges: &[],
            mutation_complete: false,
            lamport_accounts: &[],
            cu_estimate: 0,
        }];
        static IDL: ProgramIdl = ProgramIdl {
            name: "bounded_program",
            version: "0.3.0",
            description: "",
            instructions: INSTRUCTIONS,
            accounts: &[],
            events: &[],
            fingerprints: &[],
        };
        static MANIFEST: ProgramManifest = ProgramManifest {
            name: "bounded_program",
            version: "0.3.0",
            description: "",
            layouts: &[],
            layout_metadata: &[],
            instructions: MANIFEST_INSTRUCTIONS,
            events: &[],
            policies: &[],
            compatibility_pairs: &[],
            tooling_hints: &[],
            contexts: &[],
        };

        let projection = AnchorIdlJson {
            idl: &IDL,
            address: ADDRESS,
        };
        assert_eq!(
            projection.validate(),
            Err(AnchorIdlProjectionError::UnsupportedArgumentEncoding {
                instruction: "execute_intent",
                argument: "route_data",
            })
        );
        assert_eq!(
            AnchorIdlFromManifest {
                manifest: &MANIFEST,
                address: ADDRESS,
            }
            .validate(),
            Err(AnchorIdlProjectionError::UnsupportedArgumentEncoding {
                instruction: "execute_intent",
                argument: "route_data",
            })
        );
        let rendered = format!("{}", projection);

        assert!(!rendered.contains("executeIntent"));
        assert!(!rendered.contains("HopperVec"));
        assert!(!rendered.contains("\"defined\""));
        assert!(rendered.contains("hopper:anchor-idl-projection:partial"));
        assert!(rendered
            .contains("hopper:anchor-idl-projection:omits-unsupported-instruction-encodings"));
        assert!(rendered.contains("\"name\": \"setRouteFlags\""));
        assert!(rendered
            .contains("\"name\": \"routeMetaFlags\", \"type\": { \"array\": [\"u8\", 32] }"));
    }

    #[test]
    fn account_prefix_and_symbolic_layout_arrays_match_hopper_wire_bytes() {
        static HEADERED_FIELDS: &[FieldDescriptor] = &[
            FieldDescriptor {
                name: "owners",
                canonical_type: "[Address;INTENTS_PER_SHARD]",
                size: 640,
                offset: 16,
                intent: FieldIntent::Custom,
            },
            FieldDescriptor {
                name: "route_commitments",
                canonical_type: "[[u8;32];INTENTS_PER_SHARD]",
                size: 640,
                offset: 656,
                intent: FieldIntent::Custom,
            },
        ];
        static COMPACT_FIELDS: &[FieldDescriptor] = &[FieldDescriptor {
            name: "value",
            canonical_type: "u64",
            size: 8,
            offset: 1,
            intent: FieldIntent::Custom,
        }];
        static ACCOUNTS: &[LayoutManifest] = &[
            LayoutManifest {
                name: "IntentShard",
                disc: 81,
                version: 2,
                layout_id: [1; 8],
                total_size: 1_296,
                has_dynamic_tail: false,
                field_count: 2,
                fields: HEADERED_FIELDS,
            },
            LayoutManifest {
                name: "CompactValue",
                disc: 7,
                version: 9,
                layout_id: [2; 8],
                total_size: 9,
                has_dynamic_tail: false,
                field_count: 1,
                fields: COMPACT_FIELDS,
            },
        ];
        static IDL: ProgramIdl = ProgramIdl {
            name: "layout_program",
            version: "0.3.0",
            description: "",
            instructions: &[],
            accounts: ACCOUNTS,
            events: &[],
            fingerprints: &[],
        };

        let rendered = format!(
            "{}",
            AnchorIdlJson {
                idl: &IDL,
                address: ADDRESS,
            }
        );

        assert!(rendered.contains("\"name\": \"IntentShard\", \"discriminator\": [81, 2]"));
        assert!(rendered.contains("\"name\": \"CompactValue\", \"discriminator\": [7]"));
        assert!(
            rendered.contains("\"name\": \"owners\", \"type\": { \"array\": [\"pubkey\", 20] }")
        );
        assert!(rendered.contains(
            "\"name\": \"routeCommitments\", \"type\": { \"array\": [{ \"array\": [\"u8\", 32] }, 20] }"
        ));
        assert!(!rendered.contains("INTENTS_PER_SHARD"));
        assert!(!rendered.contains("\"defined\""));
    }

    #[test]
    fn ambiguous_account_prefixes_are_rejected_and_not_advertised() {
        static COLLIDING: &[LayoutManifest] = &[
            LayoutManifest {
                name: "CompactValue",
                disc: 7,
                version: 1,
                layout_id: [1; 8],
                total_size: 9,
                has_dynamic_tail: false,
                field_count: 1,
                fields: &[FieldDescriptor {
                    name: "value",
                    canonical_type: "u64",
                    size: 8,
                    offset: 1,
                    intent: FieldIntent::Custom,
                }],
            },
            LayoutManifest {
                name: "HeaderedValue",
                disc: 7,
                version: 2,
                layout_id: [2; 8],
                total_size: 24,
                has_dynamic_tail: false,
                field_count: 1,
                fields: &[FieldDescriptor {
                    name: "value",
                    canonical_type: "u64",
                    size: 8,
                    offset: 16,
                    intent: FieldIntent::Custom,
                }],
            },
        ];
        static IDL: ProgramIdl = ProgramIdl {
            name: "ambiguous_program",
            version: "0.3.0",
            description: "",
            instructions: &[],
            accounts: COLLIDING,
            events: &[],
            fingerprints: &[],
        };

        let projection = AnchorIdlJson {
            idl: &IDL,
            address: ADDRESS,
        };
        assert_eq!(
            projection.validate(),
            Err(AnchorIdlProjectionError::AmbiguousAccountDiscriminator {
                first: "CompactValue",
                second: "HeaderedValue",
            })
        );

        let rendered = format!("{}", projection);
        assert!(rendered
            .contains("hopper:anchor-idl-projection:omits-ambiguous-account-discriminators"));
        assert!(rendered.contains("\"accounts\": []"));
        assert!(!rendered.contains("\"name\": \"CompactValue\", \"discriminator\""));
        assert!(!rendered.contains("\"name\": \"HeaderedValue\", \"discriminator\""));
    }

    #[test]
    fn constants_use_current_idl_types() {
        static CONSTANTS: &[ConstantDescriptor] = &[ConstantDescriptor {
            name: "ADMIN",
            ty: "Pubkey",
            value: "11111111111111111111111111111111",
            docs: "",
        }];
        let rendered = format!(
            "{}",
            AnchorIdlWithConstants {
                idl: &EMPTY_IDL,
                address: ADDRESS,
                constants: CONSTANTS,
            }
        );
        assert!(rendered.contains(
            "{ \"name\": \"ADMIN\", \"type\": \"pubkey\", \"value\": \"11111111111111111111111111111111\" }"
        ));
    }
}
