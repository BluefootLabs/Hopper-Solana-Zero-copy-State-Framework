//! Parse the REAL `hopper.manifest.json` for `examples/hopper-sentinel`.
//!
//! The fixture in `tests/fixtures/hopper-sentinel.manifest.json` was
//! generated ONCE from source with:
//!
//! ```text
//! cargo run -p hopper-cli -- compile --emit manifest \
//!     --package hopper-sentinel \
//!     --out crates/grillo-manifest/tests/fixtures/hopper-sentinel.manifest.json
//! ```
//!
//! i.e. it is the exact output of `hopper_schema::codama::ManifestJson`
//! rendered from the SAME `PROGRAM_MANIFEST` statics the runtime enforces —
//! not a hand-written approximation.

use grillo_manifest::{MutationManifest, RangeContract};

const SENTINEL_MANIFEST: &str = include_str!("fixtures/hopper-sentinel.manifest.json");

fn manifest() -> MutationManifest {
    MutationManifest::from_json(SENTINEL_MANIFEST).expect("real sentinel manifest must parse")
}

#[test]
fn sentinel_manifest_parses() {
    let m = manifest();
    assert_eq!(m.program_name, "hopper-sentinel");
    assert_eq!(m.program_version, "0.2.1");
    // Typed `Ctx<Spec>` handlers only: tags 0,1,3,4,6,7,8. The raw
    // `&mut Context` handlers (malicious_pause=2, record_entry=5) publish no
    // instruction descriptor, so they are absent — see the raw-handler delta
    // assertion below.
    assert_eq!(m.instructions.len(), 7);
}

/// THE REQUIRED ASSERTION: the flagship `Pause` instruction (`honest_pause`)
/// parses with `strict_writes = true` and EXACTLY the two authorized ranges
/// — `paused` (offset 114, size 1) and `revision` (offset 115, size 8), both
/// on the config account (account index 1).
#[test]
fn honest_pause_has_strict_writes_and_exactly_the_two_declared_ranges() {
    let m = manifest();
    let pause = m
        .instruction("honest_pause")
        .expect("honest_pause is a published instruction");

    assert!(
        pause.strict_writes,
        "the Pause context compiled strict_writes"
    );
    assert_eq!(pause.tag, 1);
    assert_eq!(
        pause.authorized,
        vec![
            RangeContract {
                account_index: 1,
                offset: 114,
                size: 1
            }, // paused
            RangeContract {
                account_index: 1,
                offset: 115,
                size: 8
            }, // revision
        ],
        "exactly the declared paused + revision ranges, and nothing else"
    );
    // `admin` (offset 16) is NOT in the authorized set — the whole point of
    // the flagship refusal.
    assert!(
        !pause.authorized.iter().any(|r| r.contains_byte(16)),
        "admin bytes are not authorized"
    );
    assert_eq!(pause.account_name(0), Some("admin"));
    assert_eq!(pause.account_name(1), Some("config"));
    // A bare strict_writes instruction is NOT mutation-complete.
    assert!(!pause.mutation_complete);
    assert!(pause.lamport_accounts.is_empty());
}

/// `unpause` shares the same `Pause` context, so it publishes the identical
/// write set — and therefore the identical per-instruction commitment.
#[test]
fn unpause_shares_the_pause_write_set_and_commitment() {
    let m = manifest();
    let honest = m.instruction("honest_pause").unwrap();
    let unpause = m.instruction("unpause").unwrap();
    assert_eq!(honest.authorized, unpause.authorized);
    // Same ranges + flags, different name/tag => the per-instruction
    // commitment DIFFERS (identity is part of the contract) ...
    assert_ne!(honest.commitment(), unpause.commitment());
    // ... but the byte ranges themselves are identical.
    assert_eq!(honest.strict_writes, unpause.strict_writes);
}

/// `collect_fees` is the mutation-complete instruction: it declares BOTH a
/// data range (`config.revision`) and the lamport dimension
/// (`lamports(fee_sink)`).
#[test]
fn collect_fees_is_mutation_complete_with_the_declared_lamport_account() {
    let m = manifest();
    let cf = m.instruction("collect_fees").unwrap();

    assert!(cf.strict_writes);
    assert!(
        cf.mutation_complete,
        "collect_fees declared the lamport dimension"
    );
    assert_eq!(cf.lamport_accounts, vec![1], "only fee_sink (index 1)");
    // config is account index 0 here (different account order than Pause).
    assert_eq!(
        cf.authorized,
        vec![RangeContract {
            account_index: 0,
            offset: 115,
            size: 8
        }],
        "only config.revision is a declared data range"
    );
    assert_eq!(cf.account_name(0), Some("config"));
    assert_eq!(cf.account_name(1), Some("fee_sink"));
    assert_eq!(cf.account_name(2), Some("treasury"));

    assert!(cf.authorizes_lamports(1), "fee_sink may move lamports");
    assert!(!cf.authorizes_lamports(2), "treasury may NOT move lamports");
    assert!(
        !cf.authorizes_lamports(0),
        "config has no lamport permission"
    );
}

/// DELTA: raw `&mut Context` handlers publish no instruction descriptor.
#[test]
fn raw_context_handlers_are_absent_from_the_manifest() {
    let m = manifest();
    assert!(
        m.instruction_by_tag(2).is_none(),
        "malicious_pause (raw &mut Context handler) publishes no descriptor"
    );
    assert!(
        m.instruction_by_tag(5).is_none(),
        "record_entry (raw &mut Context handler) publishes no descriptor"
    );
    // The typed handlers are present.
    assert_eq!(m.instruction_by_tag(1).unwrap().name, "honest_pause");
    assert_eq!(m.instruction_by_tag(8).unwrap().name, "collect_fees");
}

#[test]
fn serde_round_trips_through_the_model() {
    let m = manifest();
    let json = serde_json::to_string(&m).expect("model serializes");
    let back: MutationManifest = serde_json::from_str(&json).expect("model deserializes");
    assert_eq!(m, back, "MutationManifest round-trips through serde");
}

#[test]
fn commitment_is_stable_across_reparses_and_sensitive_to_ranges() {
    let a = MutationManifest::from_json(SENTINEL_MANIFEST).unwrap();
    let b = MutationManifest::from_json(SENTINEL_MANIFEST).unwrap();
    assert_eq!(
        a.commitment(),
        b.commitment(),
        "deterministic across reparses"
    );

    // Tamper with one authorized range: the commitment must move.
    let mut tampered = a.clone();
    let pause = tampered
        .instructions
        .iter_mut()
        .find(|i| i.name == "honest_pause")
        .unwrap();
    pause.authorized[0].size += 1; // paused 1 -> 2 bytes
    assert_ne!(
        a.commitment(),
        tampered.commitment(),
        "widening an authorized range changes the whole-manifest commitment"
    );
}
