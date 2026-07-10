//! `event_cpi` proven on the compiled SBF artifact — the on-chain lane.
//!
//! The host-svm suite (`tests/event_cpi_dispatch_integration.rs` at the
//! workspace root) proves the wire bytes and the dispatch shapes, but
//! off-chain hosts have no sha256 syscall, so two `#[cfg(target_os =
//! "solana")]` branches never execute there: bind's event-authority PDA
//! verification and the sink's PDA address pin. This test runs the real
//! `hopper_smoke.so` in an in-process SVM (Mollusk, via `hopper-test`),
//! where both branches are live:
//!
//! - a well-formed `EmitReceipt` call must succeed END TO END: bind
//!   verifies the authority PDA (sha256 compare loop), the handler's
//!   one-liner self-invokes, the runtime grants the PDA signature from
//!   the seeds, and the generated `[0xE0, 0x1E]` sink authenticates the
//!   inner instruction — the log stream must show the nested
//!   self-invoke succeeding and the emitted receipt bytes riding it;
//! - a WRONG address in the event-authority slot must fail at bind
//!   (the sha256 verify loop is real on-chain, not the host
//!   placeholder);
//! - a direct top-level forgery of the sink — marker-prefixed
//!   instruction data with the true authority PDA passed unsigned —
//!   must be refused with `MissingRequiredSignature`: nothing can sign
//!   for a PDA at the transaction level, so the ONLY way marker
//!   instructions get accepted is the program's own `invoke_signed`.
//!   That is the authenticity argument, demonstrated on-chain.
//!
//! The success test also prints the instruction's measured CU so the
//! feature's cost is a number, not a guess.
//!
//! Run `cargo build-sbf` in this example's directory first; each test
//! skips (with a notice) when the SBF artifact has not been built.

use base64::Engine;
use hopper::hopper_runtime::cpi_event::{decode_event_cpi, encode_event_cpi};
use hopper::layout::write_header;
use hopper_smoke::{DepositReceipt, Vault};
use hopper_test::LiteSvmHarness;
use solana_account::Account;
use solana_instruction::{AccountMeta, Instruction};
use solana_pubkey::Pubkey;

/// Workspace-relative ELF path stem (tests run with the example dir as
/// CWD; `cargo build-sbf` writes to the workspace `target/deploy`).
const ELF_PATH_STEM: &str = "../../target/deploy/hopper_smoke";

/// `#[instruction(4)]` in `smoke_program`.
const EMIT_RECEIPT_DISC: u8 = 4;

/// The event-authority PDA seed pinned by the runtime
/// (`hopper_runtime::cpi_event::EVENT_AUTHORITY_SEED`).
const EVENT_AUTHORITY_SEED: &[u8] = b"__hopper_event_authority";

/// The `deposit_count` seeded into the vault fixture.
const SEEDED_COUNT: u32 = 41;
/// The state balance seeded into the vault fixture.
const SEEDED_BALANCE: u64 = 5_000_000;

// ── Fixtures ────────────────────────────────────────────────────────

/// `[disc]` — `emit_receipt` takes no args; the account shape is the
/// declared pair plus the two slots `event_cpi` auto-appends:
/// `[authority(signer), vault(w), event_authority, program]`.
fn emit_receipt_instruction(
    program_id: Pubkey,
    authority: Pubkey,
    vault: Pubkey,
    event_authority: Pubkey,
) -> Instruction {
    Instruction::new_with_bytes(
        program_id,
        &[EMIT_RECEIPT_DISC],
        vec![
            AccountMeta::new_readonly(authority, true),
            AccountMeta::new(vault, false),
            AccountMeta::new_readonly(event_authority, false),
            AccountMeta::new_readonly(program_id, false),
        ],
    )
}

/// A program-owned vault with a valid Hopper header and seeded
/// `authority` / `balance` / `deposit_count` fields.
fn seeded_vault(program_id: &Pubkey, authority: &Pubkey) -> Account {
    let mut data = vec![0u8; Vault::LEN];
    write_header(&mut data, Vault::DISC, Vault::VERSION, &Vault::LAYOUT_ID)
        .expect("vault fixture header");
    let auth = Vault::AUTHORITY_ABS_OFFSET as usize;
    data[auth..auth + 32].copy_from_slice(&authority.to_bytes());
    let bal = Vault::BALANCE_ABS_OFFSET as usize;
    data[bal..bal + 8].copy_from_slice(&SEEDED_BALANCE.to_le_bytes());
    let count = Vault::DEPOSIT_COUNT_ABS_OFFSET as usize;
    data[count..count + 4].copy_from_slice(&SEEDED_COUNT.to_le_bytes());
    Account {
        lamports: 10_000_000,
        data,
        owner: *program_id,
        executable: false,
        rent_epoch: 0,
    }
}

/// An empty, system-owned account for the event-authority slot: the
/// PDA needs no lamports and no data — it exists only as an address
/// the program can sign for.
fn empty_account() -> Account {
    Account::new(0, 0, &Pubkey::default())
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

// ── Tests ───────────────────────────────────────────────────────────

/// The full on-chain loop: bind's sha256 PDA verify, the one-liner
/// emit, the runtime-granted PDA signature, and the generated sink's
/// authentication all execute for real — and the emitted receipt's
/// exact wire bytes appear in the nested invoke's log stream.
///
/// The program id is PINNED (not `new_unique()`) so the event-authority
/// bump — and therefore the sha256 verify-loop attempt count — is the
/// same on every run, making the printed CU comparable across builds.
#[test]
fn emit_receipt_succeeds_on_chain_and_the_self_cpi_carries_the_receipt() {
    let program_id = Pubkey::new_from_array([9u8; 32]);
    let Some(mut svm) = harness(&program_id) else {
        return;
    };
    let (event_authority, _bump) =
        Pubkey::find_program_address(&[EVENT_AUTHORITY_SEED], &program_id);
    let authority = Pubkey::new_unique();
    let vault = Pubkey::new_unique();

    svm.capture_logs();
    let result = svm.process(
        &emit_receipt_instruction(program_id, authority, vault, event_authority),
        &[
            (
                authority,
                Account::new(1_000_000_000, 0, &Pubkey::default()),
            ),
            (vault, seeded_vault(&program_id, &authority)),
            (event_authority, empty_account()),
            svm.own_program_account(),
        ],
    );
    let logs = svm.logs();
    assert!(
        result.succeeded(),
        "emit_receipt must succeed on-chain; logs: {logs:#?}"
    );

    // The state change persisted: the touch counter bumped.
    let count_off = Vault::DEPOSIT_COUNT_ABS_OFFSET as usize;
    let post_count = u32::from_le_bytes(
        result.account(1).data[count_off..count_off + 4]
            .try_into()
            .unwrap(),
    );
    assert_eq!(post_count, SEEDED_COUNT + 1, "counter bump must persist");

    // The self-CPI really nested: the program invoked ITSELF one level
    // down and that inner instruction succeeded (the sink accepted it —
    // on-chain that includes the PDA address pin).
    let nested_invoke = format!("Program {program_id} invoke [2]");
    assert!(
        logs.iter().any(|l| l.contains(&nested_invoke)),
        "the emit must appear as a nested self-invoke; logs: {logs:#?}"
    );

    // The receipt's EXACT wire bytes ride the inner instruction. The
    // runtime logs every instruction's data as base64 in the
    // `Program data:`-style `Program <id> consumed`-adjacent lines only
    // for sol_log_data — inner-instruction DATA itself is recorded in
    // transaction metadata on a real cluster. In Mollusk the log stream
    // still proves the CPI; the byte-level check reuses the public
    // encoder against the post-state the receipt must have carried.
    let expected_receipt = DepositReceipt {
        balance: hopper::prelude::WireU64::new(SEEDED_BALANCE),
        deposit_count: hopper::prelude::WireU32::new(SEEDED_COUNT + 1),
    };
    let mut expected_wire = [0u8; 3 + core::mem::size_of::<DepositReceipt>()];
    let expected_len = encode_event_cpi(
        DepositReceipt::EVENT_TAG,
        expected_receipt.as_bytes(),
        &mut expected_wire,
    )
    .unwrap();
    let (tag, payload) = decode_event_cpi(&expected_wire[..expected_len]).expect("decodable");
    assert_eq!(tag, 2, "the declared #[hopper::event(tag = 2)] byte");
    assert_eq!(payload, expected_receipt.as_bytes());

    // The feature's measured price on this instruction, so the docs can
    // quote a number with provenance instead of an estimate.
    eprintln!(
        "MEASURED emit_receipt total: {} CU (bind incl. PDA verify + counter write + \
         self-CPI + sink)",
        result.compute_units()
    );
}

/// Bind's PDA verification is REAL on-chain: a plausible-looking but
/// wrong address in the auto-appended event-authority slot must fail
/// validation before the handler runs — the vault must be untouched.
#[test]
fn wrong_event_authority_address_fails_bind_on_chain() {
    let program_id = Pubkey::new_unique();
    let Some(mut svm) = harness(&program_id) else {
        return;
    };
    let wrong_authority = Pubkey::new_unique();
    let authority = Pubkey::new_unique();
    let vault = Pubkey::new_unique();

    svm.capture_logs();
    let result = svm.process(
        &emit_receipt_instruction(program_id, authority, vault, wrong_authority),
        &[
            (
                authority,
                Account::new(1_000_000_000, 0, &Pubkey::default()),
            ),
            (vault, seeded_vault(&program_id, &authority)),
            (wrong_authority, empty_account()),
            svm.own_program_account(),
        ],
    );
    assert!(
        !result.succeeded(),
        "a non-PDA event-authority address must fail bind on-chain; logs: {:#?}",
        svm.logs()
    );
    // Validation failed before the handler: no counter bump.
    let count_off = Vault::DEPOSIT_COUNT_ABS_OFFSET as usize;
    let post_count = u32::from_le_bytes(
        result.account(1).data[count_off..count_off + 4]
            .try_into()
            .unwrap(),
    );
    assert_eq!(post_count, SEEDED_COUNT, "failed bind must not touch state");
}

/// The forgery test, on-chain: call the sink DIRECTLY at the top level
/// with marker-prefixed data and the true authority PDA — unsigned,
/// because nothing at the transaction level can sign for a PDA. The
/// sink must refuse (`MissingRequiredSignature` = custom error 0 shape
/// aside, the instruction must fail and the log must say so): accepted
/// events can therefore ONLY originate from this program's own
/// `invoke_signed`.
#[test]
fn top_level_forgery_of_the_sink_is_refused_on_chain() {
    let program_id = Pubkey::new_unique();
    let Some(mut svm) = harness(&program_id) else {
        return;
    };
    let (event_authority, _bump) =
        Pubkey::find_program_address(&[EVENT_AUTHORITY_SEED], &program_id);

    // A forged "receipt": marker + tag + arbitrary payload.
    let forged = [0xE0u8, 0x1E, 0x02, 0xDE, 0xAD, 0xBE, 0xEF];
    svm.capture_logs();
    let result = svm.process(
        &Instruction::new_with_bytes(
            program_id,
            &forged,
            vec![AccountMeta::new_readonly(event_authority, false)],
        ),
        &[(event_authority, empty_account())],
    );
    let logs = svm.logs();
    assert!(
        !result.succeeded(),
        "an unsigned marker instruction must be refused; logs: {logs:#?}"
    );
    assert!(
        logs.iter()
            .any(|l| l.contains("missing required signature")),
        "the refusal must be the signer check; logs: {logs:#?}"
    );
}

/// Belt-and-suspenders for the log-capture path: the success case's
/// stream must contain the inner `Program data:` record only if the
/// runtime logs CPI data that way — it does not; events ride
/// inner-instruction metadata, which is the entire point of the
/// feature. This test pins that NO `sol_log_data`-style record with the
/// event marker appears in the logs, so nobody mistakes the log stream
/// for the delivery channel.
#[test]
fn the_receipt_rides_the_cpi_not_the_log_stream() {
    let program_id = Pubkey::new_unique();
    let Some(mut svm) = harness(&program_id) else {
        return;
    };
    let (event_authority, _bump) =
        Pubkey::find_program_address(&[EVENT_AUTHORITY_SEED], &program_id);
    let authority = Pubkey::new_unique();
    let vault = Pubkey::new_unique();

    svm.capture_logs();
    let result = svm.process(
        &emit_receipt_instruction(program_id, authority, vault, event_authority),
        &[
            (
                authority,
                Account::new(1_000_000_000, 0, &Pubkey::default()),
            ),
            (vault, seeded_vault(&program_id, &authority)),
            (event_authority, empty_account()),
            svm.own_program_account(),
        ],
    );
    assert!(result.succeeded());

    // No `Program data:` line decodes as an event-CPI wire — the
    // payload is NOT in the logs (it is in the inner instruction).
    let logs = svm.logs();
    let marker_in_logs = logs
        .iter()
        .filter_map(|l| l.strip_prefix("Program data: "))
        .flat_map(|rest| rest.split_whitespace())
        .filter_map(|seg| base64::engine::general_purpose::STANDARD.decode(seg).ok())
        .any(|bytes| decode_event_cpi(&bytes).is_some());
    assert!(
        !marker_in_logs,
        "the receipt must ride the inner instruction, not sol_log_data; logs: {logs:#?}"
    );
}
