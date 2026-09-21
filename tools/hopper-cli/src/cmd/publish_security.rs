//! `hopper publish-security`: publish, scaffold, or read back a program's
//! `security.txt` record through the Program Metadata program.
//!
//! The record lives at the canonical PDA `[program, "security"]` under
//! `ProgM6JCCvbYkfKqJYHePx4xxSUSqJp7rh8Lyv7nk7S`, stored as Utf8 + Zlib +
//! Json, exactly what the reference JS client writes for a `security.json`
//! file and what Solana Explorer's entity inspector reads
//! (`PMP_SECURITY_SEED = 'security'`, canonical only). Anchor's `anchor init`
//! writes the same file and shells out to `npx @solana-program/program-metadata`
//! to upload it; this command does the whole job in Rust over the signed
//! send path `publish-idl` already proves.
//!
//! The document is validated before it is compressed. The consumer parser
//! (`@solana/security-txt`) accepts every key as optional and ignores what it
//! does not know, so a typo'd key would publish as invisible bytes; unknown
//! keys are therefore refused, values must be strings or arrays of strings,
//! and the four fields the neodyme convention requires (`name`,
//! `project_url`, `contacts`, `policy`) must be present and non-empty unless
//! `--allow-incomplete` says otherwise.
//!
//! ```text
//! hopper publish-security --init                       # write a security.json template
//! hopper publish-security --file security.json --program-id <id> --dry-run
//! hopper publish-security --file security.json --program-id <id> --cluster devnet
//! hopper publish-security --read --program-id <id> --cluster devnet
//! ```

use std::process;

use super::publish_idl::{
    fetch_published_record, pad_seed, render_dry_run_record, resolve_publish_target,
    run_publish_send, PreparedPayload, PublishRecord, COMPRESSION_ZLIB, DATA_SOURCE_DIRECT,
    ENCODING_UTF8, FORMAT_JSON, SEED_LEN,
};

/// Seed of the security record, as the reference client and the Explorer
/// spell it (the bare word, not `security.txt`).
pub const SECURITY_SEED: &[u8] = b"security";

/// Default template path for `--init`.
pub const DEFAULT_SECURITY_FILE: &str = "security.json";

/// The twelve neodyme `security.txt` keys plus the five Program Metadata
/// extras, the union `@solana/security-txt` parses.
pub const KNOWN_KEYS: &[&str] = &[
    "name",
    "project_url",
    "contacts",
    "policy",
    "preferred_languages",
    "encryption",
    "source_code",
    "source_release",
    "source_revision",
    "auditors",
    "acknowledgements",
    "expiry",
    "logo",
    "description",
    "notification",
    "sdk",
    "version",
];

/// Keys the neodyme convention requires.
pub const REQUIRED_KEYS: &[&str] = &["name", "project_url", "contacts", "policy"];

/// A validated document ready to publish: the minified JSON plus the notes
/// a reviewer should see before signing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecurityDocument {
    /// Minified JSON, keys sorted, the bytes that get compressed.
    pub minified: String,
    /// Keys present with a non-empty value, sorted.
    pub keys: Vec<String>,
    /// Non-fatal observations (missing optional fields, an expiry date).
    pub warnings: Vec<String>,
}

/// The `security.json` scaffold: every known key, empty, so the required
/// check refuses it until it is filled in.
pub fn security_template() -> String {
    let mut out = String::from("{\n");
    let fields: &[(&str, &str)] = &[
        ("name", "\"\""),
        ("project_url", "\"\""),
        ("contacts", "[]"),
        ("policy", "\"\""),
        ("preferred_languages", "[]"),
        ("encryption", "\"\""),
        ("source_code", "\"\""),
        ("source_release", "\"\""),
        ("source_revision", "\"\""),
        ("auditors", "[]"),
        ("acknowledgements", "\"\""),
        ("expiry", "\"\""),
        ("logo", "\"\""),
        ("description", "\"\""),
        ("notification", "\"\""),
        ("sdk", "\"\""),
        ("version", "\"\""),
    ];
    for (i, (key, value)) in fields.iter().enumerate() {
        let comma = if i + 1 < fields.len() { "," } else { "" };
        out.push_str(&format!("  \"{key}\": {value}{comma}\n"));
    }
    out.push_str("}\n");
    out
}

/// Whether a JSON value carries content: a non-empty string, or an array
/// with at least one non-empty string.
fn has_content(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::String(s) => !s.trim().is_empty(),
        serde_json::Value::Array(items) => items
            .iter()
            .any(|item| matches!(item, serde_json::Value::String(s) if !s.trim().is_empty())),
        _ => false,
    }
}

/// Validate and minify a `security.json` document.
pub fn prepare_security_json(
    raw: &str,
    allow_incomplete: bool,
    allow_unknown_keys: bool,
) -> Result<SecurityDocument, String> {
    let value: serde_json::Value =
        serde_json::from_str(raw).map_err(|e| format!("security.json is not valid JSON: {e}"))?;
    let object = value
        .as_object()
        .ok_or_else(|| "security.json must be a JSON object at the top level".to_string())?;

    let mut unknown: Vec<&str> = Vec::new();
    let mut bad_values: Vec<String> = Vec::new();
    let mut keys: Vec<String> = Vec::new();
    for (key, val) in object {
        if !KNOWN_KEYS.contains(&key.as_str()) {
            unknown.push(key);
        }
        let well_typed = match val {
            serde_json::Value::String(_) => true,
            serde_json::Value::Array(items) => items.iter().all(serde_json::Value::is_string),
            _ => false,
        };
        if !well_typed {
            bad_values.push(key.clone());
        }
        if has_content(val) {
            keys.push(key.clone());
        }
    }
    if !unknown.is_empty() && !allow_unknown_keys {
        return Err(format!(
            "unknown key(s) {}: consumers ignore what they do not know, so a typo would publish \
             silently. Known keys: {}. Pass --allow-unknown-keys to publish anyway.",
            unknown
                .iter()
                .map(|k| format!("`{k}`"))
                .collect::<Vec<_>>()
                .join(", "),
            KNOWN_KEYS.join(", ")
        ));
    }
    if !bad_values.is_empty() {
        return Err(format!(
            "value(s) for {} must be a string or an array of strings",
            bad_values
                .iter()
                .map(|k| format!("`{k}`"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }

    let missing: Vec<&str> = REQUIRED_KEYS
        .iter()
        .copied()
        .filter(|k| !keys.iter().any(|present| present == k))
        .collect();
    let mut warnings = Vec::new();
    if !missing.is_empty() {
        let message = format!(
            "required field(s) missing or empty: {}",
            missing
                .iter()
                .map(|k| format!("`{k}`"))
                .collect::<Vec<_>>()
                .join(", ")
        );
        if allow_incomplete {
            warnings.push(message);
        } else {
            return Err(format!(
                "{message}. Fill them in, or pass --allow-incomplete to publish a partial record."
            ));
        }
    }
    if let Some(serde_json::Value::String(expiry)) = object.get("expiry") {
        let expiry = expiry.trim();
        let looks_like_date = expiry.len() == 10
            && expiry.char_indices().all(|(i, c)| {
                if i == 4 || i == 7 {
                    c == '-'
                } else {
                    c.is_ascii_digit()
                }
            });
        if !expiry.is_empty() && !looks_like_date {
            warnings.push(format!(
                "`expiry` is {expiry:?}; consumers expect an ISO date such as 2027-01-31"
            ));
        }
    }
    keys.sort();

    let minified = serde_json::to_string(&value).map_err(|e| format!("re-encode JSON: {e}"))?;
    Ok(SecurityDocument {
        minified,
        keys,
        warnings,
    })
}

/// Shared argument set for the record publishers (`publish-security`,
/// `publish-manifest`): the file, the program, the target, the signer,
/// and the mode switches.
#[derive(Debug, Default, Clone)]
pub(crate) struct RecordArgs {
    pub file: Option<String>,
    pub program_id: Option<String>,
    pub cluster: Option<String>,
    pub url: Option<String>,
    pub keypair: Option<String>,
    pub seed: Option<String>,
    pub overwrite: bool,
    pub yes: bool,
    pub dry_run: bool,
    pub read: bool,
    pub init: Option<String>,
    pub allow_incomplete: bool,
    pub allow_unknown_keys: bool,
}

impl RecordArgs {
    /// Parse the shared flags. `file_flags` names the flag(s) that carry the
    /// document path (`--file`/`--security`, or `--manifest`). Returns
    /// `Ok(None)` when `--help` was asked for and printed.
    pub(crate) fn parse(
        args: &[String],
        file_flags: &[&str],
        usage: fn(),
    ) -> Result<Option<Self>, String> {
        let mut out = RecordArgs::default();
        let mut i = 0;
        let value = |i: usize, flag: &str| -> Result<String, String> {
            args.get(i + 1)
                .cloned()
                .ok_or_else(|| format!("{flag} requires a value"))
        };
        while i < args.len() {
            let flag = args[i].as_str();
            match flag {
                f if file_flags.contains(&f) => {
                    out.file = Some(value(i, flag)?);
                    i += 2;
                }
                "--program-id" => {
                    out.program_id = Some(value(i, flag)?);
                    i += 2;
                }
                "--cluster" => {
                    out.cluster = Some(value(i, flag)?);
                    i += 2;
                }
                "--url" | "--rpc" => {
                    out.url = Some(value(i, flag)?);
                    i += 2;
                }
                "--keypair" | "--signer" => {
                    out.keypair = Some(value(i, flag)?);
                    i += 2;
                }
                "--seed" => {
                    out.seed = Some(value(i, flag)?);
                    i += 2;
                }
                "--init" => {
                    // Optional path; the next token is a value unless it is
                    // another flag.
                    match args.get(i + 1) {
                        Some(next) if !next.starts_with("--") => {
                            out.init = Some(next.clone());
                            i += 2;
                        }
                        _ => {
                            out.init = Some(DEFAULT_SECURITY_FILE.to_string());
                            i += 1;
                        }
                    }
                }
                "--overwrite" => {
                    out.overwrite = true;
                    i += 1;
                }
                "--yes" | "-y" => {
                    out.yes = true;
                    i += 1;
                }
                "--dry-run" => {
                    out.dry_run = true;
                    i += 1;
                }
                "--read" => {
                    out.read = true;
                    i += 1;
                }
                "--allow-incomplete" => {
                    out.allow_incomplete = true;
                    i += 1;
                }
                "--allow-unknown-keys" => {
                    out.allow_unknown_keys = true;
                    i += 1;
                }
                "--help" | "-h" => {
                    usage();
                    return Ok(None);
                }
                other => return Err(format!("unknown argument: {other}")),
            }
        }
        Ok(Some(out))
    }

    /// Decode `--program-id` into 32 bytes.
    pub(crate) fn program_id_bytes(&self) -> Result<[u8; 32], String> {
        let text = self
            .program_id
            .as_deref()
            .ok_or_else(|| "--program-id is required".to_string())?;
        let bytes = bs58::decode(text)
            .into_vec()
            .map_err(|e| format!("invalid base58 program id: {e}"))?;
        if bytes.len() != 32 {
            return Err(format!("program id must be 32 bytes, got {}", bytes.len()));
        }
        let mut out = [0u8; 32];
        out.copy_from_slice(&bytes);
        Ok(out)
    }

    /// The 16-byte seed field: `--seed` or the command's default.
    pub(crate) fn seed16(&self, default: &[u8]) -> Result<[u8; SEED_LEN], String> {
        let seed = self.seed.as_deref().map(str::as_bytes).unwrap_or(default);
        if seed.len() > SEED_LEN {
            return Err(format!(
                "seed is {} bytes; the metadata seed field is at most {SEED_LEN} bytes",
                seed.len()
            ));
        }
        Ok(pad_seed(seed))
    }
}

/// Fetch and print the record at `[program, seed]`, then hand back the
/// payload as text. Shared by every `--read`.
pub(crate) fn read_record(
    command: &str,
    args: &RecordArgs,
    seed16: &[u8; SEED_LEN],
) -> Result<(), String> {
    let program_id = args.program_id_bytes()?;
    let env_url = std::env::var("SOLANA_RPC_URL").ok();
    let target = resolve_publish_target(
        args.url.as_deref(),
        args.cluster.as_deref(),
        env_url.as_deref(),
    )?;
    let record = fetch_published_record(&target.url, &program_id, seed16)
        .map_err(|e| super::publish_idl::redact_rpc_error(&target, e))?;
    let program_b58 = bs58::encode(program_id).into_string();
    println!("=== hopper {command} (read) ===");
    println!("rpc              : {}", target.display_url());
    println!("program          : {program_b58}");
    let Some(record) = record else {
        println!(
            "record           : none (no account at the canonical [program, {:?}] PDA)",
            String::from_utf8_lossy(
                &seed16[..seed16.iter().position(|&b| b == 0).unwrap_or(SEED_LEN)]
            )
        );
        return Ok(());
    };
    let header = &record.header;
    println!("metadata PDA     : {}", record.pda_b58);
    println!("seed             : {:?}", header.seed_text());
    println!(
        "canonical        : {}   mutable: {}",
        header.canonical, header.mutable
    );
    println!(
        "authority        : {}",
        header
            .authority
            .map(|a| bs58::encode(a).into_string())
            .unwrap_or_else(|| "none (canonical: the upgrade authority)".to_string())
    );
    println!(
        "encoding         : {}  compression: {}  format: {}  data_source: {}",
        tag_name(header.encoding, &[(ENCODING_UTF8, "utf8")]),
        tag_name(
            header.compression,
            &[(0, "none"), (COMPRESSION_ZLIB, "zlib")]
        ),
        tag_name(
            header.format,
            &[(0, "none"), (FORMAT_JSON, "json"), (2, "yaml"), (3, "toml")]
        ),
        tag_name(
            header.data_source,
            &[(DATA_SOURCE_DIRECT, "direct"), (1, "url"), (2, "external")]
        ),
    );
    println!(
        "payload          : {} stored -> {} bytes   rent {} lamports",
        record.stored_len,
        record.payload.len(),
        record.lamports
    );
    println!();
    let text = String::from_utf8_lossy(&record.payload);
    if header.data_source != DATA_SOURCE_DIRECT {
        println!("pointer          : {text}");
        return Ok(());
    }
    match serde_json::from_str::<serde_json::Value>(&text) {
        Ok(value) => println!(
            "{}",
            serde_json::to_string_pretty(&value).unwrap_or_else(|_| text.into_owned())
        ),
        Err(_) => println!("{text}"),
    }
    Ok(())
}

fn tag_name(tag: u8, names: &[(u8, &str)]) -> String {
    names
        .iter()
        .find(|(t, _)| *t == tag)
        .map(|(_, n)| format!("{n} ({tag})"))
        .unwrap_or_else(|| format!("unknown ({tag})"))
}

fn print_usage() {
    eprintln!("Usage: hopper publish-security --file <security.json> --program-id <pubkey>");
    eprintln!("                               [--cluster <name> | --url <rpc>] [--keypair <path>]");
    eprintln!("                               [--seed <str>] [--overwrite] [--yes] [--dry-run]");
    eprintln!("                               [--allow-incomplete] [--allow-unknown-keys]");
    eprintln!("       hopper publish-security --init [path]");
    eprintln!("       hopper publish-security --read --program-id <pubkey> [--cluster <name> | --url <rpc>]");
    eprintln!();
    eprintln!("Publish a program's security.txt through the Program Metadata program at the");
    eprintln!("canonical [program, \"security\"] PDA, the record Solana Explorer shows on the");
    eprintln!("program page. Same signed-send path as publish-idl, zero Node dependencies.");
    eprintln!();
    eprintln!("The document is validated first: unknown keys are refused (consumers ignore");
    eprintln!("them, so a typo would publish silently), values must be strings or arrays of");
    eprintln!("strings, and name, project_url, contacts, and policy must be present.");
    eprintln!();
    eprintln!("Options:");
    eprintln!("  --file, --security <path>  The security.json to publish");
    eprintln!("  --program-id <pubkey>      Base58 program id the record describes");
    eprintln!("  --cluster <name>           devnet (default), testnet, mainnet-beta, or localnet");
    eprintln!("  --url, --rpc <rpc>         Custom RPC endpoint; treated as potentially mainnet");
    eprintln!("  --keypair <path>           Upgrade-authority + fee-payer keypair");
    eprintln!("                             (default: ~/.config/solana/id.json)");
    eprintln!(
        "  --seed <str>               Metadata seed (default \"security\", padded to 16 bytes)"
    );
    eprintln!("  --overwrite                Rewrite an already-published record (SetData)");
    eprintln!("  --allow-incomplete         Publish without every required field");
    eprintln!("  --allow-unknown-keys       Publish keys outside the security.txt schema");
    eprintln!(
        "  --init [path]              Write a security.json template (default ./security.json)"
    );
    eprintln!("  --read                     Fetch, decode, and print the published record");
    eprintln!("  --yes, -y                  Skip mainnet/custom-RPC confirmation");
    eprintln!(
        "  --dry-run                  Preview the header/PDA/plan; no network access or prompt"
    );
}

/// `hopper publish-security` entry point.
pub fn cmd_publish_security(args: &[String]) {
    let parsed = match RecordArgs::parse(args, &["--file", "--security"], print_usage) {
        Ok(Some(parsed)) => parsed,
        Ok(None) => return,
        Err(e) => {
            eprintln!("publish-security: {e}");
            print_usage();
            process::exit(1);
        }
    };
    if let Err(e) = run(&parsed) {
        eprintln!("publish-security: {e}");
        process::exit(1);
    }
}

fn run(args: &RecordArgs) -> Result<(), String> {
    if let Some(path) = &args.init {
        if std::path::Path::new(path).exists() {
            return Err(format!(
                "{path} already exists; delete it or pass a different path to --init"
            ));
        }
        std::fs::write(path, security_template()).map_err(|e| format!("write {path}: {e}"))?;
        println!("wrote {path}: fill in name, project_url, contacts, and policy, then publish it.");
        return Ok(());
    }

    let seed16 = args.seed16(SECURITY_SEED)?;
    if args.read {
        return read_record("publish-security", args, &seed16);
    }

    let path = args
        .file
        .as_deref()
        .ok_or_else(|| "--file <security.json> is required (or --init / --read)".to_string())?;
    let program_id = args.program_id_bytes()?;
    let raw = std::fs::read_to_string(path).map_err(|e| format!("read {path}: {e}"))?;
    let document = prepare_security_json(&raw, args.allow_incomplete, args.allow_unknown_keys)?;
    for warning in &document.warnings {
        eprintln!("warning: {warning}");
    }

    let payload = PreparedPayload::from_json(&document.minified);
    let record = PublishRecord {
        command: "publish-security",
        noun: "security.txt",
        source: format!("security.txt {path} (fields: {})", document.keys.join(", ")),
    };

    if args.dry_run {
        print!(
            "{}",
            render_dry_run_record(&record, &program_id, &seed16, &payload)
        );
        return Ok(());
    }

    run_publish_send(
        &record,
        &program_id,
        &seed16,
        &payload,
        args.url.as_deref(),
        args.cluster.as_deref(),
        args.keypair.as_deref(),
        args.overwrite,
        args.yes,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const COMPLETE: &str = r#"{
        "name": "Hopper Sentinel",
        "project_url": "https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework",
        "contacts": ["email:security@bluefoot.tech"],
        "policy": "https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/SECURITY.md",
        "preferred_languages": ["en"],
        "source_code": "https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework",
        "expiry": "2027-09-21"
    }"#;

    #[test]
    fn template_lists_every_known_key_and_is_refused_until_filled() {
        let template = security_template();
        let value: serde_json::Value = serde_json::from_str(&template).unwrap();
        let object = value.as_object().unwrap();
        assert_eq!(object.len(), KNOWN_KEYS.len());
        for key in KNOWN_KEYS {
            assert!(object.contains_key(*key), "template lacks {key}");
        }
        let err = prepare_security_json(&template, false, false).unwrap_err();
        assert!(err.contains("`name`") && err.contains("`policy`"), "{err}");
        // The same template publishes as an (empty) partial record on request.
        let doc = prepare_security_json(&template, true, false).unwrap();
        assert!(doc.keys.is_empty());
        assert_eq!(doc.warnings.len(), 1);
    }

    #[test]
    fn complete_document_minifies_and_sorts_keys() {
        let doc = prepare_security_json(COMPLETE, false, false).unwrap();
        assert!(doc.warnings.is_empty(), "{:?}", doc.warnings);
        assert_eq!(
            doc.keys,
            vec![
                "contacts",
                "expiry",
                "name",
                "policy",
                "preferred_languages",
                "project_url",
                "source_code"
            ]
        );
        assert!(!doc.minified.contains('\n'));
        assert!(doc
            .minified
            .starts_with("{\"contacts\":[\"email:security@bluefoot.tech\"]"));
        // Minified bytes are a valid document again with the same content.
        let round: serde_json::Value = serde_json::from_str(&doc.minified).unwrap();
        let original: serde_json::Value = serde_json::from_str(COMPLETE).unwrap();
        assert_eq!(round, original);
    }

    #[test]
    fn unknown_keys_are_refused_unless_allowed() {
        let raw = COMPLETE.replacen("\"contacts\"", "\"contact\"", 1);
        let err = prepare_security_json(&raw, false, false).unwrap_err();
        assert!(err.contains("`contact`"), "{err}");
        // Allowing the key still leaves `contacts` missing.
        let err = prepare_security_json(&raw, false, true).unwrap_err();
        assert!(err.contains("`contacts`"), "{err}");
        let doc = prepare_security_json(&raw, true, true).unwrap();
        assert!(doc.keys.iter().any(|k| k == "contact"));
    }

    #[test]
    fn values_must_be_strings_or_string_arrays() {
        let raw = COMPLETE.replacen("\"2027-09-21\"", "20270921", 1);
        let err = prepare_security_json(&raw, false, false).unwrap_err();
        assert!(err.contains("`expiry`"), "{err}");
        let raw = COMPLETE.replacen("[\"en\"]", "[\"en\", 7]", 1);
        let err = prepare_security_json(&raw, false, false).unwrap_err();
        assert!(err.contains("`preferred_languages`"), "{err}");
        let err = prepare_security_json("[1, 2]", false, false).unwrap_err();
        assert!(err.contains("JSON object"), "{err}");
        let err = prepare_security_json("{not json", false, false).unwrap_err();
        assert!(err.contains("not valid JSON"), "{err}");
    }

    #[test]
    fn expiry_shape_is_a_warning_not_a_refusal() {
        let raw = COMPLETE.replacen("2027-09-21", "next year", 1);
        let doc = prepare_security_json(&raw, false, false).unwrap();
        assert_eq!(doc.warnings.len(), 1);
        assert!(doc.warnings[0].contains("ISO date"));
    }

    #[test]
    fn empty_required_values_count_as_missing() {
        let raw = COMPLETE.replacen("[\"email:security@bluefoot.tech\"]", "[\"\"]", 1);
        let err = prepare_security_json(&raw, false, false).unwrap_err();
        assert!(err.contains("`contacts`"), "{err}");
    }

    #[test]
    fn record_args_parse_modes_and_defaults() {
        fn usage() {}
        let args: Vec<String> = ["--init", "--program-id", "x"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let parsed = RecordArgs::parse(&args, &["--file"], usage)
            .unwrap()
            .unwrap();
        assert_eq!(parsed.init.as_deref(), Some(DEFAULT_SECURITY_FILE));
        assert_eq!(parsed.program_id.as_deref(), Some("x"));

        let args: Vec<String> = ["--init", "out/sec.json", "--read", "--seed", "security"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let parsed = RecordArgs::parse(&args, &["--file"], usage)
            .unwrap()
            .unwrap();
        assert_eq!(parsed.init.as_deref(), Some("out/sec.json"));
        assert!(parsed.read);
        assert_eq!(parsed.seed16(SECURITY_SEED).unwrap(), pad_seed(b"security"));

        let args: Vec<String> = ["--seed", "seventeen-bytes-x"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let parsed = RecordArgs::parse(&args, &["--file"], usage)
            .unwrap()
            .unwrap();
        assert!(parsed
            .seed16(SECURITY_SEED)
            .unwrap_err()
            .contains("at most"));

        let args: Vec<String> = ["--bogus"].iter().map(|s| s.to_string()).collect();
        assert!(RecordArgs::parse(&args, &["--file"], usage).is_err());
    }

    #[test]
    fn program_id_must_be_32_bytes() {
        let args = RecordArgs {
            program_id: Some("ProgM6JCCvbYkfKqJYHePx4xxSUSqJp7rh8Lyv7nk7S".to_string()),
            ..Default::default()
        };
        assert_eq!(args.program_id_bytes().unwrap().len(), 32);
        let args = RecordArgs {
            program_id: Some("abc".to_string()),
            ..Default::default()
        };
        assert!(args.program_id_bytes().is_err());
        assert!(RecordArgs::default().program_id_bytes().is_err());
    }

    #[test]
    fn dry_run_names_the_security_record() {
        let doc = prepare_security_json(COMPLETE, false, false).unwrap();
        let payload = PreparedPayload::from_json(&doc.minified);
        let record = PublishRecord {
            command: "publish-security",
            noun: "security.txt",
            source: "security.txt security.json".to_string(),
        };
        let report = render_dry_run_record(&record, &[7u8; 32], &pad_seed(SECURITY_SEED), &payload);
        assert!(report.contains("=== hopper publish-security (dry run) ==="));
        assert!(report.contains("Seed:               \"security\""));
        assert!(report.contains("security.txt security.json"));
        assert!(report.contains("single inline Initialize"));
    }
}
