//! Shared `--cluster` / `--keypair` / `--commitment` handling for the
//! deploy-family commands (`deploy`, `upgrade`, `close`, `migrate`).
//!
//! Every command that touches a live cluster goes through here so the
//! mainnet guard is uniform: a mainnet operation is only allowed when
//! the caller passes `--cluster mainnet-beta` *explicitly*, and any
//! destructive op on mainnet (deploy/upgrade/close) prompts for an
//! interactive confirmation unless `--yes` is given. The default
//! cluster is **never** mainnet, so an accidental `hopper deploy` can
//! never push to mainnet from a stray config.

use std::io::{self, Write};
use std::process;

/// A resolved cluster target plus the per-invocation flags that map
/// onto the underlying `solana` CLI.
pub struct ClusterArgs {
    /// RPC URL the operation should target.
    pub url: String,
    /// Human label (`devnet`, `mainnet-beta`, `localnet`, or `custom`).
    pub label: String,
    /// True when the target is mainnet-beta.
    pub is_mainnet: bool,
    /// Optional explicit keypair path (fee payer / authority).
    pub keypair: Option<String>,
    /// Optional commitment level.
    pub commitment: Option<String>,
    /// Skip interactive confirmation prompts.
    pub yes: bool,
    /// Arguments we did not consume, forwarded to `solana`.
    pub passthrough: Vec<String>,
}

/// Translate a `--cluster` value into an RPC URL. Accepts the canonical
/// monikers and a raw URL passthrough.
pub fn cluster_url(moniker: &str) -> Option<(String, String, bool)> {
    let lower = moniker.to_ascii_lowercase();
    match lower.as_str() {
        "devnet" | "d" => Some((
            "https://api.devnet.solana.com".to_string(),
            "devnet".to_string(),
            false,
        )),
        "testnet" | "t" => Some((
            "https://api.testnet.solana.com".to_string(),
            "testnet".to_string(),
            false,
        )),
        "mainnet" | "mainnet-beta" | "m" => Some((
            "https://api.mainnet-beta.solana.com".to_string(),
            "mainnet-beta".to_string(),
            true,
        )),
        "localnet" | "localhost" | "l" => Some((
            "http://127.0.0.1:8899".to_string(),
            "localnet".to_string(),
            false,
        )),
        other if other.starts_with("http://") || other.starts_with("https://") => {
            // A raw URL. Flag it as mainnet only if it obviously points
            // at mainnet, so a custom mainnet RPC still trips the guard.
            let is_mainnet = other.contains("mainnet");
            Some((moniker.to_string(), "custom".to_string(), is_mainnet))
        }
        _ => None,
    }
}

/// Parse the shared cluster flags out of an argument list, leaving the
/// rest in `passthrough`. The default cluster is devnet — never
/// mainnet — so omitting `--cluster` can never target mainnet.
pub fn parse_cluster_args(args: &[String]) -> Result<ClusterArgs, String> {
    let mut url = "https://api.devnet.solana.com".to_string();
    let mut label = "devnet".to_string();
    let mut is_mainnet = false;
    let mut keypair = None;
    let mut commitment = None;
    let mut yes = false;
    let mut passthrough = Vec::new();
    let mut explicit_cluster = false;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--cluster" | "--url" | "-u" => {
                let v = args
                    .get(i + 1)
                    .ok_or_else(|| format!("{} requires a value", args[i]))?;
                let (resolved, lbl, mainnet) = cluster_url(v)
                    .ok_or_else(|| format!("unknown cluster moniker: {v}"))?;
                url = resolved;
                label = lbl;
                is_mainnet = mainnet;
                explicit_cluster = true;
                i += 2;
            }
            "--keypair" | "-k" => {
                keypair = Some(
                    args.get(i + 1)
                        .ok_or_else(|| format!("{} requires a path", args[i]))?
                        .clone(),
                );
                i += 2;
            }
            "--commitment" => {
                commitment = Some(
                    args.get(i + 1)
                        .ok_or_else(|| "--commitment requires a value".to_string())?
                        .clone(),
                );
                i += 2;
            }
            "--yes" | "-y" => {
                yes = true;
                i += 1;
            }
            other => {
                passthrough.push(other.to_string());
                i += 1;
            }
        }
    }

    // Mainnet is opt-in only: it must be named explicitly on the command
    // line, regardless of what the saved Solana config says.
    if is_mainnet && !explicit_cluster {
        return Err(
            "refusing to target mainnet from default config; pass --cluster mainnet-beta explicitly"
                .to_string(),
        );
    }

    Ok(ClusterArgs {
        url,
        label,
        is_mainnet,
        keypair,
        commitment,
        yes,
        passthrough,
    })
}

impl ClusterArgs {
    /// Build the `solana` CLI flags (`--url`, `--keypair`,
    /// `--commitment`) this target implies.
    pub fn solana_flags(&self) -> Vec<String> {
        let mut out = vec!["--url".to_string(), self.url.clone()];
        if let Some(kp) = &self.keypair {
            out.push("--keypair".to_string());
            out.push(kp.clone());
        }
        if let Some(c) = &self.commitment {
            out.push("--commitment".to_string());
            out.push(c.clone());
        }
        out
    }

    /// Gate a destructive operation. On mainnet, require an interactive
    /// "yes" confirmation unless `--yes` was passed. On every other
    /// cluster, proceed.
    pub fn confirm_destructive(&self, action: &str, target: &str) {
        if !self.is_mainnet || self.yes {
            return;
        }
        eprint!(
            "About to {action} on MAINNET-BETA ({target}). This is irreversible. Type 'yes' to continue: "
        );
        let _ = io::stderr().flush();
        let mut line = String::new();
        if io::stdin().read_line(&mut line).is_err() || line.trim() != "yes" {
            eprintln!("aborted.");
            process::exit(1);
        }
    }
}
