//! Opt-in public-devnet transaction proof for the Token-2022 vault.
//!
//! This test never chooses a deployment. It requires an already-deployed fresh
//! program id and an explicit fee-payer path, verifies the RPC is public
//! devnet, submits canonical Token-2022/ATA transactions, waits for finalized
//! commitment, re-reads every committed account at finalized, and emits a
//! redacted JSON receipt.

use std::str::FromStr;

use hopper::layout::HEADER_LEN;
use hopper_token_2022_vault::RewardVault;
use serde::Serialize;
use solana_client::rpc_client::RpcClient;
use solana_commitment_config::CommitmentConfig;
use solana_instruction::{AccountMeta, Instruction};
use solana_keypair::{read_keypair_file, Keypair};
use solana_pubkey::Pubkey;
use solana_signer::Signer;
use solana_system_interface::instruction as system_instruction;
use solana_transaction::Transaction;
use spl_associated_token_account_interface::{
    address::get_associated_token_address_with_program_id,
    instruction::create_associated_token_account_idempotent,
};

const DEVNET_GENESIS: &str = "EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG";
const PUBLIC_DEVNET_RPC: &str = "https://api.devnet.solana.com";
const TOKEN_2022: Pubkey = solana_pubkey::pubkey!("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb");
const ASSOCIATED_TOKEN: Pubkey =
    solana_pubkey::pubkey!("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");
const SYSTEM_PROGRAM: Pubkey = solana_pubkey::pubkey!("11111111111111111111111111111111");
const MINT_LEN: usize = 82;
const DECIMALS: u8 = 6;
const MINT_AMOUNT: u64 = 10;
const SWEEP_AMOUNT: u64 = 4;

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
    vault_state: String,
    mint: String,
    vault_ata: String,
    destination_ata: String,
    minted_total: u64,
    swept_total: u64,
    vault_amount: u64,
    destination_amount: u64,
    mint_supply: u64,
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

fn amount_instruction(
    program_id: Pubkey,
    tag: u8,
    amount: u64,
    accounts: Vec<AccountMeta>,
) -> Instruction {
    let mut data = Vec::with_capacity(9);
    data.push(tag);
    data.extend_from_slice(&amount.to_le_bytes());
    Instruction::new_with_bytes(program_id, &data, accounts)
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

fn read_u64(data: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(data[offset..offset + 8].try_into().unwrap())
}

#[test]
fn token_2022_vault_roundtrip_on_public_devnet() {
    if !live_enabled() {
        eprintln!("SKIPPED: set HOPPER_DEVNET=1 for the live Token-2022 vault proof");
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
    let program_id = Pubkey::from_str(
        &std::env::var("HOPPER_TOKEN_2022_VAULT_PROGRAM_ID")
            .expect("set HOPPER_TOKEN_2022_VAULT_PROGRAM_ID to a fresh devnet deployment"),
    )
    .expect("invalid HOPPER_TOKEN_2022_VAULT_PROGRAM_ID");
    let program_account = client
        .get_account_with_commitment(&program_id, CommitmentConfig::finalized())
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
        client.get_balance(&payer.pubkey()).expect("payer balance") > 0,
        "devnet fee payer is unfunded"
    );

    let state = Keypair::new();
    let mint = Keypair::new();
    let recipient = Pubkey::new_unique();
    let vault_ata =
        get_associated_token_address_with_program_id(&payer.pubkey(), &mint.pubkey(), &TOKEN_2022);
    let destination_ata =
        get_associated_token_address_with_program_id(&recipient, &mint.pubkey(), &TOKEN_2022);
    let mint_rent = client
        .get_minimum_balance_for_rent_exemption(MINT_LEN)
        .expect("mint rent exemption");
    let create_mint = system_instruction::create_account(
        &payer.pubkey(),
        &mint.pubkey(),
        mint_rent,
        MINT_LEN as u64,
        &TOKEN_2022,
    );
    let initialize_mint = spl_token_2022_interface::instruction::initialize_mint2(
        &TOKEN_2022,
        &mint.pubkey(),
        &payer.pubkey(),
        None,
        DECIMALS,
    )
    .expect("build InitializeMint2");
    let initialize_vault = Instruction::new_with_bytes(
        program_id,
        &[0],
        vec![
            AccountMeta::new(payer.pubkey(), true),
            AccountMeta::new(state.pubkey(), true),
            AccountMeta::new_readonly(payer.pubkey(), true),
            AccountMeta::new_readonly(SYSTEM_PROGRAM, false),
        ],
    );
    let blockhash = client.get_latest_blockhash().expect("init blockhash");
    let init_tx = Transaction::new_signed_with_payer(
        &[create_mint, initialize_mint, initialize_vault],
        Some(&payer.pubkey()),
        &[&payer, &mint, &state],
        blockhash,
    );
    let mut signatures = vec![send_finalized(&client, &init_tx, "initialize")];

    let create_destination = create_associated_token_account_idempotent(
        &payer.pubkey(),
        &recipient,
        &mint.pubkey(),
        &TOKEN_2022,
    );
    let prepare = Instruction::new_with_bytes(
        program_id,
        &[1],
        vec![
            AccountMeta::new(payer.pubkey(), true),
            AccountMeta::new_readonly(payer.pubkey(), true),
            AccountMeta::new(state.pubkey(), false),
            AccountMeta::new(vault_ata, false),
            AccountMeta::new_readonly(mint.pubkey(), false),
            AccountMeta::new_readonly(SYSTEM_PROGRAM, false),
            AccountMeta::new_readonly(TOKEN_2022, false),
            AccountMeta::new_readonly(ASSOCIATED_TOKEN, false),
        ],
    );
    let blockhash = client.get_latest_blockhash().expect("prepare blockhash");
    let prepare_tx = Transaction::new_signed_with_payer(
        &[create_destination, prepare],
        Some(&payer.pubkey()),
        &[&payer],
        blockhash,
    );
    signatures.push(send_finalized(&client, &prepare_tx, "prepare"));

    let mint_ix = amount_instruction(
        program_id,
        2,
        MINT_AMOUNT,
        vec![
            AccountMeta::new_readonly(payer.pubkey(), true),
            AccountMeta::new(state.pubkey(), false),
            AccountMeta::new(vault_ata, false),
            AccountMeta::new(mint.pubkey(), false),
            AccountMeta::new_readonly(TOKEN_2022, false),
        ],
    );
    let blockhash = client.get_latest_blockhash().expect("mint blockhash");
    let mint_tx =
        Transaction::new_signed_with_payer(&[mint_ix], Some(&payer.pubkey()), &[&payer], blockhash);
    signatures.push(send_finalized(&client, &mint_tx, "mint"));

    let sweep_ix = amount_instruction(
        program_id,
        3,
        SWEEP_AMOUNT,
        vec![
            AccountMeta::new_readonly(payer.pubkey(), true),
            AccountMeta::new(state.pubkey(), false),
            AccountMeta::new(vault_ata, false),
            AccountMeta::new(destination_ata, false),
            AccountMeta::new_readonly(mint.pubkey(), false),
            AccountMeta::new_readonly(TOKEN_2022, false),
        ],
    );
    let blockhash = client.get_latest_blockhash().expect("sweep blockhash");
    let sweep_tx = Transaction::new_signed_with_payer(
        &[sweep_ix],
        Some(&payer.pubkey()),
        &[&payer],
        blockhash,
    );
    signatures.push(send_finalized(&client, &sweep_tx, "sweep"));

    let state_account = client
        .get_account_with_commitment(&state.pubkey(), CommitmentConfig::finalized())
        .expect("fetch finalized state")
        .value
        .expect("vault state missing at finalized");
    assert_eq!(state_account.owner, program_id);
    assert_eq!(state_account.data.len(), RewardVault::LEN);
    assert_eq!(
        &state_account.data[HEADER_LEN..HEADER_LEN + 32],
        payer.pubkey().as_ref()
    );
    assert_eq!(
        &state_account.data[HEADER_LEN + 32..HEADER_LEN + 64],
        mint.pubkey().as_ref()
    );
    assert_eq!(
        &state_account.data[HEADER_LEN + 64..HEADER_LEN + 96],
        vault_ata.as_ref()
    );
    let minted_total = read_u64(&state_account.data, HEADER_LEN + 96);
    let swept_total = read_u64(&state_account.data, HEADER_LEN + 104);
    assert_eq!(minted_total, MINT_AMOUNT);
    assert_eq!(swept_total, SWEEP_AMOUNT);

    let vault_account = client
        .get_account_with_commitment(&vault_ata, CommitmentConfig::finalized())
        .expect("fetch finalized vault ATA")
        .value
        .expect("vault ATA missing at finalized");
    let destination_account = client
        .get_account_with_commitment(&destination_ata, CommitmentConfig::finalized())
        .expect("fetch finalized destination ATA")
        .value
        .expect("destination ATA missing at finalized");
    let mint_account = client
        .get_account_with_commitment(&mint.pubkey(), CommitmentConfig::finalized())
        .expect("fetch finalized mint")
        .value
        .expect("mint missing at finalized");
    assert_eq!(mint_account.owner, TOKEN_2022);
    assert!(mint_account.data.len() >= MINT_LEN);
    assert_eq!(mint_account.data[44], DECIMALS);
    assert_eq!(mint_account.data[45], 1);
    let mint_supply = read_u64(&mint_account.data, 36);
    assert_eq!(mint_supply, MINT_AMOUNT);
    for account in [&vault_account, &destination_account] {
        assert_eq!(account.owner, TOKEN_2022);
        assert!(account.data.len() >= 165);
        assert_eq!(&account.data[..32], mint.pubkey().as_ref());
        assert_eq!(account.data[108], 1);
    }
    assert_eq!(&vault_account.data[32..64], payer.pubkey().as_ref());
    let vault_amount = read_u64(&vault_account.data, 64);
    let destination_amount = read_u64(&destination_account.data, 64);
    assert_eq!(vault_amount, MINT_AMOUNT - SWEEP_AMOUNT);
    assert_eq!(destination_amount, SWEEP_AMOUNT);

    let receipt = DevnetReceipt {
        schema: "hopper.devnet-evidence.v1",
        example: "hopper-token-2022-vault",
        commitment: "finalized",
        rpc_endpoint: "redacted",
        cluster: ClusterReceipt {
            genesis_hash: DEVNET_GENESIS,
            node_version: node.solana_core,
            feature_set: node.feature_set,
        },
        program_id: program_id.to_string(),
        authority: payer.pubkey().to_string(),
        vault_state: state.pubkey().to_string(),
        mint: mint.pubkey().to_string(),
        vault_ata: vault_ata.to_string(),
        destination_ata: destination_ata.to_string(),
        minted_total,
        swept_total,
        vault_amount,
        destination_amount,
        mint_supply,
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
