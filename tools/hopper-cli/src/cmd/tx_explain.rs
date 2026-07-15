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
    let mut show_tree = false;
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
            "--tree" => show_tree = true,
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
        show_tree,
        local_manifest.as_deref(),
    ) {
        eprintln!("hopper tx explain failed: {e}");
        process::exit(1);
    }
}

fn print_usage() {
    eprintln!("Usage: hopper tx explain <signature> [--rpc <url>] [--tree] [--raw-logs]");
    eprintln!();
    eprintln!("Fetch a confirmed transaction by signature and decode every");
    eprintln!("instruction against the target Hopper program's on-chain manifest.");
    eprintln!();
    eprintln!("Options:");
    eprintln!("  --rpc <url>        RPC endpoint (default from config / env)");
    eprintln!("  --manifest <file> Local manifest used to map disc bytes to instruction");
    eprintln!("                    names when the program has no on-chain manifest");
    eprintln!("  --tree             Render the CPI call tree (per-frame CU + the touch");
    eprintln!("                     maps each frame emitted) — the call graph annotated");
    eprintln!("                     with byte-level state effects");
    eprintln!("  --raw-logs         Print the full Program-log stream verbatim");
}

/// Issue a JSON-RPC `getTransaction` and return the parsed `result`
/// object. We request `jsonParsed` + `maxSupportedTransactionVersion: 0`
/// so versioned (v0) deploy/upgrade transactions are returned rather
/// than erroring.
pub(crate) fn rpc_get_transaction(rpc_url: &str, signature: &str) -> Result<Value, String> {
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
    show_tree: bool,
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
    // Per top-level instruction: (program_id, account pubkeys), retained
    // so touch-map records found in the log stream can be joined back to
    // the instruction that emitted them.
    let mut top_level: Vec<(String, Vec<String>)> = Vec::new();

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
            top_level.push((
                ix.get("programId")
                    .and_then(Value::as_str)
                    .unwrap_or(program)
                    .to_string(),
                Vec::new(),
            ));
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
        top_level.push((program_id, accounts));
    }

    // Self-describing state effects: scan the log stream for Hopper
    // touch-map records (magic 0x7A, version 0x01) and decode them
    // against each instruction's account list and manifest field map.
    if let Some(logs) = meta
        .and_then(|m| m.get("logMessages"))
        .and_then(Value::as_array)
    {
        let log_lines: Vec<&str> = logs.iter().filter_map(Value::as_str).collect();
        let touch_maps = extract_touch_maps(&log_lines);
        if !touch_maps.is_empty() {
            println!();
            println!("touch maps (self-describing state effects):");
            for entry in &touch_maps {
                let ix_index = entry.top_ix;
                let (program_id, accounts) = top_level
                    .get(ix_index)
                    .map(|(p, a)| (p.as_str(), a.as_slice()))
                    .unwrap_or(("?", &[]));
                let manifest = manifest_cache
                    .get(program_id)
                    .and_then(|m| m.as_deref())
                    .or(local_manifest);
                println!("  [instruction {ix_index}] program {program_id}");
                for line in render_touch_map_entry(entry, accounts, manifest) {
                    println!("    {line}");
                }
            }
        }
    }

    // Self-CPI events: scan the transaction's INNER instructions for
    // the reserved Hopper event marker. Events ride inner-instruction
    // metadata (which RPC nodes never truncate, unlike logs), so this
    // is the indexer-grade read path — and with the program's manifest
    // it renders named, field-decoded events, not hex.
    if let Some(meta) = meta {
        let events = extract_cpi_events(meta, &top_level);
        if !events.is_empty() {
            println!();
            println!("events (self-CPI, from inner-instruction metadata):");
            for ev in &events {
                let manifest = manifest_cache
                    .get(&ev.target_program)
                    .and_then(|m| m.as_deref())
                    .or(local_manifest);
                println!(
                    "  [instruction {}] program {}",
                    ev.top_ix, ev.target_program
                );
                for line in render_event(ev, manifest) {
                    println!("    {line}");
                }
            }
        }
    }

    // The CPI call tree: who called whom, per-frame CU, and the touch
    // maps each frame emitted. Opt-in (`--tree`) because a flat
    // single-program transaction reads fine without it; it earns its
    // place the moment a transaction chains through more than one program.
    if show_tree {
        if let Some(logs) = meta
            .and_then(|m| m.get("logMessages"))
            .and_then(Value::as_array)
        {
            let log_lines: Vec<&str> = logs.iter().filter_map(Value::as_str).collect();
            let tree = build_cpi_tree(&log_lines);
            if tree.is_empty() {
                println!();
                println!("cpi call tree: (no program frames in the log stream)");
            } else {
                println!();
                println!("cpi call tree (depth, outcome, CU, and per-frame state effects):");
                for line in render_cpi_tree(&tree) {
                    println!("  {line}");
                }
            }
        }
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

// ---------------------------------------------------------------------------
// Hopper self-CPI event decoding
// ---------------------------------------------------------------------------
//
// `#[hopper::context(event_cpi)]` + `ctx.emit_event_cpi(&event)` emit
// events as a self-CPI whose instruction data is the versioned wire
// `[0xE0, 0x1E, tag, payload]` (constants replicated from
// `hopper-runtime/src/cpi_event.rs`, a lib this bin does not link).
// The payload lands in `meta.innerInstructions`, so it survives log
// truncation — this decoder is the same read an indexer performs.
//
// Authenticity is judged honestly: on-chain, the generated sink only
// accepts marker instructions whose event-authority PDA signed, and a
// PDA can only be signed by its own program's `invoke_signed`. So a
// marker inner instruction whose TARGET equals the program that was
// executing (a self-CPI in a SUCCESSFUL transaction) has been
// sink-authenticated on-chain. A marker-shaped CPI to a DIFFERENT
// program proves nothing and is labeled as exactly that.

/// First wire byte of the reserved Hopper CPI-event marker.
const CPI_EVENT_MARKER_0: u8 = 0xE0;
/// Second wire byte of the reserved Hopper CPI-event marker.
const CPI_EVENT_MARKER_1: u8 = 0x1E;

/// One decoded self-CPI event candidate.
#[derive(Debug, PartialEq, Eq)]
struct ExtractedEvent {
    /// Index of the enclosing top-level instruction.
    top_ix: usize,
    /// The inner instruction's target program.
    target_program: String,
    /// Whether the target equals the enclosing top-level program (a
    /// true self-CPI, sink-authenticated in a successful transaction).
    self_cpi: bool,
    /// The 1-byte event tag.
    tag: u8,
    /// The event payload (bytes after marker + tag).
    payload: Vec<u8>,
}

/// Scan `meta.innerInstructions` for marker-prefixed instruction data.
fn extract_cpi_events(meta: &Value, top_level: &[(String, Vec<String>)]) -> Vec<ExtractedEvent> {
    let mut out = Vec::new();
    let Some(groups) = meta.get("innerInstructions").and_then(Value::as_array) else {
        return out;
    };
    for group in groups {
        let Some(top_ix) = group.get("index").and_then(Value::as_u64) else {
            continue;
        };
        let top_ix = top_ix as usize;
        let parent_program = top_level.get(top_ix).map(|(p, _)| p.as_str());
        let Some(inners) = group.get("instructions").and_then(Value::as_array) else {
            continue;
        };
        for inner in inners {
            // Marker CPIs target a Hopper program, which the RPC cannot
            // json-parse, so they arrive in the partially-decoded shape
            // with base58 `data`. (`parsed` system/SPL inners cannot be
            // Hopper events by construction.)
            let Some(data_b58) = inner.get("data").and_then(Value::as_str) else {
                continue;
            };
            let Ok(bytes) = bs58::decode(data_b58).into_vec() else {
                continue;
            };
            if bytes.len() < 3 || bytes[0] != CPI_EVENT_MARKER_0 || bytes[1] != CPI_EVENT_MARKER_1 {
                continue;
            }
            let target_program = inner
                .get("programId")
                .and_then(Value::as_str)
                .unwrap_or("?")
                .to_string();
            out.push(ExtractedEvent {
                top_ix,
                self_cpi: parent_program == Some(target_program.as_str()),
                target_program,
                tag: bytes[2],
                payload: bytes[3..].to_vec(),
            });
        }
    }
    out
}

/// Render one event: authenticity line, then either a manifest-joined
/// `Name { field: value, .. }` decode or an honest tag+hex fallback.
fn render_event(ev: &ExtractedEvent, manifest_json: Option<&str>) -> Vec<String> {
    let mut out = Vec::new();
    if ev.self_cpi {
        out.push(
            "self-CPI (sink-authenticated on-chain: only this program's invoke_signed \
             can sign its event authority)"
                .to_string(),
        );
    } else {
        out.push(
            "marker-shaped CPI to a FOREIGN program — NOT a Hopper-authenticated event".to_string(),
        );
    }
    let joined = manifest_json.and_then(|m| render_event_fields(m, ev.tag, &ev.payload));
    match joined {
        Some(lines) => out.extend(lines),
        None => {
            out.push(format!(
                "tag 0x{:02x}, payload {} bytes: {}",
                ev.tag,
                ev.payload.len(),
                hex_of(&ev.payload)
            ));
        }
    }
    out
}

/// Join an event tag + payload against the manifest's `events` table.
/// Returns `None` when the manifest has no matching tag (callers fall
/// back to hex) — and renders honestly when field spans exceed the
/// payload (a manifest/payload mismatch is reported, not padded over).
fn render_event_fields(manifest_json: &str, tag: u8, payload: &[u8]) -> Option<Vec<String>> {
    let value: Value = serde_json::from_str(manifest_json).ok()?;
    let events = value.get("events").and_then(Value::as_array)?;
    let event = events
        .iter()
        .find(|e| e.get("tag").and_then(Value::as_u64) == Some(tag as u64))?;
    let name = event.get("name").and_then(Value::as_str).unwrap_or("?");
    let mut lines = vec![format!("event: {name} (tag 0x{tag:02x})")];
    let Some(fields) = event.get("fields").and_then(Value::as_array) else {
        lines.push(format!(
            "payload {} bytes: {}",
            payload.len(),
            hex_of(payload)
        ));
        return Some(lines);
    };
    for field in fields {
        let f_name = field.get("name").and_then(Value::as_str).unwrap_or("?");
        let f_type = field.get("type").and_then(Value::as_str).unwrap_or("?");
        let (Some(off), Some(size)) = (
            field.get("offset").and_then(Value::as_u64),
            field.get("size").and_then(Value::as_u64),
        ) else {
            continue;
        };
        let (off, size) = (off as usize, size as usize);
        match payload.get(off..off + size) {
            Some(bytes) => lines.push(format!(
                "  {f_name}: {} ({f_type})",
                render_scalar(f_type, bytes)
            )),
            None => lines.push(format!(
                "  {f_name}: <manifest declares [{off}..{}) but payload is {} bytes>",
                off + size,
                payload.len()
            )),
        }
    }
    Some(lines)
}

/// Render a field's bytes by declared wire type: little-endian integers
/// for the `WireU*`/`WireI*` family (and bare `u*`/`i*` spellings),
/// hex for everything else.
fn render_scalar(f_type: &str, bytes: &[u8]) -> String {
    let t = f_type.trim_start_matches("Wire").to_ascii_lowercase();
    macro_rules! le {
        ($ty:ty, $n:expr) => {{
            let mut buf = [0u8; $n];
            if bytes.len() == $n {
                buf.copy_from_slice(bytes);
                return <$ty>::from_le_bytes(buf).to_string();
            }
        }};
    }
    let render = || -> String {
        match t.as_str() {
            "u64" => le!(u64, 8),
            "u32" => le!(u32, 4),
            "u16" => le!(u16, 2),
            "u8" => le!(u8, 1),
            "i64" => le!(i64, 8),
            "i32" => le!(i32, 4),
            "u128" => le!(u128, 16),
            _ => {}
        }
        format!("0x{}", hex_of_inner(bytes))
    };
    render()
}

fn hex_of_inner(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn hex_of(bytes: &[u8]) -> String {
    format!("0x{}", hex_of_inner(bytes))
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

// ---------------------------------------------------------------------------
// Hopper touch-map decoding (self-describing transactions)
// ---------------------------------------------------------------------------
//
// Hopper programs built with the `touch-map` runtime feature can call
// `Context::emit_touch_map()` at the end of a handler, which emits one
// `sol_log_data` record describing every (account, offset, size, R/W)
// range the instruction touched. The wire format is versioned and
// documented in `hopper-runtime/src/segment_borrow.rs`:
//
//   byte 0: magic 0x7A | byte 1: version 0x01 | byte 2: flags | byte 3: count
//   then per record (9 bytes): slot u8, packed u32 LE (top bit = write,
//   low 31 bits = offset), size u32 LE.
//
// Decoders must verify magic, version, AND the exact-length equation
// `len == 4 + 9 * count` so unrelated `Program data:` payloads (Anchor
// events, receipts) are never misread as touch maps.

/// Magic byte of the touch-map wire format v1.
const TOUCH_MAP_MAGIC: u8 = 0x7A;
/// Version byte of the touch-map wire format v1.
const TOUCH_MAP_VERSION: u8 = 0x01;
/// Flags bit0: the on-chain touch log overflowed; the map is partial.
const TOUCH_MAP_FLAG_OVERFLOWED: u8 = 1 << 0;
/// Flags bit1: the encoder skipped at least one record it could not
/// represent (unmappable address or offset beyond 31 bits).
const TOUCH_MAP_FLAG_SKIPPED: u8 = 1 << 1;
/// Capacity of the on-chain touch log; overflow truncates at this count.
const TOUCH_MAP_MAX_RECORDS: usize = 32;

/// One decoded touch record.
#[derive(Debug, PartialEq, Eq)]
struct TouchRecord {
    slot: u8,
    offset: u32,
    size: u32,
    write: bool,
}

/// A decoded touch map.
#[derive(Debug, PartialEq, Eq)]
struct TouchMap {
    overflowed: bool,
    skipped: bool,
    records: Vec<TouchRecord>,
}

/// Decode a touch-map `sol_log_data` payload. Returns `None` unless the
/// magic, version, and exact-length equation all hold.
fn decode_touch_map(bytes: &[u8]) -> Option<TouchMap> {
    if bytes.len() < 4 || bytes[0] != TOUCH_MAP_MAGIC || bytes[1] != TOUCH_MAP_VERSION {
        return None;
    }
    let flags = bytes[2];
    let count = bytes[3] as usize;
    if bytes.len() != 4 + count * 9 {
        return None;
    }
    let mut records = Vec::with_capacity(count);
    for i in 0..count {
        let base = 4 + i * 9;
        let packed = u32::from_le_bytes(bytes[base + 1..base + 5].try_into().ok()?);
        records.push(TouchRecord {
            slot: bytes[base],
            offset: packed & 0x7FFF_FFFF,
            size: u32::from_le_bytes(bytes[base + 5..base + 9].try_into().ok()?),
            write: packed & 0x8000_0000 != 0,
        });
    }
    Some(TouchMap {
        overflowed: flags & TOUCH_MAP_FLAG_OVERFLOWED != 0,
        skipped: flags & TOUCH_MAP_FLAG_SKIPPED != 0,
        records,
    })
}

/// A touch map found in the log stream, with its attribution context.
#[derive(Debug, PartialEq, Eq)]
struct ExtractedTouchMap {
    /// Index of the enclosing top-level instruction.
    top_ix: usize,
    map: TouchMap,
    /// `Some(callee_program_id)` when the map was emitted while a CPI
    /// callee was executing (invoke depth > 1). Its record slots index
    /// the *callee's* account list, which the parsed transaction does
    /// not expose — so such maps must be rendered raw, never
    /// field-joined against the top-level instruction.
    cpi_program: Option<String>,
}

/// A runtime-generated structural log line. These are emitted by the
/// runtime itself and are NOT prefixed with `Program log:` /
/// `Program data:`, so a program cannot forge one via `msg!` — anything
/// a program logs arrives behind a `log:` / `data:` token, which fails
/// the base58 program-id check below.
enum StructuralLine<'a> {
    /// `Program <id> invoke [n]`
    Invoke { program: &'a str, depth: usize },
    /// `Program <id> success` or `Program <id> failed...`
    Exit,
}

/// True for bytes in the Bitcoin base58 alphabet (no `0`, `O`, `I`, `l`).
fn is_base58_byte(b: u8) -> bool {
    matches!(b, b'1'..=b'9' | b'A'..=b'H' | b'J'..=b'N' | b'P'..=b'Z' | b'a'..=b'k' | b'm'..=b'z')
}

/// Parse a log line as a structural invoke/exit marker. Returns `None`
/// for everything else — in particular for `Program log:` /
/// `Program data:` lines, whose second token (`log:`, `data:`) is not
/// base58, so spoofed text like `msg!("Program X invoke [1]")` can
/// never advance instruction or depth state.
fn parse_structural_line(line: &str) -> Option<StructuralLine<'_>> {
    let rest = line.strip_prefix("Program ")?;
    let (program, tail) = rest.split_once(' ')?;
    if program.is_empty() || !program.bytes().all(is_base58_byte) {
        return None;
    }
    if let Some(n) = tail
        .strip_prefix("invoke [")
        .and_then(|t| t.strip_suffix(']'))
    {
        let depth: usize = n.parse().ok()?;
        return Some(StructuralLine::Invoke { program, depth });
    }
    if tail == "success" || tail.starts_with("failed") {
        return Some(StructuralLine::Exit);
    }
    None
}

/// Scan a transaction's log stream for touch-map records, attributing
/// each to its **top-level** instruction and recording whether it was
/// emitted by a CPI callee.
///
/// Instruction/depth state advances ONLY on runtime-generated
/// structural lines (`Program <id> invoke [n]` / `success` / `failed`);
/// program-emitted `Program log:` / `Program data:` text cannot shift
/// attribution. A map emitted at invoke depth > 1 is tagged with the
/// callee's program id so the renderer can refuse field joins — its
/// slots are callee-relative and the callee's account list is unknown.
fn extract_touch_maps(log_lines: &[&str]) -> Vec<ExtractedTouchMap> {
    let mut out = Vec::new();
    let mut top_ix: Option<usize> = None;
    // Stack of currently-executing program ids; len() is invoke depth.
    let mut invoke_stack: Vec<String> = Vec::new();
    for line in log_lines {
        match parse_structural_line(line) {
            Some(StructuralLine::Invoke { program, depth }) => {
                if depth == 1 {
                    top_ix = Some(top_ix.map_or(0, |i| i + 1));
                    // Resync: a fresh top-level invoke means any
                    // still-stacked frames were left by a truncated or
                    // malformed stream.
                    invoke_stack.clear();
                }
                invoke_stack.push(program.to_string());
                continue;
            }
            Some(StructuralLine::Exit) => {
                invoke_stack.pop();
                continue;
            }
            None => {}
        }
        if let Some(rest) = line.strip_prefix("Program data: ") {
            for seg in rest.split_whitespace() {
                let Ok(bytes) =
                    base64::Engine::decode(&base64::engine::general_purpose::STANDARD, seg)
                else {
                    continue;
                };
                if let Some(map) = decode_touch_map(&bytes) {
                    // A data record before any invoke line would be
                    // malformed; attribute to instruction 0 rather than
                    // dropping it.
                    let cpi_program = if invoke_stack.len() > 1 {
                        invoke_stack.last().cloned()
                    } else {
                        None
                    };
                    out.push(ExtractedTouchMap {
                        top_ix: top_ix.unwrap_or(0),
                        map,
                        cpi_program,
                    });
                }
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// CPI call tree
// ---------------------------------------------------------------------------
//
// LiteSVM 0.14's `litesvm-cpi-tree` reconstructs the invocation chain
// from a transaction's flat log stream — which program called which, at
// what depth, and where each frame succeeded or failed. Hopper renders
// the same tree from the same runtime-structural lines, and then does
// the thing only a byte-range framework can: it hangs each frame's
// decoded touch-map records (its declared state effects) off that node.
// The result is not just "who called whom" but "who called whom, and
// exactly which bytes each frame wrote" — the call graph annotated with
// the self-describing state effects, read straight from a confirmed
// transaction with no extra dependency.
//
// Attribution is spoof-proof for the same reason touch-map attribution
// is: frames advance ONLY on `Program <id> invoke [n]` / `success` /
// `failed` / `consumed` lines whose program token is base58 (a program
// cannot forge one via `msg!`, whose payload sits behind a `log:` /
// `data:` token). A touch map is attributed to the frame on top of the
// invoke stack when its `Program data:` line appears.

/// How a CPI frame terminated.
#[derive(Debug, PartialEq, Eq)]
enum FrameOutcome {
    Success,
    /// `failed: <reason>` — the runtime's failure text.
    Failed(String),
    /// The stream ended (or was truncated) before this frame closed.
    Unterminated,
}

/// One node in the reconstructed CPI call tree.
#[derive(Debug, PartialEq, Eq)]
struct CpiTreeNode {
    /// The program id executing in this frame.
    program: String,
    /// Invoke depth as the runtime reported it (`invoke [n]`).
    depth: usize,
    /// How the frame terminated.
    outcome: FrameOutcome,
    /// `consumed N of M compute units`, if the runtime logged it.
    cu_consumed: Option<u64>,
    /// Decoded Hopper touch maps this exact frame emitted (its declared
    /// state effects). Empty for a frame that emitted none.
    touch_maps: Vec<TouchMap>,
    /// Nested frames this frame invoked, in order.
    children: Vec<CpiTreeNode>,
}

/// Parse the `consumed N of M compute units` tail of a `Program <id> …`
/// line, returning `N`.
fn parse_consumed_cu(tail: &str) -> Option<u64> {
    tail.strip_prefix("consumed ")
        .and_then(|t| t.split_whitespace().next())
        .and_then(|n| n.parse().ok())
}

/// Reconstruct the CPI call tree from a transaction's log stream. Roots
/// are the depth-1 (top-level) frames, in execution order. This is a
/// standalone parser (not `parse_structural_line`, which the
/// security-sensitive touch-map attribution relies on) so it can read
/// the fuller `consumed` / `failed: <reason>` vocabulary the tree wants
/// without perturbing that path. The base58 program-token guard is the
/// same, so frame state is equally unspoofable.
fn build_cpi_tree(log_lines: &[&str]) -> Vec<CpiTreeNode> {
    let mut roots: Vec<CpiTreeNode> = Vec::new();
    // Open frames, outermost first; the last is the current top of stack.
    let mut stack: Vec<CpiTreeNode> = Vec::new();

    // Attach a finished node to its parent (or the roots if it was
    // top-level).
    fn attach(node: CpiTreeNode, stack: &mut Vec<CpiTreeNode>, roots: &mut Vec<CpiTreeNode>) {
        match stack.last_mut() {
            Some(parent) => parent.children.push(node),
            None => roots.push(node),
        }
    }

    for line in log_lines {
        let Some(rest) = line.strip_prefix("Program ") else {
            continue;
        };
        let Some((program, tail)) = rest.split_once(' ') else {
            continue;
        };
        // Touch-map data lines: attribute to the current top frame. These
        // do NOT carry a base58 program token (the prefix is `data:`), so
        // they are handled before the program-token guard rejects them.
        if program == "data:" {
            for seg in tail.split_whitespace() {
                if let Ok(bytes) =
                    base64::Engine::decode(&base64::engine::general_purpose::STANDARD, seg)
                {
                    if let Some(map) = decode_touch_map(&bytes) {
                        if let Some(top) = stack.last_mut() {
                            top.touch_maps.push(map);
                        }
                    }
                }
            }
            continue;
        }
        if program.is_empty() || !program.bytes().all(is_base58_byte) {
            continue;
        }
        if let Some(n) = tail
            .strip_prefix("invoke [")
            .and_then(|t| t.strip_suffix(']'))
        {
            let depth: usize = match n.parse() {
                Ok(d) => d,
                Err(_) => continue,
            };
            stack.push(CpiTreeNode {
                program: program.to_string(),
                depth,
                outcome: FrameOutcome::Unterminated,
                cu_consumed: None,
                touch_maps: Vec::new(),
                children: Vec::new(),
            });
        } else if let Some(cu) = parse_consumed_cu(tail) {
            // `consumed` names the frame about to close (top of stack).
            if let Some(top) = stack.last_mut() {
                top.cu_consumed = Some(cu);
            }
        } else if tail == "success" {
            if let Some(mut node) = stack.pop() {
                node.outcome = FrameOutcome::Success;
                attach(node, &mut stack, &mut roots);
            }
        } else if let Some(reason) = tail.strip_prefix("failed") {
            if let Some(mut node) = stack.pop() {
                let reason = reason.trim_start_matches([':', ' ']).to_string();
                node.outcome = FrameOutcome::Failed(reason);
                attach(node, &mut stack, &mut roots);
            }
        }
    }

    // Any frames still open (truncated stream) unwind as Unterminated,
    // innermost first, so the tree is still well-formed.
    while let Some(node) = stack.pop() {
        attach(node, &mut stack, &mut roots);
    }
    roots
}

/// Render one CPI tree node and its subtree as indented lines. `prefix`
/// is the running indent; `is_last` controls the branch glyph.
fn render_cpi_node(node: &CpiTreeNode, prefix: &str, is_last: bool, out: &mut Vec<String>) {
    let branch = if is_last { "└─ " } else { "├─ " };
    let outcome = match &node.outcome {
        FrameOutcome::Success => "ok".to_string(),
        FrameOutcome::Failed(reason) if reason.is_empty() => "FAILED".to_string(),
        FrameOutcome::Failed(reason) => format!("FAILED: {reason}"),
        FrameOutcome::Unterminated => "(unterminated)".to_string(),
    };
    let cu = node
        .cu_consumed
        .map(|c| format!(", {c} CU"))
        .unwrap_or_default();
    out.push(format!(
        "{prefix}{branch}{} [{}] {outcome}{cu}",
        node.program, node.depth
    ));
    // The indent inherited by this node's own children.
    let child_prefix = format!("{prefix}{}", if is_last { "   " } else { "│  " });
    // A frame's decoded state effects hang directly under it.
    for map in &node.touch_maps {
        let writes = map.records.iter().filter(|r| r.write).count();
        let reads = map.records.len() - writes;
        let mut note = format!(
            "{child_prefix}   ↳ touch map: {} write{}, {} read{}",
            writes,
            if writes == 1 { "" } else { "s" },
            reads,
            if reads == 1 { "" } else { "s" }
        );
        if map.overflowed || map.skipped {
            note.push_str(" (partial: ");
            note.push_str(match (map.overflowed, map.skipped) {
                (true, true) => "overflowed + skipped",
                (true, false) => "overflowed",
                _ => "skipped",
            });
            note.push(')');
        }
        out.push(note);
        for r in &map.records {
            out.push(format!(
                "{child_prefix}      {} slot {} [{}..{})",
                if r.write { "W" } else { "R" },
                r.slot,
                r.offset,
                r.offset + r.size
            ));
        }
    }
    for (i, child) in node.children.iter().enumerate() {
        let last = i + 1 == node.children.len();
        render_cpi_node(child, &child_prefix, last, out);
    }
}

/// Render the whole forest of top-level frames as a call tree.
fn render_cpi_tree(roots: &[CpiTreeNode]) -> Vec<String> {
    let mut out = Vec::new();
    for (i, root) in roots.iter().enumerate() {
        render_cpi_node(root, "", i + 1 == roots.len(), &mut out);
    }
    out
}

/// Join a touched byte range against a manifest's layout field maps.
/// Returns `Layout.field`-style names for every field any layout
/// declares that overlaps `[offset, offset + size)`. Multiple layouts
/// can match — the manifest does not say which layout a given account
/// slot holds, so ambiguity is reported, not hidden.
fn field_matches(manifest_json: &str, offset: u32, size: u32) -> Vec<String> {
    let mut out = Vec::new();
    let Ok(value) = serde_json::from_str::<Value>(manifest_json) else {
        return out;
    };
    let Some(layouts) = value.get("layouts").and_then(Value::as_array) else {
        return out;
    };
    let (start, end) = (offset as u64, offset as u64 + size as u64);
    for layout in layouts {
        let layout_name = layout.get("name").and_then(Value::as_str).unwrap_or("?");
        let Some(fields) = layout.get("fields").and_then(Value::as_array) else {
            continue;
        };
        for field in fields {
            let (Some(f_off), Some(f_size)) = (
                field.get("offset").and_then(Value::as_u64),
                field.get("size").and_then(Value::as_u64),
            ) else {
                continue;
            };
            let f_end = f_off + f_size;
            if start < f_end && f_off < end {
                let field_name = field.get("name").and_then(Value::as_str).unwrap_or("?");
                out.push(format!("{layout_name}.{field_name}"));
            }
        }
    }
    out
}

/// Render a decoded touch map as human-readable lines: one per record
/// (`vault.balance`-style field joins where the manifest matches, raw
/// ranges otherwise), plus honest trailer lines for partial maps.
fn render_touch_map(
    map: &TouchMap,
    accounts: &[String],
    manifest_json: Option<&str>,
) -> Vec<String> {
    let mut out = Vec::new();
    for rec in &map.records {
        let rw = if rec.write { "W" } else { "R" };
        let end = rec.offset as u64 + rec.size as u64;
        let who = match accounts.get(rec.slot as usize) {
            Some(pubkey) => format!("slot {} ({pubkey})", rec.slot),
            None => format!("slot {}", rec.slot),
        };
        let mut fields = manifest_json
            .map(|m| field_matches(m, rec.offset, rec.size))
            .unwrap_or_default();
        let suffix = if fields.is_empty() {
            String::new()
        } else {
            if fields.len() > 5 {
                let extra = fields.len() - 5;
                fields.truncate(5);
                fields.push(format!("+{extra} more"));
            }
            format!(" -> {}", fields.join(", "))
        };
        out.push(format!("{rw} {who} [{}..{}){suffix}", rec.offset, end));
    }
    if map.overflowed {
        out.push(format!(
            "...map truncated at {TOUCH_MAP_MAX_RECORDS} records (partial: more ranges were touched)"
        ));
    }
    if map.skipped {
        out.push(
            "note: the encoder skipped at least one touched range it could not represent"
                .to_string(),
        );
    }
    out
}

/// Render an extracted touch map honestly with respect to who emitted
/// it.
///
/// - Emitted at invoke depth 1 (the top-level program itself): field-join
///   against the top-level instruction's account list and manifest.
/// - Emitted by a CPI callee (depth > 1): the record slots index the
///   *callee's* account list, which we do not have — so render raw
///   slot/range/R-W data only, with an explicit provenance note, and
///   never join pubkeys or manifest field names.
fn render_touch_map_entry(
    entry: &ExtractedTouchMap,
    accounts: &[String],
    manifest_json: Option<&str>,
) -> Vec<String> {
    match &entry.cpi_program {
        Some(callee) => {
            let mut out = vec![format!(
                "(emitted by CPI callee {callee}; slots are callee-relative, \
                 field names unavailable)"
            )];
            out.extend(render_touch_map(&entry.map, &[], None));
            out
        }
        None => render_touch_map(&entry.map, accounts, manifest_json),
    }
}

#[cfg(test)]
mod cpi_event_tests {
    use super::*;

    const EVENT_MANIFEST: &str = r#"{
        "events": [
            {
                "name": "DepositReceipt",
                "tag": 2,
                "fields": [
                    { "name": "balance", "type": "WireU64", "size": 8, "offset": 0 },
                    { "name": "deposit_count", "type": "WireU32", "size": 4, "offset": 8 }
                ]
            }
        ]
    }"#;

    /// Wire bytes for tag 2, balance = 5_000_000, count = 42.
    fn sample_wire() -> Vec<u8> {
        let mut b = vec![CPI_EVENT_MARKER_0, CPI_EVENT_MARKER_1, 0x02];
        b.extend_from_slice(&5_000_000u64.to_le_bytes());
        b.extend_from_slice(&42u32.to_le_bytes());
        b
    }

    fn meta_with_inner(target: &str, data: &[u8]) -> Value {
        serde_json::json!({
            "innerInstructions": [
                {
                    "index": 0,
                    "instructions": [
                        {
                            "programId": target,
                            "accounts": ["Auth1111"],
                            "data": bs58::encode(data).into_string()
                        }
                    ]
                }
            ]
        })
    }

    #[test]
    fn extracts_and_authenticates_a_self_cpi_event() {
        let meta = meta_with_inner("Prog1111", &sample_wire());
        let top = vec![("Prog1111".to_string(), vec![])];
        let events = extract_cpi_events(&meta, &top);
        assert_eq!(events.len(), 1);
        assert!(events[0].self_cpi, "target == parent must read as self-CPI");
        assert_eq!(events[0].tag, 0x02);
        assert_eq!(events[0].payload.len(), 12);

        let lines = render_event(&events[0], Some(EVENT_MANIFEST));
        assert!(lines[0].contains("sink-authenticated"));
        assert_eq!(lines[1], "event: DepositReceipt (tag 0x02)");
        assert_eq!(lines[2], "  balance: 5000000 (WireU64)");
        assert_eq!(lines[3], "  deposit_count: 42 (WireU32)");
    }

    #[test]
    fn foreign_target_marker_is_labeled_not_authenticated() {
        let meta = meta_with_inner("Other2222", &sample_wire());
        let top = vec![("Prog1111".to_string(), vec![])];
        let events = extract_cpi_events(&meta, &top);
        assert_eq!(events.len(), 1);
        assert!(!events[0].self_cpi);
        let lines = render_event(&events[0], None);
        assert!(lines[0].contains("FOREIGN"));
        assert!(
            lines[1].starts_with("tag 0x02, payload 12 bytes: 0x"),
            "no manifest: honest hex fallback; got {}",
            lines[1]
        );
    }

    #[test]
    fn non_marker_and_short_inner_data_are_ignored() {
        // Wrong second byte.
        let meta = meta_with_inner("Prog1111", &[0xE0, 0x77, 0x02, 1, 2]);
        let top = vec![("Prog1111".to_string(), vec![])];
        assert!(extract_cpi_events(&meta, &top).is_empty());
        // Marker but no tag byte.
        let meta = meta_with_inner("Prog1111", &[0xE0, 0x1E]);
        assert!(extract_cpi_events(&meta, &top).is_empty());
    }

    #[test]
    fn manifest_payload_mismatch_is_reported_not_padded() {
        // Payload shorter than the manifest's field spans.
        let mut wire = vec![CPI_EVENT_MARKER_0, CPI_EVENT_MARKER_1, 0x02];
        wire.extend_from_slice(&[1, 2, 3]);
        let meta = meta_with_inner("Prog1111", &wire);
        let top = vec![("Prog1111".to_string(), vec![])];
        let events = extract_cpi_events(&meta, &top);
        let lines = render_event(&events[0], Some(EVENT_MANIFEST));
        assert!(
            lines[2].contains("but payload is 3 bytes"),
            "mismatch must be explicit; got {}",
            lines[2]
        );
    }

    #[test]
    fn unknown_tag_falls_back_to_hex() {
        let mut wire = vec![CPI_EVENT_MARKER_0, CPI_EVENT_MARKER_1, 0x77];
        wire.extend_from_slice(&[0xAB, 0xCD]);
        let meta = meta_with_inner("Prog1111", &wire);
        let top = vec![("Prog1111".to_string(), vec![])];
        let events = extract_cpi_events(&meta, &top);
        let lines = render_event(&events[0], Some(EVENT_MANIFEST));
        assert_eq!(lines[1], "tag 0x77, payload 2 bytes: 0xabcd");
    }

    #[test]
    fn scalar_renderer_handles_wire_ints_and_falls_back_to_hex() {
        assert_eq!(render_scalar("WireU64", &7u64.to_le_bytes()), "7");
        assert_eq!(render_scalar("WireU32", &42u32.to_le_bytes()), "42");
        assert_eq!(render_scalar("WireI64", &(-3i64).to_le_bytes()), "-3");
        assert_eq!(render_scalar("u16", &513u16.to_le_bytes()), "513");
        // Unknown type or size mismatch: hex, never a guess.
        assert_eq!(render_scalar("[u8; 2]", &[0xDE, 0xAD]), "0xdead");
        assert_eq!(render_scalar("WireU64", &[1, 2]), "0x0102", "size mismatch");
    }
}

#[cfg(test)]
mod touch_map_tests {
    use super::*;

    /// Build a wire-format touch map from (slot, offset, size, write).
    fn encode(records: &[(u8, u32, u32, bool)], flags: u8) -> Vec<u8> {
        let mut bytes = vec![
            TOUCH_MAP_MAGIC,
            TOUCH_MAP_VERSION,
            flags,
            records.len() as u8,
        ];
        for &(slot, offset, size, write) in records {
            bytes.push(slot);
            let packed = offset | if write { 0x8000_0000 } else { 0 };
            bytes.extend_from_slice(&packed.to_le_bytes());
            bytes.extend_from_slice(&size.to_le_bytes());
        }
        bytes
    }

    const SAMPLE_MANIFEST: &str = r#"{
        "layouts": [
            {
                "name": "Vault",
                "disc": 1,
                "fields": [
                    { "name": "authority", "type": "Pubkey", "size": 32, "offset": 16 },
                    { "name": "balance", "type": "u64", "size": 8, "offset": 48 }
                ]
            }
        ]
    }"#;

    #[test]
    fn decodes_a_synthetic_touch_map() {
        let bytes = encode(&[(1, 48, 8, true), (0, 16, 32, false)], 0);
        let map = decode_touch_map(&bytes).unwrap();
        assert!(!map.overflowed);
        assert!(!map.skipped);
        assert_eq!(
            map.records,
            vec![
                TouchRecord {
                    slot: 1,
                    offset: 48,
                    size: 8,
                    write: true
                },
                TouchRecord {
                    slot: 0,
                    offset: 16,
                    size: 32,
                    write: false
                },
            ]
        );
    }

    #[test]
    fn rejects_wrong_magic_version_and_length() {
        let good = encode(&[(0, 8, 8, false)], 0);
        assert!(decode_touch_map(&good).is_some());

        let mut bad_magic = good.clone();
        bad_magic[0] = 0x7B;
        assert!(decode_touch_map(&bad_magic).is_none());

        let mut bad_version = good.clone();
        bad_version[1] = 0x02;
        assert!(decode_touch_map(&bad_version).is_none());

        // Exact-length equation: truncated or padded payloads are not
        // touch maps (this is what keeps Anchor events out).
        assert!(decode_touch_map(&good[..good.len() - 1]).is_none());
        let mut padded = good.clone();
        padded.push(0);
        assert!(decode_touch_map(&padded).is_none());
        assert!(decode_touch_map(&[]).is_none());
    }

    #[test]
    fn extracts_touch_maps_from_a_synthetic_log_stream() {
        let map_bytes = encode(&[(1, 48, 8, true)], 0);
        let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &map_bytes);
        // An Anchor-style event payload that must NOT decode as a map.
        let event_b64 =
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, b"\x7a\x01junk");
        let ix0_data = format!("Program data: {event_b64}");
        let ix1_data = format!("Program data: {b64}");
        let logs = vec![
            "Program Prog1111 invoke [1]",
            ix0_data.as_str(),
            "Program Prog1111 success",
            "Program Prog2222 invoke [1]",
            "Program log: doing work",
            ix1_data.as_str(),
            "Program Prog2222 success",
        ];
        let maps = extract_touch_maps(&logs);
        assert_eq!(maps.len(), 1);
        assert_eq!(maps[0].top_ix, 1, "attributed to the second top-level ix");
        assert_eq!(maps[0].cpi_program, None, "emitted at depth 1");
        assert_eq!(maps[0].map.records.len(), 1);
        assert_eq!(maps[0].map.records[0].offset, 48);
    }

    #[test]
    fn structural_line_parser_ignores_program_emitted_text() {
        // Runtime-generated structural lines parse.
        assert!(matches!(
            parse_structural_line("Program Prog1111 invoke [1]"),
            Some(StructuralLine::Invoke {
                program: "Prog1111",
                depth: 1
            })
        ));
        assert!(matches!(
            parse_structural_line("Program Prog1111 invoke [3]"),
            Some(StructuralLine::Invoke { depth: 3, .. })
        ));
        assert!(matches!(
            parse_structural_line("Program Prog1111 success"),
            Some(StructuralLine::Exit)
        ));
        assert!(matches!(
            parse_structural_line("Program Prog1111 failed: custom program error: 0x1"),
            Some(StructuralLine::Exit)
        ));
        // Program-emitted text never parses: `log:` / `data:` are not
        // base58 tokens, so `msg!("Program X invoke [1]")` cannot forge
        // a structural line.
        assert!(parse_structural_line("Program log: Program Prog1111 invoke [1]").is_none());
        assert!(parse_structural_line("Program log: Program Prog1111 success").is_none());
        assert!(parse_structural_line("Program data: AAAA").is_none());
        // Non-structural runtime lines do not advance state either.
        assert!(
            parse_structural_line("Program Prog1111 consumed 1234 of 200000 compute units")
                .is_none()
        );
        // `0`, `O`, `I`, `l` are outside the base58 alphabet.
        assert!(parse_structural_line("Program l0g0 invoke [1]").is_none());
    }

    #[test]
    fn spoofed_invoke_lines_do_not_shift_attribution() {
        let map_bytes = encode(&[(0, 16, 32, false)], 0);
        let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &map_bytes);
        let data_line = format!("Program data: {b64}");
        // Prog1111 tries to shift attribution by logging fake structural
        // text via msg! before emitting its touch map. Runtime prefixes
        // program logs with `Program log:`, so none of it may count.
        let logs = vec![
            "Program Prog1111 invoke [1]",
            "Program log: Program Spoof111 invoke [1]",
            "Program log: Program Spoof111 invoke [2]",
            "Program log: Program Prog1111 success",
            "Program log: Program Spoof111 success",
            data_line.as_str(),
            "Program Prog1111 success",
            "Program Prog2222 invoke [1]",
            "Program Prog2222 success",
        ];
        let maps = extract_touch_maps(&logs);
        assert_eq!(maps.len(), 1);
        assert_eq!(
            maps[0].top_ix, 0,
            "spoofed invoke/success text must not shift the instruction counter"
        );
        assert_eq!(
            maps[0].cpi_program, None,
            "spoofed invoke text must not fake a CPI depth"
        );
    }

    #[test]
    fn cpi_emitted_map_is_tagged_with_the_callee() {
        let map_bytes = encode(&[(1, 48, 8, true)], 0);
        let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &map_bytes);
        let cpi_data = format!("Program data: {b64}");
        let top_data = format!("Program data: {b64}");
        // Note: ids must be base58-clean (no `l`/`O`/`I`/`0`) or the
        // structural parser rightly rejects the line.
        let logs = vec![
            "Program Ca11er11 invoke [1]",
            "Program Ca11ee22 invoke [2]",
            cpi_data.as_str(),
            "Program Ca11ee22 success",
            top_data.as_str(),
            "Program Ca11er11 success",
        ];
        let maps = extract_touch_maps(&logs);
        assert_eq!(maps.len(), 2);
        // Emitted while the callee was executing: tagged CPI.
        assert_eq!(maps[0].top_ix, 0);
        assert_eq!(maps[0].cpi_program.as_deref(), Some("Ca11ee22"));
        // Emitted after the callee returned (back at depth 1): top-level.
        assert_eq!(maps[1].top_ix, 0);
        assert_eq!(maps[1].cpi_program, None);
    }

    #[test]
    fn cpi_emitted_map_renders_raw_with_honest_note() {
        let map = decode_touch_map(&encode(&[(1, 48, 8, true)], 0)).unwrap();
        let entry = ExtractedTouchMap {
            top_ix: 0,
            map,
            cpi_program: Some("Ca11ee22".to_string()),
        };
        // Even with the top-level account list and manifest available,
        // a CPI-emitted map must not be joined against either: the
        // slots are callee-relative.
        let accounts = vec!["Auth1111".to_string(), "Vault2222".to_string()];
        let lines = render_touch_map_entry(&entry, &accounts, Some(SAMPLE_MANIFEST));
        assert_eq!(lines.len(), 2);
        assert_eq!(
            lines[0],
            "(emitted by CPI callee Ca11ee22; slots are callee-relative, \
             field names unavailable)"
        );
        assert_eq!(lines[1], "W slot 1 [48..56)");
        assert!(
            !lines[1].contains("Vault2222") && !lines[1].contains("Vault.balance"),
            "no pubkey or field join for callee-relative slots"
        );

        // The same map at depth 1 does get the field join.
        let top_entry = ExtractedTouchMap {
            cpi_program: None,
            ..entry
        };
        let lines = render_touch_map_entry(&top_entry, &accounts, Some(SAMPLE_MANIFEST));
        assert_eq!(
            lines,
            vec!["W slot 1 (Vault2222) [48..56) -> Vault.balance"]
        );
    }

    #[test]
    fn renders_field_level_effects_from_the_manifest() {
        let map = decode_touch_map(&encode(&[(1, 48, 8, true), (0, 200, 4, false)], 0)).unwrap();
        let accounts = vec!["Auth1111".to_string(), "Vault2222".to_string()];
        let lines = render_touch_map(&map, &accounts, Some(SAMPLE_MANIFEST));
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0], "W slot 1 (Vault2222) [48..56) -> Vault.balance");
        // No field spans [200..204): honest raw-range fallback.
        assert_eq!(lines[1], "R slot 0 (Auth1111) [200..204)");
    }

    #[test]
    fn renders_honest_trailers_for_partial_maps() {
        let map = decode_touch_map(&encode(
            &[(0, 16, 32, false)],
            TOUCH_MAP_FLAG_OVERFLOWED | TOUCH_MAP_FLAG_SKIPPED,
        ))
        .unwrap();
        let lines = render_touch_map(&map, &[], Some(SAMPLE_MANIFEST));
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0], "R slot 0 [16..48) -> Vault.authority");
        assert!(lines[1].contains("...map truncated at 32 records"));
        assert!(lines[2].contains("skipped at least one touched range"));
    }

    /// A `Program data:` log line carrying a base64 touch map, exactly as
    /// the runtime emits it.
    fn data_line(records: &[(u8, u32, u32, bool)]) -> String {
        let b64 = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            encode(records, 0),
        );
        format!("Program data: {b64}")
    }

    #[test]
    fn cpi_tree_reconstructs_a_nested_chain_with_per_frame_cu() {
        let logs = vec![
            "Program Aaa invoke [1]",
            "Program log: outer",
            "Program Bbb invoke [2]",
            "Program Bbb consumed 400 of 200000 compute units",
            "Program Bbb success",
            "Program Aaa consumed 1203 of 200000 compute units",
            "Program Aaa success",
        ];
        let tree = build_cpi_tree(&logs);
        assert_eq!(tree.len(), 1, "one top-level frame");
        let root = &tree[0];
        assert_eq!(root.program, "Aaa");
        assert_eq!(root.depth, 1);
        assert_eq!(root.outcome, FrameOutcome::Success);
        assert_eq!(root.cu_consumed, Some(1203));
        assert_eq!(root.children.len(), 1, "one nested callee");
        let child = &root.children[0];
        assert_eq!(child.program, "Bbb");
        assert_eq!(child.depth, 2);
        assert_eq!(child.cu_consumed, Some(400));
        assert_eq!(child.outcome, FrameOutcome::Success);
    }

    #[test]
    fn cpi_tree_captures_the_failure_reason_on_the_refused_frame() {
        // The exact shape of the live Sentinel refusal. (The fake id must
        // be base58-clean — lowercase `l` is not in the alphabet, which
        // the structural-line guard rightly enforces.)
        let logs = vec![
            "Program SentineL1111 invoke [1]",
            "Program SentineL1111 consumed 616 of 1400000 compute units",
            "Program SentineL1111 failed: custom program error: 0xd001",
        ];
        let tree = build_cpi_tree(&logs);
        assert_eq!(tree.len(), 1);
        assert_eq!(
            tree[0].outcome,
            FrameOutcome::Failed("custom program error: 0xd001".to_string())
        );
        assert_eq!(tree[0].cu_consumed, Some(616));
        let rendered = render_cpi_tree(&tree).join("\n");
        assert!(rendered.contains("FAILED: custom program error: 0xd001"));
        assert!(rendered.contains("616 CU"));
    }

    #[test]
    fn cpi_tree_attaches_touch_maps_to_the_exact_emitting_frame() {
        // The callee Bbb emits a touch map; the parent Aaa does not. The
        // map must hang under Bbb, not Aaa.
        let map_line = data_line(&[(1, 114, 1, true), (1, 115, 8, true)]);
        let logs = vec![
            "Program Aaa invoke [1]",
            "Program Bbb invoke [2]",
            map_line.as_str(),
            "Program Bbb success",
            "Program Aaa success",
        ];
        let tree = build_cpi_tree(&logs);
        assert!(tree[0].touch_maps.is_empty(), "parent emitted none");
        let child = &tree[0].children[0];
        assert_eq!(child.touch_maps.len(), 1, "callee emitted one");
        assert_eq!(child.touch_maps[0].records.len(), 2);
        let rendered = render_cpi_tree(&tree).join("\n");
        assert!(rendered.contains("touch map: 2 writes, 0 reads"));
        assert!(rendered.contains("W slot 1 [114..115)"));
        assert!(rendered.contains("W slot 1 [115..123)"));
    }

    #[test]
    fn cpi_tree_frame_state_cannot_be_spoofed_by_program_logs() {
        // A program logs text that mimics a structural invoke line. It
        // arrives behind `log:` (not a base58 program token), so it must
        // NOT open a phantom frame.
        let logs = vec![
            "Program Aaa invoke [1]",
            "Program log: Program Zzz invoke [2]",
            "Program log: Program Zzz success",
            "Program Aaa success",
        ];
        let tree = build_cpi_tree(&logs);
        assert_eq!(tree.len(), 1);
        assert!(
            tree[0].children.is_empty(),
            "the spoofed invoke text must not create a child frame"
        );
    }

    #[test]
    fn cpi_tree_unwinds_a_truncated_stream_as_unterminated() {
        // A log stream cut off mid-CPI (RPC truncation) still yields a
        // well-formed tree; the open frames are marked Unterminated.
        let logs = vec!["Program Aaa invoke [1]", "Program Bbb invoke [2]"];
        let tree = build_cpi_tree(&logs);
        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].outcome, FrameOutcome::Unterminated);
        assert_eq!(tree[0].children.len(), 1);
        assert_eq!(tree[0].children[0].outcome, FrameOutcome::Unterminated);
    }
}
