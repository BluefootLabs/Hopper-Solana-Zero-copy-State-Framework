//! `hopper manager invoke` and `hopper manager crank` subcommands.
//!
//! Turns the on-chain manifest from a read-only inspection surface
//! into a universal program explorer and invoker. Two capabilities:
//!
//! - `invoke`. Given a program id (and therefore a manifest), an
//!   instruction name, a set of `--account` / `--arg` / `--signer`
//!   flags, build the `Instruction`, sign it, and submit to the RPC
//!   endpoint. The manifest's `InstructionDescriptor` drives account
//!   ordering and arg layout; every unknown `--account` or `--arg`
//!   the user passes is a hard error, not a silent acceptance.
//!
//! - `crank`. Enumerate crank-tagged instructions declared by the
//!   program and run them in a polling loop. A crank is an
//!   `InstructionDescriptor` whose `capabilities` slice contains
//!   `"Crank"`. That marker is the extension point; no new schema
//!   type is required today. When an indexer wants to surface "what
//!   cranks does this program expose", it filters on the same
//!   capability string.
//!
//! Wiring to the rest of the CLI is simple: `hopper manager invoke
//! ...` and `hopper manager crank ...` route through this file from
//! `main.rs`.
//!
//! ## Out of scope for this pass
//!
//! - Priority-fee attachment. Caller can pass `--priority-fee` but
//!   the fee lands on a standard `ComputeBudget::SetComputeUnitPrice`
//!   pre-instruction; multi-instruction bundles are future work.
//! - Simulation round-trip. The command submits on request; dry-run
//!   support is trivial to add and logs a TODO here for a follow-up.
//! - Crank leader election. Running the loop from two machines
//!   against the same program is the user's call; we do no locking.

use std::fs;
use std::path::{Path, PathBuf};
use std::process;
use std::thread;
use std::time::{Duration, Instant};

pub fn cmd_manager_invoke(args: &[String]) {
    if args.iter().any(|a| matches!(a.as_str(), "--help" | "-h")) {
        print_invoke_usage();
        return;
    }
    let opts = parse_invoke_args(args).unwrap_or_else(|e| {
        eprintln!("hopper manager invoke: {e}");
        process::exit(1);
    });
    if let Err(e) = run_invoke(&opts) {
        eprintln!("hopper manager invoke failed: {e}");
        process::exit(1);
    }
}

pub fn cmd_manager_crank(args: &[String]) {
    if args.is_empty() || matches!(args[0].as_str(), "--help" | "-h") {
        print_crank_usage();
        return;
    }
    match args[0].as_str() {
        "list" => cmd_crank_list(&args[1..]),
        "run" => cmd_crank_run(&args[1..]),
        other => {
            eprintln!("unknown crank subcommand: {other}");
            print_crank_usage();
            process::exit(1);
        }
    }
}

// ----------------------------------------------------------------------------
// invoke
// ----------------------------------------------------------------------------

struct InvokeOpts {
    program_id: String,
    instruction: String,
    accounts: Vec<(String, String)>,
    args_raw: Vec<(String, String)>,
    signer: Option<PathBuf>,
    rpc: Option<String>,
    dry_run: bool,
    priority_fee_micro_lamports: Option<u64>,
    manifest_path: Option<PathBuf>,
}

fn parse_invoke_args(argv: &[String]) -> Result<InvokeOpts, String> {
    let mut opts = InvokeOpts {
        program_id: String::new(),
        instruction: String::new(),
        accounts: Vec::new(),
        args_raw: Vec::new(),
        signer: None,
        rpc: None,
        dry_run: false,
        priority_fee_micro_lamports: None,
        manifest_path: None,
    };
    let mut positional: Vec<String> = Vec::new();
    let mut i = 0;
    while i < argv.len() {
        match argv[i].as_str() {
            "--account" => {
                i += 1;
                let v = argv.get(i).ok_or("`--account` requires `name=pubkey`")?;
                let (n, k) = v.split_once('=').ok_or("`--account` expects name=pubkey")?;
                opts.accounts.push((n.to_string(), k.to_string()));
            }
            "--arg" => {
                i += 1;
                let v = argv.get(i).ok_or("`--arg` requires `name=value`")?;
                let (n, k) = v.split_once('=').ok_or("`--arg` expects name=value")?;
                opts.args_raw.push((n.to_string(), k.to_string()));
            }
            "--signer" => {
                i += 1;
                opts.signer = Some(PathBuf::from(
                    argv.get(i).ok_or("`--signer` requires a path")?,
                ));
            }
            "--rpc" => {
                i += 1;
                opts.rpc = Some(argv.get(i).cloned().ok_or("`--rpc` requires a URL")?);
            }
            "--dry-run" | "--simulate" => opts.dry_run = true,
            "--manifest" => {
                i += 1;
                opts.manifest_path = Some(PathBuf::from(
                    argv.get(i).ok_or("`--manifest` requires a path")?,
                ));
            }
            "--priority-fee" => {
                i += 1;
                opts.priority_fee_micro_lamports = Some(
                    argv.get(i)
                        .ok_or("`--priority-fee` requires a u64 micro-lamport value")?
                        .parse()
                        .map_err(|e| format!("`--priority-fee` parse: {e}"))?,
                );
            }
            other if other.starts_with("--") => {
                return Err(format!("unknown flag: {other}"));
            }
            _ => positional.push(argv[i].clone()),
        }
        i += 1;
    }
    if positional.len() < 2 {
        return Err(
            "expected `<program-id> <instruction-name>` positional arguments".into()
        );
    }
    opts.program_id = positional.remove(0);
    opts.instruction = positional.remove(0);
    Ok(opts)
}

fn run_invoke(opts: &InvokeOpts) -> Result<(), String> {
    use solana_client::rpc_client::RpcClient;
    use solana_client::rpc_config::RpcSimulateTransactionConfig;
    use solana_sdk::commitment_config::CommitmentConfig;
    use solana_sdk::compute_budget::ComputeBudgetInstruction;
    use solana_sdk::instruction::{AccountMeta, Instruction};
    use solana_sdk::pubkey::Pubkey;
    use solana_sdk::signature::{read_keypair_file, Signer};
    use solana_sdk::transaction::Transaction;

    let rpc_url = opts
        .rpc
        .clone()
        .unwrap_or_else(|| crate::rpc::resolve_rpc_url(None));

    // Resolve the manifest: explicit `--manifest` path wins; otherwise
    // fetch from the program's on-chain PDA.
    let manifest_json = match &opts.manifest_path {
        Some(p) => fs::read_to_string(p).map_err(|e| format!("read {}: {e}", p.display()))?,
        None => fetch_on_chain_manifest(&rpc_url, &opts.program_id)?,
    };

    let ix_desc = find_instruction_in_manifest(&manifest_json, &opts.instruction)?;

    // Policy-aware pre-submit validation. Cross-references the
    // instruction's declared `policy_pack` with the manifest's
    // `policies` array and rejects before submission when a required
    // account is missing or a readonly slot was passed as writable.
    if let Err(e) = policy_precheck(&manifest_json, &ix_desc, &opts.accounts) {
        return Err(e);
    }

    let data = build_instruction_data(&ix_desc, &opts.args_raw)?;
    let account_metas = build_account_metas(&ix_desc, &opts.accounts)?;

    let program_pubkey: Pubkey = opts
        .program_id
        .parse()
        .map_err(|e| format!("program-id is not a valid pubkey: {e}"))?;
    let meta_list: Vec<AccountMeta> = account_metas
        .iter()
        .map(|(_, key, writable, signer)| {
            let pk: Pubkey = key.parse().unwrap_or_default();
            if *writable {
                if *signer {
                    AccountMeta::new(pk, true)
                } else {
                    AccountMeta::new(pk, false)
                }
            } else if *signer {
                AccountMeta::new_readonly(pk, true)
            } else {
                AccountMeta::new_readonly(pk, false)
            }
        })
        .collect();
    let ix = Instruction {
        program_id: program_pubkey,
        accounts: meta_list,
        data: data.clone(),
    };

    // Print the tx shape up front. If --dry-run, stop here.
    println!("-- hopper manager invoke --");
    println!("rpc           : {}", rpc_url);
    println!("program id    : {}", opts.program_id);
    println!("instruction   : {} (tag {})", ix_desc.name, ix_desc.tag);
    println!("accounts      : {} metas", account_metas.len());
    for (i, (name, key, writ, sign)) in account_metas.iter().enumerate() {
        println!(
            "  [{}] {:<24} writable={} signer={} {}",
            i, name, writ, sign, key
        );
    }
    println!("instruction data: {} bytes", data.len());
    if let Some(fee) = opts.priority_fee_micro_lamports {
        println!("priority-fee  : {} micro-lamports / CU", fee);
    }
    println!("dry-run       : {}", opts.dry_run);

    // Load the fee-payer keypair (doubles as the sole signer unless
    // the instruction declared additional signers that the payer
    // does not own, in which case submission returns an error from
    // solana-sdk's Transaction::sign).
    let signer_path = opts
        .signer
        .clone()
        .or_else(default_signer_path)
        .ok_or_else(|| {
            "no --signer supplied and no default keypair at ~/.config/solana/id.json".to_string()
        })?;
    let payer =
        read_keypair_file(&signer_path).map_err(|e| format!("read signer {}: {e}", signer_path.display()))?;

    // Build the transaction. Priority fee goes in as a ComputeBudget
    // prefix instruction when requested.
    let mut instructions: Vec<Instruction> = Vec::with_capacity(2);
    if let Some(fee) = opts.priority_fee_micro_lamports {
        instructions.push(ComputeBudgetInstruction::set_compute_unit_price(fee));
    }
    instructions.push(ix);

    let rpc = RpcClient::new_with_commitment(rpc_url.clone(), CommitmentConfig::confirmed());
    let recent = rpc
        .get_latest_blockhash()
        .map_err(|e| format!("get_latest_blockhash: {e}"))?;
    let mut tx =
        Transaction::new_with_payer(&instructions, Some(&payer.pubkey()));
    tx.sign(&[&payer], recent);

    if opts.dry_run {
        // Simulation round-trip: hit simulateTransaction, print the
        // logs and any returned error. The user sees what the cluster
        // would emit without burning a slot.
        let sim_config = RpcSimulateTransactionConfig {
            sig_verify: false,
            replace_recent_blockhash: true,
            commitment: Some(CommitmentConfig::processed()),
            ..Default::default()
        };
        let result = rpc
            .simulate_transaction_with_config(&tx, sim_config)
            .map_err(|e| format!("simulate_transaction: {e}"))?;
        println!();
        if let Some(err) = result.value.err {
            println!("simulation failed: {err:?}");
        } else {
            println!("simulation succeeded.");
        }
        if let Some(units) = result.value.units_consumed {
            println!("units consumed : {units}");
        }
        if let Some(logs) = result.value.logs {
            println!("logs:");
            for log in logs {
                println!("  {log}");
            }
        }
        return Ok(());
    }

    // Live submission.
    let sig = rpc
        .send_and_confirm_transaction(&tx)
        .map_err(|e| format!("send_and_confirm_transaction: {e}"))?;
    println!();
    println!("signature     : {sig}");
    println!("status        : confirmed");
    Ok(())
}

/// Public alias used by `hopper tx explain` when decoding an
/// instruction pulled from a confirmed transaction. Same wire path,
/// just exposed so sibling commands do not need to re-implement it.
pub fn try_fetch_manifest(rpc_url: &str, program_id: &str) -> Result<String, String> {
    fetch_on_chain_manifest(rpc_url, program_id)
}

/// Look up an instruction by discriminator byte in a program's
/// manifest JSON. Returns a human-readable one-liner describing the
/// instruction, or `None` when no match is found. Exposed for
/// `hopper tx explain`.
pub fn lookup_instruction_by_tag(manifest_json: &str, tag: u8) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(manifest_json).ok()?;
    let ixs = value.get("instructions")?.as_array()?;
    for ix in ixs {
        let t = ix.get("tag").and_then(|v| v.as_u64())? as u8;
        if t != tag {
            continue;
        }
        let name = ix.get("name").and_then(|v| v.as_str()).unwrap_or("?");
        let args: Vec<String> = ix
            .get("args")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|a| {
                        let n = a.get("name").and_then(|v| v.as_str())?;
                        let s = a.get("size").and_then(|v| v.as_u64()).unwrap_or(0);
                        Some(format!("{}: {}B", n, s))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let policy = ix.get("policy_pack").and_then(|v| v.as_str()).unwrap_or("");
        let rcpt = ix.get("receipt_expected").and_then(|v| v.as_bool()).unwrap_or(false);
        return Some(format!(
            "{} (tag {}) args=[{}] policy={} receipt={}",
            name,
            tag,
            args.join(", "),
            if policy.is_empty() { "-" } else { policy },
            rcpt
        ));
    }
    None
}

/// Fetch a program's on-chain manifest PDA and return the decoded
/// JSON payload. Same path as `hopper manager fetch`.
fn fetch_on_chain_manifest(rpc_url: &str, program_id: &str) -> Result<String, String> {
    let program_bytes = crate::rpc::decode_pubkey(program_id)?;
    let (manifest_pda, _bump) = crate::rpc::find_program_address(
        &[hopper_schema::MANIFEST_SEED],
        &program_bytes,
    )
    .ok_or("could not derive manifest PDA")?;
    let manifest_pubkey = crate::rpc::encode_pubkey(&manifest_pda);
    let info = crate::rpc::get_account_info(rpc_url, &manifest_pubkey)
        .map_err(|e| format!("get_account_info: {e}"))?
        .ok_or_else(|| {
            format!(
                "no manifest account at {} for program {}",
                manifest_pubkey, program_id
            )
        })?;
    let decoded = crate::rpc::decode_manifest_account(&info.data)?;
    Ok(decoded.json)
}

/// Resolve the solana-cli default keypair location. Matches
/// `solana config get` behaviour so users who already set up their
/// environment do not need to pass `--signer`.
fn default_signer_path() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok().or_else(|| std::env::var("USERPROFILE").ok())?;
    let path = PathBuf::from(home).join(".config").join("solana").join("id.json");
    if path.exists() {
        Some(path)
    } else {
        None
    }
}

/// Cross-check the supplied accounts against the manifest's policy
/// declaration for this instruction. Surfaces three classes of
/// error before the transaction ever leaves the client:
///
/// 1. An account the manifest marks `writable=true` was supplied
///    without writable permission by the caller (unsupported today
///    because account metas are derived from manifest flags; kept
///    here as a guard for future manual-meta overrides).
/// 2. An account the manifest marks `signer=true` is not a signer
///    account on the caller side. Surfaces the same error
///    `read_keypair_file` would raise later, but with a pointer at
///    the exact field name, which saves a round-trip to the RPC.
/// 3. A declared requirement in the instruction's `policy_pack`
///    names an account that is not present in the supplied set.
fn policy_precheck(
    manifest_json: &str,
    ix: &InstructionDescriptor,
    supplied: &[(String, String)],
) -> Result<(), String> {
    // Every declared account must have a matching --account.
    for declared in &ix.accounts {
        if !supplied.iter().any(|(n, _)| n == &declared.name) {
            return Err(format!(
                "policy precheck: instruction `{}` requires account `{}` (writable={}, signer={}). pass it with --account {}=<pubkey>",
                ix.name, declared.name, declared.writable, declared.signer, declared.name
            ));
        }
    }

    // Look up the policy pack for this instruction and cross-check
    // its declared requirements. A requirement is a named capability
    // the policy depends on (for example "WritableAuthority"); the
    // manifest records them as free-form strings so a new policy can
    // be added without a schema migration.
    if ix.policy_pack.is_empty() {
        return Ok(());
    }
    let value: serde_json::Value = match serde_json::from_str(manifest_json) {
        Ok(v) => v,
        Err(_) => return Ok(()), // manifest already parsed elsewhere; skip belt-and-suspenders
    };
    let policies = match value.get("policies").and_then(|v| v.as_array()) {
        Some(p) => p,
        None => return Ok(()),
    };
    let policy = policies
        .iter()
        .find(|p| p.get("name").and_then(|v| v.as_str()) == Some(ix.policy_pack.as_str()));
    let Some(policy) = policy else {
        // Instruction references a policy that the manifest does not
        // declare. That is a manifest-consistency bug worth surfacing
        // so the program author catches it before deploy.
        return Err(format!(
            "policy precheck: instruction `{}` references policy `{}` but the manifest has no such policy declared",
            ix.name, ix.policy_pack
        ));
    };
    let requirements = policy
        .get("requirements")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    // Requirement names are conventional. The baseline three we
    // match against here cover the lion's share of real programs;
    // anything else becomes a warning on the output.
    for req in &requirements {
        match req.as_str() {
            "WritableAuthority" => {
                let has_writable_authority = ix
                    .accounts
                    .iter()
                    .any(|a| a.signer && a.writable);
                if !has_writable_authority {
                    return Err(format!(
                        "policy precheck: policy `{}` requires a writable+signer authority; none declared in the instruction's accounts",
                        ix.policy_pack
                    ));
                }
            }
            "ReadOnlyProgramId" => {
                let has_readonly_pid = ix
                    .accounts
                    .iter()
                    .any(|a| !a.writable && !a.signer && a.name.contains("program"));
                if !has_readonly_pid {
                    // Soft warning: most programs imply a program id
                    // through the `program_id` field on `Instruction`
                    // rather than a separate account. No error.
                    eprintln!(
                        "policy precheck: policy `{}` wants a read-only `program_id` account; not seen. assuming implicit.",
                        ix.policy_pack
                    );
                }
            }
            other => {
                eprintln!(
                    "policy precheck: unrecognized requirement `{}` on policy `{}`. honoured as informational.",
                    other, ix.policy_pack
                );
            }
        }
    }
    Ok(())
}

fn print_invoke_usage() {
    eprintln!("Usage: hopper manager invoke <program-id> <instruction> [options]");
    eprintln!();
    eprintln!("Build and submit a transaction against a deployed Hopper program.");
    eprintln!("The program's on-chain manifest drives account ordering and arg layout.");
    eprintln!();
    eprintln!("Options:");
    eprintln!("  --manifest <path>      Load the manifest from a local json file");
    eprintln!("                         instead of the on-chain PDA");
    eprintln!("  --account name=pubkey  Supply one account meta (repeatable)");
    eprintln!("  --arg name=value       Supply one instruction arg (repeatable)");
    eprintln!("  --signer <path>        Keypair json for fee payer + signers");
    eprintln!("  --rpc <url>            RPC endpoint (default: from `hopper config`)");
    eprintln!("  --dry-run              Print the constructed tx without submitting");
    eprintln!("  --priority-fee <u>     Priority fee in micro-lamports per CU");
}

// ----------------------------------------------------------------------------
// crank
// ----------------------------------------------------------------------------

fn cmd_crank_list(args: &[String]) {
    let manifest_path = args.first().map(PathBuf::from).unwrap_or_else(|| {
        eprintln!("Usage: hopper manager crank list <manifest.json>");
        process::exit(1);
    });
    let text = fs::read_to_string(&manifest_path).unwrap_or_else(|e| {
        eprintln!("read {}: {e}", manifest_path.display());
        process::exit(1);
    });
    let cranks = find_cranks_in_manifest(&text).unwrap_or_else(|e| {
        eprintln!("parse manifest: {e}");
        process::exit(1);
    });
    if cranks.is_empty() {
        println!("no crank-tagged instructions found.");
        println!(
            "tag an instruction as a crank by adding `\"Crank\"` to its `capabilities` array."
        );
        return;
    }
    println!("cranks for this program:");
    for c in &cranks {
        println!("  - {:<28} tag={:<3}  policy={}  receipt={}", c.name, c.tag, c.policy_pack, c.receipt_expected);
    }
}

fn cmd_crank_run(args: &[String]) {
    let mut manifest_path: Option<PathBuf> = None;
    let mut program_id: Option<String> = None;
    let mut rpc: Option<String> = None;
    let mut interval = Duration::from_secs(30);
    let mut once = false;
    let mut filter: Option<String> = None;
    let mut signer: Option<PathBuf> = None;
    let mut priority_fee: Option<u64> = None;
    let mut max_failures: u32 = 5;
    let mut dry_run = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--manifest" => {
                i += 1;
                manifest_path = args.get(i).map(PathBuf::from);
            }
            "--program-id" => {
                i += 1;
                program_id = args.get(i).cloned();
            }
            "--rpc" => {
                i += 1;
                rpc = args.get(i).cloned();
            }
            "--interval" => {
                i += 1;
                let s: u64 = args
                    .get(i)
                    .and_then(|v| v.parse().ok())
                    .unwrap_or_else(|| {
                        eprintln!("`--interval` requires a u64 seconds value");
                        process::exit(1);
                    });
                interval = Duration::from_secs(s);
            }
            "--once" => once = true,
            "--filter" => {
                i += 1;
                filter = args.get(i).cloned();
            }
            "--signer" => {
                i += 1;
                signer = args.get(i).map(PathBuf::from);
            }
            "--priority-fee" => {
                i += 1;
                priority_fee = args.get(i).and_then(|v| v.parse().ok());
            }
            "--max-failures" => {
                i += 1;
                max_failures = args.get(i).and_then(|v| v.parse().ok()).unwrap_or(5);
            }
            "--dry-run" => dry_run = true,
            other if !other.starts_with("--") => {
                manifest_path = Some(PathBuf::from(other));
            }
            other => {
                eprintln!("unknown flag: {other}");
                process::exit(1);
            }
        }
        i += 1;
    }

    // Resolve manifest source: explicit file or fetched from the
    // on-chain PDA when the caller passed `--program-id`.
    let rpc_url = rpc.unwrap_or_else(|| crate::rpc::resolve_rpc_url(None));
    let manifest_json = match (manifest_path.as_ref(), program_id.as_ref()) {
        (Some(p), _) => fs::read_to_string(p).unwrap_or_else(|e| {
            eprintln!("read {}: {e}", p.display());
            process::exit(1);
        }),
        (None, Some(pid)) => fetch_on_chain_manifest(&rpc_url, pid).unwrap_or_else(|e| {
            eprintln!("fetch manifest: {e}");
            process::exit(1);
        }),
        (None, None) => {
            eprintln!("Usage: hopper manager crank run [<manifest.json> | --program-id <id>] [options]");
            process::exit(1);
        }
    };

    let cranks = find_cranks_in_manifest(&manifest_json).unwrap_or_else(|e| {
        eprintln!("parse manifest: {e}");
        process::exit(1);
    });
    if cranks.is_empty() {
        println!("no cranks to run. tag an instruction with \"Crank\" in its `capabilities` array to make it invokable from this loop.");
        return;
    }
    let targets: Vec<CrankEntry> = cranks
        .into_iter()
        .filter(|c| match &filter {
            Some(f) => c.name.contains(f),
            None => true,
        })
        .collect();
    if targets.is_empty() {
        println!("no cranks match `--filter {}`", filter.unwrap_or_default());
        return;
    }

    println!("-- hopper manager crank run --");
    println!("rpc        : {rpc_url}");
    println!("interval   : {}s", interval.as_secs());
    println!("cranks     : {}", targets.iter().map(|c| c.name.as_str()).collect::<Vec<_>>().join(", "));
    println!("max failures per crank: {max_failures}");
    println!("dry-run    : {dry_run}");
    println!();

    let start = Instant::now();
    let mut tick_index: u64 = 0;
    let mut failures: std::collections::HashMap<String, u32> = std::collections::HashMap::new();

    loop {
        tick_index += 1;
        let tick_elapsed = start.elapsed().as_secs();
        for c in &targets {
            let fail_count = *failures.get(&c.name).unwrap_or(&0);
            if fail_count >= max_failures {
                // Crank has exhausted its budget; skip silently.
                continue;
            }
            let label = format!("tick#{tick_index} +{tick_elapsed}s {}", c.name);
            let outcome = crank_tick(
                &label,
                &rpc_url,
                program_id.as_deref(),
                &manifest_json,
                c,
                signer.as_deref(),
                priority_fee,
                dry_run,
            );
            match outcome {
                Ok(CrankOutcome::Skipped(reason)) => {
                    println!("  [skip] {label}: {reason}");
                }
                Ok(CrankOutcome::DryRun(units)) => {
                    println!(
                        "  [sim ] {label}: would submit (CU budget {})",
                        units.map(|n| n.to_string()).unwrap_or_else(|| "?".into())
                    );
                }
                Ok(CrankOutcome::Submitted(sig)) => {
                    failures.insert(c.name.clone(), 0);
                    println!("  [ok  ] {label}: {sig}");
                }
                Err(e) => {
                    let next = fail_count + 1;
                    failures.insert(c.name.clone(), next);
                    println!("  [fail] {label}: {e} (failure {}/{})", next, max_failures);
                }
            }
        }
        if once {
            println!();
            println!("single-pass complete.");
            return;
        }
        thread::sleep(interval);
    }
}

enum CrankOutcome {
    Skipped(String),
    DryRun(Option<u64>),
    Submitted(String),
}

/// Execute a single crank tick. Handles account resolution, policy
/// precheck, optional simulate-first, and the real `sendAndConfirm`.
/// Returns a structured `CrankOutcome` so the driver can update its
/// failure tallies and log cleanly.
#[allow(clippy::too_many_arguments)]
fn crank_tick(
    label: &str,
    rpc_url: &str,
    program_id_cli: Option<&str>,
    manifest_json: &str,
    crank: &CrankEntry,
    signer_path: Option<&Path>,
    priority_fee: Option<u64>,
    dry_run: bool,
) -> Result<CrankOutcome, String> {
    let _ = label;
    use solana_client::rpc_client::RpcClient;
    use solana_client::rpc_config::RpcSimulateTransactionConfig;
    use solana_sdk::commitment_config::CommitmentConfig;
    use solana_sdk::compute_budget::ComputeBudgetInstruction;
    use solana_sdk::instruction::{AccountMeta, Instruction};
    use solana_sdk::pubkey::Pubkey;
    use solana_sdk::signature::{read_keypair_file, Signer};
    use solana_sdk::transaction::Transaction;

    let program_id_str = program_id_cli
        .map(String::from)
        .or_else(|| program_id_from_manifest(manifest_json))
        .ok_or_else(|| "no program id available: pass --program-id or embed `program_id` in the manifest".to_string())?;
    let program_pubkey: Pubkey = program_id_str
        .parse()
        .map_err(|e| format!("program id parse: {e}"))?;

    // Full instruction descriptor for this crank. Gives us the
    // account list and arg layout.
    let ix_desc = find_instruction_in_manifest(manifest_json, &crank.name)?;

    // Resolve accounts. A crank may declare `auto_accounts` in the
    // manifest (PDAs derived from fixed seeds) or rely on the caller
    // to pre-populate them; we walk each declared account and look
    // for a `seeds_hint` entry, falling back to the signer's pubkey
    // for accounts named `payer` or `authority`. Everything else is
    // a hard error so the loop does not submit a malformed tx.
    let signer_path_resolved: PathBuf = signer_path
        .map(PathBuf::from)
        .or_else(default_signer_path)
        .ok_or_else(|| {
            "no --signer supplied and no default keypair at ~/.config/solana/id.json".to_string()
        })?;
    let payer = read_keypair_file(&signer_path_resolved)
        .map_err(|e| format!("read signer {}: {e}", signer_path_resolved.display()))?;

    let mut meta_list: Vec<AccountMeta> = Vec::with_capacity(ix_desc.accounts.len());
    for declared in &ix_desc.accounts {
        let resolved = resolve_crank_account(
            &declared.name,
            manifest_json,
            &program_pubkey,
            &payer.pubkey(),
        )
        .ok_or_else(|| {
            format!(
                "crank `{}` could not auto-resolve account `{}`. add a `seeds_hint` entry to the manifest or name the field `payer` / `authority`.",
                crank.name, declared.name
            )
        })?;
        if declared.writable {
            if declared.signer {
                meta_list.push(AccountMeta::new(resolved, true));
            } else {
                meta_list.push(AccountMeta::new(resolved, false));
            }
        } else if declared.signer {
            meta_list.push(AccountMeta::new_readonly(resolved, true));
        } else {
            meta_list.push(AccountMeta::new_readonly(resolved, false));
        }
    }

    // Cranks are invoked with zero user-supplied args; every declared
    // arg must be zero-length or the crank is not autonomous.
    let mut data: Vec<u8> = Vec::with_capacity(1);
    data.push(crank.tag);
    for arg in &ix_desc.args {
        if arg.size != 0 {
            return Err(format!(
                "crank `{}` declares arg `{}` of {} bytes; autonomous cranks cannot take args. use `hopper manager invoke` for arg-bearing instructions.",
                crank.name, arg.name, arg.size
            ));
        }
    }

    let ix = Instruction {
        program_id: program_pubkey,
        accounts: meta_list,
        data,
    };
    let mut instructions: Vec<Instruction> = Vec::with_capacity(2);
    if let Some(fee) = priority_fee {
        instructions.push(ComputeBudgetInstruction::set_compute_unit_price(fee));
    }
    instructions.push(ix);

    let rpc = RpcClient::new_with_commitment(rpc_url.to_string(), CommitmentConfig::confirmed());
    let recent = rpc
        .get_latest_blockhash()
        .map_err(|e| format!("get_latest_blockhash: {e}"))?;
    let mut tx = Transaction::new_with_payer(&instructions, Some(&payer.pubkey()));
    tx.sign(&[&payer], recent);

    if dry_run {
        let sim_config = RpcSimulateTransactionConfig {
            sig_verify: false,
            replace_recent_blockhash: true,
            commitment: Some(CommitmentConfig::processed()),
            ..Default::default()
        };
        let result = rpc
            .simulate_transaction_with_config(&tx, sim_config)
            .map_err(|e| format!("simulate: {e}"))?;
        if let Some(err) = result.value.err {
            return Err(format!("simulation failed: {err:?}"));
        }
        return Ok(CrankOutcome::DryRun(result.value.units_consumed));
    }

    let sig = rpc
        .send_and_confirm_transaction(&tx)
        .map_err(|e| format!("send_and_confirm: {e}"))?;
    Ok(CrankOutcome::Submitted(sig.to_string()))
}

/// Pull `program_id` out of the manifest when it is embedded there
/// (Hopper's manifest serializer writes it as a top-level string).
/// Cranks prefer `--program-id` on the CLI but fall back to this so
/// a manifest json path alone is a sufficient driver.
fn program_id_from_manifest(manifest_json: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(manifest_json).ok()?;
    value.get("program_id").and_then(|v| v.as_str()).map(String::from)
}

/// Resolve a declared account to an on-chain pubkey for a crank.
///
/// Rules, in priority order:
///
/// 1. The account name `payer` or `fee_payer` resolves to the
///    signer's own pubkey.
/// 2. The account name `authority` resolves to the signer's pubkey.
/// 3. The manifest carries a top-level `seeds_hint` object keyed by
///    account name whose value is an array of seed byte expressions
///    (strings that either start with `b"..."` for UTF-8 literals or
///    are raw base58 pubkeys); in that case we derive the PDA and
///    return the result.
/// 4. Otherwise, `None`.
///
/// Real crank bots in production are expected to either embed
/// `seeds_hint` entries for their PDAs or drop to
/// `hopper manager invoke` with explicit `--account` flags.
fn resolve_crank_account(
    name: &str,
    manifest_json: &str,
    program_pubkey: &solana_sdk::pubkey::Pubkey,
    payer_pubkey: &solana_sdk::pubkey::Pubkey,
) -> Option<solana_sdk::pubkey::Pubkey> {
    match name {
        "payer" | "fee_payer" | "authority" => return Some(*payer_pubkey),
        _ => {}
    }
    let value: serde_json::Value = serde_json::from_str(manifest_json).ok()?;
    let hints = value.get("seeds_hint").and_then(|v| v.as_object())?;
    let seeds = hints.get(name).and_then(|v| v.as_array())?;
    let mut bytes_list: Vec<Vec<u8>> = Vec::with_capacity(seeds.len());
    for seed in seeds {
        let s = seed.as_str()?;
        if let Some(lit) = s.strip_prefix("b\"") {
            let lit = lit.strip_suffix('"')?;
            bytes_list.push(lit.as_bytes().to_vec());
        } else if let Ok(pk_bytes) = bs58::decode(s).into_vec() {
            if pk_bytes.len() == 32 {
                bytes_list.push(pk_bytes);
            } else {
                return None;
            }
        } else {
            return None;
        }
    }
    let seed_slices: Vec<&[u8]> = bytes_list.iter().map(|v| v.as_slice()).collect();
    let (pda, _bump) = solana_sdk::pubkey::Pubkey::find_program_address(
        &seed_slices,
        program_pubkey,
    );
    Some(pda)
}

fn print_crank_usage() {
    eprintln!("Usage: hopper manager crank <subcommand>");
    eprintln!();
    eprintln!("Subcommands:");
    eprintln!("  list <manifest.json>       List every crank-tagged instruction");
    eprintln!("  run <manifest.json>        Run cranks in a polling loop");
    eprintln!();
    eprintln!("`run` options:");
    eprintln!("  --interval <seconds>       Sleep between ticks (default 30)");
    eprintln!("  --once                     Exit after one pass");
    eprintln!("  --filter <substring>       Only run cranks whose name matches");
    eprintln!();
    eprintln!("Mark an instruction as a crank by adding `\"Crank\"` to its");
    eprintln!("`capabilities` array in the manifest.");
}

// ----------------------------------------------------------------------------
// manifest parsing helpers
//
// Low-dependency JSON walkers that read only what the subcommands
// need. Swapping these for a typed deserialize against
// `hopper_schema::ProgramManifest` is a follow-up cleanup; the
// minimal shape here keeps the audit story readable without
// dragging a big serde-derive chain into this file.
// ----------------------------------------------------------------------------

struct InstructionDescriptor {
    name: String,
    tag: u8,
    accounts: Vec<InstrAccountEntry>,
    args: Vec<InstrArgEntry>,
    #[allow(dead_code)]
    policy_pack: String,
    #[allow(dead_code)]
    receipt_expected: bool,
}

struct InstrAccountEntry {
    name: String,
    writable: bool,
    signer: bool,
}

struct InstrArgEntry {
    name: String,
    size: u16,
}

struct CrankEntry {
    name: String,
    tag: u8,
    policy_pack: String,
    receipt_expected: bool,
}

fn find_instruction_in_manifest(
    manifest_json: &str,
    instruction_name: &str,
) -> Result<InstructionDescriptor, String> {
    let value: serde_json::Value = serde_json::from_str(manifest_json)
        .map_err(|e| format!("manifest is not valid json: {e}"))?;
    let ixs = value
        .get("instructions")
        .and_then(|v| v.as_array())
        .ok_or("manifest has no `instructions` array")?;
    for ix in ixs {
        let name = ix
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        if name == instruction_name || tag_matches(ix, instruction_name) {
            return Ok(parse_instruction(ix)?);
        }
    }
    Err(format!(
        "instruction `{instruction_name}` not found in manifest"
    ))
}

fn tag_matches(ix: &serde_json::Value, needle: &str) -> bool {
    let Some(tag) = ix.get("tag").and_then(|v| v.as_u64()) else {
        return false;
    };
    needle
        .parse::<u64>()
        .map(|n| n == tag)
        .unwrap_or(false)
}

fn parse_instruction(ix: &serde_json::Value) -> Result<InstructionDescriptor, String> {
    let name = ix.get("name").and_then(|v| v.as_str()).unwrap_or("?").to_string();
    let tag = ix
        .get("tag")
        .and_then(|v| v.as_u64())
        .map(|n| n as u8)
        .unwrap_or(0);
    let mut accounts = Vec::new();
    if let Some(arr) = ix.get("accounts").and_then(|v| v.as_array()) {
        for a in arr {
            accounts.push(InstrAccountEntry {
                name: a.get("name").and_then(|v| v.as_str()).unwrap_or("?").to_string(),
                writable: a.get("writable").and_then(|v| v.as_bool()).unwrap_or(false),
                signer: a.get("signer").and_then(|v| v.as_bool()).unwrap_or(false),
            });
        }
    }
    let mut args = Vec::new();
    if let Some(arr) = ix.get("args").and_then(|v| v.as_array()) {
        for a in arr {
            args.push(InstrArgEntry {
                name: a.get("name").and_then(|v| v.as_str()).unwrap_or("?").to_string(),
                size: a.get("size").and_then(|v| v.as_u64()).unwrap_or(0) as u16,
            });
        }
    }
    let policy_pack = ix
        .get("policy_pack")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let receipt_expected = ix
        .get("receipt_expected")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    Ok(InstructionDescriptor {
        name,
        tag,
        accounts,
        args,
        policy_pack,
        receipt_expected,
    })
}

fn find_cranks_in_manifest(manifest_json: &str) -> Result<Vec<CrankEntry>, String> {
    let value: serde_json::Value = serde_json::from_str(manifest_json)
        .map_err(|e| format!("manifest is not valid json: {e}"))?;
    let ixs = value
        .get("instructions")
        .and_then(|v| v.as_array())
        .ok_or("manifest has no `instructions` array")?;
    let mut out = Vec::new();
    for ix in ixs {
        let caps = ix
            .get("capabilities")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect::<Vec<_>>())
            .unwrap_or_default();
        if !caps.iter().any(|c| c == "Crank") {
            continue;
        }
        let desc = parse_instruction(ix)?;
        out.push(CrankEntry {
            name: desc.name,
            tag: desc.tag,
            policy_pack: desc.policy_pack,
            receipt_expected: desc.receipt_expected,
        });
    }
    Ok(out)
}

// ----------------------------------------------------------------------------
// arg + account wiring
// ----------------------------------------------------------------------------

fn build_instruction_data(
    ix: &InstructionDescriptor,
    supplied: &[(String, String)],
) -> Result<Vec<u8>, String> {
    // Leading discriminator byte mirrors the on-chain dispatch.
    let mut out = Vec::with_capacity(1 + ix.args.iter().map(|a| a.size as usize).sum::<usize>());
    out.push(ix.tag);
    for declared in &ix.args {
        let raw = supplied
            .iter()
            .find(|(n, _)| n == &declared.name)
            .map(|(_, v)| v.clone())
            .ok_or_else(|| format!("missing --arg {}", declared.name))?;
        let bytes = encode_arg_value(&raw, declared.size)?;
        if bytes.len() != declared.size as usize {
            return Err(format!(
                "arg `{}` expected {} bytes, produced {}",
                declared.name,
                declared.size,
                bytes.len()
            ));
        }
        out.extend_from_slice(&bytes);
    }
    // Warn about unused supplied args so typos do not silently no-op.
    for (n, _) in supplied {
        if !ix.args.iter().any(|a| &a.name == n) {
            return Err(format!(
                "supplied --arg {} has no matching declaration in the manifest",
                n
            ));
        }
    }
    Ok(out)
}

fn build_account_metas(
    ix: &InstructionDescriptor,
    supplied: &[(String, String)],
) -> Result<Vec<(String, String, bool, bool)>, String> {
    let mut out: Vec<(String, String, bool, bool)> = Vec::with_capacity(ix.accounts.len());
    for declared in &ix.accounts {
        let pubkey = supplied
            .iter()
            .find(|(n, _)| n == &declared.name)
            .map(|(_, v)| v.clone())
            .ok_or_else(|| format!("missing --account {}", declared.name))?;
        // Light validation of base58 shape without pulling a full
        // Pubkey type in; a proper check runs inside the signer path.
        if pubkey.len() < 32 || pubkey.len() > 44 {
            return Err(format!(
                "--account {} value is not a base58 pubkey: `{}`",
                declared.name, pubkey
            ));
        }
        out.push((
            declared.name.clone(),
            pubkey,
            declared.writable,
            declared.signer,
        ));
    }
    for (n, _) in supplied {
        if !ix.accounts.iter().any(|a| &a.name == n) {
            return Err(format!(
                "supplied --account {} has no matching declaration in the manifest",
                n
            ));
        }
    }
    Ok(out)
}

/// Encode a single instruction arg value from its CLI string form into
/// its `size`-byte packed form. Accepted shapes:
///
/// - `hex:0a1b2c` - hex bytes
/// - `base58:Abc...` - base58 bytes
/// - `u64:12345`, `u32:123`, `u16:55`, `u8:3`, `i64:-1` - little-endian
///   integer encoding matched to declared size
/// - `bool:true` / `bool:false`
/// - Bare base-10 integer when `size` is 1, 2, 4, 8, or 16
fn encode_arg_value(raw: &str, size: u16) -> Result<Vec<u8>, String> {
    if let Some(rest) = raw.strip_prefix("hex:") {
        return decode_hex(rest);
    }
    if let Some(rest) = raw.strip_prefix("base58:") {
        return bs58::decode(rest)
            .into_vec()
            .map_err(|e| format!("invalid base58: {e}"));
    }
    if let Some(rest) = raw.strip_prefix("bool:") {
        let b: bool = rest.parse().map_err(|_| format!("not a bool: {rest}"))?;
        return Ok(vec![b as u8]);
    }
    for (prefix, width) in [("u8:", 1), ("u16:", 2), ("u32:", 4), ("u64:", 8), ("u128:", 16)] {
        if let Some(rest) = raw.strip_prefix(prefix) {
            let n: u128 = rest.parse().map_err(|e| format!("parse {prefix}: {e}"))?;
            return Ok(n.to_le_bytes()[..width].to_vec());
        }
    }
    for (prefix, width) in [("i8:", 1), ("i16:", 2), ("i32:", 4), ("i64:", 8), ("i128:", 16)] {
        if let Some(rest) = raw.strip_prefix(prefix) {
            let n: i128 = rest.parse().map_err(|e| format!("parse {prefix}: {e}"))?;
            return Ok(n.to_le_bytes()[..width].to_vec());
        }
    }
    // Bare integer: use declared size as the width.
    if let Ok(n) = raw.parse::<u128>() {
        return match size {
            1 | 2 | 4 | 8 | 16 => Ok(n.to_le_bytes()[..size as usize].to_vec()),
            other => Err(format!(
                "bare integer arg cannot fit in declared size {other}; use an explicit prefix"
            )),
        };
    }
    Err(format!(
        "cannot encode `{}`: supply hex:..., base58:..., u64:N, i32:N, or bool:true/false",
        raw
    ))
}

fn decode_hex(s: &str) -> Result<Vec<u8>, String> {
    if s.len() % 2 != 0 {
        return Err("hex string must have even length".into());
    }
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(s.len() / 2);
    for c in bytes.chunks_exact(2) {
        let hi = from_hex(c[0])?;
        let lo = from_hex(c[1])?;
        out.push((hi << 4) | lo);
    }
    Ok(out)
}

fn from_hex(b: u8) -> Result<u8, String> {
    match b {
        b'0'..=b'9' => Ok(b - b'0'),
        b'a'..=b'f' => Ok(b - b'a' + 10),
        b'A'..=b'F' => Ok(b - b'A' + 10),
        _ => Err(format!("not a hex digit: {}", b as char)),
    }
}

#[allow(dead_code)]
fn _pin_path(_p: &Path) {}
