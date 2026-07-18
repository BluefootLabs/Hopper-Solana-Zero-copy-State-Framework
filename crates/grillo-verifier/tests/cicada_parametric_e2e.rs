//! End-to-end: the REAL `hopper-cicada` manifest, resolved for a concrete
//! invocation, verified against synthetic pre/post snapshots and a touch map.
//!
//! This closes the last seam in the invocation-parametric Effect ABI path.
//! Every other parametric verifier test hand-builds an `InstructionContract`
//! in Rust; this one parses the exact JSON Hopper emits (the same
//! `PARAMETRIC_WRITE_RANGES` statics the runtime installs as its write
//! policy), resolves a real `execute_intent` for a chosen slot, and drives it
//! through `changed ⊆ acquired ⊆ authorized` — proving the producer → JSON →
//! parser → resolver → verifier chain agrees on the byte, not just in a
//! unit test's imagination.

use grillo_verifier::{
    verify, verify_invocation, AccountDelta, InconclusiveReason, MutationManifest, TouchMap,
    TouchRecord, Verdict, Violation,
};

/// The fixture is owned by the sibling crate's test tree; both crates read the
/// one canonical emitted manifest so a field-name drift fails in both.
const CICADA_MANIFEST: &str =
    include_str!("../../grillo-manifest/tests/fixtures/hopper-cicada.manifest.json");

/// The `IntentShard` is instruction account index 2 in every lifecycle handler.
const SHARD: u8 = 2;
/// Any length ≥ the shard's max authorized byte (the revisions envelope ends
/// at 9080); a real `IntentShard` is larger still. Both snapshots share it.
const SHARD_LEN: usize = 9_216;

fn manifest() -> MutationManifest {
    MutationManifest::from_json(CICADA_MANIFEST).expect("real cicada manifest parses")
}

fn write_rec(offset: u32, size: u32) -> TouchRecord {
    TouchRecord {
        slot: SHARD,
        offset,
        size,
        write: true,
    }
}

/// A complete (non-partial) touch map — the only kind that can yield a verdict.
fn complete_map(records: Vec<TouchRecord>) -> TouchMap {
    TouchMap {
        overflowed: false,
        skipped: false,
        records,
    }
}

/// The seven executor-column cells `execute_intent` writes for `slot`, as
/// `(offset, size)` — mirroring the runtime's `segment_mut` calls. Bases and
/// strides are read straight off the emitted manifest.
fn executor_cells(slot: u32) -> Vec<(u32, u32)> {
    const COLUMNS: [(u32, u32); 7] = [
        (7140, 1),  // statuses
        (7160, 32), // claimants
        (7800, 8),  // claim_expiries
        (7960, 8),  // settled_inputs
        (8120, 8),  // settled_outputs
        (8280, 32), // settlement_hashes
        (8920, 8),  // revisions
    ];
    COLUMNS
        .iter()
        .map(|&(base, size)| (base + slot * size, size))
        .collect()
}

/// The unresolved API on a parametric instruction must refuse to guess: it is
/// INCONCLUSIVE, never a false PASS against the conservative column envelope.
#[test]
fn unresolved_execute_intent_is_inconclusive_not_a_false_pass() {
    let m = manifest();
    let ix = m.instruction("execute_intent").unwrap();
    let pre = vec![0u8; SHARD_LEN];
    let verdict = verify(
        ix,
        &[AccountDelta::new(SHARD, &pre, &pre)],
        &complete_map(vec![]),
    );
    assert_eq!(
        verdict,
        Verdict::Inconclusive(InconclusiveReason::ParametricArgumentsRequired),
    );
}

/// A real slot-3 execute that writes exactly its seven executor cells, and
/// tells the runtime it did, verifies as a scoped PASS.
#[test]
fn a_real_slot3_execute_verifies_as_a_scoped_pass() {
    let m = manifest();
    let ix = m.instruction("execute_intent").unwrap();
    let slot = 3u32;

    let pre = vec![0u8; SHARD_LEN];
    let mut post = pre.clone();
    let mut records = Vec::new();
    for (off, size) in executor_cells(slot) {
        for b in off..off + size {
            post[b as usize] = 0xAB; // a genuine change
        }
        records.push(write_rec(off, size));
    }

    let verdict = verify_invocation(
        ix,
        &(slot as u16).to_le_bytes(),
        &[AccountDelta::new(SHARD, &pre, &post)],
        &complete_map(records),
    )
    .expect("slot 3 resolves");
    assert!(
        verdict.is_pass(),
        "slot-3 executor writes are exactly authorized:\n{}",
        verdict.render()
    );
}

/// Acquiring the seven executor cells but CHANGING none of them is legal —
/// access is not modification — and surfaces as PASS notes, not a violation.
#[test]
fn acquired_but_unchanged_executor_cells_are_a_clean_pass() {
    let m = manifest();
    let ix = m.instruction("execute_intent").unwrap();
    let slot = 9u32;

    let pre = vec![0u8; SHARD_LEN];
    let records: Vec<TouchRecord> = executor_cells(slot)
        .into_iter()
        .map(|(off, size)| write_rec(off, size))
        .collect();

    let verdict = verify_invocation(
        ix,
        &(slot as u16).to_le_bytes(),
        &[AccountDelta::new(SHARD, &pre, &pre)], // post == pre: nothing changed
        &complete_map(records),
    )
    .expect("slot 9 resolves");
    match verdict {
        Verdict::Pass(ev) => {
            assert_eq!(ev.changed_bytes, 0);
            assert_eq!(
                ev.acquired_unchanged.len(),
                7,
                "all seven leases went unused"
            );
        }
        other => panic!("expected a clean PASS, got:\n{}", other.render()),
    }
}

/// Under a slot-3 authorization, writing slot-4's status byte acquires a lease
/// on a byte the resolved contract does not permit — the exact-cell guarantee.
#[test]
fn writing_a_neighbor_slots_cell_is_an_unauthorized_acquisition() {
    let m = manifest();
    let ix = m.instruction("execute_intent").unwrap();

    let pre = vec![0u8; SHARD_LEN];
    let mut post = pre.clone();
    let neighbor = 7140 + 4; // slot-4 status; slot-3's is 7143
    post[neighbor] = 1;

    let verdict = verify_invocation(
        ix,
        &3u16.to_le_bytes(),
        &[AccountDelta::new(SHARD, &pre, &post)],
        &complete_map(vec![write_rec(neighbor as u32, 1)]),
    )
    .expect("slot 3 resolves");
    match verdict {
        Verdict::Violation(v) => assert!(v.contains(&Violation::UnauthorizedAcquisition {
            account_index: SHARD,
            offset: neighbor as u32,
            size: 1,
        })),
        other => panic!(
            "a slot-4 write under a slot-3 authorization must be a violation:\n{}",
            other.render()
        ),
    }
}

/// The security crux: `execute_intent` publishes NO authority over the user's
/// immutable columns (owners/route/limits), so a write there is refused even
/// for a perfectly-resolved invocation. This is what stops a compromised
/// executor from rewriting the intent's own constraints.
#[test]
fn execute_intent_cannot_touch_an_immutable_user_column() {
    let m = manifest();
    let ix = m.instruction("execute_intent").unwrap();

    let pre = vec![0u8; SHARD_LEN];
    let mut post = pre.clone();
    // owners column base 80, stride 32 → slot-3 owner cell [176, 208).
    for b in 176..208 {
        post[b] = 1;
    }

    let verdict = verify_invocation(
        ix,
        &3u16.to_le_bytes(),
        &[AccountDelta::new(SHARD, &pre, &post)],
        &complete_map(vec![write_rec(176, 32)]),
    )
    .expect("slot 3 resolves");
    match verdict {
        Verdict::Violation(v) => assert!(v.contains(&Violation::UnauthorizedAcquisition {
            account_index: SHARD,
            offset: 176,
            size: 32,
        })),
        other => panic!(
            "execute must not rewrite the immutable owners column:\n{}",
            other.render()
        ),
    }
}

/// A change to an authorized cell that the instruction never declared in its
/// touch map is an UntrackedWrite: `changed ⊄ acquired`, even though the byte
/// lies inside the authorized set.
#[test]
fn an_undeclared_change_to_an_authorized_cell_is_an_untracked_write() {
    let m = manifest();
    let ix = m.instruction("execute_intent").unwrap();

    let pre = vec![0u8; SHARD_LEN];
    let mut post = pre.clone();
    post[7143] = 0xAB; // slot-3 status: authorized, but not reported

    let verdict = verify_invocation(
        ix,
        &3u16.to_le_bytes(),
        &[AccountDelta::new(SHARD, &pre, &post)],
        &complete_map(vec![]), // empty: nothing acquired
    )
    .expect("slot 3 resolves");
    match verdict {
        Verdict::Violation(v) => assert!(v.contains(&Violation::UntrackedWrite {
            account_index: SHARD,
            offset: 7143,
        })),
        other => panic!(
            "an unreported change must be an UntrackedWrite:\n{}",
            other.render()
        ),
    }
}

/// An out-of-range slot is a resolution error, returned distinctly from any
/// verdict — it can never be mistaken for a PASS.
#[test]
fn out_of_range_slot_is_a_resolution_error_not_a_verdict() {
    let m = manifest();
    let ix = m.instruction("execute_intent").unwrap();
    let pre = vec![0u8; SHARD_LEN];
    let result = verify_invocation(
        ix,
        &20u16.to_le_bytes(), // count is 20 → slot 20 is out of range
        &[AccountDelta::new(SHARD, &pre, &pre)],
        &complete_map(vec![]),
    );
    assert!(
        result.is_err(),
        "an out-of-range selector must fail resolution, not verify"
    );
}
