//! Opt-in finalized devnet proof for V1 initialization, append migration, and
//! the tag-2 System Program deposit path.

use std::str::FromStr;

use serde::Serialize;
use solana_client::rpc_client::RpcClient;
use solana_commitment_config::CommitmentConfig;
use solana_instruction::{AccountMeta, Instruction};
use solana_keypair::{read_keypair_file, Keypair};
use solana_pubkey::Pubkey;
use solana_signer::Signer;
use solana_transaction::Transaction;

const DEVNET_GENESIS: &str = "EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG";
const PUBLIC_DEVNET_RPC: &str = "https://api.devnet.solana.com";
const INIT_V1_TAG: u8 = 0;
const MIGRATE_TAG: u8 = 1;
const DEPOSIT_V2_TAG: u8 = 2;
const V1_LEN: usize = 16 + 32 + 8;
const V2_LEN: usize = 16 + 32 + 8 + 1 + 8;
const BALANCE_OFFSET: usize = 16 + 32;
const BUMP_OFFSET: usize = V1_LEN;
const LAST_DEPOSIT_OFFSET: usize = BUMP_OFFSET + 1;
const NEW_BUMP: u8 = 254;
const DEPOSIT_AMOUNT: u64 = 1_000_000;
const SYSTEM_PROGRAM: Pubkey = solana_pubkey::pubkey!("11111111111111111111111111111111");

#[derive(Serialize)]
struct SignatureReceipt {
    label: &'static str,
    signature: String,
    finalized_slot: u64,
    outcome: &'static str,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
struct ClusterReceipt {
    genesis_hash: &'static str,
    node_version: String,
    feature_set: Option<u32>,
}

#[derive(Serialize)]
struct DevnetReceipt {
    schema: &'static str,
    example: &'static str,
    commitment: &'static str,
    rpc_endpoint: &'static str,
    cluster: ClusterReceipt,
    program_id: String,
    authority: String,
    vault: String,
    v1_size: usize,
    v2_size: usize,
    deposit_amount: u64,
    recorded_balance: u64,
    last_deposit: u64,
    vault_lamports_before_deposit: u64,
    vault_lamports_after_deposit: u64,
    signatures: Vec<SignatureReceipt>,
}

fn live_enabled() -> bool {
    match std::env::var("HOPPER_DEVNET") {
        Ok(value) if value == "1" => true,
        Ok(_) => panic!("HOPPER_DEVNET must be exactly 1 when set; refusing to skip"),
        Err(std::env::VarError::NotPresent) => {
            if std::env::var("HOPPER_REQUIRE_DEVNET").as_deref() == Ok("1") {
                panic!("HOPPER_REQUIRE_DEVNET=1 requires HOPPER_DEVNET=1; refusing to skip");
            }
            false
        }
        Err(error) => panic!("invalid HOPPER_DEVNET environment value: {error}"),
    }
}

fn rpc_url() -> String {
    std::env::var("SOLANA_RPC_URL").unwrap_or_else(|_| PUBLIC_DEVNET_RPC.to_string())
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
        if let Some(fragment) = redacted[query_start..].find('#') {
            redacted.replace_range(query_start + 1..query_start + fragment, "<redacted>");
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

fn layout_id(data: &[u8]) -> [u8; 8] {
    data[4..12]
        .try_into()
        .expect("header carries an 8-byte layout id")
}

fn read_u64(data: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(data[offset..offset + 8].try_into().unwrap())
}

fn send_finalized(
    client: &RpcClient,
    transaction: &Transaction,
    label: &'static str,
) -> SignatureReceipt {
    let signature = client
        .send_and_confirm_transaction_with_spinner_and_commitment(
            transaction,
            CommitmentConfig::finalized(),
        )
        .unwrap_or_else(|error| panic!("{label} failed: {error}"));
    let status = client
        .get_signature_status_with_commitment(&signature, CommitmentConfig::finalized())
        .unwrap_or_else(|error| panic!("{label} finalized status failed: {error}"));
    assert_eq!(
        status,
        Some(Ok(())),
        "{label} was not finalized successfully"
    );
    let status = client
        .get_signature_statuses_with_history(&[signature])
        .unwrap_or_else(|error| panic!("{label} slot lookup failed: {error}"))
        .value
        .into_iter()
        .next()
        .flatten()
        .unwrap_or_else(|| panic!("{label} finalized signature disappeared"));
    SignatureReceipt {
        label,
        signature: signature.to_string(),
        finalized_slot: status.slot,
        outcome: "succeeded",
    }
}

#[test]
fn migration_v1_to_v2_and_deposit_v2_roundtrip() {
    if !live_enabled() {
        eprintln!("SKIPPED: set HOPPER_DEVNET=1 for the live migration proof");
        return;
    }
    let receipt_path = required_receipt_path();

    let rpc = rpc_url();
    assert_eq!(
        rpc, PUBLIC_DEVNET_RPC,
        "release evidence requires the public devnet RPC endpoint"
    );
    let client = RpcClient::new_with_commitment(rpc.clone(), CommitmentConfig::finalized());
    assert_eq!(
        client
            .get_genesis_hash()
            .expect("read RPC genesis")
            .to_string(),
        DEVNET_GENESIS,
        "refusing to run this spending harness on a non-devnet cluster"
    );
    let node = client
        .get_version()
        .expect("read public devnet node version");
    let program = Pubkey::from_str(
        &std::env::var("HOPPER_MIGRATION_PROGRAM_ID")
            .expect("set HOPPER_MIGRATION_PROGRAM_ID to a fresh devnet deployment"),
    )
    .expect("invalid HOPPER_MIGRATION_PROGRAM_ID");
    let program_account = client
        .get_account_with_commitment(&program, CommitmentConfig::finalized())
        .expect("fetch deployed program")
        .value
        .expect("deployed program account is absent at finalized");
    assert!(
        program_account.executable,
        "program account is not executable"
    );
    let payer = read_keypair_file(
        std::env::var("HOPPER_KEYPAIR")
            .expect("set HOPPER_KEYPAIR to an explicit devnet-only fee-payer keypair"),
    )
    .expect("read HOPPER_KEYPAIR");
    assert!(
        client.get_balance(&payer.pubkey()).expect("payer balance") > DEPOSIT_AMOUNT,
        "devnet fee payer cannot cover the deposit"
    );
    let vault = Keypair::new();

    let init_ix = Instruction {
        program_id: program,
        accounts: vec![
            AccountMeta::new(payer.pubkey(), true),
            AccountMeta::new(vault.pubkey(), true),
            AccountMeta::new_readonly(SYSTEM_PROGRAM, false),
        ],
        data: vec![INIT_V1_TAG],
    };
    let blockhash = client.get_latest_blockhash().expect("init blockhash");
    let init_tx = Transaction::new_signed_with_payer(
        &[init_ix],
        Some(&payer.pubkey()),
        &[&payer, &vault],
        blockhash,
    );
    let mut signatures = vec![send_finalized(&client, &init_tx, "initV1")];
    let v1_account = client
        .get_account_with_commitment(&vault.pubkey(), CommitmentConfig::finalized())
        .expect("fetch V1 vault")
        .value
        .expect("V1 vault missing at finalized");
    assert_eq!(v1_account.owner, program);
    assert_eq!(v1_account.data.len(), V1_LEN);
    let v1_layout = layout_id(&v1_account.data);

    let migrate_ix = Instruction {
        program_id: program,
        accounts: vec![
            AccountMeta::new(payer.pubkey(), true),
            AccountMeta::new(vault.pubkey(), false),
            AccountMeta::new_readonly(SYSTEM_PROGRAM, false),
        ],
        data: vec![MIGRATE_TAG, NEW_BUMP],
    };
    let blockhash = client.get_latest_blockhash().expect("migrate blockhash");
    let migrate_tx = Transaction::new_signed_with_payer(
        &[migrate_ix],
        Some(&payer.pubkey()),
        &[&payer],
        blockhash,
    );
    signatures.push(send_finalized(&client, &migrate_tx, "migrateV1ToV2"));
    let migrated = client
        .get_account_with_commitment(&vault.pubkey(), CommitmentConfig::finalized())
        .expect("fetch migrated vault")
        .value
        .expect("migrated vault missing at finalized");
    assert_eq!(migrated.data.len(), V2_LEN);
    assert_ne!(v1_layout, layout_id(&migrated.data));
    assert_eq!(migrated.data[BUMP_OFFSET], NEW_BUMP);
    let vault_lamports_before_deposit = migrated.lamports;

    let mut deposit_data = Vec::with_capacity(9);
    deposit_data.push(DEPOSIT_V2_TAG);
    deposit_data.extend_from_slice(&DEPOSIT_AMOUNT.to_le_bytes());
    let deposit_ix = Instruction {
        program_id: program,
        accounts: vec![
            AccountMeta::new(payer.pubkey(), true),
            AccountMeta::new(vault.pubkey(), false),
            AccountMeta::new_readonly(SYSTEM_PROGRAM, false),
        ],
        data: deposit_data,
    };
    let blockhash = client.get_latest_blockhash().expect("deposit blockhash");
    let deposit_tx = Transaction::new_signed_with_payer(
        &[deposit_ix],
        Some(&payer.pubkey()),
        &[&payer],
        blockhash,
    );
    signatures.push(send_finalized(&client, &deposit_tx, "depositV2"));

    let final_account = client
        .get_account_with_commitment(&vault.pubkey(), CommitmentConfig::finalized())
        .expect("fetch finalized V2 vault")
        .value
        .expect("V2 vault missing at finalized");
    assert_eq!(final_account.owner, program);
    assert_eq!(final_account.data.len(), V2_LEN);
    assert_eq!(layout_id(&final_account.data), layout_id(&migrated.data));
    let recorded_balance = read_u64(&final_account.data, BALANCE_OFFSET);
    let last_deposit = read_u64(&final_account.data, LAST_DEPOSIT_OFFSET);
    assert_eq!(recorded_balance, DEPOSIT_AMOUNT);
    assert_eq!(last_deposit, DEPOSIT_AMOUNT);
    assert_eq!(
        final_account.lamports,
        vault_lamports_before_deposit + DEPOSIT_AMOUNT,
        "canonical System Transfer must fund the vault by the recorded amount"
    );

    let receipt = DevnetReceipt {
        schema: "hopper.devnet-evidence.v1",
        example: "hopper-migration",
        commitment: "finalized",
        rpc_endpoint: "redacted",
        cluster: ClusterReceipt {
            genesis_hash: DEVNET_GENESIS,
            node_version: node.solana_core,
            feature_set: node.feature_set,
        },
        program_id: program.to_string(),
        authority: payer.pubkey().to_string(),
        vault: vault.pubkey().to_string(),
        v1_size: V1_LEN,
        v2_size: V2_LEN,
        deposit_amount: DEPOSIT_AMOUNT,
        recorded_balance,
        last_deposit,
        vault_lamports_before_deposit,
        vault_lamports_after_deposit: final_account.lamports,
        signatures,
    };
    let json = serde_json::to_string_pretty(&receipt).expect("serialize devnet receipt");
    std::fs::write(&receipt_path, format!("{json}\n"))
        .unwrap_or_else(|_| panic!("failed to write HOPPER_DEVNET_RECEIPT (path redacted)"));
    println!("wrote hopper.devnet-evidence.v1 receipt");
}

fn required_receipt_path() -> std::path::PathBuf {
    std::env::var_os("HOPPER_DEVNET_RECEIPT")
        .filter(|path| !path.is_empty())
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| panic!("HOPPER_DEVNET_RECEIPT is required when HOPPER_DEVNET=1"))
}

#[test]
fn rpc_redaction_removes_credentials_and_query_values() {
    assert_eq!(
        redact_rpc_url("https://user:pass@example.invalid/path?key=secret"),
        "https://<redacted>@example.invalid/<redacted>?<redacted>"
    );
}
