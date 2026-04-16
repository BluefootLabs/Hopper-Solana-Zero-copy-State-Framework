//! Tabular summaries over a `ProgramManifest`.
//!
//! Each function returns a `String` containing the rendered report. The
//! manager crate is deliberately formatter-only: all truth comes from the
//! manifest. The CLI calls these and prints the returned strings; other
//! tools can embed the same output in different UIs.

use core::fmt::Write;

use hopper_schema::{InstructionDescriptor, ProgramManifest};

/// Render the same default overview that `ProgramManifest`'s `Display`
/// implementation produces.
pub fn program_summary(manifest: &ProgramManifest) -> String {
    format!("{}", manifest)
}

/// Render the `manager layouts` report: one block per layout with its
/// discriminator, version, fingerprint, and field table.
pub fn layouts_report(manifest: &ProgramManifest) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "=== Layouts ({}) ===", manifest.layouts.len());
    let _ = writeln!(out);
    for layout in manifest.layouts.iter() {
        let _ = writeln!(
            out,
            "{} v{}  disc={}  size={}  id={}",
            layout.name,
            layout.version,
            layout.disc,
            layout.total_size,
            hex8(&layout.layout_id)
        );
        if layout.field_count == 0 {
            let _ = writeln!(out, "  (no fields)");
        } else {
            let _ = writeln!(
                out,
                "  {:>4}  {:>20}  {:>6}  {:>6}  {:>12}",
                "#", "Field", "Off", "Size", "Type"
            );
            for (i, f) in layout.fields.iter().take(layout.field_count).enumerate() {
                let _ = writeln!(
                    out,
                    "  {:>4}  {:>20}  {:>6}  {:>6}  {:>12}",
                    i, f.name, f.offset, f.size, f.canonical_type
                );
            }
        }
        let _ = writeln!(out);
    }
    out
}

/// Render the `manager fingerprints` report: wire + semantic fingerprints
/// for every layout in the manifest.
pub fn fingerprints_report(manifest: &ProgramManifest) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "=== Layout Fingerprints ({}) ===", manifest.layouts.len());
    let _ = writeln!(out);
    for layout in manifest.layouts.iter() {
        let _ = writeln!(out, "{} v{}", layout.name, layout.version);
        let _ = writeln!(out, "  disc        : {}", layout.disc);
        let _ = writeln!(out, "  total_size  : {}", layout.total_size);
        let _ = writeln!(out, "  layout_id   : {}", hex8(&layout.layout_id));
        let _ = writeln!(out);
    }
    out
}

/// Render the `manager policies` report.
pub fn policies_report(manifest: &ProgramManifest) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "=== Policy Packs ({}) ===",
        manifest.policies.len()
    );
    let _ = writeln!(out);
    if manifest.policies.is_empty() {
        let _ = writeln!(out, "(no policy packs declared)");
        return out;
    }
    for policy in manifest.policies.iter() {
        let _ = writeln!(out, "- {}", policy.name);
        if !policy.capabilities.is_empty() {
            let _ = writeln!(
                out,
                "    capabilities : {}",
                policy.capabilities.join(", ")
            );
        }
        if !policy.requirements.is_empty() {
            let _ = writeln!(
                out,
                "    requirements : {}",
                policy.requirements.join(", ")
            );
        }
        if !policy.invariants.is_empty() {
            let _ = writeln!(
                out,
                "    invariants   : {}",
                policy.invariants.join(", ")
            );
        }
        if !policy.receipt_profile.is_empty() {
            let _ = writeln!(out, "    receipt      : {}", policy.receipt_profile);
        }
        let _ = writeln!(out);
    }
    out
}

/// Render the `manager events` report.
pub fn events_report(manifest: &ProgramManifest) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "=== Events ({}) ===", manifest.events.len());
    let _ = writeln!(out);
    if manifest.events.is_empty() {
        let _ = writeln!(out, "(no events declared)");
        return out;
    }
    for ev in manifest.events.iter() {
        let _ = writeln!(out, "{} (tag={})", ev.name, ev.tag);
        if ev.fields.is_empty() {
            let _ = writeln!(out, "  (no fields)");
        } else {
            for f in ev.fields.iter() {
                let _ = writeln!(
                    out,
                    "  {:<20}  {:>12}  offset={}  size={}",
                    f.name, f.canonical_type, f.offset, f.size
                );
            }
        }
        let _ = writeln!(out);
    }
    out
}

/// Render the `manager instruction <tag|name>` report for a single
/// instruction.
///
/// Accepts either the instruction tag (as a decimal string) or the name.
/// Returns `Err(String)` if the instruction isn't in the manifest.
pub fn instruction_report(
    manifest: &ProgramManifest,
    selector: &str,
) -> Result<String, String> {
    let instr = resolve_instruction(manifest, selector).ok_or_else(|| {
        format!(
            "no instruction matches '{}' (known: {})",
            selector,
            known_instruction_names(manifest)
        )
    })?;

    let mut out = String::new();
    let _ = writeln!(out, "=== Instruction: {} (tag {}) ===", instr.name, instr.tag);
    if instr.receipt_expected {
        let _ = writeln!(out, "  receipt: expected");
    }
    if !instr.policy_pack.is_empty() {
        let _ = writeln!(out, "  policy : {}", instr.policy_pack);
    }

    if !instr.args.is_empty() {
        let _ = writeln!(out);
        let _ = writeln!(out, "  Arguments:");
        for a in instr.args.iter() {
            let _ = writeln!(
                out,
                "    {:<16}  {:>12}  ({} bytes)",
                a.name, a.canonical_type, a.size
            );
        }
    }

    if !instr.accounts.is_empty() {
        let _ = writeln!(out);
        let _ = writeln!(out, "  Accounts:");
        for (i, acct) in instr.accounts.iter().enumerate() {
            let mut flags = Vec::with_capacity(3);
            if acct.signer {
                flags.push("signer");
            }
            if acct.writable {
                flags.push("mut");
            } else {
                flags.push("read");
            }
            if !acct.layout_ref.is_empty() {
                flags.push("typed");
            }
            let layout_ref = if acct.layout_ref.is_empty() {
                String::new()
            } else {
                format!("-> {}", acct.layout_ref)
            };
            let _ = writeln!(
                out,
                "    [{}] {:<20} {} {}",
                i,
                acct.name,
                flags.join(","),
                layout_ref,
            );
        }
    }
    Ok(out)
}

fn resolve_instruction<'a>(
    manifest: &'a ProgramManifest,
    selector: &str,
) -> Option<&'a InstructionDescriptor> {
    if let Ok(tag) = selector.parse::<u8>() {
        if let Some(ix) = manifest.find_instruction(tag) {
            return Some(ix);
        }
    }
    manifest
        .instructions
        .iter()
        .find(|ix| ix.name == selector)
}

fn known_instruction_names(manifest: &ProgramManifest) -> String {
    let names: Vec<&str> = manifest.instructions.iter().map(|i| i.name).collect();
    if names.is_empty() {
        String::from("<none>")
    } else {
        names.join(", ")
    }
}

fn hex8(bytes: &[u8; 8]) -> String {
    let mut out = String::with_capacity(16);
    for b in bytes {
        let _ = write!(out, "{:02x}", b);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // A trivial manifest keeps tests self-contained and fast.
    fn empty_manifest() -> ProgramManifest {
        ProgramManifest {
            name: "test",
            version: "0.1.0",
            description: "",
            layouts: &[],
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
    fn empty_policies_reports_zero() {
        let m = empty_manifest();
        let s = policies_report(&m);
        assert!(s.contains("(no policy packs declared)"));
    }

    #[test]
    fn empty_events_reports_zero() {
        let m = empty_manifest();
        let s = events_report(&m);
        assert!(s.contains("(no events declared)"));
    }

    #[test]
    fn instruction_report_missing_gives_error() {
        let m = empty_manifest();
        let err = instruction_report(&m, "deposit").unwrap_err();
        assert!(err.contains("deposit"));
    }
}
