//! `hopper verify` - interface integrity check between a program manifest
//! and its compiled `.so` binary.
//!
//! Release verification compares the manifest's canonical executable-interface
//! commitment with the versioned binding record emitted into the ELF by
//! `hopper::program_manifest!`. The record covers program identity/version,
//! layouts, instruction wire data and account contracts, events, policy
//! contracts, and context constraints. Descriptive and measured metadata is
//! deliberately excluded.
//!
//! The check is byte-level and deliberately offline: no Solana RPC or linker
//! consultation. Per-layout `LAYOUT_ID` searches remain supplemental
//! diagnostics. They are informational by default and fatal only with
//! `--strict`; `--release` gates on the structured interface binding instead.

use std::fs;
use std::path::{Path, PathBuf};
use std::process;

use crate::workspace;
use hopper_schema::release_binding::{
    interface_commitment, RELEASE_BINDING_COMMITMENT_OFFSET, RELEASE_BINDING_FORMAT_VERSION,
    RELEASE_BINDING_HASH_SHA256, RELEASE_BINDING_MAGIC, RELEASE_BINDING_RECORD_LEN,
};

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

    let release_commitment = opts.release.then(|| {
        let owned = crate::parse_program_manifest_json(&manifest_json).unwrap_or_else(|err| {
            eprintln!("hopper verify: cannot parse release manifest: {err}");
            process::exit(1);
        });
        interface_commitment(&crate::to_program_manifest(&owned))
    });

    let layouts = extract_layouts_from_manifest(&manifest_json).unwrap_or_else(|err| {
        eprintln!("hopper verify: {err}");
        process::exit(1);
    });

    // ── Stage 1: manifest integrity (always runs, always gates) ────
    //
    // Catches the refactor mistakes no amount of SBF inspection can:
    // duplicate layout IDs, all-zero IDs, and empty names. These are cheap,
    // unambiguous, and always fatal. This lightweight scan does not parse
    // discriminator/version pairs.
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
    println!("  OK: unique layout_id, non-zero bytes, valid names.");

    // ── Stage 1.5: effect gate (C7, opt-in via --effects) ──────────
    //
    // Composes the three shipped layers into one release check: the
    // program's emitted touch map (acquired) is verified against the
    // manifest's published write ranges (authorized) and the observed
    // byte diff (changed), independently, per `changed ⊆ acquired ⊆
    // authorized`. Any bundle that violates fails the command, the gate
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
        if opts.allow_inconclusive {
            // With the waiver active, an INCONCLUSIVE bundle contributes no
            // failure but was never byte-checked, the success line must
            // claim only what was verified.
            println!(
                "  OK: no violations; every VERIFIED bundle's writes are within its declared \
                 authorization (INCONCLUSIVE rows above, if any, were waived, not verified)."
            );
        } else {
            println!("  OK: every bundle's actual writes are within its declared authorization.");
        }
    }

    // ── Stage 1.75: authority gate (opt-in via --authority-baseline) ──
    //
    // Upgrade review: diff the released baseline manifest against this one
    // and fail when any instruction gains authority (a dropped signer, a new
    // writable account, wider byte ranges, a removed PDA or has_one binding,
    // a new CPI program, a lamport permission). Compatibility checkers treat
    // most of these as safe additive changes; this gate treats them as
    // changes someone has to sign off on.
    if let Some(baseline_path) = &opts.authority_baseline {
        println!();
        println!("Authority gate: baseline {baseline_path}");
        run_authority_gate(&opts, baseline_path, &manifest_json);
    }

    // ── Stage 2: binary verification ──
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

    if let Some(expected) = release_commitment {
        println!();
        println!("Release interface binding:");
        println!("  expected: {}", hex_bytes(&expected));
        match verify_release_binding(&binary, expected) {
            Ok(binding) => {
                println!("  binary:   {}", hex_bytes(&binding.commitment));
                println!(
                    "  OK: v{} SHA-256 commitment matched at 0x{:06x}.",
                    RELEASE_BINDING_FORMAT_VERSION, binding.offset
                );
            }
            Err(err) => {
                eprintln!("  FAIL: {err}");
                process::exit(1);
            }
        }
    }

    // Raw layout-ID occurrences remain useful diagnostics, but are not the
    // release-interface proof.
    println!();
    println!("Layout-anchor diagnostics:");
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
        "  layout-anchor presence: {} of {} layouts found in the ELF",
        found_count,
        layouts.len()
    );

    if missing_count > 0 {
        if opts.strict {
            eprintln!();
            eprintln!(
                "FAIL (--strict): {} of {} layouts not anchored in {}",
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
        println!(
            "OK: release interface binding matched; layout anchors above are supplemental diagnostics."
        );
    } else {
        println!("OK: manifest integrity passed; binary presence reported above.");
    }
}

/// Exit code for an unapproved authority widening.
const EXIT_AUTHORITY_WIDENED: i32 = 2;
/// Exit code for an unapproved change that needs review.
const EXIT_AUTHORITY_REVIEW: i32 = 3;

fn run_authority_gate(opts: &VerifyOptions, baseline_path: &str, manifest_json: &str) {
    use grillo_verifier::authority::{
        ApprovalError, AuthorityDiff, AuthorityReport, AuthorityVerdict,
    };

    let baseline_json = fs::read_to_string(baseline_path).unwrap_or_else(|err| {
        eprintln!("hopper verify: failed to read authority baseline: {err}");
        process::exit(1);
    });

    match &opts.baseline_so {
        Some(so) => {
            let owned = crate::parse_program_manifest_json(&baseline_json).unwrap_or_else(|err| {
                eprintln!("hopper verify: cannot parse baseline manifest: {err}");
                process::exit(1);
            });
            let expected = interface_commitment(&crate::to_program_manifest(&owned));
            let binary = fs::read(so).unwrap_or_else(|err| {
                eprintln!("hopper verify: failed to read baseline binary: {err}");
                process::exit(1);
            });
            if !has_elf_magic(&binary) {
                eprintln!("hopper verify: baseline {so} is not an ELF binary");
                process::exit(1);
            }
            match verify_release_binding(&binary, expected) {
                Ok(binding) => println!(
                    "  baseline bound: {} matches the commitment at 0x{:06x} in {so}",
                    hex_bytes(&expected),
                    binding.offset
                ),
                Err(err) => {
                    eprintln!("  FAIL: baseline manifest is not the one released in {so}: {err}");
                    process::exit(1);
                }
            }
        }
        None if opts.release => {
            eprintln!(
                "  FAIL: --release with --authority-baseline also requires --baseline-so, so the \
                 baseline is the manifest actually committed in the released ELF"
            );
            process::exit(1);
        }
        None => println!("  baseline bound: no (pass --baseline-so to bind it to its ELF)"),
    }

    let report = AuthorityDiff::between_json(&baseline_json, manifest_json).unwrap_or_else(|err| {
        eprintln!("hopper verify: authority diff refused its inputs: {err}");
        process::exit(1);
    });
    for line in report.render().lines() {
        println!("  {line}");
    }
    if let Some(out) = &opts.authority_report {
        fs::write(out, report.to_json()).unwrap_or_else(|err| {
            eprintln!("hopper verify: failed to write authority report {out}: {err}");
            process::exit(1);
        });
        println!("  report written: {out}");
    }

    let verdict = report.verdict();
    if verdict == AuthorityVerdict::NotWidened {
        println!("  OK: no instruction gained authority.");
        return;
    }
    let exit_code = if verdict == AuthorityVerdict::Widened {
        EXIT_AUTHORITY_WIDENED
    } else {
        EXIT_AUTHORITY_REVIEW
    };
    let Some(approval_path) = &opts.authority_approval else {
        eprintln!(
            "  FAIL: authority {}. Review the findings above, then pass the reviewed \
             --authority-report output as --authority-approval.",
            verdict.label().to_lowercase()
        );
        process::exit(exit_code);
    };
    let approval = fs::read_to_string(approval_path)
        .map_err(|err| err.to_string())
        .and_then(|json| AuthorityReport::from_json(&json).map_err(|err| err.to_string()))
        .unwrap_or_else(|err| {
            eprintln!("hopper verify: cannot read authority approval {approval_path}: {err}");
            process::exit(1);
        });
    match report.check_approval(&approval) {
        Ok(()) => println!("  OK: every widening is covered by approval {approval_path}."),
        Err(ApprovalError::Unapproved(findings)) => {
            eprintln!(
                "  FAIL: {} change(s) are not in the approval:",
                findings.len()
            );
            for f in &findings {
                eprintln!(
                    "    {} {} {} {}",
                    f.impact.label(),
                    f.instruction,
                    f.code,
                    f.detail
                );
            }
            process::exit(exit_code);
        }
        Err(err) => {
            eprintln!("  FAIL: {err}");
            process::exit(exit_code);
        }
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
    /// Release profile: requires a binary and an exact versioned
    /// executable-interface commitment. This is the public-launch/publish gate.
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
    /// Authority gate: a previously released manifest to diff against. The
    /// gate fails when the current manifest grants any instruction more
    /// authority than this baseline did.
    authority_baseline: Option<String>,
    /// The baseline release's `.so`. When present, the baseline manifest must
    /// match the interface commitment embedded in that ELF. Required under
    /// `--release`, so neither side of the diff is an unbound declaration.
    baseline_so: Option<String>,
    /// A reviewed authority report (`--authority-report` output) that
    /// approves the listed widenings for exactly this manifest pair.
    authority_approval: Option<String>,
    /// Write the authority report JSON here.
    authority_report: Option<String>,
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
    let mut authority_baseline = None;
    let mut baseline_so = None;
    let mut authority_approval = None;
    let mut authority_report = None;
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
            "--authority-baseline"
            | "--baseline-so"
            | "--authority-approval"
            | "--authority-report" => {
                let flag = arg.clone();
                i += 1;
                if i >= args.len() {
                    return Err(format!("{flag} requires a path"));
                }
                let value = Some(args[i].clone());
                match flag.as_str() {
                    "--authority-baseline" => authority_baseline = value,
                    "--baseline-so" => baseline_so = value,
                    "--authority-approval" => authority_approval = value,
                    _ => authority_report = value,
                }
                i += 1;
            }
            "--strict" => {
                strict = true;
                i += 1;
            }
            "--release" => {
                release = true;
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
        authority_baseline,
        baseline_so,
        authority_approval,
        authority_report,
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
    // (sorted for deterministic output). Fail-closed throughout: an entry
    // the directory scan cannot read counts as a failure, a bundle that
    // may exist but could not be enumerated is a bundle that did not
    // verify. The extension match is ASCII-case-insensitive so a
    // `REGRESSION.JSON` dropped in by a Windows tool is verified, not
    // silently skipped.
    let mut failures = 0u32;
    let p = Path::new(path);
    let mut bundles: Vec<PathBuf> = if p.is_dir() {
        match fs::read_dir(p) {
            Ok(entries) => {
                let mut v: Vec<PathBuf> = Vec::new();
                for entry in entries {
                    match entry {
                        Ok(e) => {
                            let q = e.path();
                            let is_json = q.extension().is_some_and(|x| {
                                x.to_str().is_some_and(|s| s.eq_ignore_ascii_case("json"))
                            });
                            if is_json {
                                v.push(q);
                            }
                        }
                        Err(err) => {
                            eprintln!(
                                "  effect gate: unreadable directory entry under {path}: {err}"
                            );
                            failures += 1;
                        }
                    }
                }
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
    if bundles.is_empty() && failures == 0 {
        eprintln!("  effect gate: no *.json evidence bundles found under {path}");
        return 1;
    }
    bundles.sort();

    println!("{:<36} {:<12} Detail", "Bundle", "Verdict");
    println!("{}", "-".repeat(80));

    let mut verified_pass = 0u32;
    let mut waived_inconclusive = 0u32;
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
                verified_pass += 1;
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
                if allow_inconclusive {
                    // Waived is NOT verified: the bundle's writes were never
                    // checked against the contract, and the summary must
                    // never fold it into the passing count.
                    waived_inconclusive += 1;
                } else {
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
        "  {} bundle(s) checked, {} verified passing, {} waived inconclusive, {} not passing{}",
        bundles.len(),
        verified_pass,
        waived_inconclusive,
        failures,
        if allow_inconclusive {
            " (inconclusive waived, NOT verified)"
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
    eprintln!("Confirms manifest integrity and reports per-layout anchor presence.");
    eprintln!("--release also requires the ELF's versioned executable-interface");
    eprintln!("commitment to match the canonical manifest commitment exactly.");
    eprintln!();
    eprintln!("Options:");
    eprintln!("  --manifest <path>   Path to the program manifest JSON");
    eprintln!("  --package <name>    Infer manifest + .so from a workspace package");
    eprintln!("  -p <name>           Short form of --package");
    eprintln!("  --so <path>         Explicit path to the .so binary");
    eprintln!("  --binary <path>     Alias for --so");
    eprintln!("  --strict            Fail when a manifest layout is not anchored in the binary");
    eprintln!("  --release           Require an exact versioned interface binding in the ELF");
    eprintln!("  --effects <path>    Effect gate: verify an evidence bundle (or a directory");
    eprintln!("                      of *.json bundles) against the manifest's published");
    eprintln!("                      write contract via the independent Grillo verifier");
    eprintln!("                      (changed \u{2286} acquired \u{2286} authorized). Any violation fails.");
    eprintln!("  --allow-inconclusive  Treat a Grillo INCONCLUSIVE bundle as a pass in the");
    eprintln!("                      effect gate (default: inconclusive is fatal)");
    eprintln!("  --authority-baseline <path>");
    eprintln!("                      Authority gate: diff a released manifest against this");
    eprintln!("                      one. Exit 2 when an instruction gains authority, 3 when");
    eprintln!("                      a change needs review (seed swap, new CPI program)");
    eprintln!("  --baseline-so <path> Bind the baseline to its released ELF commitment");
    eprintln!("                      (required with --release)");
    eprintln!("  --authority-report <path>    Write the authority report JSON");
    eprintln!("  --authority-approval <path>  A reviewed report that approves its listed");
    eprintln!("                      changes for exactly this manifest pair");
    eprintln!();
    eprintln!("Examples:");
    eprintln!("  hopper verify examples/hopper-token-2022-vault/hopper.manifest.json \\");
    eprintln!("                target/deploy/hopper_token_2022_vault.so");
    eprintln!("  hopper verify --package hopper-token-2022-vault");
    eprintln!("  hopper verify @hopper.manifest.json --so target/deploy/program.so");
    eprintln!("  hopper verify --manifest hopper.manifest.json --effects tests/bundles/");
    eprintln!("  hopper verify --release hopper.manifest.json target/deploy/program.so \\");
    eprintln!("                --authority-baseline release/v1/hopper.manifest.json \\");
    eprintln!("                --baseline-so release/v1/program.so");
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
        let parsed: serde_json::Value = serde_json::from_str(json)
            .map_err(|error| format!("invalid manifest JSON: {error}"))?;
        let explicitly_empty = parsed
            .get("layouts")
            .and_then(serde_json::Value::as_array)
            .is_some_and(Vec::is_empty);
        if !explicitly_empty {
            return Err(
                "manifest did not yield any layout_id entries. Is this a Hopper manifest?"
                    .to_string(),
            );
        }
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ReleaseBindingMatch {
    offset: usize,
    commitment: [u8; 32],
}

/// Validate every structured Hopper binding record in an ELF and return the
/// first exact match. Multiple identical records are harmless, but malformed,
/// unsupported, stale, or conflicting records fail closed so a fat/stale
/// artifact cannot pass because one embedded object happened to match.
fn verify_release_binding(
    binary: &[u8],
    expected: [u8; 32],
) -> Result<ReleaseBindingMatch, String> {
    let offsets: Vec<usize> = binary
        .windows(RELEASE_BINDING_MAGIC.len())
        .enumerate()
        .filter_map(|(offset, bytes)| (bytes == RELEASE_BINDING_MAGIC).then_some(offset))
        .collect();
    if offsets.is_empty() {
        return Err(
            "no versioned Hopper release-interface binding record was found; rebuild with \
             hopper::program_manifest!"
                .to_string(),
        );
    }

    let mut matched = None;
    for offset in offsets {
        let remaining = &binary[offset..];
        if remaining.len() < RELEASE_BINDING_RECORD_LEN {
            return Err(format!(
                "truncated Hopper release-interface binding record at 0x{offset:06x}"
            ));
        }
        let record = &remaining[..RELEASE_BINDING_RECORD_LEN];
        let version = u16::from_le_bytes([record[16], record[17]]);
        if version != RELEASE_BINDING_FORMAT_VERSION {
            return Err(format!(
                "unsupported Hopper release-interface binding version {version} at \
                 0x{offset:06x}"
            ));
        }
        if record[18] != RELEASE_BINDING_HASH_SHA256 {
            return Err(format!(
                "unsupported Hopper release-interface hash algorithm {} at 0x{offset:06x}",
                record[18]
            ));
        }
        if record[19] != 0 {
            return Err(format!(
                "unsupported Hopper release-interface binding flags 0x{:02x} at 0x{offset:06x}",
                record[19]
            ));
        }
        let declared_len =
            u32::from_le_bytes([record[20], record[21], record[22], record[23]]) as usize;
        if declared_len != RELEASE_BINDING_RECORD_LEN {
            return Err(format!(
                "invalid Hopper release-interface record length {declared_len} at \
                 0x{offset:06x}; expected {RELEASE_BINDING_RECORD_LEN}"
            ));
        }

        let mut commitment = [0u8; 32];
        commitment.copy_from_slice(
            &record[RELEASE_BINDING_COMMITMENT_OFFSET..RELEASE_BINDING_COMMITMENT_OFFSET + 32],
        );
        if commitment != expected {
            return Err(format!(
                "manifest commitment {} does not match binary commitment {} at 0x{offset:06x}",
                hex_bytes(&expected),
                hex_bytes(&commitment)
            ));
        }
        matched.get_or_insert(ReleaseBindingMatch { offset, commitment });
    }

    Ok(matched.expect("at least one binding offset was validated"))
}

fn hex_bytes(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
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
    fn accepts_an_explicitly_stateless_manifest() {
        let json = r#"{ "name": "p", "layouts": [] }"#;
        assert!(extract_layouts_from_manifest(json).unwrap().is_empty());
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
    fn release_option_requires_binary_without_enabling_legacy_anchor_strictness() {
        let args = vec!["--release".to_string(), "hopper.manifest.json".to_string()];
        let opts = parse_verify_options(&args).unwrap();
        assert!(opts.release);
        assert!(!opts.strict);
        assert!(opts.so_input(Path::new(".")).is_err());
    }

    #[test]
    fn authority_gate_flags_parse_without_consuming_positionals() {
        let args: Vec<String> = [
            "new.json",
            "--authority-baseline",
            "old.json",
            "--baseline-so",
            "old.so",
            "--authority-report",
            "report.json",
            "--authority-approval",
            "approved.json",
            "new.so",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let opts = parse_verify_options(&args).unwrap();
        assert_eq!(opts.manifest.as_deref(), Some("new.json"));
        assert_eq!(opts.so.as_deref(), Some("new.so"));
        assert_eq!(opts.authority_baseline.as_deref(), Some("old.json"));
        assert_eq!(opts.baseline_so.as_deref(), Some("old.so"));
        assert_eq!(opts.authority_report.as_deref(), Some("report.json"));
        assert_eq!(opts.authority_approval.as_deref(), Some("approved.json"));

        let missing = vec!["--authority-baseline".to_string()];
        assert!(parse_verify_options(&missing).is_err());
    }

    fn test_binding_record(commitment: [u8; 32]) -> [u8; RELEASE_BINDING_RECORD_LEN] {
        let mut record = [0u8; RELEASE_BINDING_RECORD_LEN];
        record[..RELEASE_BINDING_MAGIC.len()].copy_from_slice(&RELEASE_BINDING_MAGIC);
        record[16..18].copy_from_slice(&RELEASE_BINDING_FORMAT_VERSION.to_le_bytes());
        record[18] = RELEASE_BINDING_HASH_SHA256;
        record[20..24].copy_from_slice(&(RELEASE_BINDING_RECORD_LEN as u32).to_le_bytes());
        record[RELEASE_BINDING_COMMITMENT_OFFSET..].copy_from_slice(&commitment);
        record
    }

    fn fake_elf_with_record(record: &[u8]) -> Vec<u8> {
        let mut binary = b"\x7fELFtest-padding".to_vec();
        binary.extend_from_slice(record);
        binary.extend_from_slice(b"trailing-bytes");
        binary
    }

    #[test]
    fn release_binding_accepts_the_exact_versioned_commitment() {
        let expected = [0x5a; 32];
        let binary = fake_elf_with_record(&test_binding_record(expected));
        let matched = verify_release_binding(&binary, expected).unwrap();
        assert_eq!(matched.offset, b"\x7fELFtest-padding".len());
        assert_eq!(matched.commitment, expected);
    }

    #[test]
    fn raw_layout_bytes_cannot_satisfy_the_release_binding() {
        let mut binary = b"\x7fELF".to_vec();
        binary.extend_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8]);
        let error = verify_release_binding(&binary, [1; 32]).unwrap_err();
        assert!(error.contains("no versioned Hopper release-interface binding"));
    }

    #[test]
    fn release_binding_rejects_a_stale_commitment() {
        let binary = fake_elf_with_record(&test_binding_record([0x11; 32]));
        let error = verify_release_binding(&binary, [0x22; 32]).unwrap_err();
        assert!(error.contains("manifest commitment"));
        assert!(error.contains(&hex_bytes(&[0x11; 32])));
        assert!(error.contains(&hex_bytes(&[0x22; 32])));
    }

    #[test]
    fn release_binding_rejects_an_unsupported_record_version() {
        let mut record = test_binding_record([0x33; 32]);
        record[16..18].copy_from_slice(&2u16.to_le_bytes());
        let error = verify_release_binding(&fake_elf_with_record(&record), [0x33; 32]).unwrap_err();
        assert!(error.contains("unsupported Hopper release-interface binding version 2"));
    }

    #[test]
    fn release_binding_rejects_stale_and_current_records_in_one_binary() {
        let expected = [0x44; 32];
        let mut binary = fake_elf_with_record(&test_binding_record(expected));
        binary.extend_from_slice(&test_binding_record([0x45; 32]));
        let error = verify_release_binding(&binary, expected).unwrap_err();
        assert!(error.contains(&hex_bytes(&[0x45; 32])));
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

    fn write_bundle_dir(tag: &str, files: &[(&str, String)]) -> std::path::PathBuf {
        // A unique temp dir per test: process id + a caller tag (tests run
        // in parallel threads of ONE process, so a length-derived name
        // would collide between two tests with equal file counts).
        let mut dir = std::env::temp_dir();
        dir.push(format!("hopper-effect-gate-{}-{}", std::process::id(), tag));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        for (name, body) in files {
            fs::write(dir.join(name), body).unwrap();
        }
        dir
    }

    #[test]
    fn effect_gate_passes_an_honest_corpus() {
        let dir = write_bundle_dir("honest", &[("ok.json", pause_bundle(114))]);
        let failures = run_effect_gate(GATE_MANIFEST, dir.to_str().unwrap(), false);
        let _ = fs::remove_dir_all(&dir);
        assert_eq!(failures, 0, "an in-range write must pass the gate");
    }

    #[test]
    fn effect_gate_fails_on_an_out_of_range_write() {
        let dir = write_bundle_dir(
            "oob",
            &[
                ("ok.json", pause_bundle(114)),
                ("bad.json", pause_bundle(115)), // neighbor byte, undeclared
            ],
        );
        let failures = run_effect_gate(GATE_MANIFEST, dir.to_str().unwrap(), false);
        let _ = fs::remove_dir_all(&dir);
        assert_eq!(failures, 1, "one bundle wrote outside its declared range");
    }

    #[test]
    fn effect_gate_reports_empty_corpus_as_a_failure() {
        let dir = write_bundle_dir("empty", &[]);
        let failures = run_effect_gate(GATE_MANIFEST, dir.to_str().unwrap(), false);
        let _ = fs::remove_dir_all(&dir);
        assert_eq!(failures, 1, "no bundles is not a corpus that verified");
    }

    #[test]
    fn effect_gate_collects_uppercase_json_bundles() {
        // A violating bundle named `.JSON` (the Windows-capture shape) must
        // be collected and verified, silent case-sensitive exclusion would
        // let the release gate go green around it.
        let dir = write_bundle_dir(
            "case",
            &[
                ("ok.json", pause_bundle(114)),
                ("REGRESSION.JSON", pause_bundle(115)),
            ],
        );
        let failures = run_effect_gate(GATE_MANIFEST, dir.to_str().unwrap(), false);
        let _ = fs::remove_dir_all(&dir);
        assert_eq!(
            failures, 1,
            "an uppercase .JSON bundle must be verified, not skipped"
        );
    }
}
