//! Lazy migration at bind, proven on the compiled SBF artifact.
//!
//! `TouchNote` declares `migrate(from = NoteV1, with = note_v1_to_v2)`,
//! so binding against a version-1 note upgrades it IN PLACE — typed
//! transform, header re-stamped v1→v2 — before the handler runs. This
//! test drives the real `hopper_smoke.so` in Mollusk:
//!
//! - instruction 6 creates a NoteV1 (the old shape, on purpose);
//! - the FIRST instruction-7 touch migrates it (post-state header says
//!   version 2, the tag is widened, `touches` = 1) — the migration
//!   crank working on the compiled artifact, sha256 layout checks and
//!   all;
//! - the SECOND touch binds the now-V2 note without re-migrating
//!   (`touches` = 2, tag unchanged) — idempotence;
//! - both touches print their measured CU, so the one-time migration
//!   premium is a number, not a guess.
//!
//! Run `cargo build-sbf` in this example's directory first; the test
//! skips (with a notice) when the SBF artifact has not been built.

use hopper::layout::HEADER_LEN;
use hopper_smoke::{NoteV1, NoteV2};
use hopper_test::LiteSvmHarness;
use solana_account::Account;
use solana_instruction::{AccountMeta, Instruction};
use solana_pubkey::Pubkey;

const ELF_PATH_STEM: &str = "../../target/deploy/hopper_smoke";

/// `#[instruction(6)]` / `#[instruction(7)]` in `smoke_program`.
const INIT_NOTE_DISC: u8 = 6;
const TOUCH_NOTE_DISC: u8 = 7;

const SEEDED_TAG: u32 = 0xBEEF;

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

/// `[disc = 6][tag u32 LE]`, accounts `[payer(sw), note(sw), system]`.
fn init_note_instruction(program_id: Pubkey, payer: Pubkey, note: Pubkey) -> Instruction {
    let mut data = Vec::with_capacity(5);
    data.push(INIT_NOTE_DISC);
    data.extend_from_slice(&SEEDED_TAG.to_le_bytes());
    Instruction::new_with_bytes(
        program_id,
        &data,
        vec![
            AccountMeta::new(payer, true),
            AccountMeta::new(note, true),
            AccountMeta::new_readonly(
                solana_pubkey::pubkey!("11111111111111111111111111111111"),
                false,
            ),
        ],
    )
}

/// `[disc = 7]`, accounts `[authority(s), note(w)]`.
fn touch_note_instruction(program_id: Pubkey, authority: Pubkey, note: Pubkey) -> Instruction {
    Instruction::new_with_bytes(
        program_id,
        &[TOUCH_NOTE_DISC],
        vec![
            AccountMeta::new_readonly(authority, true),
            AccountMeta::new(note, false),
        ],
    )
}

#[test]
fn v1_note_migrates_on_first_touch_and_stays_v2_after() {
    let program_id = Pubkey::new_from_array([9u8; 32]);
    let Some(mut svm) = harness(&program_id) else {
        return;
    };
    let payer = Pubkey::new_unique();
    let note = Pubkey::new_unique();

    // ── Create the V1 note via the real init lifecycle ─────────────
    svm.capture_logs();
    let created = svm.process(
        &init_note_instruction(program_id, payer, note),
        &[
            (payer, Account::new(1_000_000_000, 0, &Pubkey::default())),
            (note, Account::new(0, 0, &Pubkey::default())),
            LiteSvmHarness::system_program_account(),
        ],
    );
    assert!(
        created.succeeded(),
        "init_note must succeed; logs: {:#?}",
        svm.logs()
    );
    let v1 = created.account(1).clone();
    assert_eq!(v1.data[1], 1, "created note header must say version 1");
    assert_eq!(
        &v1.data[HEADER_LEN..HEADER_LEN + 4],
        &SEEDED_TAG.to_le_bytes(),
        "V1 narrow tag seeded"
    );
    assert_eq!(v1.data.len(), NoteV1::LEN, "V1 allocation");
    assert_eq!(
        NoteV1::LEN,
        NoteV2::LEN,
        "the reserved-padding pattern: both versions fit one allocation"
    );

    // ── First touch: the migration crank fires ─────────────────────
    svm.capture_logs();
    let migrated = svm.process(
        &touch_note_instruction(program_id, payer, note),
        &[
            (payer, Account::new(1_000_000_000, 0, &Pubkey::default())),
            (note, v1),
        ],
    );
    assert!(
        migrated.succeeded(),
        "migrating touch must succeed; logs: {:#?}",
        svm.logs()
    );
    let v2 = migrated.account(1).clone();
    assert_eq!(v2.data[1], 2, "header re-stamped to version 2");
    let tag = u64::from_le_bytes(v2.data[HEADER_LEN..HEADER_LEN + 8].try_into().unwrap());
    assert_eq!(
        tag, SEEDED_TAG as u64,
        "tag widened u32 -> u64 by the transform"
    );
    let touches = u32::from_le_bytes(v2.data[HEADER_LEN + 8..HEADER_LEN + 12].try_into().unwrap());
    assert_eq!(touches, 1, "the handler ran AFTER the migration");
    let migrating_cu = migrated.compute_units();

    // ── Second touch: already V2, no re-migration ──────────────────
    svm.capture_logs();
    let touched = svm.process(
        &touch_note_instruction(program_id, payer, note),
        &[
            (payer, Account::new(1_000_000_000, 0, &Pubkey::default())),
            (note, v2),
        ],
    );
    assert!(
        touched.succeeded(),
        "second touch must succeed; logs: {:#?}",
        svm.logs()
    );
    let after = touched.account(1);
    assert_eq!(after.data[1], 2, "still version 2");
    let tag2 = u64::from_le_bytes(after.data[HEADER_LEN..HEADER_LEN + 8].try_into().unwrap());
    assert_eq!(tag2, SEEDED_TAG as u64, "tag untouched on repeat binds");
    let touches2 = u32::from_le_bytes(
        after.data[HEADER_LEN + 8..HEADER_LEN + 12]
            .try_into()
            .unwrap(),
    );
    assert_eq!(touches2, 2, "repeat binds only run the handler");
    let steady_cu = touched.compute_units();

    eprintln!(
        "MEASURED touch_note: migrating touch {migrating_cu} CU, steady-state touch \
         {steady_cu} CU (one-time migration premium: {} CU)",
        migrating_cu - steady_cu
    );
}

/// A foreign account (the vault layout, disc 7) in the note slot must
/// fail with the normal validation error — the migrate probe is a full
/// V1 identity check, never a sniff that could misfire on other kinds.
#[test]
fn foreign_layout_in_the_note_slot_fails_bind() {
    let program_id = Pubkey::new_from_array([9u8; 32]);
    let Some(mut svm) = harness(&program_id) else {
        return;
    };
    let payer = Pubkey::new_unique();
    let intruder = Pubkey::new_unique();
    // A program-owned account of the right SIZE but wrong identity
    // (zeroed header: disc 0, version 0 — matches neither V1 nor V2).
    let blank = Account {
        lamports: 10_000_000,
        data: vec![0u8; NoteV2::LEN],
        owner: program_id,
        executable: false,
        rent_epoch: 0,
    };

    svm.capture_logs();
    let result = svm.process(
        &touch_note_instruction(program_id, payer, intruder),
        &[
            (payer, Account::new(1_000_000_000, 0, &Pubkey::default())),
            (intruder, blank.clone()),
        ],
    );
    assert!(
        !result.succeeded(),
        "a non-note account must fail bind; logs: {:#?}",
        svm.logs()
    );
    assert_eq!(
        result.account(1).data,
        blank.data,
        "refused account must not be written"
    );
}
