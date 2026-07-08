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

/// SIMD-0339: raise the CPI account-info limit from 64 to 255
/// (`increase_cpi_account_info_limit`). Under it, each account-info and
/// instruction-account-meta also carries CU, so passing the fewest infos
/// per CPI becomes a cost axis — which Hopper's [`DynCpi`] pubkey dedup
/// exploits. Backs Hopper's raised `MAX_CPI_ACCOUNTS` ceiling.
///
/// Gate pubkey taken from agave `feature-set/src/lib.rs`
/// (`increase_cpi_account_info_limit`); reconfirm against the activation
/// pubkey for the cluster you target before shipping a build that assumes
/// >64 account-infos.
///
/// [`DynCpi`]: hopper_runtime
pub const SIMD_0339_GATE: &str = "H6iVbVaDZgDphcPbcZwc5LoznMPWQfnJ1AM7L1xzqvt5";

/// Feature accounts Hopper knows how to reason about, as
/// `(simd, gate_pubkey, what_it_unlocks)`.
pub const KNOWN_GATES: &[(&str, &str, &str)] = &[
    (
        "SIMD-0321",
        SIMD_0321_GATE,
        "r2 instruction-data pointer (hopper::fast_entrypoint! / --features simd-0321)",
    ),
    (
        "SIMD-0339",
        SIMD_0339_GATE,
        "CPI account-info limit 64->255 (raised MAX_CPI_ACCOUNTS / DynCpi info dedup)",
    ),
];

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
        Some(info) => Ok(parse_feature_account(&info.data)),
    }
}

/// Decode a present Feature account's data into an activation status.
///
/// Layout is bincode `Feature { activated_at: Option<u64> }`: a 1-byte
/// option tag, then (when the tag is `1`) the u64 LE activation slot. A
/// present-but-unrecognizable account (e.g. zero-length) is treated as
/// pending rather than active, so a build is never shipped on the strength
/// of a malformed gate account.
fn parse_feature_account(data: &[u8]) -> GateStatus {
    match data.first() {
        Some(1) if data.len() >= 9 => {
            let slot = u64::from_le_bytes([
                data[1], data[2], data[3], data[4], data[5], data[6], data[7], data[8],
            ]);
            GateStatus::Active(slot)
        }
        Some(0) | Some(1) => GateStatus::Pending,
        _ => GateStatus::Pending,
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
        println!(
            "SIMD-0339 raises the CPI account-info limit to 255 (and prices each info); rely on >64-info CPIs only where it shows [active]."
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_gates_include_both_simd_0321_and_0339() {
        let simds: std::vec::Vec<&str> = KNOWN_GATES.iter().map(|(s, _, _)| *s).collect();
        assert!(simds.contains(&"SIMD-0321"));
        assert!(simds.contains(&"SIMD-0339"));

        // Each known gate is wired to its declared pubkey constant.
        let by_simd = |name: &str| {
            KNOWN_GATES
                .iter()
                .find(|(s, _, _)| *s == name)
                .map(|(_, g, _)| *g)
        };
        assert_eq!(by_simd("SIMD-0321"), Some(SIMD_0321_GATE));
        assert_eq!(by_simd("SIMD-0339"), Some(SIMD_0339_GATE));
    }

    #[test]
    fn feature_account_parses_activation_states() {
        // Tag 0 => staged, not yet activated.
        assert_eq!(parse_feature_account(&[0]), GateStatus::Pending);

        // Tag 1 + 8-byte LE slot => active at that slot.
        let mut active = std::vec![1u8];
        active.extend_from_slice(&1_234_567_u64.to_le_bytes());
        assert_eq!(
            parse_feature_account(&active),
            GateStatus::Active(1_234_567)
        );

        // Tag 1 but truncated (no full slot) => treated as pending, never
        // as active — a malformed gate must not green-light a build.
        assert_eq!(parse_feature_account(&[1, 0, 0]), GateStatus::Pending);

        // Empty / unrecognizable data => pending, not active.
        assert_eq!(parse_feature_account(&[]), GateStatus::Pending);
    }
}
