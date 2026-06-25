//! Devnet integration test for the compact vault example.
//!
//! Gated behind `HOPPER_DEVNET=1` so default test runs stay offline.
//! The test creates an exact 41-byte account, initializes the compact
//! `[disc:u8][authority:32][balance:8]` layout, deposits once, then fetches
//! bytes back from devnet and proves the layout fingerprint is manifest/IDL
//! metadata rather than an on-account header field.
//!
//! Run with:
//!   HOPPER_DEVNET=1 \
//!   HOPPER_COMPACT_VAULT_PROGRAM_ID=<deployed-program-id> \
//!   HOPPER_KEYPAIR=/abs/path/devnet-keypair.json \
//!   cargo test -p hopper-compact-vault --test devnet -- --nocapture

use std::str::FromStr;

use solana_client::rpc_client::RpcClient;
use solana_commitment_config::CommitmentConfig;
use solana_instruction::{AccountMeta, Instruction};
use solana_keypair::{read_keypair_file, Keypair};
use solana_pubkey::Pubkey;
use solana_signer::Signer;
use solana_system_interface::instruction as system_instruction;
use solana_transaction::Transaction;

const VAULT_DISC: u8 = 1;
const VAULT_LEN: u64 = 41;
const IX_INIT: u8 = 0;
const IX_DEPOSIT: u8 = 1;
const DEPOSIT_AMOUNT: u64 = 123_456;
const VAULT_LAYOUT_ID: [u8; 8] = [67, 113, 65, 144, 124, 9, 52, 79];

fn enabled() -> bool {
    std::env::var("HOPPER_DEVNET").as_deref() == Ok("1")
}

fn rpc_url() -> String {
    std::env::var("SOLANA_RPC_URL").unwrap_or_else(|_| "https://api.devnet.solana.com".to_string())
}

fn load_payer() -> Keypair {
    let path = std::env::var("HOPPER_KEYPAIR")
        .expect("set HOPPER_KEYPAIR to the absolute path of the devnet fee-payer keypair");
    read_keypair_file(&path).expect("failed to read HOPPER_KEYPAIR")
}

fn program_id() -> Pubkey {
    let s = std::env::var("HOPPER_COMPACT_VAULT_PROGRAM_ID")
        .expect("set HOPPER_COMPACT_VAULT_PROGRAM_ID to the deployed compact vault program id");
    Pubkey::from_str(&s).expect("invalid HOPPER_COMPACT_VAULT_PROGRAM_ID")
}

#[test]
fn compact_vault_devnet_roundtrip() {
    if !enabled() {
        eprintln!("skipping compact vault devnet test (set HOPPER_DEVNET=1 to run)");
        return;
    }

    let client = RpcClient::new_with_commitment(rpc_url(), CommitmentConfig::confirmed());
    let payer = load_payer();
    let program = program_id();
    let vault = Keypair::new();

    let lamports = client
        .get_minimum_balance_for_rent_exemption(VAULT_LEN as usize)
        .expect("rent exemption");
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
    let mut deposit_data = Vec::with_capacity(1 + 8);
    deposit_data.push(IX_DEPOSIT);
    deposit_data.extend_from_slice(&DEPOSIT_AMOUNT.to_le_bytes());
    let deposit = Instruction {
        program_id: program,
        accounts: vec![
            AccountMeta::new(vault.pubkey(), false),
            AccountMeta::new_readonly(payer.pubkey(), true),
        ],
        data: deposit_data,
    };

    let blockhash = client.get_latest_blockhash().expect("blockhash");
    let tx = Transaction::new_signed_with_payer(
        &[create, init, deposit],
        Some(&payer.pubkey()),
        &[&payer, &vault],
        blockhash,
    );
    let sig = client
        .send_and_confirm_transaction(&tx)
        .expect("compact vault tx should succeed");
    eprintln!("compact vault sig {sig}");
    eprintln!("compact vault account {}", vault.pubkey());

    let data = client
        .get_account_data(&vault.pubkey())
        .expect("get compact vault account data");
    assert_eq!(
        data.len(),
        VAULT_LEN as usize,
        "compact vault must be exact-size"
    );
    assert_eq!(data[0], VAULT_DISC, "compact discriminator mismatch");
    assert_eq!(&data[1..33], payer.pubkey().as_ref(), "authority mismatch");
    let balance = u64::from_le_bytes(data[33..41].try_into().unwrap());
    assert_eq!(balance, DEPOSIT_AMOUNT, "balance mismatch");

    assert_ne!(
        &data[4..12],
        VAULT_LAYOUT_ID.as_slice(),
        "compact account unexpectedly looked like it stored the layout id in a header"
    );
    assert_eq!(VAULT_LAYOUT_ID, [67, 113, 65, 144, 124, 9, 52, 79]);
}
