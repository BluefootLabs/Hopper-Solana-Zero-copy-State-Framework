use {
    mollusk_svm::{program::keyed_account_for_system_program, Mollusk},
    serde::Serialize,
    solana_account::Account,
    solana_address::Address,
    solana_instruction::{AccountMeta, Instruction},
    std::{
        env,
        fs,
        path::{Path, PathBuf},
    },
};

const HOPPER_PROGRAM_ID: Address = Address::new_from_array([7; 32]);
const QUASAR_PROGRAM_ID: Address = Address::new_from_array(five8_const::decode_32_const(
    "33333333333333333333333333333333333333333333",
));
const PINOCCHIO_PROGRAM_ID: Address = Address::new_from_array([
    0x1e, 0x3c, 0xd6, 0x28, 0x43, 0x80, 0x94, 0x0e, 0x08, 0x62, 0x4c, 0xb8, 0x33, 0x8b, 0x77,
    0xdc, 0x33, 0x25, 0x75, 0xd1, 0x5f, 0xa3, 0x9a, 0x0f, 0x1d, 0xf1, 0x5e, 0xe0, 0x8f, 0xb8,
    0x23, 0xee,
]);
// Deterministic Anchor-framework program ID for the cross-framework
// comparison matrix. Distinct from the other slots so Mollusk can
// distinguish calls. See `bench/METHODOLOGY.md` for the shared
// contract the anchor vault must implement.
const ANCHOR_PROGRAM_ID: Address = Address::new_from_array([
    0xa7, 0xcb, 0x04, 0x12, 0xe8, 0x55, 0x6f, 0x91, 0x3b, 0x0c, 0xaa, 0xfe, 0x22, 0x40, 0x44,
    0x56, 0x8d, 0x1e, 0x63, 0x7a, 0x04, 0xcd, 0x15, 0x87, 0xbe, 0xef, 0x00, 0x00, 0xfa, 0xde,
    0xd1, 0xee,
]);
const SYSTEM_PROGRAM_ID: Address = Address::new_from_array([0; 32]);

const USER_LAMPORTS: u64 = 10_000_000_000;
const DEPOSIT_AMOUNT: u64 = 1_000_000_000;
const WITHDRAW_VAULT_LAMPORTS: u64 = 1_000_000_000;
const WITHDRAW_AMOUNT: u64 = 500_000_000;
const COUNTER_DATA_LEN: usize = 40;
const COUNTER_INITIAL_VALUE: u64 = 7;
const SHARED_USER_CASES: [[u8; 32]; 8] = [
    [0x11; 32],
    [0x22; 32],
    [0x33; 32],
    [0x44; 32],
    [0x55; 32],
    [0x66; 32],
    [0x77; 32],
    [0x88; 32],
];

#[derive(Serialize)]
struct Methodology {
    runner: &'static str,
    samples: usize,
    authorize: &'static str,
    authorize_failure: &'static str,
    counter_access: &'static str,
    deposit: &'static str,
    withdraw: &'static str,
    safety_check: &'static str,
}

#[derive(Serialize)]
struct FrameworkMetric {
    framework: &'static str,
    authorize_cu: u64,
    authorize_missing_signature_cu: u64,
    counter_access_cu: u64,
    deposit_cu: u64,
    withdraw_cu: u64,
    authorize_vs_hopper: i64,
    counter_access_vs_hopper: i64,
    deposit_vs_hopper: i64,
    withdraw_vs_hopper: i64,
    binary_size_bytes: u64,
    binary_size_kib: f64,
    unsigned_withdraw_rejected: bool,
}

#[derive(Serialize)]
struct Report {
    hopper_root: String,
    quasar_root: String,
    methodology: Methodology,
    benchmarks: Vec<FrameworkMetric>,
}

struct ProgramSpec {
    framework: &'static str,
    program_id: Address,
    binary_path: PathBuf,
}

struct Args {
    quasar_root: PathBuf,
    /// Optional anchor-vault build root. When provided, an
    /// `anchor` framework entry is appended to the measurement
    /// matrix. See `bench/METHODOLOGY.md` for the shared contract.
    anchor_root: Option<PathBuf>,
    out_dir: PathBuf,
}

fn main() {
    if let Err(err) = run() {
        eprintln!("{err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args = parse_args()?;
    let workspace_root = workspace_root()?;

    // Audit D4 closure: Hopper is the baseline. Quasar + pinocchio-style
    // are required by the legacy bench contract (quasar_root supplies
    // both). Anchor is optional, pass `--anchor-root <path>` and the
    // anchor_vault.so there is included in the matrix. Frameworks whose
    // binaries are missing are skipped and logged rather than erroring
    // out; partial runs are valid during development. CI builds require
    // every framework to be present (see `bench/METHODOLOGY.md`).
    let mut specs: Vec<ProgramSpec> = vec![
        ProgramSpec {
            framework: "hopper",
            program_id: HOPPER_PROGRAM_ID,
            binary_path: workspace_root.join("target/deploy/hopper_parity_vault.so"),
        },
        ProgramSpec {
            framework: "quasar",
            program_id: QUASAR_PROGRAM_ID,
            binary_path: args.quasar_root.join("target/deploy/quasar_vault.so"),
        },
        ProgramSpec {
            framework: "pinocchio-style",
            program_id: PINOCCHIO_PROGRAM_ID,
            binary_path: args.quasar_root.join("target/deploy/pinocchio_vault.so"),
        },
    ];
    if let Some(anchor_root) = &args.anchor_root {
        specs.push(ProgramSpec {
            framework: "anchor",
            program_id: ANCHOR_PROGRAM_ID,
            binary_path: anchor_root.join("target/deploy/anchor_vault.so"),
        });
    }

    // Hopper baseline is required. Other frameworks are skipped with
    // a log line if their artifact is missing.
    specs.retain(|spec| match ensure_file(&spec.binary_path) {
        Ok(()) => true,
        Err(msg) => {
            if spec.framework == "hopper" {
                // Keep the spec so the later baseline-lookup fails loudly.
                eprintln!("error: {msg}");
                true
            } else {
                eprintln!("skipping framework `{}`: {msg}", spec.framework);
                false
            }
        }
    });

    fs::create_dir_all(&args.out_dir).map_err(|err| {
        format!("failed to create output directory {}: {err}", args.out_dir.display())
    })?;

    let mut benchmarks = Vec::with_capacity(specs.len());
    for spec in &specs {
        benchmarks.push(run_program(spec)?);
    }

    let hopper_index = benchmarks
        .iter()
        .position(|benchmark| benchmark.framework == "hopper")
        .ok_or_else(|| "missing Hopper baseline in benchmark results".to_string())?;
    let hopper_authorize = benchmarks[hopper_index].authorize_cu;
    let hopper_counter_access = benchmarks[hopper_index].counter_access_cu;
    let hopper_deposit = benchmarks[hopper_index].deposit_cu;
    let hopper_withdraw = benchmarks[hopper_index].withdraw_cu;

    for benchmark in &mut benchmarks {
        benchmark.authorize_vs_hopper = benchmark.authorize_cu as i64 - hopper_authorize as i64;
        benchmark.counter_access_vs_hopper = benchmark.counter_access_cu as i64 - hopper_counter_access as i64;
        benchmark.deposit_vs_hopper = benchmark.deposit_cu as i64 - hopper_deposit as i64;
        benchmark.withdraw_vs_hopper = benchmark.withdraw_cu as i64 - hopper_withdraw as i64;
    }

    let report = Report {
        hopper_root: workspace_root.display().to_string(),
        quasar_root: args.quasar_root.display().to_string(),
        methodology: Methodology {
            runner: "mollusk-svm shared host runner loading all three compiled SBF binaries",
            samples: SHARED_USER_CASES.len(),
            authorize: "average across shared deterministic user seed cases; signer + writable + PDA validation only on the same ['vault', user] PDA shape with no CPI or lamport mutation",
            authorize_failure: "missing-signature variant of the authorize path; must fail without mutating balances and exposes the early validation cost",
            counter_access: "average across shared deterministic user seed cases; same ['vault', user] PDA plus a 40-byte raw state region [authority:32][counter:8], validated and incremented without CPI or lamport mutation",
            deposit: "average across shared deterministic user seed cases; user signer -> program-owned vault PDA via system-program transfer CPI with PDA seeds [\"vault\", user]",
            withdraw: "average across shared deterministic user seed cases; program-owned vault PDA -> user via direct lamport mutation after signer and PDA validation",
            safety_check: "unsigned withdraw must fail without mutating balances",
        },
        benchmarks,
    };

    let json_path = args.out_dir.join("vault-framework-comparison.json");
    let csv_path = args.out_dir.join("vault-framework-comparison.csv");

    fs::write(
        &json_path,
        serde_json::to_string_pretty(&report)
            .map_err(|err| format!("failed to serialize JSON report: {err}"))?,
    )
    .map_err(|err| format!("failed to write {}: {err}", json_path.display()))?;

    fs::write(&csv_path, csv_report(&report))
        .map_err(|err| format!("failed to write {}: {err}", csv_path.display()))?;

    println!();
    println!("Vault framework comparison");
    println!(
        "{:<16} {:>10} {:>11} {:>10} {:>11} {:>10} {:>11} {:>11}",
        "Framework",
        "Authorize",
        "AuthFail",
        "Counter",
        "Deposit",
        "Withdraw",
        "Binary KiB",
        "Safety"
    );
    for benchmark in &report.benchmarks {
        println!(
            "{:<16} {:>10} {:>11} {:>10} {:>11} {:>10} {:>11.2} {:>11}",
            benchmark.framework,
            benchmark.authorize_cu,
            benchmark.authorize_missing_signature_cu,
            benchmark.counter_access_cu,
            benchmark.deposit_cu,
            benchmark.withdraw_cu,
            benchmark.binary_size_kib,
            if benchmark.unsigned_withdraw_rejected {
                "rejected"
            } else {
                "FAILED"
            }
        );
    }
    println!();
    println!("Wrote {}", json_path.display());
    println!("Wrote {}", csv_path.display());

    Ok(())
}

fn parse_args() -> Result<Args, String> {
    let mut quasar_root = None;
    let mut anchor_root: Option<PathBuf> = None;
    let mut out_dir = None;
    let mut args = env::args().skip(1);

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--quasar-root" => {
                let value = args
                    .next()
                    .ok_or_else(|| "missing value for --quasar-root".to_string())?;
                quasar_root = Some(PathBuf::from(value));
            }
            "--anchor-root" => {
                let value = args
                    .next()
                    .ok_or_else(|| "missing value for --anchor-root".to_string())?;
                anchor_root = Some(PathBuf::from(value));
            }
            "--out-dir" => {
                let value = args
                    .next()
                    .ok_or_else(|| "missing value for --out-dir".to_string())?;
                out_dir = Some(PathBuf::from(value));
            }
            _ => {
                return Err(format!(
                    "unknown argument '{arg}'; expected --quasar-root <path> [--anchor-root <path>] [--out-dir <path>]"
                ));
            }
        }
    }

    let workspace_root = workspace_root()?;
    let quasar_root = quasar_root.ok_or_else(|| "missing required --quasar-root <path>".to_string())?;
    let quasar_root = canonicalize_existing(&quasar_root)?;
    let anchor_root = anchor_root
        .map(|p| canonicalize_existing(&p))
        .transpose()?;
    let out_dir = match out_dir {
        Some(path) if path.is_absolute() => path,
        Some(path) => workspace_root.join(path),
        None => workspace_root.join("bench/results/framework-vaults"),
    };

    Ok(Args {
        quasar_root,
        anchor_root,
        out_dir,
    })
}

fn workspace_root() -> Result<PathBuf, String> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .ok_or_else(|| format!("failed to resolve workspace root from {}", manifest_dir.display()))
}

fn canonicalize_existing(path: &Path) -> Result<PathBuf, String> {
    fs::canonicalize(path)
        .map_err(|err| format!("failed to resolve {}: {err}", path.display()))
}

fn ensure_file(path: &Path) -> Result<(), String> {
    if path.is_file() {
        Ok(())
    } else {
        Err(format!("missing benchmark artifact {}", path.display()))
    }
}

fn run_program(spec: &ProgramSpec) -> Result<FrameworkMetric, String> {
    let authorize_cu = average_cu(spec, run_authorize_case)?;
    let authorize_missing_signature_cu = average_cu(spec, run_authorize_missing_signature_case)?;
    let counter_access_cu = average_cu(spec, run_counter_access_case)?;
    let deposit_cu = average_cu(spec, run_deposit_case)?;
    let withdraw_cu = average_cu(spec, run_withdraw_case)?;
    let unsigned_withdraw_rejected = run_unsigned_withdraw_rejection(spec, shared_user(0))?;
    let binary_size_bytes = fs::metadata(&spec.binary_path)
        .map_err(|err| format!("failed to stat {}: {err}", spec.binary_path.display()))?
        .len();

    Ok(FrameworkMetric {
        framework: spec.framework,
        authorize_cu,
        authorize_missing_signature_cu,
        counter_access_cu,
        deposit_cu,
        withdraw_cu,
        authorize_vs_hopper: 0,
        counter_access_vs_hopper: 0,
        deposit_vs_hopper: 0,
        withdraw_vs_hopper: 0,
        binary_size_bytes,
        binary_size_kib: binary_size_bytes as f64 / 1024.0,
        unsigned_withdraw_rejected,
    })
}

fn average_cu(
    spec: &ProgramSpec,
    measure: fn(&ProgramSpec, Address) -> Result<u64, String>,
) -> Result<u64, String> {
    let mut total = 0u64;
    let mut i = 0;
    while i < SHARED_USER_CASES.len() {
        total = total
            .checked_add(measure(spec, shared_user(i))?)
            .ok_or_else(|| format!("{} CU total overflowed during averaging", spec.framework))?;
        i += 1;
    }
    Ok(total / SHARED_USER_CASES.len() as u64)
}

fn shared_user(index: usize) -> Address {
    Address::new_from_array(SHARED_USER_CASES[index])
}

fn run_deposit_case(spec: &ProgramSpec, user: Address) -> Result<u64, String> {
    let mollusk = mollusk(spec)?;
    let (system_program, system_program_account) = keyed_account_for_system_program();
    let vault = vault_address(&spec.program_id, &user);

    let result = mollusk.process_instruction(
        &deposit_instruction(spec.program_id, user, true, vault, system_program, DEPOSIT_AMOUNT),
        &[
            (user, Account::new(USER_LAMPORTS, 0, &SYSTEM_PROGRAM_ID)),
            (vault, Account::new(0, 0, &spec.program_id)),
            (system_program, system_program_account),
        ],
    );

    if !result.program_result.is_ok() {
        return Err(format!(
            "{} deposit failed: {:?}",
            spec.framework,
            result.program_result,
        ));
    }

    let user_after = lamports_for(&result.resulting_accounts, &user)?;
    let vault_after = lamports_for(&result.resulting_accounts, &vault)?;

    if user_after != USER_LAMPORTS - DEPOSIT_AMOUNT {
        return Err(format!(
            "{} deposit user lamports mismatch: expected {}, got {}",
            spec.framework,
            USER_LAMPORTS - DEPOSIT_AMOUNT,
            user_after,
        ));
    }
    if vault_after != DEPOSIT_AMOUNT {
        return Err(format!(
            "{} deposit vault lamports mismatch: expected {}, got {}",
            spec.framework,
            DEPOSIT_AMOUNT,
            vault_after,
        ));
    }

    Ok(result.compute_units_consumed)
}

fn run_authorize_case(spec: &ProgramSpec, user: Address) -> Result<u64, String> {
    let mollusk = mollusk(spec)?;
    let vault = vault_address(&spec.program_id, &user);

    let result = mollusk.process_instruction(
        &authorize_instruction(spec.program_id, user, true, vault),
        &[
            (user, Account::new(USER_LAMPORTS, 0, &SYSTEM_PROGRAM_ID)),
            (vault, Account::new(WITHDRAW_VAULT_LAMPORTS, 0, &spec.program_id)),
        ],
    );

    if !result.program_result.is_ok() {
        return Err(format!(
            "{} authorize failed: {:?}",
            spec.framework,
            result.program_result,
        ));
    }

    let user_after = lamports_for(&result.resulting_accounts, &user)?;
    let vault_after = lamports_for(&result.resulting_accounts, &vault)?;

    if user_after != USER_LAMPORTS || vault_after != WITHDRAW_VAULT_LAMPORTS {
        return Err(format!(
            "{} authorize mutated balances unexpectedly: user {}, vault {}",
            spec.framework,
            user_after,
            vault_after,
        ));
    }

    Ok(result.compute_units_consumed)
}

fn run_authorize_missing_signature_case(spec: &ProgramSpec, user: Address) -> Result<u64, String> {
    let mollusk = mollusk(spec)?;
    let vault = vault_address(&spec.program_id, &user);

    let result = mollusk.process_instruction(
        &authorize_instruction(spec.program_id, user, false, vault),
        &[
            (user, Account::new(USER_LAMPORTS, 0, &SYSTEM_PROGRAM_ID)),
            (vault, Account::new(WITHDRAW_VAULT_LAMPORTS, 0, &spec.program_id)),
        ],
    );

    if result.program_result.is_ok() {
        return Err(format!(
            "{} authorize missing-signature path unexpectedly succeeded",
            spec.framework,
        ));
    }

    let user_after = lamports_for(&result.resulting_accounts, &user)?;
    let vault_after = lamports_for(&result.resulting_accounts, &vault)?;
    if user_after != USER_LAMPORTS || vault_after != WITHDRAW_VAULT_LAMPORTS {
        return Err(format!(
            "{} authorize missing-signature path mutated balances unexpectedly: user {}, vault {}",
            spec.framework,
            user_after,
            vault_after,
        ));
    }

    Ok(result.compute_units_consumed)
}

fn run_counter_access_case(spec: &ProgramSpec, user: Address) -> Result<u64, String> {
    let mollusk = mollusk(spec)?;
    let vault = vault_address(&spec.program_id, &user);

    let result = mollusk.process_instruction(
        &counter_access_instruction(spec.program_id, user, true, vault),
        &[
            (user, Account::new(USER_LAMPORTS, 0, &SYSTEM_PROGRAM_ID)),
            (vault, counter_vault_account(&spec.program_id, &user)),
        ],
    );

    if !result.program_result.is_ok() {
        return Err(format!(
            "{} counter access failed: {:?}",
            spec.framework,
            result.program_result,
        ));
    }

    let user_after = lamports_for(&result.resulting_accounts, &user)?;
    let vault_after = lamports_for(&result.resulting_accounts, &vault)?;
    let counter_after = counter_for(&result.resulting_accounts, &vault)?;

    if user_after != USER_LAMPORTS || vault_after != WITHDRAW_VAULT_LAMPORTS {
        return Err(format!(
            "{} counter access mutated balances unexpectedly: user {}, vault {}",
            spec.framework,
            user_after,
            vault_after,
        ));
    }
    if counter_after != COUNTER_INITIAL_VALUE + 1 {
        return Err(format!(
            "{} counter access counter mismatch: expected {}, got {}",
            spec.framework,
            COUNTER_INITIAL_VALUE + 1,
            counter_after,
        ));
    }

    Ok(result.compute_units_consumed)
}

fn run_withdraw_case(spec: &ProgramSpec, user: Address) -> Result<u64, String> {
    let mollusk = mollusk(spec)?;
    let vault = vault_address(&spec.program_id, &user);

    let result = mollusk.process_instruction(
        &withdraw_instruction(spec.program_id, user, true, vault, WITHDRAW_AMOUNT),
        &[
            (user, Account::new(USER_LAMPORTS, 0, &SYSTEM_PROGRAM_ID)),
            (vault, Account::new(WITHDRAW_VAULT_LAMPORTS, 0, &spec.program_id)),
        ],
    );

    if !result.program_result.is_ok() {
        return Err(format!(
            "{} withdraw failed: {:?}",
            spec.framework,
            result.program_result,
        ));
    }

    let user_after = lamports_for(&result.resulting_accounts, &user)?;
    let vault_after = lamports_for(&result.resulting_accounts, &vault)?;

    if user_after != USER_LAMPORTS + WITHDRAW_AMOUNT {
        return Err(format!(
            "{} withdraw user lamports mismatch: expected {}, got {}",
            spec.framework,
            USER_LAMPORTS + WITHDRAW_AMOUNT,
            user_after,
        ));
    }
    if vault_after != WITHDRAW_VAULT_LAMPORTS - WITHDRAW_AMOUNT {
        return Err(format!(
            "{} withdraw vault lamports mismatch: expected {}, got {}",
            spec.framework,
            WITHDRAW_VAULT_LAMPORTS - WITHDRAW_AMOUNT,
            vault_after,
        ));
    }

    Ok(result.compute_units_consumed)
}

fn run_unsigned_withdraw_rejection(spec: &ProgramSpec, user: Address) -> Result<bool, String> {
    let mollusk = mollusk(spec)?;
    let vault = vault_address(&spec.program_id, &user);

    let result = mollusk.process_instruction(
        &withdraw_instruction(spec.program_id, user, false, vault, WITHDRAW_AMOUNT),
        &[
            (user, Account::new(USER_LAMPORTS, 0, &SYSTEM_PROGRAM_ID)),
            (vault, Account::new(WITHDRAW_VAULT_LAMPORTS, 0, &spec.program_id)),
        ],
    );

    if result.program_result.is_ok() {
        return Ok(false);
    }

    let user_after = lamports_for(&result.resulting_accounts, &user)?;
    let vault_after = lamports_for(&result.resulting_accounts, &vault)?;

    Ok(user_after == USER_LAMPORTS && vault_after == WITHDRAW_VAULT_LAMPORTS)
}

fn mollusk(spec: &ProgramSpec) -> Result<Mollusk, String> {
    Ok(Mollusk::new(
        &spec.program_id,
        &mollusk_program_path(&spec.binary_path),
    ))
}

fn mollusk_program_path(path: &Path) -> String {
    let mut stem = path.to_path_buf();
    stem.set_extension("");
    stem.display().to_string()
}

fn vault_address(program_id: &Address, user: &Address) -> Address {
    let (vault, _) = Address::find_program_address(&[b"vault", user.as_ref()], program_id);
    vault
}

fn deposit_instruction(
    program_id: Address,
    user: Address,
    user_is_signer: bool,
    vault: Address,
    system_program: Address,
    amount: u64,
) -> Instruction {
    let mut data = vec![0u8];
    data.extend_from_slice(&amount.to_le_bytes());
    Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(user, user_is_signer),
            AccountMeta::new(vault, false),
            AccountMeta::new_readonly(system_program, false),
        ],
        data,
    }
}

fn authorize_instruction(
    program_id: Address,
    user: Address,
    user_is_signer: bool,
    vault: Address,
) -> Instruction {
    Instruction {
        program_id,
        accounts: vec![AccountMeta::new(user, user_is_signer), AccountMeta::new(vault, false)],
        data: vec![2u8],
    }
}

fn counter_access_instruction(
    program_id: Address,
    user: Address,
    user_is_signer: bool,
    vault: Address,
) -> Instruction {
    Instruction {
        program_id,
        accounts: vec![AccountMeta::new(user, user_is_signer), AccountMeta::new(vault, false)],
        data: vec![3u8],
    }
}

fn withdraw_instruction(
    program_id: Address,
    user: Address,
    user_is_signer: bool,
    vault: Address,
    amount: u64,
) -> Instruction {
    let mut data = vec![1u8];
    data.extend_from_slice(&amount.to_le_bytes());
    Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(user, user_is_signer),
            AccountMeta::new(vault, false),
        ],
        data,
    }
}

fn lamports_for(accounts: &[(Address, Account)], key: &Address) -> Result<u64, String> {
    accounts
        .iter()
        .find(|(address, _)| address == key)
        .map(|(_, account)| account.lamports)
        .ok_or_else(|| format!("missing resulting account {}", key))
}

fn counter_vault_account(program_id: &Address, authority: &Address) -> Account {
    let mut account = Account::new(WITHDRAW_VAULT_LAMPORTS, COUNTER_DATA_LEN, program_id);
    account.data[..32].copy_from_slice(authority.as_ref());
    account.data[32..40].copy_from_slice(&COUNTER_INITIAL_VALUE.to_le_bytes());
    account
}

fn counter_for(accounts: &[(Address, Account)], key: &Address) -> Result<u64, String> {
    let account = accounts
        .iter()
        .find(|(address, _)| address == key)
        .map(|(_, account)| account)
        .ok_or_else(|| format!("missing resulting account {}", key))?;

    if account.data.len() < COUNTER_DATA_LEN {
        return Err(format!(
            "resulting account {} data too small for counter access scenario",
            key,
        ));
    }

    let mut counter_bytes = [0u8; 8];
    counter_bytes.copy_from_slice(&account.data[32..40]);
    Ok(u64::from_le_bytes(counter_bytes))
}

fn csv_report(report: &Report) -> String {
    let mut out = String::from(
        "Framework,AuthorizeCu,AuthorizeMissingSignatureCu,CounterAccessCu,DepositCu,WithdrawCu,AuthorizeVsHopper,CounterAccessVsHopper,DepositVsHopper,WithdrawVsHopper,BinarySizeBytes,BinarySizeKiB,UnsignedWithdrawRejected\n",
    );

    for benchmark in &report.benchmarks {
        out.push_str(&format!(
            "{},{},{},{},{},{},{},{},{},{},{},{:.2},{}\n",
            benchmark.framework,
            benchmark.authorize_cu,
            benchmark.authorize_missing_signature_cu,
            benchmark.counter_access_cu,
            benchmark.deposit_cu,
            benchmark.withdraw_cu,
            benchmark.authorize_vs_hopper,
            benchmark.counter_access_vs_hopper,
            benchmark.deposit_vs_hopper,
            benchmark.withdraw_vs_hopper,
            benchmark.binary_size_bytes,
            benchmark.binary_size_kib,
            benchmark.unsigned_withdraw_rejected,
        ));
    }

    out
}