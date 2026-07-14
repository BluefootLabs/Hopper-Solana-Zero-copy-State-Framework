//! THE NEW EVIDENCE — the strict-writes REFUSAL proven on the COMPILED SBF
//! artifact.
//!
//! The host suite (`tests/flagship_host.rs`) proves the refusal with borrow
//! tracking on the host. This test runs the REAL `hopper_sentinel.so` in an
//! in-process SVM (Mollusk, via `hopper-test`) — real bytecode, real compute
//! metering, real transaction result — and proves both flagship paths:
//!
//! - `honest_pause` (instruction 1) SUCCEEDS: `paused = 1`, `revision = 1`,
//!   `admin` untouched, and the emitted touch map (decoded from the log
//!   stream, exactly as `hopper tx explain` would) carries exactly the two
//!   declared writes — `paused` then `revision`.
//! - `malicious_pause` (instruction 2) — the SAME context, the SAME honest
//!   body, then a tampered admin rotation THROUGH THE CONTEXT — FAILS with the
//!   custom error `0xd001` (`Custom(0xD000 | 1)`) decoded from the transaction
//!   result, and the admin bytes are byte-for-byte unchanged.
//!
//! Both paths print their measured CU, so the guarantee — and its cost — are
//! numbers, not claims.
//!
//! Run `cargo build-sbf` in this example's directory first; each test skips
//! (with a notice) when the SBF artifact has not been built.

use base64::Engine;
use hopper::hopper_runtime::segment_borrow::{
    TouchMapRecord, TOUCH_MAP_HEADER_LEN, TOUCH_MAP_MAGIC, TOUCH_MAP_RECORD_LEN, TOUCH_MAP_VERSION,
};
use hopper::layout::write_header;
use hopper_sentinel::Config;
use hopper_test::LiteSvmHarness;
use solana_account::Account;
use solana_instruction::{AccountMeta, Instruction};
use solana_pubkey::Pubkey;

/// Workspace-relative ELF path stem (tests run with the example dir as CWD;
/// `cargo build-sbf` writes to the workspace `target/deploy`).
const ELF_PATH_STEM: &str = "../../target/deploy/hopper_sentinel";

/// `#[instruction(1)]` / `#[instruction(2)]` in `sentinel_program`.
const HONEST_PAUSE_DISC: u8 = 1;
const MALICIOUS_PAUSE_DISC: u8 = 2;

// ── Fixtures ────────────────────────────────────────────────────────

/// A program-owned config with a valid Hopper header and `admin` +
/// `withdraw_authority` seeded to `admin`.
fn seeded_config(program_id: &Pubkey, admin: &Pubkey) -> Account {
    let mut data = vec![0u8; Config::LEN];
    write_header(&mut data, Config::DISC, Config::VERSION, &Config::LAYOUT_ID)
        .expect("config fixture header");
    let a = Config::ADMIN_ABS_OFFSET as usize;
    data[a..a + 32].copy_from_slice(&admin.to_bytes());
    let w = Config::WITHDRAW_AUTHORITY_ABS_OFFSET as usize;
    data[w..w + 32].copy_from_slice(&admin.to_bytes());
    Account {
        lamports: 5_000_000,
        data,
        owner: *program_id,
        executable: false,
        rent_epoch: 0,
    }
}

/// `[disc]`, accounts `[admin(signer, ro), config(writable)]`.
fn pause_instruction(program_id: Pubkey, admin: Pubkey, config: Pubkey, disc: u8) -> Instruction {
    Instruction::new_with_bytes(
        program_id,
        &[disc],
        vec![
            AccountMeta::new_readonly(admin, true),
            AccountMeta::new(config, false),
        ],
    )
}

/// Load the harness, or `None` (skip) when the SBF artifact is absent.
fn harness(program_id: &Pubkey) -> Option<LiteSvmHarness> {
    let svm = LiteSvmHarness::load(program_id, ELF_PATH_STEM);
    if svm.is_none() {
        eprintln!(
            "SKIPPED: {ELF_PATH_STEM}.so not found - run `cargo build-sbf` in \
             examples/hopper-sentinel first"
        );
    }
    svm
}

// ── The `hopper tx explain` touch-map decode, replicated 1:1 ────────

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

/// Scan a captured log stream for decodable touch maps, exactly as
/// `hopper tx explain` scans a transaction's `logMessages`.
fn extract_touch_maps(log_lines: &[String]) -> Vec<(u8, Vec<TouchMapRecord>)> {
    let mut out = Vec::new();
    for line in log_lines {
        let Some(rest) = line.strip_prefix("Program data: ") else {
            continue;
        };
        for seg in rest.split_whitespace() {
            let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(seg) else {
                continue;
            };
            if let Some(decoded) = decode_touch_map(&bytes) {
                out.push(decoded);
            }
        }
    }
    out
}

fn admin_bytes(account: &Account) -> &[u8] {
    let a = Config::ADMIN_ABS_OFFSET as usize;
    &account.data[a..a + 32]
}

// ── Honest pause: succeeds, self-describes, admin untouched ─────────

#[test]
fn honest_pause_succeeds_on_chain_and_emits_the_declared_touch_map() {
    let program_id = Pubkey::new_unique();
    let Some(mut svm) = harness(&program_id) else {
        return;
    };
    let admin = Pubkey::new_unique();
    let config = Pubkey::new_unique();

    svm.capture_logs();
    let result = svm.process(
        &pause_instruction(program_id, admin, config, HONEST_PAUSE_DISC),
        &[
            (admin, Account::new(1_000_000_000, 0, &Pubkey::default())),
            (config, seeded_config(&program_id, &admin)),
        ],
    );
    let logs = svm.logs();
    assert!(
        result.succeeded(),
        "honest pause must succeed; logs: {logs:#?}"
    );

    // State: paused set, revision bumped, admin untouched.
    let cfg = result.account(1);
    assert_eq!(
        cfg.data[Config::PAUSED_ABS_OFFSET as usize],
        1,
        "paused must be set"
    );
    let ro = Config::REVISION_ABS_OFFSET as usize;
    assert_eq!(
        u64::from_le_bytes(cfg.data[ro..ro + 8].try_into().unwrap()),
        1,
        "revision must be bumped to 1"
    );
    assert_eq!(admin_bytes(cfg), admin.to_bytes(), "admin untouched");

    // Touch map from the real log stream: exactly the two declared writes.
    let maps = extract_touch_maps(&logs);
    assert_eq!(
        maps.len(),
        1,
        "a successful opted-in pause emits exactly one touch map; logs: {logs:#?}"
    );
    let (flags, records) = &maps[0];
    assert_eq!(*flags, 0, "map must be complete (no overflow/skip flags)");
    let writes: Vec<TouchMapRecord> = records.iter().filter(|r| r.write).cloned().collect();
    assert_eq!(
        writes,
        vec![
            TouchMapRecord {
                slot: 1,
                offset: Config::PAUSED_ABS_OFFSET,
                size: 1,
                write: true,
            },
            TouchMapRecord {
                slot: 1,
                offset: Config::REVISION_ABS_OFFSET,
                size: 8,
                write: true,
            },
        ],
        "the on-chain touch map must describe exactly the declared paused + revision writes"
    );

    eprintln!(
        "MEASURED honest_pause: {} CU (SUCCEEDS; touch map = paused + revision)",
        result.compute_units()
    );
}

// ── Malicious pause: refused on chain, admin byte-unchanged ─────────

#[test]
fn malicious_pause_is_refused_on_chain_with_0xd001_and_admin_is_unchanged() {
    let program_id = Pubkey::new_unique();
    let Some(mut svm) = harness(&program_id) else {
        return;
    };
    let admin = Pubkey::new_unique();
    let config = Pubkey::new_unique();

    svm.capture_logs();
    let result = svm.process(
        &pause_instruction(program_id, admin, config, MALICIOUS_PAUSE_DISC),
        &[
            (admin, Account::new(1_000_000_000, 0, &Pubkey::default())),
            (config, seeded_config(&program_id, &admin)),
        ],
    );
    let logs = svm.logs();

    // The framework refused the tampered admin rotation.
    assert!(
        !result.succeeded(),
        "malicious pause must FAIL; logs: {logs:#?}"
    );
    // Custom(0xD000 | 1) == 0xD001, decoded from the transaction result.
    assert!(
        logs.iter()
            .any(|l| l.contains("custom program error: 0xd001")),
        "failure must be the indexed write-policy violation 0xd001; logs: {logs:#?}"
    );
    // The refusal fired at mutable-borrow acquisition: no touch map is emitted
    // (Ok-path only), and the whole instruction rolls back.
    assert!(
        extract_touch_maps(&logs).is_empty(),
        "a refused instruction must not advertise a touch map; logs: {logs:#?}"
    );
    // The load-bearing claim: admin is byte-for-byte unchanged.
    let cfg = result.account(1);
    assert_eq!(
        admin_bytes(cfg),
        admin.to_bytes(),
        "admin must be byte-unchanged after the refused rotation"
    );

    eprintln!(
        "MEASURED malicious_pause: {} CU (REFUSED with Custom(0xD001); admin unchanged)",
        result.compute_units()
    );
}
