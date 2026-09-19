use std::{env, path::PathBuf, str::FromStr};

use hopper::prelude::Address;
use hopper_xp_program_a::Vault;
use solana_client::rpc_client::RpcClient;
use solana_commitment_config::CommitmentConfig;
use solana_instruction::{AccountMeta, Instruction};
use solana_keypair::{read_keypair_file, Keypair};
use solana_pubkey::Pubkey;
use solana_signer::Signer;
use solana_transaction::Transaction;

const DEFAULT_RPC: &str = "https://api.devnet.solana.com";
const DEFAULT_AMOUNT: u64 = 42;
const DEVNET_GENESIS: &str = "EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG";
const VAULT_AUTHORITY_OFFSET: usize = hopper::hopper_core::account::HEADER_LEN;
const VAULT_BALANCE_OFFSET: usize = VAULT_AUTHORITY_OFFSET + 32;

#[derive(Clone, Debug, Eq, PartialEq)]
struct SignatureReceipt {
    label: String,
    signature: String,
    finalized_slot: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct VerifiedVault {
    owner: Pubkey,
    authority: Pubkey,
    balance: u64,
    data_len: usize,
    layout_id: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct Provenance {
    source_commit: Option<String>,
    program_a_artifact_sha256: Option<String>,
    program_b_artifact_sha256: Option<String>,
}

fn main() {
    if let Err(err) = run() {
        eprintln!("cross-program devnet runner failed: {err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut rpc_url = DEFAULT_RPC.to_string();
    let mut keypair_path = env::var_os("SOLANA_KEYPAIR").map(PathBuf::from);
    let mut program_a_id: Option<Pubkey> = None;
    let mut program_b_id: Option<Pubkey> = None;
    let mut amount = DEFAULT_AMOUNT;

    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--rpc" => {
                rpc_url = args
                    .next()
                    .ok_or_else(|| "--rpc requires a URL".to_string())?;
            }
            "--keypair" => {
                keypair_path = Some(PathBuf::from(
                    args.next()
                        .ok_or_else(|| "--keypair requires a path".to_string())?,
                ));
            }
            "--program-a" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--program-a requires a pubkey".to_string())?;
                program_a_id = Some(
                    Pubkey::from_str(&value).map_err(|err| format!("--program-a parse: {err}"))?,
                );
            }
            "--program-b" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--program-b requires a pubkey".to_string())?;
                program_b_id = Some(
                    Pubkey::from_str(&value).map_err(|err| format!("--program-b parse: {err}"))?,
                );
            }
            "--amount" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--amount requires a u64".to_string())?;
                amount = value
                    .parse::<u64>()
                    .map_err(|err| format!("--amount parse: {err}"))?;
            }
            "--help" | "-h" => {
                print_usage();
                return Ok(());
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }

    if rpc_url != DEFAULT_RPC {
        return Err("release evidence requires the public devnet RPC endpoint".to_string());
    }

    require_devnet_gate()?;
    let receipt_path = env::var_os("HOPPER_DEVNET_RECEIPT")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| "HOPPER_DEVNET_RECEIPT is required when HOPPER_DEVNET=1".to_string())?;
    let provenance = load_provenance()?;
    let program_a_id = program_a_id.ok_or_else(|| "--program-a is required".to_string())?;
    let program_b_id = program_b_id.ok_or_else(|| "--program-b is required".to_string())?;
    let keypair_path = keypair_path
        .ok_or_else(|| "--keypair is required unless SOLANA_KEYPAIR is set".to_string())?;
    let payer = read_keypair_file(&keypair_path)
        .map_err(|_| "failed to read keypair (path redacted)".to_string())?;
    let vault = Keypair::new();
    let client = RpcClient::new_with_commitment(rpc_url.clone(), CommitmentConfig::finalized());
    let genesis = client
        .get_genesis_hash()
        .map_err(|err| rpc_error("get_genesis_hash", err, &rpc_url))?
        .to_string();
    if genesis != DEVNET_GENESIS {
        return Err(
            "refusing to run the devnet spending proof on a non-devnet cluster".to_string(),
        );
    }
    let version = client
        .get_version()
        .map_err(|err| rpc_error("get_version", err, &rpc_url))?;
    for (label, program_id) in [("program a", program_a_id), ("program b", program_b_id)] {
        require_executable_program(&client, &rpc_url, label, &program_id)?;
    }

    println!("rpc           : {}", redact_rpc_url(&rpc_url));
    println!("program a     : {program_a_id}");
    println!("program b     : {program_b_id}");
    println!("authority     : {}", payer.pubkey());
    println!("vault         : {}", vault.pubkey());
    println!("vault len     : {}", Vault::LEN);
    println!("amount        : {amount}");

    let payer_balance = client
        .get_balance(&payer.pubkey())
        .map_err(|err| rpc_error("get_balance", err, &rpc_url))?;
    println!("balance       : {} lamports", payer_balance);

    let mut signatures = vec![send_instruction(
        &client,
        &rpc_url,
        &payer,
        &[&vault],
        Instruction::new_with_bytes(
            program_a_id,
            &[0],
            vec![
                AccountMeta::new(payer.pubkey(), true),
                AccountMeta::new(vault.pubkey(), true),
                AccountMeta::new_readonly(Pubkey::default(), false),
            ],
        ),
        "program_a:init",
    )?];

    let mut deposit_data = Vec::with_capacity(9);
    deposit_data.push(1);
    deposit_data.extend_from_slice(&amount.to_le_bytes());
    signatures.push(send_instruction(
        &client,
        &rpc_url,
        &payer,
        &[],
        Instruction::new_with_bytes(
            program_a_id,
            &deposit_data,
            vec![
                AccountMeta::new_readonly(payer.pubkey(), true),
                AccountMeta::new(vault.pubkey(), false),
            ],
        ),
        "program_a:deposit",
    )?);

    signatures.push(send_instruction(
        &client,
        &rpc_url,
        &payer,
        &[],
        program_b_instruction(program_b_id, program_a_id, vault.pubkey(), &[0]),
        "program_b:read",
    )?);

    let mut min_balance_data = Vec::with_capacity(9);
    min_balance_data.push(1);
    min_balance_data.extend_from_slice(&amount.to_le_bytes());
    signatures.push(send_instruction(
        &client,
        &rpc_url,
        &payer,
        &[],
        program_b_instruction(
            program_b_id,
            program_a_id,
            vault.pubkey(),
            &min_balance_data,
        ),
        "program_b:min",
    )?);

    let verified = verify_vault(
        &client,
        &rpc_url,
        &vault.pubkey(),
        &program_a_id,
        &payer.pubkey(),
        amount,
    )?;
    let evidence = render_evidence(
        &rpc_url,
        &genesis,
        &version.solana_core,
        version.feature_set,
        &program_a_id,
        &program_b_id,
        &payer.pubkey(),
        &vault.pubkey(),
        &verified,
        &signatures,
        &provenance,
    );
    std::fs::write(&receipt_path, format!("{evidence}\n"))
        .map_err(|_| "failed to write HOPPER_DEVNET_RECEIPT (path redacted)".to_string())?;
    println!("wrote hopper.devnet-evidence.v1 receipt");
    Ok(())
}

fn print_usage() {
    eprintln!(
        "Usage: HOPPER_DEVNET=1 HOPPER_DEVNET_RECEIPT=<path> HOPPER_SOURCE_COMMIT=<sha> HOPPER_XP_PROGRAM_A_SHA256=<sha256> HOPPER_XP_PROGRAM_B_SHA256=<sha256> cargo run -p hopper-xp-devnet-runner -- --program-a <pubkey> --program-b <pubkey> --keypair <path> [--amount <u64>]"
    );
    eprintln!("       SOLANA_KEYPAIR may be used instead of --keypair.");
}

fn require_devnet_gate() -> Result<(), String> {
    validate_devnet_gate(env::var("HOPPER_DEVNET"))
}

fn validate_devnet_gate(value: Result<String, env::VarError>) -> Result<(), String> {
    match value {
        Ok(value) if value == "1" => Ok(()),
        Ok(_) => Err("HOPPER_DEVNET must be exactly 1".to_string()),
        Err(env::VarError::NotPresent) => {
            Err("HOPPER_DEVNET=1 is required for the live devnet runner".to_string())
        }
        Err(env::VarError::NotUnicode(_)) => {
            Err("HOPPER_DEVNET must be valid Unicode and exactly 1".to_string())
        }
    }
}

fn load_provenance() -> Result<Provenance, String> {
    Ok(Provenance {
        source_commit: Some(required_hex_env("HOPPER_SOURCE_COMMIT", 40)?),
        program_a_artifact_sha256: Some(required_hex_env("HOPPER_XP_PROGRAM_A_SHA256", 64)?),
        program_b_artifact_sha256: Some(required_hex_env("HOPPER_XP_PROGRAM_B_SHA256", 64)?),
    })
}

fn required_hex_env(name: &str, expected_len: usize) -> Result<String, String> {
    match env::var(name) {
        Ok(value) => validate_hex_evidence(name, &value, expected_len),
        Err(env::VarError::NotPresent) => Err(format!("{name} is required")),
        Err(env::VarError::NotUnicode(_)) => {
            Err(format!("{name} must be valid Unicode hexadecimal"))
        }
    }
}

fn validate_hex_evidence(name: &str, value: &str, expected_len: usize) -> Result<String, String> {
    if value.len() != expected_len || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!(
            "{name} must contain exactly {expected_len} hexadecimal characters"
        ));
    }
    Ok(value.to_ascii_lowercase())
}

fn require_executable_program(
    client: &RpcClient,
    rpc_url: &str,
    label: &str,
    program_id: &Pubkey,
) -> Result<(), String> {
    let account = client
        .get_account_with_commitment(program_id, CommitmentConfig::finalized())
        .map_err(|err| rpc_error(&format!("fetch {label} account"), err, rpc_url))?
        .value
        .ok_or_else(|| format!("{label} account is absent at finalized"))?;
    if !account.executable {
        return Err(format!("{label} account is not executable"));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn render_evidence(
    rpc_url: &str,
    genesis_hash: &str,
    node_version: &str,
    feature_set: Option<u32>,
    program_a_id: &Pubkey,
    program_b_id: &Pubkey,
    authority: &Pubkey,
    vault: &Pubkey,
    verified: &VerifiedVault,
    signatures: &[SignatureReceipt],
    provenance: &Provenance,
) -> String {
    let transactions = signatures
        .iter()
        .map(|receipt| {
            format!(
                "{{\"name\":{},\"signature\":{},\"finalized_slot\":{},\"outcome\":\"succeeded\"}}",
                json_string(&receipt.label),
                json_string(&receipt.signature),
                receipt.finalized_slot,
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    let feature_set = feature_set
        .map(|value| value.to_string())
        .unwrap_or_else(|| "null".to_string());
    format!(
        "{{\"schema\":\"hopper.devnet-evidence.v1\",\"example\":\"cross-program-read\",\"commitment\":\"finalized\",\"rpc_endpoint\":{},\"cluster\":{{\"genesis_hash\":{},\"node_version\":{},\"feature_set\":{}}},\"programs\":{{\"program_a\":{},\"program_b\":{}}},\"provenance\":{{\"source_commit\":{},\"program_a_artifact_sha256\":{},\"program_b_artifact_sha256\":{}}},\"accounts\":{{\"authority\":{},\"vault\":{}}},\"transactions\":[{}],\"state\":{{\"owner\":{},\"authority\":{},\"data_len\":{},\"layout_id\":{},\"balance\":{}}}}}",
        json_string(&redact_rpc_url(rpc_url)),
        json_string(genesis_hash),
        json_string(node_version),
        feature_set,
        json_string(&program_a_id.to_string()),
        json_string(&program_b_id.to_string()),
        json_optional_string(provenance.source_commit.as_deref()),
        json_optional_string(provenance.program_a_artifact_sha256.as_deref()),
        json_optional_string(provenance.program_b_artifact_sha256.as_deref()),
        json_string(&authority.to_string()),
        json_string(&vault.to_string()),
        transactions,
        json_string(&verified.owner.to_string()),
        json_string(&verified.authority.to_string()),
        verified.data_len,
        json_string(&verified.layout_id),
        verified.balance,
    )
}

fn json_optional_string(value: Option<&str>) -> String {
    value.map(json_string).unwrap_or_else(|| "null".to_string())
}

fn program_b_instruction(
    program_b_id: Pubkey,
    program_a_id: Pubkey,
    vault: Pubkey,
    data: &[u8],
) -> Instruction {
    Instruction::new_with_bytes(
        program_b_id,
        data,
        vec![
            AccountMeta::new_readonly(program_a_id, false),
            AccountMeta::new_readonly(vault, false),
        ],
    )
}

fn send_instruction(
    client: &RpcClient,
    rpc_url: &str,
    payer: &Keypair,
    extra_signers: &[&Keypair],
    instruction: Instruction,
    label: &str,
) -> Result<SignatureReceipt, String> {
    let recent = client
        .get_latest_blockhash()
        .map_err(|err| rpc_error(&format!("{label}: get_latest_blockhash"), err, rpc_url))?;
    let mut signers: Vec<&dyn Signer> = Vec::with_capacity(extra_signers.len() + 1);
    signers.push(payer);
    for signer in extra_signers {
        signers.push(*signer);
    }
    let tx =
        Transaction::new_signed_with_payer(&[instruction], Some(&payer.pubkey()), &signers, recent);
    let sig = client
        .send_and_confirm_transaction_with_spinner_and_commitment(
            &tx,
            CommitmentConfig::finalized(),
        )
        .map_err(|err| {
            rpc_error(
                &format!("{label}: send_and_confirm_transaction"),
                err,
                rpc_url,
            )
        })?;
    let finalized = client
        .get_signature_status_with_commitment(&sig, CommitmentConfig::finalized())
        .map_err(|err| rpc_error(&format!("{label}: finalized status"), err, rpc_url))?;
    match finalized {
        Some(Ok(())) => {}
        Some(Err(err)) => return Err(format!("{label}: finalized transaction failed: {err}")),
        None => return Err(format!("{label}: signature was not visible at finalized")),
    }
    let finalized_slot = client
        .get_signature_statuses_with_history(&[sig])
        .map_err(|err| rpc_error(&format!("{label}: finalized slot"), err, rpc_url))?
        .value
        .into_iter()
        .next()
        .flatten()
        .ok_or_else(|| format!("{label}: finalized signature disappeared from history"))?
        .slot;
    println!("{label:<18}: {sig}");
    Ok(SignatureReceipt {
        label: label.to_string(),
        signature: sig.to_string(),
        finalized_slot,
    })
}

fn verify_vault(
    client: &RpcClient,
    rpc_url: &str,
    vault: &Pubkey,
    program_a_id: &Pubkey,
    authority: &Pubkey,
    expected_balance: u64,
) -> Result<VerifiedVault, String> {
    let account = client
        .get_account_with_commitment(vault, CommitmentConfig::finalized())
        .map_err(|err| rpc_error("fetch vault account", err, rpc_url))?
        .value
        .ok_or_else(|| "vault account is absent at finalized".to_string())?;
    if account.owner != *program_a_id {
        return Err(format!(
            "vault owner mismatch: expected {program_a_id}, got {}",
            account.owner
        ));
    }
    if account.data.len() != Vault::LEN {
        return Err(format!(
            "vault data length mismatch: expected {}, got {}",
            Vault::LEN,
            account.data.len()
        ));
    }

    let authority_bytes = read_32(&account.data, VAULT_AUTHORITY_OFFSET)?;
    if authority_bytes != authority.to_bytes() {
        return Err("vault authority mismatch".to_string());
    }
    let balance = read_u64(&account.data, VAULT_BALANCE_OFFSET)?;
    if balance != expected_balance {
        return Err(format!(
            "vault balance mismatch: expected {expected_balance}, got {balance}"
        ));
    }

    let program_a_addr = Address::new_from_array(program_a_id.to_bytes());
    println!(
        "verified      : owner={}, balance={}, layout={:02x?}",
        program_a_addr,
        balance,
        Vault::LAYOUT_ID
    );
    Ok(VerifiedVault {
        owner: account.owner,
        authority: *authority,
        balance,
        data_len: account.data.len(),
        layout_id: hex(&Vault::LAYOUT_ID),
    })
}

fn read_32(data: &[u8], offset: usize) -> Result<[u8; 32], String> {
    let end = offset
        .checked_add(32)
        .ok_or_else(|| "[u8; 32] offset overflow".to_string())?;
    let bytes = data
        .get(offset..end)
        .ok_or_else(|| format!("[u8; 32] read out of bounds at offset {offset}"))?;
    let mut array = [0u8; 32];
    array.copy_from_slice(bytes);
    Ok(array)
}

fn read_u64(data: &[u8], offset: usize) -> Result<u64, String> {
    let end = offset
        .checked_add(8)
        .ok_or_else(|| "u64 offset overflow".to_string())?;
    let bytes = data
        .get(offset..end)
        .ok_or_else(|| format!("u64 read out of bounds at offset {offset}"))?;
    let mut array = [0u8; 8];
    array.copy_from_slice(bytes);
    Ok(u64::from_le_bytes(array))
}

fn redact_rpc_url(url: &str) -> String {
    let mut redacted = url.to_string();

    if let Some(authority_start) = redacted.find("://").map(|index| index + 3) {
        let authority_end = redacted[authority_start..]
            .find(['/', '?', '#'])
            .map(|index| authority_start + index)
            .unwrap_or(redacted.len());
        if let Some(at) = redacted[authority_start..authority_end].rfind('@') {
            redacted.replace_range(authority_start..authority_start + at, "<redacted>");
        }
    }

    if let Some(authority_start) = redacted.find("://").map(|index| index + 3) {
        let authority_end = redacted[authority_start..]
            .find(['/', '?', '#'])
            .map(|index| authority_start + index)
            .unwrap_or(redacted.len());
        if redacted.as_bytes().get(authority_end) == Some(&b'/') {
            let path_end = redacted[authority_end..]
                .find(['?', '#'])
                .map(|index| authority_end + index)
                .unwrap_or(redacted.len());
            if path_end > authority_end + 1 {
                redacted.replace_range(authority_end + 1..path_end, "<redacted>");
            }
        }
    }

    if let Some(query_start) = redacted.find('?') {
        if let Some(fragment_start) = redacted[query_start..].find('#') {
            redacted.replace_range(query_start + 1..query_start + fragment_start, "<redacted>");
        } else {
            redacted.truncate(query_start + 1);
            redacted.push_str("<redacted>");
        }
    }
    if let Some(fragment_start) = redacted.find('#') {
        redacted.truncate(fragment_start + 1);
        redacted.push_str("<redacted>");
    }

    redacted
}

fn rpc_error(label: &str, error: impl std::fmt::Display, rpc_url: &str) -> String {
    let detail = format!("{error}").replace(rpc_url, "<redacted-rpc-url>");
    if detail.contains("://") {
        format!("{label}: RPC request failed (URL redacted)")
    } else {
        format!("{label}: {detail}")
    }
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(DIGITS[(byte >> 4) as usize] as char);
        output.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    output
}

fn json_string(value: &str) -> String {
    let mut output = String::with_capacity(value.len() + 2);
    output.push('"');
    for character in value.chars() {
        match character {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            character if character.is_control() => {
                use std::fmt::Write;
                write!(output, "\\u{:04x}", character as u32).expect("write to String cannot fail");
            }
            character => output.push(character),
        }
    }
    output.push('"');
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rpc_output_never_contains_credentials_or_query_values() {
        assert_eq!(
            redact_rpc_url("https://user:pass@example.invalid/path?key=secret&mode=fast"),
            "https://<redacted>@example.invalid/<redacted>?<redacted>"
        );
        assert_eq!(
            redact_rpc_url("https://api.devnet.solana.com"),
            "https://api.devnet.solana.com"
        );
    }

    #[test]
    fn devnet_gate_is_exact_and_fail_closed() {
        assert_eq!(validate_devnet_gate(Ok("1".to_string())), Ok(()));
        assert!(validate_devnet_gate(Ok("true".to_string())).is_err());
        assert!(validate_devnet_gate(Ok("devnet".to_string())).is_err());
        assert!(validate_devnet_gate(Err(env::VarError::NotPresent)).is_err());
    }

    #[test]
    fn provenance_hashes_are_normalized_and_strict() {
        assert_eq!(
            validate_hex_evidence("hash", &"A".repeat(64), 64).unwrap(),
            "a".repeat(64)
        );
        assert!(validate_hex_evidence("hash", &"a".repeat(63), 64).is_err());
        assert!(validate_hex_evidence("hash", &"z".repeat(64), 64).is_err());
    }

    #[test]
    fn evidence_is_deterministic_and_redacts_rpc_secrets() {
        let program_a = Pubkey::new_from_array([1; 32]);
        let program_b = Pubkey::new_from_array([2; 32]);
        let authority = Pubkey::new_from_array([3; 32]);
        let vault = Pubkey::new_from_array([4; 32]);
        let verified = VerifiedVault {
            owner: program_a,
            authority,
            balance: 42,
            data_len: Vault::LEN,
            layout_id: hex(&Vault::LAYOUT_ID),
        };
        let signatures = vec![SignatureReceipt {
            label: "program_a:init".to_string(),
            signature: "signature".to_string(),
            finalized_slot: 123,
        }];
        let provenance = Provenance {
            source_commit: Some("a".repeat(40)),
            program_a_artifact_sha256: Some("b".repeat(64)),
            program_b_artifact_sha256: None,
        };
        let rpc = "https://user:password@example.invalid/private?api_key=secret";
        let first = render_evidence(
            rpc,
            DEVNET_GENESIS,
            "agave-test",
            Some(7),
            &program_a,
            &program_b,
            &authority,
            &vault,
            &verified,
            &signatures,
            &provenance,
        );
        let second = render_evidence(
            rpc,
            DEVNET_GENESIS,
            "agave-test",
            Some(7),
            &program_a,
            &program_b,
            &authority,
            &vault,
            &verified,
            &signatures,
            &provenance,
        );
        assert_eq!(first, second);
        assert!(first.contains("\"commitment\":\"finalized\""));
        assert!(first.contains("\"finalized_slot\":123"));
        assert!(first.contains("\"program_b_artifact_sha256\":null"));
        assert!(!first.contains("password"));
        assert!(!first.contains("api_key"));
        assert!(!first.contains("secret"));
    }
}
