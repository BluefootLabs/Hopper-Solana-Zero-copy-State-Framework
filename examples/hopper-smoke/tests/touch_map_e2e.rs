//! Self-describing transactions (innovation I7), proven end-to-end on
//! the compiled SBF artifact.
//!
//! The `Withdraw` context in this example opts in via
//! `#[accounts(strict_writes, emit_touch_map)]`. This test executes the
//! real `hopper_smoke.so` inside an in-process SVM (Mollusk, via
//! `hopper-test`), captures the execution's log stream — the same
//! `logMessages` lines an RPC `getTransaction` would return — and feeds
//! it through the touch-map decode that `hopper tx explain` applies to
//! on-chain transactions:
//!
//! - a **successful** withdraw must yield exactly ONE decodable
//!   touch-map record (`Program data:` payload with magic `0x7A`,
//!   version `0x01`, exact-length `4 + 9 * count`), whose entries match
//!   what the handler actually touched: one Write of the vault's
//!   `balance` field bytes;
//! - a **failed** withdraw (insufficient balance — the handler touches
//!   the balance segment FIRST, then errors) must yield ZERO touch-map
//!   records, because the generated dispatcher only emits on the
//!   handler's Ok path (the CONFIRMED-P2 fix).
//!
//! Decode-path note: `hopper tx explain`'s decoder functions live in
//! `tools/hopper-cli/src/cmd/tx_explain.rs`, which is a bin-only crate,
//! so they are not linkable from here. `decode_touch_map` /
//! `extract_touch_maps` below replicate that decode 1:1 against the
//! PUBLIC wire-format constants in `hopper_runtime::segment_borrow`,
//! and the expected payload is additionally byte-compared against the
//! public `encode_touch_map` encoder — the same function the on-chain
//! emission uses — so producer and consumer are pinned to each other.
//!
//! Run `cargo build-sbf` in this example's directory first; the test
//! skips (with a notice) when the SBF artifact has not been built.

use base64::Engine;
use hopper::hopper_runtime::segment_borrow::{
    encode_touch_map, TouchMapRecord, TOUCH_MAP_HEADER_LEN, TOUCH_MAP_MAGIC, TOUCH_MAP_RECORD_LEN,
    TOUCH_MAP_VERSION,
};
use hopper::layout::write_header;
use hopper_smoke::Vault;
use hopper_test::LiteSvmHarness;
use solana_account::Account;
use solana_instruction::{AccountMeta, Instruction};
use solana_pubkey::Pubkey;

/// Workspace-relative ELF path stem (tests run with the example dir as
/// CWD; `cargo build-sbf` writes to the workspace `target/deploy`).
const ELF_PATH_STEM: &str = "../../target/deploy/hopper_smoke";

/// `#[instruction(2)]` in `smoke_program`.
const WITHDRAW_DISC: u8 = 2;

/// Lamports the vault account holds (rent + deposited funds).
const VAULT_LAMPORTS: u64 = 10_000_000;
/// The `balance` field value seeded into vault state.
const VAULT_STATE_BALANCE: u64 = 5_000_000;
/// `WireU64` — the touched `balance` field is 8 bytes on the wire.
const BALANCE_FIELD_SIZE: u32 = 8;

// ── Fixtures ────────────────────────────────────────────────────────

fn withdraw_instruction(
    program_id: Pubkey,
    authority: Pubkey,
    vault: Pubkey,
    amount: u64,
) -> Instruction {
    // `[disc][u64 LE amount]` — the generated decoder enforces the exact
    // length, mirroring the devnet client's encoding.
    let mut data = Vec::with_capacity(9);
    data.push(WITHDRAW_DISC);
    data.extend_from_slice(&amount.to_le_bytes());
    Instruction::new_with_bytes(
        program_id,
        &data,
        vec![
            AccountMeta::new(authority, true),
            AccountMeta::new(vault, false),
        ],
    )
}

/// A program-owned vault account with a valid Hopper header and seeded
/// `authority` + `balance` fields, exactly as `initialize` + `deposit`
/// would have left it.
fn seeded_vault(program_id: &Pubkey, authority: &Pubkey) -> Account {
    let mut data = vec![0u8; Vault::LEN];
    write_header(&mut data, Vault::DISC, Vault::VERSION, &Vault::LAYOUT_ID)
        .expect("vault fixture header");
    let auth = Vault::AUTHORITY_ABS_OFFSET as usize;
    data[auth..auth + 32].copy_from_slice(&authority.to_bytes());
    let bal = Vault::BALANCE_ABS_OFFSET as usize;
    data[bal..bal + BALANCE_FIELD_SIZE as usize]
        .copy_from_slice(&VAULT_STATE_BALANCE.to_le_bytes());
    Account {
        lamports: VAULT_LAMPORTS,
        data,
        owner: *program_id,
        executable: false,
        rent_epoch: 0,
    }
}

/// Load the harness, or `None` (skip) when the SBF artifact is absent.
fn harness(program_id: &Pubkey) -> Option<LiteSvmHarness> {
    let svm = LiteSvmHarness::load(program_id, ELF_PATH_STEM);
    if svm.is_none() {
        eprintln!(
            "SKIPPED: {ELF_PATH_STEM}.so not found - run `cargo build-sbf` in \
             examples/hopper-smoke first"
        );
    }
    svm
}

// ── The `hopper tx explain` decode, replicated ──────────────────────

/// Decode one touch-map `sol_log_data` payload the way `hopper tx
/// explain` does: magic, version, and the exact-length equation must
/// all hold, then each 9-byte record is `slot u8`, `packed u32 LE`
/// (top bit = write, low 31 bits = offset), `size u32 LE`.
fn decode_touch_map(bytes: &[u8]) -> Option<(u8, Vec<TouchMapRecord>)> {
    if bytes.len() < TOUCH_MAP_HEADER_LEN
        || bytes[0] != TOUCH_MAP_MAGIC
        || bytes[1] != TOUCH_MAP_VERSION
    {
        return None;
    }
    let flags = bytes[2];
    let count = bytes[3] as usize;
    if bytes.len() != TOUCH_MAP_HEADER_LEN + count * TOUCH_MAP_RECORD_LEN {
        return None;
    }
    let mut records = Vec::with_capacity(count);
    for i in 0..count {
        let base = TOUCH_MAP_HEADER_LEN + i * TOUCH_MAP_RECORD_LEN;
        let packed = u32::from_le_bytes(bytes[base + 1..base + 5].try_into().unwrap());
        records.push(TouchMapRecord {
            slot: bytes[base],
            offset: packed & 0x7FFF_FFFF,
            size: u32::from_le_bytes(bytes[base + 5..base + 9].try_into().unwrap()),
            write: packed & 0x8000_0000 != 0,
        });
    }
    Some((flags, records))
}

/// Scan a captured log stream for decodable touch maps, the way
/// `hopper tx explain` scans a transaction's `logMessages`: only
/// `Program data:` lines are considered, every whitespace-separated
/// base64 segment is tried, and non-touch-map payloads (events, other
/// `sol_log_data` users) are skipped by the magic/version/length check.
/// Returns `(raw_payload, flags, records)` per decoded map.
fn extract_touch_maps(log_lines: &[String]) -> Vec<(Vec<u8>, u8, Vec<TouchMapRecord>)> {
    let mut out = Vec::new();
    for line in log_lines {
        let Some(rest) = line.strip_prefix("Program data: ") else {
            continue;
        };
        for seg in rest.split_whitespace() {
            let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(seg) else {
                continue;
            };
            if let Some((flags, records)) = decode_touch_map(&bytes) {
                out.push((bytes, flags, records));
            }
        }
    }
    out
}

// ── Tests ───────────────────────────────────────────────────────────

/// Ok path: the withdraw succeeds, and the log stream carries exactly
/// one decodable touch-map record describing the handler's actual
/// state effect — one Write covering the vault's `balance` bytes.
#[test]
fn ok_withdraw_emits_exactly_one_touch_map_matching_the_balance_write() {
    let program_id = Pubkey::new_unique();
    let Some(mut svm) = harness(&program_id) else {
        return;
    };

    let authority = Pubkey::new_unique();
    let vault = Pubkey::new_unique();
    let withdraw_amount: u64 = 2_000_000;

    svm.capture_logs();
    let result = svm.process(
        &withdraw_instruction(program_id, authority, vault, withdraw_amount),
        &[
            (
                authority,
                Account::new(1_000_000_000, 0, &Pubkey::default()),
            ),
            (vault, seeded_vault(&program_id, &authority)),
        ],
    );
    assert!(
        result.succeeded(),
        "withdraw must succeed; logs: {:#?}",
        svm.logs()
    );

    // Exactly ONE decodable touch-map record in the whole stream.
    let logs = svm.logs();
    let maps = extract_touch_maps(&logs);
    assert_eq!(
        maps.len(),
        1,
        "a successful opted-in withdraw must emit exactly one touch map; logs: {logs:#?}"
    );
    let (payload, flags, records) = &maps[0];

    // Complete map: no overflow, nothing skipped.
    assert_eq!(*flags, 0, "map must be complete (no overflow/skip flags)");

    // The decoded entries are exactly what the handler touched: one
    // Write of the vault's `balance` field — account slot 1 (the
    // instruction's second account), byte range [48..56) inside the
    // account data (16-byte Hopper header + 32-byte `authority` field).
    let expected = vec![TouchMapRecord {
        slot: 1,
        offset: Vault::BALANCE_ABS_OFFSET,
        size: BALANCE_FIELD_SIZE,
        write: true,
    }];
    assert_eq!(
        *records, expected,
        "decoded record must be the balance write the handler performed"
    );
    assert_eq!(Vault::BALANCE_ABS_OFFSET, 48, "layout drift guard");

    // Byte-for-byte round trip through the PUBLIC wire-format encoder —
    // the same `encode_touch_map` the on-chain emission uses — so the
    // decode above is provably reading the producer's format.
    let (expected_buf, expected_len) = encode_touch_map(&expected, false, false);
    assert_eq!(
        payload.as_slice(),
        &expected_buf[..expected_len],
        "emitted payload must round-trip the public encoder byte-for-byte"
    );

    // The slot join `hopper tx explain` renders: record slot 1 is the
    // vault account of this instruction.
    let ix = withdraw_instruction(program_id, authority, vault, withdraw_amount);
    assert_eq!(ix.accounts[records[0].slot as usize].pubkey, vault);

    // And the described write really happened: state balance debited,
    // lamports settled to the authority.
    let bal = Vault::BALANCE_ABS_OFFSET as usize;
    let post_balance = u64::from_le_bytes(
        result.account(1).data[bal..bal + BALANCE_FIELD_SIZE as usize]
            .try_into()
            .unwrap(),
    );
    assert_eq!(post_balance, VAULT_STATE_BALANCE - withdraw_amount);
    assert_eq!(result.lamports(1), VAULT_LAMPORTS - withdraw_amount);
    assert_eq!(result.lamports(0), 1_000_000_000 + withdraw_amount);
}

/// The wrapper path, on-chain: `bump_whole_vault` (instruction 5)
/// writes through `vault.get_mut()` — a whole-account borrow with no
/// segment lease — under `#[accounts(emit_touch_map)]`. Before the
/// instruction-ambient touch log, this exact shape was the touch map's
/// disclosed blind spot (wrapper borrows never reached the
/// Context-owned registry). Now the emitted map must carry exactly one
/// record: a full-account write of the vault (slot 1, `[0..Vault::LEN)`),
/// captured from inside `AccountView::load_mut` on the compiled SBF
/// artifact where the log lives in the reserved VM heap.
#[test]
fn wrapper_get_mut_write_lands_in_the_on_chain_touch_map() {
    let program_id = Pubkey::new_unique();
    let Some(mut svm) = harness(&program_id) else {
        return;
    };

    let authority = Pubkey::new_unique();
    let vault = Pubkey::new_unique();

    svm.capture_logs();
    let result = svm.process(
        // `[disc = 5]` — bump_whole_vault takes no args.
        &Instruction::new_with_bytes(
            program_id,
            &[5u8],
            vec![
                AccountMeta::new_readonly(authority, true),
                AccountMeta::new(vault, false),
            ],
        ),
        &[
            (
                authority,
                Account::new(1_000_000_000, 0, &Pubkey::default()),
            ),
            (vault, seeded_vault(&program_id, &authority)),
        ],
    );
    let logs = svm.logs();
    assert!(
        result.succeeded(),
        "bump_whole_vault must succeed; logs: {logs:#?}"
    );

    // The counter really bumped (the write the map describes).
    let count_off = Vault::DEPOSIT_COUNT_ABS_OFFSET as usize;
    let post_count = u32::from_le_bytes(
        result.account(1).data[count_off..count_off + 4]
            .try_into()
            .unwrap(),
    );
    assert_eq!(post_count, 1, "counter bump must persist");

    // Exactly one decodable map, whose single record is the
    // whole-account write the wrapper borrow performed.
    let maps = extract_touch_maps(&logs);
    assert_eq!(
        maps.len(),
        1,
        "the opted-in wrapper write must emit exactly one touch map; logs: {logs:#?}"
    );
    let (_, flags, records) = &maps[0];
    assert_eq!(*flags, 0, "map must be complete");
    assert_eq!(
        *records,
        vec![TouchMapRecord {
            slot: 1,
            offset: 0,
            size: Vault::LEN as u32,
            write: true,
        }],
        "a get_mut borrow must appear as one full-account write record"
    );
}

/// Err path: the SAME handler touches the balance segment first and
/// THEN fails (insufficient balance), so a `Drop`-based emit would have
/// fired here — the dispatcher's Ok-only routing must emit nothing for
/// the failed, rolled-back instruction.
#[test]
fn err_withdraw_emits_no_touch_map() {
    let program_id = Pubkey::new_unique();
    let Some(mut svm) = harness(&program_id) else {
        return;
    };

    let authority = Pubkey::new_unique();
    let vault = Pubkey::new_unique();

    svm.capture_logs();
    let result = svm.process(
        // More than the seeded state balance: the handler acquires the
        // balance segment lease (a real touch), reads it, and returns
        // `InsufficientBalance` (custom error 6001 = 0x1771).
        &withdraw_instruction(program_id, authority, vault, VAULT_STATE_BALANCE + 1),
        &[
            (
                authority,
                Account::new(1_000_000_000, 0, &Pubkey::default()),
            ),
            (vault, seeded_vault(&program_id, &authority)),
        ],
    );
    let logs = svm.logs();

    assert!(!result.succeeded(), "over-withdraw must fail");
    assert!(
        logs.iter()
            .any(|l| l.contains("custom program error: 0x1771")),
        "failure must be InsufficientBalance (6001); logs: {logs:#?}"
    );
    // The stream is a real execution trace (invoke + failure lines)...
    assert!(
        logs.iter().any(|l| l.contains("invoke [1]")),
        "log capture must have recorded the execution; logs: {logs:#?}"
    );
    // ...and carries ZERO decodable touch maps: the dispatcher only
    // emits after the handler returns Ok.
    assert!(
        extract_touch_maps(&logs).is_empty(),
        "a failed instruction must not advertise a touch map; logs: {logs:#?}"
    );
}
