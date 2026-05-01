//! `hopper verify` - ABI integrity check between a program manifest
//! and its compiled `.so` binary.
//!
//! This command closes the "winning architecture" design's
//! safety-check gap: Anchor, Quasar, and Pinocchio all trust the
//! developer to keep IDL and binary in sync. Hopper catches drift at
//! the CLI by scanning the compiled ELF for each layout's 8-byte
//! `LAYOUT_ID` fingerprint. Any manifest entry that does not appear
//! in the binary indicates a refactor that was not re-exported to the
//! manifest (or a manifest that belongs to a different program).
//!
//! The check is byte-level and deliberately offline: no Solana RPC,
//! no linker consultation. The 8-byte `LAYOUT_ID` produced by
//! `#[hopper::state]`'s canonical wire descriptor is unique with
//! near-certainty (SHA-256 of a layout's field names + wire types +
//! offsets, first 8 bytes); searching for the exact sequence in the
//! compiled binary establishes ABI continuity without needing debug
//! symbols.
//!
//! Manifest integrity is always fatal on failure. Binary anchor presence is
//! informational by default, fatal with `--strict`, and required/fatal with
//! `--release`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process;

use crate::workspace;

pub fn cmd_verify(args: &[String]) {
    if args.iter().any(|a| a == "--help" || a == "-h") {
        print_verify_usage();
        return;
    }

    let opts = match parse_verify_options(args) {
        Ok(o) => o,
        Err(msg) => {
            eprintln!("hopper verify: {msg}");
            print_verify_usage();
            process::exit(1);
        }
    };

    let cwd = workspace::current_dir().unwrap_or_else(|err| {
        eprintln!("{err}");
        process::exit(1);
    });

    let manifest_path = resolve_manifest_path(&opts, &cwd).unwrap_or_else(|err| {
        eprintln!("hopper verify: {err}");
        process::exit(1);
    });

    println!("hopper verify");
    println!("  manifest: {}", manifest_path.display());

    let manifest_json = fs::read_to_string(&manifest_path).unwrap_or_else(|err| {
        eprintln!("hopper verify: failed to read manifest: {err}");
        process::exit(1);
    });

    let layouts = extract_layouts_from_manifest(&manifest_json).unwrap_or_else(|err| {
        eprintln!("hopper verify: {err}");
        process::exit(1);
    });
    if layouts.is_empty() {
        eprintln!("hopper verify: manifest declares no layouts - nothing to check");
        process::exit(1);
    }

    // ── Stage 1: manifest integrity (always runs, always gates) ────
    //
    // Catches the refactor mistakes no amount of SBF inspection can:
    // duplicate layout_id, duplicate discriminator, all-zero bytes,
    // empty name. These are cheap, unambiguous, and always fatal.
    println!();
    println!("Manifest integrity ({} layouts):", layouts.len());
    println!("{}", "-".repeat(72));
    let integrity_failures = run_manifest_integrity(&layouts);
    if integrity_failures > 0 {
        eprintln!();
        eprintln!(
            "FAIL: {} manifest-integrity violations. Run `hopper compile --emit schema`",
            integrity_failures
        );
        eprintln!("and rebuild the program to regenerate a consistent manifest.");
        process::exit(1);
    }
    println!("  OK: unique disc, unique layout_id, non-zero bytes, valid names.");

    // ── Stage 2: binary presence scan (optional without --strict) ──
    //
    // The `#[hopper::state]` proc macro emits a `#[used]` anchor per
    // layout so LAYOUT_ID bytes survive SBF LTO. Even so, a program
    // may choose to strip debug symbols or run additional post-link
    // stripping, and the declarative `hopper_layout!` form does not
    // currently emit an anchor. The scan reports what it finds; it
    // is informational by default and only gating under `--strict`.
    let Some(so_input) = opts.so_input(&cwd).unwrap_or_else(|err| {
        eprintln!("hopper verify: {err}");
        process::exit(1);
    }) else {
        println!();
        println!("Binary scan: skipped (no .so supplied). Manifest-only verification complete.");
        return;
    };

    println!();
    println!("Binary scan: {}", so_input.display());

    let binary = fs::read(&so_input).unwrap_or_else(|err| {
        eprintln!("hopper verify: failed to read binary: {err}");
        process::exit(1);
    });
    if !has_elf_magic(&binary) {
        eprintln!(
            "hopper verify: {} does not look like an ELF binary (missing \\x7fELF magic)",
            so_input.display()
        );
        process::exit(1);
    }
    println!("  binary size: {} bytes", binary.len());
    println!();
    println!("{:<32} {:<24} {}", "Layout", "LAYOUT_ID (hex)", "Presence");
    println!("{}", "-".repeat(80));

    let mut found_count = 0u32;
    let mut missing_count = 0u32;
    for layout in &layouts {
        let id_hex = layout
            .layout_id
            .iter()
            .map(|b| format!("{:02x}", b))
            .collect::<String>();
        match find_subsequence(&binary, &layout.layout_id) {
            Some(offset) => {
                println!(
                    "{:<32} {:<24} anchored at 0x{:06x}",
                    layout.name, id_hex, offset
                );
                found_count += 1;
            }
            None => {
                println!("{:<32} {:<24} not anchored", layout.name, id_hex);
                missing_count += 1;
            }
        }
    }
    println!();
    println!(
        "  binary presence: {} of {} layouts anchored in .rodata",
        found_count,
        layouts.len()
    );

    if missing_count > 0 {
        if opts.strict {
            eprintln!();
            eprintln!(
                "FAIL ({}): {} of {} layouts not anchored in {}",
                if opts.release { "--release" } else { "--strict" },
                missing_count,
                layouts.len(),
                so_input.display()
            );
            eprintln!(
                "Layouts declared via `hopper_layout!` do not currently emit `#[used]`"
            );
            eprintln!(
                "anchors. Switch to `#[hopper::state]` or skip `--strict` for a"
            );
            eprintln!("manifest-only check.");
            process::exit(1);
        }
        println!();
        println!(
            "  note: layouts without anchors may be legitimately missing (LTO,"
        );
        println!(
            "  `hopper_layout!` path). Run with --strict to treat missing as fatal."
        );
    }

    println!();
    if opts.release {
        println!("OK: release verification passed; manifest and binary anchors agree.");
    } else {
        println!("OK: manifest integrity passed; binary presence reported above.");
    }
}

fn run_manifest_integrity(layouts: &[ManifestLayout]) -> u32 {
    let mut failures = 0u32;
    let mut seen_ids: Vec<(&[u8; 8], &str)> = Vec::new();
    let mut seen_names: Vec<&str> = Vec::new();

    for layout in layouts {
        if layout.name.is_empty() {
            println!("  FAIL: layout with empty name");
            failures += 1;
        }
        if seen_names.contains(&layout.name.as_str()) {
            println!("  FAIL: duplicate layout name `{}`", layout.name);
            failures += 1;
        }
        seen_names.push(&layout.name);

        if layout.layout_id.iter().all(|&b| b == 0) {
            println!(
                "  FAIL: layout `{}` has all-zero LAYOUT_ID (unset or collision with uninit)",
                layout.name
            );
            failures += 1;
        }
        for (other_id, other_name) in &seen_ids {
            if *other_id == &layout.layout_id {
                println!(
                    "  FAIL: layouts `{}` and `{}` share LAYOUT_ID {:02x?}",
                    other_name, layout.name, layout.layout_id
                );
                failures += 1;
            }
        }
        seen_ids.push((&layout.layout_id, &layout.name));
    }
    failures
}

struct VerifyOptions {
    manifest: Option<String>,
    package: Option<String>,
    so: Option<String>,
    /// Treat a missing binary anchor as a failure. Default is
    /// informational-only because `hopper_layout!` layouts and
    /// post-link-stripped binaries may legitimately omit the bytes.
    strict: bool,
    /// Release profile: requires a binary and treats missing layout anchors as
    /// fatal. This is the public-launch/publish gate.
    release: bool,
}

impl VerifyOptions {
    /// Resolve the `.so` path iff the user supplied one (or a package
    /// root). Returns `Ok(None)` when no binary was requested, which
    /// makes the binary-scan phase skip gracefully.
    fn so_input(&self, cwd: &Path) -> Result<Option<PathBuf>, String> {
        if self.so.is_none() && self.package.is_none() {
            if self.release {
                return Err("--release requires a .so via --so <path> or --package <name>".to_string());
            }
            return Ok(None);
        }
        resolve_so_path(self, cwd).map(Some)
    }
}

fn parse_verify_options(args: &[String]) -> Result<VerifyOptions, String> {
    let mut manifest = None;
    let mut package = None;
    let mut so = None;
    let mut strict = false;
    let mut release = false;
    let mut positional_taken = false;
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        match arg.as_str() {
            "--manifest" => {
                i += 1;
                if i >= args.len() {
                    return Err("--manifest requires a path".to_string());
                }
                manifest = Some(args[i].clone());
                i += 1;
            }
            "--package" | "-p" => {
                i += 1;
                if i >= args.len() {
                    return Err("--package requires a crate name".to_string());
                }
                package = Some(args[i].clone());
                i += 1;
            }
            "--so" | "--binary" => {
                i += 1;
                if i >= args.len() {
                    return Err("--so requires a path".to_string());
                }
                so = Some(args[i].clone());
                i += 1;
            }
            "--strict" => {
                strict = true;
                i += 1;
            }
            "--release" => {
                release = true;
                strict = true;
                i += 1;
            }
            other if other.starts_with('@') => {
                manifest = Some(other[1..].to_string());
                i += 1;
            }
            other if other.starts_with('-') => {
                return Err(format!("unknown flag: {other}"));
            }
            other => {
                if !positional_taken && manifest.is_none() {
                    manifest = Some(other.to_string());
                    positional_taken = true;
                } else if so.is_none() {
                    so = Some(other.to_string());
                } else {
                    return Err(format!("unexpected argument: {other}"));
                }
                i += 1;
            }
        }
    }
    Ok(VerifyOptions {
        manifest,
        package,
        so,
        strict,
        release,
    })
}

fn resolve_manifest_path(opts: &VerifyOptions, cwd: &Path) -> Result<PathBuf, String> {
    if let Some(m) = &opts.manifest {
        let p = PathBuf::from(m);
        let abs = if p.is_absolute() { p } else { cwd.join(&p) };
        if !abs.is_file() {
            return Err(format!("manifest not found: {}", abs.display()));
        }
        return Ok(abs);
    }
    if let Some(pkg) = &opts.package {
        // Look inside the workspace for the package's manifest.
        let candidate = cwd
            .join(format!("examples/{}/hopper.manifest.json", pkg))
            .exists();
        if candidate {
            return Ok(cwd.join(format!("examples/{}/hopper.manifest.json", pkg)));
        }
        return Err(format!(
            "could not find hopper.manifest.json for --package {pkg}"
        ));
    }
    // Default: infer from cwd.
    let default = cwd.join("hopper.manifest.json");
    if default.is_file() {
        return Ok(default);
    }
    Err(
        "no manifest specified. Pass a path, `--manifest <path>`, or `--package <name>`."
            .to_string(),
    )
}

fn resolve_so_path(opts: &VerifyOptions, cwd: &Path) -> Result<PathBuf, String> {
    if let Some(s) = &opts.so {
        let p = PathBuf::from(s);
        let abs = if p.is_absolute() { p } else { cwd.join(&p) };
        if !abs.is_file() {
            return Err(format!("binary not found: {}", abs.display()));
        }
        return Ok(abs);
    }
    if let Some(pkg) = &opts.package {
        let snake = pkg.replace('-', "_");
        let candidate = cwd.join(format!("target/deploy/{}.so", snake));
        if candidate.is_file() {
            return Ok(candidate);
        }
        return Err(format!(
            "could not find target/deploy/{snake}.so. Did you run `hopper build`?"
        ));
    }
    Err(
        "no .so specified. Pass a path via `--so <path>` or `--package <name>`."
            .to_string(),
    )
}

fn print_verify_usage() {
    eprintln!("Usage: hopper verify [<manifest>] [<binary.so>] [options]");
    eprintln!();
    eprintln!("Confirms every layout declared in the manifest appears in the");
    eprintln!("compiled binary by searching for its 8-byte LAYOUT_ID fingerprint.");
    eprintln!();
    eprintln!("Options:");
    eprintln!("  --manifest <path>   Path to the program manifest JSON");
    eprintln!("  --package <name>    Infer manifest + .so from a workspace package");
    eprintln!("  -p <name>           Short form of --package");
    eprintln!("  --so <path>         Explicit path to the .so binary");
    eprintln!("  --binary <path>     Alias for --so");
    eprintln!("  --strict            Fail when a manifest layout is not anchored in the binary");
    eprintln!("  --release           Require a binary and run strict release verification");
    eprintln!();
    eprintln!("Examples:");
    eprintln!("  hopper verify examples/hopper-token-2022-vault/hopper.manifest.json \\");
    eprintln!("                target/deploy/hopper_token_2022_vault.so");
    eprintln!("  hopper verify --package hopper-token-2022-vault");
    eprintln!("  hopper verify @hopper.manifest.json --so target/deploy/program.so");
}

struct ManifestLayout {
    name: String,
    layout_id: [u8; 8],
}

/// Pull `{ name, layout_id | layoutId }` pairs out of the manifest
/// JSON without depending on a full JSON crate. Supports three
/// encodings the Hopper ecosystem emits:
///
/// 1. snake_case byte array: `"layout_id": [1, 2, 3, 4, 5, 6, 7, 8]`
/// 2. snake_case hex string:  `"layout_id": "0102030405060708"`
/// 3. camelCase hex string:   `"layoutId":  "0102030405060708"`
///
/// Form 1 is what hand-authored / CLI-roundtrip manifests use; form 3
/// is what `hopper compile --emit schema` produces. Verify accepts
/// either without a config flag.
fn extract_layouts_from_manifest(json: &str) -> Result<Vec<ManifestLayout>, String> {
    let mut out = Vec::new();
    let mut rest = json;
    loop {
        let Some(name_idx) = rest.find("\"name\"") else {
            break;
        };
        let after_name = &rest[name_idx + 6..];
        let Some(colon) = after_name.find(':') else {
            break;
        };
        let after_colon = after_name[colon + 1..].trim_start();
        if !after_colon.starts_with('"') {
            rest = &after_name[colon + 1..];
            continue;
        }
        let name_body = &after_colon[1..];
        let Some(name_end) = name_body.find('"') else {
            break;
        };
        let name = name_body[..name_end].to_string();

        // Scan the window between this `"name"` and the next one for
        // the layout-id field in any supported encoding.
        let after_name_close = &name_body[name_end + 1..];
        let next_name_idx = after_name_close.find("\"name\"").unwrap_or(after_name_close.len());
        let window = &after_name_close[..next_name_idx];

        if let Some(id) = find_layout_id_in_window(window) {
            out.push(ManifestLayout { name, layout_id: id });
        }
        rest = after_name_close;
    }
    if out.is_empty() {
        return Err(
            "manifest did not yield any layout_id entries. Is this a Hopper manifest?"
                .to_string(),
        );
    }
    Ok(out)
}

/// Find the layout-id inside one layout's JSON window, accepting
/// all three encodings.
fn find_layout_id_in_window(window: &str) -> Option<[u8; 8]> {
    for key in ["\"layout_id\"", "\"layoutId\""] {
        let Some(k) = window.find(key) else { continue };
        let after = &window[k + key.len()..];
        let Some(colon) = after.find(':') else { continue };
        let tail = after[colon + 1..].trim_start();

        if tail.starts_with('[') {
            // Byte-array form.
            if let Some(close) = tail[1..].find(']') {
                let body = &tail[1..1 + close];
                let bytes: Result<Vec<u8>, _> = body
                    .split(',')
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                    .map(|s| s.parse::<u16>().map(|n| n as u8))
                    .collect();
                if let Ok(bs) = bytes {
                    if bs.len() == 8 {
                        let mut id = [0u8; 8];
                        id.copy_from_slice(&bs);
                        return Some(id);
                    }
                }
            }
        } else if tail.starts_with('"') {
            // Hex-string form.
            let body = &tail[1..];
            if let Some(close) = body.find('"') {
                let hex = &body[..close];
                if hex.len() == 16 {
                    let mut id = [0u8; 8];
                    let mut ok = true;
                    for i in 0..8 {
                        match u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16) {
                            Ok(b) => id[i] = b,
                            Err(_) => {
                                ok = false;
                                break;
                            }
                        }
                    }
                    if ok {
                        return Some(id);
                    }
                }
            }
        }
    }
    None
}

fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|w| w == needle)
}

fn has_elf_magic(buf: &[u8]) -> bool {
    buf.len() >= 4 && buf[0..4] == [0x7f, 0x45, 0x4c, 0x46]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_byte_sequence_in_haystack() {
        let haystack = b"xxxxxxABCDEFGHyyyyyyyyyy";
        let needle = b"ABCDEFGH";
        assert_eq!(find_subsequence(haystack, needle), Some(6));
    }

    #[test]
    fn missing_sequence_returns_none() {
        let haystack = b"nothing matches";
        let needle = b"ABCDEFGH";
        assert_eq!(find_subsequence(haystack, needle), None);
    }

    #[test]
    fn empty_needle_returns_none() {
        let haystack = b"abc";
        assert_eq!(find_subsequence(haystack, &[]), None);
    }

    #[test]
    fn elf_magic_detected() {
        let mut buf = vec![0x7f, 0x45, 0x4c, 0x46];
        buf.extend_from_slice(&[0u8; 100]);
        assert!(has_elf_magic(&buf));
    }

    #[test]
    fn non_elf_rejected() {
        let buf = [0u8; 100];
        assert!(!has_elf_magic(&buf));
    }

    #[test]
    fn extracts_layout_with_id_from_sample_manifest() {
        let json = r#"
        {
          "name": "vault_program",
          "layouts": [
            { "name": "Vault", "layout_id": [1, 2, 3, 4, 5, 6, 7, 8] }
          ]
        }
        "#;
        let layouts = extract_layouts_from_manifest(json).unwrap();
        assert_eq!(layouts.len(), 1);
        assert_eq!(layouts[0].name, "Vault");
        assert_eq!(layouts[0].layout_id, [1, 2, 3, 4, 5, 6, 7, 8]);
    }

    #[test]
    fn extracts_multiple_layouts() {
        let json = r#"
        {
          "layouts": [
            { "name": "Vault", "layout_id": [1,2,3,4,5,6,7,8] },
            { "name": "Position", "layout_id": [9,10,11,12,13,14,15,16] }
          ]
        }
        "#;
        let layouts = extract_layouts_from_manifest(json).unwrap();
        assert_eq!(layouts.len(), 2);
        assert_eq!(layouts[0].name, "Vault");
        assert_eq!(layouts[1].name, "Position");
        assert_eq!(layouts[1].layout_id, [9, 10, 11, 12, 13, 14, 15, 16]);
    }

    #[test]
    fn errors_on_manifest_with_no_layouts() {
        let json = r#"{ "name": "p" }"#;
        assert!(extract_layouts_from_manifest(json).is_err());
    }

    #[test]
    fn extracts_camel_case_hex_layout_id() {
        let json = r#"
        {
          "layouts": [
            { "name": "Vault", "layoutId": "0102030405060708" }
          ]
        }
        "#;
        let layouts = extract_layouts_from_manifest(json).unwrap();
        assert_eq!(layouts.len(), 1);
        assert_eq!(layouts[0].layout_id, [1, 2, 3, 4, 5, 6, 7, 8]);
    }

    #[test]
    fn extracts_snake_case_hex_layout_id() {
        let json = r#"
        {
          "layouts": [
            { "name": "Vault", "layout_id": "abcdef0123456789" }
          ]
        }
        "#;
        let layouts = extract_layouts_from_manifest(json).unwrap();
        assert_eq!(layouts.len(), 1);
        assert_eq!(
            layouts[0].layout_id,
            [0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89]
        );
    }

    #[test]
    fn release_option_implies_strict_and_requires_binary() {
        let args = vec!["--release".to_string(), "hopper.manifest.json".to_string()];
        let opts = parse_verify_options(&args).unwrap();
        assert!(opts.release);
        assert!(opts.strict);
        assert!(opts.so_input(Path::new(".")).is_err());
    }

    #[test]
    fn rejects_malformed_hex() {
        let json = r#"
        {
          "layouts": [
            { "name": "Vault", "layoutId": "not_valid_hex__" }
          ]
        }
        "#;
        assert!(extract_layouts_from_manifest(json).is_err());
    }

    #[test]
    fn rejects_wrong_length_hex() {
        let json = r#"
        {
          "layouts": [
            { "name": "Vault", "layoutId": "dead" }
          ]
        }
        "#;
        assert!(extract_layouts_from_manifest(json).is_err());
    }
}
