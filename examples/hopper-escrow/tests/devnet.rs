//! Finalized devnet evidence for Hopper's escrow state lifecycle.
//!
//! This probe deliberately covers state initialization, `has_one` authority
//! enforcement, rollback, and close. It does not claim or exercise SPL token
//! custody or transfer flow.

use std::{
    str::FromStr,
    thread,
    time::{Duration, Instant},
};

use hopper_escrow::Escrow;
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
const MAKE_TAG: u8 = 0;
const CANCEL_TAG: u8 = 2;
const AMOUNT_OFFERED: u64 = 1_000;
const AMOUNT_WANTED: u64 = 2_000;
const SYSTEM_PROGRAM: Pubkey = solana_pubkey::pubkey!("11111111111111111111111111111111");

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
    fee: u64,
}

#[derive(Debug, Eq, PartialEq)]
struct EscrowValues {
    amount_offered: u64,
    amount_wanted: u64,
    bump: u8,
}

#[test]
fn escrow_finalized_state_lifecycle_evidence() {
    if !devnet_enabled() {
        eprintln!("skipping escrow devnet test (set HOPPER_DEVNET=1 to run)");
        return;
    }
    if let Err(error) = run() {
        panic!("escrow finalized devnet evidence failed: {error}");
    }
}

fn run() -> Result<(), String> {
    let receipt_path = required_receipt_path()?;
    let rpc_url = std::env::var("SOLANA_RPC_URL").unwrap_or_else(|_| DEFAULT_RPC.to_string());
    if rpc_url != DEFAULT_RPC {
        return Err("release evidence requires the public devnet RPC endpoint".to_string());
    }
    let client = RpcClient::new_with_commitment(rpc_url.clone(), CommitmentConfig::finalized());
    let maker = load_maker()?;
    let program = program_id()?;
    let (genesis_hash, node_version, feature_set) = cluster_identity(&client, &rpc_url)?;
    require_executable_program(&client, &rpc_url, &program)?;

    let escrow = Keypair::new();
    let wrong_maker = Keypair::new();
    let mint_a = Pubkey::new_unique();
    let mint_b = Pubkey::new_unique();
    // A system account must hold the rent-exempt minimum for its size after
    // every transaction, so the wrong signer is funded at exactly that floor
    // and drained by the same amount at cleanup.
    let wrong_maker_lamports = client
        .get_minimum_balance_for_rent_exemption(0)
        .map_err(|error| format!("read rent-exempt minimum: {error}"))?;
    let fund_wrong_maker = submit_finalized(
        &client,
        &rpc_url,
        &maker,
        &[],
        &[system_instruction::transfer(
            &maker.pubkey(),
            &wrong_maker.pubkey(),
            wrong_maker_lamports,
        )],
        false,
        "fund-wrong-maker",
    )?;
    require_success(&fund_wrong_maker, "fund-wrong-maker")?;
    wait_for_snapshot(&client, &rpc_url, &wrong_maker.pubkey(), |snapshot| {
        snapshot.lamports == 1 && snapshot.owner == Pubkey::default() && snapshot.data.is_empty()
    })?;
    let make = make_instruction(program, maker.pubkey(), escrow.pubkey(), mint_a, mint_b);
    let make_outcome = submit_finalized(
        &client,
        &rpc_url,
        &maker,
        &[&escrow],
        &[make],
        false,
        "make",
    )?;
    require_success(&make_outcome, "make")?;
    let initialized = wait_for_snapshot(&client, &rpc_url, &escrow.pubkey(), |snapshot| {
        decode_escrow(snapshot, &program, &maker.pubkey(), &mint_a, &mint_b)
            == Ok(EscrowValues {
                amount_offered: AMOUNT_OFFERED,
                amount_wanted: AMOUNT_WANTED,
                bump: 0,
            })
    })?;
    let initialized_values =
        decode_escrow(&initialized, &program, &maker.pubkey(), &mint_a, &mint_b)?;
    let initialized_hash = account_sha256(&initialized);

    let wrong_cancel = cancel_instruction(program, wrong_maker.pubkey(), escrow.pubkey());
    let rejected = submit_finalized(
        &client,
        &rpc_url,
        &maker,
        &[&wrong_maker],
        &[wrong_cancel],
        true,
        "wrong-maker-cancel",
    )?;
    require_failure(
        &rejected,
        "wrong-maker-cancel",
        InstructionError::InvalidAccountData,
    )?;
    let rollback = wait_for_exact_snapshot(&client, &rpc_url, &escrow.pubkey(), &initialized)?;
    let rollback_values = decode_escrow(&rollback, &program, &maker.pubkey(), &mint_a, &mint_b)?;
    let rollback_hash = account_sha256(&rollback);

    let maker_before_close = fetch_snapshot(&client, &rpc_url, &maker.pubkey())?
        .ok_or_else(|| "maker account disappeared before close".to_string())?;
    let cancel = cancel_instruction(program, maker.pubkey(), escrow.pubkey());
    let close_outcome = submit_finalized(
        &client,
        &rpc_url,
        &maker,
        &[],
        &[cancel],
        false,
        "authorized-cancel-close",
    )?;
    require_success(&close_outcome, "authorized-cancel-close")?;
    wait_for_absence(&client, &rpc_url, &escrow.pubkey())?;
    let maker_after_close = wait_for_snapshot(&client, &rpc_url, &maker.pubkey(), |snapshot| {
        snapshot.lamports
            == maker_before_close
                .lamports
                .checked_add(initialized.lamports)
                .and_then(|value| value.checked_sub(close_outcome.fee))
                .unwrap_or(u64::MAX)
    })?;
    let expected_maker_after_close = maker_before_close
        .lamports
        .checked_add(initialized.lamports)
        .and_then(|value| value.checked_sub(close_outcome.fee))
        .ok_or_else(|| "close-destination lamport arithmetic overflowed".to_string())?;
    if maker_after_close.lamports != expected_maker_after_close {
        return Err("escrow rent was not returned exactly to the maker after fee".to_string());
    }

    let cleanup_wrong_maker = submit_finalized(
        &client,
        &rpc_url,
        &maker,
        &[&wrong_maker],
        &[system_instruction::transfer(
            &wrong_maker.pubkey(),
            &maker.pubkey(),
            wrong_maker_lamports,
        )],
        false,
        "cleanup-wrong-maker",
    )?;
    require_success(&cleanup_wrong_maker, "cleanup-wrong-maker")?;
    wait_for_absence(&client, &rpc_url, &wrong_maker.pubkey())?;

    let feature_set_json = feature_set
        .map(|value| value.to_string())
        .unwrap_or_else(|| "null".to_string());
    let rejection_error = rejected
        .error
        .as_ref()
        .map(|error| json_string(&format!("{error:?}")))
        .ok_or_else(|| "wrong-maker rejection did not carry an error".to_string())?;
    let receipt = format!(
        "{{\"schema\":\"hopper.devnet-evidence.v1\",\"example\":\"hopper-escrow\",\"scope\":\"state-init-has-one-close-no-token-flow\",\"commitment\":\"finalized\",\"rpc_endpoint\":\"redacted\",\"cluster\":{{\"genesis_hash\":{},\"node_version\":{},\"feature_set\":{}}},\"program_id\":{},\"accounts\":{{\"maker\":{},\"escrow\":{},\"wrong_maker\":{},\"mint_a_identity_only\":{},\"mint_b_identity_only\":{}}},\"transactions\":[{{\"name\":\"fund-wrong-maker\",\"signature\":{},\"slot\":{},\"outcome\":\"succeeded\"}},{{\"name\":\"make\",\"signature\":{},\"slot\":{},\"outcome\":\"succeeded\"}},{{\"name\":\"wrong-maker-cancel\",\"signature\":{},\"slot\":{},\"outcome\":\"rejected\",\"expected_error\":\"InvalidAccountData\",\"error\":{}}},{{\"name\":\"authorized-cancel-close\",\"signature\":{},\"slot\":{},\"outcome\":\"succeeded\",\"fee\":{}}},{{\"name\":\"cleanup-wrong-maker\",\"signature\":{},\"slot\":{},\"outcome\":\"succeeded\"}}],\"state\":{{\"initialized\":{{\"account_sha256\":{},\"data_len\":{},\"lamports\":{},\"amount_offered\":{},\"amount_wanted\":{},\"bump\":{}}},\"wrong_maker_rollback\":{{\"pre_account_sha256\":{},\"post_account_sha256\":{},\"pre_amount_offered\":{},\"post_amount_offered\":{},\"pre_amount_wanted\":{},\"post_amount_wanted\":{},\"exact_snapshot_unchanged\":true}},\"closed\":{{\"account_exists\":false,\"maker_lamports_before\":{},\"maker_lamports_after\":{},\"rent_returned_after_fee\":true}},\"cleanup\":{{\"wrong_maker_account_exists\":false}}}}}}",
        json_string(&genesis_hash),
        json_string(&node_version),
        feature_set_json,
        json_string(&program.to_string()),
        json_string(&maker.pubkey().to_string()),
        json_string(&escrow.pubkey().to_string()),
        json_string(&wrong_maker.pubkey().to_string()),
        json_string(&mint_a.to_string()),
        json_string(&mint_b.to_string()),
        json_string(&fund_wrong_maker.signature),
        fund_wrong_maker.slot,
        json_string(&make_outcome.signature),
        make_outcome.slot,
        json_string(&rejected.signature),
        rejected.slot,
        rejection_error,
        json_string(&close_outcome.signature),
        close_outcome.slot,
        close_outcome.fee,
        json_string(&cleanup_wrong_maker.signature),
        cleanup_wrong_maker.slot,
        json_string(&initialized_hash),
        initialized.data.len(),
        initialized.lamports,
        initialized_values.amount_offered,
        initialized_values.amount_wanted,
        initialized_values.bump,
        json_string(&initialized_hash),
        json_string(&rollback_hash),
        initialized_values.amount_offered,
        rollback_values.amount_offered,
        initialized_values.amount_wanted,
        rollback_values.amount_wanted,
        maker_before_close.lamports,
        maker_after_close.lamports,
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

fn load_maker() -> Result<Keypair, String> {
    let path = std::env::var("HOPPER_KEYPAIR")
        .map_err(|_| "HOPPER_KEYPAIR is required when HOPPER_DEVNET=1".to_string())?;
    read_keypair_file(path).map_err(|_| "failed to read HOPPER_KEYPAIR (path redacted)".to_string())
}

fn program_id() -> Result<Pubkey, String> {
    let value = std::env::var("HOPPER_ESCROW_PROGRAM_ID")
        .map_err(|_| "HOPPER_ESCROW_PROGRAM_ID is required when HOPPER_DEVNET=1".to_string())?;
    Pubkey::from_str(&value).map_err(|_| "invalid HOPPER_ESCROW_PROGRAM_ID".to_string())
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

fn make_instruction(
    program: Pubkey,
    maker: Pubkey,
    escrow: Pubkey,
    mint_a: Pubkey,
    mint_b: Pubkey,
) -> Instruction {
    let mut data = Vec::with_capacity(81);
    data.push(MAKE_TAG);
    data.extend_from_slice(mint_a.as_ref());
    data.extend_from_slice(mint_b.as_ref());
    data.extend_from_slice(&AMOUNT_OFFERED.to_le_bytes());
    data.extend_from_slice(&AMOUNT_WANTED.to_le_bytes());
    Instruction {
        program_id: program,
        accounts: vec![
            AccountMeta::new(maker, true),
            AccountMeta::new(escrow, true),
            AccountMeta::new_readonly(SYSTEM_PROGRAM, false),
        ],
        data,
    }
}

fn cancel_instruction(program: Pubkey, maker: Pubkey, escrow: Pubkey) -> Instruction {
    Instruction {
        program_id: program,
        accounts: vec![
            AccountMeta::new(maker, true),
            AccountMeta::new(escrow, false),
        ],
        data: vec![CANCEL_TAG],
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
    let fee = client
        .get_fee_for_message(&transaction.message)
        .map_err(|error| rpc_error(&format!("{label}: get transaction fee"), error, rpc_url))?;
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
    wait_for_finalized(client, rpc_url, signature.to_string(), fee, label)
}

fn wait_for_finalized(
    client: &RpcClient,
    rpc_url: &str,
    signature: String,
    fee: u64,
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
                    fee,
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
                "closed account {address} remained visible at finalized before timeout"
            ));
        }
        thread::sleep(POLL_INTERVAL);
    }
}

fn decode_escrow(
    snapshot: &AccountSnapshot,
    program: &Pubkey,
    maker: &Pubkey,
    mint_a: &Pubkey,
    mint_b: &Pubkey,
) -> Result<EscrowValues, String> {
    if snapshot.owner != *program {
        return Err(format!(
            "escrow owner mismatch: expected {program}, got {}",
            snapshot.owner
        ));
    }
    if snapshot.executable {
        return Err("escrow state account must not be executable".to_string());
    }
    if snapshot.data.len() != Escrow::INIT_SPACE {
        return Err(format!(
            "escrow length mismatch: expected {}, got {}",
            Escrow::INIT_SPACE,
            snapshot.data.len()
        ));
    }
    if snapshot.data[0] != Escrow::DISC || snapshot.data[1] != Escrow::VERSION {
        return Err("escrow header discriminator/version mismatch".to_string());
    }
    if snapshot.data[4..12] != Escrow::LAYOUT_ID {
        return Err("escrow layout id mismatch".to_string());
    }
    require_pubkey_field(
        &snapshot.data,
        Escrow::MAKER_ABS_OFFSET as usize,
        maker,
        "maker",
    )?;
    require_zero_pubkey_field(
        &snapshot.data,
        Escrow::MAKER_TA_ABS_OFFSET as usize,
        "maker_ta",
    )?;
    require_pubkey_field(
        &snapshot.data,
        Escrow::MINT_A_ABS_OFFSET as usize,
        mint_a,
        "mint_a",
    )?;
    require_pubkey_field(
        &snapshot.data,
        Escrow::MINT_B_ABS_OFFSET as usize,
        mint_b,
        "mint_b",
    )?;
    Ok(EscrowValues {
        amount_offered: read_u64(
            &snapshot.data,
            Escrow::AMOUNT_OFFERED_ABS_OFFSET as usize,
            "amount_offered",
        )?,
        amount_wanted: read_u64(
            &snapshot.data,
            Escrow::AMOUNT_WANTED_ABS_OFFSET as usize,
            "amount_wanted",
        )?,
        bump: *snapshot
            .data
            .get(Escrow::BUMP_ABS_OFFSET as usize)
            .ok_or_else(|| "bump field out of bounds".to_string())?,
    })
}

fn require_pubkey_field(
    data: &[u8],
    offset: usize,
    expected: &Pubkey,
    label: &str,
) -> Result<(), String> {
    let bytes = data
        .get(offset..offset + 32)
        .ok_or_else(|| format!("{label} field out of bounds"))?;
    if bytes != expected.as_ref() {
        return Err(format!("{label} mismatch"));
    }
    Ok(())
}

fn require_zero_pubkey_field(data: &[u8], offset: usize, label: &str) -> Result<(), String> {
    let bytes = data
        .get(offset..offset + 32)
        .ok_or_else(|| format!("{label} field out of bounds"))?;
    if bytes != [0u8; 32] {
        return Err(format!("{label} must be zero in the state-only example"));
    }
    Ok(())
}

fn read_u64(data: &[u8], offset: usize, label: &str) -> Result<u64, String> {
    let bytes = data
        .get(offset..offset + 8)
        .ok_or_else(|| format!("{label} field out of bounds"))?;
    let mut value = [0u8; 8];
    value.copy_from_slice(bytes);
    Ok(u64::from_le_bytes(value))
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
