//! Opt-in finalized devnet proof for exact byte-range isolation in the
//! segmented orderbook.

use std::{ops::Range, str::FromStr};

use hopper::hopper_core::account::registry::{segment_id, SegmentRegistry};
use serde::Serialize;
use solana_client::rpc_client::RpcClient;
use solana_commitment_config::CommitmentConfig;
use solana_instruction::{AccountMeta, Instruction};
use solana_keypair::{read_keypair_file, Keypair};
use solana_pubkey::Pubkey;
use solana_signer::Signer;
use solana_system_interface::instruction as system_instruction;
use solana_transaction::Transaction;

const DEVNET_GENESIS: &str = "EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG";
const PUBLIC_DEVNET_RPC: &str = "https://api.devnet.solana.com";
const INIT_BOOK_TAG: u8 = 0;
const POST_BID_TAG: u8 = 1;
const SYSTEM_PROGRAM: Pubkey = solana_pubkey::pubkey!("11111111111111111111111111111111");
const PRICE: u64 = 1_000;
const SIZE: u64 = 5;
const SEQUENCE: u64 = 1;

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
struct SegmentReceipt {
    name: &'static str,
    start: usize,
    end: usize,
    size: usize,
}

#[derive(Serialize)]
struct DevnetReceipt {
    schema: &'static str,
    example: &'static str,
    commitment: &'static str,
    rpc_endpoint: &'static str,
    cluster: ClusterReceipt,
    program_id: String,
    owner: String,
    book: String,
    book_size: usize,
    bids: SegmentReceipt,
    asks: SegmentReceipt,
    events: SegmentReceipt,
    bid_count: u32,
    bid_price: u64,
    bid_size: u64,
    bid_sequence: u64,
    unchanged_outside_bids: bool,
    unchanged_outside_record_and_count: bool,
    /// Every transaction the lane sent, as the evidence contract names it.
    #[serde(rename = "transactions")]
    signatures: Vec<SignatureReceipt>,
}

struct BookRanges {
    bids: Range<usize>,
    asks: Range<usize>,
    events: Range<usize>,
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

fn segment_range(registry: &SegmentRegistry<'_>, name: &str) -> Range<usize> {
    let (_, entry) = registry
        .find(&segment_id(name))
        .unwrap_or_else(|error| panic!("missing {name} registry entry: {error:?}"));
    let start = entry.offset() as usize;
    start..start + entry.size() as usize
}

fn decode_ranges(data: &[u8]) -> BookRanges {
    let registry = SegmentRegistry::from_account(data).expect("decode segment registry");
    assert_eq!(registry.segment_count(), 3);
    let bids = segment_range(&registry, "bids");
    let asks = segment_range(&registry, "asks");
    let events = segment_range(&registry, "events");
    assert_eq!(bids.start, registry.data_region_offset());
    assert_eq!(bids.end, asks.start);
    assert_eq!(asks.end, events.start);
    assert_eq!(events.end, data.len());
    assert_eq!(bids.len(), 8 + 1_024 * 56);
    assert_eq!(asks.len(), 8 + 1_024 * 56);
    assert_eq!(events.len(), 8 + 512 * 48);
    BookRanges { bids, asks, events }
}

fn bid_data() -> Vec<u8> {
    let mut data = Vec::with_capacity(25);
    data.push(POST_BID_TAG);
    data.extend_from_slice(&PRICE.to_le_bytes());
    data.extend_from_slice(&SIZE.to_le_bytes());
    data.extend_from_slice(&SEQUENCE.to_le_bytes());
    data
}

fn read_u32(data: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap())
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

fn segment_receipt(name: &'static str, range: &Range<usize>) -> SegmentReceipt {
    SegmentReceipt {
        name,
        start: range.start,
        end: range.end,
        size: range.len(),
    }
}

#[test]
fn orderbook_post_bid_changes_only_the_decoded_bids_record() {
    if !live_enabled() {
        eprintln!("SKIPPED: set HOPPER_DEVNET=1 for the live orderbook proof");
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
        &std::env::var("HOPPER_ORDERBOOK_PROGRAM_ID")
            .expect("set HOPPER_ORDERBOOK_PROGRAM_ID to a fresh devnet deployment"),
    )
    .expect("invalid HOPPER_ORDERBOOK_PROGRAM_ID");
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
    let book = Keypair::new();
    let lamports = client
        .get_minimum_balance_for_rent_exemption(hopper_orderbook::BOOK_ACCOUNT_SIZE)
        .expect("book rent exemption");
    assert!(
        client.get_balance(&payer.pubkey()).expect("payer balance") > lamports,
        "devnet fee payer cannot fund the large book account"
    );
    let create_book = system_instruction::create_account(
        &payer.pubkey(),
        &book.pubkey(),
        lamports,
        hopper_orderbook::BOOK_ACCOUNT_SIZE as u64,
        &program,
    );
    let init_ix = Instruction {
        program_id: program,
        accounts: vec![
            AccountMeta::new(payer.pubkey(), true),
            AccountMeta::new(book.pubkey(), true),
            AccountMeta::new_readonly(SYSTEM_PROGRAM, false),
        ],
        data: vec![INIT_BOOK_TAG],
    };
    let blockhash = client.get_latest_blockhash().expect("init blockhash");
    let init_tx = Transaction::new_signed_with_payer(
        &[create_book, init_ix],
        Some(&payer.pubkey()),
        &[&payer, &book],
        blockhash,
    );
    let mut signatures = vec![send_finalized(&client, &init_tx, "initBook")];
    let after_init_account = client
        .get_account_with_commitment(&book.pubkey(), CommitmentConfig::finalized())
        .expect("fetch initialized book")
        .value
        .expect("initialized book missing at finalized");
    assert_eq!(after_init_account.owner, program);
    assert_eq!(
        after_init_account.data.len(),
        hopper_orderbook::BOOK_ACCOUNT_SIZE
    );
    let ranges = decode_ranges(&after_init_account.data);
    for range in [&ranges.bids, &ranges.asks, &ranges.events] {
        assert!(
            after_init_account.data[range.clone()]
                .iter()
                .all(|byte| *byte == 0),
            "initialized segment must be zeroed"
        );
    }

    let bid_ix = Instruction {
        program_id: program,
        accounts: vec![
            AccountMeta::new(payer.pubkey(), true),
            AccountMeta::new(book.pubkey(), false),
        ],
        data: bid_data(),
    };
    let blockhash = client.get_latest_blockhash().expect("post-bid blockhash");
    let bid_tx =
        Transaction::new_signed_with_payer(&[bid_ix], Some(&payer.pubkey()), &[&payer], blockhash);
    signatures.push(send_finalized(&client, &bid_tx, "postBid"));
    let after_bid_account = client
        .get_account_with_commitment(&book.pubkey(), CommitmentConfig::finalized())
        .expect("fetch book after bid")
        .value
        .expect("book missing after finalized bid");
    assert_eq!(after_bid_account.owner, program);
    assert_eq!(after_bid_account.data.len(), after_init_account.data.len());
    let final_ranges = decode_ranges(&after_bid_account.data);
    assert_eq!(final_ranges.bids, ranges.bids);
    assert_eq!(final_ranges.asks, ranges.asks);
    assert_eq!(final_ranges.events, ranges.events);

    let start = ranges.bids.start;
    assert_eq!(read_u32(&after_bid_account.data, start), 1);
    assert_eq!(read_u32(&after_bid_account.data, start + 4), 0);
    assert_eq!(
        &after_bid_account.data[start + 8..start + 40],
        payer.pubkey().as_ref()
    );
    assert_eq!(read_u64(&after_bid_account.data, start + 40), PRICE);
    assert_eq!(read_u64(&after_bid_account.data, start + 48), SIZE);
    assert_eq!(read_u64(&after_bid_account.data, start + 56), SEQUENCE);

    assert_eq!(
        &after_bid_account.data[..ranges.bids.start],
        &after_init_account.data[..ranges.bids.start],
        "header and segment registry changed"
    );
    assert_eq!(
        &after_bid_account.data[ranges.asks.clone()],
        &after_init_account.data[ranges.asks.clone()],
        "post_bid changed asks"
    );
    assert_eq!(
        &after_bid_account.data[ranges.events.clone()],
        &after_init_account.data[ranges.events.clone()],
        "post_bid changed events"
    );
    assert_eq!(
        &after_bid_account.data[start + 4..start + 8],
        &after_init_account.data[start + 4..start + 8],
        "post_bid changed the bids head"
    );
    assert_eq!(
        &after_bid_account.data[start + 64..ranges.bids.end],
        &after_init_account.data[start + 64..ranges.bids.end],
        "post_bid changed unused bids bytes"
    );

    let receipt = DevnetReceipt {
        schema: "hopper.devnet-evidence.v1",
        example: "hopper-orderbook",
        commitment: "finalized",
        rpc_endpoint: "redacted",
        cluster: ClusterReceipt {
            genesis_hash: DEVNET_GENESIS,
            node_version: node.solana_core,
            feature_set: node.feature_set,
        },
        program_id: program.to_string(),
        owner: payer.pubkey().to_string(),
        book: book.pubkey().to_string(),
        book_size: after_bid_account.data.len(),
        bids: segment_receipt("bids", &ranges.bids),
        asks: segment_receipt("asks", &ranges.asks),
        events: segment_receipt("events", &ranges.events),
        bid_count: 1,
        bid_price: PRICE,
        bid_size: SIZE,
        bid_sequence: SEQUENCE,
        unchanged_outside_bids: true,
        unchanged_outside_record_and_count: true,
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
