//! # Codama Projection Module
//!
//! JSON formatting for the three-tier schema export:
//!
//! ```text
//! ProgramManifest  ⊃  ProgramIdl  ⊃  CodamaProjection
//!       (rich)         (public)         (interop)
//! ```
//!
//! All formatters use `core::fmt::Write` so they work in `#![no_std]`.

use core::fmt;

use crate::{
    ArgDescriptor, CodamaProjection, CompatibilityPair, DescriptorMetadata, EventDescriptor,
    FieldDescriptor, IdlAccountEntry, InstructionDescriptor, LayoutFingerprint, LayoutKind,
    LayoutManifest, MigrationPolicy, PdaSeedHint, PolicyDescriptor, ProgramIdl, ProgramManifest,
};

// ---------------------------------------------------------------------------
// Shared JSON helpers
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

fn write_hex_json(f: &mut fmt::Formatter<'_>, bytes: &[u8]) -> fmt::Result {
    write!(f, "\"")?;
    for b in bytes {
        write!(f, "{:02x}", b)?;
    }
    write!(f, "\"")
}

fn write_indent(f: &mut fmt::Formatter<'_>, level: usize) -> fmt::Result {
    for _ in 0..level {
        write!(f, "  ")?;
    }
    Ok(())
}

fn write_str_array(f: &mut fmt::Formatter<'_>, items: &[&str], indent: usize) -> fmt::Result {
    if items.is_empty() {
        return write!(f, "[]");
    }
    writeln!(f, "[")?;
    for (i, s) in items.iter().enumerate() {
        write_indent(f, indent + 1)?;
        write_json_str(f, s)?;
        if i + 1 < items.len() {
            writeln!(f, ",")?;
        } else {
            writeln!(f)?;
        }
    }
    write_indent(f, indent)?;
    write!(f, "]")
}

// ---------------------------------------------------------------------------
// FieldDescriptor JSON
// ---------------------------------------------------------------------------

fn write_field_json(
    f: &mut fmt::Formatter<'_>,
    field: &FieldDescriptor,
    indent: usize,
) -> fmt::Result {
    write_indent(f, indent)?;
    write!(f, "{{ \"name\": ")?;
    write_json_str(f, field.name)?;
    write!(f, ", \"type\": ")?;
    write_json_str(f, field.canonical_type)?;
    write!(
        f,
        ", \"size\": {}, \"offset\": {}",
        field.size, field.offset
    )?;
    write!(f, ", \"intent\": ")?;
    write_json_str(f, field.intent.name())?;
    write!(f, " }}")
}

fn write_fields_json(
    f: &mut fmt::Formatter<'_>,
    fields: &[FieldDescriptor],
    indent: usize,
) -> fmt::Result {
    if fields.is_empty() {
        return write!(f, "[]");
    }
    writeln!(f, "[")?;
    for (i, field) in fields.iter().enumerate() {
        write_field_json(f, field, indent + 1)?;
        if i + 1 < fields.len() {
            writeln!(f, ",")?;
        } else {
            writeln!(f)?;
        }
    }
    write_indent(f, indent)?;
    write!(f, "]")
}

// ---------------------------------------------------------------------------
// ArgDescriptor JSON
// ---------------------------------------------------------------------------

fn write_args_json(
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
        write_json_str(f, arg.canonical_type)?;
        write!(f, ", \"size\": {} }}", arg.size)?;
        if i + 1 < args.len() {
            writeln!(f, ",")?;
        } else {
            writeln!(f)?;
        }
    }
    write_indent(f, indent)?;
    write!(f, "]")
}

// ---------------------------------------------------------------------------
// CodamaProjection JSON
// ---------------------------------------------------------------------------

fn write_idl_account_json(
    f: &mut fmt::Formatter<'_>,
    a: &IdlAccountEntry,
    indent: usize,
) -> fmt::Result {
    write_indent(f, indent)?;
    writeln!(f, "{{")?;
    write_indent(f, indent + 1)?;
    write!(f, "\"name\": ")?;
    write_json_str(f, a.name)?;
    writeln!(f, ",")?;
    write_indent(f, indent + 1)?;
    writeln!(f, "\"writable\": {},", a.writable)?;
    write_indent(f, indent + 1)?;
    write!(f, "\"signer\": {}", a.signer)?;
    if !a.layout_ref.is_empty() {
        writeln!(f, ",")?;
        write_indent(f, indent + 1)?;
        write!(f, "\"layoutRef\": ")?;
        write_json_str(f, a.layout_ref)?;
    }
    if !a.pda_seeds.is_empty() {
        writeln!(f, ",")?;
        write_indent(f, indent + 1)?;
        write!(f, "\"pdaSeeds\": ")?;
        write_pda_seeds_json(f, a.pda_seeds, indent + 1)?;
    }
    writeln!(f)?;
    write_indent(f, indent)?;
    write!(f, "}}")
}

fn write_pda_seeds_json(
    f: &mut fmt::Formatter<'_>,
    seeds: &[PdaSeedHint],
    indent: usize,
) -> fmt::Result {
    writeln!(f, "[")?;
    for (i, seed) in seeds.iter().enumerate() {
        write_indent(f, indent + 1)?;
        write!(f, "{{ \"kind\": ")?;
        write_json_str(f, seed.kind)?;
        write!(f, ", \"value\": ")?;
        write_json_str(f, seed.value)?;
        write!(f, " }}")?;
        if i + 1 < seeds.len() {
            writeln!(f, ",")?;
        } else {
            writeln!(f)?;
        }
    }
    write_indent(f, indent)?;
    write!(f, "]")
}

/// Wrapper for JSON formatting of `CodamaProjection`.
pub struct CodamaJson<'a>(pub &'a CodamaProjection);

impl<'a> fmt::Display for CodamaJson<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let p = self.0;
        writeln!(f, "{{")?;
        write!(f, "  \"name\": ")?;
        write_json_str(f, p.name)?;
        writeln!(f, ",")?;
        write!(f, "  \"version\": ")?;
        write_json_str(f, p.version)?;
        writeln!(f, ",")?;

        // Instructions
        write!(f, "  \"instructions\": ")?;
        if p.instructions.is_empty() {
            writeln!(f, "[],")?;
        } else {
            writeln!(f, "[")?;
            for (i, ix) in p.instructions.iter().enumerate() {
                write_indent(f, 2)?;
                writeln!(f, "{{")?;
                write_indent(f, 3)?;
                write!(f, "\"name\": ")?;
                write_json_str(f, ix.name)?;
                writeln!(f, ",")?;
                write_indent(f, 3)?;
                writeln!(f, "\"discriminator\": {},", ix.discriminator)?;
                write_indent(f, 3)?;
                write!(f, "\"args\": ")?;
                write_args_json(f, ix.args, 3)?;
                writeln!(f, ",")?;
                write_indent(f, 3)?;
                write!(f, "\"accounts\": ")?;
                if ix.accounts.is_empty() {
                    write!(f, "[]")?;
                } else {
                    writeln!(f, "[")?;
                    for (j, a) in ix.accounts.iter().enumerate() {
                        write_idl_account_json(f, a, 4)?;
                        if j + 1 < ix.accounts.len() {
                            writeln!(f, ",")?;
                        } else {
                            writeln!(f)?;
                        }
                    }
                    write_indent(f, 3)?;
                    write!(f, "]")?;
                }
                writeln!(f)?;
                write_indent(f, 2)?;
                write!(f, "}}")?;
                if i + 1 < p.instructions.len() {
                    writeln!(f, ",")?;
                } else {
                    writeln!(f)?;
                }
            }
            writeln!(f, "  ],")?;
        }

        // Accounts
        write!(f, "  \"accounts\": ")?;
        if p.accounts.is_empty() {
            writeln!(f, "[],")?;
        } else {
            writeln!(f, "[")?;
            for (i, a) in p.accounts.iter().enumerate() {
                write_indent(f, 2)?;
                writeln!(f, "{{")?;
                write_indent(f, 3)?;
                write!(f, "\"name\": ")?;
                write_json_str(f, a.name)?;
                writeln!(f, ",")?;
                write_indent(f, 3)?;
                writeln!(f, "\"discriminator\": {},", a.discriminator)?;
                write_indent(f, 3)?;
                writeln!(f, "\"size\": {},", a.size)?;
                write_indent(f, 3)?;
                write!(f, "\"fields\": ")?;
                write_fields_json(f, a.fields, 3)?;
                writeln!(f)?;
                write_indent(f, 2)?;
                write!(f, "}}")?;
                if i + 1 < p.accounts.len() {
                    writeln!(f, ",")?;
                } else {
                    writeln!(f)?;
                }
            }
            writeln!(f, "  ],")?;
        }

        // Events
        write!(f, "  \"events\": ")?;
        if p.events.is_empty() {
            writeln!(f, "[]")?;
        } else {
            writeln!(f, "[")?;
            for (i, e) in p.events.iter().enumerate() {
                write_indent(f, 2)?;
                writeln!(f, "{{")?;
                write_indent(f, 3)?;
                write!(f, "\"name\": ")?;
                write_json_str(f, e.name)?;
                writeln!(f, ",")?;
                write_indent(f, 3)?;
                writeln!(f, "\"discriminator\": {},", e.discriminator)?;
                write_indent(f, 3)?;
                write!(f, "\"fields\": ")?;
                write_fields_json(f, e.fields, 3)?;
                writeln!(f)?;
                write_indent(f, 2)?;
                write!(f, "}}")?;
                if i + 1 < p.events.len() {
                    writeln!(f, ",")?;
                } else {
                    writeln!(f)?;
                }
            }
            writeln!(f, "  ]")?;
        }

        write!(f, "}}")
    }
}

// ---------------------------------------------------------------------------
// ProgramIdl JSON
// ---------------------------------------------------------------------------

/// Wrapper for JSON formatting of `ProgramIdl`.
pub struct IdlJson<'a>(pub &'a ProgramIdl);

impl<'a> fmt::Display for IdlJson<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let p = self.0;
        writeln!(f, "{{")?;
        write!(f, "  \"name\": ")?;
        write_json_str(f, p.name)?;
        writeln!(f, ",")?;
        write!(f, "  \"version\": ")?;
        write_json_str(f, p.version)?;
        writeln!(f, ",")?;
        write!(f, "  \"description\": ")?;
        write_json_str(f, p.description)?;
        writeln!(f, ",")?;

        // Instructions
        write!(f, "  \"instructions\": ")?;
        if p.instructions.is_empty() {
            writeln!(f, "[],")?;
        } else {
            writeln!(f, "[")?;
            for (i, ix) in p.instructions.iter().enumerate() {
                write_indent(f, 2)?;
                writeln!(f, "{{")?;
                write_indent(f, 3)?;
                write!(f, "\"name\": ")?;
                write_json_str(f, ix.name)?;
                writeln!(f, ",")?;
                write_indent(f, 3)?;
                writeln!(f, "\"tag\": {},", ix.tag)?;
                write_indent(f, 3)?;
                write!(f, "\"args\": ")?;
                write_args_json(f, ix.args, 3)?;
                writeln!(f, ",")?;
                write_indent(f, 3)?;
                write!(f, "\"accounts\": ")?;
                if ix.accounts.is_empty() {
                    write!(f, "[]")?;
                } else {
                    writeln!(f, "[")?;
                    for (j, a) in ix.accounts.iter().enumerate() {
                        write_idl_account_json(f, a, 4)?;
                        if j + 1 < ix.accounts.len() {
                            writeln!(f, ",")?;
                        } else {
                            writeln!(f)?;
                        }
                    }
                    write_indent(f, 3)?;
                    write!(f, "]")?;
                }
                writeln!(f)?;
                write_indent(f, 2)?;
                write!(f, "}}")?;
                if i + 1 < p.instructions.len() {
                    writeln!(f, ",")?;
                } else {
                    writeln!(f)?;
                }
            }
            writeln!(f, "  ],")?;
        }

        // Accounts
        write!(f, "  \"accounts\": ")?;
        if p.accounts.is_empty() {
            writeln!(f, "[],")?;
        } else {
            writeln!(f, "[")?;
            for (i, a) in p.accounts.iter().enumerate() {
                write_indent(f, 2)?;
                writeln!(f, "{{")?;
                write_indent(f, 3)?;
                write!(f, "\"name\": ")?;
                write_json_str(f, a.name)?;
                writeln!(f, ",")?;
                write_indent(f, 3)?;
                writeln!(f, "\"disc\": {},", a.disc)?;
                write_indent(f, 3)?;
                writeln!(f, "\"version\": {},", a.version)?;
                write_indent(f, 3)?;
                write!(f, "\"layoutId\": ")?;
                write_hex_json(f, &a.layout_id)?;
                writeln!(f, ",")?;
                write_indent(f, 3)?;
                writeln!(f, "\"totalSize\": {},", a.total_size)?;
                write_indent(f, 3)?;
                write!(f, "\"fields\": ")?;
                write_fields_json(f, a.fields, 3)?;
                writeln!(f)?;
                write_indent(f, 2)?;
                write!(f, "}}")?;
                if i + 1 < p.accounts.len() {
                    writeln!(f, ",")?;
                } else {
                    writeln!(f)?;
                }
            }
            writeln!(f, "  ],")?;
        }

        // Events
        write!(f, "  \"events\": ")?;
        if p.events.is_empty() {
            writeln!(f, "[],")?;
        } else {
            writeln!(f, "[")?;
            for (i, e) in p.events.iter().enumerate() {
                write_indent(f, 2)?;
                writeln!(f, "{{")?;
                write_indent(f, 3)?;
                write!(f, "\"name\": ")?;
                write_json_str(f, e.name)?;
                writeln!(f, ",")?;
                write_indent(f, 3)?;
                writeln!(f, "\"tag\": {},", e.tag)?;
                write_indent(f, 3)?;
                write!(f, "\"fields\": ")?;
                write_fields_json(f, e.fields, 3)?;
                writeln!(f)?;
                write_indent(f, 2)?;
                write!(f, "}}")?;
                if i + 1 < p.events.len() {
                    writeln!(f, ",")?;
                } else {
                    writeln!(f)?;
                }
            }
            writeln!(f, "  ],")?;
        }

        // Fingerprints
        write!(f, "  \"fingerprints\": ")?;
        if p.fingerprints.is_empty() {
            writeln!(f, "[]")?;
        } else {
            writeln!(f, "[")?;
            for (i, (fp, name)) in p.fingerprints.iter().enumerate() {
                write_indent(f, 2)?;
                write!(f, "{{ \"layoutId\": ")?;
                write_hex_json(f, fp)?;
                write!(f, ", \"name\": ")?;
                write_json_str(f, name)?;
                write!(f, " }}")?;
                if i + 1 < p.fingerprints.len() {
                    writeln!(f, ",")?;
                } else {
                    writeln!(f)?;
                }
            }
            writeln!(f, "  ]")?;
        }

        write!(f, "}}")
    }
}

// ---------------------------------------------------------------------------
// ProgramManifest JSON
// ---------------------------------------------------------------------------

/// Wrapper for JSON formatting of `ProgramManifest`.
pub struct ManifestJson<'a>(pub &'a ProgramManifest);

impl<'a> fmt::Display for ManifestJson<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let p = self.0;
        writeln!(f, "{{")?;
        write!(f, "  \"name\": ")?;
        write_json_str(f, p.name)?;
        writeln!(f, ",")?;
        write!(f, "  \"version\": ")?;
        write_json_str(f, p.version)?;
        writeln!(f, ",")?;
        write!(f, "  \"description\": ")?;
        write_json_str(f, p.description)?;
        writeln!(f, ",")?;

        // Layouts
        write!(f, "  \"layouts\": ")?;
        write_layout_array(f, p.layouts)?;
        writeln!(f, ",")?;

        // Instructions
        write!(f, "  \"instructions\": ")?;
        write_instruction_array(f, p.instructions)?;
        writeln!(f, ",")?;

        // Events
        write!(f, "  \"events\": ")?;
        write_event_array(f, p.events)?;
        writeln!(f, ",")?;

        // Policies
        write!(f, "  \"policies\": ")?;
        write_policy_array(f, p.policies)?;
        writeln!(f, ",")?;

        // Compatibility rules
        write!(f, "  \"compatRules\": ")?;
        write_compat_pair_array(f, p.compatibility_pairs)?;
        writeln!(f, ",")?;

        // Receipt wire schema
        write!(f, "  \"receiptSchema\": ")?;
        write_receipt_schema(f)?;
        writeln!(f, ",")?;

        // Tooling hints
        write!(f, "  \"toolingHints\": ")?;
        write_str_array(f, p.tooling_hints, 1)?;
        writeln!(f)?;

        write!(f, "}}")
    }
}

fn write_layout_array(f: &mut fmt::Formatter<'_>, layouts: &[LayoutManifest]) -> fmt::Result {
    if layouts.is_empty() {
        return write!(f, "[]");
    }
    writeln!(f, "[")?;
    for (i, l) in layouts.iter().enumerate() {
        write_indent(f, 2)?;
        writeln!(f, "{{")?;
        write_indent(f, 3)?;
        write!(f, "\"name\": ")?;
        write_json_str(f, l.name)?;
        writeln!(f, ",")?;
        write_indent(f, 3)?;
        writeln!(f, "\"disc\": {},", l.disc)?;
        write_indent(f, 3)?;
        writeln!(f, "\"version\": {},", l.version)?;
        write_indent(f, 3)?;
        write!(f, "\"layoutId\": ")?;
        write_hex_json(f, &l.layout_id)?;
        writeln!(f, ",")?;
        write_indent(f, 3)?;
        writeln!(f, "\"totalSize\": {},", l.total_size)?;
        write_indent(f, 3)?;
        writeln!(f, "\"fieldCount\": {},", l.field_count)?;
        write_indent(f, 3)?;
        write!(f, "\"fields\": ")?;
        write_fields_json(f, l.fields, 3)?;
        writeln!(f, ",")?;
        // Semantic fingerprint (v2)
        let fp = LayoutFingerprint::from_manifest(l);
        write_indent(f, 3)?;
        write!(f, "\"semanticFingerprint\": ")?;
        write_hex_json(f, &fp.semantic_hash)?;
        writeln!(f)?;
        write_indent(f, 2)?;
        write!(f, "}}")?;
        if i + 1 < layouts.len() {
            writeln!(f, ",")?;
        } else {
            writeln!(f)?;
        }
    }
    write_indent(f, 1)?;
    write!(f, "]")
}

fn write_instruction_array(
    f: &mut fmt::Formatter<'_>,
    instrs: &[InstructionDescriptor],
) -> fmt::Result {
    if instrs.is_empty() {
        return write!(f, "[]");
    }
    writeln!(f, "[")?;
    for (i, ix) in instrs.iter().enumerate() {
        write_indent(f, 2)?;
        writeln!(f, "{{")?;
        write_indent(f, 3)?;
        write!(f, "\"name\": ")?;
        write_json_str(f, ix.name)?;
        writeln!(f, ",")?;
        write_indent(f, 3)?;
        writeln!(f, "\"tag\": {},", ix.tag)?;
        write_indent(f, 3)?;
        write!(f, "\"args\": ")?;
        write_args_json(f, ix.args, 3)?;
        writeln!(f, ",")?;
        write_indent(f, 3)?;
        write!(f, "\"accounts\": ")?;
        write_account_entry_array(f, ix.accounts, 3)?;
        writeln!(f, ",")?;
        write_indent(f, 3)?;
        write!(f, "\"capabilities\": ")?;
        write_str_array(f, ix.capabilities, 3)?;
        writeln!(f, ",")?;
        write_indent(f, 3)?;
        write!(f, "\"policyPack\": ")?;
        write_json_str(f, ix.policy_pack)?;
        writeln!(f, ",")?;
        write_indent(f, 3)?;
        writeln!(f, "\"receiptExpected\": {},", ix.receipt_expected)?;
        // Measured CU upper bound (SIMD-553 play: tight client compute
        // budgets are the cheapest transactions). Emitted only when the
        // author published an estimate; 0 means unknown and the key is
        // omitted entirely so consumers never mistake "unmeasured" for
        // "costs zero CU".
        if ix.cu_estimate > 0 {
            write_indent(f, 3)?;
            writeln!(f, "\"cuEstimate\": {},", ix.cu_estimate)?;
        }
        write_indent(f, 3)?;
        writeln!(f, "\"strictWrites\": {},", ix.strict_writes)?;
        write_indent(f, 3)?;
        write!(f, "\"writeRanges\": ")?;
        write_write_ranges_json(f, ix, 3)?;
        writeln!(f)?;
        write_indent(f, 2)?;
        write!(f, "}}")?;
        if i + 1 < instrs.len() {
            writeln!(f, ",")?;
        } else {
            writeln!(f)?;
        }
    }
    write_indent(f, 1)?;
    write!(f, "]")
}

fn write_account_entry_array(
    f: &mut fmt::Formatter<'_>,
    accounts: &[crate::AccountEntry],
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
            ", \"writable\": {}, \"signer\": {}",
            a.writable, a.signer
        )?;
        if !a.layout_ref.is_empty() {
            write!(f, ", \"layoutRef\": ")?;
            write_json_str(f, a.layout_ref)?;
        }
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

/// Emit an instruction's declared byte-range write-set as a JSON array.
///
/// Each entry is `{ "account": "<name>", "accountIndex": N, "offset": N,
/// "size": N }`. `accountIndex` is the [`WriteRange::account_index`] the
/// runtime `WritePolicy` enforces; `account` resolves that index to the
/// instruction's account name when in range (empty string otherwise, e.g. a
/// hand-authored manifest whose ranges out-run its account list). Offsets and
/// sizes are the exact values the `strict_writes` macro compiled, so tooling
/// sees the same byte surface the runtime enforces.
fn write_write_ranges_json(
    f: &mut fmt::Formatter<'_>,
    ix: &InstructionDescriptor,
    indent: usize,
) -> fmt::Result {
    let ranges = ix.write_ranges;
    if ranges.is_empty() {
        return write!(f, "[]");
    }
    writeln!(f, "[")?;
    for (i, r) in ranges.iter().enumerate() {
        let name = ix
            .accounts
            .get(r.account_index as usize)
            .map(|a| a.name)
            .unwrap_or("");
        write_indent(f, indent + 1)?;
        write!(f, "{{ \"account\": ")?;
        write_json_str(f, name)?;
        write!(
            f,
            ", \"accountIndex\": {}, \"offset\": {}, \"size\": {} }}",
            r.account_index, r.offset, r.size
        )?;
        if i + 1 < ranges.len() {
            writeln!(f, ",")?;
        } else {
            writeln!(f)?;
        }
    }
    write_indent(f, indent)?;
    write!(f, "]")
}

fn write_event_array(f: &mut fmt::Formatter<'_>, events: &[EventDescriptor]) -> fmt::Result {
    if events.is_empty() {
        return write!(f, "[]");
    }
    writeln!(f, "[")?;
    for (i, e) in events.iter().enumerate() {
        write_indent(f, 2)?;
        writeln!(f, "{{")?;
        write_indent(f, 3)?;
        write!(f, "\"name\": ")?;
        write_json_str(f, e.name)?;
        writeln!(f, ",")?;
        write_indent(f, 3)?;
        writeln!(f, "\"tag\": {},", e.tag)?;
        write_indent(f, 3)?;
        write!(f, "\"fields\": ")?;
        write_fields_json(f, e.fields, 3)?;
        writeln!(f)?;
        write_indent(f, 2)?;
        write!(f, "}}")?;
        if i + 1 < events.len() {
            writeln!(f, ",")?;
        } else {
            writeln!(f)?;
        }
    }
    write_indent(f, 1)?;
    write!(f, "]")
}

fn write_policy_array(f: &mut fmt::Formatter<'_>, policies: &[PolicyDescriptor]) -> fmt::Result {
    if policies.is_empty() {
        return write!(f, "[]");
    }
    writeln!(f, "[")?;
    for (i, p) in policies.iter().enumerate() {
        write_indent(f, 2)?;
        writeln!(f, "{{")?;
        write_indent(f, 3)?;
        write!(f, "\"name\": ")?;
        write_json_str(f, p.name)?;
        writeln!(f, ",")?;
        write_indent(f, 3)?;
        write!(f, "\"capabilities\": ")?;
        write_str_array(f, p.capabilities, 3)?;
        writeln!(f, ",")?;
        write_indent(f, 3)?;
        write!(f, "\"requirements\": ")?;
        write_str_array(f, p.requirements, 3)?;
        writeln!(f, ",")?;
        write_indent(f, 3)?;
        write!(f, "\"invariants\": ")?;
        write_str_array(f, p.invariants, 3)?;
        writeln!(f, ",")?;
        write_indent(f, 3)?;
        write!(f, "\"receiptProfile\": ")?;
        write_json_str(f, p.receipt_profile)?;
        writeln!(f)?;
        write_indent(f, 2)?;
        write!(f, "}}")?;
        if i + 1 < policies.len() {
            writeln!(f, ",")?;
        } else {
            writeln!(f)?;
        }
    }
    write_indent(f, 1)?;
    write!(f, "]")
}

fn write_compat_pair_array(f: &mut fmt::Formatter<'_>, pairs: &[CompatibilityPair]) -> fmt::Result {
    if pairs.is_empty() {
        return write!(f, "[]");
    }
    writeln!(f, "[")?;
    for (i, cp) in pairs.iter().enumerate() {
        write_indent(f, 2)?;
        writeln!(f, "{{")?;
        write_indent(f, 3)?;
        write!(f, "\"from\": ")?;
        write_json_str(f, cp.from_layout)?;
        writeln!(f, ",")?;
        write_indent(f, 3)?;
        writeln!(f, "\"fromVersion\": {},", cp.from_version)?;
        write_indent(f, 3)?;
        write!(f, "\"to\": ")?;
        write_json_str(f, cp.to_layout)?;
        writeln!(f, ",")?;
        write_indent(f, 3)?;
        writeln!(f, "\"toVersion\": {},", cp.to_version)?;
        write_indent(f, 3)?;
        let policy_name = match cp.policy {
            MigrationPolicy::NoOp => "noop",
            MigrationPolicy::AppendOnly => "append-only",
            MigrationPolicy::RequiresMigration => "requires-migration",
            MigrationPolicy::Incompatible => "incompatible",
        };
        write!(f, "\"policy\": ")?;
        write_json_str(f, policy_name)?;
        writeln!(f, ",")?;
        write_indent(f, 3)?;
        writeln!(f, "\"backwardReadable\": {}", cp.backward_readable)?;
        write_indent(f, 2)?;
        write!(f, "}}")?;
        if i + 1 < pairs.len() {
            writeln!(f, ",")?;
        } else {
            writeln!(f)?;
        }
    }
    write_indent(f, 1)?;
    write!(f, "]")
}

/// Emit the fixed 72-byte receipt wire schema as a JSON object.
/// This describes the binary layout so tools can decode receipts
/// without linking the Hopper crate.
fn write_receipt_schema(f: &mut fmt::Formatter<'_>) -> fmt::Result {
    writeln!(f, "{{")?;
    write_indent(f, 2)?;
    writeln!(f, "\"size\": 72,")?;
    write_indent(f, 2)?;
    writeln!(f, "\"legacySize\": 64,")?;
    write_indent(f, 2)?;
    writeln!(f, "\"fields\": [")?;
    let fields: &[(&str, &str, u8, u8)] = &[
        ("layout_id", "bytes", 0, 8),
        ("changed_fields", "u64", 8, 8),
        ("changed_bytes", "u32", 16, 4),
        ("changed_regions", "u16", 20, 2),
        ("old_size", "u32", 22, 4),
        ("new_size", "u32", 26, 4),
        ("invariants_checked", "u16", 30, 2),
        ("flags", "u8", 32, 1),
        ("before_fingerprint", "bytes", 33, 8),
        ("after_fingerprint", "bytes", 41, 8),
        ("segment_changed_mask", "u16", 49, 2),
        ("policy_flags", "u32", 51, 4),
        ("journal_appends", "u16", 55, 2),
        ("cpi_count", "u8", 57, 1),
        ("phase", "u8", 58, 1),
        ("validation_bundle_id", "u16", 59, 2),
        ("compat_impact", "u8", 61, 1),
        ("migration_flags", "u8", 62, 1),
        ("failed_invariant_idx", "u8", 63, 1),
        ("failed_error_code", "u32", 64, 4),
        ("failure_stage", "u8", 68, 1),
        ("reserved", "bytes", 69, 3),
    ];
    for (i, (name, ty, offset, size)) in fields.iter().enumerate() {
        write_indent(f, 3)?;
        write!(f, "{{ \"name\": ")?;
        write_json_str(f, name)?;
        write!(f, ", \"type\": ")?;
        write_json_str(f, ty)?;
        write!(f, ", \"offset\": {}, \"size\": {} }}", offset, size)?;
        if i + 1 < fields.len() {
            writeln!(f, ",")?;
        } else {
            writeln!(f)?;
        }
    }
    write_indent(f, 2)?;
    writeln!(f, "]")?;
    write_indent(f, 1)?;
    write!(f, "}}")
}

// ---------------------------------------------------------------------------
// Projection wrappers: ProgramManifest → IDL JSON / Codama JSON
// ---------------------------------------------------------------------------

/// Projects a `ProgramManifest` to IDL-level JSON (public schema subset).
///
/// Strips internal policy logic, migration planner hints, trust internals,
/// and unsafe metadata. Retains: instructions (with args + accounts),
/// account layouts (with fields), events, and layout fingerprints.
pub struct IdlJsonFromManifest<'a>(pub &'a ProgramManifest);

impl<'a> fmt::Display for IdlJsonFromManifest<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let p = self.0;
        writeln!(f, "{{")?;
        write!(f, "  \"name\": ")?;
        write_json_str(f, p.name)?;
        writeln!(f, ",")?;
        write!(f, "  \"version\": ")?;
        write_json_str(f, p.version)?;
        writeln!(f, ",")?;
        write!(f, "  \"description\": ")?;
        write_json_str(f, p.description)?;
        writeln!(f, ",")?;

        // Instructions (projected: drop capabilities, policy_pack, receipt_expected)
        write!(f, "  \"instructions\": ")?;
        if p.instructions.is_empty() {
            writeln!(f, "[],")?;
        } else {
            writeln!(f, "[")?;
            for (i, ix) in p.instructions.iter().enumerate() {
                write_indent(f, 2)?;
                writeln!(f, "{{")?;
                write_indent(f, 3)?;
                write!(f, "\"name\": ")?;
                write_json_str(f, ix.name)?;
                writeln!(f, ",")?;
                write_indent(f, 3)?;
                writeln!(f, "\"tag\": {},", ix.tag)?;
                write_indent(f, 3)?;
                write!(f, "\"args\": ")?;
                write_args_json(f, ix.args, 3)?;
                writeln!(f, ",")?;
                write_indent(f, 3)?;
                write!(f, "\"accounts\": ")?;
                write_account_entry_array(f, ix.accounts, 3)?;
                writeln!(f)?;
                write_indent(f, 2)?;
                write!(f, "}}")?;
                if i + 1 < p.instructions.len() {
                    writeln!(f, ",")?;
                } else {
                    writeln!(f)?;
                }
            }
            writeln!(f, "  ],")?;
        }

        // Accounts (full layout manifests)
        write!(f, "  \"accounts\": ")?;
        write_layout_array(f, p.layouts)?;
        writeln!(f, ",")?;

        // Events
        write!(f, "  \"events\": ")?;
        write_event_array(f, p.events)?;
        writeln!(f, ",")?;

        // Fingerprints (derived from layouts)
        write!(f, "  \"fingerprints\": ")?;
        if p.layouts.is_empty() {
            writeln!(f, "[]")?;
        } else {
            writeln!(f, "[")?;
            for (i, l) in p.layouts.iter().enumerate() {
                write_indent(f, 2)?;
                write!(f, "{{ \"layoutId\": ")?;
                write_hex_json(f, &l.layout_id)?;
                write!(f, ", \"name\": ")?;
                write_json_str(f, l.name)?;
                write!(f, " }}")?;
                if i + 1 < p.layouts.len() {
                    writeln!(f, ",")?;
                } else {
                    writeln!(f)?;
                }
            }
            writeln!(f, "  ]")?;
        }

        write!(f, "}}")
    }
}

/// Projects a `ProgramManifest` to Codama-level JSON (interop subset).
///
/// Only the minimal fields needed for Codama/Kinobi tooling:
/// instructions (name, discriminator, args, accounts), account types
/// (name, discriminator, size, fields), and events.
pub struct CodamaJsonFromManifest<'a>(pub &'a ProgramManifest);

impl<'a> fmt::Display for CodamaJsonFromManifest<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let p = self.0;
        writeln!(f, "{{")?;
        write!(f, "  \"name\": ")?;
        write_json_str(f, p.name)?;
        writeln!(f, ",")?;
        write!(f, "  \"version\": ")?;
        write_json_str(f, p.version)?;
        writeln!(f, ",")?;

        // Instructions (Codama: name, discriminator=tag, args, accounts as flat entries)
        write!(f, "  \"instructions\": ")?;
        if p.instructions.is_empty() {
            writeln!(f, "[],")?;
        } else {
            writeln!(f, "[")?;
            for (i, ix) in p.instructions.iter().enumerate() {
                write_indent(f, 2)?;
                writeln!(f, "{{")?;
                write_indent(f, 3)?;
                write!(f, "\"name\": ")?;
                write_json_str(f, ix.name)?;
                writeln!(f, ",")?;
                write_indent(f, 3)?;
                writeln!(f, "\"discriminator\": {},", ix.tag)?;
                write_indent(f, 3)?;
                write!(f, "\"args\": ")?;
                write_args_json(f, ix.args, 3)?;
                writeln!(f, ",")?;
                write_indent(f, 3)?;
                write!(f, "\"accounts\": ")?;
                write_account_entry_array(f, ix.accounts, 3)?;
                writeln!(f)?;
                write_indent(f, 2)?;
                write!(f, "}}")?;
                if i + 1 < p.instructions.len() {
                    writeln!(f, ",")?;
                } else {
                    writeln!(f)?;
                }
            }
            writeln!(f, "  ],")?;
        }

        // Accounts (Codama: name, discriminator=disc, size, fields)
        write!(f, "  \"accounts\": ")?;
        if p.layouts.is_empty() {
            writeln!(f, "[],")?;
        } else {
            writeln!(f, "[")?;
            for (i, l) in p.layouts.iter().enumerate() {
                write_indent(f, 2)?;
                writeln!(f, "{{")?;
                write_indent(f, 3)?;
                write!(f, "\"name\": ")?;
                write_json_str(f, l.name)?;
                writeln!(f, ",")?;
                write_indent(f, 3)?;
                writeln!(f, "\"discriminator\": {},", l.disc)?;
                write_indent(f, 3)?;
                writeln!(f, "\"size\": {},", l.total_size)?;
                write_indent(f, 3)?;
                write!(f, "\"fields\": ")?;
                write_fields_json(f, l.fields, 3)?;
                writeln!(f)?;
                write_indent(f, 2)?;
                write!(f, "}}")?;
                if i + 1 < p.layouts.len() {
                    writeln!(f, ",")?;
                } else {
                    writeln!(f)?;
                }
            }
            writeln!(f, "  ],")?;
        }

        // Events (Codama: name, discriminator=tag, fields)
        write!(f, "  \"events\": ")?;
        if p.events.is_empty() {
            writeln!(f, "[]")?;
        } else {
            writeln!(f, "[")?;
            for (i, e) in p.events.iter().enumerate() {
                write_indent(f, 2)?;
                writeln!(f, "{{")?;
                write_indent(f, 3)?;
                write!(f, "\"name\": ")?;
                write_json_str(f, e.name)?;
                writeln!(f, ",")?;
                write_indent(f, 3)?;
                writeln!(f, "\"discriminator\": {},", e.tag)?;
                write_indent(f, 3)?;
                write!(f, "\"fields\": ")?;
                write_fields_json(f, e.fields, 3)?;
                writeln!(f)?;
                write_indent(f, 2)?;
                write!(f, "}}")?;
                if i + 1 < p.events.len() {
                    writeln!(f, ",")?;
                } else {
                    writeln!(f)?;
                }
            }
            writeln!(f, "  ]")?;
        }

        write!(f, "}}")
    }
}

// ---------------------------------------------------------------------------
// DescriptorMetadata JSON -- fail-closed off-chain decode record
// ---------------------------------------------------------------------------

fn layout_kind_str(kind: LayoutKind) -> &'static str {
    match kind {
        LayoutKind::Compact => "compact",
        LayoutKind::Headered => "headered",
    }
}

/// JSON wrapper for a single [`DescriptorMetadata`] record.
///
/// Emits exactly what a generated client embeds to fail closed before
/// zero-copy-decoding: identity (`name`/`disc`/`version`), shape
/// (`kind`/`bodyOffset`/`bodySize`/`fixedSize`/`minSize`/`hasDynamicTail`/
/// `deprecated`), the 8-byte `layoutId` and 16-byte `fingerprint` as hex, the
/// `loadedDataSizeRecommendation`, and the field wire `offsets`.
pub struct DescriptorMetadataJson<'a>(pub &'a DescriptorMetadata);

impl<'a> fmt::Display for DescriptorMetadataJson<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let m = self.0;
        let n = &m.node;
        writeln!(f, "{{")?;
        write!(f, "  \"name\": ")?;
        write_json_str(f, n.name)?;
        writeln!(f, ",")?;
        writeln!(f, "  \"disc\": {},", n.disc)?;
        writeln!(f, "  \"version\": {},", n.version)?;
        writeln!(f, "  \"kind\": \"{}\",", layout_kind_str(n.kind))?;
        writeln!(f, "  \"bodyOffset\": {},", n.body_offset)?;
        writeln!(f, "  \"bodySize\": {},", n.body_size)?;
        writeln!(f, "  \"fixedSize\": {},", n.body_size)?;
        writeln!(f, "  \"minSize\": {},", n.min_size)?;
        writeln!(f, "  \"hasDynamicTail\": {},", n.has_dynamic_tail)?;
        writeln!(f, "  \"deprecated\": {},", n.deprecated)?;
        write!(f, "  \"layoutId\": ")?;
        write_hex_json(f, &n.layout_id)?;
        writeln!(f, ",")?;
        write!(f, "  \"fingerprint\": ")?;
        write_hex_json(f, &n.fingerprint.as_bytes())?;
        writeln!(f, ",")?;
        writeln!(
            f,
            "  \"loadedDataSizeRecommendation\": {},",
            m.loaded_data_size
        )?;
        write!(f, "  \"fields\": ")?;
        write_fields_json(f, m.fields, 1)?;
        writeln!(f)?;
        write!(f, "}}")
    }
}

/// JSON wrapper for a program's full descriptor-metadata set.
///
/// Emits an `accounts` array of [`DescriptorMetadataJson`] records plus two
/// transaction-builder budgets summed across the set: `minLoadedDataSize`
/// (the floor each layout's loader enforces) and `recommendedLoadedDataSize`
/// (the floor plus per-layout margins). Both are saturating; clamp to the
/// runtime maximum at the call site.
pub struct DescriptorMetadataSetJson<'a>(pub &'a [DescriptorMetadata]);

impl<'a> fmt::Display for DescriptorMetadataSetJson<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let set = self.0;
        let mut min_total: u64 = 0;
        let mut rec_total: u64 = 0;
        for m in set {
            min_total = min_total.saturating_add(m.node.min_size as u64);
            rec_total = rec_total.saturating_add(m.loaded_data_size);
        }
        writeln!(f, "{{")?;
        write!(f, "  \"accounts\": ")?;
        if set.is_empty() {
            write!(f, "[]")?;
        } else {
            writeln!(f, "[")?;
            for (i, m) in set.iter().enumerate() {
                write!(f, "{}", DescriptorMetadataJson(m))?;
                if i + 1 < set.len() {
                    writeln!(f, ",")?;
                } else {
                    writeln!(f)?;
                }
            }
            write!(f, "  ]")?;
        }
        writeln!(f, ",")?;
        writeln!(f, "  \"minLoadedDataSize\": {},", min_total)?;
        writeln!(f, "  \"recommendedLoadedDataSize\": {}", rec_total)?;
        write!(f, "}}")
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    extern crate alloc;
    use alloc::format;

    use super::*;
    use crate::{
        AccountDescriptor, CodamaAccount, CodamaInstruction, CodamaProjection, FieldIntent,
        ProgramIdl, ProgramManifest,
    };

    #[test]
    fn codama_json_empty() {
        let c = CodamaProjection::empty();
        let json = format!("{}", CodamaJson(&c));
        assert!(json.contains("\"name\": \"\""));
        assert!(json.contains("\"instructions\": []"));
        assert!(json.contains("\"accounts\": []"));
        assert!(json.contains("\"events\": []"));
    }

    #[test]
    fn idl_json_empty() {
        let idl = ProgramIdl::empty();
        let json = format!("{}", IdlJson(&idl));
        assert!(json.contains("\"name\": \"\""));
        assert!(json.contains("\"instructions\": []"));
        assert!(json.contains("\"accounts\": []"));
        assert!(json.contains("\"events\": []"));
        assert!(json.contains("\"fingerprints\": []"));
    }

    #[test]
    fn manifest_json_empty() {
        let m = ProgramManifest::empty();
        let json = format!("{}", ManifestJson(&m));
        assert!(json.contains("\"name\": \"\""));
        assert!(json.contains("\"layouts\": []"));
        assert!(json.contains("\"instructions\": []"));
        assert!(json.contains("\"events\": []"));
        assert!(json.contains("\"policies\": []"));
    }

    #[test]
    fn codama_json_with_instruction() {
        static ARGS: &[ArgDescriptor] = &[ArgDescriptor {
            name: "amount",
            canonical_type: "WireU64",
            size: 8,
        }];
        static ACCOUNTS: &[IdlAccountEntry] = &[IdlAccountEntry {
            name: "vault",
            writable: true,
            signer: false,
            layout_ref: "VaultState",
            pda_seeds: &[],
        }];
        static IX: &[CodamaInstruction] = &[CodamaInstruction {
            name: "deposit",
            discriminator: 1,
            args: ARGS,
            accounts: ACCOUNTS,
        }];
        let c = CodamaProjection {
            name: "test_program",
            version: "0.1.0",
            instructions: IX,
            accounts: &[],
            events: &[],
        };
        let json = format!("{}", CodamaJson(&c));
        assert!(json.contains("\"test_program\""));
        assert!(json.contains("\"deposit\""));
        assert!(json.contains("\"discriminator\": 1"));
        assert!(json.contains("\"amount\""));
        assert!(json.contains("\"vault\""));
        assert!(json.contains("\"writable\": true"));
        assert!(json.contains("\"layoutRef\": \"VaultState\""));
    }

    #[test]
    fn codama_json_with_account() {
        static FIELDS: &[FieldDescriptor] = &[FieldDescriptor {
            name: "balance",
            canonical_type: "WireU64",
            size: 8,
            offset: 16,
            intent: FieldIntent::Custom,
        }];
        static ACCTS: &[CodamaAccount] = &[CodamaAccount {
            name: "VaultState",
            discriminator: 1,
            size: 24,
            fields: FIELDS,
        }];
        let c = CodamaProjection {
            name: "test",
            version: "0.1.0",
            instructions: &[],
            accounts: ACCTS,
            events: &[],
        };
        let json = format!("{}", CodamaJson(&c));
        assert!(json.contains("\"VaultState\""));
        assert!(json.contains("\"discriminator\": 1"));
        assert!(json.contains("\"size\": 24"));
        assert!(json.contains("\"balance\""));
    }

    #[test]
    fn manifest_json_with_policy() {
        static POLICIES: &[PolicyDescriptor] = &[PolicyDescriptor {
            name: "TREASURY_WRITE",
            capabilities: &["MutatesState", "MutatesTreasury"],
            requirements: &["SignerAuthority"],
            invariants: &[],
            receipt_profile: "full",
        }];
        let m = ProgramManifest {
            name: "test",
            version: "0.1.0",
            description: "A test program",
            layouts: &[],
            layout_metadata: &[],
            instructions: &[],
            events: &[],
            policies: POLICIES,
            compatibility_pairs: &[],
            tooling_hints: &["show_receipt"],
            contexts: &[],
        };
        let json = format!("{}", ManifestJson(&m));
        assert!(json.contains("\"TREASURY_WRITE\""));
        assert!(json.contains("\"MutatesState\""));
        assert!(json.contains("\"SignerAuthority\""));
        assert!(json.contains("\"full\""));
        assert!(json.contains("\"show_receipt\""));
    }

    #[test]
    fn json_str_escapes_special_chars() {
        static FIELDS: &[FieldDescriptor] = &[];
        static ACCTS: &[CodamaAccount] = &[CodamaAccount {
            name: "has\"quotes",
            discriminator: 1,
            size: 16,
            fields: FIELDS,
        }];
        let c = CodamaProjection {
            name: "test\\prog",
            version: "1.0",
            instructions: &[],
            accounts: ACCTS,
            events: &[],
        };
        let json = format!("{}", CodamaJson(&c));
        assert!(json.contains("\"test\\\\prog\""));
        assert!(json.contains("\"has\\\"quotes\""));
    }

    #[test]
    fn idl_from_manifest_projection() {
        static FIELDS: &[FieldDescriptor] = &[FieldDescriptor {
            name: "balance",
            canonical_type: "WireU64",
            size: 8,
            offset: 16,
            intent: FieldIntent::Custom,
        }];
        static LAYOUTS: &[LayoutManifest] = &[LayoutManifest {
            name: "Vault",
            disc: 1,
            version: 1,
            layout_id: [1, 2, 3, 4, 5, 6, 7, 8],
            total_size: 24,
            field_count: 1,
            fields: FIELDS,
        }];
        static ARGS: &[ArgDescriptor] = &[ArgDescriptor {
            name: "amount",
            canonical_type: "WireU64",
            size: 8,
        }];
        static ACCTS: &[crate::AccountEntry] = &[crate::AccountEntry {
            name: "vault",
            writable: true,
            signer: false,
            layout_ref: "Vault",
            seeds: &[],
        }];
        static IX: &[InstructionDescriptor] = &[InstructionDescriptor {
            name: "deposit",
            tag: 1,
            args: ARGS,
            accounts: ACCTS,
            capabilities: &["MutatesState"],
            policy_pack: "TREASURY_WRITE",
            receipt_expected: true,
            strict_writes: false,
            write_ranges: &[],
            cu_estimate: 0,
        }];
        let m = ProgramManifest {
            name: "vault_prog",
            version: "1.0.0",
            description: "A vault",
            layouts: LAYOUTS,
            layout_metadata: &[],
            instructions: IX,
            events: &[],
            policies: &[],
            compatibility_pairs: &[],
            tooling_hints: &[],
            contexts: &[],
        };
        let json = format!("{}", IdlJsonFromManifest(&m));
        // IDL should have instruction name+tag+args+accounts but NOT capabilities/policyPack
        assert!(json.contains("\"deposit\""));
        assert!(json.contains("\"tag\": 1"));
        assert!(json.contains("\"amount\""));
        assert!(json.contains("\"vault\""));
        assert!(!json.contains("\"capabilities\""));
        assert!(!json.contains("\"policyPack\""));
        assert!(!json.contains("\"receiptExpected\""));
        // IDL should have fingerprints derived from layouts
        assert!(json.contains("\"fingerprints\""));
        assert!(json.contains("\"Vault\""));
    }

    #[test]
    fn codama_from_manifest_projection() {
        static ARGS: &[ArgDescriptor] = &[ArgDescriptor {
            name: "amount",
            canonical_type: "WireU64",
            size: 8,
        }];
        static ACCTS: &[crate::AccountEntry] = &[crate::AccountEntry {
            name: "vault",
            writable: true,
            signer: false,
            layout_ref: "Vault",
            seeds: &[],
        }];
        static IX: &[InstructionDescriptor] = &[InstructionDescriptor {
            name: "deposit",
            tag: 1,
            args: ARGS,
            accounts: ACCTS,
            capabilities: &["MutatesState"],
            policy_pack: "TREASURY_WRITE",
            receipt_expected: true,
            strict_writes: false,
            write_ranges: &[],
            cu_estimate: 0,
        }];
        static FIELDS: &[FieldDescriptor] = &[FieldDescriptor {
            name: "balance",
            canonical_type: "WireU64",
            size: 8,
            offset: 16,
            intent: FieldIntent::Custom,
        }];
        static LAYOUTS: &[LayoutManifest] = &[LayoutManifest {
            name: "Vault",
            disc: 1,
            version: 1,
            layout_id: [1, 2, 3, 4, 5, 6, 7, 8],
            total_size: 24,
            field_count: 1,
            fields: FIELDS,
        }];
        let m = ProgramManifest {
            name: "vault_prog",
            version: "1.0.0",
            description: "A vault",
            layouts: LAYOUTS,
            layout_metadata: &[],
            instructions: IX,
            events: &[],
            policies: &[],
            compatibility_pairs: &[],
            tooling_hints: &[],
            contexts: &[],
        };
        let json = format!("{}", CodamaJsonFromManifest(&m));
        // Codama: discriminator instead of tag
        assert!(json.contains("\"discriminator\": 1"));
        assert!(json.contains("\"deposit\""));
        assert!(json.contains("\"Vault\""));
        assert!(json.contains("\"size\": 24"));
        // Should NOT contain internal fields
        assert!(!json.contains("\"capabilities\""));
        assert!(!json.contains("\"policyPack\""));
        assert!(!json.contains("\"receiptExpected\""));
        assert!(!json.contains("\"fingerprints\""));
        assert!(!json.contains("\"toolingHints\""));
    }

    // -- DescriptorMetadata JSON --

    const VAULT_FIELDS: &[FieldDescriptor] = &[
        FieldDescriptor {
            name: "authority",
            canonical_type: "[u8;32]",
            size: 32,
            offset: 1,
            intent: FieldIntent::Custom,
        },
        FieldDescriptor {
            name: "balance",
            canonical_type: "WireU64",
            size: 8,
            offset: 33,
            intent: FieldIntent::Custom,
        },
    ];

    fn vault_descriptor() -> AccountDescriptor {
        AccountDescriptor::compact("Vault", 1, 1, 40, [0xAB; 8])
    }

    #[test]
    fn descriptor_metadata_json_carries_fail_closed_fields() {
        let d = vault_descriptor();
        let meta = DescriptorMetadata::from_descriptor(&d, VAULT_FIELDS);
        let json = format!("{}", DescriptorMetadataJson(&meta));

        assert!(json.contains("\"name\": \"Vault\""));
        assert!(json.contains("\"disc\": 1"));
        assert!(json.contains("\"version\": 1"));
        assert!(json.contains("\"kind\": \"compact\""));
        assert!(json.contains("\"bodyOffset\": 1"));
        assert!(json.contains("\"bodySize\": 40"));
        assert!(json.contains("\"fixedSize\": 40"));
        // compact: min_size = 1 + body_size.
        assert!(json.contains("\"minSize\": 41"));
        assert!(json.contains("\"hasDynamicTail\": false"));
        assert!(json.contains("\"deprecated\": false"));
        assert!(json.contains("\"layoutId\": \"abababababababab\""));

        // Fingerprint hex matches the descriptor's own to_hex (32 ASCII chars).
        let fp_hex = meta.fingerprint_hex();
        let fp_str = core::str::from_utf8(&fp_hex).unwrap();
        assert!(json.contains(fp_str));
        assert_eq!(fp_hex.len(), 32);

        // Default recommendation: min_size + flat margin (no tail headroom).
        assert!(json.contains("\"loadedDataSizeRecommendation\": 297"));

        // Field offsets are present for client decode.
        assert!(json.contains("\"name\": \"authority\""));
        assert!(json.contains("\"offset\": 33"));
    }

    #[test]
    fn descriptor_metadata_json_headered_and_dynamic_tail() {
        // Headered layout with a dynamic tail: body offset is 16, kind is
        // "headered", and the growable flag pulls in tail headroom.
        let d = AccountDescriptor::headered("Order", 2, 3, 48, [0x11; 8]).with_dynamic_tail();
        let meta = DescriptorMetadata::from_descriptor(&d, &[]);
        let json = format!("{}", DescriptorMetadataJson(&meta));

        assert!(json.contains("\"kind\": \"headered\""));
        assert!(json.contains("\"bodyOffset\": 16"));
        assert!(json.contains("\"minSize\": 64"));
        assert!(json.contains("\"hasDynamicTail\": true"));
        // min_size 64 + tail_headroom 1024 + margin 256 = 1344.
        assert!(json.contains("\"loadedDataSizeRecommendation\": 1344"));
        assert!(json.contains("\"fields\": []"));
    }

    #[test]
    fn descriptor_metadata_set_json_sums_budgets() {
        let d = vault_descriptor();
        let meta = DescriptorMetadata::from_descriptor(&d, VAULT_FIELDS);
        let set = [meta, meta];
        let json = format!("{}", DescriptorMetadataSetJson(&set));

        assert!(json.contains("\"accounts\": ["));
        // Two 41-byte floors.
        assert!(json.contains("\"minLoadedDataSize\": 82"));
        // Two (41 + 256) recommendations.
        assert!(json.contains("\"recommendedLoadedDataSize\": 594"));
    }

    #[test]
    fn descriptor_metadata_set_json_empty() {
        let json = format!("{}", DescriptorMetadataSetJson(&[]));
        assert!(json.contains("\"accounts\": []"));
        assert!(json.contains("\"minLoadedDataSize\": 0"));
        assert!(json.contains("\"recommendedLoadedDataSize\": 0"));
    }

    // -- BLD-WR: strict_writes byte-range publication --

    // A strict_writes `deposit`: vault (account 1) declares two exact field
    // ranges; config (account 2) is declared Sealevel-writable but has no
    // declared byte range, so it is provably read-only under the policy.
    static WR_ACCTS: &[crate::AccountEntry] = &[
        crate::AccountEntry {
            name: "authority",
            writable: false,
            signer: true,
            layout_ref: "",
            seeds: &[],
        },
        crate::AccountEntry {
            name: "vault",
            writable: true,
            signer: false,
            layout_ref: "Vault",
            seeds: &[],
        },
        crate::AccountEntry {
            name: "config",
            writable: true,
            signer: false,
            layout_ref: "Config",
            seeds: &[],
        },
    ];
    // vault balance [16, 24) and nonce [24, 32) — the same values the
    // strict_writes macro compiles into the enforced WritePolicy.
    static WR_RANGES: &[crate::WriteRange] = &[
        crate::WriteRange::new(1, 16, 8),
        crate::WriteRange::new(1, 24, 8),
    ];
    static WR_STRICT_IX: &[InstructionDescriptor] = &[InstructionDescriptor {
        name: "deposit",
        tag: 1,
        args: &[],
        accounts: WR_ACCTS,
        capabilities: &["MutatesState"],
        policy_pack: "VAULT_WRITE",
        receipt_expected: true,
        strict_writes: true,
        write_ranges: WR_RANGES,
        cu_estimate: 0,
    }];
    static WR_LOOSE_IX: &[InstructionDescriptor] = &[InstructionDescriptor {
        name: "deposit",
        tag: 1,
        args: &[],
        accounts: WR_ACCTS,
        capabilities: &["MutatesState"],
        policy_pack: "VAULT_WRITE",
        receipt_expected: true,
        strict_writes: false,
        write_ranges: &[],
        cu_estimate: 0,
    }];

    fn wr_manifest(instructions: &'static [InstructionDescriptor]) -> ProgramManifest {
        ProgramManifest {
            name: "vault_prog",
            version: "1.0.0",
            description: "A vault",
            layouts: &[],
            layout_metadata: &[],
            instructions,
            events: &[],
            policies: &[],
            compatibility_pairs: &[],
            tooling_hints: &[],
            contexts: &[],
        }
    }

    #[test]
    fn manifest_emits_strict_write_ranges_byte_matching_source() {
        let m = wr_manifest(WR_STRICT_IX);
        let json = format!("{}", ManifestJson(&m));

        // The instruction is flagged strict and carries the exact byte ranges.
        assert!(json.contains("\"strictWrites\": true"));
        // vault's two declared ranges, byte-identical to WR_RANGES.
        assert!(json
            .contains("\"account\": \"vault\", \"accountIndex\": 1, \"offset\": 16, \"size\": 8"));
        assert!(json
            .contains("\"account\": \"vault\", \"accountIndex\": 1, \"offset\": 24, \"size\": 8"));
        // config (account 2) declares no range, so it must NOT appear.
        assert!(!json.contains("\"account\": \"config\""));
    }

    #[test]
    fn manifest_non_strict_instruction_emits_empty_write_ranges() {
        let m = wr_manifest(WR_LOOSE_IX);
        let json = format!("{}", ManifestJson(&m));
        assert!(json.contains("\"strictWrites\": false"));
        assert!(json.contains("\"writeRanges\": []"));
    }

    // -- BLD-CU: measured per-instruction CU estimate publication --

    #[test]
    fn manifest_emits_cu_estimate_when_published() {
        // An author-supplied measured bound is emitted verbatim as cuEstimate.
        static CU_IX: &[InstructionDescriptor] = &[InstructionDescriptor {
            name: "deposit",
            tag: 1,
            args: &[],
            accounts: WR_ACCTS,
            capabilities: &["MutatesState"],
            policy_pack: "VAULT_WRITE",
            receipt_expected: true,
            strict_writes: false,
            write_ranges: &[],
            cu_estimate: 4_800,
        }];
        let m = wr_manifest(CU_IX);
        let json = format!("{}", ManifestJson(&m));
        assert!(json.contains("\"cuEstimate\": 4800"));
    }

    #[test]
    fn manifest_omits_cu_estimate_when_unknown() {
        // cu_estimate == 0 means "unmeasured", not "free": the key must be
        // absent so no client budgets a transaction from a fabricated zero.
        let m = wr_manifest(WR_LOOSE_IX);
        let json = format!("{}", ManifestJson(&m));
        assert!(!json.contains("cuEstimate"));
    }

    #[test]
    fn cu_budget_with_margin_pads_and_clamps() {
        // Unknown estimate publishes no budget at all.
        assert_eq!(WR_LOOSE_IX[0].cu_budget_with_margin(), None);
        // A published estimate gains a 10% margin (never shrinks)...
        let ix = InstructionDescriptor {
            cu_estimate: 10_000,
            ..WR_LOOSE_IX[0]
        };
        assert_eq!(ix.cu_budget_with_margin(), Some(11_000));
        // ...and clamps to the runtime's 1.4M CU transaction cap.
        let big = InstructionDescriptor {
            cu_estimate: 1_399_999,
            ..WR_LOOSE_IX[0]
        };
        assert_eq!(
            big.cu_budget_with_margin(),
            Some(crate::MAX_COMPUTE_UNIT_LIMIT)
        );
    }

    #[test]
    fn effective_writable_is_a_sound_passthrough() {
        // Demotion based on data write-ranges alone is unsound (a WriteRange
        // is a data range; Sealevel writability also covers lamport-only
        // mutation like close/sweep recipients). Until write sets can be
        // declared mutation-complete, effective_writable preserves the
        // declared flag for every account under both strict and non-strict
        // instructions, and never promotes a read-only account.
        let ix = &WR_STRICT_IX[0];
        assert!(!ix.effective_writable(0, false)); // read-only stays read-only
        assert!(ix.effective_writable(1, true)); // writable stays writable
        assert!(ix.effective_writable(2, true)); // NOT demoted (would be unsound)

        let loose = &WR_LOOSE_IX[0];
        assert!(loose.effective_writable(2, true));

        // account_has_declared_write still answers the raw range query for
        // schedulers/indexers even though the client no longer demotes on it.
        assert!(ix.account_has_declared_write(1));
        assert!(!ix.account_has_declared_write(2));
    }

    #[test]
    fn strict_with_empty_ranges_demotes_nothing() {
        // A strict_writes instruction whose ranges are entirely empty means
        // "ranges not populated" (the macro does not yet auto-emit them),
        // NOT "everything is read-only". Demoting here would send a written
        // account as read-only and fail the transaction on chain. Every
        // declared-writable account must survive unchanged.
        static UNPOP_IX: &[InstructionDescriptor] = &[InstructionDescriptor {
            name: "deposit",
            tag: 1,
            args: &[],
            accounts: WR_ACCTS,
            capabilities: &["MutatesState"],
            policy_pack: "VAULT_WRITE",
            receipt_expected: true,
            strict_writes: true,
            write_ranges: &[],
            cu_estimate: 0,
        }];
        let ix = &UNPOP_IX[0];
        assert!(ix.effective_writable(1, true));
        assert!(ix.effective_writable(2, true));
        // A declared read-only account still stays read-only (never promoted).
        assert!(!ix.effective_writable(0, false));
    }

    #[test]
    fn decode_guard_fails_closed_on_mismatch() {
        let d = vault_descriptor();
        let same = d.fingerprint();
        assert!(crate::decode_allowed(&d.fingerprint(), &same));

        // A version bump changes the wire-identity fingerprint, so a client
        // embedding the old fingerprint must refuse to decode.
        let bumped = AccountDescriptor::compact("Vault", 1, 2, 40, [0xAB; 8]);
        assert!(!crate::decode_allowed(
            &d.fingerprint(),
            &bumped.fingerprint()
        ));
    }
}
