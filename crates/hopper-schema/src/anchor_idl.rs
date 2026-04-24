//! # Anchor IDL Emitter
//!
//! Emits an Anchor-compatible IDL (`idl.json`) from a `ProgramManifest`
//! or `ProgramIdl`. Closes audit recommendation R8 from
//! [`../../../AUDIT.md`](../../../AUDIT.md).
//!
//! ## Why this exists
//!
//! Codama is the emerging standard for Solana IDLs, and
//! `hopper::codama::CodamaJsonFromManifest` is Hopper's preferred
//! interop surface. But in 2026 the long tail of wallets, explorers,
//! bundlers, and client generators still consume Anchor IDL JSON
//! directly. Shipping both bridges raises Hopper's adoption ceiling
//! without changing how Hopper programs are authored: the manifest
//! stays the single source of truth, and both emitters project from
//! it.
//!
//! ## What gets emitted
//!
//! The output is the classic Anchor 0.30.x IDL shape:
//!
//! ```text
//! {
//!   "version": "0.1.0",
//!   "name": "my_program",
//!   "metadata": { "description": "..." },
//!   "instructions": [
//!     {
//!       "name": "deposit",
//!       "discriminator": [0, 0, 0, 0, 0, 0, 0, 0],
//!       "accounts": [
//!         { "name": "user", "isMut": true, "isSigner": true },
//!         { "name": "vault", "isMut": true, "isSigner": false }
//!       ],
//!       "args": [ { "name": "amount", "type": "u64" } ]
//!     }
//!   ],
//!   "accounts": [
//!     {
//!       "name": "Vault",
//!       "discriminator": [...],
//!       "type": { "kind": "struct", "fields": [
//!         { "name": "authority", "type": "publicKey" },
//!         { "name": "balance", "type": "u64" }
//!       ]}
//!     }
//!   ],
//!   "events": [...],
//!   "errors": [],
//!   "types": []
//! }
//! ```
//!
//! Notable translation rules:
//!
//! * Anchor discriminators are 8-byte arrays. Hopper `disc: u8` tags
//!   are left-padded with zeros, so a Hopper `disc = 3` becomes
//!   `[3, 0, 0, 0, 0, 0, 0, 0]`. This matches the in-tree
//!   `bench/anchor-vault` convention (R9).
//! * Anchor uses `"isMut"` and `"isSigner"` (not `"writable"` /
//!   `"signer"` like Codama). The emitter converts.
//! * Anchor account types wrap in `{ "kind": "struct", "fields": ... }`
//!   instead of placing fields directly.
//! * Canonical Hopper types map as follows (see `anchor_type_for`):
//!   - `"u64"`, `"i64"`, `"u32"`, `"i32"`, `"u16"`, `"i16"`, `"u8"`,
//!     `"i8"`, `"bool"` → pass through.
//!   - `"WireU64"` / `"WireU32"` / `"WireU16"` / `"WireBool"` → strip
//!     the `Wire` prefix.
//!   - `"[u8; N]"` → `{ "array": ["u8", N] }`.
//!   - `"Pubkey"` / `"TypedAddress<...>"` → `"publicKey"`.
//!   - Everything else is passed through as a string literal so the
//!     downstream consumer can decide. Unknown types come out as
//!     `"unknown_type"` only if the canonical type is empty.
//!
//! All formatters use `core::fmt::Write` so this module stays
//! `#![no_std]`-compatible with the rest of hopper-schema.

use core::fmt;

use crate::{
    ArgDescriptor, EventDescriptor, FieldDescriptor, IdlAccountEntry,
    InstructionDescriptor, LayoutManifest, ProgramIdl, ProgramManifest,
};

// ---------------------------------------------------------------------------
// Shared JSON helpers (self-contained so codama.rs can stay untouched).
// ---------------------------------------------------------------------------

fn write_json_str(f: &mut fmt::Formatter<'_>, s: &str) -> fmt::Result {
    write!(f, "\"")?;
    for c in s.chars() {
        match c {
            '"' => write!(f, "\\\"")?,
            '\\' => write!(f, "\\\\")?,
            '\n' => write!(f, "\\n")?,
            '\r' => write!(f, "\\r")?,
            '\t' => write!(f, "\\t")?,
            _ => write!(f, "{}", c)?,
        }
    }
    write!(f, "\"")
}

fn write_indent(f: &mut fmt::Formatter<'_>, level: usize) -> fmt::Result {
    for _ in 0..level {
        write!(f, "  ")?;
    }
    Ok(())
}

/// Parse `[u8; N]` / `[u16; N]` / `[i64; N]` etc. into the inner type
/// and length. Returns `None` for any other shape.
fn parse_array_type(s: &str) -> Option<(&str, u32)> {
    let s = s.trim();
    let inner = s.strip_prefix('[')?.strip_suffix(']')?;
    let (elem, len_str) = inner.split_once(';')?;
    let len: u32 = len_str.trim().parse().ok()?;
    Some((elem.trim(), len))
}

/// Project a canonical Hopper type string into Anchor's type JSON.
/// Writes directly to the formatter so unknown types can fall back
/// to a quoted string without allocating.
fn write_anchor_type(f: &mut fmt::Formatter<'_>, canonical: &str) -> fmt::Result {
    let c = canonical.trim();
    if c.is_empty() {
        return write_json_str(f, "unknown_type");
    }

    // Passthrough primitives.
    match c {
        "u8" | "u16" | "u32" | "u64" | "u128"
        | "i8" | "i16" | "i32" | "i64" | "i128"
        | "f32" | "f64" | "bool" | "bytes" | "string" => {
            return write_json_str(f, c);
        }
        _ => {}
    }

    // Wire-typed primitives strip the `Wire` prefix.
    if let Some(stripped) = c.strip_prefix("Wire") {
        let lowered = match stripped {
            "U8"   => Some("u8"),
            "U16"  => Some("u16"),
            "U32"  => Some("u32"),
            "U64"  => Some("u64"),
            "U128" => Some("u128"),
            "I8"   => Some("i8"),
            "I16"  => Some("i16"),
            "I32"  => Some("i32"),
            "I64"  => Some("i64"),
            "Bool" => Some("bool"),
            _ => None,
        };
        if let Some(name) = lowered {
            return write_json_str(f, name);
        }
    }

    // Pubkey aliases.
    if c == "Pubkey" || c.starts_with("TypedAddress<") || c == "Address" {
        return write_json_str(f, "publicKey");
    }

    // Array types `[T; N]`.
    if let Some((elem, len)) = parse_array_type(c) {
        write!(f, "{{ \"array\": [")?;
        write_anchor_type(f, elem)?;
        write!(f, ", {}] }}", len)?;
        return Ok(());
    }

    // Option<T>.
    if let Some(inner) = c.strip_prefix("Option<").and_then(|s| s.strip_suffix('>')) {
        write!(f, "{{ \"option\": ")?;
        write_anchor_type(f, inner)?;
        write!(f, " }}")?;
        return Ok(());
    }

    // Vec<T> (rare in zero-copy; emit as { "vec": T }).
    if let Some(inner) = c.strip_prefix("Vec<").and_then(|s| s.strip_suffix('>')) {
        write!(f, "{{ \"vec\": ")?;
        write_anchor_type(f, inner)?;
        write!(f, " }}")?;
        return Ok(());
    }

    // Fallback: treat as a user-defined type name and emit the
    // Anchor `{ "defined": "Name" }` wrapper so the downstream IDL
    // consumer can look it up in `types: [...]`.
    write!(f, "{{ \"defined\": ")?;
    write_json_str(f, c)?;
    write!(f, " }}")
}

/// Write an 8-byte Anchor discriminator array given a `u8` tag.
/// `tag` is left-padded with zeros: `3` becomes `[3, 0, 0, 0, 0, 0, 0, 0]`.
fn write_byte_discriminator(f: &mut fmt::Formatter<'_>, tag: u8) -> fmt::Result {
    write!(
        f,
        "[{}, 0, 0, 0, 0, 0, 0, 0]",
        tag
    )
}

/// Write an 8-byte discriminator array from the first 8 bytes of a
/// layout's fingerprint. Anchor's account discriminators are 8 bytes;
/// Hopper's `layout_id` is already 8 bytes so this is a direct copy.
fn write_layout_discriminator(f: &mut fmt::Formatter<'_>, layout_id: &[u8; 8]) -> fmt::Result {
    write!(f, "[")?;
    for (i, b) in layout_id.iter().enumerate() {
        if i > 0 {
            write!(f, ", ")?;
        }
        write!(f, "{}", b)?;
    }
    write!(f, "]")
}

// ---------------------------------------------------------------------------
// Instruction emitter
// ---------------------------------------------------------------------------

fn write_instruction_accounts(
    f: &mut fmt::Formatter<'_>,
    accounts: &[IdlAccountEntry],
    indent: usize,
) -> fmt::Result {
    if accounts.is_empty() {
        return write!(f, "[]");
    }
    writeln!(f, "[")?;
    for (i, a) in accounts.iter().enumerate() {
        write_indent(f, indent + 1)?;
        write!(f, "{{ \"name\": ")?;
        write_json_str(f, a.name)?;
        write!(
            f,
            ", \"isMut\": {}, \"isSigner\": {}",
            a.writable, a.signer
        )?;
        write!(f, " }}")?;
        if i + 1 < accounts.len() {
            writeln!(f, ",")?;
        } else {
            writeln!(f)?;
        }
    }
    write_indent(f, indent)?;
    write!(f, "]")
}

fn write_instruction_args(
    f: &mut fmt::Formatter<'_>,
    args: &[ArgDescriptor],
    indent: usize,
) -> fmt::Result {
    if args.is_empty() {
        return write!(f, "[]");
    }
    writeln!(f, "[")?;
    for (i, arg) in args.iter().enumerate() {
        write_indent(f, indent + 1)?;
        write!(f, "{{ \"name\": ")?;
        write_json_str(f, arg.name)?;
        write!(f, ", \"type\": ")?;
        write_anchor_type(f, arg.canonical_type)?;
        write!(f, " }}")?;
        if i + 1 < args.len() {
            writeln!(f, ",")?;
        } else {
            writeln!(f)?;
        }
    }
    write_indent(f, indent)?;
    write!(f, "]")
}

fn write_instruction(
    f: &mut fmt::Formatter<'_>,
    ix: &InstructionDescriptor,
    indent: usize,
) -> fmt::Result {
    write_indent(f, indent)?;
    writeln!(f, "{{")?;
    write_indent(f, indent + 1)?;
    write!(f, "\"name\": ")?;
    write_json_str(f, ix.name)?;
    writeln!(f, ",")?;
    write_indent(f, indent + 1)?;
    write!(f, "\"discriminator\": ")?;
    write_byte_discriminator(f, ix.tag)?;
    writeln!(f, ",")?;
    write_indent(f, indent + 1)?;
    write!(f, "\"accounts\": ")?;
    write_instruction_accounts(f, ix.accounts, indent + 1)?;
    writeln!(f, ",")?;
    write_indent(f, indent + 1)?;
    write!(f, "\"args\": ")?;
    write_instruction_args(f, ix.args, indent + 1)?;
    writeln!(f)?;
    write_indent(f, indent)?;
    write!(f, "}}")
}

// ---------------------------------------------------------------------------
// Account layout emitter
// ---------------------------------------------------------------------------

fn write_account_fields(
    f: &mut fmt::Formatter<'_>,
    fields: &[FieldDescriptor],
    indent: usize,
) -> fmt::Result {
    if fields.is_empty() {
        return write!(f, "[]");
    }
    writeln!(f, "[")?;
    for (i, field) in fields.iter().enumerate() {
        write_indent(f, indent + 1)?;
        write!(f, "{{ \"name\": ")?;
        write_json_str(f, field.name)?;
        write!(f, ", \"type\": ")?;
        write_anchor_type(f, field.canonical_type)?;
        write!(f, " }}")?;
        if i + 1 < fields.len() {
            writeln!(f, ",")?;
        } else {
            writeln!(f)?;
        }
    }
    write_indent(f, indent)?;
    write!(f, "]")
}

fn write_account_layout(
    f: &mut fmt::Formatter<'_>,
    layout: &LayoutManifest,
    indent: usize,
) -> fmt::Result {
    write_indent(f, indent)?;
    writeln!(f, "{{")?;
    write_indent(f, indent + 1)?;
    write!(f, "\"name\": ")?;
    write_json_str(f, layout.name)?;
    writeln!(f, ",")?;
    write_indent(f, indent + 1)?;
    write!(f, "\"discriminator\": ")?;
    write_layout_discriminator(f, &layout.layout_id)?;
    writeln!(f, ",")?;
    write_indent(f, indent + 1)?;
    writeln!(f, "\"type\": {{")?;
    write_indent(f, indent + 2)?;
    writeln!(f, "\"kind\": \"struct\",")?;
    write_indent(f, indent + 2)?;
    write!(f, "\"fields\": ")?;
    write_account_fields(f, layout.fields, indent + 2)?;
    writeln!(f)?;
    write_indent(f, indent + 1)?;
    writeln!(f, "}}")?;
    write_indent(f, indent)?;
    write!(f, "}}")
}

// ---------------------------------------------------------------------------
// Event emitter
// ---------------------------------------------------------------------------

fn write_event(
    f: &mut fmt::Formatter<'_>,
    event: &EventDescriptor,
    indent: usize,
) -> fmt::Result {
    write_indent(f, indent)?;
    writeln!(f, "{{")?;
    write_indent(f, indent + 1)?;
    write!(f, "\"name\": ")?;
    write_json_str(f, event.name)?;
    writeln!(f, ",")?;
    write_indent(f, indent + 1)?;
    write!(f, "\"discriminator\": ")?;
    write_byte_discriminator(f, event.tag)?;
    writeln!(f, ",")?;
    write_indent(f, indent + 1)?;
    write!(f, "\"fields\": ")?;
    write_account_fields(f, event.fields, indent + 1)?;
    writeln!(f)?;
    write_indent(f, indent)?;
    write!(f, "}}")
}

// ---------------------------------------------------------------------------
// Top-level emitters
// ---------------------------------------------------------------------------

fn write_preamble(
    f: &mut fmt::Formatter<'_>,
    name: &str,
    version: &str,
    description: &str,
) -> fmt::Result {
    writeln!(f, "{{")?;
    write!(f, "  \"version\": ")?;
    write_json_str(f, version)?;
    writeln!(f, ",")?;
    write!(f, "  \"name\": ")?;
    write_json_str(f, name)?;
    writeln!(f, ",")?;
    if !description.is_empty() {
        writeln!(f, "  \"metadata\": {{")?;
        write!(f, "    \"description\": ")?;
        write_json_str(f, description)?;
        writeln!(f)?;
        writeln!(f, "  }},")?;
    } else {
        writeln!(f, "  \"metadata\": {{}},")?;
    }
    Ok(())
}

fn write_instruction_array(
    f: &mut fmt::Formatter<'_>,
    instructions: &[InstructionDescriptor],
) -> fmt::Result {
    if instructions.is_empty() {
        return writeln!(f, "  \"instructions\": [],");
    }
    writeln!(f, "  \"instructions\": [")?;
    for (i, ix) in instructions.iter().enumerate() {
        write_instruction(f, ix, 2)?;
        if i + 1 < instructions.len() {
            writeln!(f, ",")?;
        } else {
            writeln!(f)?;
        }
    }
    writeln!(f, "  ],")
}

fn write_account_array(
    f: &mut fmt::Formatter<'_>,
    layouts: &[LayoutManifest],
) -> fmt::Result {
    if layouts.is_empty() {
        return writeln!(f, "  \"accounts\": [],");
    }
    writeln!(f, "  \"accounts\": [")?;
    for (i, l) in layouts.iter().enumerate() {
        write_account_layout(f, l, 2)?;
        if i + 1 < layouts.len() {
            writeln!(f, ",")?;
        } else {
            writeln!(f)?;
        }
    }
    writeln!(f, "  ],")
}

fn write_event_array(
    f: &mut fmt::Formatter<'_>,
    events: &[EventDescriptor],
) -> fmt::Result {
    if events.is_empty() {
        return writeln!(f, "  \"events\": [],");
    }
    writeln!(f, "  \"events\": [")?;
    for (i, e) in events.iter().enumerate() {
        write_event(f, e, 2)?;
        if i + 1 < events.len() {
            writeln!(f, ",")?;
        } else {
            writeln!(f)?;
        }
    }
    writeln!(f, "  ],")
}

// ---------------------------------------------------------------------------
// Public wrapper types
// ---------------------------------------------------------------------------

/// Emit an Anchor-style IDL JSON directly from a `ProgramIdl`
/// (the public-schema projection).
pub struct AnchorIdlJson<'a>(pub &'a ProgramIdl);

impl<'a> fmt::Display for AnchorIdlJson<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let idl = self.0;
        write_preamble(f, idl.name, idl.version, idl.description)?;
        write_instruction_array(f, idl.instructions)?;
        write_account_array(f, idl.accounts)?;
        write_event_array(f, idl.events)?;
        // Anchor IDLs always have these even if empty.
        writeln!(f, "  \"errors\": [],")?;
        writeln!(f, "  \"types\": []")?;
        write!(f, "}}")
    }
}

/// Project a full `ProgramManifest` into an Anchor-style IDL JSON.
/// Strips policy, receipts, capabilities, trust metadata, and
/// migration hints — keeps only what Anchor's IDL consumers expect.
pub struct AnchorIdlFromManifest<'a>(pub &'a ProgramManifest);

impl<'a> fmt::Display for AnchorIdlFromManifest<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let m = self.0;
        write_preamble(f, m.name, m.version, m.description)?;
        write_instruction_array(f, m.instructions)?;
        write_account_array(f, m.layouts)?;
        write_event_array(f, m.events)?;
        writeln!(f, "  \"errors\": [],")?;
        writeln!(f, "  \"types\": []")?;
        write!(f, "}}")
    }
}
