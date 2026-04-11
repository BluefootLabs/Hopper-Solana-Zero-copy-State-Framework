use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde_json::Value as JsonValue;
use solana_client::rpc_client::RpcClient;
use solana_client::rpc_config::RpcSimulateTransactionConfig;
use solana_sdk::commitment_config::CommitmentConfig;
use solana_sdk::instruction::{AccountMeta, Instruction};
use solana_sdk::native_token::LAMPORTS_PER_SOL;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::{read_keypair_file, Keypair, Signer};
use solana_sdk::transaction::Transaction;
use solana_system_interface::instruction as system_instruction;
use toml::Value as TomlValue;

use crate::workspace;

const BENCH_ACCOUNT_LEN: usize = 57;
const WRITE_HEADER_DISC: u8 = 6;
const PROC_MACRO_TYPED_DISPATCH_PAYLOAD: [u8; 8] = 7u64.to_le_bytes();

#[derive(Clone, Copy)]
enum FixtureMode {
    None,
    BlankAccount,
    InitializedAccount,
    DuplicateBlankAccount,
}

impl FixtureMode {
    fn needs_fixture(self) -> bool {
        !matches!(self, Self::None)
    }

    fn needs_header_init(self) -> bool {
        matches!(self, Self::InitializedAccount)
    }

    fn duplicate_accounts(self) -> bool {
        matches!(self, Self::DuplicateBlankAccount)
    }

    fn label(self) -> &'static str {
        match self {
            Self::None => "no-accounts",
            Self::BlankAccount => "fresh-program-owned-account",
            Self::InitializedAccount => "header-initialized-program-owned-account",
            Self::DuplicateBlankAccount => "duplicate-program-owned-account",
        }
    }
}

#[derive(Clone, Copy)]
struct BenchmarkCase {
    disc: u8,
    name: &'static str,
    baseline_key: &'static str,
    fixture: FixtureMode,
    payload: &'static [u8],
}

const BENCHMARK_CASES: &[BenchmarkCase] = &[
    BenchmarkCase { disc: 0, name: "check_signer", baseline_key: "check_signer", fixture: FixtureMode::BlankAccount, payload: &[] },
    BenchmarkCase { disc: 1, name: "check_writable", baseline_key: "check_writable", fixture: FixtureMode::BlankAccount, payload: &[] },
    BenchmarkCase { disc: 2, name: "check_owner", baseline_key: "check_owner", fixture: FixtureMode::BlankAccount, payload: &[] },
    BenchmarkCase { disc: 3, name: "check_account_tier1", baseline_key: "check_account_tier1", fixture: FixtureMode::InitializedAccount, payload: &[] },
    BenchmarkCase { disc: 4, name: "check_keys_eq", baseline_key: "check_keys_eq", fixture: FixtureMode::DuplicateBlankAccount, payload: &[] },
    BenchmarkCase { disc: 5, name: "overlay", baseline_key: "overlay_57b", fixture: FixtureMode::InitializedAccount, payload: &[] },
    BenchmarkCase { disc: 6, name: "write_header", baseline_key: "write_header", fixture: FixtureMode::BlankAccount, payload: &[] },
    BenchmarkCase { disc: 7, name: "zero_init", baseline_key: "zero_init_57b", fixture: FixtureMode::BlankAccount, payload: &[] },
    BenchmarkCase { disc: 8, name: "check_account_fast", baseline_key: "check_account_fast", fixture: FixtureMode::BlankAccount, payload: &[] },
    BenchmarkCase { disc: 9, name: "emit_event", baseline_key: "emit_event_32b", fixture: FixtureMode::None, payload: &[] },
    BenchmarkCase { disc: 10, name: "trust_strict_load", baseline_key: "trust_strict_load", fixture: FixtureMode::InitializedAccount, payload: &[] },
    BenchmarkCase { disc: 11, name: "pod_from_bytes", baseline_key: "pod_from_bytes_57b", fixture: FixtureMode::InitializedAccount, payload: &[] },
    BenchmarkCase { disc: 12, name: "receipt_begin_commit", baseline_key: "receipt_begin_commit", fixture: FixtureMode::InitializedAccount, payload: &[] },
    BenchmarkCase { disc: 13, name: "fingerprint_check", baseline_key: "fingerprint_check", fixture: FixtureMode::InitializedAccount, payload: &[] },
    BenchmarkCase { disc: 14, name: "state_diff", baseline_key: "state_diff", fixture: FixtureMode::InitializedAccount, payload: &[] },
    BenchmarkCase { disc: 15, name: "overlay_mut_field_set", baseline_key: "overlay_mut_57b", fixture: FixtureMode::InitializedAccount, payload: &[] },
    BenchmarkCase { disc: 16, name: "raw_cast_baseline", baseline_key: "raw_cast_baseline", fixture: FixtureMode::InitializedAccount, payload: &[] },
    BenchmarkCase { disc: 17, name: "receipt_full", baseline_key: "receipt_full_enriched", fixture: FixtureMode::InitializedAccount, payload: &[] },
    BenchmarkCase { disc: 18, name: "receipt_emit", baseline_key: "receipt_emit", fixture: FixtureMode::InitializedAccount, payload: &[] },
    BenchmarkCase { disc: 19, name: "proc_macro_typed_dispatch", baseline_key: "proc_macro_typed_dispatch", fixture: FixtureMode::InitializedAccount, payload: &PROC_MACRO_TYPED_DISPATCH_PAYLOAD },
];

#[derive(Default)]
struct PrimitiveBenchOptions {
    rpc_url: Option<String>,
    keypair_path: Option<PathBuf>,
    out_dir: Option<PathBuf>,
    program_id: Option<String>,
    no_build: bool,
    no_deploy: bool,
    fail_on_regression_percent: Option<u64>,
}

#[derive(Clone)]
struct BaselineEntry {
    budget_cu: u64,
    category: String,
}

struct BaselineSet {
    tolerance_percent: u64,
    entries: BTreeMap<String, BaselineEntry>,
}

#[derive(Serialize)]
struct BenchMetadata {
    generated_at_unix_seconds: u64,
    workspace_root: String,
    rpc_url: String,
    program_id: String,
    solana_core_version: String,
    rustc_version: String,
    git_commit: String,
    keypair_path: String,
    baseline_tolerance_percent: u64,
    benchmark_count: usize,
}

#[derive(Serialize)]
struct BenchResult {
    disc: u8,
    name: String,
    baseline_key: String,
    baseline_category: Option<String>,
    fixture: String,
    measured_cu: Option<u64>,
    total_units_consumed: Option<u64>,
    baseline_cu: Option<u64>,
    allowed_cu: Option<u64>,
    delta_pct: Option<f64>,
    status: String,
    regression: bool,
    logs: Vec<String>,
    error: Option<String>,
}

#[derive(Serialize)]
struct BenchReport {
    metadata: BenchMetadata,
    results: Vec<BenchResult>,
}

pub fn run_primitive_bench(args: &[String]) -> Result<(), String> {
    let cwd = workspace::current_dir()?;
    let workspace_root = workspace::find_workspace_root(&cwd)?;
    let options = parse_options(args, &workspace_root)?;
    let baselines = load_baselines(&workspace_root.join("bench").join("cu_baselines.toml"))?;
    let fail_on_regression_percent = options
        .fail_on_regression_percent
        .unwrap_or(baselines.tolerance_percent);

    if !options.no_build {
        println!("Building hopper-bench with cargo build-sbf...");
        let build_args = vec!["build-sbf".to_string(), "-p".to_string(), "hopper-bench".to_string()];
        let status = workspace::run_status("cargo", &build_args, &workspace_root)?;
        if !status.success() {
            return Err("cargo build-sbf -p hopper-bench failed".to_string());
        }
    }

    let rpc_url = options
        .rpc_url
        .clone()
        .or_else(|| std::env::var("SOLANA_RPC_URL").ok())
        .unwrap_or_else(|| "http://127.0.0.1:8899".to_string());

    let keypair_path = options
        .keypair_path
        .clone()
        .or_else(workspace::default_solana_keypair_path)
        .ok_or_else(|| "Could not determine a Solana keypair path. Use --keypair or SOLANA_KEYPAIR.".to_string())?;

    let client = RpcClient::new_with_commitment(rpc_url.clone(), CommitmentConfig::confirmed());
    let payer = read_keypair_file(&keypair_path).map_err(|err| {
        format!(
            "Failed to read keypair {}: {err}",
            keypair_path.display()
        )
    })?;

    ensure_payer_balance(&client, &payer, &rpc_url)?;

    let program_id = if let Some(existing) = options.program_id.as_deref() {
        existing.parse::<Pubkey>().map_err(|err| format!("Invalid --program-id: {err}"))?
    } else {
        if options.no_deploy {
            return Err("--no-deploy requires --program-id".to_string());
        }
        deploy_bench_program(&workspace_root, &rpc_url, &keypair_path)?
    };

    let mut results = Vec::with_capacity(BENCHMARK_CASES.len());
    let mut failures = 0usize;

    println!("Running {} primitive benchmarks against {}", BENCHMARK_CASES.len(), program_id);
    for case in BENCHMARK_CASES {
        let baseline = baselines.entries.get(case.baseline_key);
        let allowed_cu = baseline.map(|entry| allowed_budget(entry.budget_cu, fail_on_regression_percent));

        let benchmark_result = match run_case(&client, &payer, &program_id, case) {
            Ok((measured_cu, total_units_consumed, logs)) => {
                let baseline_cu = baseline.map(|entry| entry.budget_cu);
                let delta_pct = baseline_cu.map(|budget| percentage_delta(measured_cu, budget));
                let regression = allowed_cu.map(|allowed| measured_cu > allowed).unwrap_or(false);
                if regression {
                    failures += 1;
                }
                println!(
                    "  {:>2} {:<24} {:>4} CU{}",
                    case.disc,
                    case.name,
                    measured_cu,
                    baseline_cu
                        .map(|budget| format!(" (baseline {}, allowed {})", budget, allowed_cu.unwrap_or(budget)))
                        .unwrap_or_default()
                );
                BenchResult {
                    disc: case.disc,
                    name: case.name.to_string(),
                    baseline_key: case.baseline_key.to_string(),
                    baseline_category: baseline.map(|entry| entry.category.clone()),
                    fixture: case.fixture.label().to_string(),
                    measured_cu: Some(measured_cu),
                    total_units_consumed,
                    baseline_cu,
                    allowed_cu,
                    delta_pct,
                    status: if regression { "regression" } else { "ok" }.to_string(),
                    regression,
                    logs,
                    error: None,
                }
            }
            Err(err) => {
                failures += 1;
                println!("  {:>2} {:<24} ERROR", case.disc, case.name);
                BenchResult {
                    disc: case.disc,
                    name: case.name.to_string(),
                    baseline_key: case.baseline_key.to_string(),
                    baseline_category: baseline.map(|entry| entry.category.clone()),
                    fixture: case.fixture.label().to_string(),
                    measured_cu: None,
                    total_units_consumed: None,
                    baseline_cu: baseline.map(|entry| entry.budget_cu),
                    allowed_cu,
                    delta_pct: None,
                    status: "error".to_string(),
                    regression: true,
                    logs: Vec::new(),
                    error: Some(err),
                }
            }
        };
        results.push(benchmark_result);
    }

    let version = client
        .get_version()
        .map(|value| value.solana_core)
        .unwrap_or_else(|_| "unknown".to_string());
    let metadata = BenchMetadata {
        generated_at_unix_seconds: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        workspace_root: workspace_root.display().to_string(),
        rpc_url,
        program_id: program_id.to_string(),
        solana_core_version: version,
        rustc_version: read_tool_version(&workspace_root, "rustc", &["--version".to_string()]),
        git_commit: read_tool_version(&workspace_root, "git", &["rev-parse".to_string(), "HEAD".to_string()]),
        keypair_path: keypair_path.display().to_string(),
        baseline_tolerance_percent: fail_on_regression_percent,
        benchmark_count: BENCHMARK_CASES.len(),
    };

    let report = BenchReport { metadata, results };
    let out_dir = options
        .out_dir
        .clone()
        .unwrap_or_else(|| workspace_root.join("bench").join("results"));
    fs::create_dir_all(&out_dir)
        .map_err(|err| format!("Failed to create output directory {}: {err}", out_dir.display()))?;

    let json_path = out_dir.join("primitive-bench-results.json");
    let csv_path = out_dir.join("primitive-bench-results.csv");
    write_json_report(&json_path, &report)?;
    write_csv_report(&csv_path, &report)?;

    println!("Wrote {}", json_path.display());
    println!("Wrote {}", csv_path.display());

    if failures > 0 {
        return Err(format!(
            "{} benchmark case(s) failed or exceeded the allowed regression budget",
            failures
        ));
    }

    Ok(())
}

fn parse_options(args: &[String], workspace_root: &Path) -> Result<PrimitiveBenchOptions, String> {
    let mut options = PrimitiveBenchOptions::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--rpc" => {
                let value = args.get(i + 1).ok_or_else(|| "--rpc requires a URL".to_string())?;
                options.rpc_url = Some(value.clone());
                i += 2;
            }
            "--keypair" => {
                let value = args.get(i + 1).ok_or_else(|| "--keypair requires a path".to_string())?;
                options.keypair_path = Some(PathBuf::from(value));
                i += 2;
            }
            "--out-dir" => {
                let value = args.get(i + 1).ok_or_else(|| "--out-dir requires a path".to_string())?;
                options.out_dir = Some(resolve_path(workspace_root, value));
                i += 2;
            }
            "--program-id" => {
                let value = args.get(i + 1).ok_or_else(|| "--program-id requires a pubkey".to_string())?;
                options.program_id = Some(value.clone());
                i += 2;
            }
            "--no-build" => {
                options.no_build = true;
                i += 1;
            }
            "--no-deploy" => {
                options.no_deploy = true;
                i += 1;
            }
            "--fail-on-regression" => {
                let value = args.get(i + 1).ok_or_else(|| "--fail-on-regression requires a percentage".to_string())?;
                options.fail_on_regression_percent = Some(parse_u64_flag("--fail-on-regression", value)?);
                i += 2;
            }
            other if other.starts_with("--fail-on-regression=") => {
                let value = other.split_once('=').map(|(_, value)| value).unwrap_or_default();
                options.fail_on_regression_percent = Some(parse_u64_flag("--fail-on-regression", value)?);
                i += 1;
            }
            other => {
                return Err(format!("Unknown profile bench argument: {other}"));
            }
        }
    }

    Ok(options)
}

fn parse_u64_flag(flag: &str, value: &str) -> Result<u64, String> {
    value
        .parse::<u64>()
        .map_err(|err| format!("{flag} requires an integer percentage: {err}"))
}

fn resolve_path(workspace_root: &Path, value: &str) -> PathBuf {
    let candidate = PathBuf::from(value);
    if candidate.is_absolute() {
        candidate
    } else {
        workspace_root.join(candidate)
    }
}

fn load_baselines(path: &Path) -> Result<BaselineSet, String> {
    let raw = fs::read_to_string(path)
        .map_err(|err| format!("Failed to read {}: {err}", path.display()))?;
    let value = raw
        .parse::<TomlValue>()
        .map_err(|err| format!("Failed to parse {}: {err}", path.display()))?;
    let table = value
        .as_table()
        .ok_or_else(|| format!("{} is not a TOML table", path.display()))?;

    let tolerance_percent = table
        .get("meta")
        .and_then(TomlValue::as_table)
        .and_then(|meta| meta.get("tolerance_percent"))
        .and_then(TomlValue::as_integer)
        .unwrap_or(5) as u64;

    let mut entries = BTreeMap::new();
    for (key, value) in table {
        if key == "meta" {
            continue;
        }
        let Some(entry_table) = value.as_table() else {
            continue;
        };
        let Some(budget_cu) = entry_table.get("budget_cu").and_then(TomlValue::as_integer) else {
            continue;
        };
        let category = entry_table
            .get("category")
            .and_then(TomlValue::as_str)
            .unwrap_or("")
            .to_string();
        entries.insert(
            key.clone(),
            BaselineEntry {
                budget_cu: budget_cu as u64,
                category,
            },
        );
    }

    Ok(BaselineSet {
        tolerance_percent,
        entries,
    })
}

fn ensure_payer_balance(client: &RpcClient, payer: &Keypair, rpc_url: &str) -> Result<(), String> {
    let balance = client
        .get_balance(&payer.pubkey())
        .map_err(|err| format!("Failed to read payer balance: {err}"))?;
    if balance >= LAMPORTS_PER_SOL {
        return Ok(());
    }

    if rpc_url.contains("127.0.0.1") || rpc_url.contains("localhost") {
        let signature = client
            .request_airdrop(&payer.pubkey(), 10 * LAMPORTS_PER_SOL)
            .map_err(|err| format!("Failed to request local airdrop: {err}"))?;
        client
            .poll_for_signature(&signature)
            .map_err(|err| format!("Failed waiting for airdrop confirmation: {err}"))?;
        return Ok(());
    }

    Err(format!(
        "Fee payer {} has insufficient balance and {} is not a local validator",
        payer.pubkey(), rpc_url
    ))
}

fn deploy_bench_program(workspace_root: &Path, rpc_url: &str, keypair_path: &Path) -> Result<Pubkey, String> {
    let so_path = resolve_bench_program_path(workspace_root)?;
    let args = vec![
        "program".to_string(),
        "deploy".to_string(),
        so_path.display().to_string(),
        "--output".to_string(),
        "json".to_string(),
        "--url".to_string(),
        rpc_url.to_string(),
        "--keypair".to_string(),
        keypair_path.display().to_string(),
    ];

    let output = workspace::run_output("solana", &args, workspace_root)?;
    if !output.status.success() {
        return Err(format!(
            "solana program deploy failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: JsonValue = serde_json::from_str(stdout.trim()).map_err(|err| {
        format!("Failed to parse deploy output as JSON: {err}")
    })?;
    let program_id = json
        .get("programId")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| "Deploy output did not contain programId".to_string())?;
    program_id
        .parse::<Pubkey>()
        .map_err(|err| format!("Invalid programId from deploy output: {err}"))
}

fn resolve_bench_program_path(workspace_root: &Path) -> Result<PathBuf, String> {
    let candidates = [
        workspace_root.join("target").join("deploy").join("hopper_bench.so"),
        workspace_root
            .join("bench")
            .join("hopper-bench")
            .join("target")
            .join("deploy")
            .join("hopper_bench.so"),
    ];

    candidates
        .into_iter()
        .find(|candidate| candidate.exists())
        .ok_or_else(|| {
            format!(
                "Could not find hopper_bench.so under {} after build-sbf",
                workspace_root.display()
            )
        })
}

fn run_case(
    client: &RpcClient,
    payer: &Keypair,
    program_id: &Pubkey,
    case: &BenchmarkCase,
) -> Result<(u64, Option<u64>, Vec<String>), String> {
    let fixture = if case.fixture.needs_fixture() {
        Some(create_fixture(client, payer, program_id, case.fixture.needs_header_init())?)
    } else {
        None
    };

    let accounts = match &fixture {
        Some(keypair) if case.fixture.duplicate_accounts() => {
            vec![
                AccountMeta::new(keypair.pubkey(), true),
                AccountMeta::new(keypair.pubkey(), true),
            ]
        }
        Some(keypair) => vec![AccountMeta::new(keypair.pubkey(), true)],
        None => Vec::new(),
    };

    let mut instruction_data = Vec::with_capacity(1 + case.payload.len());
    instruction_data.push(case.disc);
    instruction_data.extend_from_slice(case.payload);
    let instruction = Instruction::new_with_bytes(*program_id, &instruction_data, accounts);
    let signers: Vec<&Keypair> = fixture.as_ref().into_iter().collect();
    let simulation = simulate_instruction(client, payer, &[instruction], &signers)?;

    let logs = simulation.logs.unwrap_or_default();
    if let Some(err) = simulation.err {
        return Err(format!("Simulation error: {err:?}"));
    }

    let measured = parse_bounded_delta(case.name, &logs)
        .or_else(|| parse_fallback_delta(&logs))
        .ok_or_else(|| format!("Could not parse CU delta from logs for {}", case.name))?;

    Ok((measured, simulation.units_consumed, logs))
}

fn create_fixture(
    client: &RpcClient,
    payer: &Keypair,
    program_id: &Pubkey,
    initialize_header: bool,
) -> Result<Keypair, String> {
    let fixture = Keypair::new();
    let rent = client
        .get_minimum_balance_for_rent_exemption(BENCH_ACCOUNT_LEN)
        .map_err(|err| format!("Failed to query rent exemption: {err}"))?;
    let create_ix = system_instruction::create_account(
        &payer.pubkey(),
        &fixture.pubkey(),
        rent,
        BENCH_ACCOUNT_LEN as u64,
        program_id,
    );
    send_instruction(client, payer, &[create_ix], &[&fixture])?;

    if initialize_header {
        let init_ix = Instruction::new_with_bytes(
            *program_id,
            &[WRITE_HEADER_DISC],
            vec![AccountMeta::new(fixture.pubkey(), true)],
        );
        send_instruction(client, payer, &[init_ix], &[&fixture])?;
    }

    Ok(fixture)
}

fn send_instruction(
    client: &RpcClient,
    payer: &Keypair,
    instructions: &[Instruction],
    extra_signers: &[&Keypair],
) -> Result<(), String> {
    let recent_blockhash = client
        .get_latest_blockhash()
        .map_err(|err| format!("Failed to fetch recent blockhash: {err}"))?;
    let mut signers: Vec<&dyn Signer> = Vec::with_capacity(extra_signers.len() + 1);
    signers.push(payer);
    for signer in extra_signers {
        signers.push(*signer);
    }
    let tx = Transaction::new_signed_with_payer(
        instructions,
        Some(&payer.pubkey()),
        &signers,
        recent_blockhash,
    );
    client
        .send_and_confirm_transaction(&tx)
        .map_err(|err| format!("Failed to send transaction: {err}"))?;
    Ok(())
}

fn simulate_instruction(
    client: &RpcClient,
    payer: &Keypair,
    instructions: &[Instruction],
    extra_signers: &[&Keypair],
) -> Result<solana_client::rpc_response::RpcSimulateTransactionResult, String> {
    let recent_blockhash = client
        .get_latest_blockhash()
        .map_err(|err| format!("Failed to fetch recent blockhash: {err}"))?;
    let mut signers: Vec<&dyn Signer> = Vec::with_capacity(extra_signers.len() + 1);
    signers.push(payer);
    for signer in extra_signers {
        signers.push(*signer);
    }

    let tx = Transaction::new_signed_with_payer(
        instructions,
        Some(&payer.pubkey()),
        &signers,
        recent_blockhash,
    );
    let response = client
        .simulate_transaction_with_config(
            &tx,
            RpcSimulateTransactionConfig {
                sig_verify: false,
                replace_recent_blockhash: true,
                ..RpcSimulateTransactionConfig::default()
            },
        )
        .map_err(|err| format!("Failed to simulate transaction: {err}"))?;
    Ok(response.value)
}

fn parse_bounded_delta(case_name: &str, logs: &[String]) -> Option<u64> {
    let begin = format!("BEGIN {case_name}");
    let end = format!("END {case_name}");
    let mut inside = false;
    let mut values = Vec::new();

    for line in logs {
        if line.contains(&begin) {
            inside = true;
            continue;
        }
        if inside && line.contains(&end) {
            break;
        }
        if inside {
            if let Some(value) = extract_first_number(line) {
                values.push(value);
            }
        }
    }

    if values.len() >= 2 && values[0] >= values[1] {
        Some(values[0] - values[1])
    } else {
        None
    }
}

fn parse_fallback_delta(logs: &[String]) -> Option<u64> {
    let values: Vec<u64> = logs.iter().filter_map(|line| extract_first_number(line)).collect();
    if values.len() >= 2 && values[0] >= values[1] {
        Some(values[0] - values[1])
    } else {
        None
    }
}

fn extract_first_number(line: &str) -> Option<u64> {
    let mut digits = String::new();
    let mut capturing = false;
    for ch in line.chars() {
        if ch.is_ascii_digit() {
            digits.push(ch);
            capturing = true;
        } else if capturing {
            break;
        }
    }
    if digits.is_empty() {
        None
    } else {
        digits.parse::<u64>().ok()
    }
}

fn allowed_budget(budget: u64, tolerance_percent: u64) -> u64 {
    budget + ((budget * tolerance_percent) / 100)
}

fn percentage_delta(measured: u64, baseline: u64) -> f64 {
    if baseline == 0 {
        0.0
    } else {
        ((measured as f64 - baseline as f64) / baseline as f64) * 100.0
    }
}

fn read_tool_version(workspace_root: &Path, program: &str, args: &[String]) -> String {
    workspace::run_output(program, args, workspace_root)
        .ok()
        .and_then(|output| {
            if output.status.success() {
                Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
            } else {
                None
            }
        })
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

fn write_json_report(path: &Path, report: &BenchReport) -> Result<(), String> {
    let json = serde_json::to_string_pretty(report)
        .map_err(|err| format!("Failed to serialize JSON report: {err}"))?;
    fs::write(path, json).map_err(|err| format!("Failed to write {}: {err}", path.display()))
}

fn write_csv_report(path: &Path, report: &BenchReport) -> Result<(), String> {
    let mut csv = String::from("disc,name,baseline_key,baseline_category,fixture,measured_cu,total_units_consumed,baseline_cu,allowed_cu,delta_pct,status,regression,error\n");
    for result in &report.results {
        let delta = result
            .delta_pct
            .map(|value| format!("{value:.2}"))
            .unwrap_or_default();
        let error = result.error.clone().unwrap_or_default().replace('"', "'");
        csv.push_str(&format!(
            "{},{},{},{},{},{},{},{},{},{},{},{},\"{}\"\n",
            result.disc,
            result.name,
            result.baseline_key,
            result.baseline_category.clone().unwrap_or_default(),
            result.fixture,
            option_to_csv(result.measured_cu),
            option_to_csv(result.total_units_consumed),
            option_to_csv(result.baseline_cu),
            option_to_csv(result.allowed_cu),
            delta,
            result.status,
            result.regression,
            error,
        ));
    }
    fs::write(path, csv).map_err(|err| format!("Failed to write {}: {err}", path.display()))
}

fn option_to_csv(value: Option<u64>) -> String {
    value.map(|value| value.to_string()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_delta_uses_begin_end_markers() {
        let logs = vec![
            "Program log: BEGIN overlay".to_string(),
            "Program consumption: 199980 units remaining".to_string(),
            "Program consumption: 199972 units remaining".to_string(),
            "Program log: END overlay".to_string(),
        ];
        assert_eq!(parse_bounded_delta("overlay", &logs), Some(8));
    }

    #[test]
    fn baseline_file_parsing_reads_budget_entries() {
        let path = std::env::temp_dir().join("hopper-cli-bench-baselines.toml");
        fs::write(
            &path,
            "[meta]\ntolerance_percent = 7\n\n[overlay_57b]\nbudget_cu = 8\ncategory = \"overlay\"\n",
        )
        .unwrap();
        let baselines = load_baselines(&path).unwrap();
        assert_eq!(baselines.tolerance_percent, 7);
        assert_eq!(baselines.entries.get("overlay_57b").unwrap().budget_cu, 8);
        let _ = fs::remove_file(path);
    }
}
