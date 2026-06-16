//! Cluster feature-gate inspection.
//!
//! Hopper compiles some fast paths against Solana features that ship
//! behind a runtime feature gate (today: SIMD-0321, the `r2`
//! instruction-data pointer used by `hopper::fast_entrypoint!` under the
//! `simd-0321` cargo feature). A program built to *assume* an unactivated
//! feature is unsound on a cluster that has not enabled it.
//!
//! `hopper feature-gate` checks a feature account on the target cluster
//! and reports whether it is active, so a `--features simd-0321` build is
//! only shipped where the gate is live. No other Solana framework ties
//! its compile-time configuration to on-chain feature-gate state.

use crate::cmd::cluster::cluster_url;
use crate::rpc;

/// SIMD-0321: VM `r2` instruction-data pointer at entrypoint. Backs the
/// `simd-0321` cargo feature / `hopper::fast_entrypoint!` fast path.
pub const SIMD_0321_GATE: &str = "5xXZc66h4UdB6Yq7FzdBxBiRAFMMScMLwHxk2QZDaNZL";

/// Feature accounts Hopper knows how to reason about, as
/// `(simd, gate_pubkey, what_it_unlocks)`.
pub const KNOWN_GATES: &[(&str, &str, &str)] = &[(
    "SIMD-0321",
    SIMD_0321_GATE,
    "r2 instruction-data pointer (hopper::fast_entrypoint! / --features simd-0321)",
)];

/// Activation state of a feature gate on a cluster.
#[derive(Debug, PartialEq, Eq)]
pub enum GateStatus {
    /// Feature account is present and `activated_at` is set.
    Active(u64),
    /// Feature account is present but not yet activated (staged).
    Pending,
    /// No feature account exists at the gate address.
    NotPresent,
}

/// Query a feature gate's activation status on a cluster.
///
/// The feature account data is bincode `Feature { activated_at:
/// Option<u64> }`: a 1-byte option tag, then (when `1`) the u64 LE
/// activation slot.
pub fn gate_status(rpc_url: &str, gate_pubkey: &str) -> Result<GateStatus, String> {
    match rpc::get_account_info(rpc_url, gate_pubkey)? {
        None => Ok(GateStatus::NotPresent),
        Some(info) => {
            let data = &info.data;
            match data.first() {
                Some(1) if data.len() >= 9 => {
                    let slot = u64::from_le_bytes([
                        data[1], data[2], data[3], data[4], data[5], data[6], data[7], data[8],
                    ]);
                    Ok(GateStatus::Active(slot))
                }
                Some(0) | Some(1) => Ok(GateStatus::Pending),
                // Account exists but is not a recognizable Feature account
                // (e.g. zero-length): treat as pending rather than active.
                _ => Ok(GateStatus::Pending),
            }
        }
    }
}

/// `hopper feature-gate [--cluster <c>] [<gate-pubkey>]`
///
/// With no pubkey, reports every gate in [`KNOWN_GATES`].
pub fn cmd_feature_gate(args: &[String]) {
    let mut cluster = "devnet".to_string();
    let mut explicit_gate: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--cluster" | "--url" | "-u" => {
                if let Some(v) = args.get(i + 1) {
                    cluster = v.clone();
                }
                i += 2;
            }
            other => {
                explicit_gate = Some(other.to_string());
                i += 1;
            }
        }
    }

    let (url, label, _is_mainnet) = match cluster_url(&cluster) {
        Some(t) => t,
        None => {
            eprintln!("hopper feature-gate: unknown cluster '{cluster}'");
            std::process::exit(1);
        }
    };

    println!("Feature gates on {label} ({url}):");
    println!();

    let gates: Vec<(String, String, String)> = match &explicit_gate {
        Some(g) => vec![("(custom)".to_string(), g.clone(), String::new())],
        None => KNOWN_GATES
            .iter()
            .map(|(s, g, d)| (s.to_string(), g.to_string(), d.to_string()))
            .collect(),
    };

    for (simd, gate, desc) in &gates {
        match gate_status(&url, gate) {
            Ok(GateStatus::Active(slot)) => {
                println!("  [active]  {simd}  {gate}");
                println!("            activated at slot {slot}");
            }
            Ok(GateStatus::Pending) => {
                println!("  [pending] {simd}  {gate}");
                println!("            staged but NOT active — do not ship a build that assumes it");
            }
            Ok(GateStatus::NotPresent) => {
                println!("  [absent]  {simd}  {gate}");
                println!("            no feature account — not available on this cluster");
            }
            Err(e) => {
                println!("  [error]   {simd}  {gate}");
                println!("            {e}");
            }
        }
        if !desc.is_empty() {
            println!("            unlocks: {desc}");
        }
        println!();
    }

    if explicit_gate.is_none() {
        println!(
            "Build with `--features simd-0321` only when SIMD-0321 shows [active] on your target cluster."
        );
    }
}
