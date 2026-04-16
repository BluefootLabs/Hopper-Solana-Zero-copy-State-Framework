//! Cross-version analysis: compatibility verdicts and semantic field diffs.
//!
//! These routines ingest two `LayoutManifest`s (or two versions of the same
//! account's bytes) and explain whether upgrades are safe, what fields
//! moved, and what downstream migration steps are implied — all from the
//! schema truth, never re-derived.

use core::fmt::Write;

use hopper_schema::{
    compare_fields, decode_header, is_append_compatible, is_backward_readable, requires_migration,
    CompatibilityVerdict, FieldCompat, LayoutManifest, ProgramManifest,
};

/// Render a compatibility report between two versions of the same layout
/// name declared in the manifest.
///
/// `from_version` is the current on-chain version; `to_version` is the
/// target version. Returns `Err(String)` if either version is missing.
pub fn compatibility_report(
    manifest: &ProgramManifest,
    layout_name: &str,
    from_version: u8,
    to_version: u8,
) -> Result<String, String> {
    let older = find_layout_version(manifest, layout_name, from_version).ok_or_else(|| {
        format!(
            "layout {} v{} not in manifest",
            layout_name, from_version
        )
    })?;
    let newer = find_layout_version(manifest, layout_name, to_version).ok_or_else(|| {
        format!(
            "layout {} v{} not in manifest",
            layout_name, to_version
        )
    })?;

    let verdict = CompatibilityVerdict::between(older, newer);

    let mut out = String::new();
    let _ = writeln!(
        out,
        "=== Compatibility: {} v{} -> v{} ===",
        layout_name, from_version, to_version
    );
    let _ = writeln!(
        out,
        "  {} v{}  ({} bytes, {} fields)",
        older.name, older.version, older.total_size, older.field_count
    );
    let _ = writeln!(
        out,
        "  {} v{}  ({} bytes, {} fields)",
        newer.name, newer.version, newer.total_size, newer.field_count
    );
    let _ = writeln!(out);
    let _ = writeln!(out, "  Verdict            : {}", verdict.name());
    let _ = writeln!(out, "  Description        : {}", describe_verdict(verdict));
    let _ = writeln!(
        out,
        "  Append compatible  : {}",
        is_append_compatible(older, newer)
    );
    let _ = writeln!(
        out,
        "  Backward readable  : {}",
        is_backward_readable(older, newer)
    );
    let _ = writeln!(
        out,
        "  Requires migration : {}",
        requires_migration(older, newer)
    );

    // Field-level detail.
    let report = compare_fields::<64>(older, newer);
    if report.count > 0 {
        let _ = writeln!(out);
        let _ = writeln!(out, "  Field changes:");
        for entry in report.entries.iter().take(report.count) {
            let _ = writeln!(
                out,
                "    {:<20}  {}",
                entry.name,
                describe_field_compat(&entry.status)
            );
        }
    }
    Ok(out)
}

/// Render a byte-level comparison between two account data blobs whose
/// layouts may be at different versions. Identifies each side, then
/// delegates to `compatibility_report` if layouts differ.
pub fn field_diff_report(
    manifest: &ProgramManifest,
    before: &[u8],
    after: &[u8],
) -> Result<String, String> {
    let Some(before_hdr) = decode_header(before) else {
        return Err(String::from("'before' data is too short to decode"));
    };
    let Some(after_hdr) = decode_header(after) else {
        return Err(String::from("'after' data is too short to decode"));
    };

    let older = manifest.identify_from_data(before).ok_or_else(|| {
        format!(
            "cannot identify 'before' layout (disc={}, id={})",
            before_hdr.disc,
            hex8(&before_hdr.layout_id)
        )
    })?;
    let newer = manifest.identify_from_data(after).ok_or_else(|| {
        format!(
            "cannot identify 'after' layout (disc={}, id={})",
            after_hdr.disc,
            hex8(&after_hdr.layout_id)
        )
    })?;

    let mut out = String::new();
    let _ = writeln!(out, "=== Semantic Field Diff ===");
    let _ = writeln!(
        out,
        "  before: {} v{} ({} bytes)",
        older.name,
        older.version,
        before.len()
    );
    let _ = writeln!(
        out,
        "  after : {} v{} ({} bytes)",
        newer.name,
        newer.version,
        after.len()
    );
    let _ = writeln!(out);

    if older.name != newer.name {
        let _ = writeln!(
            out,
            "  layouts have different names — treating as full replacement"
        );
        return Ok(out);
    }

    if older.version == newer.version {
        diff_same_version(&mut out, older, before, after);
        return Ok(out);
    }

    // Cross-version: reuse the compatibility report, then append a concrete
    // per-field byte diff for fields that exist in both versions.
    let compat = compatibility_report(manifest, older.name, older.version, newer.version)?;
    out.push_str(&compat);
    out.push('\n');
    diff_cross_version(&mut out, older, newer, before, after);
    Ok(out)
}

fn diff_same_version(
    out: &mut String,
    layout: &LayoutManifest,
    before: &[u8],
    after: &[u8],
) {
    let _ = writeln!(out, "  (same version, showing per-field byte deltas)");
    for field in layout.fields.iter().take(layout.field_count) {
        let end = field.offset as usize + field.size as usize;
        let before_slice = before.get(field.offset as usize..end);
        let after_slice = after.get(field.offset as usize..end);
        match (before_slice, after_slice) {
            (Some(b), Some(a)) if b != a => {
                let _ = writeln!(
                    out,
                    "    {:<20}  CHANGED  before={}  after={}",
                    field.name,
                    hex_any(b),
                    hex_any(a)
                );
            }
            (Some(_), Some(_)) => {}
            _ => {
                let _ = writeln!(
                    out,
                    "    {:<20}  SKIPPED (out of bounds)",
                    field.name
                );
            }
        }
    }
}

fn diff_cross_version(
    out: &mut String,
    older: &LayoutManifest,
    newer: &LayoutManifest,
    before: &[u8],
    after: &[u8],
) {
    let _ = writeln!(out, "  Per-field byte deltas:");
    for older_field in older.fields.iter().take(older.field_count) {
        let Some(newer_field) = newer
            .fields
            .iter()
            .take(newer.field_count)
            .find(|f| f.name == older_field.name)
        else {
            let _ = writeln!(
                out,
                "    {:<20}  REMOVED in v{}",
                older_field.name, newer.version
            );
            continue;
        };
        let ob_end = older_field.offset as usize + older_field.size as usize;
        let nb_end = newer_field.offset as usize + newer_field.size as usize;
        let b = before.get(older_field.offset as usize..ob_end);
        let a = after.get(newer_field.offset as usize..nb_end);
        match (b, a) {
            (Some(b), Some(a)) => {
                if b == a {
                    continue;
                }
                let _ = writeln!(
                    out,
                    "    {:<20}  CHANGED  v{}: {}  v{}: {}",
                    older_field.name,
                    older.version,
                    hex_any(b),
                    newer.version,
                    hex_any(a),
                );
            }
            _ => {
                let _ = writeln!(
                    out,
                    "    {:<20}  SKIPPED (out of bounds)",
                    older_field.name
                );
            }
        }
    }
    for newer_field in newer.fields.iter().take(newer.field_count) {
        if !older
            .fields
            .iter()
            .take(older.field_count)
            .any(|f| f.name == newer_field.name)
        {
            let _ = writeln!(
                out,
                "    {:<20}  ADDED in v{}",
                newer_field.name, newer.version
            );
        }
    }
}

fn describe_verdict(v: CompatibilityVerdict) -> &'static str {
    match v {
        CompatibilityVerdict::Identical => {
            "byte-identical layouts, no transition required"
        }
        CompatibilityVerdict::WireCompatible => {
            "wire-compatible; only semantic metadata differs"
        }
        CompatibilityVerdict::AppendSafe => {
            "append-safe; old readers can still decode new data"
        }
        CompatibilityVerdict::MigrationRequired => {
            "migration required; existing bytes must be rewritten"
        }
        CompatibilityVerdict::Incompatible => {
            "incompatible; discriminators diverge"
        }
    }
}

fn describe_field_compat(compat: &FieldCompat) -> &'static str {
    match compat {
        FieldCompat::Identical => "identical",
        FieldCompat::Changed => "changed (type or size)",
        FieldCompat::Added => "added",
        FieldCompat::Removed => "removed",
    }
}

fn find_layout_version<'a>(
    manifest: &'a ProgramManifest,
    name: &str,
    version: u8,
) -> Option<&'a LayoutManifest> {
    manifest
        .layouts
        .iter()
        .find(|l| l.name == name && l.version == version)
}

fn hex8(bytes: &[u8; 8]) -> String {
    let mut out = String::with_capacity(16);
    for b in bytes {
        let _ = write!(out, "{:02x}", b);
    }
    out
}

fn hex_any(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(out, "{:02x}", b);
    }
    out
}
