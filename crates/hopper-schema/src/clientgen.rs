//! # Client Generation Module
//!
//! Generates typed client SDKs from Hopper schema types.
//!
//! The generator operates on the in-memory schema types (`ProgramManifest`,
//! `LayoutManifest`, `InstructionDescriptor`, etc.) and produces source code
//! via `core::fmt::Display` wrappers.
//!
//! ## Supported Languages
//!
//! - **TypeScript**: Full SDK with accounts, instructions, events, types.
//! - **Kotlin**: Android SDK for Solana Mobile, using `@solana/web3.js` patterns
//!   mapped to `com.solana.mobilewalletadapter` and `org.sol4k`.
//! - **Python, Go, C, Rust**: Transport-neutral clients in sibling modules.
//!
//! ## Usage
//!
//! ```text
//! hopper client gen --ts @my-program.manifest.json
//! hopper client gen --kt @my-program.manifest.json
//! hopper client gen --go @my-program.manifest.json
//! hopper client gen --c @my-program.manifest.json
//! ```

use core::fmt;

extern crate alloc;

use crate::{
    ArgDescriptor, ArgEncoding, EventDescriptor, InstructionDescriptor, LayoutManifest,
    ProgramManifest,
};

const HOPPER_HEADER_SIZE: usize = 16;

fn layout_is_compact(layout: &LayoutManifest) -> bool {
    layout.fields.iter().map(|field| field.offset).min() == Some(1)
        || layout.total_size < HOPPER_HEADER_SIZE
}

// ---------------------------------------------------------------------------
// Type Mapping
// ---------------------------------------------------------------------------

/// Map a Hopper canonical type name to a TypeScript type.
fn ts_type(canonical: &str) -> &str {
    match canonical {
        "u8" | "u16" | "u32" | "i8" | "i16" | "i32" => "number",
        "u64" | "u128" | "i64" | "i128" => "bigint",
        "bool" => "boolean",
        "Pubkey" => "PublicKey",
        // `[u8; N]` byte arrays and any unknown type both map to `Uint8Array`.
        _ => "Uint8Array",
    }
}

fn bounded_vec_element_type(canonical: &str) -> Option<&str> {
    let start = canonical.find('<')? + 1;
    let rest = &canonical[start..];
    let mut depth = 0usize;
    for (index, byte) in rest.bytes().enumerate() {
        match byte {
            b'<' | b'[' | b'(' => depth += 1,
            b'>' | b']' | b')' => depth = depth.saturating_sub(1),
            b',' if depth == 0 => return Some(&rest[..index]),
            _ => {}
        }
    }
    None
}

fn write_ts_arg_type(f: &mut fmt::Formatter<'_>, arg: &ArgDescriptor) -> fmt::Result {
    match arg.encoding {
        ArgEncoding::BoundedString { .. } => write!(f, "string"),
        ArgEncoding::BoundedVec { .. } => {
            let element = bounded_vec_element_type(arg.canonical_type).unwrap_or("u8");
            if element == "u8" {
                write!(f, "Uint8Array")
            } else {
                write!(f, "readonly {}[]", ts_type(element))
            }
        }
        ArgEncoding::Fixed => write!(f, "{}", ts_type(arg.canonical_type)),
    }
}

/// PascalCase from snake_case or as-is.
fn write_pascal(f: &mut fmt::Formatter<'_>, name: &str) -> fmt::Result {
    let mut capitalize_next = true;
    for c in name.chars() {
        if c == '_' || c == '-' {
            capitalize_next = true;
        } else if capitalize_next {
            for uc in c.to_uppercase() {
                write!(f, "{}", uc)?;
            }
            capitalize_next = false;
        } else {
            write!(f, "{}", c)?;
        }
    }
    Ok(())
}

/// camelCase from snake_case or as-is.
/// Write `text` as a double-quoted TypeScript string literal, escaping `\` and
/// `"` so seed literals like `b"a\"b"` stay valid in the generated source.
fn write_ts_string(f: &mut fmt::Formatter<'_>, text: &str) -> fmt::Result {
    write!(f, "\"")?;
    for c in text.chars() {
        match c {
            '\\' => write!(f, "\\\\")?,
            '"' => write!(f, "\\\"")?,
            other => write!(f, "{}", other)?,
        }
    }
    write!(f, "\"")
}

/// Write `text` as a double-quoted Kotlin string literal, escaping `\`, `"`,
/// and `$` (Kotlin string-template sigil) so seed literals stay valid.
fn write_kt_string(f: &mut fmt::Formatter<'_>, text: &str) -> fmt::Result {
    write!(f, "\"")?;
    for c in text.chars() {
        match c {
            '\\' => write!(f, "\\\\")?,
            '"' => write!(f, "\\\"")?,
            '$' => write!(f, "\\$")?,
            other => write!(f, "{}", other)?,
        }
    }
    write!(f, "\"")
}

/// Whether a generated client should auto-resolve `acc` as a program-derived
/// address. True only when the account has seeds and *every* seed is a byte
/// literal or another account reference — anything an arg/unknown seed would
/// require encoding is left caller-provided so a client never derives a wrong
/// address. Shared by every language generator.
pub(crate) fn account_is_auto_pda(acc: &crate::AccountEntry) -> bool {
    acc.is_pda()
        && acc.seeds.iter().all(|s| {
            matches!(
                crate::classify_seed(s),
                crate::SeedPart::Literal(_) | crate::SeedPart::Account(_)
            )
        })
}

/// Write a human-readable summary of an account's classified PDA seeds, e.g.
/// `vault: literal "vault", account authority`. The dependency-free generators
/// (Go / Python / C, which carry no PDA-derivation crypto) emit this as a
/// comment so a host using a real Solana SDK knows exactly what to derive.
pub(crate) fn write_seed_plan(
    f: &mut fmt::Formatter<'_>,
    acc: &crate::AccountEntry,
) -> fmt::Result {
    write!(f, "{}:", acc.name)?;
    for (i, seed) in acc.seeds.iter().enumerate() {
        write!(f, "{}", if i == 0 { " " } else { ", " })?;
        match crate::classify_seed(seed) {
            crate::SeedPart::Literal(text) => write!(f, "literal \"{}\"", text)?,
            crate::SeedPart::Account(name) => write!(f, "account {}", name)?,
            crate::SeedPart::Arg(name) => write!(f, "arg {}", name)?,
            crate::SeedPart::Unknown(expr) => write!(f, "expr {}", expr)?,
        }
    }
    Ok(())
}

fn write_camel(f: &mut fmt::Formatter<'_>, name: &str) -> fmt::Result {
    let mut capitalize_next = false;
    let mut first = true;
    for c in name.chars() {
        if c == '_' || c == '-' {
            capitalize_next = true;
        } else if capitalize_next {
            for uc in c.to_uppercase() {
                write!(f, "{}", uc)?;
            }
            capitalize_next = false;
        } else if first {
            for lc in c.to_lowercase() {
                write!(f, "{}", lc)?;
            }
            first = false;
        } else {
            write!(f, "{}", c)?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// TypeScript Accounts
// ---------------------------------------------------------------------------

/// Generates `accounts.ts` content from a `ProgramManifest`.
pub struct TsAccounts<'a>(pub &'a ProgramManifest);

impl<'a> fmt::Display for TsAccounts<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let prog = self.0;

        writeln!(f, "// Auto-generated by hopper client gen --ts")?;
        writeln!(f, "// Program: {} v{}", prog.name, prog.version)?;
        writeln!(f, "// DO NOT EDIT")?;
        writeln!(f)?;
        writeln!(f, "import {{ PublicKey }} from \"@solana/web3.js\";")?;
        writeln!(f)?;

        // Header/compact account constants
        writeln!(f, "/** Hopper account header size in bytes. */")?;
        writeln!(f, "export const HEADER_SIZE = {};", HOPPER_HEADER_SIZE)?;
        writeln!(
            f,
            "/** Compact account body starts immediately after the discriminator byte. */"
        )?;
        writeln!(f, "export const COMPACT_BODY_OFFSET = 1;")?;
        writeln!(f)?;
        // Offset of the 8-byte LAYOUT_ID fingerprint within the
        // Hopper header. Clients read bytes [4, 12) to assert the
        // account matches the expected layout. See audit ST2 for the
        // client-side ABI guard.
        writeln!(
            f,
            "/** Byte offset of the 8-byte layout fingerprint in a Hopper account header. */"
        )?;
        writeln!(f, "export const LAYOUT_ID_OFFSET = 4;")?;
        writeln!(f, "/** Byte length of the layout fingerprint. */")?;
        writeln!(f, "export const LAYOUT_ID_LENGTH = 8;")?;
        writeln!(f)?;
        writeln!(
            f,
            "export type AccountEncoding = \"headered\" | \"compact\";"
        )?;
        writeln!(f)?;
        writeln!(f, "export interface LayoutIdentity {{")?;
        writeln!(f, "  name: string;")?;
        writeln!(f, "  encoding: AccountEncoding;")?;
        writeln!(f, "  disc: number;")?;
        writeln!(f, "  size: number;")?;
        writeln!(f, "  layoutId: string;")?;
        writeln!(f, "}}")?;
        writeln!(f)?;
        writeln!(
            f,
            "export function layoutFingerprint(layout: LayoutIdentity): string {{"
        )?;
        writeln!(f, "  return layout.layoutId;")?;
        writeln!(f, "}}")?;
        writeln!(f)?;
        // Generic layout-fingerprint verifier. Used by every
        // per-layout `assertXxxLayout(data)` generated below.
        writeln!(f, "/**")?;
        writeln!(
            f,
            " * Raise if `data` is not a Hopper account encoding the expected layout."
        )?;
        writeln!(f, " *")?;
        writeln!(
            f,
            " * Reads the 8-byte LAYOUT_ID fingerprint from the 16-byte Hopper header"
        )?;
        writeln!(
            f,
            " * (bytes 4..12) and compares it against `expectedHex` (16 lowercase hex chars)."
        )?;
        writeln!(
            f,
            " * This is the client-side complement to the runtime check `load::<T>()` runs"
        )?;
        writeln!(
            f,
            " * before handing out a typed Ref. Mismatch means the on-chain program was"
        )?;
        writeln!(
            f,
            " * upgraded with a different ABI than the client was generated against."
        )?;
        writeln!(f, " */")?;
        writeln!(
            f,
            "export function assertLayoutId(data: Uint8Array, expectedHex: string): void {{"
        )?;
        writeln!(f, "  if (data.length < HEADER_SIZE) {{")?;
        writeln!(
            f,
            "    throw new Error(`Hopper account too short: ${{data.length}} < ${{HEADER_SIZE}}`);"
        )?;
        writeln!(f, "  }}")?;
        writeln!(f, "  let actualHex = \"\";")?;
        writeln!(f, "  for (let i = 0; i < LAYOUT_ID_LENGTH; i++) {{")?;
        writeln!(
            f,
            "    actualHex += data[LAYOUT_ID_OFFSET + i].toString(16).padStart(2, \"0\");"
        )?;
        writeln!(f, "  }}")?;
        writeln!(f, "  if (actualHex !== expectedHex.toLowerCase()) {{")?;
        writeln!(f, "    throw new Error(")?;
        writeln!(f, "      `Hopper layout mismatch: account header reports ${{actualHex}}, expected ${{expectedHex}}`,")?;
        writeln!(f, "    );")?;
        writeln!(f, "  }}")?;
        writeln!(f, "}}")?;
        writeln!(f)?;

        writeln!(f, "/**")?;
        writeln!(
            f,
            " * Raise if `data` is not a compact Hopper account for `layout`."
        )?;
        writeln!(f, " *")?;
        writeln!(
            f,
            " * Compact accounts do not store the 8-byte LAYOUT_ID in account data;"
        )?;
        writeln!(
            f,
            " * the fingerprint is carried by the manifest/IDL and exposed through"
        )?;
        writeln!(
            f,
            " * `layoutFingerprint(layout)` while the hot-path bytes prove disc + exact size."
        )?;
        writeln!(f, " */")?;
        writeln!(f, "export function assertCompactLayout(data: Uint8Array, layout: LayoutIdentity): void {{")?;
        writeln!(f, "  if (data.length !== layout.size) {{")?;
        writeln!(f, "    throw new Error(`Hopper compact account size mismatch for ${{layout.name}}: ${{data.length}} !== ${{layout.size}}`);")?;
        writeln!(f, "  }}")?;
        writeln!(f, "  if (data[0] !== layout.disc) {{")?;
        writeln!(f, "    throw new Error(`Hopper compact discriminator mismatch for ${{layout.name}}: ${{data[0]}} !== ${{layout.disc}}`);")?;
        writeln!(f, "  }}")?;
        writeln!(f, "}}")?;
        writeln!(f)?;

        for layout in prog.layouts.iter() {
            // Interface
            write!(f, "export interface ")?;
            write_pascal(f, layout.name)?;
            writeln!(f, " {{")?;
            for field in layout.fields.iter() {
                write!(f, "  ")?;
                write_camel(f, field.name)?;
                writeln!(f, ": {};", ts_type(field.canonical_type))?;
            }
            writeln!(f, "}}")?;
            writeln!(f)?;

            // Layout fingerprint constant (hex). Pairs with the
            // runtime's `T::LAYOUT_ID` so the client and the program
            // agree on the ABI byte-for-byte.
            write!(f, "export const ")?;
            write_upper_snake(f, layout.name)?;
            write!(f, "_LAYOUT_ID = \"")?;
            for b in layout.layout_id.iter() {
                write!(f, "{:02x}", b)?;
            }
            writeln!(f, "\";")?;
            writeln!(f)?;

            write!(f, "export const ")?;
            write_upper_snake(f, layout.name)?;
            writeln!(f, "_SIZE = {};", layout.total_size)?;
            write!(f, "export const ")?;
            write_upper_snake(f, layout.name)?;
            writeln!(
                f,
                "_ACCOUNT_ENCODING: AccountEncoding = \"{}\";",
                if layout_is_compact(layout) {
                    "compact"
                } else {
                    "headered"
                }
            )?;
            writeln!(f)?;

            // Discriminator constant
            write!(f, "export const ")?;
            write_upper_snake(f, layout.name)?;
            writeln!(f, "_DISC = {};", layout.disc)?;
            writeln!(f)?;

            write!(f, "export const ")?;
            write_upper_snake(f, layout.name)?;
            writeln!(f, "_LAYOUT: LayoutIdentity = {{")?;
            writeln!(f, "  name: \"{}\",", layout.name)?;
            write!(f, "  encoding: ")?;
            write_upper_snake(f, layout.name)?;
            writeln!(f, "_ACCOUNT_ENCODING,")?;
            write!(f, "  disc: ")?;
            write_upper_snake(f, layout.name)?;
            writeln!(f, "_DISC,")?;
            write!(f, "  size: ")?;
            write_upper_snake(f, layout.name)?;
            writeln!(f, "_SIZE,")?;
            write!(f, "  layoutId: ")?;
            write_upper_snake(f, layout.name)?;
            writeln!(f, "_LAYOUT_ID,")?;
            writeln!(f, "}};")?;
            writeln!(f)?;

            // Per-layout assertion helper. Thin wrapper over
            // the headered or compact assertion for convenience at call sites.
            write!(f, "export function assert")?;
            write_pascal(f, layout.name)?;
            writeln!(f, "Layout(data: Uint8Array): void {{")?;
            if layout_is_compact(layout) {
                write!(f, "  assertCompactLayout(data, ")?;
                write_upper_snake(f, layout.name)?;
                writeln!(f, "_LAYOUT);")?;
            } else {
                write!(f, "  assertLayoutId(data, ")?;
                write_upper_snake(f, layout.name)?;
                writeln!(f, "_LAYOUT_ID);")?;
            }
            writeln!(f, "}}")?;
            writeln!(f)?;

            // Decoder
            write!(f, "export function decode")?;
            write_pascal(f, layout.name)?;
            writeln!(f, "(data: Uint8Array): ")?;
            write!(f, "  ")?;
            write_pascal(f, layout.name)?;
            writeln!(f, " {{")?;
            write!(f, "  assert")?;
            write_pascal(f, layout.name)?;
            writeln!(f, "Layout(data);")?;
            writeln!(f, "  if (data.length < {}) {{", layout.total_size)?;
            writeln!(
                f,
                "    throw new Error(`Data too small for {}: ${{data.length}} < {}`);",
                layout.name, layout.total_size
            )?;
            writeln!(f, "  }}")?;
            writeln!(
                f,
                "  const view = new DataView(data.buffer, data.byteOffset, data.byteLength);"
            )?;

            for field in layout.fields.iter() {
                let offset = field.offset as usize;
                let end = offset + field.size as usize;
                write!(f, "  const ")?;
                write_camel(f, field.name)?;
                write!(f, " = ")?;
                write_decode_expr(f, field.canonical_type, offset, end)?;
                writeln!(f, ";")?;
            }

            writeln!(f, "  return {{")?;
            for field in layout.fields.iter() {
                write!(f, "    ")?;
                write_camel(f, field.name)?;
                writeln!(f, ",")?;
            }
            writeln!(f, "  }};")?;
            writeln!(f, "}}")?;
            writeln!(f)?;
        }

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// TypeScript Instructions
// ---------------------------------------------------------------------------

/// Generates `instructions.ts` content from a `ProgramManifest`.
pub struct TsInstructions<'a>(pub &'a ProgramManifest);

impl<'a> fmt::Display for TsInstructions<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let prog = self.0;

        writeln!(f, "// Auto-generated by hopper client gen --ts")?;
        writeln!(f, "// Program: {} v{}", prog.name, prog.version)?;
        writeln!(f, "// DO NOT EDIT")?;
        writeln!(f)?;
        writeln!(
            f,
            "import {{ AccountMeta, PublicKey, TransactionInstruction }} from \"@solana/web3.js\";"
        )?;
        writeln!(f)?;

        for ix in prog.instructions.iter() {
            // Args interface
            if !ix.args.is_empty() {
                write!(f, "export interface ")?;
                write_pascal(f, ix.name)?;
                writeln!(f, "Args {{")?;
                for arg in ix.args.iter() {
                    write!(f, "  ")?;
                    write_camel(f, arg.name)?;
                    write!(f, ": ")?;
                    write_ts_arg_type(f, arg)?;
                    writeln!(f, ";")?;
                }
                writeln!(f, "}}")?;
                writeln!(f)?;
            }

            // An account is auto-resolved as a PDA only when *every* seed is a
            // byte literal or another account reference. Any arg/unknown seed
            // falls back to caller-provided so the client never derives a wrong
            // address from a seed it cannot encode.
            let is_auto_pda = |acc: &crate::AccountEntry| account_is_auto_pda(acc);

            // Accounts interface: only caller-provided accounts. PDAs are
            // derived in the builder, so callers never pass (or mis-derive) them.
            write!(f, "export interface ")?;
            write_pascal(f, ix.name)?;
            writeln!(f, "Accounts {{")?;
            for acc in ix.accounts.iter() {
                if is_auto_pda(acc) {
                    continue;
                }
                write!(f, "  ")?;
                write_camel(f, acc.name)?;
                writeln!(f, ": PublicKey;")?;
            }
            writeln!(f, "}}")?;
            writeln!(f)?;

            // Builder function
            write!(f, "export function create")?;
            write_pascal(f, ix.name)?;
            writeln!(f, "Instruction(")?;
            if !ix.args.is_empty() {
                write!(f, "  args: ")?;
                write_pascal(f, ix.name)?;
                writeln!(f, "Args,")?;
            }
            write!(f, "  accounts: ")?;
            write_pascal(f, ix.name)?;
            writeln!(f, "Accounts,")?;
            writeln!(f, "  programId: PublicKey,")?;
            if ix.remaining_accounts.is_some() {
                writeln!(f, "  remainingAccounts: readonly AccountMeta[] = [],")?;
            }
            writeln!(f, "): TransactionInstruction {{")?;

            if let Some(remaining) = ix.remaining_accounts {
                writeln!(
                    f,
                    "  if (remainingAccounts.length > {}) throw new Error(\"{} accepts at most {} remaining accounts\");",
                    remaining.max,
                    ix.name,
                    remaining.max,
                )?;
            }

            // Resolve program-derived addresses from their on-chain seeds, so
            // the caller supplies only the accounts it actually owns.
            if ix.accounts.iter().any(&is_auto_pda) {
                writeln!(
                    f,
                    "  // Program-derived addresses, resolved from on-chain seeds."
                )?;
                for acc in ix.accounts.iter() {
                    if !is_auto_pda(acc) {
                        continue;
                    }
                    write!(f, "  const [")?;
                    write_camel(f, acc.name)?;
                    writeln!(f, "] = PublicKey.findProgramAddressSync(")?;
                    write!(f, "    [")?;
                    for (i, seed) in acc.seeds.iter().enumerate() {
                        if i > 0 {
                            write!(f, ", ")?;
                        }
                        match crate::classify_seed(seed) {
                            crate::SeedPart::Literal(text) => {
                                write!(f, "Buffer.from(")?;
                                write_ts_string(f, text)?;
                                write!(f, ")")?;
                            }
                            crate::SeedPart::Account(name) => {
                                // Reference an already-derived PDA local when the
                                // seed points at another PDA; otherwise the
                                // caller-provided account.
                                let refs_pda =
                                    ix.accounts.iter().any(|a| a.name == name && is_auto_pda(a));
                                if !refs_pda {
                                    write!(f, "accounts.")?;
                                }
                                write_camel(f, name)?;
                                write!(f, ".toBuffer()")?;
                            }
                            crate::SeedPart::Arg(_) | crate::SeedPart::Unknown(_) => {
                                // Unreachable: is_auto_pda rejects these.
                            }
                        }
                    }
                    writeln!(f, "],")?;
                    writeln!(f, "    programId,")?;
                    writeln!(f, "  );")?;
                }
                writeln!(f)?;
            }

            // Build instruction data
            if ix.args.iter().any(|arg| arg.fixed_size().is_none()) {
                for arg in ix.args.iter() {
                    match arg.encoding {
                        ArgEncoding::BoundedVec {
                            max_len,
                            element_size,
                        } => {
                            write!(f, "  if (args.")?;
                            write_camel(f, arg.name)?;
                            writeln!(f, ".length > {}) throw new Error(\"{} exceeds its bounded capacity of {}\");", max_len, arg.name, max_len)?;
                            if bounded_vec_element_type(arg.canonical_type).unwrap_or("u8") != "u8"
                            {
                                write!(f, "  for (const value of args.")?;
                                write_camel(f, arg.name)?;
                                writeln!(f, ") {{")?;
                                let element =
                                    bounded_vec_element_type(arg.canonical_type).unwrap_or("");
                                if !matches!(
                                    element,
                                    "u16"
                                        | "i16"
                                        | "u32"
                                        | "i32"
                                        | "u64"
                                        | "i64"
                                        | "u128"
                                        | "i128"
                                        | "bool"
                                        | "Pubkey"
                                ) {
                                    writeln!(f, "    if (value.length !== {}) throw new Error(\"{} element has the wrong encoded size\");", element_size, arg.name)?;
                                }
                                writeln!(f, "  }}")?;
                            }
                        }
                        ArgEncoding::BoundedString { max_len } => {
                            write!(f, "  const ")?;
                            write_camel(f, arg.name)?;
                            write!(f, "Bytes = new TextEncoder().encode(args.")?;
                            write_camel(f, arg.name)?;
                            writeln!(f, ");")?;
                            write!(f, "  if (")?;
                            write_camel(f, arg.name)?;
                            writeln!(f, "Bytes.length > {}) throw new Error(\"{} exceeds its bounded UTF-8 capacity of {} bytes\");", max_len, arg.name, max_len)?;
                        }
                        ArgEncoding::Fixed => {}
                    }
                }
                write!(f, "  const data = new Uint8Array(1")?;
                for arg in ix.args.iter() {
                    match arg.encoding {
                        ArgEncoding::Fixed => write!(f, " + {}", arg.size)?,
                        ArgEncoding::BoundedVec { element_size, .. } => {
                            write!(f, " + 2 + args.")?;
                            write_camel(f, arg.name)?;
                            write!(f, ".length * {}", element_size)?;
                        }
                        ArgEncoding::BoundedString { .. } => {
                            write!(f, " + 2 + ")?;
                            write_camel(f, arg.name)?;
                            write!(f, "Bytes.length")?;
                        }
                    }
                }
                writeln!(f, ");")?;
                writeln!(f, "  const view = new DataView(data.buffer);")?;
                writeln!(f, "  data[0] = {}; // instruction discriminator", ix.tag)?;
                writeln!(f, "  let offset = 1;")?;
                for arg in ix.args.iter() {
                    write_encode_dynamic_expr(f, arg)?;
                }
            } else {
                let data_size = instruction_data_size(ix);
                writeln!(f, "  const data = new Uint8Array({});", data_size)?;
                writeln!(f, "  const view = new DataView(data.buffer);")?;
                writeln!(f, "  data[0] = {}; // instruction discriminator", ix.tag)?;

                let mut offset = 1usize; // first byte is tag
                for arg in ix.args.iter() {
                    write_encode_expr(f, arg.canonical_type, arg.name, offset)?;
                    offset += arg.size as usize;
                }
            }

            writeln!(f)?;

            // Build keys array. PDA accounts use their derived local; provided
            // accounts come from the `accounts` argument.
            writeln!(f, "  const keys = [")?;
            for (idx, acc) in ix.accounts.iter().enumerate() {
                if is_auto_pda(acc) {
                    write!(f, "    {{ pubkey: ")?;
                    write_camel(f, acc.name)?;
                } else {
                    write!(f, "    {{ pubkey: accounts.")?;
                    write_camel(f, acc.name)?;
                }
                // `effective_writable` (BLD-MUT contract): passthrough
                // unless the instruction is `mutation_complete`
                // (strict_writes + declared lamport dimension); when
                // complete, an account with neither a data range nor
                // lamport permission is provably untouched and is demoted
                // to read-only. Never promotes.
                writeln!(
                    f,
                    ", isSigner: {}, isWritable: {} }},",
                    acc.signer,
                    ix.effective_writable(idx, acc.writable)
                )?;
            }
            writeln!(f, "  ];")?;
            if ix.remaining_accounts.is_some() {
                writeln!(f, "  keys.push(...remainingAccounts);")?;
            }
            writeln!(f)?;
            writeln!(
                f,
                "  return new TransactionInstruction({{ keys, programId, data }});"
            )?;
            writeln!(f, "}}")?;
            writeln!(f)?;

            // Compute-budget companion (BLD-CU). Only generated when the
            // descriptor publishes a measured CU upper bound; an unknown
            // estimate (0) emits nothing, so unmeasured programs keep the
            // exact pre-BLD-CU output. The requested limit is the published
            // bound plus a 10% margin (clamped to the 1.4M runtime cap) —
            // the margin only ever raises the limit, never shrinks it, so an
            // honest estimate cannot fail the transaction.
            if let Some(budget) = ix.cu_budget_with_margin() {
                write!(f, "/** Measured CU upper bound for `")?;
                write_camel(f, ix.name)?;
                writeln!(f, "` (author-published, conservative). */")?;
                write!(f, "export const ")?;
                write_upper_snake(f, ix.name)?;
                writeln!(f, "_CU_ESTIMATE = {};", ix.cu_estimate)?;
                writeln!(f)?;
                write!(f, "/**\n * Like `create")?;
                write_pascal(f, ix.name)?;
                writeln!(f, "Instruction`, but prepends a")?;
                writeln!(
                    f,
                    " * `SetComputeUnitLimit({})` (estimate + 10% margin) so the",
                    budget
                )?;
                writeln!(
                    f,
                    " * transaction requests only the CU it was measured to need."
                )?;
                writeln!(f, " */")?;
                write!(f, "export function create")?;
                write_pascal(f, ix.name)?;
                writeln!(f, "InstructionWithBudget(")?;
                if !ix.args.is_empty() {
                    write!(f, "  args: ")?;
                    write_pascal(f, ix.name)?;
                    writeln!(f, "Args,")?;
                }
                write!(f, "  accounts: ")?;
                write_pascal(f, ix.name)?;
                writeln!(f, "Accounts,")?;
                writeln!(f, "  programId: PublicKey,")?;
                writeln!(f, "): TransactionInstruction[] {{")?;
                writeln!(f, "  const budgetData = new Uint8Array(5);")?;
                writeln!(f, "  budgetData[0] = 2; // SetComputeUnitLimit")?;
                writeln!(
                    f,
                    "  new DataView(budgetData.buffer).setUint32(1, {}, true);",
                    budget
                )?;
                writeln!(f, "  const budgetIx = new TransactionInstruction({{")?;
                writeln!(f, "    keys: [],")?;
                writeln!(
                    f,
                    "    programId: new PublicKey(\"ComputeBudget111111111111111111111111111111\"),"
                )?;
                writeln!(f, "    data: budgetData,")?;
                writeln!(f, "  }});")?;
                write!(f, "  return [budgetIx, create")?;
                write_pascal(f, ix.name)?;
                if ix.args.is_empty() {
                    writeln!(f, "Instruction(accounts, programId)];")?;
                } else {
                    writeln!(f, "Instruction(args, accounts, programId)];")?;
                }
                writeln!(f, "}}")?;
                writeln!(f)?;
            }
        }

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// TypeScript Events
// ---------------------------------------------------------------------------

/// Generates `events.ts` content from a `ProgramManifest`.
pub struct TsEvents<'a>(pub &'a ProgramManifest);

impl<'a> fmt::Display for TsEvents<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let prog = self.0;

        writeln!(f, "// Auto-generated by hopper client gen --ts")?;
        writeln!(f, "// Program: {} v{}", prog.name, prog.version)?;
        writeln!(f, "// DO NOT EDIT")?;
        writeln!(f)?;
        writeln!(f, "import {{ PublicKey }} from \"@solana/web3.js\";")?;
        writeln!(f)?;

        write_ts_touch_map_decoder(f)?;

        if prog.events.is_empty() {
            writeln!(f, "// No events defined for this program.")?;
            return Ok(());
        }

        for event in prog.events.iter() {
            // Interface
            write!(f, "export interface ")?;
            write_pascal(f, event.name)?;
            writeln!(f, "Event {{")?;
            for field in event.fields.iter() {
                write!(f, "  ")?;
                write_camel(f, field.name)?;
                writeln!(f, ": {};", ts_type(field.canonical_type))?;
            }
            writeln!(f, "}}")?;
            writeln!(f)?;

            // Discriminator constant
            write!(f, "export const ")?;
            write_upper_snake(f, event.name)?;
            writeln!(f, "_EVENT_DISC = {};", event.tag)?;
            write!(f, "export const ")?;
            write_upper_snake(f, event.name)?;
            writeln!(f, "_EVENT_DATA_LEN = {};", event_data_size(event))?;
            writeln!(f)?;

            // Decoder
            write!(f, "export function decode")?;
            write_pascal(f, event.name)?;
            writeln!(f, "Event(data: Uint8Array): ")?;
            write!(f, "  ")?;
            write_pascal(f, event.name)?;
            writeln!(f, "Event {{")?;
            write!(f, "  if (data.length < ")?;
            write_upper_snake(f, event.name)?;
            writeln!(f, "_EVENT_DATA_LEN) {{")?;
            write!(
                f,
                "    throw new Error(`Hopper event too short: ${{data.length}} < ${{"
            )?;
            write_upper_snake(f, event.name)?;
            writeln!(f, "_EVENT_DATA_LEN}}`);")?;
            writeln!(f, "  }}")?;
            write!(f, "  if (data[0] !== ")?;
            write_upper_snake(f, event.name)?;
            writeln!(f, "_EVENT_DISC) {{")?;
            write!(
                f,
                "    throw new Error(`Hopper event tag mismatch: ${{data[0]}} !== ${{"
            )?;
            write_upper_snake(f, event.name)?;
            writeln!(f, "_EVENT_DISC}}`);")?;
            writeln!(f, "  }}")?;
            writeln!(
                f,
                "  const view = new DataView(data.buffer, data.byteOffset, data.byteLength);"
            )?;

            for field in event.fields.iter() {
                let offset = field.offset as usize + 1;
                let end = offset + field.size as usize;
                write!(f, "  const ")?;
                write_camel(f, field.name)?;
                write!(f, " = ")?;
                write_decode_expr(f, field.canonical_type, offset, end)?;
                writeln!(f, ";")?;
            }

            writeln!(f, "  return {{")?;
            for field in event.fields.iter() {
                write!(f, "    ")?;
                write_camel(f, field.name)?;
                writeln!(f, ",")?;
            }
            writeln!(f, "  }};")?;
            writeln!(f, "}}")?;
            writeln!(f)?;
        }

        Ok(())
    }
}

/// Emit the Hopper touch-map decoder into `events.ts`.
///
/// Touch maps are the runtime's self-describing state-effect records
/// (`touch-map` feature): one `sol_log_data` payload per instruction,
/// wire format v1 as documented in `hopper-runtime/src/segment_borrow.rs`
/// (magic `0x7A`, version `0x01`, flags, count, then 9-byte records).
/// The generated helper is a pure function so indexers can decode
/// `Program data:` payloads without the Hopper CLI; it returns `null`
/// on wrong magic/version or a length that violates `4 + 9 * count`.
fn write_ts_touch_map_decoder(f: &mut fmt::Formatter<'_>) -> fmt::Result {
    writeln!(
        f,
        "/** One touched byte range from a Hopper touch map (wire format v1). */"
    )?;
    writeln!(f, "export interface HopperTouchRecord {{")?;
    writeln!(
        f,
        "  /** Account index into the emitting instruction's account list. */"
    )?;
    writeln!(f, "  slot: number;")?;
    writeln!(f, "  /** Byte offset within the account data. */")?;
    writeln!(f, "  offset: number;")?;
    writeln!(f, "  /** Byte length of the touched range. */")?;
    writeln!(f, "  size: number;")?;
    writeln!(f, "  /** True for a write, false for a read. */")?;
    writeln!(f, "  write: boolean;")?;
    writeln!(f, "}}")?;
    writeln!(f)?;
    writeln!(
        f,
        "/** A decoded Hopper touch map: an instruction's self-described state effects. */"
    )?;
    writeln!(f, "export interface HopperTouchMap {{")?;
    writeln!(f, "  version: number;")?;
    writeln!(
        f,
        "  /** True when the on-chain log overflowed: the map is PARTIAL. */"
    )?;
    writeln!(f, "  overflowed: boolean;")?;
    writeln!(
        f,
        "  /** True when the encoder skipped a range it could not represent. */"
    )?;
    writeln!(f, "  recordsSkipped: boolean;")?;
    writeln!(f, "  records: HopperTouchRecord[];")?;
    writeln!(f, "}}")?;
    writeln!(f)?;
    writeln!(f, "export const HOPPER_TOUCH_MAP_MAGIC = 0x7a;")?;
    writeln!(f, "export const HOPPER_TOUCH_MAP_VERSION = 0x01;")?;
    writeln!(f)?;
    writeln!(f, "/**")?;
    writeln!(
        f,
        " * Decode a Hopper touch map from a `Program data:` payload."
    )?;
    writeln!(f, " *")?;
    writeln!(
        f,
        " * Pure function; returns null unless the magic, version, and exact-length"
    )?;
    writeln!(
        f,
        " * equation (length === 4 + 9 * count) all hold, so other log payloads"
    )?;
    writeln!(
        f,
        " * (Anchor events, receipts) are never misread as touch maps."
    )?;
    writeln!(f, " */")?;
    writeln!(
        f,
        "export function decodeHopperTouchMap(data: Uint8Array): HopperTouchMap | null {{"
    )?;
    writeln!(f, "  if (data.length < 4) return null;")?;
    writeln!(
        f,
        "  if (data[0] !== HOPPER_TOUCH_MAP_MAGIC || data[1] !== HOPPER_TOUCH_MAP_VERSION) return null;"
    )?;
    writeln!(f, "  const flags = data[2];")?;
    writeln!(f, "  const count = data[3];")?;
    writeln!(f, "  if (data.length !== 4 + count * 9) return null;")?;
    writeln!(
        f,
        "  const view = new DataView(data.buffer, data.byteOffset, data.byteLength);"
    )?;
    writeln!(f, "  const records: HopperTouchRecord[] = [];")?;
    writeln!(f, "  for (let i = 0; i < count; i++) {{")?;
    writeln!(f, "    const base = 4 + i * 9;")?;
    writeln!(f, "    const packed = view.getUint32(base + 1, true);")?;
    writeln!(f, "    records.push({{")?;
    writeln!(f, "      slot: data[base],")?;
    writeln!(f, "      offset: packed & 0x7fffffff,")?;
    writeln!(f, "      size: view.getUint32(base + 5, true),")?;
    writeln!(f, "      write: (packed & 0x80000000) !== 0,")?;
    writeln!(f, "    }});")?;
    writeln!(f, "  }}")?;
    writeln!(f, "  return {{")?;
    writeln!(f, "    version: data[1],")?;
    writeln!(f, "    overflowed: (flags & 0x01) !== 0,")?;
    writeln!(f, "    recordsSkipped: (flags & 0x02) !== 0,")?;
    writeln!(f, "    records,")?;
    writeln!(f, "  }};")?;
    writeln!(f, "}}")?;
    writeln!(f)
}

// ---------------------------------------------------------------------------
// TypeScript Types
// ---------------------------------------------------------------------------

/// Generates `types.ts` content (shared type aliases).
pub struct TsTypes<'a>(pub &'a ProgramManifest);

impl<'a> fmt::Display for TsTypes<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let prog = self.0;

        writeln!(f, "// Auto-generated by hopper client gen --ts")?;
        writeln!(f, "// Program: {} v{}", prog.name, prog.version)?;
        writeln!(f, "// DO NOT EDIT")?;
        writeln!(f)?;
        writeln!(f, "import {{ PublicKey }} from \"@solana/web3.js\";")?;
        writeln!(f)?;
        writeln!(f, "/** Hopper account header (16 bytes). */")?;
        writeln!(f, "export interface HopperHeader {{")?;
        writeln!(f, "  disc: number;")?;
        writeln!(f, "  version: number;")?;
        writeln!(f, "  flags: number;")?;
        writeln!(f, "  layoutId: Uint8Array;")?;
        writeln!(f, "  schemaEpoch: number;")?;
        writeln!(f, "}}")?;
        writeln!(f)?;
        writeln!(f, "const HEADER_SIZE = 16;")?;
        writeln!(f, "const LAYOUT_ID_OFFSET = 4;")?;
        writeln!(f, "const LAYOUT_ID_LENGTH = 8;")?;
        writeln!(f)?;
        writeln!(f, "/** Decode the Hopper 16-byte account header. */")?;
        writeln!(
            f,
            "export function decodeHeader(data: Uint8Array): HopperHeader {{"
        )?;
        writeln!(f, "  if (data.length < HEADER_SIZE) {{")?;
        writeln!(
            f,
            "    throw new Error(`Hopper account too short: ${{data.length}} < ${{HEADER_SIZE}}`);"
        )?;
        writeln!(f, "  }}")?;
        writeln!(
            f,
            "  const view = new DataView(data.buffer, data.byteOffset, data.byteLength);"
        )?;
        writeln!(f, "  return {{")?;
        writeln!(f, "    disc: data[0],")?;
        writeln!(f, "    version: data[1],")?;
        writeln!(f, "    flags: view.getUint16(2, true),")?;
        writeln!(
            f,
            "    layoutId: data.slice(LAYOUT_ID_OFFSET, LAYOUT_ID_OFFSET + LAYOUT_ID_LENGTH),"
        )?;
        writeln!(f, "    schemaEpoch: view.getUint32(12, true),")?;
        writeln!(f, "  }};")?;
        writeln!(f, "}}")?;
        writeln!(f)?;
        writeln!(
            f,
            "/** All account discriminators for {} v{}. */",
            prog.name, prog.version
        )?;
        writeln!(f, "export const Discriminators = {{")?;
        for layout in prog.layouts.iter() {
            write!(f, "  ")?;
            write_pascal(f, layout.name)?;
            writeln!(f, ": {},", layout.disc)?;
        }
        writeln!(f, "}} as const;")?;

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// TypeScript Index
// ---------------------------------------------------------------------------

/// Generates `index.ts` content (re-exports).
pub struct TsIndex<'a>(pub &'a ProgramManifest);

impl<'a> fmt::Display for TsIndex<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let prog = self.0;

        writeln!(f, "// Auto-generated by hopper client gen --ts")?;
        writeln!(f, "// Program: {} v{}", prog.name, prog.version)?;
        writeln!(f, "// DO NOT EDIT")?;
        writeln!(f)?;
        writeln!(f, "export * from \"./types\";")?;
        writeln!(f, "export * from \"./accounts\";")?;
        writeln!(f, "export * from \"./instructions\";")?;
        writeln!(f, "export * from \"./events\";")?;

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Full Client Generator
// ---------------------------------------------------------------------------

/// Generates all TypeScript client files for a program and writes them
/// using newline separators with file markers.
///
/// Output format:
/// ```text
/// === accounts.ts ===
/// <contents>
/// === instructions.ts ===
/// <contents>
/// === events.ts ===
/// <contents>
/// === types.ts ===
/// <contents>
/// === index.ts ===
/// <contents>
/// ```
pub struct TsClientGen<'a>(pub &'a ProgramManifest);

impl<'a> fmt::Display for TsClientGen<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let prog = self.0;

        writeln!(f, "=== types.ts ===")?;
        write!(f, "{}", TsTypes(prog))?;
        writeln!(f)?;

        writeln!(f, "=== accounts.ts ===")?;
        write!(f, "{}", TsAccounts(prog))?;
        writeln!(f)?;

        writeln!(f, "=== instructions.ts ===")?;
        write!(f, "{}", TsInstructions(prog))?;
        writeln!(f)?;

        writeln!(f, "=== events.ts ===")?;
        write!(f, "{}", TsEvents(prog))?;
        writeln!(f)?;

        writeln!(f, "=== index.ts ===")?;
        write!(f, "{}", TsIndex(prog))?;

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn write_upper_snake(f: &mut fmt::Formatter<'_>, name: &str) -> fmt::Result {
    for c in name.chars() {
        if c == '-' || c == ' ' {
            write!(f, "_")?;
        } else {
            for uc in c.to_uppercase() {
                write!(f, "{}", uc)?;
            }
        }
    }
    Ok(())
}

fn write_decode_expr(
    f: &mut fmt::Formatter<'_>,
    canonical: &str,
    offset: usize,
    end: usize,
) -> fmt::Result {
    match canonical {
        "u8" => write!(f, "data[{}]", offset),
        "i8" => write!(f, "view.getInt8({})", offset),
        "u16" => write!(f, "view.getUint16({}, true)", offset),
        "i16" => write!(f, "view.getInt16({}, true)", offset),
        "u32" => write!(f, "view.getUint32({}, true)", offset),
        "i32" => write!(f, "view.getInt32({}, true)", offset),
        "u64" => write!(f, "view.getBigUint64({}, true)", offset),
        "i64" => write!(f, "view.getBigInt64({}, true)", offset),
        "u128" => {
            write!(
                f,
                "view.getBigUint64({}, true) | (view.getBigUint64({}, true) << 64n)",
                offset,
                offset + 8
            )
        }
        "i128" => {
            write!(
                f,
                "view.getBigInt64({}, true) | (view.getBigUint64({}, true) << 64n)",
                offset + 8,
                offset
            )
        }
        "bool" => write!(f, "data[{}] !== 0", offset),
        "Pubkey" => write!(f, "new PublicKey(data.slice({}, {}))", offset, end),
        _ => write!(f, "data.slice({}, {})", offset, end),
    }
}

fn write_encode_expr(
    f: &mut fmt::Formatter<'_>,
    canonical: &str,
    name: &str,
    offset: usize,
) -> fmt::Result {
    match canonical {
        "u8" => {
            write!(f, "  data[{}] = args.", offset)?;
            write_camel(f, name)?;
            writeln!(f, ";")
        }
        "i8" => {
            write!(f, "  view.setInt8({}, args.", offset)?;
            write_camel(f, name)?;
            writeln!(f, ");")
        }
        "u16" => {
            write!(f, "  view.setUint16({}, args.", offset)?;
            write_camel(f, name)?;
            writeln!(f, ", true);")
        }
        "i16" => {
            write!(f, "  view.setInt16({}, args.", offset)?;
            write_camel(f, name)?;
            writeln!(f, ", true);")
        }
        "u32" => {
            write!(f, "  view.setUint32({}, args.", offset)?;
            write_camel(f, name)?;
            writeln!(f, ", true);")
        }
        "i32" => {
            write!(f, "  view.setInt32({}, args.", offset)?;
            write_camel(f, name)?;
            writeln!(f, ", true);")
        }
        "u64" => {
            write!(f, "  view.setBigUint64({}, args.", offset)?;
            write_camel(f, name)?;
            writeln!(f, ", true);")
        }
        "i64" => {
            write!(f, "  view.setBigInt64({}, args.", offset)?;
            write_camel(f, name)?;
            writeln!(f, ", true);")
        }
        "u128" => {
            write!(f, "  view.setBigUint64({}, args.", offset)?;
            write_camel(f, name)?;
            writeln!(f, " & 0xFFFFFFFFFFFFFFFFn, true);")?;
            write!(f, "  view.setBigUint64({}, args.", offset + 8)?;
            write_camel(f, name)?;
            writeln!(f, " >> 64n, true);")
        }
        "bool" => {
            write!(f, "  data[{}] = args.", offset)?;
            write_camel(f, name)?;
            writeln!(f, " ? 1 : 0;")
        }
        "Pubkey" => {
            write!(f, "  data.set(args.")?;
            write_camel(f, name)?;
            writeln!(f, ".toBytes(), {});", offset)
        }
        _ => {
            write!(f, "  data.set(args.")?;
            write_camel(f, name)?;
            writeln!(f, ", {});", offset)
        }
    }
}

fn write_encode_dynamic_expr(f: &mut fmt::Formatter<'_>, arg: &ArgDescriptor) -> fmt::Result {
    let mut arg_ref = alloc::string::String::from("args.");
    // `write_camel` writes to a formatter, not a String. Argument names emitted
    // by the program macro are Rust identifiers today, so snake_case -> camel
    // is reproduced locally for cursor-based expressions.
    let mut upper = false;
    for ch in arg.name.chars() {
        if ch == '_' || ch == '-' {
            upper = true;
        } else if upper {
            arg_ref.extend(ch.to_uppercase());
            upper = false;
        } else {
            arg_ref.push(ch);
        }
    }

    match arg.encoding {
        ArgEncoding::BoundedString { .. } => {
            let bytes = &arg_ref[5..];
            writeln!(f, "  view.setUint16(offset, {}Bytes.length, true);", bytes)?;
            writeln!(f, "  offset += 2;")?;
            writeln!(f, "  data.set({}Bytes, offset);", bytes)?;
            writeln!(f, "  offset += {}Bytes.length;", bytes)
        }
        ArgEncoding::BoundedVec { element_size, .. } => {
            let element = bounded_vec_element_type(arg.canonical_type).unwrap_or("u8");
            writeln!(f, "  view.setUint16(offset, {}.length, true);", arg_ref)?;
            writeln!(f, "  offset += 2;")?;
            if element == "u8" {
                writeln!(f, "  data.set({}, offset);", arg_ref)?;
                return writeln!(f, "  offset += {}.length;", arg_ref);
            }
            writeln!(f, "  for (const value of {}) {{", arg_ref)?;
            match element {
                "u16" => writeln!(f, "    view.setUint16(offset, value, true);")?,
                "i16" => writeln!(f, "    view.setInt16(offset, value, true);")?,
                "u32" => writeln!(f, "    view.setUint32(offset, value, true);")?,
                "i32" => writeln!(f, "    view.setInt32(offset, value, true);")?,
                "u64" => writeln!(f, "    view.setBigUint64(offset, value, true);")?,
                "i64" => writeln!(f, "    view.setBigInt64(offset, value, true);")?,
                "u128" => {
                    writeln!(
                        f,
                        "    view.setBigUint64(offset, value & 0xFFFFFFFFFFFFFFFFn, true);"
                    )?;
                    writeln!(f, "    view.setBigUint64(offset + 8, value >> 64n, true);")?;
                }
                "i128" => {
                    writeln!(
                        f,
                        "    view.setBigUint64(offset, value & 0xFFFFFFFFFFFFFFFFn, true);"
                    )?;
                    writeln!(f, "    view.setBigInt64(offset + 8, value >> 64n, true);")?;
                }
                "bool" => writeln!(f, "    data[offset] = value ? 1 : 0;")?,
                "Pubkey" => writeln!(f, "    data.set(value.toBytes(), offset);")?,
                _ => writeln!(f, "    data.set(value, offset);")?,
            }
            writeln!(f, "    offset += {};", element_size)?;
            writeln!(f, "  }}")
        }
        ArgEncoding::Fixed => {
            match arg.canonical_type {
                "u8" => writeln!(f, "  data[offset] = {};", arg_ref)?,
                "i8" => writeln!(f, "  view.setInt8(offset, {});", arg_ref)?,
                "u16" => writeln!(f, "  view.setUint16(offset, {}, true);", arg_ref)?,
                "i16" => writeln!(f, "  view.setInt16(offset, {}, true);", arg_ref)?,
                "u32" => writeln!(f, "  view.setUint32(offset, {}, true);", arg_ref)?,
                "i32" => writeln!(f, "  view.setInt32(offset, {}, true);", arg_ref)?,
                "u64" => writeln!(f, "  view.setBigUint64(offset, {}, true);", arg_ref)?,
                "i64" => writeln!(f, "  view.setBigInt64(offset, {}, true);", arg_ref)?,
                "u128" => {
                    writeln!(
                        f,
                        "  view.setBigUint64(offset, {} & 0xFFFFFFFFFFFFFFFFn, true);",
                        arg_ref
                    )?;
                    writeln!(
                        f,
                        "  view.setBigUint64(offset + 8, {} >> 64n, true);",
                        arg_ref
                    )?;
                }
                "bool" => writeln!(f, "  data[offset] = {} ? 1 : 0;", arg_ref)?,
                "Pubkey" => writeln!(f, "  data.set({}.toBytes(), offset);", arg_ref)?,
                _ => writeln!(f, "  data.set({}, offset);", arg_ref)?,
            }
            writeln!(f, "  offset += {};", arg.size)
        }
    }
}

fn instruction_data_size(ix: &InstructionDescriptor) -> usize {
    let mut size = 1usize; // discriminator byte
    for arg in ix.args.iter() {
        size += arg.size as usize;
    }
    size
}

fn event_data_size(event: &EventDescriptor) -> usize {
    1 + event
        .fields
        .iter()
        .map(|field| field.offset as usize + field.size as usize)
        .max()
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Kotlin Type Mapping
// ---------------------------------------------------------------------------

/// Map a Hopper canonical type name to a Kotlin type.
fn kt_type(canonical: &str) -> &str {
    match canonical {
        "u8" => "UByte",
        "i8" => "Byte",
        "u16" => "UShort",
        "i16" => "Short",
        "u32" => "UInt",
        "i32" => "Int",
        "u64" => "ULong",
        "i64" => "Long",
        "u128" | "i128" => "ByteArray",
        "bool" => "Boolean",
        "Pubkey" => "PublicKey",
        // `[u8; N]` byte arrays and any unknown type both map to `ByteArray`.
        _ => "ByteArray",
    }
}

/// Write PascalCase (used for Kotlin classes/interfaces).
fn write_kt_pascal(f: &mut fmt::Formatter<'_>, name: &str) -> fmt::Result {
    write_pascal(f, name)
}

/// Write camelCase (used for Kotlin properties/functions).
fn write_kt_camel(f: &mut fmt::Formatter<'_>, name: &str) -> fmt::Result {
    write_camel(f, name)
}

/// Write SCREAMING_SNAKE_CASE for constants.
fn write_kt_const(f: &mut fmt::Formatter<'_>, name: &str) -> fmt::Result {
    write_upper_snake(f, name)
}

fn write_kt_decode_expr(
    f: &mut fmt::Formatter<'_>,
    canonical: &str,
    offset: usize,
    end: usize,
) -> fmt::Result {
    match canonical {
        "u8" => write!(f, "data[{}].toUByte()", offset),
        "i8" => write!(f, "data[{}]", offset),
        "u16" => write!(
            f,
            "ByteBuffer.wrap(data, {}, 2).order(ByteOrder.LITTLE_ENDIAN).short.toUShort()",
            offset
        ),
        "i16" => write!(
            f,
            "ByteBuffer.wrap(data, {}, 2).order(ByteOrder.LITTLE_ENDIAN).short",
            offset
        ),
        "u32" => write!(
            f,
            "ByteBuffer.wrap(data, {}, 4).order(ByteOrder.LITTLE_ENDIAN).int.toUInt()",
            offset
        ),
        "i32" => write!(
            f,
            "ByteBuffer.wrap(data, {}, 4).order(ByteOrder.LITTLE_ENDIAN).int",
            offset
        ),
        "u64" => write!(
            f,
            "ByteBuffer.wrap(data, {}, 8).order(ByteOrder.LITTLE_ENDIAN).long.toULong()",
            offset
        ),
        "i64" => write!(
            f,
            "ByteBuffer.wrap(data, {}, 8).order(ByteOrder.LITTLE_ENDIAN).long",
            offset
        ),
        "u128" | "i128" => write!(f, "data.copyOfRange({}, {})", offset, end),
        "bool" => write!(f, "data[{}] != 0.toByte()", offset),
        "Pubkey" => write!(f, "PublicKey(data.copyOfRange({}, {}))", offset, end),
        _ => write!(f, "data.copyOfRange({}, {})", offset, end),
    }
}

fn write_kt_encode_expr(
    f: &mut fmt::Formatter<'_>,
    canonical: &str,
    name: &str,
    offset: usize,
) -> fmt::Result {
    match canonical {
        "u8" => {
            write!(f, "    data[{}] = args.", offset)?;
            write_kt_camel(f, name)?;
            writeln!(f, ".toByte()")
        }
        "i8" => {
            write!(f, "    data[{}] = args.", offset)?;
            write_kt_camel(f, name)?;
            writeln!(f)
        }
        "u16" => {
            write!(
                f,
                "    ByteBuffer.wrap(data, {}, 2).order(ByteOrder.LITTLE_ENDIAN).putShort(args.",
                offset
            )?;
            write_kt_camel(f, name)?;
            writeln!(f, ".toShort())")
        }
        "i16" => {
            write!(
                f,
                "    ByteBuffer.wrap(data, {}, 2).order(ByteOrder.LITTLE_ENDIAN).putShort(args.",
                offset
            )?;
            write_kt_camel(f, name)?;
            writeln!(f, ")")
        }
        "u32" => {
            write!(
                f,
                "    ByteBuffer.wrap(data, {}, 4).order(ByteOrder.LITTLE_ENDIAN).putInt(args.",
                offset
            )?;
            write_kt_camel(f, name)?;
            writeln!(f, ".toInt())")
        }
        "i32" => {
            write!(
                f,
                "    ByteBuffer.wrap(data, {}, 4).order(ByteOrder.LITTLE_ENDIAN).putInt(args.",
                offset
            )?;
            write_kt_camel(f, name)?;
            writeln!(f, ")")
        }
        "u64" => {
            write!(
                f,
                "    ByteBuffer.wrap(data, {}, 8).order(ByteOrder.LITTLE_ENDIAN).putLong(args.",
                offset
            )?;
            write_kt_camel(f, name)?;
            writeln!(f, ".toLong())")
        }
        "i64" => {
            write!(
                f,
                "    ByteBuffer.wrap(data, {}, 8).order(ByteOrder.LITTLE_ENDIAN).putLong(args.",
                offset
            )?;
            write_kt_camel(f, name)?;
            writeln!(f, ")")
        }
        "bool" => {
            write!(f, "    data[{}] = if (args.", offset)?;
            write_kt_camel(f, name)?;
            writeln!(f, ") 1.toByte() else 0.toByte()")
        }
        "Pubkey" => {
            write!(f, "    args.")?;
            write_kt_camel(f, name)?;
            writeln!(f, ".toByteArray().copyInto(data, {})", offset)
        }
        _ => {
            write!(f, "    args.")?;
            write_kt_camel(f, name)?;
            writeln!(f, ".copyInto(data, {})", offset)
        }
    }
}

// ---------------------------------------------------------------------------
// Kotlin Accounts
// ---------------------------------------------------------------------------

/// Generates Kotlin data classes and decoders for account layouts.
pub struct KtAccounts<'a>(pub &'a ProgramManifest);

impl<'a> fmt::Display for KtAccounts<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let prog = self.0;

        writeln!(f, "// Auto-generated by hopper client gen --kt")?;
        writeln!(f, "// Program: {} v{}", prog.name, prog.version)?;
        writeln!(f, "// DO NOT EDIT")?;
        writeln!(f)?;
        writeln!(
            f,
            "package hopper.generated.{}",
            prog.name.replace('-', "_")
        )?;
        writeln!(f)?;
        writeln!(f, "import org.sol4k.PublicKey")?;
        writeln!(f, "import java.nio.ByteBuffer")?;
        writeln!(f, "import java.nio.ByteOrder")?;
        writeln!(f)?;

        // Hopper header layout constants. The 16-byte header carries the
        // 8-byte LAYOUT_ID at bytes [4, 12). Clients compare this against
        // the compiled-in per-layout constant before decoding, which is
        // the client-side complement to the runtime's `load::<T>()`
        // fingerprint check.
        writeln!(f, "/** Hopper account header size in bytes. */")?;
        writeln!(f, "const val HEADER_SIZE: Int = {}", HOPPER_HEADER_SIZE)?;
        writeln!(
            f,
            "/** Compact account body starts immediately after the discriminator byte. */"
        )?;
        writeln!(f, "const val COMPACT_BODY_OFFSET: Int = 1")?;
        writeln!(
            f,
            "/** Byte offset of the 8-byte layout fingerprint in a Hopper header. */"
        )?;
        writeln!(f, "const val LAYOUT_ID_OFFSET: Int = 4")?;
        writeln!(f, "/** Byte length of the layout fingerprint. */")?;
        writeln!(f, "const val LAYOUT_ID_LENGTH: Int = 8")?;
        writeln!(f)?;

        writeln!(f, "enum class AccountEncoding {{ Headered, Compact }}")?;
        writeln!(f)?;
        writeln!(f, "data class LayoutIdentity(")?;
        writeln!(f, "    val name: String,")?;
        writeln!(f, "    val encoding: AccountEncoding,")?;
        writeln!(f, "    val disc: Int,")?;
        writeln!(f, "    val size: Int,")?;
        writeln!(f, "    val layoutId: String,")?;
        writeln!(f, ")")?;
        writeln!(f)?;
        writeln!(
            f,
            "fun layoutFingerprint(layout: LayoutIdentity): String = layout.layoutId"
        )?;
        writeln!(f)?;

        writeln!(
            f,
            "class LayoutMismatchException(expected: String, actual: String) :"
        )?;
        writeln!(f, "    RuntimeException(\"Hopper layout mismatch: account header reports $actual, expected $expected\")")?;
        writeln!(f)?;

        writeln!(f, "/**")?;
        writeln!(
            f,
            " * Raise if `data` is not a Hopper account encoding the expected layout."
        )?;
        writeln!(f, " *")?;
        writeln!(
            f,
            " * Reads the 8-byte LAYOUT_ID fingerprint from the 16-byte Hopper header"
        )?;
        writeln!(
            f,
            " * (bytes 4..12) and compares it against `expectedHex` (16 lowercase hex chars)."
        )?;
        writeln!(
            f,
            " * Mismatch means the on-chain program was upgraded with a different ABI"
        )?;
        writeln!(f, " * than the client was generated against.")?;
        writeln!(f, " */")?;
        writeln!(
            f,
            "fun assertLayoutId(data: ByteArray, expectedHex: String) {{"
        )?;
        writeln!(f, "    if (data.size < HEADER_SIZE) {{")?;
        writeln!(f, "        throw RuntimeException(\"Hopper account too short: ${{data.size}} < $HEADER_SIZE\")")?;
        writeln!(f, "    }}")?;
        writeln!(f, "    val sb = StringBuilder(LAYOUT_ID_LENGTH * 2)")?;
        writeln!(f, "    for (i in 0 until LAYOUT_ID_LENGTH) {{")?;
        writeln!(
            f,
            "        val byte = data[LAYOUT_ID_OFFSET + i].toInt() and 0xFF"
        )?;
        writeln!(f, "        sb.append(String.format(\"%02x\", byte))")?;
        writeln!(f, "    }}")?;
        writeln!(f, "    val actualHex = sb.toString()")?;
        writeln!(f, "    if (actualHex != expectedHex.lowercase()) {{")?;
        writeln!(
            f,
            "        throw LayoutMismatchException(expectedHex, actualHex)"
        )?;
        writeln!(f, "    }}")?;
        writeln!(f, "}}")?;
        writeln!(f)?;

        writeln!(f, "/**")?;
        writeln!(
            f,
            " * Raise if `data` is not a compact Hopper account for `layout`."
        )?;
        writeln!(
            f,
            " * Compact accounts carry only discriminator + body; the layout fingerprint"
        )?;
        writeln!(
            f,
            " * comes from manifest/IDL metadata through `layoutFingerprint(layout)`."
        )?;
        writeln!(f, " */")?;
        writeln!(
            f,
            "fun assertCompactLayout(data: ByteArray, layout: LayoutIdentity) {{"
        )?;
        writeln!(f, "    if (data.size != layout.size) {{")?;
        writeln!(f, "        throw RuntimeException(\"Hopper compact account size mismatch for ${{layout.name}}: ${{data.size}} != ${{layout.size}}\")")?;
        writeln!(f, "    }}")?;
        writeln!(f, "    val actualDisc = data[0].toInt() and 0xFF")?;
        writeln!(f, "    if (actualDisc != layout.disc) {{")?;
        writeln!(f, "        throw RuntimeException(\"Hopper compact discriminator mismatch for ${{layout.name}}: $actualDisc != ${{layout.disc}}\")")?;
        writeln!(f, "    }}")?;
        writeln!(f, "}}")?;
        writeln!(f)?;

        for layout in prog.layouts.iter() {
            // Data class
            write!(f, "data class ")?;
            write_kt_pascal(f, layout.name)?;
            writeln!(f, "(")?;
            for (i, field) in layout.fields.iter().enumerate() {
                write!(f, "    val ")?;
                write_kt_camel(f, field.name)?;
                write!(f, ": {}", kt_type(field.canonical_type))?;
                if i + 1 < layout.fields.len() {
                    writeln!(f, ",")?;
                } else {
                    writeln!(f)?;
                }
            }
            writeln!(f, ")")?;
            writeln!(f)?;

            // Layout fingerprint constant (hex). Pairs with the
            // runtime's `T::LAYOUT_ID` so the client and the program
            // agree on the ABI byte-for-byte.
            write!(f, "const val ")?;
            write_kt_const(f, layout.name)?;
            write!(f, "_LAYOUT_ID: String = \"")?;
            for b in layout.layout_id.iter() {
                write!(f, "{:02x}", b)?;
            }
            writeln!(f, "\"")?;
            writeln!(f)?;

            write!(f, "const val ")?;
            write_kt_const(f, layout.name)?;
            writeln!(f, "_SIZE: Int = {}", layout.total_size)?;
            write!(f, "val ")?;
            write_kt_const(f, layout.name)?;
            writeln!(
                f,
                "_ACCOUNT_ENCODING: AccountEncoding = AccountEncoding.{}",
                if layout_is_compact(layout) {
                    "Compact"
                } else {
                    "Headered"
                }
            )?;
            writeln!(f)?;

            // Discriminator constant
            write!(f, "const val ")?;
            write_kt_const(f, layout.name)?;
            writeln!(f, "_DISC: Byte = {}", layout.disc)?;
            writeln!(f)?;

            write!(f, "val ")?;
            write_kt_const(f, layout.name)?;
            writeln!(f, "_LAYOUT: LayoutIdentity = LayoutIdentity(")?;
            writeln!(f, "    name = \"{}\",", layout.name)?;
            write!(f, "    encoding = ")?;
            write_kt_const(f, layout.name)?;
            writeln!(f, "_ACCOUNT_ENCODING,")?;
            write!(f, "    disc = ")?;
            write_kt_const(f, layout.name)?;
            writeln!(f, "_DISC.toInt() and 0xFF,")?;
            write!(f, "    size = ")?;
            write_kt_const(f, layout.name)?;
            writeln!(f, "_SIZE,")?;
            write!(f, "    layoutId = ")?;
            write_kt_const(f, layout.name)?;
            writeln!(f, "_LAYOUT_ID,")?;
            writeln!(f, ")")?;
            writeln!(f)?;

            // Per-layout assertion helper. Thin wrapper over
            // the headered or compact assertion.
            write!(f, "fun assert")?;
            write_kt_pascal(f, layout.name)?;
            writeln!(f, "Layout(data: ByteArray) {{")?;
            if layout_is_compact(layout) {
                write!(f, "    assertCompactLayout(data, ")?;
                write_kt_const(f, layout.name)?;
                writeln!(f, "_LAYOUT)")?;
            } else {
                write!(f, "    assertLayoutId(data, ")?;
                write_kt_const(f, layout.name)?;
                writeln!(f, "_LAYOUT_ID)")?;
            }
            writeln!(f, "}}")?;
            writeln!(f)?;

            // Decoder
            write!(f, "fun decode")?;
            write_kt_pascal(f, layout.name)?;
            write!(f, "(data: ByteArray): ")?;
            write_kt_pascal(f, layout.name)?;
            writeln!(f, " {{")?;
            write!(f, "    assert")?;
            write_kt_pascal(f, layout.name)?;
            writeln!(f, "Layout(data)")?;
            writeln!(
                f,
                "    require(data.size >= {}) {{ \"Data too small for {}\" }}",
                layout.total_size, layout.name
            )?;

            for field in layout.fields.iter() {
                let offset = field.offset as usize;
                let end = offset + field.size as usize;
                write!(f, "    val ")?;
                write_kt_camel(f, field.name)?;
                write!(f, " = ")?;
                write_kt_decode_expr(f, field.canonical_type, offset, end)?;
                writeln!(f)?;
            }

            write!(f, "    return ")?;
            write_kt_pascal(f, layout.name)?;
            writeln!(f, "(")?;
            for (i, field) in layout.fields.iter().enumerate() {
                write!(f, "        ")?;
                write_kt_camel(f, field.name)?;
                write!(f, " = ")?;
                write_kt_camel(f, field.name)?;
                if i + 1 < layout.fields.len() {
                    writeln!(f, ",")?;
                } else {
                    writeln!(f)?;
                }
            }
            writeln!(f, "    )")?;
            writeln!(f, "}}")?;
            writeln!(f)?;
        }

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Kotlin Instructions
// ---------------------------------------------------------------------------

/// Generates Kotlin instruction builders from a `ProgramManifest`.
pub struct KtInstructions<'a>(pub &'a ProgramManifest);

impl<'a> fmt::Display for KtInstructions<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let prog = self.0;

        writeln!(f, "// Auto-generated by hopper client gen --kt")?;
        writeln!(f, "// Program: {} v{}", prog.name, prog.version)?;
        writeln!(f, "// DO NOT EDIT")?;
        writeln!(f)?;
        writeln!(
            f,
            "package hopper.generated.{}",
            prog.name.replace('-', "_")
        )?;
        writeln!(f)?;
        writeln!(f, "import org.sol4k.PublicKey")?;
        writeln!(f, "import org.sol4k.instruction.AccountMeta")?;
        writeln!(f, "import org.sol4k.instruction.Instruction")?;
        writeln!(f, "import java.nio.ByteBuffer")?;
        writeln!(f, "import java.nio.ByteOrder")?;
        writeln!(f)?;

        for ix in prog.instructions.iter() {
            // Args data class
            if !ix.args.is_empty() {
                write!(f, "data class ")?;
                write_kt_pascal(f, ix.name)?;
                writeln!(f, "Args(")?;
                for (i, arg) in ix.args.iter().enumerate() {
                    write!(f, "    val ")?;
                    write_kt_camel(f, arg.name)?;
                    write!(f, ": {}", kt_type(arg.canonical_type))?;
                    if i + 1 < ix.args.len() {
                        writeln!(f, ",")?;
                    } else {
                        writeln!(f)?;
                    }
                }
                writeln!(f, ")")?;
                writeln!(f)?;
            }

            // Accounts data class: only caller-provided accounts. PDAs are
            // derived in the builder, so callers never pass (or mis-derive) them.
            write!(f, "data class ")?;
            write_kt_pascal(f, ix.name)?;
            writeln!(f, "Accounts(")?;
            for (i, acc) in ix.accounts.iter().enumerate() {
                if account_is_auto_pda(acc) {
                    continue;
                }
                write!(f, "    val ")?;
                write_kt_camel(f, acc.name)?;
                write!(f, ": PublicKey")?;
                if i + 1 < ix.accounts.len() {
                    writeln!(f, ",")?;
                } else {
                    writeln!(f)?;
                }
            }
            writeln!(f, ")")?;
            writeln!(f)?;

            // Builder function
            write!(f, "fun create")?;
            write_kt_pascal(f, ix.name)?;
            writeln!(f, "Instruction(")?;
            if !ix.args.is_empty() {
                write!(f, "    args: ")?;
                write_kt_pascal(f, ix.name)?;
                writeln!(f, "Args,")?;
            }
            write!(f, "    accounts: ")?;
            write_kt_pascal(f, ix.name)?;
            writeln!(f, "Accounts,")?;
            writeln!(f, "    programId: PublicKey,")?;
            writeln!(f, "): Instruction {{")?;

            // Resolve program-derived addresses from their on-chain seeds.
            if ix.accounts.iter().any(account_is_auto_pda) {
                writeln!(
                    f,
                    "    // Program-derived addresses, resolved from on-chain seeds."
                )?;
                for acc in ix.accounts.iter() {
                    if !account_is_auto_pda(acc) {
                        continue;
                    }
                    write!(f, "    val ")?;
                    write_kt_camel(f, acc.name)?;
                    writeln!(f, " = PublicKey.findProgramAddress(")?;
                    writeln!(f, "        listOf(")?;
                    for (i, seed) in acc.seeds.iter().enumerate() {
                        let comma = if i + 1 < acc.seeds.len() { "," } else { "" };
                        write!(f, "            ")?;
                        match crate::classify_seed(seed) {
                            crate::SeedPart::Literal(text) => {
                                write_kt_string(f, text)?;
                                write!(f, ".toByteArray()")?;
                            }
                            crate::SeedPart::Account(name) => {
                                let refs_pda = ix
                                    .accounts
                                    .iter()
                                    .any(|a| a.name == name && account_is_auto_pda(a));
                                if !refs_pda {
                                    write!(f, "accounts.")?;
                                }
                                write_kt_camel(f, name)?;
                                write!(f, ".bytes()")?;
                            }
                            crate::SeedPart::Arg(_) | crate::SeedPart::Unknown(_) => {}
                        }
                        writeln!(f, "{}", comma)?;
                    }
                    writeln!(f, "        ),")?;
                    writeln!(f, "        programId,")?;
                    writeln!(f, "    ).address")?;
                }
                writeln!(f)?;
            }

            let data_size = instruction_data_size(ix);
            writeln!(f, "    val data = ByteArray({})", data_size)?;
            writeln!(
                f,
                "    data[0] = {}.toByte() // instruction discriminator",
                ix.tag
            )?;

            let mut offset = 1usize;
            for arg in ix.args.iter() {
                write_kt_encode_expr(f, arg.canonical_type, arg.name, offset)?;
                offset += arg.size as usize;
            }

            writeln!(f)?;
            writeln!(f, "    val keys = listOf(")?;
            for acc in ix.accounts.iter() {
                if account_is_auto_pda(acc) {
                    write!(f, "        AccountMeta(")?;
                    write_kt_camel(f, acc.name)?;
                } else {
                    write!(f, "        AccountMeta(accounts.")?;
                    write_kt_camel(f, acc.name)?;
                }
                writeln!(
                    f,
                    ", isSigner = {}, isWritable = {}),",
                    acc.signer, acc.writable
                )?;
            }
            writeln!(f, "    )")?;
            writeln!(f)?;
            writeln!(f, "    return Instruction(programId, keys, data)")?;
            writeln!(f, "}}")?;
            writeln!(f)?;
        }

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Kotlin Events
// ---------------------------------------------------------------------------

/// Generates Kotlin event data classes and decoders.
pub struct KtEvents<'a>(pub &'a ProgramManifest);

impl<'a> fmt::Display for KtEvents<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let prog = self.0;

        writeln!(f, "// Auto-generated by hopper client gen --kt")?;
        writeln!(f, "// Program: {} v{}", prog.name, prog.version)?;
        writeln!(f, "// DO NOT EDIT")?;
        writeln!(f)?;
        writeln!(
            f,
            "package hopper.generated.{}",
            prog.name.replace('-', "_")
        )?;
        writeln!(f)?;
        writeln!(f, "import org.sol4k.PublicKey")?;
        writeln!(f, "import java.nio.ByteBuffer")?;
        writeln!(f, "import java.nio.ByteOrder")?;
        writeln!(f)?;

        if prog.events.is_empty() {
            writeln!(f, "// No events defined for this program.")?;
            return Ok(());
        }

        for event in prog.events.iter() {
            write!(f, "data class ")?;
            write_kt_pascal(f, event.name)?;
            writeln!(f, "Event(")?;
            for (i, field) in event.fields.iter().enumerate() {
                write!(f, "    val ")?;
                write_kt_camel(f, field.name)?;
                write!(f, ": {}", kt_type(field.canonical_type))?;
                if i + 1 < event.fields.len() {
                    writeln!(f, ",")?;
                } else {
                    writeln!(f)?;
                }
            }
            writeln!(f, ")")?;
            writeln!(f)?;

            write!(f, "const val ")?;
            write_kt_const(f, event.name)?;
            writeln!(f, "_EVENT_DISC: Byte = {}", event.tag)?;
            write!(f, "const val ")?;
            write_kt_const(f, event.name)?;
            writeln!(f, "_EVENT_DATA_LEN: Int = {}", event_data_size(event))?;
            writeln!(f)?;

            write!(f, "fun decode")?;
            write_kt_pascal(f, event.name)?;
            write!(f, "Event(data: ByteArray): ")?;
            write_kt_pascal(f, event.name)?;
            writeln!(f, "Event {{")?;
            write!(f, "    require(data.size >= ")?;
            write_kt_const(f, event.name)?;
            writeln!(
                f,
                "_EVENT_DATA_LEN) {{ \"Data too small for {} event\" }}",
                event.name
            )?;
            write!(f, "    require(data[0] == ")?;
            write_kt_const(f, event.name)?;
            writeln!(f, "_EVENT_DISC) {{ \"Event tag mismatch\" }}")?;

            for field in event.fields.iter() {
                let offset = field.offset as usize + 1;
                let end = offset + field.size as usize;
                write!(f, "    val ")?;
                write_kt_camel(f, field.name)?;
                write!(f, " = ")?;
                write_kt_decode_expr(f, field.canonical_type, offset, end)?;
                writeln!(f)?;
            }

            write!(f, "    return ")?;
            write_kt_pascal(f, event.name)?;
            writeln!(f, "Event(")?;
            for (i, field) in event.fields.iter().enumerate() {
                write!(f, "        ")?;
                write_kt_camel(f, field.name)?;
                write!(f, " = ")?;
                write_kt_camel(f, field.name)?;
                if i + 1 < event.fields.len() {
                    writeln!(f, ",")?;
                } else {
                    writeln!(f)?;
                }
            }
            writeln!(f, "    )")?;
            writeln!(f, "}}")?;
            writeln!(f)?;
        }

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Kotlin Types
// ---------------------------------------------------------------------------

/// Generates Kotlin header type and discriminator constants.
pub struct KtTypes<'a>(pub &'a ProgramManifest);

impl<'a> fmt::Display for KtTypes<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let prog = self.0;

        writeln!(f, "// Auto-generated by hopper client gen --kt")?;
        writeln!(f, "// Program: {} v{}", prog.name, prog.version)?;
        writeln!(f, "// DO NOT EDIT")?;
        writeln!(f)?;
        writeln!(
            f,
            "package hopper.generated.{}",
            prog.name.replace('-', "_")
        )?;
        writeln!(f)?;
        writeln!(f, "import java.nio.ByteBuffer")?;
        writeln!(f, "import java.nio.ByteOrder")?;
        writeln!(f)?;
        writeln!(f, "/** Hopper account header (16 bytes). */")?;
        writeln!(f, "data class HopperHeader(")?;
        writeln!(f, "    val disc: UByte,")?;
        writeln!(f, "    val version: UByte,")?;
        writeln!(f, "    val flags: UShort,")?;
        writeln!(f, "    val layoutId: ByteArray,")?;
        writeln!(f, "    val schemaEpoch: UInt")?;
        writeln!(f, ")")?;
        writeln!(f)?;
        writeln!(f, "private const val TYPES_HEADER_SIZE: Int = 16")?;
        writeln!(f, "private const val TYPES_LAYOUT_ID_OFFSET: Int = 4")?;
        writeln!(f, "private const val TYPES_LAYOUT_ID_LENGTH: Int = 8")?;
        writeln!(f)?;
        writeln!(f, "/** Decode the Hopper 16-byte account header. */")?;
        writeln!(f, "fun decodeHeader(data: ByteArray): HopperHeader {{")?;
        writeln!(
            f,
            "    require(data.size >= TYPES_HEADER_SIZE) {{ \"Data too small for header\" }}"
        )?;
        writeln!(f, "    return HopperHeader(")?;
        writeln!(f, "        disc = data[0].toUByte(),")?;
        writeln!(f, "        version = data[1].toUByte(),")?;
        writeln!(
            f,
            "        flags = ByteBuffer.wrap(data, 2, 2).order(ByteOrder.LITTLE_ENDIAN).short.toUShort(),"
        )?;
        writeln!(
            f,
            "        layoutId = data.copyOfRange(TYPES_LAYOUT_ID_OFFSET, TYPES_LAYOUT_ID_OFFSET + TYPES_LAYOUT_ID_LENGTH),"
        )?;
        writeln!(
            f,
            "        schemaEpoch = ByteBuffer.wrap(data, 12, 4).order(ByteOrder.LITTLE_ENDIAN).int.toUInt()"
        )?;
        writeln!(f, "    )")?;
        writeln!(f, "}}")?;
        writeln!(f)?;
        writeln!(
            f,
            "/** All account discriminators for {} v{}. */",
            prog.name, prog.version
        )?;
        writeln!(f, "object Discriminators {{")?;
        for layout in prog.layouts.iter() {
            write!(f, "    const val ")?;
            write_kt_const(f, layout.name)?;
            writeln!(f, ": Byte = {}", layout.disc)?;
        }
        writeln!(f, "}}")?;

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Full Kotlin Client Generator
// ---------------------------------------------------------------------------

/// Generates all Kotlin client files for a program.
///
/// Output format uses file markers like the TypeScript generator:
/// ```text
/// === Types.kt ===
/// <contents>
/// === Accounts.kt ===
/// <contents>
/// === Instructions.kt ===
/// <contents>
/// === Events.kt ===
/// <contents>
/// ```
pub struct KtClientGen<'a>(pub &'a ProgramManifest);

impl<'a> fmt::Display for KtClientGen<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let prog = self.0;

        writeln!(f, "=== Types.kt ===")?;
        write!(f, "{}", KtTypes(prog))?;
        writeln!(f)?;

        writeln!(f, "=== Accounts.kt ===")?;
        write!(f, "{}", KtAccounts(prog))?;
        writeln!(f)?;

        writeln!(f, "=== Instructions.kt ===")?;
        write!(f, "{}", KtInstructions(prog))?;
        writeln!(f)?;

        writeln!(f, "=== Events.kt ===")?;
        write!(f, "{}", KtEvents(prog))?;

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    extern crate alloc;
    use super::*;
    use crate::{
        AccountEntry, ArgDescriptor, EventDescriptor, FieldDescriptor, FieldIntent, LayoutManifest,
    };
    use alloc::string::ToString;

    fn test_manifest() -> ProgramManifest {
        static FIELDS: &[FieldDescriptor] = &[
            FieldDescriptor {
                name: "authority",
                canonical_type: "Pubkey",
                size: 32,
                offset: 16,
                intent: FieldIntent::Custom,
            },
            FieldDescriptor {
                name: "amount",
                canonical_type: "u64",
                size: 8,
                offset: 48,
                intent: FieldIntent::Custom,
            },
            FieldDescriptor {
                name: "is_active",
                canonical_type: "bool",
                size: 1,
                offset: 56,
                intent: FieldIntent::Custom,
            },
        ];

        static LAYOUTS: &[LayoutManifest] = &[LayoutManifest {
            name: "vault",
            disc: 1,
            version: 1,
            layout_id: [0xAA, 0xBB, 0xCC, 0xDD, 0x11, 0x22, 0x33, 0x44],
            total_size: 64,
            field_count: 3,
            fields: FIELDS,
        }];

        static ARGS: &[ArgDescriptor] = &[ArgDescriptor {
            name: "amount",
            canonical_type: "u64",
            size: 8,
            encoding: crate::ArgEncoding::Fixed,
        }];

        static ACCOUNTS: &[AccountEntry] = &[
            AccountEntry {
                name: "authority",
                writable: false,
                signer: true,
                layout_ref: "",
                seeds: &[],
            },
            AccountEntry {
                name: "vault",
                writable: true,
                signer: false,
                layout_ref: "vault",
                seeds: &["b\"vault\"", "authority.key().as_ref()"],
            },
        ];

        static INSTRUCTIONS: &[InstructionDescriptor] = &[InstructionDescriptor {
            name: "deposit",
            tag: 0,
            args: ARGS,
            accounts: ACCOUNTS,
            remaining_accounts: None,
            capabilities: &["write"],
            policy_pack: "standard",
            receipt_expected: true,
            strict_writes: false,
            write_ranges: &[],
            parametric_write_ranges: &[],
            mutation_complete: false,
            lamport_accounts: &[],
            cu_estimate: 0,
        }];

        static EVENT_FIELDS: &[FieldDescriptor] = &[
            FieldDescriptor {
                name: "depositor",
                canonical_type: "Pubkey",
                size: 32,
                offset: 0,
                intent: FieldIntent::Custom,
            },
            FieldDescriptor {
                name: "amount",
                canonical_type: "u64",
                size: 8,
                offset: 32,
                intent: FieldIntent::Custom,
            },
        ];

        static EVENTS: &[EventDescriptor] = &[EventDescriptor {
            name: "deposit_event",
            tag: 0,
            fields: EVENT_FIELDS,
        }];

        ProgramManifest {
            name: "test_vault",
            version: "0.1.0",
            description: "A test vault program",
            layouts: LAYOUTS,
            layout_metadata: &[],
            instructions: INSTRUCTIONS,
            events: EVENTS,
            policies: &[],
            compatibility_pairs: &[],
            tooling_hints: &[],
            contexts: &[],
        }
    }

    fn compact_manifest() -> ProgramManifest {
        static FIELDS: &[FieldDescriptor] = &[
            FieldDescriptor {
                name: "authority",
                canonical_type: "Pubkey",
                size: 32,
                offset: 1,
                intent: FieldIntent::Custom,
            },
            FieldDescriptor {
                name: "balance",
                canonical_type: "u64",
                size: 8,
                offset: 33,
                intent: FieldIntent::Custom,
            },
        ];

        static LAYOUTS: &[LayoutManifest] = &[LayoutManifest {
            name: "compact_vault",
            disc: 7,
            version: 1,
            layout_id: [0x10, 0x20, 0x30, 0x40, 0x50, 0x60, 0x70, 0x80],
            total_size: 41,
            field_count: 2,
            fields: FIELDS,
        }];

        ProgramManifest {
            name: "compact_vault_program",
            version: "0.1.0",
            description: "A compact account manifest",
            layouts: LAYOUTS,
            layout_metadata: &[],
            instructions: &[],
            events: &[],
            policies: &[],
            compatibility_pairs: &[],
            tooling_hints: &[],
            contexts: &[],
        }
    }

    #[test]
    fn ts_accounts_generates_interface() {
        let m = test_manifest();
        let output = TsAccounts(&m).to_string();
        assert!(output.contains("export interface Vault {"));
        assert!(output.contains("authority: PublicKey;"));
        assert!(output.contains("amount: bigint;"));
        assert!(output.contains("isActive: boolean;"));
    }

    #[test]
    fn ts_accounts_generates_decoder() {
        let m = test_manifest();
        let output = TsAccounts(&m).to_string();
        assert!(output.contains("export function decodeVault(data: Uint8Array)"));
        assert!(output.contains("export function decodeVault(data: Uint8Array): \n  Vault {\n  assertVaultLayout(data);"));
        assert!(
            output.contains("throw new Error(`Data too small for vault: ${data.length} < 64`);")
        );
        assert!(output.contains("new PublicKey(data.slice(16, 48))"));
        assert!(output.contains("view.getBigUint64(48, true)"));
        assert!(output.contains("data[56] !== 0"));
    }

    #[test]
    fn ts_accounts_generates_discriminator() {
        let m = test_manifest();
        let output = TsAccounts(&m).to_string();
        assert!(output.contains("export const VAULT_DISC = 1;"));
    }

    #[test]
    fn ts_instructions_generates_builder() {
        let m = test_manifest();
        let output = TsInstructions(&m).to_string();
        assert!(output.contains("export interface DepositArgs {"));
        assert!(output.contains("export interface DepositAccounts {"));
        assert!(output.contains("export function createDepositInstruction("));
        assert!(output.contains("data[0] = 0; // instruction discriminator"));
        assert!(output.contains("view.setBigUint64(1, args.amount, true);"));
    }

    #[test]
    fn ts_instructions_account_meta() {
        let m = test_manifest();
        let output = TsInstructions(&m).to_string();
        // `authority` is caller-provided; `vault` is a PDA, so its key comes
        // from the derived local, not `accounts.vault`.
        assert!(output.contains("accounts.authority, isSigner: true, isWritable: false"));
        assert!(output.contains("pubkey: vault, isSigner: false, isWritable: true"));
        assert!(!output.contains("accounts.vault"));
    }

    #[test]
    fn ts_instructions_auto_derives_pda_accounts() {
        let m = test_manifest();
        let output = TsInstructions(&m).to_string();

        // The PDA account is dropped from the caller-facing interface...
        assert!(output.contains("export interface DepositAccounts {"));
        assert!(output.contains("  authority: PublicKey;"));
        assert!(!output.contains("  vault: PublicKey;"));

        // ...and derived in the builder from its on-chain seeds: the literal
        // `b"vault"` and the `authority` account's address.
        assert!(output.contains("const [vault] = PublicKey.findProgramAddressSync("));
        assert!(output.contains("[Buffer.from(\"vault\"), accounts.authority.toBuffer()],"));
        assert!(output.contains("    programId,"));
    }

    #[test]
    fn ts_events_generates_decoder() {
        let m = test_manifest();
        let output = TsEvents(&m).to_string();
        assert!(output.contains("export interface DepositEventEvent {"));
        assert!(output.contains("export function decodeDepositEventEvent(data: Uint8Array)"));
        assert!(output.contains("DEPOSIT_EVENT_EVENT_DISC = 0;"));
        assert!(output.contains("DEPOSIT_EVENT_EVENT_DATA_LEN = 41;"));
        assert!(output.contains("data[0] !== DEPOSIT_EVENT_EVENT_DISC"));
        assert!(output.contains("new PublicKey(data.slice(1, 33))"));
        assert!(output.contains("view.getBigUint64(33, true)"));
    }

    #[test]
    fn ts_events_emits_touch_map_decoder() {
        let m = test_manifest();
        let output = TsEvents(&m).to_string();
        // Wire-format constants (v1: magic 0x7A, version 0x01).
        assert!(output.contains("export const HOPPER_TOUCH_MAP_MAGIC = 0x7a;"));
        assert!(output.contains("export const HOPPER_TOUCH_MAP_VERSION = 0x01;"));
        // Pure, versioned decoder returning null on any mismatch.
        assert!(output.contains(
            "export function decodeHopperTouchMap(data: Uint8Array): HopperTouchMap | null {"
        ));
        assert!(output.contains(
            "if (data[0] !== HOPPER_TOUCH_MAP_MAGIC || data[1] !== HOPPER_TOUCH_MAP_VERSION) return null;"
        ));
        // Exact-length equation guards against misreading other payloads.
        assert!(output.contains("if (data.length !== 4 + count * 9) return null;"));
        // Record decode: packed u32 LE, top bit = write, low 31 = offset.
        assert!(output.contains("const packed = view.getUint32(base + 1, true);"));
        assert!(output.contains("offset: packed & 0x7fffffff,"));
        assert!(output.contains("write: (packed & 0x80000000) !== 0,"));
        assert!(output.contains("size: view.getUint32(base + 5, true),"));
        // Honesty flags surface in the decoded shape.
        assert!(output.contains("overflowed: (flags & 0x01) !== 0,"));
        assert!(output.contains("recordsSkipped: (flags & 0x02) !== 0,"));
    }

    #[test]
    fn ts_touch_map_decoder_is_emitted_even_without_events() {
        let m = compact_manifest();
        let output = TsEvents(&m).to_string();
        assert!(output.contains("export function decodeHopperTouchMap"));
        assert!(output.contains("// No events defined for this program."));
    }

    #[test]
    fn ts_types_generates_header() {
        let m = test_manifest();
        let output = TsTypes(&m).to_string();
        assert!(output.contains("export interface HopperHeader {"));
        assert!(output.contains("export function decodeHeader(data: Uint8Array)"));
        assert!(output.contains("flags: view.getUint16(2, true),"));
        assert!(output.contains(
            "layoutId: data.slice(LAYOUT_ID_OFFSET, LAYOUT_ID_OFFSET + LAYOUT_ID_LENGTH),"
        ));
        assert!(output.contains("schemaEpoch: view.getUint32(12, true),"));
        assert!(output.contains("Vault: 1,"));
    }

    #[test]
    fn ts_decode_header_and_assert_layout_id_share_offset() {
        let m = test_manifest();
        let accounts = TsAccounts(&m).to_string();
        let types = TsTypes(&m).to_string();
        assert!(accounts.contains("export const LAYOUT_ID_OFFSET = 4;"));
        assert!(accounts.contains("data[LAYOUT_ID_OFFSET + i]"));
        assert!(types.contains("const LAYOUT_ID_OFFSET = 4;"));
        assert!(types.contains("data.slice(LAYOUT_ID_OFFSET, LAYOUT_ID_OFFSET + LAYOUT_ID_LENGTH)"));
        assert!(!types.contains("data.slice(2, 10)"));
    }

    #[test]
    fn ts_index_reexports_all() {
        let m = test_manifest();
        let output = TsIndex(&m).to_string();
        assert!(output.contains("export * from \"./types\";"));
        assert!(output.contains("export * from \"./accounts\";"));
        assert!(output.contains("export * from \"./instructions\";"));
        assert!(output.contains("export * from \"./events\";"));
    }

    #[test]
    fn ts_full_client_gen_has_all_sections() {
        let m = test_manifest();
        let output = TsClientGen(&m).to_string();
        assert!(output.contains("=== types.ts ==="));
        assert!(output.contains("=== accounts.ts ==="));
        assert!(output.contains("=== instructions.ts ==="));
        assert!(output.contains("=== events.ts ==="));
        assert!(output.contains("=== index.ts ==="));
    }

    #[test]
    fn ts_accounts_emits_layout_id_constants_and_assertion_helpers() {
        let m = test_manifest();
        let output = TsAccounts(&m).to_string();
        // Generic helper and offset constants are always present.
        assert!(output.contains("export const LAYOUT_ID_OFFSET = 4;"));
        assert!(output.contains("export const LAYOUT_ID_LENGTH = 8;"));
        assert!(output.contains(
            "export function assertLayoutId(data: Uint8Array, expectedHex: string): void"
        ));
        // Per-layout const + assertion with the real 16-hex-char id.
        assert!(output.contains("export const VAULT_LAYOUT_ID = \"aabbccdd11223344\";"));
        assert!(output.contains("export function assertVaultLayout(data: Uint8Array): void"));
        assert!(output.contains("assertLayoutId(data, VAULT_LAYOUT_ID);"));
    }

    #[test]
    fn ts_accounts_uses_manifest_fingerprint_for_compact_layouts() {
        let m = compact_manifest();
        let output = TsAccounts(&m).to_string();
        assert!(output.contains(
            "export const COMPACT_VAULT_ACCOUNT_ENCODING: AccountEncoding = \"compact\";"
        ));
        assert!(output.contains("export const COMPACT_VAULT_LAYOUT_ID = \"1020304050607080\";"));
        assert!(output.contains("export const COMPACT_VAULT_SIZE = 41;"));
        assert!(output.contains("export const COMPACT_VAULT_DISC = 7;"));
        assert!(output.contains("export const COMPACT_VAULT_LAYOUT: LayoutIdentity = {"));
        assert!(output.contains("assertCompactLayout(data, COMPACT_VAULT_LAYOUT);"));
        assert!(output.contains("layoutFingerprint(layout)"));
        assert!(output.contains("new PublicKey(data.slice(1, 33))"));
        assert!(output.contains("view.getBigUint64(33, true)"));
        assert!(!output.contains("assertLayoutId(data, COMPACT_VAULT_LAYOUT_ID);"));
    }

    #[test]
    fn ts_assert_layout_id_handles_short_buffer_check() {
        let m = test_manifest();
        let output = TsAccounts(&m).to_string();
        // The guard explicitly rejects buffers shorter than the
        // header so callers cannot accidentally trust a truncated
        // slice.
        assert!(output.contains("if (data.length < HEADER_SIZE)"));
        assert!(output.contains("throw new Error"));
    }

    #[test]
    fn ts_data_size_calculation() {
        static ARGS: &[ArgDescriptor] = &[
            ArgDescriptor {
                name: "amount",
                canonical_type: "u64",
                size: 8,
                encoding: crate::ArgEncoding::Fixed,
            },
            ArgDescriptor {
                name: "bump",
                canonical_type: "u8",
                size: 1,
                encoding: crate::ArgEncoding::Fixed,
            },
        ];
        let ix = InstructionDescriptor {
            name: "test",
            tag: 0,
            args: ARGS,
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
        };
        // 1 (disc) + 8 (u64) + 1 (u8) = 10
        assert_eq!(instruction_data_size(&ix), 10);
    }

    #[test]
    fn ts_bounded_vector_and_remaining_accounts_are_encoded_exactly() {
        static ARGS: &[ArgDescriptor] = &[
            ArgDescriptor {
                name: "route_data",
                canonical_type: "HopperVec<u8,MAX_ROUTE_DATA>",
                size: 514,
                encoding: crate::ArgEncoding::BoundedVec {
                    max_len: 512,
                    element_size: 1,
                },
            },
            ArgDescriptor {
                name: "route_meta_flags",
                canonical_type: "[u8;MAX_ROUTE_ACCOUNTS]",
                size: 32,
                encoding: crate::ArgEncoding::Fixed,
            },
        ];
        static IX: &[InstructionDescriptor] = &[InstructionDescriptor {
            name: "execute_intent",
            tag: 6,
            args: ARGS,
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
        }];
        let mut manifest = test_manifest();
        manifest.instructions = IX;
        let output = TsInstructions(&manifest).to_string();

        assert!(output.contains("remainingAccounts: readonly AccountMeta[] = []"));
        assert!(output.contains("remainingAccounts.length > 32"));
        assert!(output.contains("keys.push(...remainingAccounts)"));
        assert!(output.contains("args.routeData.length > 512"));
        assert!(output.contains("1 + 2 + args.routeData.length * 1 + 32"));
        assert!(output.contains("view.setUint16(offset, args.routeData.length, true)"));
        assert!(output.contains("data.set(args.routeMetaFlags, offset)"));
    }

    // -- BLD-WR: strict_writes account demotion in the TS builder --

    // Three caller-provided (non-PDA) accounts so each appears verbatim in the
    // generated `keys` array: authority (read-only signer), vault (written),
    // config (Sealevel-writable but with no declared byte range).
    static WR_ACCTS: &[AccountEntry] = &[
        AccountEntry {
            name: "authority",
            writable: false,
            signer: true,
            layout_ref: "",
            seeds: &[],
        },
        AccountEntry {
            name: "vault",
            writable: true,
            signer: false,
            layout_ref: "",
            seeds: &[],
        },
        AccountEntry {
            name: "config",
            writable: true,
            signer: false,
            layout_ref: "",
            seeds: &[],
        },
    ];
    static WR_RANGES: &[crate::WriteRange] = &[crate::WriteRange::new(1, 16, 8)];

    fn wr_manifest(strict: bool) -> ProgramManifest {
        static STRICT_IX: &[InstructionDescriptor] = &[InstructionDescriptor {
            name: "deposit",
            tag: 0,
            args: &[],
            accounts: WR_ACCTS,
            remaining_accounts: None,
            capabilities: &[],
            policy_pack: "",
            receipt_expected: false,
            strict_writes: true,
            write_ranges: WR_RANGES,
            parametric_write_ranges: &[],
            mutation_complete: false,
            lamport_accounts: &[],
            cu_estimate: 0,
        }];
        static LOOSE_IX: &[InstructionDescriptor] = &[InstructionDescriptor {
            name: "deposit",
            tag: 0,
            args: &[],
            accounts: WR_ACCTS,
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
        ProgramManifest {
            name: "vault_prog",
            version: "1.0.0",
            description: "",
            layouts: &[],
            layout_metadata: &[],
            instructions: if strict { STRICT_IX } else { LOOSE_IX },
            events: &[],
            policies: &[],
            compatibility_pairs: &[],
            tooling_hints: &[],
            contexts: &[],
        }
    }

    #[test]
    fn ts_strict_writes_preserves_declared_writability() {
        // Demotion on data-range absence is unsound (lamport-writable
        // accounts carry no data range), so the client preserves the
        // declared flag: a Sealevel-writable account stays writable even
        // under a strict_writes instruction that declares no range for it.
        let out = TsInstructions(&wr_manifest(true)).to_string();
        assert!(out.contains("pubkey: accounts.vault, isSigner: false, isWritable: true"));
        assert!(out.contains("pubkey: accounts.config, isSigner: false, isWritable: true"));
    }

    #[test]
    fn ts_non_strict_instruction_keeps_declared_writable() {
        // Identical accounts, but a non-strict instruction: config keeps its
        // declared writable flag (current, pre-BLD-WR behavior).
        let out = TsInstructions(&wr_manifest(false)).to_string();
        assert!(out.contains("pubkey: accounts.config, isSigner: false, isWritable: true"));
    }

    // -- BLD-CU: compute-budget companion generation --

    fn cu_manifest(estimate: u32) -> ProgramManifest {
        static CU_IX_SET: &[InstructionDescriptor] = &[InstructionDescriptor {
            name: "deposit",
            tag: 0,
            args: &[],
            accounts: WR_ACCTS,
            remaining_accounts: None,
            capabilities: &[],
            policy_pack: "",
            receipt_expected: false,
            strict_writes: false,
            write_ranges: &[],
            parametric_write_ranges: &[],
            mutation_complete: false,
            lamport_accounts: &[],
            cu_estimate: 10_000,
        }];
        static CU_IX_UNSET: &[InstructionDescriptor] = &[InstructionDescriptor {
            name: "deposit",
            tag: 0,
            args: &[],
            accounts: WR_ACCTS,
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
        ProgramManifest {
            name: "vault_prog",
            version: "1.0.0",
            description: "",
            layouts: &[],
            layout_metadata: &[],
            instructions: if estimate > 0 { CU_IX_SET } else { CU_IX_UNSET },
            events: &[],
            policies: &[],
            compatibility_pairs: &[],
            tooling_hints: &[],
            contexts: &[],
        }
    }

    #[test]
    fn ts_cu_estimate_generates_budget_companion() {
        let out = TsInstructions(&cu_manifest(10_000)).to_string();
        // The raw measured bound is exported verbatim...
        assert!(out.contains("export const DEPOSIT_CU_ESTIMATE = 10000;"));
        // ...and the companion prepends SetComputeUnitLimit(estimate + 10%).
        assert!(out.contains("export function createDepositInstructionWithBudget("));
        assert!(out.contains("budgetData[0] = 2; // SetComputeUnitLimit"));
        assert!(out.contains("new DataView(budgetData.buffer).setUint32(1, 11000, true);"));
        assert!(out.contains("ComputeBudget111111111111111111111111111111"));
        assert!(out.contains("return [budgetIx, createDepositInstruction(accounts, programId)];"));
        // The plain builder is still generated unchanged.
        assert!(out.contains("export function createDepositInstruction("));
    }

    #[test]
    fn ts_unknown_cu_estimate_emits_no_budget_code() {
        // 0 = unmeasured: output must be byte-identical to pre-BLD-CU
        // behavior — no constant, no companion, no compute-budget program id.
        let out = TsInstructions(&cu_manifest(0)).to_string();
        assert!(!out.contains("CU_ESTIMATE"));
        assert!(!out.contains("WithBudget"));
        assert!(!out.contains("ComputeBudget111111111111111111111111111111"));
    }

    // -- Kotlin generator tests --

    #[test]
    fn kt_accounts_generates_data_class() {
        let m = test_manifest();
        let output = KtAccounts(&m).to_string();
        assert!(output.contains("data class Vault("));
        assert!(output.contains("val authority: PublicKey"));
        assert!(output.contains("val amount: ULong"));
        assert!(output.contains("val isActive: Boolean"));
    }

    #[test]
    fn kt_accounts_generates_decoder() {
        let m = test_manifest();
        let output = KtAccounts(&m).to_string();
        assert!(output.contains("fun decodeVault(data: ByteArray): Vault {"));
        assert!(output.contains("PublicKey(data.copyOfRange(16, 48))"));
        assert!(output.contains("ByteBuffer.wrap(data, 48, 8)"));
        assert!(output.contains("data[56] != 0.toByte()"));
    }

    #[test]
    fn kt_accounts_generates_discriminator() {
        let m = test_manifest();
        let output = KtAccounts(&m).to_string();
        assert!(output.contains("const val VAULT_DISC: Byte = 1"));
    }

    #[test]
    fn kt_accounts_emits_layout_id_constants_and_assertion_helpers() {
        let m = test_manifest();
        let output = KtAccounts(&m).to_string();
        // Generic helper and offset constants are always present.
        assert!(output.contains("const val HEADER_SIZE: Int = 16"));
        assert!(output.contains("const val LAYOUT_ID_OFFSET: Int = 4"));
        assert!(output.contains("const val LAYOUT_ID_LENGTH: Int = 8"));
        assert!(output.contains("fun assertLayoutId(data: ByteArray, expectedHex: String) {"));
        // Per-layout const + assertion with the real 16-hex-char id.
        assert!(output.contains("const val VAULT_LAYOUT_ID: String = \"aabbccdd11223344\""));
        assert!(output.contains("fun assertVaultLayout(data: ByteArray) {"));
        assert!(output.contains("assertLayoutId(data, VAULT_LAYOUT_ID)"));
        // Decoder calls the assertion first so a mismatched account
        // fails with LayoutMismatchException instead of decoding garbage.
        assert!(output
            .contains("fun decodeVault(data: ByteArray): Vault {\n    assertVaultLayout(data)"));
    }

    #[test]
    fn kt_accounts_uses_manifest_fingerprint_for_compact_layouts() {
        let m = compact_manifest();
        let output = KtAccounts(&m).to_string();
        assert!(output.contains(
            "val COMPACT_VAULT_ACCOUNT_ENCODING: AccountEncoding = AccountEncoding.Compact"
        ));
        assert!(output.contains("const val COMPACT_VAULT_LAYOUT_ID: String = \"1020304050607080\""));
        assert!(output.contains("const val COMPACT_VAULT_SIZE: Int = 41"));
        assert!(output.contains("const val COMPACT_VAULT_DISC: Byte = 7"));
        assert!(output.contains("val COMPACT_VAULT_LAYOUT: LayoutIdentity = LayoutIdentity("));
        assert!(output.contains("assertCompactLayout(data, COMPACT_VAULT_LAYOUT)"));
        assert!(output
            .contains("fun layoutFingerprint(layout: LayoutIdentity): String = layout.layoutId"));
        assert!(output.contains("PublicKey(data.copyOfRange(1, 33))"));
        assert!(output.contains("ByteBuffer.wrap(data, 33, 8)"));
        assert!(!output.contains("assertLayoutId(data, COMPACT_VAULT_LAYOUT_ID)"));
    }

    #[test]
    fn kt_instructions_generates_builder() {
        let m = test_manifest();
        let output = KtInstructions(&m).to_string();
        assert!(output.contains("data class DepositArgs("));
        assert!(output.contains("data class DepositAccounts("));
        assert!(output.contains("fun createDepositInstruction("));
        assert!(output.contains("data[0] = 0.toByte() // instruction discriminator"));
    }

    #[test]
    fn kt_instructions_account_meta() {
        let m = test_manifest();
        let output = KtInstructions(&m).to_string();
        assert!(output.contains("isSigner = true, isWritable = false"));
        assert!(output.contains("isSigner = false, isWritable = true"));
    }

    #[test]
    fn kt_instructions_auto_derives_pda_accounts() {
        let m = test_manifest();
        let output = KtInstructions(&m).to_string();

        // The PDA account is dropped from the caller-facing data class...
        assert!(output.contains("data class DepositAccounts("));
        assert!(output.contains("val authority: PublicKey"));
        assert!(!output.contains("val vault: PublicKey"));

        // ...and derived in the builder from its on-chain seeds via sol4k.
        assert!(output.contains("val vault = PublicKey.findProgramAddress("));
        assert!(output.contains("\"vault\".toByteArray()"));
        assert!(output.contains("accounts.authority.bytes()"));
        assert!(output.contains(").address"));

        // The PDA key uses the derived local, not accounts.vault.
        assert!(output.contains("AccountMeta(vault, isSigner = false, isWritable = true)"));
        assert!(!output.contains("accounts.vault"));
    }

    #[test]
    fn kt_events_generates_decoder() {
        let m = test_manifest();
        let output = KtEvents(&m).to_string();
        assert!(output.contains("data class DepositEventEvent("));
        assert!(output.contains("fun decodeDepositEventEvent(data: ByteArray)"));
        assert!(output.contains("DEPOSIT_EVENT_EVENT_DISC: Byte = 0"));
        assert!(output.contains("DEPOSIT_EVENT_EVENT_DATA_LEN: Int = 41"));
        assert!(output.contains("data[0] == DEPOSIT_EVENT_EVENT_DISC"));
        assert!(output.contains("PublicKey(data.copyOfRange(1, 33))"));
        assert!(output.contains("ByteBuffer.wrap(data, 33, 8)"));
    }

    #[test]
    fn kt_types_generates_header() {
        let m = test_manifest();
        let output = KtTypes(&m).to_string();
        assert!(output.contains("data class HopperHeader("));
        assert!(output.contains("fun decodeHeader(data: ByteArray): HopperHeader {"));
        assert!(output.contains(
            "flags = ByteBuffer.wrap(data, 2, 2).order(ByteOrder.LITTLE_ENDIAN).short.toUShort(),"
        ));
        assert!(output.contains("layoutId = data.copyOfRange(TYPES_LAYOUT_ID_OFFSET, TYPES_LAYOUT_ID_OFFSET + TYPES_LAYOUT_ID_LENGTH),"));
        assert!(output.contains("schemaEpoch = ByteBuffer.wrap(data, 12, 4).order(ByteOrder.LITTLE_ENDIAN).int.toUInt()"));
        assert!(!output.contains("data.copyOfRange(2, 10)"));
        assert!(output.contains("VAULT: Byte = 1"));
    }

    #[test]
    fn kt_full_client_gen_has_all_sections() {
        let m = test_manifest();
        let output = KtClientGen(&m).to_string();
        assert!(output.contains("=== Types.kt ==="));
        assert!(output.contains("=== Accounts.kt ==="));
        assert!(output.contains("=== Instructions.kt ==="));
        assert!(output.contains("=== Events.kt ==="));
    }
}
