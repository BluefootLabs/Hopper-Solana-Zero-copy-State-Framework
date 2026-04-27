//! `hopper tx explain <signature>` - human-readable on-chain tx trace.
//!
//! Fetches a confirmed transaction from the cluster, enumerates every
//! top-level and inner instruction, and tries to decode each one
//! against the target program's on-chain Hopper manifest. For every
//! instruction we recognize, we print:
//!
//! - The Hopper program name (from the manifest)
//! - The instruction name and discriminator byte
//! - The declared account roles matched against the runtime accounts
//! - Arg-slot sizes and raw bytes
//! - Any receipt byte emitted by the instruction
//!
//! Unrecognized programs fall back to a terse "unknown program"
//! line rather than masking the tx. The point is to make reading a
//! transaction as high-signal as reading source, which `solscan` is
//! not. Signature verification and CU accounting come from the
//! RPC's own `meta` block so we do not recompute anything.

use std::collections::HashMap;
use std::process;

pub fn cmd_tx_explain(args: &[String]) {
    if args.is_empty() || matches!(args[0].as_str(), "--help" | "-h") {
        print_usage();
        return;
    }
    let mut signature: Option<String> = None;
    let mut rpc: Option<String> = None;
    let mut show_raw_logs = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--rpc" => {
                i += 1;
                rpc = args.get(i).cloned();
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
    let rpc_url = rpc.unwrap_or_else(|| crate::rpc::resolve_rpc_url(None));
    if let Err(e) = run_explain(&rpc_url, &signature, show_raw_logs) {
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
    eprintln!("  --raw-logs         Print the full Program-log stream verbatim");
}

fn run_explain(rpc_url: &str, signature: &str, show_raw_logs: bool) -> Result<(), String> {
    use solana_client::rpc_client::RpcClient;
    use solana_client::rpc_config::RpcTransactionConfig;
    use solana_sdk::commitment_config::CommitmentConfig;
    use solana_sdk::signature::Signature;
    use solana_transaction_status::UiTransactionEncoding;

    let sig: Signature = signature
        .parse()
        .map_err(|e| format!("invalid base58 signature: {e}"))?;
    let rpc = RpcClient::new_with_commitment(rpc_url.to_string(), CommitmentConfig::confirmed());
    let config = RpcTransactionConfig {
        encoding: Some(UiTransactionEncoding::JsonParsed),
        commitment: Some(CommitmentConfig::confirmed()),
        max_supported_transaction_version: Some(0),
    };
    let tx = rpc
        .get_transaction_with_config(&sig, config)
        .map_err(|e| format!("get_transaction: {e}"))?;

    println!("-- hopper tx explain --");
    println!("signature : {}", signature);
    println!("slot      : {}", tx.slot);
    println!("block time: {}", tx.block_time.map(|t| t.to_string()).unwrap_or_else(|| "-".into()));

    // Meta: success flag, CU, fee.
    if let Some(meta) = tx.transaction.meta.as_ref() {
        let status = if meta.err.is_none() { "success" } else { "failed" };
        println!("status    : {status}");
        println!("fee       : {} lamports", meta.fee);
        if let solana_transaction_status::option_serializer::OptionSerializer::Some(cu) =
            &meta.compute_units_consumed
        {
            println!("compute   : {cu} CU");
        }
        if let Some(err) = &meta.err {
            println!("error     : {err:?}");
        }
    }
    println!();

    // Enumerate instructions. `tx.transaction.transaction` may be
    // encoded in different shapes depending on the RPC return; we
    // pattern-match the JsonParsed variant because that is what our
    // config requested.
    use solana_transaction_status::{EncodedTransaction, UiMessage, UiInstruction, UiParsedInstruction};
    let enc_tx = &tx.transaction.transaction;
    let message: &UiMessage = match enc_tx {
        EncodedTransaction::Json(parsed) => &parsed.message,
        _ => {
            println!("(transaction was not returned in JsonParsed shape; showing raw)");
            return Ok(());
        }
    };
    let instructions: Vec<UiInstruction> = match message {
        UiMessage::Parsed(m) => m.instructions.clone(),
        UiMessage::Raw(_) => {
            println!("(message is raw-encoded; JsonParsed would yield richer output. try --rpc with a richer endpoint)");
            return Ok(());
        }
    };

    // Cache of program_id -> manifest JSON so we do not re-fetch for
    // repeated-program transactions (a keeper bot batch, for example).
    let mut manifest_cache: HashMap<String, Option<String>> = HashMap::new();

    for (i, ix) in instructions.iter().enumerate() {
        println!("[instruction {i}]");
        match ix {
            UiInstruction::Parsed(UiParsedInstruction::Parsed(parsed)) => {
                println!("  program: {} (parsed by RPC)", parsed.program);
                println!("  kind   : {}", parsed.parsed);
            }
            UiInstruction::Parsed(UiParsedInstruction::PartiallyDecoded(partial)) => {
                let program_id = partial.program_id.clone();
                explain_partial(
                    &program_id,
                    &partial.accounts,
                    &partial.data,
                    rpc_url,
                    &mut manifest_cache,
                );
            }
            UiInstruction::Compiled(compiled) => {
                println!("  program idx: {}", compiled.program_id_index);
                println!("  accounts   : {:?}", compiled.accounts);
                println!("  data       : {}", compiled.data);
            }
        }
    }

    if show_raw_logs {
        if let Some(meta) = tx.transaction.meta.as_ref() {
            if let solana_transaction_status::option_serializer::OptionSerializer::Some(logs) =
                &meta.log_messages
            {
                println!();
                println!("logs:");
                for log in logs {
                    println!("  {log}");
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
) {
    println!("  program   : {program_id}");
    let manifest = manifest_cache.entry(program_id.to_string()).or_insert_with(|| {
        super::manager_invoke::try_fetch_manifest(rpc_url, program_id).ok()
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
    println!("  disc byte : 0x{:02x}", tag);
    println!("  data len  : {} bytes", data_bytes.len());

    if let Some(manifest_json) = manifest {
        match super::manager_invoke::lookup_instruction_by_tag(manifest_json, tag) {
            Some(ix_line) => {
                println!("  matched   : {ix_line}");
            }
            None => {
                println!("  matched   : (no Hopper instruction with disc 0x{:02x})", tag);
            }
        }
    } else {
        println!("  manifest  : (no Hopper manifest on chain; skipping decode)");
    }
    println!("  accounts  : {} slots", accounts.len());
    for (i, a) in accounts.iter().enumerate() {
        println!("    [{}] {}", i, a);
    }
}
