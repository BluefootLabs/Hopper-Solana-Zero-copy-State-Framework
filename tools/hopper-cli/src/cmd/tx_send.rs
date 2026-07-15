//! `hopper tx send` — fire one arbitrary instruction. No Node required.
//!
//! Every Solana developer has hit this wall: you want to poke a single
//! instruction at a program — a discriminator byte, a couple of
//! accounts — and the official CLI has no generic instruction sender,
//! so out comes a scratch TypeScript file and a `node_modules` tree.
//! `hopper tx send` is that missing primitive in pure Rust, riding the
//! same signed-send stack `hopper publish-idl` already battle-tests:
//!
//! ```text
//! hopper tx send --program <pubkey> \
//!     --account payer:sw --account <vault>:w --account 11111111111111111111111111111111 \
//!     --data 015a00000000000000 \
//!     --keypair ~/.config/solana/id.json --rpc https://api.devnet.solana.com
//! ```
//!
//! - `--account <pubkey>[:flags]` is ORDERED (repeat once per slot);
//!   flags are `s` (signer) and/or `w` (writable). The literal pubkey
//!   `payer` resolves to the fee payer, so the common
//!   payer-is-the-authority shape needs no copy-paste.
//! - Every `s` slot must be covered by the fee payer or a `--signer`
//!   keypair; the send is refused BEFORE the RPC round-trip otherwise.
//! - `--data <hex>` is the raw instruction data (optional `0x` prefix;
//!   empty/omitted sends zero bytes).
//! - `--allow-failure` skips preflight simulation and lands the
//!   transaction even when the program will refuse it, then reports the
//!   on-chain error as data. This is how you put a *refusal* on the
//!   record: a strict-writes violation (`Custom(0xD0__)`) only becomes
//!   a citable signature if the RPC is not allowed to reject it first.
//! - `--dry-run` prints the exact instruction plan and signer coverage
//!   without touching the network — same preview discipline as
//!   `publish-idl`.
//!
//! After confirmation the command fetches the transaction and reports
//! the measured compute units, because a send you cannot budget from
//! is half a tool.

use std::process;

use solana_client::rpc_client::RpcClient;
use solana_commitment_config::CommitmentConfig;
use solana_instruction::{AccountMeta, Instruction};
use solana_keypair::{read_keypair_file, Keypair};
use solana_pubkey::Pubkey;
use solana_signer::Signer as _;
use solana_transaction::Transaction;

/// One parsed `--account` spec: pubkey (or the `payer` placeholder),
/// signer flag, writable flag.
#[derive(Debug, PartialEq, Eq, Clone)]
struct AccountSpec {
    /// `None` = the `payer` placeholder, resolved after keypair load.
    pubkey: Option<Pubkey>,
    signer: bool,
    writable: bool,
}

/// Parse `<pubkey>[:flags]` where flags ⊆ {s, w}. The literal `payer`
/// stands for the fee payer's pubkey.
fn parse_account_spec(spec: &str) -> Result<AccountSpec, String> {
    let (addr, flags) = match spec.split_once(':') {
        Some((a, f)) => (a, f),
        None => (spec, ""),
    };
    let mut signer = false;
    let mut writable = false;
    for c in flags.chars() {
        match c {
            's' => signer = true,
            'w' => writable = true,
            other => {
                return Err(format!(
                    "unknown account flag '{other}' in --account {spec} (use s and/or w, \
                     e.g. {addr}:sw)"
                ))
            }
        }
    }
    let pubkey = if addr == "payer" {
        None
    } else {
        Some(
            addr.parse::<Pubkey>()
                .map_err(|e| format!("invalid pubkey in --account {spec}: {e}"))?,
        )
    };
    Ok(AccountSpec {
        pubkey,
        signer,
        writable,
    })
}

/// Parse hex instruction data (optional `0x` prefix, whitespace-free).
fn parse_hex_data(hex: &str) -> Result<Vec<u8>, String> {
    let hex = hex.strip_prefix("0x").unwrap_or(hex);
    if hex.is_empty() {
        return Ok(Vec::new());
    }
    if hex.len() % 2 != 0 {
        return Err(format!(
            "--data hex must have an even number of digits (got {})",
            hex.len()
        ));
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&hex[i..i + 2], 16)
                .map_err(|e| format!("invalid hex byte at position {i}: {e}"))
        })
        .collect()
}

/// Which provided keypair (by index into `signer_pubkeys`) covers each
/// signer-flagged slot. Pure so the refusal path is unit-testable:
/// returns the uncovered pubkeys.
fn uncovered_signers(specs: &[(Pubkey, bool)], signer_pubkeys: &[Pubkey]) -> Vec<Pubkey> {
    specs
        .iter()
        .filter(|(pk, is_signer)| *is_signer && !signer_pubkeys.contains(pk))
        .map(|(pk, _)| *pk)
        .collect()
}

fn print_usage() {
    eprintln!("Usage: hopper tx send --program <pubkey> [--data <hex>]");
    eprintln!("           --account <pubkey|payer>[:s][:w] ...   (ordered; repeat per slot)");
    eprintln!("           --keypair <path> [--signer <path>]... [--rpc <url>]");
    eprintln!("           [--compute-limit <units>] [--allow-failure] [--dry-run]");
    eprintln!();
    eprintln!("Send one instruction with explicit account metas and raw hex data,");
    eprintln!("signed locally — the generic instruction sender the stock tooling");
    eprintln!("lacks without a JS scratch script. Account flags: s = signer,");
    eprintln!("w = writable (e.g. --account HoppR...:sw). The literal `payer`");
    eprintln!("resolves to the fee payer's pubkey.");
    eprintln!();
    eprintln!("After confirmation the transaction is fetched back and its measured");
    eprintln!("compute units printed. --dry-run previews the exact plan offline.");
}

pub fn cmd_tx_send(args: &[String]) {
    if args.is_empty() || matches!(args[0].as_str(), "--help" | "-h") {
        print_usage();
        return;
    }
    let mut program: Option<String> = None;
    let mut data_hex: Option<String> = None;
    let mut account_specs: Vec<String> = Vec::new();
    let mut keypair_path: Option<String> = None;
    let mut extra_signer_paths: Vec<String> = Vec::new();
    let mut rpc: Option<String> = None;
    let mut compute_limit: Option<u32> = None;
    let mut allow_failure = false;
    let mut dry_run = false;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--program" => {
                i += 1;
                program = args.get(i).cloned();
            }
            "--data" => {
                i += 1;
                data_hex = args.get(i).cloned();
            }
            "--account" => {
                i += 1;
                match args.get(i) {
                    Some(a) => account_specs.push(a.clone()),
                    None => {
                        eprintln!("--account needs a value");
                        process::exit(1);
                    }
                }
            }
            "--keypair" | "--signer-keypair" => {
                i += 1;
                keypair_path = args.get(i).cloned();
            }
            "--signer" => {
                i += 1;
                match args.get(i) {
                    Some(p) => extra_signer_paths.push(p.clone()),
                    None => {
                        eprintln!("--signer needs a path");
                        process::exit(1);
                    }
                }
            }
            "--rpc" | "--url" => {
                i += 1;
                rpc = args.get(i).cloned();
            }
            "--compute-limit" => {
                i += 1;
                compute_limit = args.get(i).and_then(|v| v.parse().ok());
            }
            "--allow-failure" => allow_failure = true,
            "--dry-run" => dry_run = true,
            other => {
                eprintln!("unknown arg: {other}");
                print_usage();
                process::exit(1);
            }
        }
        i += 1;
    }

    if let Err(e) = run_send(
        program.as_deref(),
        data_hex.as_deref(),
        &account_specs,
        keypair_path.as_deref(),
        &extra_signer_paths,
        rpc.as_deref(),
        compute_limit,
        allow_failure,
        dry_run,
    ) {
        eprintln!("hopper tx send failed: {e}");
        process::exit(1);
    }
}

#[allow(clippy::too_many_arguments)]
fn run_send(
    program: Option<&str>,
    data_hex: Option<&str>,
    account_specs: &[String],
    keypair_path: Option<&str>,
    extra_signer_paths: &[String],
    rpc: Option<&str>,
    compute_limit: Option<u32>,
    allow_failure: bool,
    dry_run: bool,
) -> Result<(), String> {
    let program_id = program
        .ok_or("missing --program <pubkey>")?
        .parse::<Pubkey>()
        .map_err(|e| format!("invalid --program pubkey: {e}"))?;
    let data = parse_hex_data(data_hex.unwrap_or(""))?;

    // Fee payer + extra signers.
    let keypair_path = keypair_path.ok_or(
        "missing --keypair <path> (the fee payer; every send needs one, even --dry-run, \
         so the plan can resolve `payer` slots)",
    )?;
    let payer: Keypair = read_keypair_file(keypair_path)
        .map_err(|e| format!("could not read --keypair {keypair_path}: {e}"))?;
    let mut signers: Vec<Keypair> = Vec::new();
    for path in extra_signer_paths {
        signers.push(
            read_keypair_file(path).map_err(|e| format!("could not read --signer {path}: {e}"))?,
        );
    }

    // Resolve account metas (ordered), substituting the payer placeholder.
    let mut metas: Vec<AccountMeta> = Vec::new();
    let mut signer_slots: Vec<(Pubkey, bool)> = Vec::new();
    for spec_str in account_specs {
        let spec = parse_account_spec(spec_str)?;
        let pubkey = spec.pubkey.unwrap_or_else(|| payer.pubkey());
        metas.push(if spec.writable {
            if spec.signer {
                AccountMeta::new(pubkey, true)
            } else {
                AccountMeta::new(pubkey, false)
            }
        } else if spec.signer {
            AccountMeta::new_readonly(pubkey, true)
        } else {
            AccountMeta::new_readonly(pubkey, false)
        });
        signer_slots.push((pubkey, spec.signer));
    }

    // Refuse-before-send: every signer-flagged slot must be covered by a
    // provided keypair.
    let mut covered: Vec<Pubkey> = vec![payer.pubkey()];
    covered.extend(signers.iter().map(|k| k.pubkey()));
    let uncovered = uncovered_signers(&signer_slots, &covered);
    if !uncovered.is_empty() {
        return Err(format!(
            "signer-flagged account(s) not covered by any provided keypair: {}. \
             Pass the matching keypair via --signer <path>.",
            uncovered
                .iter()
                .map(|p| p.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }

    let rpc_url = rpc
        .map(str::to_string)
        .unwrap_or_else(|| crate::rpc::resolve_rpc_url(None));

    // The plan, printed for both --dry-run and the real send.
    println!("-- hopper tx send --");
    println!("rpc       : {rpc_url}");
    println!("program   : {program_id}");
    println!("data      : {} bytes{}", data.len(), {
        if data.is_empty() {
            String::new()
        } else {
            format!(
                " (0x{})",
                data.iter().map(|b| format!("{b:02x}")).collect::<String>()
            )
        }
    });
    println!("accounts  : {} slots", metas.len());
    for (i, m) in metas.iter().enumerate() {
        println!(
            "  [{i}] {} {}{}",
            m.pubkey,
            if m.is_signer { "s" } else { "-" },
            if m.is_writable { "w" } else { "-" }
        );
    }
    println!(
        "signers   : payer {}{}",
        payer.pubkey(),
        signers
            .iter()
            .map(|k| format!(", {}", k.pubkey()))
            .collect::<String>()
    );
    if let Some(cu) = compute_limit {
        println!("cu limit  : {cu}");
    }
    if allow_failure {
        println!("preflight : skipped (--allow-failure; an on-chain refusal will land)");
    }
    if dry_run {
        println!();
        println!("dry run: nothing sent.");
        return Ok(());
    }

    let mut instructions: Vec<Instruction> = Vec::new();
    if let Some(units) = compute_limit {
        instructions.push(
            solana_compute_budget_interface::ComputeBudgetInstruction::set_compute_unit_limit(
                units,
            ),
        );
    }
    instructions.push(Instruction {
        program_id,
        accounts: metas,
        data,
    });

    let client = RpcClient::new_with_commitment(rpc_url.clone(), CommitmentConfig::confirmed());
    let blockhash = client
        .get_latest_blockhash()
        .map_err(|e| format!("get_latest_blockhash failed: {e}"))?;
    let mut all_signers: Vec<&Keypair> = vec![&payer];
    all_signers.extend(signers.iter());
    let tx = Transaction::new_signed_with_payer(
        &instructions,
        Some(&payer.pubkey()),
        &all_signers,
        blockhash,
    );
    let signature = if allow_failure {
        // Preflight simulation would reject a tx the program is going to
        // refuse — but landing that refusal IS the goal here. Send raw,
        // then poll for a commitment-level status ourselves, treating
        // "confirmed with a program error" as a successful LANDING.
        use solana_client::rpc_config::RpcSendTransactionConfig;
        let sig = client
            .send_transaction_with_config(
                &tx,
                RpcSendTransactionConfig {
                    skip_preflight: true,
                    ..RpcSendTransactionConfig::default()
                },
            )
            .map_err(|e| format!("send (skip-preflight) failed: {e}"))?;
        let mut landed = false;
        for _ in 0..30 {
            std::thread::sleep(std::time::Duration::from_secs(2));
            if let Ok(resp) = client.get_signature_statuses(&[sig]) {
                if let Some(Some(status)) = resp.value.first() {
                    if status.confirmation_status.is_some() {
                        landed = true;
                        break;
                    }
                }
            }
        }
        if !landed {
            return Err(format!(
                "transaction {sig} was sent but did not reach confirmed commitment within \
                 60s; check the explorer before retrying"
            ));
        }
        sig
    } else {
        client
            .send_and_confirm_transaction(&tx)
            .map_err(|e| format!("send_and_confirm failed: {e}"))?
    };
    println!();
    println!("signature : {signature}");
    println!("confirmed : yes");

    // Fetch the measured cost back so the send is budget-legible. Best
    // effort: a lagging RPC must not turn a landed tx into an error.
    match super::tx_explain::rpc_get_transaction(&rpc_url, &signature.to_string()) {
        Ok(result) => {
            match result.get("meta").and_then(|m| m.get("err")) {
                Some(err) if !err.is_null() => {
                    // The refusal, on the record: report it as data, not
                    // as a tool failure — the send did exactly its job.
                    println!("program   : REFUSED — {err}");
                }
                _ => println!("program   : Ok"),
            }
            if let Some(cu) = result
                .get("meta")
                .and_then(|m| m.get("computeUnitsConsumed"))
                .and_then(serde_json::Value::as_u64)
            {
                println!("compute   : {cu} CU");
            }
            if let Some(fee) = result
                .get("meta")
                .and_then(|m| m.get("fee"))
                .and_then(serde_json::Value::as_u64)
            {
                println!("fee       : {fee} lamports");
            }
        }
        Err(_) => println!("compute   : (transaction landed; RPC has not indexed it yet)"),
    }
    println!();
    println!("explain it: hopper tx explain {signature} --rpc {rpc_url}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_spec_parses_flags_and_payer_placeholder() {
        let pk = "11111111111111111111111111111111";
        let parsed = parse_account_spec(&format!("{pk}:sw")).unwrap();
        assert!(parsed.signer && parsed.writable);
        assert_eq!(parsed.pubkey.unwrap().to_string(), pk);

        let plain = parse_account_spec(pk).unwrap();
        assert!(!plain.signer && !plain.writable);

        let payer = parse_account_spec("payer:sw").unwrap();
        assert!(payer.pubkey.is_none() && payer.signer && payer.writable);

        assert!(parse_account_spec(&format!("{pk}:sx")).is_err(), "x flag");
        assert!(parse_account_spec("notapubkey:w").is_err());
    }

    #[test]
    fn hex_data_parses_with_and_without_prefix() {
        assert_eq!(parse_hex_data("0x0102ff").unwrap(), vec![0x01, 0x02, 0xff]);
        assert_eq!(parse_hex_data("00").unwrap(), vec![0]);
        assert_eq!(parse_hex_data("").unwrap(), Vec::<u8>::new());
        assert!(parse_hex_data("abc").is_err(), "odd length");
        assert!(parse_hex_data("zz").is_err(), "not hex");
    }

    #[test]
    fn uncovered_signers_reports_exactly_the_missing_ones() {
        let a = Pubkey::new_unique();
        let b = Pubkey::new_unique();
        let c = Pubkey::new_unique();
        let specs = vec![(a, true), (b, false), (c, true)];
        // Only `a` covered: `c` must be reported, `b` (not a signer) not.
        assert_eq!(uncovered_signers(&specs, &[a]), vec![c]);
        assert!(uncovered_signers(&specs, &[a, c]).is_empty());
    }
}
