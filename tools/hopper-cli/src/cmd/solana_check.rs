use std::fs;
use std::path::{Path, PathBuf};
use std::process::{self, Command};

#[derive(Default)]
struct CheckOptions {
    all: bool,
    manifest_path: Option<PathBuf>,
    build_sbf: bool,
}

#[derive(Default)]
struct PackageReport {
    package: String,
    path: PathBuf,
    failures: Vec<String>,
    warnings: Vec<String>,
}

pub fn cmd_solana_check(args: &[String]) {
    let options = parse_args(args);
    let manifests = if options.all {
        collect_candidate_manifests(&std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
    } else {
        vec![options
            .manifest_path
            .clone()
            .unwrap_or_else(|| PathBuf::from("Cargo.toml"))]
    };

    if manifests.is_empty() {
        eprintln!("No Cargo.toml files with program-shaped sources were found.");
        process::exit(1);
    }

    let mut failed = false;
    for manifest in manifests {
        let report = check_package(&manifest, options.build_sbf);
        if !report.failures.is_empty() {
            failed = true;
        }
        print_report(&report);
    }

    if failed {
        process::exit(1);
    }
}

fn parse_args(args: &[String]) -> CheckOptions {
    let mut options = CheckOptions::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--all" => options.all = true,
            "--build-sbf" => options.build_sbf = true,
            "--manifest-path" => {
                i += 1;
                if i >= args.len() {
                    usage_and_exit();
                }
                options.manifest_path = Some(PathBuf::from(&args[i]));
            }
            "--help" | "-h" => usage_and_exit(),
            other if !other.starts_with('-') && options.manifest_path.is_none() => {
                options.manifest_path = Some(PathBuf::from(other));
            }
            other => {
                eprintln!("Unknown solana-check argument: {other}");
                usage_and_exit();
            }
        }
        i += 1;
    }
    options
}

fn usage_and_exit() -> ! {
    eprintln!("Usage: hopper solana-check [--all] [--manifest-path Cargo.toml] [--build-sbf]");
    eprintln!();
    eprintln!("Checks that Hopper programs are Rust-valid and SBF-valid by construction.");
    process::exit(1);
}

fn collect_candidate_manifests(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    visit_dirs(root, &mut out, 0);
    out.sort();
    out.dedup();
    out
}

fn visit_dirs(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    if depth > 5 {
        return;
    }
    let name = dir.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    if matches!(name, "target" | ".git" | "node_modules" | "test-ledger") {
        return;
    }
    let manifest = dir.join("Cargo.toml");
    if manifest.exists()
        && dir.join("src").join("lib.rs").exists()
        && is_program_candidate(&manifest)
    {
        out.push(manifest);
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            visit_dirs(&path, out, depth + 1);
        }
    }
}

fn is_program_candidate(manifest: &Path) -> bool {
    let root = manifest.parent().unwrap_or_else(|| Path::new("."));
    let source_path = root.join("src").join("lib.rs");
    let source = fs::read_to_string(&source_path).unwrap_or_default();
    if source_has_hopper_entrypoint(&source) {
        return true;
    }

    let Ok(text) = fs::read_to_string(manifest) else {
        return false;
    };
    let Ok(value) = text.parse::<toml::Value>() else {
        return false;
    };
    has_cdylib(&value)
}

fn check_package(manifest: &Path, build_sbf: bool) -> PackageReport {
    let text = fs::read_to_string(manifest).unwrap_or_else(|err| {
        eprintln!("Failed to read {}: {err}", manifest.display());
        process::exit(1);
    });
    let value: toml::Value = text.parse().unwrap_or_else(|err| {
        eprintln!("Failed to parse {}: {err}", manifest.display());
        process::exit(1);
    });
    let package = value
        .get("package")
        .and_then(|p| p.get("name"))
        .and_then(|n| n.as_str())
        .unwrap_or("<unknown>")
        .to_string();
    let root = manifest.parent().unwrap_or_else(|| Path::new("."));
    let source_path = root.join("src").join("lib.rs");
    let source = fs::read_to_string(&source_path).unwrap_or_default();
    let mut report = PackageReport {
        package,
        path: manifest.to_path_buf(),
        failures: Vec::new(),
        warnings: Vec::new(),
    };

    if source.trim().is_empty() {
        report
            .warnings
            .push("missing src/lib.rs, skipped source checks".to_string());
        return report;
    }

    if !has_cdylib(&value) {
        report
            .failures
            .push("[lib].crate-type must include \"cdylib\" for build-sbf output".to_string());
    }
    if !source.contains("target_os = \"solana\"") || !source.contains("no_std") {
        report
            .warnings
            .push("source does not advertise cfg_attr(target_os = \"solana\", no_std)".to_string());
    }
    if !source.contains("no_allocator!") {
        report
            .warnings
            .push("source does not call hopper::no_allocator!(); verify the SBF allocator path is intentional".to_string());
    }
    if !source.contains("nostd_panic_handler!") {
        report
            .warnings
            .push("source does not call hopper::nostd_panic_handler!(); verify panic handling is intentional".to_string());
    }
    let has_program_macro = source_has_hopper_entrypoint(&source);
    if !has_program_macro {
        report
            .warnings
            .push("no #[program] or Hopper entrypoint macro found in src/lib.rs".to_string());
    }
    if source.contains("entrypoint = false")
        && !source.contains("fast_entrypoint!")
        && !source.contains("program_entrypoint!")
    {
        report.failures.push(
            "#[program(entrypoint = false)] requires an explicit Hopper entrypoint macro"
                .to_string(),
        );
    }
    if !uses_single_backend(&value) {
        report
            .failures
            .push("select exactly one Hopper backend feature for SBF program crates".to_string());
    }
    if build_sbf {
        match Command::new("cargo")
            .arg("build-sbf")
            .arg("--manifest-path")
            .arg(manifest)
            .status()
        {
            Ok(status) if status.success() => {}
            Ok(status) => report
                .failures
                .push(format!("cargo build-sbf failed with status {status}")),
            Err(err) => report.failures.push(format!(
                "failed to launch cargo build-sbf for {}: {err}",
                manifest.display()
            )),
        }
    }

    report
}

fn source_has_hopper_entrypoint(source: &str) -> bool {
    source.contains("#[program")
        || source.contains("#[hopper::program")
        || source.contains("#[hopper_program")
        || source.contains("fast_entrypoint!")
        || source.contains("program_entrypoint!")
}

fn has_cdylib(value: &toml::Value) -> bool {
    value
        .get("lib")
        .and_then(|lib| lib.get("crate-type"))
        .and_then(|types| types.as_array())
        .map(|types| types.iter().any(|ty| ty.as_str() == Some("cdylib")))
        .unwrap_or(false)
}

fn uses_single_backend(value: &toml::Value) -> bool {
    let Some(features) = value.get("features").and_then(|f| f.as_table()) else {
        return true;
    };
    let backend_features = [
        "hopper-native-backend",
        "hopper-anchor-backend",
        "hopper-pinocchio-backend",
    ];
    let declared = backend_features
        .iter()
        .filter(|name| features.contains_key(**name))
        .count();
    declared <= 1
}

fn print_report(report: &PackageReport) {
    if report.failures.is_empty() {
        println!(
            "PASS solana-check {} ({})",
            report.package,
            report.path.display()
        );
    } else {
        println!(
            "FAIL solana-check {} ({})",
            report.package,
            report.path.display()
        );
    }
    for failure in &report.failures {
        println!("  error: {failure}");
    }
    for warning in &report.warnings {
        println!("  warn: {warning}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn all_mode_collects_only_program_candidates() {
        let root = unique_temp_dir();

        write_package(
            &root.join("runtime"),
            r#"[package]
name = "runtime"
version = "0.0.0"
edition = "2021"
"#,
            "pub fn helper() {}",
        );
        write_package(
            &root.join("missing_cdylib_program"),
            r#"[package]
name = "missing_cdylib_program"
version = "0.0.0"
edition = "2021"
"#,
            "#[program]\npub mod demo {}",
        );
        write_package(
            &root.join("cdylib_program"),
            r#"[package]
name = "cdylib_program"
version = "0.0.0"
edition = "2021"

[lib]
crate-type = ["cdylib", "lib"]
"#,
            "pub fn process() {}",
        );

        let manifests = collect_candidate_manifests(&root);
        let rendered = manifests
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>();

        assert_eq!(rendered.len(), 2, "collected manifests: {rendered:?}");
        assert!(rendered.iter().any(|path| path.contains("cdylib_program")));
        assert!(rendered
            .iter()
            .any(|path| path.contains("missing_cdylib_program")));
        assert!(!rendered.iter().any(|path| path.contains("runtime")));

        let _ = fs::remove_dir_all(root);
    }

    fn unique_temp_dir() -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after unix epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "hopper_solana_check_test_{}_{}",
            process::id(),
            suffix
        ));
        fs::create_dir_all(&root).expect("create temp root");
        root
    }

    fn write_package(root: &Path, manifest: &str, source: &str) {
        let source_dir = root.join("src");
        fs::create_dir_all(&source_dir).expect("create package source dir");
        fs::write(root.join("Cargo.toml"), manifest).expect("write package manifest");
        fs::write(source_dir.join("lib.rs"), source).expect("write package source");
    }
}
