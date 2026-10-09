//! `hopper audit-check`, verify the repository's audit-readiness evidence.
//!
//! The manifest is intentionally data, not prose: critical artifacts are
//! content-addressed, quality-gate attestations expire, and open blockers are
//! impossible to hide behind an optimistic heading in a document.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process;
use std::time::{SystemTime, UNIX_EPOCH};

const DEFAULT_MANIFEST: &str = "audit/readiness.json";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReadinessManifest {
    schema_version: u32,
    reviewed_on: String,
    max_review_age_days: u64,
    external_audit: ExternalAudit,
    evidence: Vec<Evidence>,
    quality_gates: Vec<QualityGate>,
    known_blockers: Vec<Blocker>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ExternalAudit {
    status: String,
    #[serde(default)]
    phase: Option<String>,
    #[serde(default)]
    started_on: Option<String>,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    provider: Option<String>,
    report: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Evidence {
    id: String,
    path: String,
    sha256: String,
    required: bool,
    description: String,
    #[serde(default)]
    normalize_line_endings: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct QualityGate {
    id: String,
    command: String,
    last_run_on: String,
    status: String,
    max_age_days: u64,
    required: bool,
    #[serde(default)]
    receipt: Option<HashedFile>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HashedFile {
    path: String,
    sha256: String,
    #[serde(default)]
    normalize_line_endings: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct GateReceipt {
    schema_version: u32,
    command: String,
    argv: Vec<String>,
    completed_on: String,
    exit_code: i32,
    source_files: BTreeMap<String, String>,
    log: HashedFile,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Blocker {
    id: String,
    severity: String,
    status: String,
    #[serde(default = "default_release_blocking")]
    release_blocking: bool,
    description: String,
}

const fn default_release_blocking() -> bool {
    true
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AuditReport {
    schema_version: u32,
    ready: bool,
    reviewed_on: String,
    evidence_verified: usize,
    quality_gates_current: usize,
    findings: Vec<Finding>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Finding {
    level: FindingLevel,
    id: String,
    message: String,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "lowercase")]
enum FindingLevel {
    Blocker,
    Warning,
}

impl AuditReport {
    fn blocker(&mut self, id: impl Into<String>, message: impl Into<String>) {
        self.findings.push(Finding {
            level: FindingLevel::Blocker,
            id: id.into(),
            message: message.into(),
        });
    }

    fn warning(&mut self, id: impl Into<String>, message: impl Into<String>) {
        self.findings.push(Finding {
            level: FindingLevel::Warning,
            id: id.into(),
            message: message.into(),
        });
    }

    fn finish(&mut self) {
        self.ready = !self
            .findings
            .iter()
            .any(|finding| matches!(finding.level, FindingLevel::Blocker));
    }
}

/// CLI entry point.
pub fn cmd_audit_check(args: &[String]) {
    let mut root = PathBuf::from(".");
    let mut manifest_path = PathBuf::from(DEFAULT_MANIFEST);
    let mut json = false;
    let mut strict = false;
    let mut i = 0;

    while i < args.len() {
        match args[i].as_str() {
            "--root" => {
                i += 1;
                let Some(value) = args.get(i) else {
                    fail("--root requires a path");
                };
                root = PathBuf::from(value);
            }
            "--manifest" => {
                i += 1;
                let Some(value) = args.get(i) else {
                    fail("--manifest requires a path relative to --root");
                };
                manifest_path = PathBuf::from(value);
            }
            "--json" => json = true,
            "--strict" => strict = true,
            "--help" | "-h" => {
                print_usage();
                return;
            }
            other => fail(&format!("unknown argument: {other}")),
        }
        i += 1;
    }

    if !is_safe_relative(&manifest_path) {
        fail("--manifest must be a safe path relative to --root");
    }

    let manifest_file = root.join(&manifest_path);
    let raw = fs::read_to_string(&manifest_file)
        .unwrap_or_else(|error| fail(&format!("cannot read {}: {error}", manifest_file.display())));
    let manifest: ReadinessManifest = serde_json::from_str(&raw).unwrap_or_else(|error| {
        fail(&format!(
            "cannot parse {}: {error}",
            manifest_file.display()
        ))
    });
    let today = epoch_days(SystemTime::now()).unwrap_or_else(|error| fail(&error));
    let report = analyze(&root, &manifest, today);

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report).expect("AuditReport is serializable")
        );
    } else {
        print_report(&report);
    }

    if strict && !report.ready {
        process::exit(1);
    }
}

fn analyze(root: &Path, manifest: &ReadinessManifest, today: i64) -> AuditReport {
    let mut report = AuditReport {
        schema_version: manifest.schema_version,
        ready: false,
        reviewed_on: manifest.reviewed_on.clone(),
        evidence_verified: 0,
        quality_gates_current: 0,
        findings: Vec::new(),
    };

    if !matches!(manifest.schema_version, 1 | 2) {
        report.blocker(
            "schema-version",
            format!(
                "unsupported readiness schema {}; expected 1 or 2",
                manifest.schema_version
            ),
        );
    }

    check_freshness(
        &mut report,
        "readiness-review",
        &manifest.reviewed_on,
        manifest.max_review_age_days,
        true,
        today,
    );

    check_external_audit(&mut report, root, &manifest.external_audit, today);
    check_manifest_structure(&mut report, manifest);

    for evidence in &manifest.evidence {
        if !is_safe_relative(Path::new(&evidence.path)) {
            add_required_finding(
                &mut report,
                evidence.required,
                &evidence.id,
                format!("unsafe evidence path: {}", evidence.path),
            );
            continue;
        }
        let path = root.join(&evidence.path);
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) => {
                add_required_finding(
                    &mut report,
                    evidence.required,
                    &evidence.id,
                    format!("missing evidence {}: {error}", evidence.path),
                );
                continue;
            }
        };
        let actual = digest_hex(&bytes, evidence.normalize_line_endings);
        if actual.eq_ignore_ascii_case(&evidence.sha256) {
            report.evidence_verified += 1;
        } else {
            add_required_finding(
                &mut report,
                evidence.required,
                &evidence.id,
                format!(
                    "digest mismatch for {} ({})",
                    evidence.path, evidence.description
                ),
            );
        }
    }

    for gate in &manifest.quality_gates {
        let before = report.findings.len();
        check_freshness(
            &mut report,
            &gate.id,
            &gate.last_run_on,
            gate.max_age_days,
            gate.required,
            today,
        );
        if gate.status != "passed" {
            add_required_finding(
                &mut report,
                gate.required,
                &gate.id,
                format!("latest gate status is '{}'", gate.status),
            );
        }
        if let Err(error) = check_gate_receipt(root, gate, manifest.schema_version) {
            add_required_finding(&mut report, gate.required, &gate.id, error);
        }
        if before == report.findings.len() && !gate.command.trim().is_empty() {
            report.quality_gates_current += 1;
        }
        for finding in &mut report.findings[before..] {
            finding
                .message
                .push_str(&format!("; rerun `{}`", gate.command));
        }
    }

    for blocker in &manifest.known_blockers {
        if blocker.status != "closed" {
            let message = format!(
                "{} {} item is {}: {}",
                blocker.severity,
                if blocker.release_blocking {
                    "release-blocking"
                } else {
                    "tracked"
                },
                blocker.status,
                blocker.description
            );
            if blocker.release_blocking {
                report.blocker(&blocker.id, message);
            } else {
                report.warning(&blocker.id, message);
            }
        }
    }

    report.finish();
    report
}

fn digest_hex(bytes: &[u8], normalize_line_endings: bool) -> String {
    if normalize_line_endings {
        let normalized: Vec<u8> = bytes
            .iter()
            .enumerate()
            .filter_map(|(index, &byte)| {
                (byte != b'\r' || bytes.get(index + 1) != Some(&b'\n')).then_some(byte)
            })
            .collect();
        format!("{:x}", Sha256::digest(&normalized))
    } else {
        format!("{:x}", Sha256::digest(bytes))
    }
}

fn read_hashed(root: &Path, file: &HashedFile) -> Result<Vec<u8>, String> {
    if !is_safe_relative(Path::new(&file.path)) {
        return Err(format!("unsafe receipt path: {}", file.path));
    }
    let root = root.canonicalize().map_err(|error| error.to_string())?;
    let path = root
        .join(&file.path)
        .canonicalize()
        .map_err(|error| format!("cannot resolve receipt input {}: {error}", file.path))?;
    if !path.starts_with(&root) {
        return Err(format!("receipt input escapes root: {}", file.path));
    }
    let bytes = fs::read(path).map_err(|error| error.to_string())?;
    if !digest_hex(&bytes, file.normalize_line_endings).eq_ignore_ascii_case(&file.sha256) {
        return Err(format!("receipt digest mismatch: {}", file.path));
    }
    Ok(bytes)
}

fn gate_source_paths(root: &Path) -> Result<BTreeSet<String>, String> {
    let output = process::Command::new("git")
        .args([
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ])
        .current_dir(root)
        .output()
        .map_err(|error| format!("cannot inventory gate sources: {error}"))?;
    if !output.status.success() {
        return Err("cannot inventory gate sources with git ls-files".into());
    }
    let paths = String::from_utf8(output.stdout).map_err(|error| error.to_string())?;
    Ok(paths
        .split('\0')
        .filter(|path| {
            !path.starts_with("audit/")
                && (*path == "Cargo.lock"
                    || matches!(
                        Path::new(path).extension().and_then(|ext| ext.to_str()),
                        Some("rs" | "toml" | "py" | "yml" | "yaml" | "json" | "stderr" | "md")
                    ))
        })
        .map(str::to_owned)
        .collect())
}

fn check_gate_receipt(root: &Path, gate: &QualityGate, schema: u32) -> Result<(), String> {
    let Some(file) = &gate.receipt else {
        return if schema == 2 && gate.required {
            Err("required quality gate has no execution receipt".into())
        } else {
            Ok(())
        };
    };
    let receipt: GateReceipt = serde_json::from_slice(&read_hashed(root, file)?)
        .map_err(|error| format!("invalid gate receipt: {error}"))?;
    if receipt.schema_version != 1 || receipt.exit_code != 0 {
        return Err("gate receipt has an unsupported schema or unsuccessful exit code".into());
    }
    if receipt.command != gate.command
        || receipt.argv.is_empty()
        || receipt.argv.join(" ") != receipt.command
        || receipt.completed_on != gate.last_run_on
    {
        return Err("gate receipt command or completion date differs from the attestation".into());
    }
    if receipt.source_files.is_empty()
        || receipt
            .source_files
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>()
            != gate_source_paths(root)?
    {
        return Err(
            "gate receipt source inventory differs; rerun after adding or removing sources".into(),
        );
    }
    for (path, sha256) in receipt.source_files {
        read_hashed(
            root,
            &HashedFile {
                path,
                sha256,
                normalize_line_endings: true,
            },
        )?;
    }
    read_hashed(root, &receipt.log)?;
    Ok(())
}

fn check_manifest_structure(report: &mut AuditReport, manifest: &ReadinessManifest) {
    if !manifest.evidence.iter().any(|item| item.required) {
        report.blocker(
            "required-evidence",
            "at least one required evidence artifact is needed",
        );
    }
    if !manifest.quality_gates.iter().any(|gate| gate.required) {
        report.blocker(
            "required-gates",
            "at least one required quality gate is needed",
        );
    }
    let mut ids = BTreeSet::new();
    for id in manifest
        .evidence
        .iter()
        .map(|item| item.id.as_str())
        .chain(manifest.quality_gates.iter().map(|gate| gate.id.as_str()))
        .chain(manifest.known_blockers.iter().map(|item| item.id.as_str()))
    {
        if id.trim().is_empty() || id != id.trim() {
            report.blocker(
                "manifest-id",
                "identifiers must be nonempty and have no surrounding whitespace",
            );
        } else if !ids.insert(id) {
            report.blocker("manifest-id", format!("duplicate identifier '{id}'"));
        }
    }
    for gate in &manifest.quality_gates {
        if gate.command.trim().is_empty() {
            report.blocker(&gate.id, "quality gate has no command to reproduce it");
        }
    }
}

fn check_external_audit(report: &mut AuditReport, root: &Path, audit: &ExternalAudit, today: i64) {
    match audit.status.as_str() {
        "not-started" => report.blocker("external-audit", "independent review has not started"),
        "preparation-in-progress" => {
            let phase = audit.phase.as_deref().unwrap_or("").trim();
            if phase.is_empty() {
                report.blocker(
                    "external-audit",
                    "external-audit preparation has no current phase",
                );
            }

            match audit.started_on.as_deref() {
                Some(date) => check_freshness(
                    report,
                    "external-audit-preparation",
                    date,
                    u64::MAX,
                    true,
                    today,
                ),
                None => report.blocker(
                    "external-audit",
                    "external-audit preparation has no start date",
                ),
            }

            match audit.scope.as_deref().map(Path::new) {
                Some(path) if is_safe_relative(path) && root.join(path).is_file() => {}
                Some(path) if !is_safe_relative(path) => report.blocker(
                    "external-audit",
                    format!("unsafe external-audit scope path: {}", path.display()),
                ),
                Some(path) => report.blocker(
                    "external-audit",
                    format!("external-audit scope is missing: {}", path.display()),
                ),
                None => report.blocker(
                    "external-audit",
                    "external-audit preparation has no scope artifact",
                ),
            }

            report.blocker(
                "external-audit",
                format!(
                    "external-audit preparation is in progress at phase '{phase}'; no independent reviewer is engaged and no independent review has started"
                ),
            );
        }
        "in-progress" => {
            let phase = audit.phase.as_deref().unwrap_or("").trim();
            if phase.is_empty() {
                report.blocker("external-audit", "in-progress review has no current phase");
            }

            match audit.started_on.as_deref() {
                Some(date) => {
                    check_freshness(report, "external-audit-start", date, u64::MAX, true, today)
                }
                None => report.blocker("external-audit", "in-progress review has no start date"),
            }

            match audit.scope.as_deref().map(Path::new) {
                Some(path) if is_safe_relative(path) && root.join(path).is_file() => {}
                Some(path) if !is_safe_relative(path) => report.blocker(
                    "external-audit",
                    format!("unsafe external-audit scope path: {}", path.display()),
                ),
                Some(path) => report.blocker(
                    "external-audit",
                    format!("external-audit scope is missing: {}", path.display()),
                ),
                None => {
                    report.blocker("external-audit", "in-progress review has no scope artifact")
                }
            }

            let provider = audit.provider.as_deref().unwrap_or("").trim();
            if provider.is_empty() {
                report.blocker(
                    "external-audit",
                    format!(
                        "independent review is in progress at phase '{phase}', but no reviewer is engaged"
                    ),
                );
            } else {
                report.blocker(
                    "external-audit",
                    format!(
                        "independent review by '{provider}' is in progress at phase '{phase}'; no completed report is registered"
                    ),
                );
            }
        }
        "independent-complete" => {
            let provider = audit.provider.as_deref().unwrap_or("").trim();
            if provider.is_empty() {
                report.blocker(
                    "external-audit",
                    "completed independent review has no reviewer identity",
                );
            }

            match audit.report.as_deref().map(Path::new) {
                Some(path) if is_safe_relative(path) && root.join(path).is_file() => {}
                Some(path) if !is_safe_relative(path) => report.blocker(
                    "external-audit",
                    format!("unsafe external-audit report path: {}", path.display()),
                ),
                Some(path) => report.blocker(
                    "external-audit",
                    format!(
                        "completed external-audit report is missing: {}",
                        path.display()
                    ),
                ),
                None => report.blocker(
                    "external-audit",
                    "completed independent review has no report artifact",
                ),
            }
        }
        other => report.blocker(
            "external-audit",
            format!("unsupported external-audit status '{other}'"),
        ),
    }
}

fn check_freshness(
    report: &mut AuditReport,
    id: &str,
    date: &str,
    max_age_days: u64,
    required: bool,
    today: i64,
) {
    let passed = match parse_date(date) {
        Ok(days) => days,
        Err(error) => {
            add_required_finding(report, required, id, error);
            return;
        }
    };
    let age = today.saturating_sub(passed);
    if age < 0 {
        add_required_finding(
            report,
            required,
            id,
            format!("date {date} is in the future"),
        );
    } else if age as u64 > max_age_days {
        add_required_finding(
            report,
            required,
            id,
            format!("attestation is {age} days old (maximum {max_age_days})"),
        );
    }
}

fn add_required_finding(
    report: &mut AuditReport,
    required: bool,
    id: impl Into<String>,
    message: impl Into<String>,
) {
    if required {
        report.blocker(id, message);
    } else {
        report.warning(id, message);
    }
}

fn is_safe_relative(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_) | Component::CurDir))
}

fn epoch_days(now: SystemTime) -> Result<i64, String> {
    now.duration_since(UNIX_EPOCH)
        .map(|duration| (duration.as_secs() / 86_400) as i64)
        .map_err(|_| "system clock is before the Unix epoch".to_string())
}

fn parse_date(value: &str) -> Result<i64, String> {
    // Bound the year before civil-date arithmetic and accept only YYYY-MM-DD.
    // An unbounded i32 year can overflow that calculation in release builds.
    let bytes = value.as_bytes();
    if bytes.len() != 10
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes
            .iter()
            .enumerate()
            .any(|(i, byte)| i != 4 && i != 7 && !byte.is_ascii_digit())
    {
        return Err(format!("invalid date: {value}"));
    }
    let mut parts = value.split('-');
    let year: i32 = parts
        .next()
        .and_then(|part| part.parse().ok())
        .ok_or_else(|| format!("invalid date: {value}"))?;
    let month: u32 = parts
        .next()
        .and_then(|part| part.parse().ok())
        .ok_or_else(|| format!("invalid date: {value}"))?;
    let day: u32 = parts
        .next()
        .and_then(|part| part.parse().ok())
        .ok_or_else(|| format!("invalid date: {value}"))?;
    if parts.next().is_some() || year == 0 || !(1..=12).contains(&month) {
        return Err(format!("invalid date: {value}"));
    }
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let month_days = [
        31,
        28 + u32::from(leap),
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    if day == 0 || day > month_days[(month - 1) as usize] {
        return Err(format!("invalid date: {value}"));
    }

    // Howard Hinnant's civil-date conversion, shifted to Unix epoch days.
    let adjusted_year = year - i32::from(month <= 2);
    let era = if adjusted_year >= 0 {
        adjusted_year
    } else {
        adjusted_year - 399
    } / 400;
    let year_of_era = (adjusted_year - era * 400) as u32;
    let adjusted_month = if month > 2 { month - 3 } else { month + 9 };
    let day_of_year = (153 * adjusted_month + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    Ok((era * 146_097 + day_of_era as i32 - 719_468) as i64)
}

fn print_report(report: &AuditReport) {
    println!("hopper audit-check");
    println!(
        "status             : {}",
        if report.ready { "READY" } else { "NOT READY" }
    );
    println!("reviewed           : {}", report.reviewed_on);
    println!("evidence verified  : {}", report.evidence_verified);
    println!("quality gates fresh: {}", report.quality_gates_current);
    if report.findings.is_empty() {
        println!("findings           : none");
    } else {
        println!("findings:");
        for finding in &report.findings {
            println!("  {:?} [{}] {}", finding.level, finding.id, finding.message);
        }
    }
}

fn print_usage() {
    println!("Usage: hopper audit-check [--root <path>] [--manifest <relative-path>] [--json] [--strict]");
    println!();
    println!("Verify content-addressed audit evidence, attestation freshness, and known blockers.");
    println!("--strict exits non-zero while any required readiness blocker remains open.");
}

fn fail(message: &str) -> ! {
    eprintln!("hopper audit-check: {message}");
    process::exit(2);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn manifest(digest: String) -> ReadinessManifest {
        ReadinessManifest {
            schema_version: 1,
            reviewed_on: "2026-08-15".into(),
            max_review_age_days: 30,
            external_audit: ExternalAudit {
                status: "independent-complete".into(),
                phase: Some("complete".into()),
                started_on: Some("2026-08-01".into()),
                scope: Some("scope.md".into()),
                provider: Some("Independent Reviewer".into()),
                report: Some("report.pdf".into()),
            },
            evidence: vec![Evidence {
                id: "policy".into(),
                path: "SECURITY.md".into(),
                sha256: digest,
                required: true,
                description: "policy".into(),
                normalize_line_endings: false,
            }],
            quality_gates: vec![QualityGate {
                id: "tests".into(),
                command: "cargo test".into(),
                last_run_on: "2026-08-15".into(),
                status: "passed".into(),
                max_age_days: 7,
                required: true,
                receipt: None,
            }],
            known_blockers: Vec::new(),
        }
    }

    #[test]
    fn complete_current_evidence_is_ready() {
        let root = std::env::temp_dir().join(format!("hopper-audit-{}", process::id()));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("SECURITY.md"), b"policy").unwrap();
        fs::write(root.join("report.pdf"), b"report").unwrap();
        let digest = format!("{:x}", Sha256::digest(b"policy"));
        let report = analyze(&root, &manifest(digest), parse_date("2026-08-15").unwrap());
        fs::remove_dir_all(root).unwrap();
        assert!(report.ready);
        assert_eq!(report.evidence_verified, 1);
        assert_eq!(report.quality_gates_current, 1);
    }

    #[test]
    fn tampering_and_open_blockers_fail_closed() {
        let root = std::env::temp_dir().join(format!("hopper-audit-bad-{}", process::id()));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("SECURITY.md"), b"changed").unwrap();
        fs::write(root.join("report.pdf"), b"report").unwrap();
        let mut input = manifest(format!("{:x}", Sha256::digest(b"policy")));
        input.known_blockers.push(Blocker {
            id: "external-review".into(),
            severity: "critical".into(),
            status: "open".into(),
            release_blocking: true,
            description: "review not complete".into(),
        });
        let report = analyze(&root, &input, parse_date("2026-08-15").unwrap());
        fs::remove_dir_all(root).unwrap();
        assert!(!report.ready);
        assert_eq!(report.findings.len(), 2);
    }

    #[test]
    fn dates_are_validated_and_age_expires() {
        assert_eq!(parse_date("1970-01-01").unwrap(), 0);
        assert!(parse_date("2024-02-29").is_ok());
        assert!(parse_date("2025-02-29").is_err());
        for invalid in [
            "2147483647-01-01",
            "0000-01-01",
            "2026-1-01",
            "+2026-01-01",
            "2026-01-01 ",
        ] {
            assert!(parse_date(invalid).is_err(), "accepted {invalid}");
        }
        assert!(parse_date("9999-12-31").is_ok());

        let mut report = AuditReport {
            schema_version: 1,
            ready: false,
            reviewed_on: String::new(),
            evidence_verified: 0,
            quality_gates_current: 0,
            findings: Vec::new(),
        };
        check_freshness(
            &mut report,
            "stale",
            "2026-07-01",
            30,
            true,
            parse_date("2026-08-15").unwrap(),
        );
        assert_eq!(report.findings.len(), 1);
    }

    #[test]
    fn evidence_paths_cannot_escape_root() {
        assert!(is_safe_relative(Path::new("docs/SECURITY.md")));
        assert!(!is_safe_relative(Path::new("../SECURITY.md")));
        assert!(!is_safe_relative(Path::new("C:/SECURITY.md")));
    }

    #[test]
    fn missing_required_checks_and_ambiguous_ids_fail_closed() {
        let mut input = manifest(String::new());
        for optional_only in [false, true] {
            if optional_only {
                input = manifest(String::new());
                input.evidence[0].required = false;
                input.quality_gates[0].required = false;
            } else {
                input.evidence.clear();
                input.quality_gates.clear();
            }
            let report = analyze(Path::new("."), &input, parse_date("2026-08-15").unwrap());
            assert!(!report.ready);
            for id in ["required-evidence", "required-gates"] {
                assert!(report.findings.iter().any(|finding| finding.id == id));
            }
        }
        for id in ["policy", "", " tests "] {
            let mut input = manifest(String::new());
            input.quality_gates[0].id = id.into();
            let report = analyze(Path::new("."), &input, parse_date("2026-08-15").unwrap());
            assert!(report
                .findings
                .iter()
                .any(|finding| finding.id == "manifest-id"));
        }
    }

    #[test]
    fn only_reproducible_passed_current_gates_count() {
        for (status, date, command, expected) in [
            ("passed", "2026-08-15", "cargo test", 1),
            ("failed", "2026-08-15", "cargo test", 0),
            ("passed", "2026-08-01", "cargo test", 0),
            ("passed", "2026-08-16", "cargo test", 0),
            ("passed", "2026-08-15", "  ", 0),
        ] {
            let mut input = manifest(String::new());
            input.quality_gates[0].status = status.into();
            input.quality_gates[0].last_run_on = date.into();
            input.quality_gates[0].command = command.into();
            let report = analyze(Path::new("."), &input, parse_date("2026-08-15").unwrap());
            assert_eq!(
                report.quality_gates_current, expected,
                "{status} {date} {command}"
            );
        }
    }

    #[test]
    fn receipt_binds_command_date_result_log_and_complete_source_inventory() {
        let root = std::env::temp_dir().join(format!("hopper-audit-receipt-{}", process::id()));
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(root.join("audit")).unwrap();
        assert!(process::Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(&root)
            .status()
            .unwrap()
            .success());
        fs::write(root.join("lib.rs"), b"source").unwrap();
        fs::write(root.join("README.md"), b"documentation").unwrap();
        fs::write(root.join("test.log"), b"passed").unwrap();
        let mut input = manifest(String::new());
        let gate = &mut input.quality_gates[0];
        let receipt = serde_json::json!({
            "schemaVersion": 1, "command": "cargo test", "argv": ["cargo", "test"],
            "completedOn": "2026-08-15", "exitCode": 0,
            "sourceFiles": {
                "lib.rs": format!("{:x}", Sha256::digest(b"source")),
                "README.md": format!("{:x}", Sha256::digest(b"documentation"))
            },
            "log": {"path": "test.log", "sha256": format!("{:x}", Sha256::digest(b"passed"))}
        });
        let save = |gate: &mut QualityGate, value: &serde_json::Value| {
            let bytes = serde_json::to_vec(value).unwrap();
            fs::write(root.join("audit/receipt.json"), &bytes).unwrap();
            gate.receipt = Some(HashedFile {
                path: "audit/receipt.json".into(),
                sha256: format!("{:x}", Sha256::digest(&bytes)),
                normalize_line_endings: false,
            });
        };
        assert!(check_gate_receipt(&root, gate, 2).is_err());
        assert!(check_gate_receipt(&root, gate, 1).is_ok());
        save(gate, &receipt);
        assert!(check_gate_receipt(&root, gate, 2).is_ok());
        // Source hashes survive Git's text checkout conversion; binary logs do not.
        assert_eq!(digest_hex(b"a\r\nb\r", true), digest_hex(b"a\nb\r", true));
        assert_ne!(digest_hex(b"a\r\n", false), digest_hex(b"a\n", false));
        for (key, value) in [
            ("exitCode", serde_json::json!(1)),
            ("schemaVersion", serde_json::json!(99)),
            ("command", serde_json::json!("cargo check")),
            ("argv", serde_json::json!(["cargo", "check"])),
            ("completedOn", serde_json::json!("2026-08-14")),
            ("sourceFiles", serde_json::json!({})),
        ] {
            let mut changed = receipt.clone();
            changed[key] = value;
            save(gate, &changed);
            assert!(
                check_gate_receipt(&root, gate, 2).is_err(),
                "accepted {key}"
            );
        }
        save(gate, &receipt);
        for (path, contents) in [
            ("test.log", b"tamper".as_slice()),
            ("lib.rs", b"changed"),
            ("new.rs", b"added"),
            ("README.md", b"changed example"),
            ("GUIDE.md", b"new guide"),
        ] {
            fs::write(root.join(path), contents).unwrap();
            assert!(
                check_gate_receipt(&root, gate, 2).is_err(),
                "accepted changed {path}"
            );
            match path {
                "test.log" => fs::write(root.join(path), b"passed").unwrap(),
                "lib.rs" => fs::write(root.join(path), b"source").unwrap(),
                "README.md" => fs::write(root.join(path), b"documentation").unwrap(),
                _ => fs::remove_file(root.join(path)).unwrap(),
            }
        }
        fs::remove_file(root.join("README.md")).unwrap();
        assert!(check_gate_receipt(&root, gate, 2).is_err());
        fs::write(root.join("README.md"), b"documentation").unwrap();
        assert!(check_gate_receipt(&root, gate, 2).is_ok());
        let mut escaped = receipt.clone();
        escaped["log"]["path"] = serde_json::json!("../outside.log");
        save(gate, &escaped);
        assert!(check_gate_receipt(&root, gate, 2).is_err());
        save(gate, &receipt);
        fs::write(root.join("audit/receipt.json"), b"{}").unwrap();
        assert!(check_gate_receipt(&root, gate, 2).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn preparation_in_progress_is_honest_but_not_an_independent_review() {
        let root = std::env::temp_dir().join(format!("hopper-audit-progress-{}", process::id()));
        fs::create_dir_all(root.join("audit")).unwrap();
        fs::write(root.join("SECURITY.md"), b"policy").unwrap();
        fs::write(root.join("audit/scope.md"), b"scope").unwrap();
        let digest = format!("{:x}", Sha256::digest(b"policy"));
        let mut input = manifest(digest);
        input.external_audit = ExternalAudit {
            status: "preparation-in-progress".into(),
            phase: Some("scope-and-evidence-preparation".into()),
            started_on: Some("2026-08-16".into()),
            scope: Some("audit/scope.md".into()),
            provider: None,
            report: None,
        };
        let report = analyze(&root, &input, parse_date("2026-08-16").unwrap());
        fs::remove_dir_all(root).unwrap();
        assert!(!report.ready);
        assert!(report.findings.iter().any(|finding| {
            finding.id == "external-audit"
                && finding
                    .message
                    .contains("no independent review has started")
        }));
    }

    #[test]
    fn tracked_future_protocol_work_warns_without_blocking_release() {
        let root = std::env::temp_dir().join(format!("hopper-audit-tracked-{}", process::id()));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("SECURITY.md"), b"policy").unwrap();
        fs::write(root.join("report.pdf"), b"report").unwrap();
        let digest = format!("{:x}", Sha256::digest(b"policy"));
        let mut input = manifest(digest);
        input.known_blockers.push(Blocker {
            id: "future-protocol".into(),
            severity: "medium".into(),
            status: "tracked".into(),
            release_blocking: false,
            description: "feature is not active on Mainnet".into(),
        });
        let report = analyze(&root, &input, parse_date("2026-08-16").unwrap());
        fs::remove_dir_all(root).unwrap();
        assert!(report.ready);
        assert!(report.findings.iter().any(|finding| {
            finding.id == "future-protocol" && matches!(finding.level, FindingLevel::Warning)
        }));
    }
}
