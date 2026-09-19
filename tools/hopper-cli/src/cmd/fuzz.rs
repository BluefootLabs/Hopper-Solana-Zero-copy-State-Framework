//! Manifest-derived adversarial test planning and execution.
//!
//! Hopper already has hand-authored libFuzzer targets for the lowest-level
//! parsers. This command closes the program-specific gap: it projects every
//! published layout, instruction, account role, write range, parametric cell,
//! lamport permission, variadic suffix, PDA, and migration edge into stable,
//! seeded cases. `fuzz run` sends those cases to an application adapter and
//! fails closed unless the adapter reports every generated and user-required
//! invariant. The plan is content-addressed so CI can also fail when a manifest
//! changes without its adversarial surface changing with it.

use hopper_schema::accounts::{AccountLifecycle, ContextDescriptor};
use hopper_schema::{ArgEncoding, InstructionDescriptor, MigrationPolicy, ProgramManifest};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{self, Command, Stdio};

const PLAN_SCHEMA: &str = "hopper.manifest-fuzz-plan.v2";
const CASE_SEED_SCHEMA: &str = "hopper.manifest-fuzz-case-seed.v1";
const HARNESS_REQUEST_SCHEMA: &str = "hopper.manifest-fuzz-request.v1";
const HARNESS_RESPONSE_SCHEMA: &str = "hopper.manifest-fuzz-response.v1";
const HARNESS_REPORT_SCHEMA: &str = "hopper.manifest-fuzz-report.v1";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ManifestFuzzPlan {
    schema: String,
    program: String,
    program_version: String,
    contract_commitment: String,
    coverage: FuzzCoverage,
    cases: Vec<FuzzCase>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FuzzCoverage {
    layouts: usize,
    fields: usize,
    layout_metadata: usize,
    instructions: usize,
    instruction_arguments: usize,
    account_roles: usize,
    contexts: usize,
    context_accounts: usize,
    matched_instruction_contexts: usize,
    alias_pairs: usize,
    typed_account_constraints: usize,
    pda_constraints: usize,
    lifecycle_constraints: usize,
    has_one_constraints: usize,
    owner_constraints: usize,
    address_constraints: usize,
    optional_account_constraints: usize,
    policies: usize,
    policy_requirements: usize,
    policy_invariants: usize,
    policy_attachments: usize,
    write_ranges: usize,
    parametric_write_ranges: usize,
    lamport_permissions: usize,
    migration_pairs: usize,
    strict_write_instructions: usize,
    mutation_complete_instructions: usize,
    generated_cases: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FuzzCase {
    id: String,
    /// Stable 128-bit seed, encoded as lowercase hexadecimal to avoid JSON's
    /// lossy integer range in JavaScript adapters.
    seed: String,
    kind: String,
    target: String,
    expectation: String,
    /// Hooks an adapter must affirm after executing this mutation. These are
    /// generated from the contract dimension, not supplied by the adapter.
    required_invariants: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    parameters: BTreeMap<String, u64>,
}

/// One batch handed to an application-owned SVM/program-test adapter on stdin.
///
/// The adapter must write exactly one [`HarnessResponse`] JSON object to
/// stdout. Diagnostics belong on stderr so the protocol remains unambiguous.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HarnessRequest {
    schema: String,
    program: String,
    program_version: String,
    contract_commitment: String,
    cases: Vec<FuzzCase>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HarnessResponse {
    schema: String,
    contract_commitment: String,
    results: Vec<HarnessResult>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum HarnessOutcome {
    Passed,
    Failed,
    Skipped,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HarnessResult {
    id: String,
    outcome: HarnessOutcome,
    #[serde(default)]
    checked_invariants: Vec<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    detail: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HarnessReport {
    schema: String,
    program: String,
    program_version: String,
    contract_commitment: String,
    adapter: String,
    total: usize,
    passed: usize,
    skipped: usize,
    failed: usize,
    results: Vec<HarnessResult>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct RunSummary {
    passed: usize,
    skipped: usize,
    failed: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct RunValidation {
    summary: RunSummary,
    results: Vec<HarnessResult>,
    failures: Vec<String>,
}

pub fn cmd_fuzz(args: &[String], load_manifest: fn(&str) -> ProgramManifest) {
    match args.first().map(String::as_str) {
        Some("generate") => generate_command(&args[1..], load_manifest),
        Some("check") => check_command(&args[1..], load_manifest),
        Some("run") => run_command(&args[1..], load_manifest),
        Some("--help") | Some("-h") | None => print_usage(),
        Some(other) => fail(&format!(
            "unknown fuzz command `{other}`; expected generate, check, or run"
        )),
    }
}

fn generate_command(args: &[String], load_manifest: fn(&str) -> ProgramManifest) {
    let options = parse_options(args, false);
    let manifest_arg = options
        .program
        .as_deref()
        .unwrap_or_else(|| fail("fuzz generate requires --program <manifest>"));
    let manifest = load_manifest(manifest_arg);
    let plan = build_plan(&manifest);
    let out = options
        .out
        .unwrap_or_else(|| default_plan_path(manifest.name));
    write_plan(&out, &plan);
    if let Some(corpus) = options.corpus {
        write_seed_corpus(&corpus, &plan);
        println!(
            "Generated {} deterministic seed files at {}",
            plan.cases.len(),
            corpus.display()
        );
    }
    println!(
        "Generated {} manifest-derived cases for {} at {}",
        plan.cases.len(),
        manifest.name,
        out.display()
    );
    println!("Contract commitment: {}", plan.contract_commitment);
}

fn check_command(args: &[String], load_manifest: fn(&str) -> ProgramManifest) {
    let options = parse_options(args, true);
    let manifest_arg = options
        .program
        .as_deref()
        .unwrap_or_else(|| fail("fuzz check requires --program <manifest>"));
    let manifest = load_manifest(manifest_arg);
    let expected = build_plan(&manifest);
    let plan_path = options
        .out
        .unwrap_or_else(|| default_plan_path(manifest.name));
    let actual = read_plan(&plan_path);
    require_current_plan(&actual, &expected, &plan_path, manifest_arg);
    println!(
        "Verified {} manifest-derived cases for {} ({})",
        actual.cases.len(),
        manifest.name,
        actual.contract_commitment
    );
}

fn run_command(args: &[String], load_manifest: fn(&str) -> ProgramManifest) {
    let options = parse_run_options(args);
    let manifest_arg = options
        .program
        .as_deref()
        .unwrap_or_else(|| fail("fuzz run requires --program <manifest>"));
    let adapter = options
        .adapter
        .as_deref()
        .unwrap_or_else(|| fail("fuzz run requires --adapter <executable>"));
    let manifest = load_manifest(manifest_arg);
    let expected = build_plan(&manifest);
    let plan = if let Some(path) = options.plan.as_deref() {
        let committed = read_plan(path);
        require_current_plan(&committed, &expected, path, manifest_arg);
        committed
    } else {
        expected
    };

    let request = build_request(&plan, &options.case_ids, &options.required_invariants)
        .unwrap_or_else(|error| fail(&error));
    if request.cases.is_empty() {
        fail("fuzz run selected no cases");
    }

    let response = invoke_adapter(adapter, &options.adapter_args, &request)
        .unwrap_or_else(|error| fail(&error));
    let validation = validate_response(&request, response, options.allow_skips)
        .unwrap_or_else(|error| fail(&error));
    let RunValidation {
        summary,
        results,
        failures,
    } = validation;

    let report = HarnessReport {
        schema: HARNESS_REPORT_SCHEMA.to_string(),
        program: request.program.clone(),
        program_version: request.program_version.clone(),
        contract_commitment: request.contract_commitment.clone(),
        adapter: adapter.to_string(),
        total: results.len(),
        passed: summary.passed,
        skipped: summary.skipped,
        failed: summary.failed,
        results,
    };
    if let Some(path) = options.report.as_deref() {
        write_json(path, &report, "fuzz report");
    }

    println!(
        "Executed {} manifest-derived cases for {}: {} passed, {} skipped",
        report.total, report.program, report.passed, report.skipped
    );
    println!("Contract commitment: {}", report.contract_commitment);
    if let Some(path) = options.report {
        println!("Report: {}", path.display());
    }
    if !failures.is_empty() {
        fail(&format!(
            "adapter failed {} case(s): {}",
            failures.len(),
            failure_preview(&failures)
        ));
    }
}

fn failure_preview(failures: &[String]) -> String {
    const LIMIT: usize = 8;
    let mut preview = failures
        .iter()
        .take(LIMIT)
        .cloned()
        .collect::<Vec<_>>()
        .join("; ");
    if failures.len() > LIMIT {
        preview.push_str(&format!("; and {} more", failures.len() - LIMIT));
    }
    preview
}

fn read_plan(path: &Path) -> ManifestFuzzPlan {
    let raw = fs::read_to_string(path).unwrap_or_else(|error| {
        fail(&format!(
            "cannot read fuzz plan {}: {error}; regenerate it with `hopper fuzz generate`",
            path.display()
        ))
    });
    serde_json::from_str(&raw).unwrap_or_else(|error| {
        fail(&format!(
            "cannot parse fuzz plan {}: {error}",
            path.display()
        ))
    })
}

fn require_current_plan(
    actual: &ManifestFuzzPlan,
    expected: &ManifestFuzzPlan,
    path: &Path,
    manifest_arg: &str,
) {
    if actual != expected {
        fail(&format!(
            "fuzz plan {} is stale for {}; regenerate it with `hopper fuzz generate --program {} --out {}`",
            path.display(),
            expected.program,
            manifest_arg,
            path.display()
        ));
    }
}

#[derive(Default)]
struct Options {
    program: Option<String>,
    out: Option<PathBuf>,
    corpus: Option<PathBuf>,
}

#[derive(Default)]
struct RunOptions {
    program: Option<String>,
    plan: Option<PathBuf>,
    adapter: Option<String>,
    adapter_args: Vec<String>,
    case_ids: Vec<String>,
    required_invariants: Vec<String>,
    report: Option<PathBuf>,
    allow_skips: bool,
}

fn parse_options(args: &[String], checking: bool) -> Options {
    let mut options = Options::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--program" => {
                i += 1;
                options.program = Some(required_value(args, i, "--program"));
            }
            "--out" | "--plan" => {
                i += 1;
                options.out = Some(PathBuf::from(required_value(args, i, "--out")));
            }
            "--corpus" if !checking => {
                i += 1;
                options.corpus = Some(PathBuf::from(required_value(args, i, "--corpus")));
            }
            "--help" | "-h" => {
                print_usage();
                process::exit(0);
            }
            other => fail(&format!("unknown fuzz argument `{other}`")),
        }
        i += 1;
    }
    options
}

fn parse_run_options(args: &[String]) -> RunOptions {
    let mut options = RunOptions::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--program" => {
                i += 1;
                options.program = Some(required_value(args, i, "--program"));
            }
            "--plan" => {
                i += 1;
                options.plan = Some(PathBuf::from(required_value(args, i, "--plan")));
            }
            "--adapter" => {
                i += 1;
                options.adapter = Some(required_value(args, i, "--adapter"));
            }
            "--adapter-arg" => {
                i += 1;
                options
                    .adapter_args
                    .push(required_value(args, i, "--adapter-arg"));
            }
            "--case" => {
                i += 1;
                options.case_ids.push(required_value(args, i, "--case"));
            }
            "--require-invariant" => {
                i += 1;
                options
                    .required_invariants
                    .push(required_value(args, i, "--require-invariant"));
            }
            "--report" => {
                i += 1;
                options.report = Some(PathBuf::from(required_value(args, i, "--report")));
            }
            "--allow-skips" => options.allow_skips = true,
            "--help" | "-h" => {
                print_usage();
                process::exit(0);
            }
            other => fail(&format!("unknown fuzz run argument `{other}`")),
        }
        i += 1;
    }
    options
}

fn required_value(args: &[String], index: usize, flag: &str) -> String {
    args.get(index)
        .cloned()
        .unwrap_or_else(|| fail(&format!("{flag} requires a value")))
}

fn default_plan_path(program: &str) -> PathBuf {
    PathBuf::from("target")
        .join("hopper")
        .join("fuzz")
        .join(format!("{}.plan.json", slug(program)))
}

fn write_plan(path: &Path, plan: &ManifestFuzzPlan) {
    write_json(path, plan, "fuzz plan");
}

fn write_json<T: Serialize>(path: &Path, value: &T, label: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .unwrap_or_else(|error| fail(&format!("cannot create {}: {error}", parent.display())));
    }
    let mut json = serde_json::to_string_pretty(value)
        .unwrap_or_else(|error| fail(&format!("cannot serialize {label}: {error}")));
    json.push('\n');
    fs::write(path, json)
        .unwrap_or_else(|error| fail(&format!("cannot write {label} {}: {error}", path.display())));
}

fn write_seed_corpus(dir: &Path, plan: &ManifestFuzzPlan) {
    fs::create_dir_all(dir)
        .unwrap_or_else(|error| fail(&format!("cannot create {}: {error}", dir.display())));
    for (index, case) in plan.cases.iter().enumerate() {
        let path = dir.join(format!("{:04}-{}.json", index + 1, slug(&case.id)));
        let mut json = serde_json::to_string(case).expect("fuzz case is serializable");
        json.push('\n');
        fs::write(&path, json)
            .unwrap_or_else(|error| fail(&format!("cannot write {}: {error}", path.display())));
    }
}

fn build_request(
    plan: &ManifestFuzzPlan,
    selected_ids: &[String],
    custom_invariants: &[String],
) -> Result<HarnessRequest, String> {
    let selected: BTreeSet<&str> = selected_ids.iter().map(String::as_str).collect();
    if selected.len() != selected_ids.len() {
        return Err("duplicate --case selector".to_string());
    }

    let known: BTreeSet<&str> = plan.cases.iter().map(|case| case.id.as_str()).collect();
    if let Some(unknown) = selected.iter().find(|id| !known.contains(**id)) {
        return Err(format!("unknown generated case `{unknown}`"));
    }

    let mut required = BTreeSet::new();
    for invariant in custom_invariants {
        let invariant = invariant.trim();
        if invariant.is_empty() {
            return Err("--require-invariant cannot be empty".to_string());
        }
        required.insert(invariant.to_string());
    }

    let cases = plan
        .cases
        .iter()
        .filter(|case| selected.is_empty() || selected.contains(case.id.as_str()))
        .cloned()
        .map(|mut case| {
            let mut invariants: BTreeSet<String> = case.required_invariants.into_iter().collect();
            invariants.extend(required.iter().cloned());
            case.required_invariants = invariants.into_iter().collect();
            case
        })
        .collect();

    Ok(HarnessRequest {
        schema: HARNESS_REQUEST_SCHEMA.to_string(),
        program: plan.program.clone(),
        program_version: plan.program_version.clone(),
        contract_commitment: plan.contract_commitment.clone(),
        cases,
    })
}

fn invoke_adapter(
    adapter: &str,
    adapter_args: &[String],
    request: &HarnessRequest,
) -> Result<HarnessResponse, String> {
    let mut child = Command::new(adapter)
        .args(adapter_args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|error| format!("cannot start fuzz adapter `{adapter}`: {error}"))?;

    let mut input = serde_json::to_vec(request)
        .map_err(|error| format!("cannot serialize fuzz adapter request: {error}"))?;
    input.push(b'\n');
    let write_result = child
        .stdin
        .take()
        .ok_or_else(|| "fuzz adapter stdin was not piped".to_string())
        .and_then(|mut stdin| {
            stdin
                .write_all(&input)
                .map_err(|error| format!("cannot write fuzz adapter request: {error}"))
        });
    if let Err(error) = write_result {
        let _ = child.kill();
        let _ = child.wait();
        return Err(error);
    }

    let output = child
        .wait_with_output()
        .map_err(|error| format!("cannot wait for fuzz adapter `{adapter}`: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "fuzz adapter `{adapter}` exited with {}",
            output
                .status
                .code()
                .map_or_else(|| "a signal".to_string(), |code| format!("status {code}"))
        ));
    }
    if output.stdout.is_empty() {
        return Err(format!(
            "fuzz adapter `{adapter}` returned no JSON response on stdout"
        ));
    }
    serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("invalid JSON response from fuzz adapter `{adapter}`: {error}"))
}

fn validate_response(
    request: &HarnessRequest,
    response: HarnessResponse,
    allow_skips: bool,
) -> Result<RunValidation, String> {
    if response.schema != HARNESS_RESPONSE_SCHEMA {
        return Err(format!(
            "unsupported adapter response schema `{}`; expected `{HARNESS_RESPONSE_SCHEMA}`",
            response.schema
        ));
    }
    if response.contract_commitment != request.contract_commitment {
        return Err(format!(
            "adapter response commitment {} does not match request {}",
            response.contract_commitment, request.contract_commitment
        ));
    }

    let requested: BTreeSet<&str> = request.cases.iter().map(|case| case.id.as_str()).collect();
    let mut results = BTreeMap::new();
    for result in response.results {
        if !requested.contains(result.id.as_str()) {
            return Err(format!(
                "adapter returned unknown or unselected case `{}`",
                result.id
            ));
        }
        let id = result.id.clone();
        if results.insert(id.clone(), result).is_some() {
            return Err(format!("adapter returned duplicate result `{id}`"));
        }
    }
    if results.len() != request.cases.len() {
        let missing = request
            .cases
            .iter()
            .find(|case| !results.contains_key(&case.id))
            .map_or("<unknown>", |case| case.id.as_str());
        return Err(format!("adapter omitted generated case `{missing}`"));
    }

    let mut validation = RunValidation::default();
    for case in &request.cases {
        let mut result = results
            .remove(&case.id)
            .expect("complete result set was checked above");
        let checked: BTreeSet<&str> = result
            .checked_invariants
            .iter()
            .map(String::as_str)
            .collect();
        let missing: Vec<&str> = case
            .required_invariants
            .iter()
            .map(String::as_str)
            .filter(|invariant| !checked.contains(invariant))
            .collect();

        match result.outcome {
            HarnessOutcome::Passed if missing.is_empty() => validation.summary.passed += 1,
            HarnessOutcome::Skipped if allow_skips => validation.summary.skipped += 1,
            HarnessOutcome::Passed => {
                result.outcome = HarnessOutcome::Failed;
                append_detail(
                    &mut result.detail,
                    &format!("missing required invariants: {}", missing.join(", ")),
                );
                validation.summary.failed += 1;
                validation.failures.push(format!(
                    "{} omitted invariants {}",
                    case.id,
                    missing.join(", ")
                ));
            }
            HarnessOutcome::Skipped => {
                result.outcome = HarnessOutcome::Failed;
                append_detail(
                    &mut result.detail,
                    "adapter skipped case without --allow-skips",
                );
                validation.summary.failed += 1;
                validation.failures.push(format!("{} was skipped", case.id));
            }
            HarnessOutcome::Failed => {
                validation.summary.failed += 1;
                validation.failures.push(if result.detail.is_empty() {
                    format!("{} failed", case.id)
                } else {
                    format!("{} failed: {}", case.id, result.detail)
                });
            }
        }
        validation.results.push(result);
    }
    Ok(validation)
}

fn append_detail(detail: &mut String, message: &str) {
    if !detail.is_empty() {
        detail.push_str("; ");
    }
    detail.push_str(message);
}

fn build_plan(manifest: &ProgramManifest) -> ManifestFuzzPlan {
    let mut builder = PlanBuilder::default();

    builder.coverage.layout_metadata = manifest.layout_metadata.len();
    builder.coverage.contexts = manifest.contexts.len();
    builder.coverage.context_accounts = manifest
        .contexts
        .iter()
        .map(|context| context.accounts.len())
        .sum();
    builder.coverage.policies = manifest.policies.len();

    for (policy_index, policy) in manifest.policies.iter().enumerate() {
        builder.coverage.policy_requirements += policy.requirements.len();
        builder.coverage.policy_invariants += policy.invariants.len();
        for (requirement_index, requirement) in policy.requirements.iter().enumerate() {
            builder.case(
                format!("policy-{policy_index}-requirement-{requirement_index}"),
                "policy-requirement",
                &format!("policy:{}.requirement:{requirement}", policy.name),
                "must-reject-unsatisfied-requirement-before-write",
                params(&[
                    ("policyIndex", policy_index as u64),
                    ("requirementIndex", requirement_index as u64),
                ]),
            );
        }
        for (invariant_index, invariant) in policy.invariants.iter().enumerate() {
            builder.case(
                format!("policy-{policy_index}-invariant-{invariant_index}"),
                "policy-invariant",
                &format!("policy:{}.invariant:{invariant}", policy.name),
                "must-reject-and-rollback-invariant-violation",
                params(&[
                    ("policyIndex", policy_index as u64),
                    ("invariantIndex", invariant_index as u64),
                ]),
            );
        }
    }

    for (layout_index, layout) in manifest.layouts.iter().enumerate() {
        builder.coverage.layouts += 1;
        builder.coverage.fields += layout.fields.len();
        let target = format!("layout:{}", layout.name);
        for length in [0usize, 1, 15] {
            if length < layout.total_size {
                builder.case(
                    format!("layout-{layout_index}-truncate-{length}"),
                    "layout-truncation",
                    &target,
                    "must-reject-without-panic",
                    params(&[("length", length as u64)]),
                );
            }
        }
        if layout.total_size > 0 {
            builder.case(
                format!("layout-{layout_index}-truncate-body"),
                "layout-truncation",
                &target,
                "must-reject-without-panic",
                params(&[("length", (layout.total_size - 1) as u64)]),
            );
        }
        builder.case(
            format!("layout-{layout_index}-wrong-disc"),
            "layout-identity",
            &target,
            "must-reject",
            params(&[("disc", layout.disc.wrapping_add(1) as u64)]),
        );
        builder.case(
            format!("layout-{layout_index}-wrong-version"),
            "layout-identity",
            &target,
            "must-reject",
            params(&[("version", layout.version.wrapping_add(1) as u64)]),
        );
        builder.case(
            format!("layout-{layout_index}-wrong-layout-id"),
            "layout-identity",
            &target,
            "must-reject",
            params(&[("flipByte", 0)]),
        );
        for (field_index, field) in layout.fields.iter().enumerate() {
            builder.case(
                format!("layout-{layout_index}-field-{field_index}-start"),
                "field-boundary",
                &format!("{target}.{}", field.name),
                "must-remain-in-layout",
                params(&[
                    ("offset", field.offset as u64),
                    ("size", field.size as u64),
                    ("layoutSize", layout.total_size as u64),
                ]),
            );
            let end = field.offset as u64 + field.size as u64;
            if end > 0 {
                builder.case(
                    format!("layout-{layout_index}-field-{field_index}-last-byte"),
                    "field-boundary",
                    &format!("{target}.{}", field.name),
                    "must-remain-in-layout",
                    params(&[("offset", end - 1), ("size", 1)]),
                );
            }
        }
    }

    for (pair_index, pair) in manifest.compatibility_pairs.iter().enumerate() {
        builder.coverage.migration_pairs += 1;
        let from = manifest
            .layouts
            .iter()
            .find(|layout| layout.name == pair.from_layout && layout.version == pair.from_version);
        let to = manifest
            .layouts
            .iter()
            .find(|layout| layout.name == pair.to_layout && layout.version == pair.to_version);
        let policy = migration_policy_code(pair.policy);
        let forward_expectation = match pair.policy {
            MigrationPolicy::NoOp => "must-preserve-bytes-without-migration",
            MigrationPolicy::AppendOnly => "must-preserve-prefix-and-zero-appended-region",
            MigrationPolicy::RequiresMigration => "typed-transform-must-succeed-or-fail-atomically",
            MigrationPolicy::Incompatible => "must-reject-before-write",
        };
        builder.case(
            format!("compatibility-{pair_index}-forward"),
            "migration-direction",
            &format!("{}->{}", pair.from_layout, pair.to_layout),
            forward_expectation,
            params(&[
                ("fromVersion", pair.from_version as u64),
                ("toVersion", pair.to_version as u64),
                (
                    "fromSize",
                    from.map_or(0, |layout| layout.total_size) as u64,
                ),
                ("toSize", to.map_or(0, |layout| layout.total_size) as u64),
                ("policy", policy),
                ("backwardReadable", pair.backward_readable as u64),
            ]),
        );
        builder.case(
            format!("compatibility-{pair_index}-backward-read"),
            "migration-backward-read",
            &format!("{}<-{}", pair.from_layout, pair.to_layout),
            if pair.backward_readable {
                "old-reader-must-accept-declared-compatible-prefix"
            } else {
                "old-reader-must-reject-before-typed-access"
            },
            params(&[
                ("readerVersion", pair.from_version as u64),
                ("storedVersion", pair.to_version as u64),
                ("policy", policy),
                ("backwardReadable", pair.backward_readable as u64),
            ]),
        );
    }

    for (instruction_index, instruction) in manifest.instructions.iter().enumerate() {
        let context = matching_context(instruction, manifest.contexts);
        builder.coverage.matched_instruction_contexts += context.is_some() as usize;
        let write_ranges = if instruction.write_ranges.is_empty() {
            context.map_or(&[][..], |context| context.write_ranges)
        } else {
            instruction.write_ranges
        };
        let parametric_write_ranges = if instruction.parametric_write_ranges.is_empty() {
            context.map_or(&[][..], |context| context.parametric_write_ranges)
        } else {
            instruction.parametric_write_ranges
        };
        let mutation_complete = instruction.mutation_complete
            || context.is_some_and(|context| context.mutation_complete);
        let lamport_accounts = if instruction.lamport_accounts.is_empty() {
            context.map_or(&[][..], |context| context.lamport_accounts)
        } else {
            instruction.lamport_accounts
        };
        let strict_writes =
            instruction.strict_writes || context.is_some_and(|context| context.strict_writes);
        builder.coverage.instructions += 1;
        builder.coverage.instruction_arguments += instruction.args.len();
        builder.coverage.account_roles += instruction.accounts.len();
        builder.coverage.write_ranges += write_ranges.len();
        builder.coverage.parametric_write_ranges += parametric_write_ranges.len();
        builder.coverage.lamport_permissions += lamport_accounts.len();
        builder.coverage.strict_write_instructions += strict_writes as usize;
        builder.coverage.mutation_complete_instructions += mutation_complete as usize;
        let target = format!("instruction:{}", instruction.name);

        let mut attached_policies = BTreeSet::new();
        if !instruction.policy_pack.is_empty() {
            attached_policies.insert(instruction.policy_pack);
        }
        if let Some(context) = context {
            attached_policies.extend(context.policies.iter().copied());
            attached_policies.extend(
                context
                    .accounts
                    .iter()
                    .map(|account| account.policy_ref)
                    .filter(|policy| !policy.is_empty()),
            );
        }
        for (attachment_index, policy) in attached_policies.into_iter().enumerate() {
            builder.coverage.policy_attachments += 1;
            builder.case(
                format!("instruction-{instruction_index}-policy-{attachment_index}"),
                "policy-attachment",
                &format!("{target}.policy:{policy}"),
                "must-enforce-attached-policy-before-commit",
                params(&[("policyAttachment", attachment_index as u64)]),
            );
        }
        if instruction.receipt_expected || context.is_some_and(|context| context.receipts_expected)
        {
            builder.case(
                format!("instruction-{instruction_index}-receipt"),
                "receipt-contract",
                &target,
                "must-emit-verifiable-receipt-on-success",
                BTreeMap::new(),
            );
        }
        let discriminator_len = instruction.discriminator_bytes().len();
        builder.case(
            format!("instruction-{instruction_index}-truncated-discriminator"),
            "instruction-truncation",
            &target,
            "must-reject-without-panic",
            params(&[("length", discriminator_len.saturating_sub(1) as u64)]),
        );

        let mut wire_offset = discriminator_len as u64;
        for (arg_index, arg) in instruction.args.iter().enumerate() {
            match arg.encoding {
                ArgEncoding::Fixed => {
                    if arg.size > 0 {
                        builder.case(
                            format!("instruction-{instruction_index}-arg-{arg_index}-truncated"),
                            "argument-boundary",
                            &format!("{target}.{}", arg.name),
                            "must-reject-without-panic",
                            params(&[("length", wire_offset + arg.size as u64 - 1)]),
                        );
                    }
                    wire_offset += arg.size as u64;
                }
                ArgEncoding::BoundedVec {
                    max_len,
                    element_size,
                } => {
                    for selected in [0u64, max_len as u64, max_len as u64 + 1] {
                        builder.case(
                            format!(
                                "instruction-{instruction_index}-arg-{arg_index}-vec-{selected}"
                            ),
                            "bounded-argument",
                            &format!("{target}.{}", arg.name),
                            if selected <= max_len as u64 {
                                "must-parse-when-payload-fits"
                            } else {
                                "must-reject"
                            },
                            params(&[
                                ("selectedLength", selected),
                                ("maxLength", max_len as u64),
                                ("elementSize", element_size as u64),
                            ]),
                        );
                    }
                    wire_offset += arg.size as u64;
                }
                ArgEncoding::BoundedString { max_len } => {
                    for selected in [0u64, max_len as u64, max_len as u64 + 1] {
                        builder.case(
                            format!(
                                "instruction-{instruction_index}-arg-{arg_index}-string-{selected}"
                            ),
                            "bounded-argument",
                            &format!("{target}.{}", arg.name),
                            if selected <= max_len as u64 {
                                "must-validate-utf8-and-bounds"
                            } else {
                                "must-reject"
                            },
                            params(&[("selectedLength", selected), ("maxLength", max_len as u64)]),
                        );
                    }
                    wire_offset += arg.size as u64;
                }
            }
        }

        for (account_index, account) in instruction.accounts.iter().enumerate() {
            let context_account = context.and_then(|context| context.accounts.get(account_index));
            let layout_ref = if account.layout_ref.is_empty() {
                context_account.map_or("", |account| account.layout_ref)
            } else {
                account.layout_ref
            };
            let seeds = if account.seeds.is_empty() {
                context_account.map_or(&[][..], |account| account.seeds)
            } else {
                account.seeds
            };
            if account.signer {
                builder.case(
                    format!(
                        "instruction-{instruction_index}-account-{account_index}-missing-signer"
                    ),
                    "account-privilege",
                    &format!("{target}.{}", account.name),
                    "must-reject",
                    params(&[("accountIndex", account_index as u64)]),
                );
            }
            if account.writable {
                builder.case(
                    format!("instruction-{instruction_index}-account-{account_index}-readonly"),
                    "account-privilege",
                    &format!("{target}.{}", account.name),
                    "must-reject-before-write",
                    params(&[("accountIndex", account_index as u64)]),
                );
            }
            if !layout_ref.is_empty() {
                builder.coverage.typed_account_constraints += 1;
                for mutation in ["wrong-owner", "wrong-layout"] {
                    builder.case(
                        format!(
                            "instruction-{instruction_index}-account-{account_index}-{mutation}"
                        ),
                        "typed-account-substitution",
                        &format!("{target}.{}", account.name),
                        "must-reject-before-typed-access",
                        params(&[("accountIndex", account_index as u64)]),
                    );
                }
            }
            if !seeds.is_empty() {
                builder.coverage.pda_constraints += 1;
                builder.case(
                    format!("instruction-{instruction_index}-account-{account_index}-wrong-pda"),
                    "pda-substitution",
                    &format!("{target}.{}", account.name),
                    "must-reject",
                    params(&[
                        ("accountIndex", account_index as u64),
                        ("seedCount", seeds.len() as u64),
                    ]),
                );
            }

            let Some(context_account) = context_account else {
                continue;
            };
            if context_account.lifecycle != AccountLifecycle::Existing {
                builder.coverage.lifecycle_constraints += 1;
                let payer_index = context
                    .and_then(|context| {
                        context
                            .accounts
                            .iter()
                            .position(|candidate| candidate.name == context_account.payer)
                    })
                    .map_or(u64::MAX, |index| index as u64);
                let expectation = match context_account.lifecycle {
                    AccountLifecycle::Existing => unreachable!(),
                    AccountLifecycle::Init => "must-reject-preinitialized-account-before-write",
                    AccountLifecycle::InitIfNeeded => {
                        "must-reject-invalid-existing-account-or-initialize-atomically"
                    }
                    AccountLifecycle::Realloc => "must-reject-invalid-resize-and-preserve-account",
                    AccountLifecycle::Close => {
                        "must-close-only-after-success-and-preserve-recipient"
                    }
                };
                builder.case(
                    format!("instruction-{instruction_index}-account-{account_index}-lifecycle"),
                    "account-lifecycle",
                    &format!("{target}.{}", account.name),
                    expectation,
                    params(&[
                        ("accountIndex", account_index as u64),
                        ("lifecycle", lifecycle_code(context_account.lifecycle)),
                        ("payerIndex", payer_index),
                        ("initSpace", context_account.init_space as u64),
                    ]),
                );
            }
            for (constraint_index, related_account) in context_account.has_one.iter().enumerate() {
                builder.coverage.has_one_constraints += 1;
                let related_index = context
                    .and_then(|context| {
                        context
                            .accounts
                            .iter()
                            .position(|candidate| candidate.name == *related_account)
                    })
                    .map_or(u64::MAX, |index| index as u64);
                builder.case(
                    format!(
                        "instruction-{instruction_index}-account-{account_index}-has-one-{constraint_index}"
                    ),
                    "has-one-substitution",
                    &format!("{target}.{}->{related_account}", account.name),
                    "must-reject-related-key-mismatch-before-write",
                    params(&[
                        ("accountIndex", account_index as u64),
                        ("relatedAccountIndex", related_index),
                    ]),
                );
            }
            if !context_account.expected_owner.is_empty() {
                builder.coverage.owner_constraints += 1;
                builder.case(
                    format!(
                        "instruction-{instruction_index}-account-{account_index}-expected-owner"
                    ),
                    "owner-substitution",
                    &format!("{target}.{}", account.name),
                    "must-reject-owner-mismatch-before-access",
                    params(&[("accountIndex", account_index as u64)]),
                );
            }
            if !context_account.expected_address.is_empty() {
                builder.coverage.address_constraints += 1;
                builder.case(
                    format!(
                        "instruction-{instruction_index}-account-{account_index}-expected-address"
                    ),
                    "address-substitution",
                    &format!("{target}.{}", account.name),
                    "must-reject-address-mismatch-before-access",
                    params(&[("accountIndex", account_index as u64)]),
                );
            }
            if context_account.optional {
                builder.coverage.optional_account_constraints += 1;
                for (variant, expectation) in [
                    (
                        "absent",
                        "must-handle-absence-without-shifting-account-roles",
                    ),
                    ("invalid-present", "must-reject-invalid-present-account"),
                ] {
                    builder.case(
                        format!(
                            "instruction-{instruction_index}-account-{account_index}-optional-{variant}"
                        ),
                        "optional-account-boundary",
                        &format!("{target}.{}", account.name),
                        expectation,
                        params(&[("accountIndex", account_index as u64)]),
                    );
                }
            }
        }

        for left in 0..instruction.accounts.len() {
            for right in (left + 1)..instruction.accounts.len() {
                builder.coverage.alias_pairs += 1;
                builder.case(
                    format!("instruction-{instruction_index}-alias-{left}-{right}"),
                    "duplicate-account-alias",
                    &target,
                    "must-preserve-borrow-and-role-invariants",
                    params(&[("left", left as u64), ("right", right as u64)]),
                );
            }
        }

        let mut emitted_write_escapes = BTreeSet::new();
        for (range_index, range) in write_ranges.iter().enumerate() {
            builder.case(
                format!("instruction-{instruction_index}-write-{range_index}-exact"),
                "write-range-boundary",
                &target,
                "must-authorize",
                params(&[
                    ("accountIndex", range.account_index as u64),
                    ("offset", range.offset as u64),
                    ("size", range.size as u64),
                ]),
            );
            if range.offset > 0
                && register_union_write_escape(
                    write_ranges,
                    &mut emitted_write_escapes,
                    range.account_index,
                    range.offset - 1,
                    1,
                )
            {
                builder.case(
                    format!("instruction-{instruction_index}-write-{range_index}-before"),
                    "write-range-escape",
                    &target,
                    "must-reject-before-write",
                    params(&[
                        ("accountIndex", range.account_index as u64),
                        ("offset", range.offset as u64 - 1),
                        ("size", 1),
                    ]),
                );
            }
            if range.size != u32::MAX
                && register_union_write_escape(
                    write_ranges,
                    &mut emitted_write_escapes,
                    range.account_index,
                    range.offset + range.size,
                    1,
                )
            {
                builder.case(
                    format!("instruction-{instruction_index}-write-{range_index}-after"),
                    "write-range-escape",
                    &target,
                    "must-reject-before-write",
                    params(&[
                        ("accountIndex", range.account_index as u64),
                        ("offset", range.offset as u64 + range.size as u64),
                        ("size", 1),
                    ]),
                );
            }
        }

        for (range_index, range) in parametric_write_ranges.iter().enumerate() {
            for (label, selected, expectation) in [
                ("first", 0, "must-authorize-only-selected-cell"),
                (
                    "last",
                    range.count.saturating_sub(1),
                    "must-authorize-only-selected-cell",
                ),
                ("overflow", range.count, "must-reject-before-write"),
            ] {
                builder.case(
                    format!("instruction-{instruction_index}-parametric-{range_index}-{label}"),
                    "parametric-write-selector",
                    &format!("{target}.{}", range.segment_name),
                    expectation,
                    params(&[
                        ("accountIndex", range.account_index as u64),
                        ("selected", selected as u64),
                        ("count", range.count as u64),
                        ("stride", range.stride as u64),
                        ("cellSize", range.cell_size as u64),
                    ]),
                );
            }
        }

        if mutation_complete {
            for account_index in 0..instruction.accounts.len() {
                let allowed = u8::try_from(account_index)
                    .ok()
                    .is_some_and(|index| lamport_accounts.contains(&index));
                builder.case(
                    format!("instruction-{instruction_index}-lamports-{account_index}"),
                    "lamport-permission",
                    &target,
                    if allowed {
                        "must-authorize"
                    } else {
                        "must-reject-before-balance-change"
                    },
                    params(&[("accountIndex", account_index as u64)]),
                );
            }
        }

        if let Some(remaining) = instruction.remaining_accounts {
            for (label, count, expectation) in [
                ("empty", 0u64, "must-accept-count"),
                ("max", remaining.max as u64, "must-accept-count"),
                (
                    "overflow",
                    remaining.max as u64 + 1,
                    "must-reject-before-dispatch",
                ),
            ] {
                builder.case(
                    format!("instruction-{instruction_index}-remaining-{label}"),
                    "remaining-account-boundary",
                    &target,
                    expectation,
                    params(&[("count", count), ("max", remaining.max as u64)]),
                );
            }
        }
    }

    builder.finish(manifest)
}

fn migration_policy_code(policy: MigrationPolicy) -> u64 {
    match policy {
        MigrationPolicy::NoOp => 0,
        MigrationPolicy::AppendOnly => 1,
        MigrationPolicy::RequiresMigration => 2,
        MigrationPolicy::Incompatible => 3,
    }
}

fn lifecycle_code(lifecycle: AccountLifecycle) -> u64 {
    match lifecycle {
        AccountLifecycle::Existing => 0,
        AccountLifecycle::Init => 1,
        AccountLifecycle::InitIfNeeded => 2,
        AccountLifecycle::Realloc => 3,
        AccountLifecycle::Close => 4,
    }
}

/// Register a hostile write probe only when the requested byte range is
/// outside the instruction's complete authorized union.
///
/// A boundary byte immediately after one declared range can be the first byte
/// of an adjacent or overlapping range. Treating that byte as an escape would
/// generate a false rejection expectation and let a transport-only adapter
/// masquerade as execution evidence. The set also prevents identical/nested
/// ranges from emitting duplicate probes.
fn register_union_write_escape(
    write_ranges: &[hopper_schema::WriteRange],
    emitted: &mut BTreeSet<(u8, u32, u32)>,
    account_index: u8,
    offset: u32,
    size: u32,
) -> bool {
    if write_ranges
        .iter()
        .any(|range| range.account_index == account_index && range.contains(offset, size))
    {
        return false;
    }
    emitted.insert((account_index, offset, size))
}

fn matching_context<'a>(
    instruction: &InstructionDescriptor,
    contexts: &'a [ContextDescriptor],
) -> Option<&'a ContextDescriptor> {
    let instruction_name = canonical_identifier(instruction.name);
    if let Some(named) = contexts
        .iter()
        .filter(|context| context_matches_instruction(instruction, context))
        .find(|context| canonical_identifier(context.name) == instruction_name)
    {
        return Some(named);
    }
    let mut candidates = contexts
        .iter()
        .filter(|context| context_matches_instruction(instruction, context));
    let only = candidates.next()?;
    candidates.next().is_none().then_some(only)
}

fn context_matches_instruction(
    instruction: &InstructionDescriptor,
    context: &ContextDescriptor,
) -> bool {
    context.accounts.len() == instruction.accounts.len()
        && instruction.accounts.iter().zip(context.accounts).all(
            |(instruction_account, context_account)| {
                instruction_account.name == context_account.name
                    && instruction_account.writable == context_account.writable
                    && instruction_account.signer == context_account.signer
                    && (instruction_account.layout_ref.is_empty()
                        || context_account.layout_ref.is_empty()
                        || instruction_account.layout_ref == context_account.layout_ref)
            },
        )
}

fn canonical_identifier(value: &str) -> String {
    value
        .bytes()
        .filter(u8::is_ascii_alphanumeric)
        .map(|byte| byte.to_ascii_lowercase() as char)
        .collect()
}

#[derive(Default)]
struct PlanBuilder {
    cases: Vec<FuzzCase>,
    ids: BTreeSet<String>,
    coverage: FuzzCoverage,
}

impl PlanBuilder {
    fn case(
        &mut self,
        id: String,
        kind: &str,
        target: &str,
        expectation: &str,
        parameters: BTreeMap<String, u64>,
    ) {
        assert!(
            self.ids.insert(id.clone()),
            "duplicate generated case id: {id}"
        );
        self.cases.push(FuzzCase {
            id,
            seed: String::new(),
            kind: kind.to_string(),
            target: target.to_string(),
            expectation: expectation.to_string(),
            required_invariants: Vec::new(),
            parameters,
        });
    }

    fn finish(mut self, manifest: &ProgramManifest) -> ManifestFuzzPlan {
        let source_commitment = fuzz_contract_digest(manifest);
        for case in &mut self.cases {
            case.seed = case_seed(manifest, &source_commitment, case);
            case.required_invariants = required_invariants(case);
        }
        self.coverage.generated_cases = self.cases.len();
        let preimage = serde_json::to_vec(&(
            PLAN_SCHEMA,
            manifest.name,
            manifest.version,
            hex_lower(&source_commitment),
            &self.coverage,
            &self.cases,
        ))
        .expect("fuzz plan commitment input is serializable");
        let commitment = format!("{:x}", Sha256::digest(preimage));
        ManifestFuzzPlan {
            schema: PLAN_SCHEMA.to_string(),
            program: manifest.name.to_string(),
            program_version: manifest.version.to_string(),
            contract_commitment: commitment,
            coverage: self.coverage,
            cases: self.cases,
        }
    }
}

fn case_seed(manifest: &ProgramManifest, source_commitment: &[u8; 32], case: &FuzzCase) -> String {
    let preimage = serde_json::to_vec(&(
        CASE_SEED_SCHEMA,
        manifest.name,
        manifest.version,
        hex_lower(source_commitment),
        &case.id,
        &case.kind,
        &case.target,
        &case.expectation,
        &case.parameters,
    ))
    .expect("fuzz seed input is serializable");
    let digest = Sha256::digest(preimage);
    hex_lower(&digest[..16])
}

/// Hash every layout and instruction property that can affect a generated
/// mutation or the program contract an adapter executes. Case IDs alone are
/// not sufficient: a discriminator, layout fingerprint, PDA seed expression,
/// policy name, or parametric base can change while case counts stay equal.
fn fuzz_contract_digest(manifest: &ProgramManifest) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hash_string(&mut hasher, "hopper.manifest-fuzz-source.v2");
    hash_string(&mut hasher, manifest.name);
    hash_string(&mut hasher, manifest.version);

    hash_u64(&mut hasher, manifest.layouts.len() as u64);
    for layout in manifest.layouts {
        hash_string(&mut hasher, layout.name);
        hash_u8(&mut hasher, layout.disc);
        hash_u8(&mut hasher, layout.version);
        hash_bytes(&mut hasher, &layout.layout_id);
        hash_u64(&mut hasher, layout.total_size as u64);
        hash_u64(&mut hasher, layout.field_count as u64);
        hash_u64(&mut hasher, layout.fields.len() as u64);
        for field in layout.fields {
            hash_string(&mut hasher, field.name);
            hash_string(&mut hasher, field.canonical_type);
            hash_u16(&mut hasher, field.size);
            hash_u16(&mut hasher, field.offset);
            hash_string(&mut hasher, field.intent.name());
        }
    }

    hash_u64(&mut hasher, manifest.layout_metadata.len() as u64);
    for metadata in manifest.layout_metadata {
        hash_string(&mut hasher, metadata.name);
        hash_string_slice(&mut hasher, metadata.segment_roles);
        hash_bool(&mut hasher, metadata.append_safe);
        hash_bool(&mut hasher, metadata.migration_required);
        hash_bool(&mut hasher, metadata.rebuildable);
        hash_string(&mut hasher, metadata.policy_pack);
        hash_string_slice(&mut hasher, metadata.invariant_pack);
        hash_string(&mut hasher, metadata.receipt_profile);
        hash_string_slice(&mut hasher, metadata.phase_requirements);
        hash_string(&mut hasher, metadata.trust_profile);
        hash_string_slice(&mut hasher, metadata.manager_hints);
    }

    hash_u64(&mut hasher, manifest.instructions.len() as u64);
    for instruction in manifest.instructions {
        hash_string(&mut hasher, instruction.name);
        hash_u8(&mut hasher, instruction.tag);
        hash_bytes(&mut hasher, instruction.discriminator_bytes());

        hash_u64(&mut hasher, instruction.args.len() as u64);
        for arg in instruction.args {
            hash_string(&mut hasher, arg.name);
            hash_string(&mut hasher, arg.canonical_type);
            hash_u16(&mut hasher, arg.size);
            match arg.encoding {
                ArgEncoding::Fixed => hash_u8(&mut hasher, 0),
                ArgEncoding::BoundedVec {
                    max_len,
                    element_size,
                } => {
                    hash_u8(&mut hasher, 1);
                    hash_u16(&mut hasher, max_len);
                    hash_u16(&mut hasher, element_size);
                }
                ArgEncoding::BoundedString { max_len } => {
                    hash_u8(&mut hasher, 2);
                    hash_u16(&mut hasher, max_len);
                }
            }
        }

        hash_u64(&mut hasher, instruction.accounts.len() as u64);
        for account in instruction.accounts {
            hash_string(&mut hasher, account.name);
            hash_bool(&mut hasher, account.writable);
            hash_bool(&mut hasher, account.signer);
            hash_string(&mut hasher, account.layout_ref);
            hash_u64(&mut hasher, account.seeds.len() as u64);
            for seed in account.seeds {
                hash_string(&mut hasher, seed);
            }
        }

        match instruction.remaining_accounts {
            Some(remaining) => {
                hash_bool(&mut hasher, true);
                hash_u16(&mut hasher, remaining.max);
            }
            None => hash_bool(&mut hasher, false),
        }
        hash_u64(&mut hasher, instruction.capabilities.len() as u64);
        for capability in instruction.capabilities {
            hash_string(&mut hasher, capability);
        }
        hash_string(&mut hasher, instruction.policy_pack);
        hash_bool(&mut hasher, instruction.receipt_expected);
        hash_bool(&mut hasher, instruction.strict_writes);

        hash_u64(&mut hasher, instruction.write_ranges.len() as u64);
        for range in instruction.write_ranges {
            hash_u8(&mut hasher, range.account_index);
            hash_u32(&mut hasher, range.offset);
            hash_u32(&mut hasher, range.size);
        }
        hash_u64(
            &mut hasher,
            instruction.parametric_write_ranges.len() as u64,
        );
        for range in instruction.parametric_write_ranges {
            hash_u8(&mut hasher, range.account_index);
            hash_u32(&mut hasher, range.base_offset);
            hash_u32(&mut hasher, range.stride);
            hash_u32(&mut hasher, range.cell_size);
            hash_u32(&mut hasher, range.count);
            hash_u8(&mut hasher, range.argument_index);
            hash_string(&mut hasher, range.argument_name);
            hash_string(&mut hasher, range.segment_name);
        }
        hash_bool(&mut hasher, instruction.mutation_complete);
        hash_bytes(&mut hasher, instruction.lamport_accounts);
        hash_u32(&mut hasher, instruction.cu_estimate);
    }

    hash_u64(&mut hasher, manifest.policies.len() as u64);
    for policy in manifest.policies {
        hash_string(&mut hasher, policy.name);
        hash_string_slice(&mut hasher, policy.capabilities);
        hash_string_slice(&mut hasher, policy.requirements);
        hash_string_slice(&mut hasher, policy.invariants);
        hash_string(&mut hasher, policy.receipt_profile);
    }

    hash_u64(&mut hasher, manifest.compatibility_pairs.len() as u64);
    for pair in manifest.compatibility_pairs {
        hash_string(&mut hasher, pair.from_layout);
        hash_u8(&mut hasher, pair.from_version);
        hash_string(&mut hasher, pair.to_layout);
        hash_u8(&mut hasher, pair.to_version);
        hash_u64(&mut hasher, migration_policy_code(pair.policy));
        hash_bool(&mut hasher, pair.backward_readable);
    }

    hash_u64(&mut hasher, manifest.contexts.len() as u64);
    for context in manifest.contexts {
        hash_string(&mut hasher, context.name);
        hash_u64(&mut hasher, context.accounts.len() as u64);
        for account in context.accounts {
            hash_string(&mut hasher, account.name);
            hash_string(&mut hasher, account.kind);
            hash_bool(&mut hasher, account.writable);
            hash_bool(&mut hasher, account.signer);
            hash_string(&mut hasher, account.layout_ref);
            hash_string(&mut hasher, account.policy_ref);
            hash_string_slice(&mut hasher, account.seeds);
            hash_bool(&mut hasher, account.optional);
            hash_u64(&mut hasher, lifecycle_code(account.lifecycle));
            hash_string(&mut hasher, account.payer);
            hash_u32(&mut hasher, account.init_space);
            hash_string_slice(&mut hasher, account.has_one);
            hash_string(&mut hasher, account.expected_address);
            hash_string(&mut hasher, account.expected_owner);
        }
        hash_string_slice(&mut hasher, context.policies);
        hash_bool(&mut hasher, context.receipts_expected);
        hash_string_slice(&mut hasher, context.mutation_classes);
        hash_bool(&mut hasher, context.strict_writes);
        hash_u64(&mut hasher, context.write_ranges.len() as u64);
        for range in context.write_ranges {
            hash_u8(&mut hasher, range.account_index);
            hash_u32(&mut hasher, range.offset);
            hash_u32(&mut hasher, range.size);
        }
        hash_u64(&mut hasher, context.parametric_write_ranges.len() as u64);
        for range in context.parametric_write_ranges {
            hash_u8(&mut hasher, range.account_index);
            hash_u32(&mut hasher, range.base_offset);
            hash_u32(&mut hasher, range.stride);
            hash_u32(&mut hasher, range.cell_size);
            hash_u32(&mut hasher, range.count);
            hash_u8(&mut hasher, range.argument_index);
            hash_string(&mut hasher, range.argument_name);
            hash_string(&mut hasher, range.segment_name);
        }
        hash_bool(&mut hasher, context.mutation_complete);
        hash_bytes(&mut hasher, context.lamport_accounts);
    }

    let digest = hasher.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    out
}

fn hash_string_slice(hasher: &mut Sha256, values: &[&str]) {
    hash_u64(hasher, values.len() as u64);
    for value in values {
        hash_string(hasher, value);
    }
}

fn hash_bytes(hasher: &mut Sha256, bytes: &[u8]) {
    hash_u64(hasher, bytes.len() as u64);
    hasher.update(bytes);
}

fn hash_string(hasher: &mut Sha256, value: &str) {
    hash_bytes(hasher, value.as_bytes());
}

fn hash_bool(hasher: &mut Sha256, value: bool) {
    hash_u8(hasher, u8::from(value));
}

fn hash_u8(hasher: &mut Sha256, value: u8) {
    hasher.update([value]);
}

fn hash_u16(hasher: &mut Sha256, value: u16) {
    hasher.update(value.to_le_bytes());
}

fn hash_u32(hasher: &mut Sha256, value: u32) {
    hasher.update(value.to_le_bytes());
}

fn hash_u64(hasher: &mut Sha256, value: u64) {
    hasher.update(value.to_le_bytes());
}

fn required_invariants(case: &FuzzCase) -> Vec<String> {
    let mut invariants = BTreeSet::from(["no-panic".to_string()]);
    if case.expectation.contains("reject") || case.expectation.contains("fail-atomically") {
        invariants.insert("atomic-rejection".to_string());
    }
    let contract = match case.kind.as_str() {
        "layout-truncation" | "layout-identity" => "layout-gate",
        "field-boundary" => "field-bounds",
        "migration-direction" | "migration-backward-read" => "migration-postconditions",
        "instruction-truncation" | "argument-boundary" | "bounded-argument" => "instruction-parser",
        "account-privilege"
        | "typed-account-substitution"
        | "pda-substitution"
        | "account-lifecycle"
        | "has-one-substitution"
        | "owner-substitution"
        | "address-substitution"
        | "optional-account-boundary" => "account-constraints",
        "policy-requirement" | "policy-invariant" | "policy-attachment" => "policy-enforcement",
        "receipt-contract" => "receipt-integrity",
        "duplicate-account-alias" => "alias-safety",
        "write-range-boundary" | "write-range-escape" | "parametric-write-selector" => {
            "write-confinement"
        }
        "lamport-permission" => "lamport-confinement",
        "remaining-account-boundary" => "remaining-account-bounds",
        _ => "manifest-expectation",
    };
    invariants.insert(contract.to_string());
    invariants.into_iter().collect()
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

fn params(entries: &[(&str, u64)]) -> BTreeMap<String, u64> {
    entries
        .iter()
        .map(|(key, value)| ((*key).to_string(), *value))
        .collect()
}

fn slug(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut previous_dash = false;
    for ch in value.chars() {
        let normalized = if ch.is_ascii_alphanumeric() {
            previous_dash = false;
            ch.to_ascii_lowercase()
        } else if previous_dash {
            continue;
        } else {
            previous_dash = true;
            '-'
        };
        out.push(normalized);
    }
    out.trim_matches('-').to_string()
}

fn print_usage() {
    eprintln!("Usage:");
    eprintln!("  hopper fuzz generate --program <manifest> [--out <plan.json>] [--corpus <dir>]");
    eprintln!("  hopper fuzz check --program <manifest> [--plan <plan.json>]");
    eprintln!("  hopper fuzz run --program <manifest> --adapter <executable> [--plan <plan.json>]");
    eprintln!(
        "                  [--adapter-arg <value>] [--case <id>] [--require-invariant <name>]"
    );
    eprintln!("                  [--report <report.json>] [--allow-skips]");
    eprintln!();
    eprintln!(
        "The plan deterministically covers layout identity and truncation, field boundaries,"
    );
    eprintln!("instruction arguments, Accounts-derived constraints, policies, privileges and");
    eprintln!("aliases, write-range escapes, parametric selectors, declared lamport permissions,");
    eprintln!("remaining accounts, and declared compatibility contracts.");
    eprintln!(
        "`run` sends one JSON request to the adapter's stdin and expects one JSON response on stdout."
    );
}

fn fail(message: &str) -> ! {
    eprintln!("hopper fuzz: {message}");
    process::exit(1);
}

#[cfg(test)]
mod tests {
    use super::*;
    use hopper_schema::accounts::{AccountLifecycle, ContextAccountDescriptor, ContextDescriptor};
    use hopper_schema::{
        AccountEntry, ArgDescriptor, CompatibilityPair, FieldDescriptor, FieldIntent,
        InstructionDescriptor, LayoutManifest, ParametricWriteRange, PolicyDescriptor,
        RemainingAccountsDescriptor, WriteRange,
    };

    static FIELDS_V1: &[FieldDescriptor] = &[FieldDescriptor {
        name: "value",
        canonical_type: "u64",
        size: 8,
        offset: 16,
        intent: FieldIntent::Counter,
    }];
    static FIELDS_V2: &[FieldDescriptor] = &[
        FieldDescriptor {
            name: "value",
            canonical_type: "u64",
            size: 8,
            offset: 16,
            intent: FieldIntent::Counter,
        },
        FieldDescriptor {
            name: "authority",
            canonical_type: "Address",
            size: 32,
            offset: 24,
            intent: FieldIntent::Authority,
        },
    ];
    static LAYOUTS: &[LayoutManifest] = &[
        LayoutManifest {
            name: "CounterV1",
            disc: 7,
            version: 1,
            layout_id: [1; 8],
            total_size: 24,
            has_dynamic_tail: false,
            field_count: 1,
            fields: FIELDS_V1,
        },
        LayoutManifest {
            name: "CounterV2",
            disc: 7,
            version: 2,
            layout_id: [2; 8],
            total_size: 56,
            has_dynamic_tail: false,
            field_count: 2,
            fields: FIELDS_V2,
        },
    ];
    static ACCOUNTS: &[AccountEntry] = &[
        AccountEntry {
            name: "authority",
            writable: false,
            signer: true,
            layout_ref: "",
            seeds: &[],
        },
        AccountEntry {
            name: "counter",
            writable: true,
            signer: false,
            layout_ref: "CounterV2",
            seeds: &["counter", "authority"],
        },
    ];
    static ARGS: &[ArgDescriptor] = &[ArgDescriptor {
        name: "cell",
        canonical_type: "u32",
        size: 4,
        encoding: ArgEncoding::Fixed,
    }];
    static WRITES: &[WriteRange] = &[WriteRange {
        account_index: 1,
        offset: 16,
        size: 8,
    }];
    static PARAMETRIC: &[ParametricWriteRange] = &[ParametricWriteRange::new(
        1, 24, 8, 8, 4, 0, "cell", "cells",
    )];
    static INSTRUCTIONS: &[InstructionDescriptor] = &[InstructionDescriptor {
        name: "increment",
        tag: 1,
        discriminator: &[1],
        args: ARGS,
        accounts: ACCOUNTS,
        remaining_accounts: Some(RemainingAccountsDescriptor { max: 3 }),
        capabilities: &[],
        policy_pack: "",
        receipt_expected: false,
        strict_writes: true,
        write_ranges: WRITES,
        parametric_write_ranges: PARAMETRIC,
        mutation_complete: true,
        lamport_accounts: &[],
        cu_estimate: 2_000,
    }];
    static CONTEXT_ACCOUNTS: &[ContextAccountDescriptor] = &[
        ContextAccountDescriptor {
            name: "authority",
            kind: "Signer",
            writable: false,
            signer: true,
            layout_ref: "",
            policy_ref: "",
            seeds: &[],
            optional: false,
            lifecycle: AccountLifecycle::Existing,
            payer: "",
            init_space: 0,
            has_one: &[],
            expected_address: "",
            expected_owner: "",
        },
        ContextAccountDescriptor {
            name: "counter",
            kind: "InitAccount",
            writable: true,
            signer: false,
            layout_ref: "CounterV2",
            policy_ref: "CounterPolicy",
            seeds: &["counter", "authority.address()"],
            optional: true,
            lifecycle: AccountLifecycle::Init,
            payer: "authority",
            init_space: 56,
            has_one: &["authority"],
            expected_address: "counter_address",
            expected_owner: "counter_owner",
        },
    ];
    static CONTEXTS: &[ContextDescriptor] = &[ContextDescriptor {
        name: "Increment",
        accounts: CONTEXT_ACCOUNTS,
        policies: &["CounterPolicy"],
        receipts_expected: true,
        mutation_classes: &["InPlace"],
        strict_writes: true,
        write_ranges: WRITES,
        parametric_write_ranges: PARAMETRIC,
        mutation_complete: true,
        lamport_accounts: &[],
    }];
    static POLICIES: &[PolicyDescriptor] = &[PolicyDescriptor {
        name: "CounterPolicy",
        capabilities: &["MutatesState"],
        requirements: &["SignerAuthority"],
        invariants: &["CounterMonotonic"],
        receipt_profile: "CounterReceipt",
    }];
    static COMPATIBILITY: &[CompatibilityPair] = &[CompatibilityPair {
        from_layout: "CounterV1",
        from_version: 1,
        to_layout: "CounterV2",
        to_version: 2,
        policy: MigrationPolicy::RequiresMigration,
        backward_readable: false,
    }];
    const LOSSLESS_MANIFEST_JSON: &str = r#"{
      "name": "lossless-fixture",
      "version": "1.0.0",
      "description": "full C3 contract",
      "layouts": [
        {
          "name": "StateV1", "disc": 9, "version": 1,
          "layoutId": "0101010101010101", "totalSize": 24, "fieldCount": 1,
          "fields": [{"name":"value","type":"u64","size":8,"offset":16,"intent":"counter"}]
        },
        {
          "name": "StateV2", "disc": 9, "version": 2,
          "layoutId": "0202020202020202", "totalSize": 32, "fieldCount": 1,
          "fields": [{"name":"value","type":"u64","size":8,"offset":16,"intent":"counter"}]
        }
      ],
      "instructions": [{
        "name": "initialize", "tag": 1, "discriminatorBytes": [1],
        "args": [{"name":"slot","type":"u32","size":4,"encoding":"fixed"}],
        "accounts": [
          {"name":"authority","writable":false,"signer":true},
          {"name":"state","writable":true,"signer":false,"layoutRef":"StateV2"}
        ],
        "capabilities": ["MutatesState"], "policyPack": "", "receiptExpected": true,
        "strictWrites": false, "writeRanges": [], "parametricWriteRanges": []
      }],
      "events": [],
      "policies": [{
        "name":"StatePolicy", "capabilities":["MutatesState"],
        "requirements":["SignerAuthority"], "invariants":["StateValid"],
        "receiptProfile":"StateReceipt"
      }],
      "layoutMetadata": [{
        "name":"StateV2", "segmentRoles":["core"], "appendSafe":true,
        "migrationRequired":false, "rebuildable":false, "policyPack":"StatePolicy",
        "invariantPack":["StateValid"], "receiptProfile":"StateReceipt",
        "phaseRequirements":["Update"], "trustProfile":"verified",
        "managerHints":["show-value"]
      }],
      "contexts": [{
        "name":"Initialize",
        "accounts":[
          {"name":"authority","kind":"Signer","writable":false,"signer":true,
           "seeds":[],"optional":false,"lifecycle":"existing","hasOne":[]},
          {"name":"state","kind":"InitAccount","writable":true,"signer":false,
           "layoutRef":"StateV2","policyRef":"StatePolicy","seeds":["seed-v1","authority.address()"],
           "optional":false,"lifecycle":"init","payer":"authority","initSpace":32,
           "hasOne":["authority"],"expectedAddress":"state-address-v1",
           "expectedOwner":"state-owner-v1"}
        ],
        "policies":["StatePolicy"], "receiptsExpected":true,
        "mutationClasses":["Initialization"], "strictWrites":true,
        "mutationComplete":true, "lamportAccounts":[1],
        "writeRanges":[{"account":"state","accountIndex":1,"offset":16,"size":8}],
        "parametricWriteRanges":[{
          "accountIndex":1,"baseOffset":16,"stride":8,"cellSize":8,"count":2,
          "argumentIndex":0,"argument":"slot","segment":"cells"
        }]
      }],
      "compatRules": [{
        "from":"StateV1","fromVersion":1,"to":"StateV2","toVersion":2,
        "policy":"requires-migration","backwardReadable":false
      }],
      "toolingHints": ["lossless"]
    }"#;

    fn manifest() -> ProgramManifest {
        ProgramManifest {
            name: "fuzz-fixture",
            version: "0.3.0",
            description: "fixture",
            layouts: LAYOUTS,
            layout_metadata: &[],
            instructions: INSTRUCTIONS,
            events: &[],
            policies: POLICIES,
            compatibility_pairs: COMPATIBILITY,
            tooling_hints: &[],
            contexts: CONTEXTS,
        }
    }

    #[test]
    fn plan_covers_every_contract_dimension() {
        let plan = build_plan(&manifest());
        assert_eq!(plan.schema, PLAN_SCHEMA);
        assert_eq!(plan.coverage.layouts, 2);
        assert_eq!(plan.coverage.fields, 3);
        assert_eq!(plan.coverage.instructions, 1);
        assert_eq!(plan.coverage.contexts, 1);
        assert_eq!(plan.coverage.matched_instruction_contexts, 1);
        assert_eq!(plan.coverage.alias_pairs, 1);
        assert_eq!(plan.coverage.write_ranges, 1);
        assert_eq!(plan.coverage.parametric_write_ranges, 1);
        assert_eq!(plan.coverage.migration_pairs, 1);
        for kind in [
            "layout-truncation",
            "layout-identity",
            "field-boundary",
            "migration-direction",
            "migration-backward-read",
            "instruction-truncation",
            "argument-boundary",
            "account-privilege",
            "typed-account-substitution",
            "pda-substitution",
            "account-lifecycle",
            "has-one-substitution",
            "owner-substitution",
            "address-substitution",
            "optional-account-boundary",
            "policy-requirement",
            "policy-invariant",
            "policy-attachment",
            "receipt-contract",
            "duplicate-account-alias",
            "write-range-boundary",
            "write-range-escape",
            "parametric-write-selector",
            "lamport-permission",
            "remaining-account-boundary",
        ] {
            assert!(
                plan.cases.iter().any(|case| case.kind == kind),
                "missing generated case kind {kind}"
            );
        }
    }

    #[test]
    fn write_escape_probes_are_outside_the_union_and_unique() {
        static RANGES: &[WriteRange] = &[
            // Adjacent, overlapping, nested, and duplicate ranges on account
            // one form one continuous [10, 40) authorized union.
            WriteRange::new(1, 10, 10),
            WriteRange::new(1, 20, 10),
            WriteRange::new(1, 25, 15),
            WriteRange::new(1, 12, 4),
            WriteRange::new(1, 10, 10),
            // Account two deliberately has a gap, so both sides of that gap
            // remain useful hostile probes.
            WriteRange::new(2, 5, 3),
            WriteRange::new(2, 10, 3),
        ];

        let mut emitted = BTreeSet::new();
        let mut probes = Vec::new();
        for range in RANGES {
            if range.offset > 0
                && register_union_write_escape(
                    RANGES,
                    &mut emitted,
                    range.account_index,
                    range.offset - 1,
                    1,
                )
            {
                probes.push((range.account_index, range.offset - 1, 1));
            }
            if range.size != u32::MAX
                && register_union_write_escape(
                    RANGES,
                    &mut emitted,
                    range.account_index,
                    range.offset + range.size,
                    1,
                )
            {
                probes.push((range.account_index, range.offset + range.size, 1));
            }
        }

        assert_eq!(
            probes,
            vec![
                (1, 9, 1),
                (1, 40, 1),
                (2, 4, 1),
                (2, 8, 1),
                (2, 9, 1),
                (2, 13, 1)
            ]
        );
        assert_eq!(emitted.len(), probes.len());
        assert!(probes.iter().all(|(account_index, offset, size)| {
            !RANGES.iter().any(|range| {
                range.account_index == *account_index && range.contains(*offset, *size)
            })
        }));
    }

    #[test]
    fn migrations_come_only_from_declared_compatibility_pairs() {
        let mut undeclared = manifest();
        undeclared.compatibility_pairs = &[];
        let undeclared_plan = build_plan(&undeclared);
        assert_eq!(undeclared_plan.coverage.migration_pairs, 0);
        assert!(!undeclared_plan.cases.iter().any(|case| matches!(
            case.kind.as_str(),
            "migration-direction" | "migration-backward-read"
        )));

        let declared_plan = build_plan(&manifest());
        assert_eq!(declared_plan.coverage.migration_pairs, 1);
        assert_eq!(
            declared_plan
                .cases
                .iter()
                .filter(|case| matches!(
                    case.kind.as_str(),
                    "migration-direction" | "migration-backward-read"
                ))
                .count(),
            2
        );
    }

    #[test]
    fn migration_policy_and_backward_readability_bind_plan_and_seeds() {
        let original = build_plan(&manifest());
        let changed_pair = CompatibilityPair {
            policy: MigrationPolicy::AppendOnly,
            backward_readable: true,
            ..COMPATIBILITY[0]
        };
        let changed_pairs = Box::leak(vec![changed_pair].into_boxed_slice());
        let mut changed_manifest = manifest();
        changed_manifest.compatibility_pairs = changed_pairs;
        let changed = build_plan(&changed_manifest);

        assert_eq!(original.coverage, changed.coverage);
        assert_ne!(original.contract_commitment, changed.contract_commitment);
        assert_ne!(original.cases[0].seed, changed.cases[0].seed);
        let changed_forward = changed
            .cases
            .iter()
            .find(|case| case.id == "compatibility-0-forward")
            .expect("declared forward compatibility case");
        assert_eq!(changed_forward.parameters["policy"], 1);
        assert!(changed_forward.expectation.contains("appended-region"));
        let backward = changed
            .cases
            .iter()
            .find(|case| case.id == "compatibility-0-backward-read")
            .expect("declared backward-read case");
        assert_eq!(backward.parameters["backwardReadable"], 1);
        assert!(backward.expectation.contains("must-accept"));
    }

    fn plan_from_json(json: &str) -> (ProgramManifest, ManifestFuzzPlan) {
        let owned = crate::parse_program_manifest_json(json).expect("manifest JSON parses");
        let manifest = crate::to_program_manifest(&owned);
        let plan = build_plan(&manifest);
        (manifest, plan)
    }

    #[test]
    fn json_loaded_context_contract_generates_constraint_and_effect_cases() {
        let (manifest, plan) = plan_from_json(LOSSLESS_MANIFEST_JSON);

        assert_eq!(manifest.layout_metadata.len(), 1);
        assert_eq!(manifest.compatibility_pairs.len(), 1);
        assert_eq!(manifest.tooling_hints, &["lossless"]);
        assert_eq!(manifest.contexts.len(), 1);
        assert_eq!(manifest.contexts[0].parametric_write_ranges.len(), 1);
        assert!(manifest.contexts[0].mutation_complete);
        assert_eq!(manifest.contexts[0].lamport_accounts, &[1]);
        assert_eq!(manifest.instructions[0].accounts[1].layout_ref, "StateV2");
        assert_eq!(
            manifest.instructions[0].accounts[1].seeds,
            &["seed-v1", "authority.address()"]
        );

        assert_eq!(plan.coverage.matched_instruction_contexts, 1);
        assert_eq!(plan.coverage.typed_account_constraints, 1);
        assert_eq!(plan.coverage.pda_constraints, 1);
        assert_eq!(plan.coverage.lifecycle_constraints, 1);
        assert_eq!(plan.coverage.has_one_constraints, 1);
        assert_eq!(plan.coverage.owner_constraints, 1);
        assert_eq!(plan.coverage.address_constraints, 1);
        assert_eq!(plan.coverage.parametric_write_ranges, 1);
        assert_eq!(plan.coverage.lamport_permissions, 1);
        assert_eq!(plan.coverage.migration_pairs, 1);
        for kind in [
            "typed-account-substitution",
            "pda-substitution",
            "account-lifecycle",
            "has-one-substitution",
            "owner-substitution",
            "address-substitution",
            "parametric-write-selector",
            "lamport-permission",
            "migration-direction",
            "migration-backward-read",
        ] {
            assert!(
                plan.cases.iter().any(|case| case.kind == kind),
                "missing JSON-derived case kind {kind}"
            );
        }
    }

    #[test]
    fn json_loaded_pda_typed_lifecycle_and_migration_drift_change_seeds() {
        let (_, original) = plan_from_json(LOSSLESS_MANIFEST_JSON);
        let variants = [
            LOSSLESS_MANIFEST_JSON.replace("seed-v1", "seed-v2"),
            LOSSLESS_MANIFEST_JSON
                .replace("\"layoutRef\":\"StateV2\"", "\"layoutRef\":\"StateV1\""),
            LOSSLESS_MANIFEST_JSON.replace("\"lifecycle\":\"init\"", "\"lifecycle\":\"close\""),
            LOSSLESS_MANIFEST_JSON.replace(
                "\"policy\":\"requires-migration\"",
                "\"policy\":\"append-only\"",
            ),
        ];

        for changed_json in variants {
            let (_, changed) = plan_from_json(&changed_json);
            assert_ne!(original.contract_commitment, changed.contract_commitment);
            assert_ne!(original.cases[0].seed, changed.cases[0].seed);
        }
    }

    #[test]
    fn plan_is_deterministic_and_ids_are_unique() {
        let first = build_plan(&manifest());
        let second = build_plan(&manifest());
        assert_eq!(first, second);
        assert_eq!(first.contract_commitment.len(), 64);
        let ids: BTreeSet<_> = first.cases.iter().map(|case| &case.id).collect();
        assert_eq!(ids.len(), first.cases.len());
        assert_eq!(first.coverage.generated_cases, first.cases.len());

        let seeds: BTreeSet<_> = first.cases.iter().map(|case| &case.seed).collect();
        assert_eq!(seeds.len(), first.cases.len());
        assert!(first
            .cases
            .iter()
            .all(|case| case.seed.len() == 32
                && case.seed.bytes().all(|byte| byte.is_ascii_hexdigit())));
    }

    #[test]
    fn commitment_and_seeds_bind_values_not_only_case_counts() {
        let original = build_plan(&manifest());
        let mut changed_layout = LAYOUTS[0];
        changed_layout.layout_id = [9; 8];
        let layouts = Box::leak(vec![changed_layout, LAYOUTS[1]].into_boxed_slice());
        let mut changed_manifest = manifest();
        changed_manifest.layouts = layouts;
        let changed = build_plan(&changed_manifest);

        assert_eq!(original.coverage, changed.coverage);
        assert_eq!(
            original
                .cases
                .iter()
                .map(|case| &case.id)
                .collect::<Vec<_>>(),
            changed
                .cases
                .iter()
                .map(|case| &case.id)
                .collect::<Vec<_>>()
        );
        assert_ne!(original.contract_commitment, changed.contract_commitment);
        assert_ne!(original.cases[0].seed, changed.cases[0].seed);
    }

    #[test]
    fn hostile_cases_carry_generated_invariant_hooks() {
        let plan = build_plan(&manifest());
        let truncation = plan
            .cases
            .iter()
            .find(|case| case.kind == "instruction-truncation")
            .expect("instruction truncation case");
        assert!(truncation
            .required_invariants
            .contains(&"no-panic".to_string()));
        assert!(truncation
            .required_invariants
            .contains(&"atomic-rejection".to_string()));
        assert!(truncation
            .required_invariants
            .contains(&"instruction-parser".to_string()));

        let escaped_write = plan
            .cases
            .iter()
            .find(|case| case.kind == "write-range-escape")
            .expect("write escape case");
        assert!(escaped_write
            .required_invariants
            .contains(&"write-confinement".to_string()));
        assert!(escaped_write
            .required_invariants
            .contains(&"atomic-rejection".to_string()));
    }

    fn passing_response(request: &HarnessRequest) -> HarnessResponse {
        HarnessResponse {
            schema: HARNESS_RESPONSE_SCHEMA.to_string(),
            contract_commitment: request.contract_commitment.clone(),
            results: request
                .cases
                .iter()
                .map(|case| HarnessResult {
                    id: case.id.clone(),
                    outcome: HarnessOutcome::Passed,
                    checked_invariants: case.required_invariants.clone(),
                    detail: String::new(),
                })
                .collect(),
        }
    }

    #[test]
    fn runner_selects_exact_cases_and_adds_custom_invariants() {
        let plan = build_plan(&manifest());
        let selected = plan.cases[3].id.clone();
        let request = build_request(
            &plan,
            std::slice::from_ref(&selected),
            &["balance-conservation".to_string()],
        )
        .unwrap();
        assert_eq!(request.schema, HARNESS_REQUEST_SCHEMA);
        assert_eq!(request.cases.len(), 1);
        assert_eq!(request.cases[0].id, selected);
        assert!(request.cases[0]
            .required_invariants
            .contains(&"balance-conservation".to_string()));

        let error = build_request(&plan, &["not-a-case".to_string()], &[]).unwrap_err();
        assert!(error.contains("unknown generated case"));
    }

    #[test]
    fn runner_accepts_only_complete_invariant_checked_results() {
        let plan = build_plan(&manifest());
        let request = build_request(
            &plan,
            std::slice::from_ref(&plan.cases[0].id),
            &["business-state-unchanged".to_string()],
        )
        .unwrap();
        let validation = validate_response(&request, passing_response(&request), false).unwrap();
        assert_eq!(validation.summary.passed, 1);
        assert_eq!(validation.summary.failed, 0);
        assert!(validation.failures.is_empty());

        let mut incomplete = passing_response(&request);
        incomplete.results[0]
            .checked_invariants
            .retain(|name| name != "business-state-unchanged");
        let validation = validate_response(&request, incomplete, false).unwrap();
        assert_eq!(validation.summary.failed, 1);
        assert_eq!(validation.results[0].outcome, HarnessOutcome::Failed);
        assert!(validation.results[0]
            .detail
            .contains("business-state-unchanged"));
    }

    #[test]
    fn runner_fails_closed_on_missing_duplicate_or_skipped_results() {
        let plan = build_plan(&manifest());
        let request = build_request(
            &plan,
            &[plan.cases[0].id.clone(), plan.cases[1].id.clone()],
            &[],
        )
        .unwrap();

        let mut missing = passing_response(&request);
        missing.results.pop();
        assert!(validate_response(&request, missing, false)
            .unwrap_err()
            .contains("omitted generated case"));

        let mut duplicate = passing_response(&request);
        duplicate.results.push(duplicate.results[0].clone());
        assert!(validate_response(&request, duplicate, false)
            .unwrap_err()
            .contains("duplicate result"));

        let mut skipped = passing_response(&request);
        skipped.results[0].outcome = HarnessOutcome::Skipped;
        let strict = validate_response(&request, skipped.clone(), false).unwrap();
        assert_eq!(strict.summary.failed, 1);
        assert_eq!(strict.summary.passed, 1);
        let permissive = validate_response(&request, skipped, true).unwrap();
        assert_eq!(permissive.summary.skipped, 1);
        assert_eq!(permissive.summary.passed, 1);
    }

    #[test]
    fn adapter_protocol_is_strict_and_round_trips() {
        let plan = build_plan(&manifest());
        let request = build_request(&plan, std::slice::from_ref(&plan.cases[0].id), &[]).unwrap();
        let request_json = serde_json::to_string(&request).unwrap();
        assert!(request_json.contains(HARNESS_REQUEST_SCHEMA));
        assert_eq!(
            serde_json::from_str::<HarnessRequest>(&request_json).unwrap(),
            request
        );

        let response = passing_response(&request);
        let mut response_value = serde_json::to_value(&response).unwrap();
        response_value["unexpected"] = serde_json::json!(true);
        assert!(serde_json::from_value::<HarnessResponse>(response_value).is_err());
    }
}
