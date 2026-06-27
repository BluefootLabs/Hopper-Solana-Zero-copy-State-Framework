//! Devnet integration test for the layout-migration example.
//!
//! Gated behind `HOPPER_DEVNET=1` so the default `cargo test` run stays
//! offline. When enabled it:
//!   1. sends `init_v1` (tag 0) to self-create a fresh V1 vault,
//!   2. reads it back and asserts the V1 layout size,
//!   3. sends `migrate_v1_to_v2` (tag 1) to grow it in place,
//!   4. reads it back and asserts the V2 layout id changed + grew.
//!
//! This is the on-chain counterpart to `hopper migrate`: the program
//! itself performs the append-only `LayoutMigration` against a live
//! devnet account.
//!
//! Run with:
//!   HOPPER_DEVNET=1 \
//!   HOPPER_MIGRATION_PROGRAM_ID=EuDECNLNwPAptWC5NmenBBfjSuhZtmpPwpMQ7Z1P2GMt \
//!   HOPPER_KEYPAIR=/abs/path/devnet-keypair.json \
//!   cargo test -p hopper-migration --test devnet -- --nocapture

use std::str::FromStr;

use solana_client::rpc_client::RpcClient;
use solana_commitment_config::CommitmentConfig;
use solana_instruction::{AccountMeta, Instruction};
use solana_keypair::{read_keypair_file, Keypair};
use solana_pubkey::Pubkey;
use solana_signer::Signer;
use solana_transaction::Transaction;

const INIT_V1_TAG: u8 = 0;
const MIGRATE_TAG: u8 = 1;
const V1_LEN: usize = 16 + 32 + 8; // 56
const V2_LEN: usize = 16 + 32 + 8 + 1 + 8; // 65
const NEW_BUMP: u8 = 254;

const SYSTEM_PROGRAM: Pubkey = solana_pubkey::pubkey!("11111111111111111111111111111111");

fn enabled() -> bool {
    std::env::var("HOPPER_DEVNET").as_deref() == Ok("1")
}

fn rpc_url() -> String {
    std::env::var("SOLANA_RPC_URL").unwrap_or_else(|_| "https://api.devnet.solana.com".to_string())
}

fn load_payer() -> Keypair {
    let path = std::env::var("HOPPER_KEYPAIR")
        .expect("set HOPPER_KEYPAIR to the absolute path of the fee-payer/authority keypair");
    read_keypair_file(&path).expect("failed to read HOPPER_KEYPAIR")
}

fn program_id() -> Pubkey {
    let s = std::env::var("HOPPER_MIGRATION_PROGRAM_ID")
        .expect("set HOPPER_MIGRATION_PROGRAM_ID to the deployed migration program id");
    Pubkey::from_str(&s).expect("invalid HOPPER_MIGRATION_PROGRAM_ID")
}

/// The layout id occupies header bytes [4, 12). We assert the layout id
/// *changes* across the migration rather than hard-coding the
/// macro-derived fingerprint, which keeps the test robust to a layout-id
/// recomputation in the framework.
fn layout_id(data: &[u8]) -> [u8; 8] {
    data[4..12]
        .try_into()
        .expect("header carries an 8-byte layout id")
}

#[test]
fn migration_v1_to_v2_roundtrip() {
    if !enabled() {
        eprintln!("skipping migration devnet test (set HOPPER_DEVNET=1 to run)");
        return;
    }

    let client = RpcClient::new_with_commitment(rpc_url(), CommitmentConfig::confirmed());
    let payer = load_payer();
    let program = program_id();
    let vault = Keypair::new();

    // 1. init_v1: payer (signer, writable), vault (init, signer),
    //    system_program.
    let init_ix = Instruction {
        program_id: program,
        accounts: vec![
            AccountMeta::new(payer.pubkey(), true),
            AccountMeta::new(vault.pubkey(), true),
            AccountMeta::new_readonly(SYSTEM_PROGRAM, false),
        ],
        data: vec![INIT_V1_TAG],
    };
    let bh = client.get_latest_blockhash().expect("blockhash");
    let init_tx = Transaction::new_signed_with_payer(
        &[init_ix],
        Some(&payer.pubkey()),
        &[&payer, &vault],
        bh,
    );
    let init_sig = client
        .send_and_confirm_transaction(&init_tx)
        .expect("init_v1 tx should succeed");
    eprintln!("init_v1 sig {init_sig}");
    eprintln!("vault account {}", vault.pubkey());

    let v1_data = client
        .get_account_data(&vault.pubkey())
        .expect("get vault data after init");
    assert_eq!(
        v1_data.len(),
        V1_LEN,
        "freshly-initialized vault is V1-sized"
    );
    let v1_layout = layout_id(&v1_data);

    // 2. migrate_v1_to_v2: authority (signer), vault (writable). The
    //    instruction data byte 0 is the new V2 `bump`.
    let migrate_ix = Instruction {
        program_id: program,
        accounts: vec![
            AccountMeta::new(payer.pubkey(), true),
            AccountMeta::new(vault.pubkey(), false),
            AccountMeta::new_readonly(SYSTEM_PROGRAM, false),
        ],
        data: vec![MIGRATE_TAG, NEW_BUMP],
    };
    let bh = client.get_latest_blockhash().expect("blockhash");
    let migrate_tx =
        Transaction::new_signed_with_payer(&[migrate_ix], Some(&payer.pubkey()), &[&payer], bh);
    let migrate_sig = client
        .send_and_confirm_transaction(&migrate_tx)
        .expect("migrate_v1_to_v2 tx should succeed");
    eprintln!("migrate sig {migrate_sig}");

    let v2_data = client
        .get_account_data(&vault.pubkey())
        .expect("get vault data after migrate");
    assert_eq!(v2_data.len(), V2_LEN, "migrated vault grew to V2 size");
    let v2_layout = layout_id(&v2_data);
    assert_ne!(
        v1_layout, v2_layout,
        "layout id must change across an append migration"
    );
    // The appended bump byte sits right after the V1 region (16+32+8).
    assert_eq!(v2_data[V1_LEN], NEW_BUMP, "appended bump byte round-trips");
    eprintln!("migration round-trip ok: {V1_LEN}B V1 -> {V2_LEN}B V2");
}
