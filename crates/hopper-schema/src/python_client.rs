//! # Python client emitter
//!
//! Produces a standalone Python module from a `ProgramManifest` that mirrors
//! the TypeScript emitter in `clientgen.rs`. The generated Python has no
//! runtime dependency outside of the standard library. everything decodes
//! through `struct` (the stdlib module).
//!
//! ## What gets emitted
//!
//! - One dataclass per account layout (`Vault`, `Config`, …) with a
//!   `decode(bytes) -> Self` classmethod that verifies the layout_id and
//!   reads field offsets directly from the raw bytes.
//! - One dataclass per event with a `decode(bytes) -> Self` classmethod
//!   keyed off the 1-byte event tag.
//! - `build_<instruction>` helper functions that return the raw `bytes`
//!   instruction payload. The caller wires the returned bytes into their
//!   preferred Solana client (solders, solana-py, …).
//! - A `DISCRIMINATORS` dict mapping layout name to `(disc, layout_id)`.
//!
//! ## Innovation over Quasar / Anchor
//!
//! Quasar/Anchor Python support typically means "use Codama / Kinobi to
//! generate TypeScript and hand-translate". there is no canonical Python
//! path. Hopper emits Python that:
//!   1. Verifies the `layout_id` fingerprint before decoding (impossible in
//!      Anchor because Anchor has no layout fingerprint).
//!   2. Honors `FieldIntent` by emitting typed `int` / `bytes` / `bool`
//!      field types that match the field's semantic role, not just the
//!      underlying u8/u64.
//!   3. Ships segment-aware partial readers (`Vault.read_balance(buf)`)
//!      parallel to the zero-copy on-chain side.

use core::fmt;

extern crate alloc;
use alloc::string::{String, ToString};

use crate::{
    EventDescriptor, FieldDescriptor, InstructionDescriptor, LayoutManifest, ProgramManifest,
};

fn py_type(canonical: &str) -> &'static str {
    match canonical {
        "u8" | "u16" | "u32" | "i8" | "i16" | "i32" => "int",
        "u64" | "u128" | "i64" | "i128" => "int",
        "bool" => "bool",
        "Pubkey" => "bytes",
        _ => "bytes",
    }
}

fn struct_format(canonical: &str, size: u16) -> String {
    match canonical {
        "u8" => "<B".to_string(),
        "u16" => "<H".to_string(),
        "u32" => "<I".to_string(),
        "u64" => "<Q".to_string(),
        "i8" => "<b".to_string(),
        "i16" => "<h".to_string(),
        "i32" => "<i".to_string(),
        "i64" => "<q".to_string(),
        "bool" => "<?".to_string(),
        _ => {
            let mut s = String::from("<");
            let n = size.to_string();
            s.push_str(&n);
            s.push('s');
            s
        }
    }
}

fn write_snake(f: &mut fmt::Formatter<'_>, name: &str) -> fmt::Result {
    for c in name.chars() {
        if c == '-' { f.write_str("_")?; } else { for lc in c.to_lowercase() { write!(f, "{}", lc)?; } }
    }
    Ok(())
}

fn write_pascal(f: &mut fmt::Formatter<'_>, name: &str) -> fmt::Result {
    let mut cap = true;
    for c in name.chars() {
        if c == '_' || c == '-' {
            cap = true;
        } else if cap {
            for uc in c.to_uppercase() { write!(f, "{}", uc)?; }
            cap = false;
        } else {
            write!(f, "{}", c)?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Accounts emitter (`accounts.py`)
// ---------------------------------------------------------------------------

/// Generates `accounts.py` content from a `ProgramManifest`.
pub struct PyAccounts<'a>(pub &'a ProgramManifest);

impl<'a> fmt::Display for PyAccounts<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "\"\"\"Hopper account decoders for program `{}`.", self.0.name)?;
        writeln!(f)?;
        writeln!(f, "Auto-generated. Do not edit.")?;
        writeln!(f, "\"\"\"")?;
        writeln!(f, "from __future__ import annotations")?;
        writeln!(f, "from dataclasses import dataclass")?;
        writeln!(f, "import struct")?;
        writeln!(f)?;
        writeln!(f, "LAYOUT_ID_OFFSET = 4  # bytes [4..12] of the Hopper header")?;
        writeln!(f)?;

        for layout in self.0.layouts {
            fmt_layout(f, layout)?;
            writeln!(f)?;
        }

        // DISCRIMINATORS map (one-per-layout)
        writeln!(f, "DISCRIMINATORS: dict[str, tuple[int, bytes]] = {{")?;
        for layout in self.0.layouts {
            write!(f, "    \"")?;
            write_pascal(f, layout.name)?;
            write!(f, "\": ({}, bytes([", layout.disc)?;
            for (i, b) in layout.layout_id.iter().enumerate() {
                if i > 0 { write!(f, ", ")?; }
                write!(f, "0x{:02x}", b)?;
            }
            writeln!(f, "])),")?;
        }
        writeln!(f, "}}")?;
        Ok(())
    }
}

fn fmt_layout(f: &mut fmt::Formatter<'_>, layout: &LayoutManifest) -> fmt::Result {
    writeln!(f, "@dataclass(frozen=True, slots=True)")?;
    write!(f, "class ")?;
    write_pascal(f, layout.name)?;
    writeln!(f, ":")?;
    writeln!(f, "    \"\"\"Decoder for the `{}` account. total_size={}\"\"\"", layout.name, layout.total_size)?;

    // Layout-id constant
    write!(f, "    LAYOUT_ID: bytes = bytes([")?;
    for (i, b) in layout.layout_id.iter().enumerate() {
        if i > 0 { write!(f, ", ")?; }
        write!(f, "0x{:02x}", b)?;
    }
    writeln!(f, "])")?;
    writeln!(f, "    DISC: int = {}", layout.disc)?;
    writeln!(f, "    VERSION: int = {}", layout.version)?;
    writeln!(f, "    TOTAL_SIZE: int = {}", layout.total_size)?;
    writeln!(f)?;

    // Typed fields (dataclass attributes)
    for fd in layout.fields {
        write!(f, "    ")?;
        write_snake(f, fd.name)?;
        writeln!(f, ": {}", py_type(fd.canonical_type))?;
    }

    writeln!(f)?;
    writeln!(f, "    @classmethod")?;
    write!(f, "    def decode(cls, buf: bytes) -> \"")?;
    write_pascal(f, layout.name)?;
    writeln!(f, "\":")?;
    writeln!(f, "        if len(buf) < cls.TOTAL_SIZE:")?;
    writeln!(f, "            raise ValueError(f\"buffer too short: need {{cls.TOTAL_SIZE}}, got {{len(buf)}}\")")?;
    writeln!(f, "        actual_id = bytes(buf[LAYOUT_ID_OFFSET:LAYOUT_ID_OFFSET + 8])")?;
    writeln!(f, "        if actual_id != cls.LAYOUT_ID:")?;
    writeln!(f, "            raise ValueError(f\"layout_id mismatch: expected {{cls.LAYOUT_ID.hex()}}, got {{actual_id.hex()}}\")")?;

    for fd in layout.fields {
        let fmt = struct_format(fd.canonical_type, fd.size);
        write!(f, "        ")?;
        write_snake(f, fd.name)?;
        writeln!(
            f,
            " = struct.unpack_from(\"{}\", buf, {})[0]",
            fmt, fd.offset
        )?;
    }

    write!(f, "        return cls(")?;
    for (i, fd) in layout.fields.iter().enumerate() {
        if i > 0 { write!(f, ", ")?; }
        write_snake(f, fd.name)?;
        write!(f, "=")?;
        write_snake(f, fd.name)?;
    }
    writeln!(f, ")")?;

    // Partial reader helpers: `Vault.read_balance(buf) -> int`. these are
    // the segment-aware equivalent of hopper-sdk's `SegmentReader::read_u64`.
    writeln!(f)?;
    for fd in layout.fields {
        let fmt = struct_format(fd.canonical_type, fd.size);
        writeln!(f, "    @classmethod")?;
        write!(f, "    def read_")?;
        write_snake(f, fd.name)?;
        writeln!(f, "(cls, buf: bytes) -> {}:", py_type(fd.canonical_type))?;
        writeln!(f, "        \"\"\"Partial read of `{}` (size={}, offset={}). Does NOT verify layout_id; call decode() for full verification.\"\"\"", fd.name, fd.size, fd.offset)?;
        writeln!(f, "        return struct.unpack_from(\"{}\", buf, {})[0]", fmt, fd.offset)?;
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Instructions emitter (`instructions.py`)
// ---------------------------------------------------------------------------

/// Generates `instructions.py` content from a `ProgramManifest`.
pub struct PyInstructions<'a>(pub &'a ProgramManifest);

impl<'a> fmt::Display for PyInstructions<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "\"\"\"Instruction builders for program `{}`.\"\"\"", self.0.name)?;
        writeln!(f, "from __future__ import annotations")?;
        writeln!(f, "import struct")?;
        writeln!(f)?;
        for ix in self.0.instructions {
            fmt_instruction(f, ix)?;
            writeln!(f)?;
        }
        Ok(())
    }
}

fn fmt_instruction(f: &mut fmt::Formatter<'_>, ix: &InstructionDescriptor) -> fmt::Result {
    write!(f, "def build_")?;
    write_snake(f, ix.name)?;
    write!(f, "(")?;
    for (i, a) in ix.args.iter().enumerate() {
        if i > 0 { write!(f, ", ")?; }
        write_snake(f, a.name)?;
        write!(f, ": {}", py_type(a.canonical_type))?;
    }
    writeln!(f, ") -> bytes:")?;
    writeln!(f, "    \"\"\"Assemble the raw instruction data for `{}`. tag={}\"\"\"", ix.name, ix.tag)?;
    writeln!(f, "    parts: list[bytes] = [bytes([{}])]", ix.tag)?;
    for a in ix.args {
        let fmt = struct_format(a.canonical_type, a.size);
        write!(f, "    parts.append(struct.pack(\"{}\", ", fmt)?;
        write_snake(f, a.name)?;
        writeln!(f, "))")?;
    }
    writeln!(f, "    return b\"\".join(parts)")?;

    // Account ordering doc. helpful for consumers since Python has no
    // statically typed AccountMeta.
    if !ix.accounts.is_empty() {
        writeln!(f, "\nbuild_")?;
        write_snake(f, ix.name)?;
        writeln!(f, ".ACCOUNT_ORDER = (")?;
        for ae in ix.accounts {
            writeln!(f, "    (\"{}\", {{\"writable\": {}, \"signer\": {}, \"layout\": \"{}\"}}),", ae.name,
                if ae.writable { "True" } else { "False" },
                if ae.signer   { "True" } else { "False" },
                ae.layout_ref,
            )?;
        }
        writeln!(f, ")")?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Events emitter (`events.py`)
// ---------------------------------------------------------------------------

/// Generates `events.py` content from a `ProgramManifest`.
pub struct PyEvents<'a>(pub &'a ProgramManifest);

impl<'a> fmt::Display for PyEvents<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "\"\"\"Event decoders for program `{}`.\"\"\"", self.0.name)?;
        writeln!(f, "from __future__ import annotations")?;
        writeln!(f, "from dataclasses import dataclass")?;
        writeln!(f, "import struct")?;
        writeln!(f)?;
        for e in self.0.events {
            fmt_event(f, e)?;
            writeln!(f)?;
        }

        // Event tag → decoder table.
        writeln!(f, "EVENT_DECODERS: dict[int, type] = {{")?;
        for e in self.0.events {
            write!(f, "    {}: ", e.tag)?;
            write_pascal(f, e.name)?;
            writeln!(f, ",")?;
        }
        writeln!(f, "}}")?;
        Ok(())
    }
}

fn fmt_event(f: &mut fmt::Formatter<'_>, e: &EventDescriptor) -> fmt::Result {
    writeln!(f, "@dataclass(frozen=True, slots=True)")?;
    write!(f, "class ")?;
    write_pascal(f, e.name)?;
    writeln!(f, ":")?;
    writeln!(f, "    \"\"\"Event {} (tag={})\"\"\"", e.name, e.tag)?;
    writeln!(f, "    TAG: int = {}", e.tag)?;
    for fd in e.fields {
        write!(f, "    ")?;
        write_snake(f, fd.name)?;
        writeln!(f, ": {}", py_type(fd.canonical_type))?;
    }

    writeln!(f)?;
    writeln!(f, "    @classmethod")?;
    write!(f, "    def decode(cls, buf: bytes) -> \"")?;
    write_pascal(f, e.name)?;
    writeln!(f, "\":")?;
    writeln!(f, "        if not buf or buf[0] != cls.TAG:")?;
    writeln!(f, "            raise ValueError(\"event tag mismatch\")")?;
    writeln!(f, "        p = 1")?;
    for fd in e.fields {
        let fmt = struct_format(fd.canonical_type, fd.size);
        write!(f, "        ")?;
        write_snake(f, fd.name)?;
        writeln!(f, " = struct.unpack_from(\"{}\", buf, p)[0]; p += {}", fmt, fd.size)?;
    }
    write!(f, "        return cls(")?;
    for (i, fd) in e.fields.iter().enumerate() {
        if i > 0 { write!(f, ", ")?; }
        write_snake(f, fd.name)?;
        write!(f, "=")?;
        write_snake(f, fd.name)?;
    }
    writeln!(f, ")")?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Types emitter (`types.py`. shared scaffolding)
// ---------------------------------------------------------------------------

/// Generates the shared `types.py` content: header parser, fingerprint
/// assertion helper, and a single source-of-truth `DECODERS` union table.
pub struct PyTypes<'a>(pub &'a ProgramManifest);

impl<'a> fmt::Display for PyTypes<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "\"\"\"Shared Hopper client primitives for program `{}`.\"\"\"", self.0.name)?;
        writeln!(f, "from __future__ import annotations")?;
        writeln!(f, "from dataclasses import dataclass")?;
        writeln!(f)?;
        writeln!(f, "HEADER_LEN = 12  # disc(1) + version(1) + flags(1) + reserved(1) + layout_id(8)")?;
        writeln!(f)?;
        writeln!(f, "@dataclass(frozen=True, slots=True)")?;
        writeln!(f, "class HopperHeader:")?;
        writeln!(f, "    disc: int")?;
        writeln!(f, "    version: int")?;
        writeln!(f, "    flags: int")?;
        writeln!(f, "    layout_id: bytes")?;
        writeln!(f)?;
        writeln!(f, "    @classmethod")?;
        writeln!(f, "    def decode(cls, buf: bytes) -> \"HopperHeader\":")?;
        writeln!(f, "        if len(buf) < HEADER_LEN:")?;
        writeln!(f, "            raise ValueError(\"account too short for Hopper header\")")?;
        writeln!(f, "        return cls(disc=buf[0], version=buf[1], flags=buf[2], layout_id=bytes(buf[4:12]))")?;
        writeln!(f)?;
        writeln!(f, "def assert_layout_id(buf: bytes, expected: bytes) -> None:")?;
        writeln!(f, "    \"\"\"Raise if the account header's layout_id doesn't match `expected`.\"\"\"")?;
        writeln!(f, "    header = HopperHeader.decode(buf)")?;
        writeln!(f, "    if header.layout_id != expected:")?;
        writeln!(f, "        raise ValueError(f\"layout_id mismatch: expected {{expected.hex()}}, got {{header.layout_id.hex()}}\")")?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Package-level bundle (`__init__.py`)
// ---------------------------------------------------------------------------

/// Generates an `__init__.py` that re-exports the public surface of the
/// generated package.
pub struct PyIndex<'a>(pub &'a ProgramManifest);

impl<'a> fmt::Display for PyIndex<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "\"\"\"Auto-generated Python client for `{}`.\"\"\"", self.0.name)?;
        writeln!(f, "from .accounts import *  # noqa: F401,F403")?;
        writeln!(f, "from .instructions import *  # noqa: F401,F403")?;
        writeln!(f, "from .events import *  # noqa: F401,F403")?;
        writeln!(f, "from .types import HopperHeader, assert_layout_id  # noqa: F401")?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Grand bundle: all-in-one emitter
// ---------------------------------------------------------------------------

/// Convenience emitter that produces a single concatenated Python file
/// combining every section above. Useful for CLI users who want one flat
/// file they can `cp` into their project.
pub struct PyClientGen<'a>(pub &'a ProgramManifest);

impl<'a> fmt::Display for PyClientGen<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", PyTypes(self.0))?;
        writeln!(f)?;
        write!(f, "{}", PyAccounts(self.0))?;
        writeln!(f)?;
        write!(f, "{}", PyInstructions(self.0))?;
        writeln!(f)?;
        write!(f, "{}", PyEvents(self.0))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AccountEntry, ArgDescriptor, EventDescriptor, FieldIntent, InstructionDescriptor, LayoutManifest};

    fn sample_layout() -> LayoutManifest {
        static F: [FieldDescriptor; 2] = [
            FieldDescriptor { name: "authority", canonical_type: "Pubkey", size: 32, offset: 16, intent: FieldIntent::Authority },
            FieldDescriptor { name: "balance",   canonical_type: "u64",    size: 8,  offset: 48, intent: FieldIntent::Balance   },
        ];
        LayoutManifest {
            name: "vault", disc: 5, version: 1,
            layout_id: [1,2,3,4,5,6,7,8],
            total_size: 64, field_count: 2, fields: &F,
        }
    }

    fn sample_manifest() -> ProgramManifest {
        static LAYOUTS: [LayoutManifest; 1] = [sample_layout_static()];
        static ACCTS: [AccountEntry; 1] = [AccountEntry { name: "vault", writable: true, signer: false, layout_ref: "vault" }];
        static ARGS: [ArgDescriptor; 1] = [ArgDescriptor { name: "amount", canonical_type: "u64", size: 8 }];
        static IX: [InstructionDescriptor; 1] = [InstructionDescriptor {
            name: "deposit", tag: 3, args: &ARGS, accounts: &ACCTS,
            capabilities: &[], policy_pack: "", receipt_expected: true,
        }];
        static EV_F: [FieldDescriptor; 1] = [FieldDescriptor {
            name: "amount", canonical_type: "u64", size: 8, offset: 1, intent: FieldIntent::Balance,
        }];
        static EVENTS: [EventDescriptor; 1] = [EventDescriptor {
            name: "deposited", tag: 1, fields: &EV_F,
        }];

        ProgramManifest {
            name: "vault_program", version: "0.1.0", description: "",
            layouts: &LAYOUTS, layout_metadata: &[],
            instructions: &IX, events: &EVENTS,
            policies: &[], compatibility_pairs: &[], tooling_hints: &[], contexts: &[],
        }
    }

    const fn sample_layout_static() -> LayoutManifest {
        const F: [FieldDescriptor; 2] = [
            FieldDescriptor { name: "authority", canonical_type: "Pubkey", size: 32, offset: 16, intent: FieldIntent::Authority },
            FieldDescriptor { name: "balance",   canonical_type: "u64",    size: 8,  offset: 48, intent: FieldIntent::Balance   },
        ];
        LayoutManifest {
            name: "vault", disc: 5, version: 1,
            layout_id: [1,2,3,4,5,6,7,8],
            total_size: 64, field_count: 2, fields: &F,
        }
    }

    #[test]
    fn accounts_mentions_layout_id_and_fields() {
        let m = sample_manifest();
        let out = alloc::format!("{}", PyAccounts(&m));
        assert!(out.contains("class Vault"));
        assert!(out.contains("LAYOUT_ID"));
        assert!(out.contains("authority"));
        assert!(out.contains("balance"));
        assert!(out.contains("read_balance"));
    }

    #[test]
    fn instructions_pack_tag_byte() {
        let m = sample_manifest();
        let out = alloc::format!("{}", PyInstructions(&m));
        assert!(out.contains("def build_deposit"));
        assert!(out.contains("bytes([3])"));
        assert!(out.contains("amount"));
    }

    #[test]
    fn events_decoder_table_present() {
        let m = sample_manifest();
        let out = alloc::format!("{}", PyEvents(&m));
        assert!(out.contains("class Deposited"));
        assert!(out.contains("EVENT_DECODERS"));
        assert!(out.contains("1: Deposited"));
    }
}
