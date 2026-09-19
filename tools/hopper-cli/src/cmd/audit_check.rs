//! `hopper audit-check`, verify the repository's audit-readiness evidence.
//!
//! The manifest is intentionally data, not prose: critical artifacts are
//! content-addressed, quality-gate attestations expire, and open blockers are
//! impossible to hide behind an optimistic heading in a document.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
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

    if manifest.schema_version != 1 {
        report.blocker(
            "schema-version",
            format!(
                "unsupported readiness schema {}; expected 1",
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
        let actual = format!("{:x}", Sha256::digest(&bytes));
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
        if before == report.findings.len() {
            report.quality_gates_current += 1;
        }
        if gate.status != "passed" {
            add_required_finding(
                &mut report,
                gate.required,
                &gate.id,
                format!("latest gate status is '{}'", gate.status),
            );
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
    if parts.next().is_some() || !(1..=12).contains(&month) {
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
            }],
            quality_gates: vec![QualityGate {
                id: "tests".into(),
                command: "cargo test".into(),
                last_run_on: "2026-08-15".into(),
                status: "passed".into(),
                max_age_days: 7,
                required: true,
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
