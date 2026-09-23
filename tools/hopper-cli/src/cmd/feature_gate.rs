//! Finalized RPC observations for target-cluster deployment assumptions.
//! A report records feature state, not an authenticated ledger proof.

use crate::cmd::cluster::{cluster_url, redact_rpc_url};
use crate::rpc;
use base64::Engine;
use serde::Serialize;
use serde_json::{json, Value};

pub const SIMD_0321_GATE: &str = "5xXZc66h4UdB6Yq7FzdBxBiRAFMMScMLwHxk2QZDaNZL";
pub const SIMD_0339_GATE: &str = "H6iVbVaDZgDphcPbcZwc5LoznMPWQfnJ1AM7L1xzqvt5";
pub const SIMD_0449_GATE: &str = "ptr9umikaeAS7ZBBp2fsfRhie16F1V2jCKA2y6gXNAK";
const FEATURE_PROGRAM: &str = "Feature111111111111111111111111111111111111";

/// Keys checked against Agave c17c5962 on 2026-09-23. A source entry or
/// proposal status alone does not establish activation on a cluster.
pub const KNOWN_GATES: &[(&str, &str, &str)] = &[
    ("SIMD-0321", SIMD_0321_GATE, "r2 instruction-data pointer"),
    ("SIMD-0339", SIMD_0339_GATE, "CPI account-info limit of 255"),
    (
        "SIMD-0385",
        "txv1aq4pp281K9um3tnPgkfX8UqtFT6wcVW3hNezGLL",
        "transaction v1 envelope; legacy/v0 remain 1232 bytes",
    ),
    ("SIMD-0449", SIMD_0449_GATE, "direct account-pointer table"),
    (
        "SIMD-0459",
        "EDGMC5kxFxGk4ixsNkGt8bW7QL5hDMXnbwaZvYMwNfzF",
        "syscall parameter address restrictions",
    ),
    (
        "SIMD-0460",
        "7VgiehxNxu53KdxgLspGQY8myE6f7UokaWa4jsGcaSz",
        "virtual address-space adjustments",
    ),
    (
        "SBPF-v3",
        "5cC3foj77CWun58pC51ebHFUWavHWKarWyR5UUik7dnC",
        "SBPF v3 deployment and execution",
    ),
    (
        "SIMD-0500",
        "B8JJXCy5amZyWG9r7EnUYLwzXSXTxG7GZ1qZ1qggo83g",
        "disable deployment of SBPF v0/v1/v2",
    ),
    (
        "SIMD-0512",
        "s512oDwgx8hjMnaQjXfqqrZroVj4HvC6TkN3iSSWXCh",
        "SHA-512 syscall",
    ),
    (
        "SIMD-0049",
        "5TuppMutoyzhUSfuYdhgzD47F92GL1g89KpCZQKqedxP",
        "remaining-compute-units syscall",
    ),
    (
        "SIMD-0194",
        "rent6iVy6PDoViPBeJ6k5EJQrkj62h7DPyLbWGHwjrC",
        "deprecate rent exemption threshold",
    ),
    (
        "account-data-direct-mapping",
        "CR3dVN2Yoo95Y96kLSTaziWDAQT2MNEpiWh5cqVq2pNE",
        "direct mapping of account data",
    ),
];

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "status", content = "activated_at", rename_all = "snake_case")]
pub enum GateStatus {
    Active(u64),
    Pending,
    NotPresent,
}

#[derive(Debug, Serialize)]
struct GateObservation {
    name: String,
    pubkey: String,
    description: String,
    #[serde(flatten)]
    state: GateStatus,
}

#[derive(Debug, Serialize)]
struct GateReport {
    schema: &'static str,
    cluster: String,
    rpc: String,
    genesis_hash: String,
    commitment: &'static str,
    observed_slot: u64,
    required: Vec<String>,
    requirements_met: bool,
    gates: Vec<GateObservation>,
}

#[derive(Debug, PartialEq, Eq)]
struct GateArgs {
    cluster: String,
    json: bool,
    explicit: Option<String>,
    required: Vec<String>,
}

fn resolve_gate(value: &str) -> Result<(&str, &str, &str), String> {
    if let Some(&(name, pubkey, description)) = KNOWN_GATES
        .iter()
        .find(|(name, pubkey, _)| *name == value || *pubkey == value)
    {
        return Ok((name, pubkey, description));
    }
    rpc::decode_pubkey(value)
        .map_err(|_| format!("unknown feature name or invalid pubkey: {value}"))?;
    Ok(("(custom)", value, "custom feature account"))
}

fn parse_args(args: &[String]) -> Result<GateArgs, String> {
    let mut parsed = GateArgs {
        cluster: "devnet".into(),
        json: false,
        explicit: None,
        required: Vec::new(),
    };
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--cluster" | "--url" | "-u" | "--require" => {
                let value = iter
                    .next()
                    .filter(|v| !v.starts_with('-'))
                    .ok_or_else(|| format!("{arg} requires a value"))?;
                if arg == "--require" {
                    let (_, key, _) = resolve_gate(value)?;
                    if !parsed.required.iter().any(|old| old == key) {
                        parsed.required.push(key.to_string());
                    }
                } else {
                    parsed.cluster = value.clone();
                }
            }
            "--json" => parsed.json = true,
            unknown if unknown.starts_with('-') => {
                return Err(format!("unknown option: {unknown}"))
            }
            gate => {
                resolve_gate(gate)?;
                if parsed.explicit.replace(gate.to_string()).is_some() {
                    return Err(
                        "only one positional feature is accepted; use repeated --require".into(),
                    );
                }
            }
        }
    }
    if parsed.explicit.is_some() && !parsed.required.is_empty() {
        return Err("use either a positional feature or --require, not both".into());
    }
    Ok(parsed)
}

fn request(url: &str, method: &str, params: Value) -> Result<Value, String> {
    let body = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params});
    let response = ureq::post(url)
        .timeout(std::time::Duration::from_secs(30))
        .set("Content-Type", "application/json")
        .send_string(&body.to_string())
        // Endpoint URLs may contain API credentials. Do not print the HTTP
        // library's full error, which includes the request URL.
        .map_err(|_| format!("{method}: RPC request failed"))?;
    let text = response
        .into_string()
        .map_err(|_| format!("{method}: response read failed"))?;
    let value: Value =
        serde_json::from_str(&text).map_err(|_| format!("{method}: invalid JSON"))?;
    if value.get("error").is_some() {
        return Err(format!("{method}: RPC returned an error"));
    }
    value
        .get("result")
        .cloned()
        .ok_or_else(|| format!("{method}: missing result"))
}

fn parse_feature_account(account: &Value, observed_slot: u64) -> Result<GateStatus, String> {
    if account.is_null() {
        return Ok(GateStatus::NotPresent);
    }
    if account.get("owner").and_then(Value::as_str) != Some(FEATURE_PROGRAM)
        || account.get("executable").and_then(Value::as_bool) != Some(false)
    {
        return Err("feature account has the wrong owner or executable flag".into());
    }
    let data = account
        .get("data")
        .and_then(Value::as_array)
        .ok_or("feature account is missing base64 data")?;
    if data.len() != 2 || data[1].as_str() != Some("base64") {
        return Err("feature account has an unexpected encoding".into());
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(
            data[0]
                .as_str()
                .ok_or("feature data must be a base64 string")?,
        )
        .map_err(|_| "feature account has invalid base64")?;
    match bytes.as_slice() {
        [0] | [0, _, _, _, _, _, _, _, _] => Ok(GateStatus::Pending),
        [1, slot @ ..] if slot.len() == 8 => {
            let activated_at = u64::from_le_bytes(slot.try_into().expect("matched eight bytes"));
            if activated_at > observed_slot {
                return Err("feature activation is later than the RPC observation slot".into());
            }
            Ok(GateStatus::Active(activated_at))
        }
        _ => Err("feature account has a malformed activation record".into()),
    }
}

fn parse_snapshot(
    result: &Value,
    gates: &[(&str, &str, &str)],
) -> Result<(u64, Vec<GateObservation>), String> {
    let slot = result
        .pointer("/context/slot")
        .and_then(Value::as_u64)
        .ok_or("feature response is missing its observation slot")?;
    let accounts = result
        .get("value")
        .and_then(Value::as_array)
        .ok_or("feature response is missing its account array")?;
    if accounts.len() != gates.len() {
        return Err("feature response account count does not match request".into());
    }
    let observations = gates
        .iter()
        .zip(accounts)
        .map(|(&(name, key, description), account)| {
            Ok(GateObservation {
                name: name.into(),
                pubkey: key.into(),
                description: description.into(),
                state: parse_feature_account(account, slot)
                    .map_err(|error| format!("{name} ({key}): {error}"))?,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok((slot, observations))
}

fn run(args: &GateArgs) -> Result<GateReport, String> {
    let (url, label, _) = cluster_url(&args.cluster).ok_or("unknown cluster")?;
    let gates: Vec<_> = if let Some(explicit) = &args.explicit {
        vec![resolve_gate(explicit)?]
    } else if !args.required.is_empty() {
        args.required
            .iter()
            .map(|gate| resolve_gate(gate))
            .collect::<Result<_, _>>()?
    } else {
        KNOWN_GATES.to_vec()
    };
    let genesis = request(&url, "getGenesisHash", json!([]))?;
    let genesis_hash = genesis
        .as_str()
        .ok_or("RPC returned an invalid genesis hash")?
        .to_string();
    rpc::decode_pubkey(&genesis_hash).map_err(|_| "RPC returned an invalid genesis hash")?;
    validate_genesis(&label, &genesis_hash)?;
    let keys: Vec<_> = gates.iter().map(|(_, key, _)| *key).collect();
    let result = request(
        &url,
        "getMultipleAccounts",
        json!([
            keys, {"encoding": "base64", "commitment": "finalized"}
        ]),
    )?;
    let (observed_slot, gates) = parse_snapshot(&result, &gates)?;
    let requirements_met = args.required.iter().all(|key| {
        gates
            .iter()
            .any(|gate| gate.pubkey == *key && matches!(gate.state, GateStatus::Active(_)))
    });
    Ok(GateReport {
        schema: "hopper.feature-gates.v1",
        cluster: label,
        rpc: redact_rpc_url(&url),
        genesis_hash,
        commitment: "finalized",
        observed_slot,
        required: args.required.clone(),
        requirements_met,
        gates,
    })
}

fn validate_genesis(cluster: &str, actual: &str) -> Result<(), String> {
    let expected = match cluster {
        "devnet" => "EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG",
        "testnet" => "4uhcVJyU9pJkvQyS88uRDiswHXSCkY3zQawwpjk2NsNY",
        "mainnet-beta" => "5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d",
        // Custom endpoints retain the reported genesis without claiming a
        // named cluster. Local validator genesis hashes vary by ledger.
        _ => return Ok(()),
    };
    if actual == expected {
        Ok(())
    } else {
        Err(format!("RPC genesis does not match {cluster}"))
    }
}

/// Exit 0 after a valid observation, 1 on argument/RPC/data errors, or 2
/// when a --require gate is absent or pending.
pub fn cmd_feature_gate(args: &[String]) {
    let json_requested = args.iter().any(|arg| arg == "--json");
    let result =
        parse_args(args).and_then(|parsed| run(&parsed).map(|report| (parsed.json, report)));
    match result {
        Ok((json, report)) => {
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&report).expect("serialize report")
                );
            } else {
                println!(
                    "Feature gates on {} at finalized slot {}:",
                    report.cluster, report.observed_slot
                );
                println!("Genesis: {}", report.genesis_hash);
                for gate in &report.gates {
                    println!(
                        "  {:?}  {}  {}\n    {}",
                        gate.state, gate.name, gate.pubkey, gate.description
                    );
                }
                println!("RPC observation only. An active gate does not change the transaction format your client emits.");
            }
            if !report.requirements_met {
                std::process::exit(2);
            }
        }
        Err(error) => {
            if json_requested {
                println!(
                    "{}",
                    json!({"schema": "hopper.feature-gates.v1", "error": error})
                );
            } else {
                eprintln!("hopper feature-gate: {error}");
            }
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feature(bytes: &[u8]) -> Value {
        json!({
            "owner": FEATURE_PROGRAM, "executable": false,
            "data": [base64::engine::general_purpose::STANDARD.encode(bytes), "base64"]
        })
    }

    #[test]
    fn named_cluster_cannot_report_another_networks_activation() {
        let devnet = "EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG";
        assert!(validate_genesis("devnet", devnet).is_ok());
        assert!(validate_genesis("mainnet-beta", devnet).is_err());
        assert!(validate_genesis("testnet", devnet).is_err());
        assert!(validate_genesis("custom", devnet).is_ok());
    }

    #[test]
    fn malformed_or_unowned_records_never_prove_activation() {
        let mut bytes = vec![1];
        bytes.extend_from_slice(&123u64.to_le_bytes());
        assert_eq!(
            parse_feature_account(&feature(&bytes), 123),
            Ok(GateStatus::Active(123))
        );
        assert!(parse_feature_account(&feature(&bytes), 122).is_err());
        assert_eq!(
            parse_feature_account(&feature(&[0; 9]), 123),
            Ok(GateStatus::Pending)
        );
        assert_eq!(
            parse_feature_account(&Value::Null, 123),
            Ok(GateStatus::NotPresent)
        );
        for invalid in [&[][..], &[2][..], &[1, 0][..], &[1; 10][..]] {
            assert!(parse_feature_account(&feature(invalid), 123).is_err());
        }
        let mut invalid = feature(&bytes);
        invalid["owner"] = json!("11111111111111111111111111111111");
        assert!(parse_feature_account(&invalid, 123).is_err());
        invalid = feature(&bytes);
        invalid["executable"] = json!(true);
        assert!(parse_feature_account(&invalid, 123).is_err());
        invalid = feature(&bytes);
        invalid["data"][1] = json!("base64+zstd");
        assert!(parse_feature_account(&invalid, 123).is_err());
    }

    #[test]
    fn snapshot_requires_complete_context_and_exact_account_count() {
        let gates = &[KNOWN_GATES[0]];
        let valid = json!({"context": {"slot": 99}, "value": [null]});
        assert!(parse_snapshot(&valid, gates).is_ok());
        for invalid in [
            json!({"value": [null]}),
            json!({"context": {"slot": 99}, "value": []}),
            json!({"context": {"slot": 99}, "value": [null, null]}),
        ] {
            assert!(parse_snapshot(&invalid, gates).is_err());
        }
    }

    #[test]
    fn arguments_cannot_silently_drop_requirements() {
        let args = |parts: &[&str]| parts.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let parsed = parse_args(&args(&[
            "--require",
            "SIMD-0449",
            "--require",
            SIMD_0449_GATE,
            "--json",
        ]))
        .unwrap();
        assert_eq!(parsed.required, [SIMD_0449_GATE]);
        assert!(parsed.json);
        for invalid in [
            vec!["--cluster"],
            vec!["--require"],
            vec!["--requre", "SIMD-0449"],
            vec!["--require", "unknown"],
            vec!["--cluster", "--json"],
            vec!["SIMD-0321", "SIMD-0449"],
            vec!["SIMD-0321", "--require", "SIMD-0449"],
        ] {
            assert!(parse_args(&args(&invalid)).is_err());
        }
        for (_, pubkey, _) in KNOWN_GATES {
            assert!(rpc::decode_pubkey(pubkey).is_ok());
        }
    }
}
