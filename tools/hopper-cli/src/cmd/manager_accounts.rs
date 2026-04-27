//! `hopper manager accounts list <program-id>` - enumerate every live
//! on-chain account for a Hopper program, grouped by declared layout.
//!
//! Workflow:
//!
//! 1. Fetch the program's on-chain manifest PDA.
//! 2. Read the manifest's `layouts` array. Each layout carries its
//!    discriminator byte and a byte-size field.
//! 3. Call `getProgramAccounts` with a `memcmp` filter on byte 0
//!    equalling the layout discriminator, paired with a `dataSize`
//!    filter when the layout is fixed-length. One RPC per layout
//!    keeps the individual response small and lets us stream results.
//! 4. Render a table of (layout, account count, top N addresses).
//!
//! Flags control verbosity (`--addresses N` to list up to N hits per
//! layout) and scope (`--only <layout>` to query just one).

use std::collections::BTreeMap;
use std::process;

pub fn cmd_manager_accounts(args: &[String]) {
    if args.is_empty() || matches!(args[0].as_str(), "--help" | "-h") {
        print_usage();
        return;
    }
    match args[0].as_str() {
        "list" => cmd_list(&args[1..]),
        other => {
            eprintln!("unknown accounts subcommand: {other}");
            print_usage();
            process::exit(1);
        }
    }
}

fn print_usage() {
    eprintln!("Usage: hopper manager accounts <subcommand>");
    eprintln!();
    eprintln!("Subcommands:");
    eprintln!("  list <program-id> [options]   List live accounts grouped by layout");
    eprintln!();
    eprintln!("`list` options:");
    eprintln!("  --rpc <url>                   RPC endpoint (default: from config / env)");
    eprintln!("  --addresses <N>               Show up to N addresses per layout (default 5)");
    eprintln!("  --only <layout>               Query only one layout by name");
    eprintln!("  --json                        Emit raw JSON instead of a table");
}

fn cmd_list(args: &[String]) {
    let mut program_id: Option<String> = None;
    let mut rpc: Option<String> = None;
    let mut max_addresses: usize = 5;
    let mut only_layout: Option<String> = None;
    let mut json_out = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--rpc" => {
                i += 1;
                rpc = args.get(i).cloned();
            }
            "--addresses" => {
                i += 1;
                max_addresses = args.get(i).and_then(|v| v.parse().ok()).unwrap_or(5);
            }
            "--only" => {
                i += 1;
                only_layout = args.get(i).cloned();
            }
            "--json" => json_out = true,
            other if !other.starts_with("--") && program_id.is_none() => {
                program_id = Some(other.to_string());
            }
            other => {
                eprintln!("unknown flag: {other}");
                print_usage();
                process::exit(1);
            }
        }
        i += 1;
    }
    let program_id = program_id.unwrap_or_else(|| {
        eprintln!("missing <program-id> arg");
        print_usage();
        process::exit(1);
    });
    let rpc_url = rpc.unwrap_or_else(|| crate::rpc::resolve_rpc_url(None));

    if let Err(e) = run_list(&rpc_url, &program_id, max_addresses, only_layout.as_deref(), json_out) {
        eprintln!("hopper manager accounts list failed: {e}");
        process::exit(1);
    }
}

fn run_list(
    rpc_url: &str,
    program_id: &str,
    max_addresses: usize,
    only_layout: Option<&str>,
    json_out: bool,
) -> Result<(), String> {
    // Fetch the manifest using the same helper invoke/explain use.
    let manifest_json =
        super::manager_invoke::try_fetch_manifest(rpc_url, program_id)
            .map_err(|e| format!("fetch manifest: {e}"))?;

    let layouts = parse_layouts(&manifest_json)?;
    if layouts.is_empty() {
        if json_out {
            println!("[]");
        } else {
            println!("program `{}` declares no layouts", program_id);
        }
        return Ok(());
    }

    // Query each layout. When `--only` is set, filter the driver list.
    let targets: Vec<&LayoutEntry> = layouts
        .iter()
        .filter(|l| only_layout.map(|n| n == l.name).unwrap_or(true))
        .collect();

    let mut results: BTreeMap<String, LayoutResult> = BTreeMap::new();
    for layout in &targets {
        let hits = get_program_accounts_by_disc(
            rpc_url,
            program_id,
            layout.disc,
            layout.byte_size,
        )
        .map_err(|e| format!("getProgramAccounts({}): {e}", layout.name))?;
        results.insert(
            layout.name.clone(),
            LayoutResult {
                disc: layout.disc,
                byte_size: layout.byte_size,
                addresses: hits,
            },
        );
    }

    if json_out {
        render_json(&results);
    } else {
        render_table(&results, max_addresses);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Manifest parsing
// ---------------------------------------------------------------------------

struct LayoutEntry {
    name: String,
    disc: u8,
    byte_size: Option<u64>,
}

fn parse_layouts(manifest_json: &str) -> Result<Vec<LayoutEntry>, String> {
    let value: serde_json::Value = serde_json::from_str(manifest_json)
        .map_err(|e| format!("parse manifest: {e}"))?;
    let arr = value
        .get("layouts")
        .and_then(|v| v.as_array())
        .ok_or("manifest has no `layouts` array")?;
    let mut out: Vec<LayoutEntry> = Vec::new();
    for layout in arr {
        let name = layout
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("(unnamed)")
            .to_string();
        let disc = layout
            .get("disc")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| format!("layout `{name}` missing `disc`"))? as u8;
        // Byte size may be reported as `body_size` plus a 16-byte
        // header, or as a direct `len` field, depending on the
        // schema version. Both branches cover the real shape.
        let byte_size = layout
            .get("len")
            .and_then(|v| v.as_u64())
            .or_else(|| {
                let body = layout.get("body_size").and_then(|v| v.as_u64())?;
                Some(body + 16)
            });
        out.push(LayoutEntry { name, disc, byte_size });
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// RPC
// ---------------------------------------------------------------------------

/// Call `getProgramAccounts` with a 1-byte memcmp filter on byte 0
/// plus an optional `dataSize` filter. Returns the list of matching
/// account pubkeys. We deliberately do not return data (base64
/// payloads balloon the response for minimal extra value; users who
/// want the bytes follow up with `hopper inspect`).
fn get_program_accounts_by_disc(
    rpc_url: &str,
    program_id: &str,
    disc_byte: u8,
    data_size: Option<u64>,
) -> Result<Vec<String>, String> {
    let data_size_filter = match data_size {
        Some(n) => format!(r#",{{"dataSize":{n}}}"#),
        None => String::new(),
    };
    // `bytes` is a base58-encoded one-byte value. base58 of a single
    // byte never requires decoding complications; just encode
    // directly.
    let memcmp_bytes = bs58::encode([disc_byte]).into_string();
    let body = format!(
        r#"{{"jsonrpc":"2.0","id":1,"method":"getProgramAccounts","params":["{program_id}",{{"encoding":"base64","commitment":"confirmed","dataSlice":{{"offset":0,"length":0}},"filters":[{{"memcmp":{{"offset":0,"bytes":"{memcmp_bytes}"}}}}{data_size_filter}]}}]}}"#
    );
    let resp = ureq::post(rpc_url)
        .set("Content-Type", "application/json")
        .send_string(&body)
        .map_err(|e| format!("http: {e}"))?;
    let text = resp
        .into_string()
        .map_err(|e| format!("read body: {e}"))?;

    let parsed: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| format!("parse RPC response: {e}"))?;
    if let Some(err) = parsed.get("error") {
        return Err(format!("rpc error: {err}"));
    }
    let result = parsed
        .get("result")
        .and_then(|v| v.as_array())
        .ok_or("rpc response has no `result` array")?;
    let mut out: Vec<String> = Vec::with_capacity(result.len());
    for entry in result {
        if let Some(pk) = entry.get("pubkey").and_then(|v| v.as_str()) {
            out.push(pk.to_string());
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Render
// ---------------------------------------------------------------------------

struct LayoutResult {
    disc: u8,
    byte_size: Option<u64>,
    addresses: Vec<String>,
}

fn render_table(results: &BTreeMap<String, LayoutResult>, max_addresses: usize) {
    let name_w = results
        .keys()
        .map(|s| s.len())
        .chain(std::iter::once("layout".len()))
        .max()
        .unwrap_or(8);
    println!(
        "{:<name_w$}  {:>5}  {:>8}  {:>7}  sample addresses",
        "layout", "disc", "bytes", "count",
        name_w = name_w
    );
    println!(
        "{:-<name_w$}  {:-<5}  {:-<8}  {:-<7}  {:-<40}",
        "", "", "", "", "",
        name_w = name_w
    );
    for (name, r) in results {
        let size = r.byte_size.map(|n| n.to_string()).unwrap_or_else(|| "?".into());
        let count = r.addresses.len();
        let sample: Vec<&str> = r
            .addresses
            .iter()
            .take(max_addresses)
            .map(String::as_str)
            .collect();
        println!(
            "{:<name_w$}  {:>5}  {:>8}  {:>7}  {}",
            name,
            r.disc,
            size,
            count,
            sample.join(", "),
            name_w = name_w
        );
        if count > max_addresses {
            println!(
                "{:<name_w$}  {:>5}  {:>8}  {:>7}  (+{} more)",
                "",
                "",
                "",
                "",
                count - max_addresses,
                name_w = name_w
            );
        }
    }
}

fn render_json(results: &BTreeMap<String, LayoutResult>) {
    let mut out = serde_json::Map::new();
    for (name, r) in results {
        out.insert(
            name.clone(),
            serde_json::json!({
                "disc": r.disc,
                "byte_size": r.byte_size,
                "count": r.addresses.len(),
                "addresses": r.addresses,
            }),
        );
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::Value::Object(out))
            .unwrap_or_else(|_| "{}".into())
    );
}
