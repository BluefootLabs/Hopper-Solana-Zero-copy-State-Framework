//! `hopper tx explain <signature>` / `hopper explain <signature>` -
//! human-readable on-chain tx trace.
//!
//! Fetches a confirmed transaction from the cluster, enumerates every
//! top-level instruction, and tries to decode each one against the
//! target program's on-chain Hopper manifest. For every instruction we
//! recognize, we print:
//!
//! - The target program id
//! - The instruction discriminator byte
//! - The matched Hopper instruction name (from the on-chain manifest)
//! - The account slots the instruction touched
//!
//! Unrecognized programs fall back to a terse line rather than masking
//! the tx. The point is to make reading a transaction as high-signal as
//! reading source.
//!
//! We talk to the RPC over raw JSON (`ureq` + `serde_json`) rather than
//! the typed `solana-client` decoder: devnet/mainnet periodically add
//! response fields (e.g. `costUnits`) and version shapes that a pinned
//! SDK struct rejects with an opaque deserialize error. Parsing the
//! `jsonParsed` response directly keeps `explain` working across RPC
//! upgrades and versioned (v0) transactions.

use std::collections::HashMap;
use std::process;

use serde_json::Value;

pub fn cmd_tx_explain(args: &[String]) {
    if args.is_empty() || matches!(args[0].as_str(), "--help" | "-h") {
        print_usage();
        return;
    }
    let mut signature: Option<String> = None;
    let mut rpc: Option<String> = None;
    let mut show_raw_logs = false;
    let mut manifest_path: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--rpc" => {
                i += 1;
                rpc = args.get(i).cloned();
            }
            "--manifest" => {
                i += 1;
                manifest_path = args.get(i).cloned();
            }
            "--raw-logs" => show_raw_logs = true,
            other if !other.starts_with("--") && signature.is_none() => {
                signature = Some(other.to_string());
            }
            other => {
                eprintln!("unknown arg: {other}");
                print_usage();
                process::exit(1);
            }
        }
        i += 1;
    }
    let signature = signature.unwrap_or_else(|| {
        eprintln!("missing <signature> arg");
        print_usage();
        process::exit(1);
    });
    // A `--manifest <file>` is an explicit decode source: it maps disc
    // bytes to instruction names even when the program has not published
    // its manifest on chain.
    let local_manifest = manifest_path.and_then(|p| match std::fs::read_to_string(&p) {
        Ok(s) => Some(s),
        Err(e) => {
            eprintln!("warning: could not read --manifest {p}: {e}");
            None
        }
    });
    let rpc_url = rpc.unwrap_or_else(|| crate::rpc::resolve_rpc_url(None));
    if let Err(e) = run_explain(
        &rpc_url,
        &signature,
        show_raw_logs,
        local_manifest.as_deref(),
    ) {
        eprintln!("hopper tx explain failed: {e}");
        process::exit(1);
    }
}

fn print_usage() {
    eprintln!("Usage: hopper tx explain <signature> [--rpc <url>] [--raw-logs]");
    eprintln!();
    eprintln!("Fetch a confirmed transaction by signature and decode every");
    eprintln!("instruction against the target Hopper program's on-chain manifest.");
    eprintln!();
    eprintln!("Options:");
    eprintln!("  --rpc <url>        RPC endpoint (default from config / env)");
    eprintln!("  --manifest <file> Local manifest used to map disc bytes to instruction");
    eprintln!("                    names when the program has no on-chain manifest");
    eprintln!("  --raw-logs         Print the full Program-log stream verbatim");
}

/// Issue a JSON-RPC `getTransaction` and return the parsed `result`
/// object. We request `jsonParsed` + `maxSupportedTransactionVersion: 0`
/// so versioned (v0) deploy/upgrade transactions are returned rather
/// than erroring.
fn rpc_get_transaction(rpc_url: &str, signature: &str) -> Result<Value, String> {
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "getTransaction",
        "params": [
            signature,
            {
                "encoding": "jsonParsed",
                "commitment": "confirmed",
                "maxSupportedTransactionVersion": 0
            }
        ]
    });
    let resp = ureq::post(rpc_url)
        .set("Content-Type", "application/json")
        .send_string(&body.to_string())
        .map_err(|e| format!("RPC request failed: {e}"))?;
    let text = resp
        .into_string()
        .map_err(|e| format!("failed to read RPC response: {e}"))?;
    let parsed: Value =
        serde_json::from_str(&text).map_err(|e| format!("invalid JSON from RPC: {e}"))?;
    if let Some(err) = parsed.get("error") {
        let msg = err
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("unknown RPC error");
        return Err(format!("RPC error: {msg}"));
    }
    let result = parsed.get("result").cloned().unwrap_or(Value::Null);
    if result.is_null() {
        return Err(
            "transaction not found (null result). It may not be confirmed yet, or the RPC \
             does not retain it. Try a different --rpc endpoint or wait for finalization."
                .to_string(),
        );
    }
    Ok(result)
}

fn run_explain(
    rpc_url: &str,
    signature: &str,
    show_raw_logs: bool,
    local_manifest: Option<&str>,
) -> Result<(), String> {
    // Validate the signature is base58 before we spend a round-trip.
    if bs58::decode(signature).into_vec().is_err() {
        return Err(format!("invalid base58 signature: {signature}"));
    }
    let result = rpc_get_transaction(rpc_url, signature)?;

    println!("-- hopper tx explain --");
    println!("signature : {signature}");
    if let Some(slot) = result.get("slot").and_then(Value::as_u64) {
        println!("slot      : {slot}");
    }
    println!(
        "block time: {}",
        result
            .get("blockTime")
            .and_then(Value::as_i64)
            .map(|t| t.to_string())
            .unwrap_or_else(|| "-".into())
    );

    // Meta: success, fee, compute.
    let meta = result.get("meta");
    if let Some(meta) = meta {
        let status = if meta.get("err").map(Value::is_null).unwrap_or(true) {
            "success"
        } else {
            "failed"
        };
        println!("status    : {status}");
        if let Some(fee) = meta.get("fee").and_then(Value::as_u64) {
            println!("fee       : {fee} lamports");
        }
        if let Some(cu) = meta.get("computeUnitsConsumed").and_then(Value::as_u64) {
            println!("compute   : {cu} CU");
        }
        if let Some(err) = meta.get("err") {
            if !err.is_null() {
                println!("error     : {err}");
            }
        }
    }
    println!();

    // Enumerate top-level instructions from the parsed message.
    let instructions = result
        .get("transaction")
        .and_then(|t| t.get("message"))
        .and_then(|m| m.get("instructions"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    if instructions.is_empty() {
        println!("(no instructions decoded; message may be raw-encoded)");
    }

    // Cache program_id -> manifest JSON so a repeated-program tx (a
    // keeper batch, say) only fetches each manifest once.
    let mut manifest_cache: HashMap<String, Option<String>> = HashMap::new();

    for (i, ix) in instructions.iter().enumerate() {
        println!("[instruction {i}]");
        // RPC-parsed (system/SPL) instructions carry a `parsed` field.
        if let Some(parsed) = ix.get("parsed") {
            let program = ix
                .get("program")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            println!("  program: {program} (parsed by RPC)");
            println!("  kind   : {parsed}");
            continue;
        }
        // Otherwise it is a partially-decoded instruction with a
        // program id, base58 data, and account list.
        let program_id = ix
            .get("programId")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let data_b58 = ix.get("data").and_then(Value::as_str).unwrap_or_default();
        let accounts: Vec<String> = ix
            .get("accounts")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        explain_partial(
            &program_id,
            &accounts,
            data_b58,
            rpc_url,
            &mut manifest_cache,
            local_manifest,
        );
    }

    if show_raw_logs {
        if let Some(logs) = meta
            .and_then(|m| m.get("logMessages"))
            .and_then(Value::as_array)
        {
            println!();
            println!("logs:");
            for log in logs {
                if let Some(s) = log.as_str() {
                    println!("  {s}");
                }
            }
        }
    }

    Ok(())
}

fn explain_partial(
    program_id: &str,
    accounts: &[String],
    data_b58: &str,
    rpc_url: &str,
    manifest_cache: &mut HashMap<String, Option<String>>,
    local_manifest: Option<&str>,
) {
    println!("  program   : {program_id}");
    // Prefer the on-chain manifest; fall back to an operator-supplied
    // `--manifest` file so decode still works for programs that have
    // not published their manifest yet.
    let manifest = manifest_cache
        .entry(program_id.to_string())
        .or_insert_with(|| {
            super::manager_invoke::try_fetch_manifest(rpc_url, program_id)
                .ok()
                .or_else(|| local_manifest.map(str::to_string))
        });
    let data_bytes = match bs58::decode(data_b58).into_vec() {
        Ok(b) => b,
        Err(e) => {
            println!("  data      : <base58 decode failed: {e}>");
            return;
        }
    };
    if data_bytes.is_empty() {
        println!("  data      : (empty)");
        return;
    }
    let tag = data_bytes[0];
    println!("  disc byte : 0x{tag:02x}");
    println!("  data len  : {} bytes", data_bytes.len());

    if let Some(manifest_json) = manifest {
        match super::manager_invoke::lookup_instruction_by_tag(manifest_json, tag) {
            Some(ix_line) => println!("  matched   : {ix_line}"),
            None => println!("  matched   : (no Hopper instruction with disc 0x{tag:02x})"),
        }
    } else {
        println!("  manifest  : (no Hopper manifest on chain; skipping decode)");
    }
    println!("  accounts  : {} slots", accounts.len());
    for (i, a) in accounts.iter().enumerate() {
        println!("    [{i}] {a}");
    }
}
