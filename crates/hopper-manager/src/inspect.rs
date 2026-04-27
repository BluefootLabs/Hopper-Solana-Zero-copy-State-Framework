//! Header, segment, and field-level inspection of Hopper account bytes.
//!
//! Every function here is a pure `&ProgramManifest + &[u8] -> String` (or
//! `Option<String>`), no I/O, no panics. Consumes the schema truth.

use core::fmt::Write;

use hopper_schema::{
    decode_account_fields, decode_header, decode_segments, DecodedHeader, DecodedSegment,
    LayoutManifest, ProgramManifest,
};

const MAX_FIELDS: usize = 64;
const MAX_SEGMENTS: usize = 32;

/// Structured identification result.
///
/// `Match` means the manifest contains a layout whose `(disc, layout_id)`
/// matches the data's Hopper header. `NoMatch` means none did.
#[derive(Debug, Clone, Copy)]
pub enum IdentifyOutcome<'a> {
    Match {
        layout: &'a LayoutManifest,
        header: DecodedHeader,
        data_len: usize,
        size_mismatch: bool,
    },
    NoMatch {
        header: DecodedHeader,
        data_len: usize,
    },
    HeaderTooShort {
        data_len: usize,
    },
}

/// Identify which layout (if any) in the manifest matches the account bytes.
///
/// This is the same matching rule used at runtime: `(disc, layout_id)` must
/// agree with a layout in the manifest. No guessing, no fuzzy matching.
#[inline]
pub fn identify_account<'a>(manifest: &'a ProgramManifest, data: &[u8]) -> IdentifyOutcome<'a> {
    let Some(header) = decode_header(data) else {
        return IdentifyOutcome::HeaderTooShort {
            data_len: data.len(),
        };
    };
    match manifest.identify_from_data(data) {
        Some(layout) => IdentifyOutcome::Match {
            layout,
            header,
            data_len: data.len(),
            size_mismatch: data.len() != layout.total_size,
        },
        None => IdentifyOutcome::NoMatch {
            header,
            data_len: data.len(),
        },
    }
}

/// Render the identification outcome as the same "=== Account Identification ==="
/// block the CLI's `hopper manager identify` emits.
pub fn identify_report(manifest: &ProgramManifest, data: &[u8]) -> String {
    let mut out = String::new();
    match identify_account(manifest, data) {
        IdentifyOutcome::HeaderTooShort { data_len } => {
            let _ = writeln!(
                out,
                "Data too short for Hopper header (need 16 bytes, got {})",
                data_len
            );
        }
        IdentifyOutcome::Match {
            layout,
            header,
            data_len,
            size_mismatch,
        } => {
            let _ = writeln!(out, "=== Account Identification ===");
            let _ = writeln!(out, "  Data size    : {} bytes", data_len);
            let _ = writeln!(out, "  Header disc  : {}", header.disc);
            let _ = writeln!(out, "  Header ver   : {}", header.version);
            let _ = writeln!(out, "  Layout ID    : {}", hex8(&header.layout_id));
            let _ = writeln!(out);
            let _ = writeln!(out, "  MATCH: {} v{}", layout.name, layout.version);
            let _ = writeln!(out, "  Expected size: {} bytes", layout.total_size);
            let _ = writeln!(out, "  Fields       : {}", layout.field_count);
            if size_mismatch {
                let _ = writeln!(
                    out,
                    "  WARNING: data size ({}) != expected size ({})",
                    data_len, layout.total_size
                );
            }
        }
        IdentifyOutcome::NoMatch { header, data_len } => {
            let _ = writeln!(out, "=== Account Identification ===");
            let _ = writeln!(out, "  Data size    : {} bytes", data_len);
            let _ = writeln!(out, "  Header disc  : {}", header.disc);
            let _ = writeln!(out, "  Header ver   : {}", header.version);
            let _ = writeln!(out, "  Layout ID    : {}", hex8(&header.layout_id));
            let _ = writeln!(out);
            let _ = writeln!(
                out,
                "  NO MATCH: This account does not match any layout in the manifest."
            );
            let _ = writeln!(out);
            let _ = writeln!(out, "Known layouts:");
            for l in manifest.layouts.iter() {
                let _ = writeln!(
                    out,
                    "    {} v{} (disc={}, id={})",
                    l.name,
                    l.version,
                    l.disc,
                    hex8(&l.layout_id)
                );
            }
        }
    }
    out
}

/// Render the full table of decoded fields for an account, matching the
/// CLI's `hopper manager decode` output.
///
/// Returns `Err(String)` if the account cannot be identified against the
/// manifest. The error string is a human-readable diagnostic.
pub fn decode_account(
    manifest: &ProgramManifest,
    data: &[u8],
    heading: &str,
) -> Result<String, String> {
    if data.len() < 16 {
        return Err(format!(
            "Data too short for Hopper header (need 16, got {})",
            data.len()
        ));
    }
    let header =
        decode_header(data).ok_or_else(|| String::from("Failed to decode Hopper header"))?;
    let layout = manifest.identify_from_data(data).ok_or_else(|| {
        format!(
            "Cannot identify account type (disc={}, layout_id={})",
            header.disc,
            hex8(&header.layout_id),
        )
    })?;

    let mut out = String::new();
    let _ = writeln!(
        out,
        "=== {}: {} v{} ===",
        heading, layout.name, layout.version
    );
    let _ = writeln!(
        out,
        "  Size: {} bytes (expected {})",
        data.len(),
        layout.total_size
    );
    let _ = writeln!(
        out,
        "  Flags: {} (0x{:04x})",
        format_flags(header.flags),
        header.flags
    );
    let _ = writeln!(out, "  Disc : {}", header.disc);
    let _ = writeln!(out, "  Wire : {}", hex8(&layout.layout_id));
    let _ = writeln!(out);

    if layout.field_count == 0 {
        let _ = writeln!(out, "  (no field descriptors in manifest)");
        return Ok(out);
    }

    let (count, fields) = decode_account_fields::<MAX_FIELDS>(data, layout);
    let mut val_buf = [0u8; 128];
    let _ = writeln!(
        out,
        "  {:>4}  {:>20}  {:>12}  {:>6}  {:>6}  Value",
        "#", "Field", "Type", "Offset", "Size"
    );
    let _ = writeln!(out, "  {}", "-".repeat(76));
    for (i, slot) in fields.iter().enumerate().take(count) {
        if let Some(ref field) = slot {
            let val_len = field.format_value(&mut val_buf);
            let val_str = core::str::from_utf8(&val_buf[..val_len]).unwrap_or("???");
            let _ = writeln!(
                out,
                "  {:>4}  {:>20}  {:>12}  {:>6}  {:>6}  {}",
                i, field.name, field.canonical_type, field.offset, field.size, val_str
            );
        }
    }
    let _ = writeln!(out);
    let _ = writeln!(out, "  Decoded {}/{} fields.", count, layout.field_count);
    Ok(out)
}

/// Render a bare header report (not layout-aware).
///
/// Useful when the manifest is unknown or the caller just wants the raw
/// header bytes interpreted.
pub fn header_report(data: &[u8]) -> String {
    let mut out = String::new();
    match decode_header(data) {
        Some(h) => {
            let _ = writeln!(out, "=== Hopper Header ===");
            let _ = writeln!(out, "  Disc          : {}", h.disc);
            let _ = writeln!(out, "  Version       : {}", h.version);
            let _ = writeln!(
                out,
                "  Flags         : 0x{:04x} ({})",
                h.flags,
                format_flags(h.flags)
            );
            let _ = writeln!(out, "  Layout ID     : {}", hex8(&h.layout_id));
            let _ = writeln!(out, "  Reserved      : {}", hex4(&h.reserved));
        }
        None => {
            let _ = writeln!(
                out,
                "Data too short to decode Hopper header (need 16 bytes, got {})",
                data.len()
            );
        }
    }
    out
}

/// Render a segment map report (after the Hopper header) for accounts
/// that carry segment metadata.
pub fn segment_map_report(data: &[u8]) -> String {
    let mut out = String::new();
    match decode_segments::<MAX_SEGMENTS>(data) {
        Some((count, segs)) => {
            let _ = writeln!(out, "=== Segment Map ({} entries) ===", count);
            for (i, seg) in segs.iter().enumerate().take(count) {
                render_segment_line(&mut out, i, seg);
            }
        }
        None => {
            let _ = writeln!(out, "No segment map present (or data too short).");
        }
    }
    out
}

fn render_segment_line(out: &mut String, index: usize, seg: &DecodedSegment) {
    let _ = writeln!(
        out,
        "  [{}] id={} offset={} size={} flags=0x{:04x} ver={}",
        index,
        hex_any(&seg.id),
        seg.offset,
        seg.size,
        seg.flags,
        seg.version,
    );
}

// ── Local formatting helpers ─────────────────────────────────────────

fn format_flags(flags: u16) -> String {
    let mut parts = Vec::with_capacity(4);
    if flags & 0x0001 != 0 {
        parts.push("INITIALIZED");
    }
    if flags & 0x0002 != 0 {
        parts.push("LOCKED");
    }
    if flags & 0x0004 != 0 {
        parts.push("UPGRADED");
    }
    if flags & 0x0008 != 0 {
        parts.push("CLOSED");
    }
    if parts.is_empty() {
        String::from("none")
    } else {
        parts.join("|")
    }
}

fn hex8(bytes: &[u8; 8]) -> String {
    let mut out = String::with_capacity(16);
    for b in bytes {
        let _ = write!(out, "{:02x}", b);
    }
    out
}

fn hex4(bytes: &[u8; 4]) -> String {
    let mut out = String::with_capacity(8);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_report_handles_short_data() {
        let out = header_report(&[0x01, 0x02]);
        assert!(out.contains("Data too short"));
    }

    #[test]
    fn format_flags_all_zero_is_none() {
        assert_eq!(format_flags(0), "none");
    }

    #[test]
    fn format_flags_combines_known_bits() {
        let s = format_flags(0x0001 | 0x0004);
        assert!(s.contains("INITIALIZED"));
        assert!(s.contains("UPGRADED"));
    }
}
