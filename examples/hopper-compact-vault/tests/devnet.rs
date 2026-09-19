//! Finalized devnet evidence for the compact vault example.
//!
//! The test stays offline while `HOPPER_DEVNET` is absent. When the variable is
//! present it must be exactly `1`; any other value fails instead of silently
//! skipping. An enabled run requires a deployed program and a funded fee
//! payer, verifies the cluster is Solana devnet, and emits one deterministic
//! JSON record after every transaction and state read reaches `finalized`.

use std::{
    str::FromStr,
    thread,
    time::{Duration, Instant},
};

use hopper_compact_vault::Vault;
use solana_client::{rpc_client::RpcClient, rpc_config::RpcSendTransactionConfig};
use solana_commitment_config::CommitmentConfig;
use solana_instruction::{AccountMeta, Instruction};
use solana_keypair::{read_keypair_file, Keypair};
use solana_pubkey::Pubkey;
use solana_signer::Signer;
use solana_system_interface::instruction as system_instruction;
use solana_transaction::{InstructionError, Transaction, TransactionError};

const DEFAULT_RPC: &str = "https://api.devnet.solana.com";
const DEVNET_GENESIS_HASH: &str = "EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG";
const FINALIZED_TIMEOUT: Duration = Duration::from_secs(180);
const POLL_INTERVAL: Duration = Duration::from_millis(500);
const VAULT_DISC: u8 = 1;
const VAULT_LEN: u64 = 41;
const IX_INIT: u8 = 0;
const IX_DEPOSIT: u8 = 1;
const DEPOSIT_AMOUNT: u64 = 123_456;
const VAULT_LAYOUT_ID: [u8; 8] = [67, 113, 65, 144, 124, 9, 52, 79];

#[derive(Clone, Debug, Eq, PartialEq)]
struct AccountSnapshot {
    lamports: u64,
    owner: Pubkey,
    executable: bool,
    rent_epoch: u64,
    data: Vec<u8>,
}

#[derive(Debug)]
struct FinalizedOutcome {
    signature: String,
    slot: u64,
    error: Option<TransactionError>,
}

#[test]
fn compact_vault_finalized_devnet_evidence() {
    if !devnet_enabled() {
        eprintln!("skipping compact vault devnet test (set HOPPER_DEVNET=1 to run)");
        return;
    }
    if let Err(error) = run() {
        panic!("compact vault finalized devnet evidence failed: {error}");
    }
}

fn run() -> Result<(), String> {
    let receipt_path = required_receipt_path()?;
    let rpc_url = std::env::var("SOLANA_RPC_URL").unwrap_or_else(|_| DEFAULT_RPC.to_string());
    if rpc_url != DEFAULT_RPC {
        return Err("release evidence requires the public devnet RPC endpoint".to_string());
    }
    let client = RpcClient::new_with_commitment(rpc_url.clone(), CommitmentConfig::finalized());
    let payer = load_payer()?;
    let program = program_id()?;
    let (genesis_hash, node_version, feature_set) = cluster_identity(&client, &rpc_url)?;
    require_executable_program(&client, &rpc_url, &program)?;

    let vault = Keypair::new();
    let wrong_authority = Keypair::new();
    // A system account must hold the rent-exempt minimum for its size after
    // every transaction, so the wrong signer is funded at exactly that floor
    // and drained by the same amount at cleanup.
    let wrong_authority_lamports = client
        .get_minimum_balance_for_rent_exemption(0)
        .map_err(|error| format!("read rent-exempt minimum: {error}"))?;
    let fund_wrong_authority = submit_finalized(
        &client,
        &rpc_url,
        &payer,
        &[],
        &[system_instruction::transfer(
            &payer.pubkey(),
            &wrong_authority.pubkey(),
            wrong_authority_lamports,
        )],
        false,
        "fund-wrong-authority",
    )?;
    require_success(&fund_wrong_authority, "fund-wrong-authority")?;
    wait_for_snapshot(&client, &rpc_url, &wrong_authority.pubkey(), |snapshot| {
        snapshot.lamports == wrong_authority_lamports
            && snapshot.owner == Pubkey::default()
            && snapshot.data.is_empty()
    })?;
    let lamports = client
        .get_minimum_balance_for_rent_exemption(VAULT_LEN as usize)
        .map_err(|error| rpc_error("rent exemption", error, &rpc_url))?;

    let create = system_instruction::create_account(
        &payer.pubkey(),
        &vault.pubkey(),
        lamports,
        VAULT_LEN,
        &program,
    );
    let init = Instruction {
        program_id: program,
        accounts: vec![
            AccountMeta::new(vault.pubkey(), false),
            AccountMeta::new_readonly(payer.pubkey(), true),
        ],
        data: vec![IX_INIT],
    };
    let init_outcome = submit_finalized(
        &client,
        &rpc_url,
        &payer,
        &[&vault],
        &[create, init],
        false,
        "create-and-initialize",
    )?;
    require_success(&init_outcome, "create-and-initialize")?;

    let initialized = wait_for_snapshot(&client, &rpc_url, &vault.pubkey(), |snapshot| {
        decode_vault(snapshot, &program, &payer.pubkey()) == Ok(0)
    })?;
    let initialized_balance = decode_vault(&initialized, &program, &payer.pubkey())?;
    let initialized_hash = account_sha256(&initialized);

    let wrong_deposit = deposit_instruction(
        program,
        vault.pubkey(),
        wrong_authority.pubkey(),
        DEPOSIT_AMOUNT,
    );
    let rejected = submit_finalized(
        &client,
        &rpc_url,
        &payer,
        &[&wrong_authority],
        &[wrong_deposit],
        true,
        "wrong-authority-deposit",
    )?;
    require_failure(
        &rejected,
        "wrong-authority-deposit",
        InstructionError::IncorrectAuthority,
    )?;
    let rollback = wait_for_exact_snapshot(&client, &rpc_url, &vault.pubkey(), &initialized)?;
    let rollback_balance = decode_vault(&rollback, &program, &payer.pubkey())?;
    let rollback_hash = account_sha256(&rollback);

    let deposit = deposit_instruction(program, vault.pubkey(), payer.pubkey(), DEPOSIT_AMOUNT);
    let deposit_outcome = submit_finalized(
        &client,
        &rpc_url,
        &payer,
        &[],
        &[deposit],
        false,
        "authorized-deposit",
    )?;
    require_success(&deposit_outcome, "authorized-deposit")?;
    let deposited = wait_for_snapshot(&client, &rpc_url, &vault.pubkey(), |snapshot| {
        decode_vault(snapshot, &program, &payer.pubkey()) == Ok(DEPOSIT_AMOUNT)
    })?;
    let deposited_balance = decode_vault(&deposited, &program, &payer.pubkey())?;
    let deposited_hash = account_sha256(&deposited);

    let cleanup_wrong_authority = submit_finalized(
        &client,
        &rpc_url,
        &payer,
        &[&wrong_authority],
        &[system_instruction::transfer(
            &wrong_authority.pubkey(),
            &payer.pubkey(),
            wrong_authority_lamports,
        )],
        false,
        "cleanup-wrong-authority",
    )?;
    require_success(&cleanup_wrong_authority, "cleanup-wrong-authority")?;
    wait_for_absence(&client, &rpc_url, &wrong_authority.pubkey())?;

    let feature_set_json = feature_set
        .map(|value| value.to_string())
        .unwrap_or_else(|| "null".to_string());
    let rejection_error = rejected
        .error
        .as_ref()
        .map(|error| json_string(&format!("{error:?}")))
        .ok_or_else(|| "wrong-authority rejection did not carry an error".to_string())?;
    let receipt = format!(
        "{{\"schema\":\"hopper.devnet-evidence.v1\",\"example\":\"hopper-compact-vault\",\"commitment\":\"finalized\",\"rpc_endpoint\":\"redacted\",\"cluster\":{{\"genesis_hash\":{},\"node_version\":{},\"feature_set\":{}}},\"program_id\":{},\"accounts\":{{\"authority\":{},\"vault\":{},\"wrong_authority\":{}}},\"transactions\":[{{\"name\":\"fund-wrong-authority\",\"signature\":{},\"slot\":{},\"outcome\":\"succeeded\"}},{{\"name\":\"create-and-initialize\",\"signature\":{},\"slot\":{},\"outcome\":\"succeeded\"}},{{\"name\":\"wrong-authority-deposit\",\"signature\":{},\"slot\":{},\"outcome\":\"rejected\",\"expected_error\":\"IncorrectAuthority\",\"error\":{}}},{{\"name\":\"authorized-deposit\",\"signature\":{},\"slot\":{},\"outcome\":\"succeeded\"}},{{\"name\":\"cleanup-wrong-authority\",\"signature\":{},\"slot\":{},\"outcome\":\"succeeded\"}}],\"state\":{{\"layout_bytes\":{},\"layout_id\":\"437141907c09344f\",\"initialized\":{{\"account_sha256\":{},\"balance\":{}}},\"wrong_authority_rollback\":{{\"pre_account_sha256\":{},\"post_account_sha256\":{},\"pre_balance\":{},\"post_balance\":{},\"exact_snapshot_unchanged\":true}},\"deposited\":{{\"account_sha256\":{},\"balance\":{}}},\"cleanup\":{{\"wrong_authority_account_exists\":false}}}}}}",
        json_string(&genesis_hash),
        json_string(&node_version),
        feature_set_json,
        json_string(&program.to_string()),
        json_string(&payer.pubkey().to_string()),
        json_string(&vault.pubkey().to_string()),
        json_string(&wrong_authority.pubkey().to_string()),
        json_string(&fund_wrong_authority.signature),
        fund_wrong_authority.slot,
        json_string(&init_outcome.signature),
        init_outcome.slot,
        json_string(&rejected.signature),
        rejected.slot,
        rejection_error,
        json_string(&deposit_outcome.signature),
        deposit_outcome.slot,
        json_string(&cleanup_wrong_authority.signature),
        cleanup_wrong_authority.slot,
        VAULT_LEN,
        json_string(&initialized_hash),
        initialized_balance,
        json_string(&initialized_hash),
        json_string(&rollback_hash),
        initialized_balance,
        rollback_balance,
        json_string(&deposited_hash),
        deposited_balance,
    );
    std::fs::write(&receipt_path, format!("{receipt}\n"))
        .map_err(|_| "failed to write HOPPER_DEVNET_RECEIPT (path redacted)".to_string())?;
    println!("wrote hopper.devnet-evidence.v1 receipt");
    Ok(())
}

fn devnet_enabled() -> bool {
    match std::env::var("HOPPER_DEVNET") {
        Err(std::env::VarError::NotPresent) => {
            if std::env::var("HOPPER_REQUIRE_DEVNET").as_deref() == Ok("1") {
                panic!("HOPPER_REQUIRE_DEVNET=1 requires HOPPER_DEVNET=1; refusing to skip")
            }
            false
        }
        Err(std::env::VarError::NotUnicode(_)) => {
            panic!("HOPPER_DEVNET must be valid Unicode and exactly 1")
        }
        Ok(value) if value == "1" => true,
        Ok(_) => panic!("HOPPER_DEVNET must be exactly 1 when set"),
    }
}

fn required_receipt_path() -> Result<std::path::PathBuf, String> {
    std::env::var_os("HOPPER_DEVNET_RECEIPT")
        .filter(|path| !path.is_empty())
        .map(std::path::PathBuf::from)
        .ok_or_else(|| "HOPPER_DEVNET_RECEIPT is required when HOPPER_DEVNET=1".to_string())
}

fn load_payer() -> Result<Keypair, String> {
    let path = std::env::var("HOPPER_KEYPAIR")
        .map_err(|_| "HOPPER_KEYPAIR is required when HOPPER_DEVNET=1".to_string())?;
    read_keypair_file(path).map_err(|_| "failed to read HOPPER_KEYPAIR (path redacted)".to_string())
}

fn program_id() -> Result<Pubkey, String> {
    let value = std::env::var("HOPPER_COMPACT_VAULT_PROGRAM_ID").map_err(|_| {
        "HOPPER_COMPACT_VAULT_PROGRAM_ID is required when HOPPER_DEVNET=1".to_string()
    })?;
    Pubkey::from_str(&value).map_err(|_| "invalid HOPPER_COMPACT_VAULT_PROGRAM_ID".to_string())
}

fn cluster_identity(
    client: &RpcClient,
    rpc_url: &str,
) -> Result<(String, String, Option<u32>), String> {
    let genesis_hash = client
        .get_genesis_hash()
        .map_err(|error| rpc_error("get genesis hash", error, rpc_url))?
        .to_string();
    if genesis_hash != DEVNET_GENESIS_HASH {
        return Err(format!(
            "refusing non-devnet cluster: expected genesis {DEVNET_GENESIS_HASH}, got {genesis_hash}"
        ));
    }
    let version = client
        .get_version()
        .map_err(|error| rpc_error("get node version", error, rpc_url))?;
    Ok((genesis_hash, version.solana_core, version.feature_set))
}

fn require_executable_program(
    client: &RpcClient,
    rpc_url: &str,
    program: &Pubkey,
) -> Result<(), String> {
    let account = client
        .get_account_with_commitment(program, CommitmentConfig::finalized())
        .map_err(|error| rpc_error("get program account", error, rpc_url))?
        .value
        .ok_or_else(|| format!("program account {program} does not exist at finalized"))?;
    if !account.executable {
        return Err(format!("program account {program} is not executable"));
    }
    Ok(())
}

fn deposit_instruction(
    program: Pubkey,
    vault: Pubkey,
    authority: Pubkey,
    amount: u64,
) -> Instruction {
    let mut data = Vec::with_capacity(9);
    data.push(IX_DEPOSIT);
    data.extend_from_slice(&amount.to_le_bytes());
    Instruction {
        program_id: program,
        accounts: vec![
            AccountMeta::new(vault, false),
            AccountMeta::new_readonly(authority, true),
        ],
        data,
    }
}

fn submit_finalized(
    client: &RpcClient,
    rpc_url: &str,
    payer: &Keypair,
    extra_signers: &[&Keypair],
    instructions: &[Instruction],
    skip_preflight: bool,
    label: &str,
) -> Result<FinalizedOutcome, String> {
    let blockhash = client
        .get_latest_blockhash()
        .map_err(|error| rpc_error(&format!("{label}: get blockhash"), error, rpc_url))?;
    let mut signers: Vec<&dyn Signer> = vec![payer];
    signers.extend(extra_signers.iter().map(|signer| *signer as &dyn Signer));
    let transaction = Transaction::new_signed_with_payer(
        instructions,
        Some(&payer.pubkey()),
        &signers,
        blockhash,
    );
    let signature = client
        .send_transaction_with_config(
            &transaction,
            RpcSendTransactionConfig {
                skip_preflight,
                preflight_commitment: Some(CommitmentConfig::finalized().commitment),
                max_retries: Some(5),
                ..RpcSendTransactionConfig::default()
            },
        )
        .map_err(|error| rpc_error(&format!("{label}: send transaction"), error, rpc_url))?;
    wait_for_finalized(client, rpc_url, signature.to_string(), label)
}

fn wait_for_finalized(
    client: &RpcClient,
    rpc_url: &str,
    signature: String,
    label: &str,
) -> Result<FinalizedOutcome, String> {
    let parsed = signature
        .parse()
        .map_err(|_| format!("{label}: invalid signature returned by RPC"))?;
    let deadline = Instant::now() + FINALIZED_TIMEOUT;
    loop {
        let response = client.get_signature_statuses(&[parsed]).map_err(|error| {
            rpc_error(&format!("{label}: get signature status"), error, rpc_url)
        })?;
        if let Some(status) = response.value.into_iter().next().flatten() {
            if status.satisfies_commitment(CommitmentConfig::finalized()) {
                return Ok(FinalizedOutcome {
                    signature,
                    slot: status.slot,
                    error: status.err,
                });
            }
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "{label}: transaction {signature} did not reach finalized before timeout"
            ));
        }
        thread::sleep(POLL_INTERVAL);
    }
}

fn require_success(outcome: &FinalizedOutcome, label: &str) -> Result<(), String> {
    match &outcome.error {
        None => Ok(()),
        Some(error) => Err(format!(
            "{label}: finalized transaction {} failed: {error}",
            outcome.signature
        )),
    }
}

fn require_failure(
    outcome: &FinalizedOutcome,
    label: &str,
    expected_error: InstructionError,
) -> Result<(), String> {
    match outcome.error.as_ref() {
        Some(TransactionError::InstructionError(0, actual)) if *actual == expected_error => Ok(()),
        Some(actual) => Err(format!(
            "{label}: expected InstructionError(0, {expected_error:?}), got {actual:?}"
        )),
        None => Err(format!(
            "{label}: finalized transaction {} unexpectedly succeeded",
            outcome.signature
        )),
    }
}

fn fetch_snapshot(
    client: &RpcClient,
    rpc_url: &str,
    address: &Pubkey,
) -> Result<Option<AccountSnapshot>, String> {
    let account = client
        .get_account_with_commitment(address, CommitmentConfig::finalized())
        .map_err(|error| rpc_error("get finalized account", error, rpc_url))?
        .value;
    Ok(account.map(|account| AccountSnapshot {
        lamports: account.lamports,
        owner: account.owner,
        executable: account.executable,
        rent_epoch: account.rent_epoch,
        data: account.data,
    }))
}

fn wait_for_snapshot(
    client: &RpcClient,
    rpc_url: &str,
    address: &Pubkey,
    predicate: impl Fn(&AccountSnapshot) -> bool,
) -> Result<AccountSnapshot, String> {
    let deadline = Instant::now() + FINALIZED_TIMEOUT;
    loop {
        if let Some(snapshot) = fetch_snapshot(client, rpc_url, address)? {
            if predicate(&snapshot) {
                return Ok(snapshot);
            }
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "account {address} did not reach expected finalized state before timeout"
            ));
        }
        thread::sleep(POLL_INTERVAL);
    }
}

fn wait_for_exact_snapshot(
    client: &RpcClient,
    rpc_url: &str,
    address: &Pubkey,
    expected: &AccountSnapshot,
) -> Result<AccountSnapshot, String> {
    wait_for_snapshot(client, rpc_url, address, |snapshot| snapshot == expected)
}

fn wait_for_absence(client: &RpcClient, rpc_url: &str, address: &Pubkey) -> Result<(), String> {
    let deadline = Instant::now() + FINALIZED_TIMEOUT;
    loop {
        if fetch_snapshot(client, rpc_url, address)?.is_none() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "account {address} remained visible at finalized before timeout"
            ));
        }
        thread::sleep(POLL_INTERVAL);
    }
}

fn decode_vault(
    snapshot: &AccountSnapshot,
    program: &Pubkey,
    authority: &Pubkey,
) -> Result<u64, String> {
    if snapshot.owner != *program {
        return Err(format!(
            "vault owner mismatch: expected {program}, got {}",
            snapshot.owner
        ));
    }
    if snapshot.executable {
        return Err("vault account must not be executable".to_string());
    }
    if snapshot.data.len() != Vault::COMPACT_LEN || snapshot.data.len() != VAULT_LEN as usize {
        return Err(format!(
            "vault length mismatch: expected {VAULT_LEN}, got {}",
            snapshot.data.len()
        ));
    }
    if snapshot.data[0] != VAULT_DISC {
        return Err(format!(
            "compact discriminator mismatch: expected {VAULT_DISC}, got {}",
            snapshot.data[0]
        ));
    }
    if &snapshot.data[1..33] != authority.as_ref() {
        return Err("compact authority mismatch".to_string());
    }
    if &snapshot.data[4..12] == VAULT_LAYOUT_ID.as_slice() {
        return Err("compact account unexpectedly stores the layout id as a header".to_string());
    }
    let mut balance = [0u8; 8];
    balance.copy_from_slice(&snapshot.data[33..41]);
    Ok(u64::from_le_bytes(balance))
}

fn account_sha256(snapshot: &AccountSnapshot) -> String {
    let mut canonical = Vec::with_capacity(8 + 32 + 1 + 8 + 8 + snapshot.data.len());
    canonical.extend_from_slice(&snapshot.lamports.to_le_bytes());
    canonical.extend_from_slice(snapshot.owner.as_ref());
    canonical.push(u8::from(snapshot.executable));
    canonical.extend_from_slice(&snapshot.rent_epoch.to_le_bytes());
    canonical.extend_from_slice(&(snapshot.data.len() as u64).to_le_bytes());
    canonical.extend_from_slice(&snapshot.data);
    hex(&hopper::hopper_runtime::sha256::sha256(&canonical))
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

fn rpc_error(label: &str, error: impl std::fmt::Display, rpc_url: &str) -> String {
    let detail = format!("{error}").replace(rpc_url, "<redacted-rpc-url>");
    if detail.contains("://") {
        format!("{label}: RPC request failed (URL redacted)")
    } else {
        format!("{label}: {detail}")
    }
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

#[test]
fn evidence_helpers_redact_urls_and_escape_json() {
    let url = "https://user:secret@example.invalid/path?key=secret";
    let output = rpc_error("probe", format!("request to {url} failed"), url);
    assert!(!output.contains("secret"));
    assert!(!output.contains("://"));
    assert_eq!(json_string("a\"b\\c\n"), "\"a\\\"b\\\\c\\n\"");
}

#[test]
fn devnet_genesis_pin_is_complete() {
    assert_eq!(
        DEVNET_GENESIS_HASH,
        "EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG"
    );
}
