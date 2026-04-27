//! `hopper config` subcommand tree.
//!
//! Minimal global config at `$HOME/.hopper/config.toml`. Matches the
//! shape of `quasar config` and `solana config` so users can build
//! muscle memory in one tool and use it in every.
//!
//! Stored keys (all optional, all strings):
//!
//! - `cluster_url` - RPC endpoint. `mainnet`, `devnet`, `localnet`
//!   resolve to canonical URLs; any other value is used verbatim.
//! - `payer` - path to the fee-payer keypair json.
//! - `default_program_id` - base58 program id used when a command
//!   needs one and the user did not pass `--program-id`.
//! - `default_keypair` - path to a program-authority keypair used
//!   when a command needs an upgrade authority.
//! - `default_manifest` - path to a `HopperProgramManifest` json
//!   used when a command needs one.
//!
//! The config is a flat table, no nesting. Commands use it as a
//! fallback only; CLI flags always win.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::process;

pub fn cmd_config(args: &[String]) {
    if args.is_empty() || matches!(args[0].as_str(), "--help" | "-h" | "help") {
        print_usage();
        return;
    }
    match args[0].as_str() {
        "get" => cmd_get(&args[1..]),
        "set" => cmd_set(&args[1..]),
        "list" | "ls" => cmd_list(),
        "reset" => cmd_reset(),
        "path" => cmd_path(),
        other => {
            eprintln!("Unknown config subcommand: {other}");
            print_usage();
            process::exit(1);
        }
    }
}

fn print_usage() {
    eprintln!("Usage: hopper config <subcommand>");
    eprintln!();
    eprintln!("Subcommands:");
    eprintln!("  get <key>                  Print a single config value");
    eprintln!("  set <key> <value>          Store a config value");
    eprintln!("  list                       Print every config key + value");
    eprintln!("  reset                      Remove the config file");
    eprintln!("  path                       Print the config file path");
    eprintln!();
    eprintln!("Known keys:");
    eprintln!("  cluster_url                `mainnet` / `devnet` / `localnet` or a full URL");
    eprintln!("  payer                      Path to fee-payer keypair.json");
    eprintln!("  default_program_id         Program id fallback for commands that need one");
    eprintln!("  default_keypair            Upgrade-authority keypair fallback");
    eprintln!("  default_manifest           Manifest-json fallback for commands that need one");
}

fn cmd_get(args: &[String]) {
    let Some(key) = args.first() else {
        eprintln!("Usage: hopper config get <key>");
        process::exit(1);
    };
    validate_key(key);
    let config = load().unwrap_or_default();
    match config.get(key) {
        Some(v) => println!("{v}"),
        None => {
            eprintln!("(unset)");
            process::exit(1);
        }
    }
}

fn cmd_set(args: &[String]) {
    if args.len() < 2 {
        eprintln!("Usage: hopper config set <key> <value>");
        process::exit(1);
    }
    let key = &args[0];
    let value = &args[1];
    validate_key(key);
    let mut config = load().unwrap_or_default();
    config.insert(key.clone(), value.clone());
    save(&config).unwrap_or_else(|e| {
        eprintln!("failed to write config: {e}");
        process::exit(1);
    });
    println!("{key} = {value}");
}

fn cmd_list() {
    let config = load().unwrap_or_default();
    if config.is_empty() {
        println!("(empty; use `hopper config set <key> <value>`)");
        return;
    }
    let width = config.keys().map(String::len).max().unwrap_or(0);
    for (k, v) in &config {
        println!("{k:<width$}  {v}", width = width);
    }
}

fn cmd_reset() {
    let path = config_path();
    if path.exists() {
        if let Err(e) = fs::remove_file(&path) {
            eprintln!("failed to remove {}: {e}", path.display());
            process::exit(1);
        }
    }
    println!("config cleared");
}

fn cmd_path() {
    println!("{}", config_path().display());
}

const KNOWN_KEYS: &[&str] = &[
    "cluster_url",
    "payer",
    "default_program_id",
    "default_keypair",
    "default_manifest",
];

fn validate_key(key: &str) {
    if !KNOWN_KEYS.contains(&key) {
        eprintln!(
            "warning: unknown config key `{key}`. known keys: {}",
            KNOWN_KEYS.join(", ")
        );
    }
}

fn config_path() -> PathBuf {
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join(".hopper").join("config.toml");
    }
    // Windows fallback.
    if let Ok(profile) = std::env::var("USERPROFILE") {
        return PathBuf::from(profile).join(".hopper").join("config.toml");
    }
    PathBuf::from(".hopper").join("config.toml")
}

/// Load the config. Returns an empty map when the file does not
/// exist; returns the parsed table otherwise. Format is a flat
/// `key = "value"` TOML subset so we can parse it without pulling a
/// full TOML library into this one command.
fn load() -> Option<BTreeMap<String, String>> {
    let path = config_path();
    let text = fs::read_to_string(&path).ok()?;
    Some(parse_flat_toml(&text))
}

fn save(config: &BTreeMap<String, String>) -> std::io::Result<()> {
    let path = config_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut out = String::new();
    for (k, v) in config {
        // TOML escape: double-quote strings, escape backslashes and
        // double-quotes. Good enough for our limited value set.
        let escaped = v.replace('\\', "\\\\").replace('"', "\\\"");
        out.push_str(&format!("{k} = \"{escaped}\"\n"));
    }
    fs::write(&path, out)
}

fn parse_flat_toml(text: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let Some((key, value)) = trimmed.split_once('=') else {
            continue;
        };
        let key = key.trim().to_string();
        let value = value.trim();
        let unquoted = if value.starts_with('"') && value.ends_with('"') && value.len() >= 2 {
            value[1..value.len() - 1]
                .replace("\\\"", "\"")
                .replace("\\\\", "\\")
        } else {
            value.to_string()
        };
        out.insert(key, unquoted);
    }
    out
}

/// Resolve the cluster URL for a config value. Short aliases map to
/// canonical endpoints; anything else is returned verbatim.
pub fn resolve_cluster(value: &str) -> String {
    match value {
        "mainnet" | "mainnet-beta" => "https://api.mainnet-beta.solana.com".into(),
        "devnet" => "https://api.devnet.solana.com".into(),
        "testnet" => "https://api.testnet.solana.com".into(),
        "localnet" | "local" => "http://127.0.0.1:8899".into(),
        other => other.into(),
    }
}
