//! Devnet integration test for the escrow example.
//!
//! Gated behind `HOPPER_DEVNET=1` so the default `cargo test` run stays
//! offline. When enabled it sends a real `make` (tag 0) instruction to
//! the deployed escrow program, which self-initializes a fresh `Escrow`
//! account via the `init` lifecycle, then reads it back and asserts the
//! offered/wanted amounts round-tripped.
//!
//! The `make` signature it prints is a real Hopper instruction tx that
//! `hopper explain <sig>` decodes against the on-chain manifest.
//!
//! Run with:
//!   HOPPER_DEVNET=1 \
//!   HOPPER_ESCROW_PROGRAM_ID=5Ficb6k1Lv8tV8pThmQLU9H4MAYGbArwGRH2vrTHoPuN \
//!   HOPPER_KEYPAIR=/abs/path/devnet-keypair.json \
//!   cargo test -p hopper-escrow --test devnet -- --nocapture

use std::str::FromStr;

use solana_client::rpc_client::RpcClient;
use solana_commitment_config::CommitmentConfig;
use solana_instruction::{AccountMeta, Instruction};
use solana_keypair::{read_keypair_file, Keypair};
use solana_pubkey::Pubkey;
use solana_signer::Signer;
use solana_transaction::Transaction;

const MAKE_TAG: u8 = 0;
// 16-byte header + maker(32) + maker_ta(32) + mint_a(32) + mint_b(32)
// + amount_offered(8) + amount_wanted(8) + bump(1), rounded for align.
const ESCROW_INIT_SPACE: u64 = 161;
const AMOUNT_OFFERED: u64 = 1_000;
const AMOUNT_WANTED: u64 = 2_000;
// Field offsets within the Escrow account data (after the 16-byte header).
const OFF_AMOUNT_OFFERED: usize = 16 + 32 + 32 + 32 + 32;
const OFF_AMOUNT_WANTED: usize = OFF_AMOUNT_OFFERED + 8;

const SYSTEM_PROGRAM: Pubkey = solana_pubkey::pubkey!("11111111111111111111111111111111");

fn enabled() -> bool {
    std::env::var("HOPPER_DEVNET").as_deref() == Ok("1")
}

fn rpc_url() -> String {
    std::env::var("SOLANA_RPC_URL").unwrap_or_else(|_| "https://api.devnet.solana.com".to_string())
}

fn load_payer() -> Keypair {
    let path = std::env::var("HOPPER_KEYPAIR")
        .expect("set HOPPER_KEYPAIR to the absolute path of the fee-payer/maker keypair");
    read_keypair_file(&path).expect("failed to read HOPPER_KEYPAIR")
}

fn program_id() -> Pubkey {
    let s = std::env::var("HOPPER_ESCROW_PROGRAM_ID")
        .expect("set HOPPER_ESCROW_PROGRAM_ID to the deployed escrow program id");
    Pubkey::from_str(&s).expect("invalid HOPPER_ESCROW_PROGRAM_ID")
}

/// `make` instruction data: tag + mint_a(32) + mint_b(32) +
/// amount_offered(8 LE) + amount_wanted(8 LE).
fn make_ix_data(mint_a: &Pubkey, mint_b: &Pubkey, offered: u64, wanted: u64) -> Vec<u8> {
    let mut data = Vec::with_capacity(1 + 32 + 32 + 8 + 8);
    data.push(MAKE_TAG);
    data.extend_from_slice(&mint_a.to_bytes());
    data.extend_from_slice(&mint_b.to_bytes());
    data.extend_from_slice(&offered.to_le_bytes());
    data.extend_from_slice(&wanted.to_le_bytes());
    data
}

#[test]
fn escrow_make_roundtrip() {
    if !enabled() {
        eprintln!("skipping escrow devnet test (set HOPPER_DEVNET=1 to run)");
        return;
    }

    let client = RpcClient::new_with_commitment(rpc_url(), CommitmentConfig::confirmed());
    let payer = load_payer();
    let program = program_id();

    let escrow = Keypair::new();
    let mint_a = Pubkey::new_unique();
    let mint_b = Pubkey::new_unique();

    // Make: maker (signer, writable, payer), escrow (init, signer),
    // system_program. Order matches `#[derive(Accounts)] struct Make`.
    let ix = Instruction {
        program_id: program,
        accounts: vec![
            AccountMeta::new(payer.pubkey(), true),
            AccountMeta::new(escrow.pubkey(), true),
            AccountMeta::new_readonly(SYSTEM_PROGRAM, false),
        ],
        data: make_ix_data(&mint_a, &mint_b, AMOUNT_OFFERED, AMOUNT_WANTED),
    };

    let blockhash = client.get_latest_blockhash().expect("blockhash");
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&payer.pubkey()),
        &[&payer, &escrow],
        blockhash,
    );

    let sig = client
        .send_and_confirm_transaction(&tx)
        .expect("make tx should succeed");
    eprintln!("escrow make sig {sig}");
    eprintln!("escrow account {}", escrow.pubkey());

    let data = client
        .get_account_data(&escrow.pubkey())
        .expect("get escrow account data");
    assert!(
        data.len() as u64 >= ESCROW_INIT_SPACE,
        "escrow account smaller than expected: {} bytes",
        data.len()
    );

    let offered = u64::from_le_bytes(
        data[OFF_AMOUNT_OFFERED..OFF_AMOUNT_OFFERED + 8]
            .try_into()
            .unwrap(),
    );
    let wanted = u64::from_le_bytes(
        data[OFF_AMOUNT_WANTED..OFF_AMOUNT_WANTED + 8]
            .try_into()
            .unwrap(),
    );
    assert_eq!(offered, AMOUNT_OFFERED, "amount_offered mismatch");
    assert_eq!(wanted, AMOUNT_WANTED, "amount_wanted mismatch");
    eprintln!("escrow round-trip ok: offered={offered} wanted={wanted}");
}
