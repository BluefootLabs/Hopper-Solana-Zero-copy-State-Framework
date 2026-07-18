//! Verdict-algorithm units on synthetic contracts and snapshots: PASS, each
//! violation shape, the acquired-but-unchanged note, and the two
//! INCONCLUSIVE gates (no byte contract / partial map).

use grillo_manifest::{
    ArgContract, ArgEncodingContract, InstructionContract, ParametricRangeContract, RangeContract,
};
use grillo_verifier::{
    verify, verify_invocation, AccountDelta, InconclusiveReason, TouchMap, TouchRecord, Verdict,
    Violation,
};

// ── builders ────────────────────────────────────────────────────────────

fn contract(
    strict_writes: bool,
    mutation_complete: bool,
    authorized: Vec<RangeContract>,
    lamport_accounts: Vec<u8>,
) -> InstructionContract {
    InstructionContract {
        name: "ix".to_string(),
        tag: 0,
        strict_writes,
        mutation_complete,
        authorized,
        parametric: vec![],
        lamport_accounts,
        args: vec![],
        accounts: vec![],
    }
}

fn rng(account_index: u8, offset: u32, size: u32) -> RangeContract {
    RangeContract {
        account_index,
        offset,
        size,
    }
}

fn write(slot: u8, offset: u32, size: u32) -> TouchRecord {
    TouchRecord {
        slot,
        offset,
        size,
        write: true,
    }
}

fn read(slot: u8, offset: u32, size: u32) -> TouchRecord {
    TouchRecord {
        slot,
        offset,
        size,
        write: false,
    }
}

fn complete_map(records: Vec<TouchRecord>) -> TouchMap {
    TouchMap {
        overflowed: false,
        skipped: false,
        records,
    }
}

// ── PASS ────────────────────────────────────────────────────────────────

#[test]
fn pass_when_changed_within_acquired_within_authorized() {
    let c = contract(true, false, vec![rng(1, 114, 1), rng(1, 115, 8)], vec![]);
    let pre = vec![0u8; 200];
    let mut post = pre.clone();
    post[114] = 1; // paused
    post[115] = 1; // revision low byte
    let map = complete_map(vec![write(1, 114, 1), write(1, 115, 8)]);

    match verify(&c, &[AccountDelta::new(1, &pre, &post)], &map) {
        Verdict::Pass(ev) => {
            assert_eq!(ev.observed_data_accounts, vec![1]);
            assert!(ev.observed_lamport_accounts.is_empty());
            assert_eq!(ev.changed_bytes, 2);
            assert_eq!(ev.changed, vec![rng(1, 114, 2)]);
            assert!(ev.acquired_unchanged.is_empty());
        }
        other => panic!("expected PASS, got:\n{}", other.render()),
    }
}

#[test]
fn acquired_but_unchanged_is_a_pass_note_not_a_violation() {
    // The instruction acquired a WRITE lease on [0..8) but changed nothing —
    // access is not modification.
    let c = contract(true, false, vec![rng(1, 0, 8)], vec![]);
    let pre = vec![7u8; 8];
    let post = pre.clone(); // unchanged
    let map = complete_map(vec![write(1, 0, 8)]);

    match verify(&c, &[AccountDelta::new(1, &pre, &post)], &map) {
        Verdict::Pass(ev) => {
            assert_eq!(ev.changed_bytes, 0);
            assert!(ev.changed.is_empty());
            assert_eq!(ev.acquired_unchanged, vec![rng(1, 0, 8)]);
        }
        other => panic!("expected PASS, got:\n{}", other.render()),
    }
}

#[test]
fn reads_outside_authorized_are_not_violations() {
    // A READ record far outside the authorized set is access, not
    // modification, so it must not trigger UnauthorizedAcquisition.
    let c = contract(true, false, vec![rng(1, 0, 4)], vec![]);
    let pre = vec![0u8; 8];
    let mut post = pre.clone();
    post[0] = 1;
    let map = complete_map(vec![read(1, 1000, 50), write(1, 0, 4)]);
    assert!(verify(&c, &[AccountDelta::new(1, &pre, &post)], &map).is_pass());
}

#[test]
fn whole_account_authorized_range_covers_any_write_without_overflow() {
    // authorized = (offset 0, size u32::MAX) is the whole-account sentinel
    // the manifest emits; a write anywhere inside must pass.
    let c = contract(true, false, vec![rng(0, 0, u32::MAX)], vec![]);
    let pre = vec![0u8; 64];
    let mut post = pre.clone();
    post[48] = 9;
    let map = complete_map(vec![write(0, 48, 8)]);
    assert!(verify(&c, &[AccountDelta::new(0, &pre, &post)], &map).is_pass());
}

// ── VIOLATION: changed ⊄ acquired ─────────────────────────────────────────

#[test]
fn untracked_write_when_a_changed_byte_is_not_acquired() {
    // The touch map omits the revision write, but revision's low byte
    // changed — the changed byte at 115 is covered by no WRITE record.
    let c = contract(true, false, vec![rng(1, 114, 1), rng(1, 115, 8)], vec![]);
    let pre = vec![0u8; 200];
    let mut post = pre.clone();
    post[114] = 1;
    post[115] = 1;
    let map = complete_map(vec![write(1, 114, 1)]); // revision write MISSING

    match verify(&c, &[AccountDelta::new(1, &pre, &post)], &map) {
        Verdict::Violation(v) => {
            assert_eq!(
                v,
                vec![Violation::UntrackedWrite {
                    account_index: 1,
                    offset: 115
                }]
            );
        }
        other => panic!("expected VIOLATION, got:\n{}", other.render()),
    }
}

// ── VIOLATION: acquired ⊄ authorized ──────────────────────────────────────

#[test]
fn unauthorized_acquisition_when_a_write_record_escapes_the_authorized_set() {
    // A forged admin write record (16, 32) is acquired but not authorized.
    let c = contract(true, false, vec![rng(1, 114, 1), rng(1, 115, 8)], vec![]);
    let pre = vec![0u8; 200];
    let mut post = pre.clone();
    post[114] = 1;
    post[115] = 1;
    let map = complete_map(vec![write(1, 114, 1), write(1, 115, 8), write(1, 16, 32)]);

    match verify(&c, &[AccountDelta::new(1, &pre, &post)], &map) {
        Verdict::Violation(v) => {
            assert_eq!(
                v,
                vec![Violation::UnauthorizedAcquisition {
                    account_index: 1,
                    offset: 16,
                    size: 32
                }]
            );
        }
        other => panic!("expected VIOLATION, got:\n{}", other.render()),
    }
}

#[test]
fn a_write_spanning_two_adjacent_authorized_ranges_is_authorized() {
    // authorized = {(114,1),(115,8)}; a single write record (114, 9) covers
    // exactly their union — set-containment, so no violation.
    let c = contract(true, false, vec![rng(1, 114, 1), rng(1, 115, 8)], vec![]);
    let pre = vec![0u8; 200];
    let mut post = pre.clone();
    post[114] = 1;
    let map = complete_map(vec![write(1, 114, 9)]);
    assert!(
        verify(&c, &[AccountDelta::new(1, &pre, &post)], &map).is_pass(),
        "a write covering the union of adjacent authorized ranges is authorized"
    );
}

// ── VIOLATION: lamport dimension ──────────────────────────────────────────

#[test]
fn unauthorized_lamport_delta_on_a_mutation_complete_contract() {
    // config(0) declares revision; fee_sink(1) is a lamport target;
    // treasury(2) is neither — a lamport drain on it is a violation.
    let c = contract(true, true, vec![rng(0, 115, 8)], vec![1]);
    let pre_cfg = vec![0u8; 160];
    let mut post_cfg = pre_cfg.clone();
    post_cfg[115] = 1;
    let empty: &[u8] = &[];
    let map = complete_map(vec![write(0, 115, 8)]);

    let deltas = [
        AccountDelta::new(0, &pre_cfg, &post_cfg),
        AccountDelta::new(1, empty, empty).with_lamports(1_000, 2_000), // authorized credit
        AccountDelta::new(2, empty, empty).with_lamports(5_000, 0),     // UNAUTHORIZED drain
    ];
    match verify(&c, &deltas, &map) {
        Verdict::Violation(v) => {
            assert_eq!(
                v,
                vec![Violation::UnauthorizedLamportDelta {
                    account_index: 2,
                    pre_lamports: 5_000,
                    post_lamports: 0
                }]
            );
        }
        other => panic!("expected VIOLATION, got:\n{}", other.render()),
    }
}

#[test]
fn lamport_delta_is_ignored_when_not_mutation_complete() {
    // Without the lamport dimension declared, a lamport move is a
    // passthrough (undeclared), not a violation.
    let c = contract(true, false, vec![], vec![]);
    let empty: &[u8] = &[];
    let map = complete_map(vec![]);
    let deltas = [AccountDelta::new(0, empty, empty).with_lamports(1_000, 500)];
    assert!(verify(&c, &deltas, &map).is_pass());
}

#[test]
fn pass_evidence_distinguishes_observed_and_unobserved_lamports() {
    let c = contract(true, true, vec![], vec![1]);
    let empty: &[u8] = &[];
    let deltas = [
        AccountDelta::new(0, empty, empty),
        AccountDelta::new(1, empty, empty).with_lamports(0, 0),
    ];
    match verify(&c, &deltas, &complete_map(vec![])) {
        Verdict::Pass(ev) => {
            assert_eq!(ev.observed_data_accounts, vec![0, 1]);
            assert_eq!(ev.observed_lamport_accounts, vec![1]);
        }
        other => panic!("expected scoped PASS, got:\n{}", other.render()),
    }
}

// ── INCONCLUSIVE ──────────────────────────────────────────────────────────

#[test]
fn no_byte_contract_is_inconclusive() {
    let c = contract(false, false, vec![], vec![]);
    let pre = vec![0u8; 8];
    let mut post = pre.clone();
    post[0] = 1;
    let map = complete_map(vec![write(0, 0, 1)]);
    assert_eq!(
        verify(&c, &[AccountDelta::new(0, &pre, &post)], &map),
        Verdict::Inconclusive(InconclusiveReason::NoByteContract)
    );
}

#[test]
fn a_partial_map_is_inconclusive_never_a_false_pass() {
    // The map claims a forged out-of-contract write AND is flagged partial.
    // Partiality wins: byte attribution is impossible, so the verdict must
    // be INCONCLUSIVE_PARTIAL_MAP — not a VIOLATION and never a PASS.
    let c = contract(true, false, vec![rng(1, 114, 1)], vec![]);
    let pre = vec![0u8; 200];
    let mut post = pre.clone();
    post[114] = 1;

    let overflowed = TouchMap {
        overflowed: true,
        skipped: false,
        records: vec![write(1, 16, 32)], // would be unauthorized if attributable
    };
    assert_eq!(
        verify(&c, &[AccountDelta::new(1, &pre, &post)], &overflowed),
        Verdict::Inconclusive(InconclusiveReason::PartialTouchMap {
            overflowed: true,
            skipped: false
        })
    );

    let skipped = TouchMap {
        overflowed: false,
        skipped: true,
        records: vec![write(1, 114, 1)],
    };
    assert_eq!(
        verify(&c, &[AccountDelta::new(1, &pre, &post)], &skipped),
        Verdict::Inconclusive(InconclusiveReason::PartialTouchMap {
            overflowed: false,
            skipped: true
        })
    );
}

// ── render ────────────────────────────────────────────────────────────────

#[test]
fn parametric_contract_requires_wire_resolution_and_rejects_neighbor_cell() {
    let mut c = contract(true, false, vec![rng(1, 100, 10)], vec![]);
    c.args.push(ArgContract {
        name: "slot".to_string(),
        canonical_type: "u16".to_string(),
        size: 2,
        encoding: ArgEncodingContract::Fixed,
        max_len: None,
        element_size: None,
    });
    c.parametric.push(ParametricRangeContract {
        account_index: 1,
        base_offset: 100,
        stride: 1,
        cell_size: 1,
        count: 10,
        argument_index: 0,
        argument_name: "slot".to_string(),
        segment_name: "statuses".to_string(),
    });

    let pre = vec![0u8; 120];
    let mut selected_post = pre.clone();
    selected_post[103] = 1;
    assert_eq!(
        verify(
            &c,
            &[AccountDelta::new(1, &pre, &selected_post)],
            &complete_map(vec![write(1, 103, 1)]),
        ),
        Verdict::Inconclusive(InconclusiveReason::ParametricArgumentsRequired)
    );
    assert!(verify_invocation(
        &c,
        &3u16.to_le_bytes(),
        &[AccountDelta::new(1, &pre, &selected_post)],
        &complete_map(vec![write(1, 103, 1)]),
    )
    .unwrap()
    .is_pass());

    let mut neighbor_post = pre.clone();
    neighbor_post[104] = 1;
    assert!(matches!(
        verify_invocation(
            &c,
            &3u16.to_le_bytes(),
            &[AccountDelta::new(1, &pre, &neighbor_post)],
            &complete_map(vec![write(1, 104, 1)]),
        )
        .unwrap(),
        Verdict::Violation(_)
    ));
}

#[test]
fn render_reports_each_verdict_shape() {
    let c = contract(true, false, vec![rng(1, 114, 1)], vec![]);
    let pre = vec![0u8; 200];
    let mut post = pre.clone();
    post[114] = 1;

    let pass = verify(
        &c,
        &[AccountDelta::new(1, &pre, &post)],
        &complete_map(vec![write(1, 114, 1)]),
    );
    let rendered = pass.render();
    assert!(rendered.contains("SCOPED PASS"));
    assert!(rendered.contains("observed data accounts: [1]"));

    let mut bad = post.clone();
    bad[16] = 9;
    let violation = verify(
        &c,
        &[AccountDelta::new(1, &pre, &bad)],
        &complete_map(vec![write(1, 114, 1)]),
    );
    let text = violation.render();
    assert!(text.contains("VIOLATION"));
    assert!(text.contains("UntrackedWrite"));
    assert!(text.contains("account 1"));

    let inconclusive = verify(
        &c,
        &[AccountDelta::new(1, &pre, &post)],
        &TouchMap {
            overflowed: true,
            skipped: false,
            records: vec![],
        },
    );
    assert!(inconclusive.render().contains("INCONCLUSIVE_PARTIAL_MAP"));
}
