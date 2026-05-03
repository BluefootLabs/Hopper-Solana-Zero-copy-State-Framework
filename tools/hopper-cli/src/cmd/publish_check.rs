//! `hopper publish-check` - source and release gate for public Hopper releases.
//!
//! `hopper verify --release` proves a manifest and compiled binary agree on
//! layout fingerprints. `publish-check` wraps that binary check with the
//! source-tree gates called out by the safety audit: no stale benchmark
//! placeholders in release-facing docs, no Pinocchio in the default feature
//! tree, legacy SPL Token builders still feature-gated, client generators still
//! asserting layout IDs, and fuzz targets still present.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{self, Command};

use crate::workspace;

pub fn cmd_publish_check(args: &[String]) {
    if args.iter().any(|a| a == "--help" || a == "-h") {
        print_usage();
        return;
    }

    let opts = match PublishCheckOptions::parse(args) {
        Ok(opts) => opts,
        Err(err) => {
            eprintln!("hopper publish-check: {err}");
            print_usage();
            process::exit(1);
        }
    };

    let cwd = workspace::current_dir().unwrap_or_else(|err| {
        eprintln!("hopper publish-check: {err}");
        process::exit(1);
    });
    let root = opts.workspace_root(&cwd).unwrap_or_else(|err| {
        eprintln!("hopper publish-check: {err}");
        process::exit(1);
    });

    println!("hopper publish-check");
    println!("  workspace: {}", root.display());
    if opts.source_only {
        println!("  mode: source-only (binary ABI check skipped by explicit flag)");
    } else {
        println!("  mode: release (requires manifest + .so, or --package)");
    }
    if opts.full {
        println!("  depth: full");
    }

    let mut failures = 0u32;
    failures += run_stage("release ABI verification", || run_release_verify(&opts, &root)) as u32;
    failures += run_stage("release documentation scan", || scan_release_docs(&root)) as u32;
    failures += run_stage("default feature tree", || check_default_feature_tree(&root)) as u32;
    failures += run_stage("legacy token instruction gate", || check_legacy_token_gate(&root)) as u32;
    failures += run_stage("client layout-id assertions", || check_client_layout_tests(&root)) as u32;
    failures += run_stage("fuzz target inventory", || check_fuzz_inventory(&root)) as u32;

    if opts.full {
        failures += run_stage("core test suite", || run_cargo_status(&root, &["test", "-p", "hopper-core"])) as u32;
        failures += run_stage("trybuild UI suite", || run_cargo_status(&root, &["test", "-p", "hopper-trybuild"])) as u32;
    }

    println!();
    if failures == 0 {
        println!("OK: publish gate passed.");
    } else {
        eprintln!("FAIL: {failures} publish-check stage(s) failed.");
        process::exit(1);
    }
}

fn run_stage<F>(name: &str, f: F) -> bool
where
    F: FnOnce() -> Result<(), String>,
{
    println!();
    println!("{name}");
    println!("{}", "-".repeat(name.len()));
    match f() {
        Ok(()) => {
            println!("OK");
            false
        }
        Err(err) => {
            eprintln!("FAIL: {err}");
            true
        }
    }
}

#[derive(Default)]
struct PublishCheckOptions {
    manifest: Option<String>,
    so: Option<String>,
    package: Option<String>,
    workspace_root: Option<String>,
    source_only: bool,
    full: bool,
}

impl PublishCheckOptions {
    fn parse(args: &[String]) -> Result<Self, String> {
        let mut opts = Self::default();
        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "--manifest" => {
                    i += 1;
                    opts.manifest = Some(take_value(args, i, "--manifest")?);
                    i += 1;
                }
                "--so" | "--binary" => {
                    i += 1;
                    opts.so = Some(take_value(args, i, args[i - 1].as_str())?);
                    i += 1;
                }
                "--package" | "-p" => {
                    i += 1;
                    opts.package = Some(take_value(args, i, args[i - 1].as_str())?);
                    i += 1;
                }
                "--workspace-root" => {
                    i += 1;
                    opts.workspace_root = Some(take_value(args, i, "--workspace-root")?);
                    i += 1;
                }
                "--source-only" => {
                    opts.source_only = true;
                    i += 1;
                }
                "--full" => {
                    opts.full = true;
                    i += 1;
                }
                other if other.starts_with('-') => {
                    return Err(format!("unknown flag: {other}"));
                }
                other => {
                    if opts.manifest.is_none() {
                        opts.manifest = Some(other.to_string());
                    } else if opts.so.is_none() {
                        opts.so = Some(other.to_string());
                    } else {
                        return Err(format!("unexpected argument: {other}"));
                    }
                    i += 1;
                }
            }
        }

        if opts.source_only && (opts.manifest.is_some() || opts.so.is_some() || opts.package.is_some()) {
            return Err("--source-only cannot be combined with manifest, .so, or --package".to_string());
        }
        if opts.package.is_some() && (opts.manifest.is_some() || opts.so.is_some()) {
            return Err("use either --package or explicit --manifest/--so, not both".to_string());
        }
        Ok(opts)
    }

    fn workspace_root(&self, cwd: &Path) -> Result<PathBuf, String> {
        if let Some(root) = &self.workspace_root {
            let path = PathBuf::from(root);
            let abs = if path.is_absolute() { path } else { cwd.join(path) };
            if !abs.join("Cargo.toml").is_file() {
                return Err(format!("workspace root has no Cargo.toml: {}", abs.display()));
            }
            return Ok(abs);
        }
        workspace::find_workspace_root(cwd)
    }
}

fn take_value(args: &[String], index: usize, flag: &str) -> Result<String, String> {
    args.get(index)
        .cloned()
        .ok_or_else(|| format!("{flag} requires a value"))
}

fn run_release_verify(opts: &PublishCheckOptions, root: &Path) -> Result<(), String> {
    if opts.source_only {
        println!("skipped by --source-only; run with --package or --manifest/--so for release artifacts");
        return Ok(());
    }

    let mut verify_args = vec!["verify".to_string(), "--release".to_string()];
    if let Some(package) = &opts.package {
        verify_args.push("--package".to_string());
        verify_args.push(package.clone());
    } else {
        let manifest = opts.manifest.as_ref().ok_or_else(|| {
            "release mode requires --package or --manifest <path> plus --so <program.so>".to_string()
        })?;
        let so = opts.so.as_ref().ok_or_else(|| {
            "release mode requires --package or --manifest <path> plus --so <program.so>".to_string()
        })?;
        verify_args.push("--manifest".to_string());
        verify_args.push(manifest.clone());
        verify_args.push("--so".to_string());
        verify_args.push(so.clone());
    }

    let exe = std::env::current_exe().map_err(|err| format!("failed to locate hopper executable: {err}"))?;
    let status = Command::new(&exe)
        .args(&verify_args)
        .current_dir(root)
        .status()
        .map_err(|err| format!("failed to run {}: {err}", display_command(&exe, &verify_args)))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "{} exited with status {status}",
            display_command(&exe, &verify_args)
        ))
    }
}

fn scan_release_docs(root: &Path) -> Result<(), String> {
    let docs = [
        "README.md",
        "BENCHMARKS.md",
        "AUDIT.md",
        "docs/WHY_HOPPER.md",
        "docs/QUASAR_PINOCCHIO_REPLACEMENT.md",
        "docs/HOPPER_NATIVE_ENHANCEMENTS.md",
        "docs/UNSAFE_INVARIANTS.md",
        "docs/POLICY_GUARANTEES.md",
        "docs/CLI_REFERENCE.md",
    ];
    let banned = [
        ("re-run pending", "pending benchmark placeholder"),
        ("audit pending", "pending audit placeholder"),
        ("bench/hopper-bench", "stale in-tree benchmark path"),
        ("bench/pinocchio-vault", "stale in-tree benchmark path"),
        ("bench/anchor-vault", "stale in-tree benchmark path"),
        ("bench/lazy-dispatch-vault", "stale in-tree benchmark path"),
        ("bench/framework-vault-bench", "stale in-tree benchmark path"),
        ("bench/compare-framework-vaults", "stale in-tree benchmark path"),
        ("bench/methodology.md", "stale in-tree benchmark path"),
        ("bench/cu_baselines.toml", "stale in-tree benchmark path"),
    ];

    let mut hits = Vec::new();
    for rel in docs {
        let path = root.join(rel);
        if !path.is_file() {
            continue;
        }
        let text = fs::read_to_string(&path)
            .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
        let lower = text.to_lowercase();
        for (needle, reason) in banned {
            if lower.contains(needle) {
                hits.push(format!("{rel}: {reason}: `{needle}`"));
            }
        }
    }

    if hits.is_empty() {
        Ok(())
    } else {
        Err(format!("release docs contain stale markers:\n{}", hits.join("\n")))
    }
}

fn check_default_feature_tree(root: &Path) -> Result<(), String> {
    let args = ["tree", "-p", "hopper", "--edges", "normal,build"];
    let output = workspace::run_output("cargo", &to_strings(&args), root)?;
    if !output.status.success() {
        return Err(format!(
            "{} failed:\n{}",
            workspace::display_command("cargo", &to_strings(&args)),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let offenders: Vec<&str> = stdout
        .lines()
        .filter(|line| line.to_lowercase().contains("pinocchio"))
        .collect();
    if offenders.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "default Hopper dependency tree includes Pinocchio:\n{}",
            offenders.join("\n")
        ))
    }
}

fn check_legacy_token_gate(root: &Path) -> Result<(), String> {
    let lib = root.join("crates/hopper-spl/hopper-token/src/lib.rs");
    let text = fs::read_to_string(&lib)
        .map_err(|err| format!("failed to read {}: {err}", lib.display()))?;
    let lines: Vec<&str> = text.lines().collect();
    let mut unguarded = Vec::new();
    for (idx, line) in lines.iter().enumerate() {
        if !line.contains("Approve, Burn, MintTo, Transfer") {
            continue;
        }
        let guarded = idx > 0 && lines[idx - 1].contains("legacy-token-instructions")
            || idx > 1 && lines[idx - 2].contains("legacy-token-instructions");
        if !guarded {
            unguarded.push(idx + 1);
        }
    }
    if !unguarded.is_empty() {
        return Err(format!(
            "deprecated plain SPL Token builders are exported without cfg at {} lines {:?}",
            lib.display(),
            unguarded
        ));
    }

    // Disable the token crate's default features so this check proves the
    // deprecated plain builders are not required by the public default API,
    // but keep an explicit backend enabled because hopper-runtime has no
    // backend-free build mode.
    run_cargo_status(
        root,
        &[
            "check",
            "-p",
            "hopper-token",
            "--no-default-features",
            "--features",
            "hopper-native-backend",
        ],
    )
}

fn check_client_layout_tests(root: &Path) -> Result<(), String> {
    run_cargo_status(root, &["test", "-p", "hopper-schema", "layout_id", "--lib"])
}

fn check_fuzz_inventory(root: &Path) -> Result<(), String> {
    let fuzz_dir = root.join("fuzz");
    let manifest = fuzz_dir.join("Cargo.toml");
    let targets = fuzz_dir.join("fuzz_targets");
    if !manifest.is_file() {
        return Err(format!("missing fuzz manifest: {}", manifest.display()));
    }
    if !targets.is_dir() {
        return Err(format!("missing fuzz targets directory: {}", targets.display()));
    }
    let mut count = 0usize;
    for entry in fs::read_dir(&targets)
        .map_err(|err| format!("failed to read {}: {err}", targets.display()))?
    {
        let entry = entry.map_err(|err| format!("failed to read fuzz target entry: {err}"))?;
        if entry.path().extension().and_then(|ext| ext.to_str()) == Some("rs") {
            count += 1;
        }
    }
    if count == 0 {
        Err(format!("no fuzz target .rs files under {}", targets.display()))
    } else {
        println!("found {count} fuzz targets");
        Ok(())
    }
}

fn run_cargo_status(root: &Path, args: &[&str]) -> Result<(), String> {
    let args_vec = to_strings(args);
    let status = workspace::run_status("cargo", &args_vec, root)?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "{} exited with status {status}",
            workspace::display_command("cargo", &args_vec)
        ))
    }
}

fn to_strings(args: &[&str]) -> Vec<String> {
    args.iter().map(|s| s.to_string()).collect()
}

fn display_command(program: &Path, args: &[String]) -> String {
    let mut rendered = program.display().to_string();
    for arg in args {
        rendered.push(' ');
        if arg.contains(' ') {
            rendered.push('"');
            rendered.push_str(arg);
            rendered.push('"');
        } else {
            rendered.push_str(arg);
        }
    }
    rendered
}

fn print_usage() {
    eprintln!("Usage: hopper publish-check [--package <name> | --manifest <path> --so <program.so>] [--full]");
    eprintln!("       hopper publish-check --source-only [--full]");
    eprintln!();
    eprintln!("Runs the public-release gate:");
    eprintln!("  1. hopper verify --release for the manifest + binary");
    eprintln!("  2. release documentation placeholder/stale-path scan");
    eprintln!("  3. default feature tree excludes Pinocchio");
    eprintln!("  4. legacy SPL Token builders remain feature-gated");
    eprintln!("  5. TS/Kotlin/Rust/Python client layout-id tests");
    eprintln!("  6. fuzz target inventory");
    eprintln!();
    eprintln!("Options:");
    eprintln!("  --package, -p <name>       Infer hopper.manifest.json and target/deploy/<name>.so");
    eprintln!("  --manifest <path>          Explicit Hopper manifest JSON");
    eprintln!("  --so, --binary <path>      Explicit compiled program .so");
    eprintln!("  --workspace-root <path>    Workspace root to check (default: search upward)");
    eprintln!("  --source-only              Skip binary ABI verification; useful before SBF build");
    eprintln!("  --full                     Also run hopper-core and hopper-trybuild test suites");
}
