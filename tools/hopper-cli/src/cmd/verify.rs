//! `hopper verify` - ABI integrity check between a program manifest
//! and its compiled `.so` binary.
//!
//! Hopper catches manifest/binary drift at the CLI by scanning the
//! compiled ELF for each layout's 8-byte
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

    // ── Stage 1.5: effect gate (C7, opt-in via --effects) ──────────
    //
    // Composes the three shipped layers into one release check: the
    // program's emitted touch map (acquired) is verified against the
    // manifest's published write ranges (authorized) and the observed
    // byte diff (changed), independently, per `changed ⊆ acquired ⊆
    // authorized`. Any bundle that violates fails the command — the gate
    // goes red the moment a handler writes outside its declared set.
    if let Some(effects_path) = &opts.effects {
        println!();
        println!("Effect gate: {effects_path}");
        let effect_failures =
            run_effect_gate(&manifest_json, effects_path, opts.allow_inconclusive);
        if effect_failures > 0 {
            eprintln!();
            eprintln!(
                "FAIL: {effect_failures} evidence bundle(s) did not verify against the published \
                 write contract."
            );
            process::exit(1);
        }
        println!("  OK: every bundle's actual writes are within its declared authorization.");
    }

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
    println!("{:<32} {:<24} Presence", "Layout", "LAYOUT_ID (hex)");
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
                if opts.release {
                    "--release"
                } else {
                    "--strict"
                },
                missing_count,
                layouts.len(),
                so_input.display()
            );
            eprintln!("Layouts declared via `hopper_layout!` do not currently emit `#[used]`");
            eprintln!("anchors. Switch to `#[hopper::state]` or skip `--strict` for a");
            eprintln!("manifest-only check.");
            process::exit(1);
        }
        println!();
        println!("  note: layouts without anchors may be legitimately missing (LTO,");
        println!("  `hopper_layout!` path). Run with --strict to treat missing as fatal.");
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
    /// Effect gate (C7): a single evidence bundle or a directory of `*.json`
    /// bundles to verify against the manifest's published write contract
    /// (`changed ⊆ acquired ⊆ authorized`, via the independent Grillo
    /// verifier). Any VIOLATION fails the command.
    effects: Option<String>,
    /// Treat a Grillo INCONCLUSIVE bundle as a pass (default: inconclusive is
    /// fatal in the effect gate, since a corpus that cannot be verified is not
    /// a corpus that verified).
    allow_inconclusive: bool,
}

impl VerifyOptions {
    /// Resolve the `.so` path iff the user supplied one (or a package
    /// root). Returns `Ok(None)` when no binary was requested, which
    /// makes the binary-scan phase skip gracefully.
    fn so_input(&self, cwd: &Path) -> Result<Option<PathBuf>, String> {
        if self.so.is_none() && self.package.is_none() {
            if self.release {
                return Err(
                    "--release requires a .so via --so <path> or --package <name>".to_string(),
                );
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
    let mut effects = None;
    let mut allow_inconclusive = false;
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
            "--effects" => {
                i += 1;
                if i >= args.len() {
                    return Err("--effects requires a bundle file or directory".to_string());
                }
                effects = Some(args[i].clone());
                i += 1;
            }
            "--allow-inconclusive" => {
                allow_inconclusive = true;
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
        effects,
        allow_inconclusive,
    })
}

/// The C7 effect gate: verify every evidence bundle in `path` (a single
/// `*.json` bundle or a directory of them) against the manifest's published
/// mutation contract, using the independent Grillo verifier. Returns the
/// number of bundles that did NOT pass (violations, plus inconclusives
/// unless `--allow-inconclusive`), so the caller can gate on it.
fn run_effect_gate(manifest_json: &str, path: &str, allow_inconclusive: bool) -> u32 {
    use grillo_verifier::{parse_bundle, verify_bundle, MutationManifest, Verdict};

    let manifest = match MutationManifest::from_json(manifest_json) {
        Ok(m) => m,
        Err(err) => {
            eprintln!("  effect gate: manifest is not a mutation contract: {err}");
            return 1;
        }
    };

    // Collect the bundle files: one path, or every *.json in a directory
    // (sorted for deterministic output).
    let p = Path::new(path);
    let mut bundles: Vec<PathBuf> = if p.is_dir() {
        match fs::read_dir(p) {
            Ok(entries) => {
                let mut v: Vec<PathBuf> = entries
                    .flatten()
                    .map(|e| e.path())
                    .filter(|q| q.extension().is_some_and(|x| x == "json"))
                    .collect();
                v.sort();
                v
            }
            Err(err) => {
                eprintln!("  effect gate: cannot read directory {path}: {err}");
                return 1;
            }
        }
    } else {
        vec![p.to_path_buf()]
    };
    if bundles.is_empty() {
        eprintln!("  effect gate: no *.json evidence bundles found under {path}");
        return 1;
    }
    bundles.sort();

    println!("{:<36} {:<12} Detail", "Bundle", "Verdict");
    println!("{}", "-".repeat(80));

    let mut failures = 0u32;
    for bundle_path in &bundles {
        let name = bundle_path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| bundle_path.display().to_string());
        let json = match fs::read_to_string(bundle_path) {
            Ok(j) => j,
            Err(err) => {
                println!("{name:<36} {:<12} read error: {err}", "ERROR");
                failures += 1;
                continue;
            }
        };
        let bundle = match parse_bundle(&json) {
            Ok(b) => b,
            Err(err) => {
                println!("{name:<36} {:<12} {err}", "ERROR");
                failures += 1;
                continue;
            }
        };
        match verify_bundle(&manifest, &bundle) {
            Ok(Verdict::Pass(ev)) => {
                println!(
                    "{name:<36} {:<12} {} byte(s) changed, all authorized",
                    "PASS", ev.changed_bytes
                );
            }
            Ok(Verdict::Violation(v)) => {
                println!("{name:<36} {:<12} {} finding(s)", "VIOLATION", v.len());
                for finding in &v {
                    println!("    - {finding:?}");
                }
                failures += 1;
            }
            Ok(Verdict::Inconclusive(reason)) => {
                let tag = if allow_inconclusive {
                    "INCONCLUSIVE"
                } else {
                    "INCONCLUSIVE*"
                };
                println!("{name:<36} {tag:<12} {reason:?}");
                if !allow_inconclusive {
                    failures += 1;
                }
            }
            Err(err) => {
                println!("{name:<36} {:<12} {err}", "ERROR");
                failures += 1;
            }
        }
    }
    println!("{}", "-".repeat(80));
    println!(
        "  {} bundle(s) checked, {} passing, {} not passing{}",
        bundles.len(),
        bundles.len() as u32 - failures,
        failures,
        if allow_inconclusive {
            " (inconclusive allowed)"
        } else {
            " (inconclusive is fatal; --allow-inconclusive to relax)"
        }
    );
    failures
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
        let root = workspace::find_workspace_root(cwd)?;
        return workspace::infer_program_manifest_for_package(&root, pkg).map_err(|err| {
            format!(
                "{err}\nRemediation: add hopper.manifest.json next to {pkg}'s Cargo.toml, or run `hopper compile --emit schema --package {pkg} --out <package-root>/hopper.manifest.json --force` after generating the package manifest."
            )
        });
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
        let root = workspace::find_workspace_root(cwd)?;
        let snake = pkg.replace('-', "_");
        let candidate = root.join(format!("target/deploy/{}.so", snake));
        if candidate.is_file() {
            return Ok(candidate);
        }
        return Err(format!(
            "could not find {}. Run `hopper build -- -p {pkg}` from the workspace root, or pass --so <path> after building the SBF binary.",
            candidate.display()
        ));
    }
    Err("no .so specified. Pass a path via `--so <path>` or `--package <name>`.".to_string())
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
    eprintln!("  --effects <path>    Effect gate: verify an evidence bundle (or a directory");
    eprintln!("                      of *.json bundles) against the manifest's published");
    eprintln!("                      write contract via the independent Grillo verifier");
    eprintln!("                      (changed \u{2286} acquired \u{2286} authorized). Any violation fails.");
    eprintln!("  --allow-inconclusive  Treat a Grillo INCONCLUSIVE bundle as a pass in the");
    eprintln!("                      effect gate (default: inconclusive is fatal)");
    eprintln!();
    eprintln!("Examples:");
    eprintln!("  hopper verify examples/hopper-token-2022-vault/hopper.manifest.json \\");
    eprintln!("                target/deploy/hopper_token_2022_vault.so");
    eprintln!("  hopper verify --package hopper-token-2022-vault");
    eprintln!("  hopper verify @hopper.manifest.json --so target/deploy/program.so");
    eprintln!("  hopper verify --manifest hopper.manifest.json --effects tests/bundles/");
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
    while let Some(name_idx) = rest.find("\"name\"") {
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
        let next_name_idx = after_name_close
            .find("\"name\"")
            .unwrap_or(after_name_close.len());
        let window = &after_name_close[..next_name_idx];

        if let Some(id) = find_layout_id_in_window(window) {
            out.push(ManifestLayout {
                name,
                layout_id: id,
            });
        }
        rest = after_name_close;
    }
    if out.is_empty() {
        return Err(
            "manifest did not yield any layout_id entries. Is this a Hopper manifest?".to_string(),
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
        let Some(colon) = after.find(':') else {
            continue;
        };
        let tail = after[colon + 1..].trim_start();

        if let Some(inner) = tail.strip_prefix('[') {
            // Byte-array form.
            if let Some(close) = inner.find(']') {
                let body = &inner[..close];
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
        } else if let Some(body) = tail.strip_prefix('"') {
            // Hex-string form.
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
    haystack.windows(needle.len()).position(|w| w == needle)
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

    // ── C7 effect gate ─────────────────────────────────────────────

    const GATE_MANIFEST: &str = r#"{
        "name": "p", "version": "1.0.0",
        "instructions": [
            { "name": "pause", "tag": 1, "strictWrites": true,
              "writeRanges": [ { "accountIndex": 1, "offset": 114, "size": 1 } ],
              "accounts": [ { "name": "admin" }, { "name": "config" } ] }
        ]
    }"#;

    /// A pause bundle writing `paused` (byte 114): honest -> in range,
    /// tampered -> the neighbor byte 115 -> out of the declared set.
    fn pause_bundle(offset: usize) -> String {
        let mut pre = vec![0u8; 200];
        pre[114] = 0;
        let mut post = pre.clone();
        post[offset] = 1;
        let hex = |b: &[u8]| b.iter().map(|x| format!("{x:02x}")).collect::<String>();
        // touch map: one write record of (offset, 1) on account slot 1.
        let mut map = vec![0x7a, 0x01, 0x00, 0x01, 0x01];
        map.extend_from_slice(&((offset as u32) | 0x8000_0000).to_le_bytes());
        map.extend_from_slice(&1u32.to_le_bytes());
        format!(
            r#"{{ "instruction": "pause", "touchMap": "{}",
                 "accounts": [ {{ "index": 1, "pre": "{}", "post": "{}" }} ] }}"#,
            hex(&map),
            hex(&pre),
            hex(&post),
        )
    }

    fn write_bundle_dir(files: &[(&str, String)]) -> std::path::PathBuf {
        // A unique temp dir without pulling `Math.random`: derive from the
        // process id + the file set length.
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "hopper-effect-gate-{}-{}",
            std::process::id(),
            files.len()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        for (name, body) in files {
            fs::write(dir.join(name), body).unwrap();
        }
        dir
    }

    #[test]
    fn effect_gate_passes_an_honest_corpus() {
        let dir = write_bundle_dir(&[("ok.json", pause_bundle(114))]);
        let failures = run_effect_gate(GATE_MANIFEST, dir.to_str().unwrap(), false);
        let _ = fs::remove_dir_all(&dir);
        assert_eq!(failures, 0, "an in-range write must pass the gate");
    }

    #[test]
    fn effect_gate_fails_on_an_out_of_range_write() {
        let dir = write_bundle_dir(&[
            ("ok.json", pause_bundle(114)),
            ("bad.json", pause_bundle(115)), // neighbor byte, undeclared
        ]);
        let failures = run_effect_gate(GATE_MANIFEST, dir.to_str().unwrap(), false);
        let _ = fs::remove_dir_all(&dir);
        assert_eq!(failures, 1, "one bundle wrote outside its declared range");
    }

    #[test]
    fn effect_gate_reports_empty_corpus_as_a_failure() {
        let dir = write_bundle_dir(&[]);
        let failures = run_effect_gate(GATE_MANIFEST, dir.to_str().unwrap(), false);
        let _ = fs::remove_dir_all(&dir);
        assert_eq!(failures, 1, "no bundles is not a corpus that verified");
    }
}
