//! The `grillo verify` bundle path, driven against the REAL emitted
//! hopper-cicada manifest — the same fixture the parametric round-trip
//! test parses. Proves the offline evidence-bundle format an independent
//! party would hand the `grillo` CLI decodes to the same byte verdicts as
//! the in-process verifier API: PASS, both violation classes, and the
//! fail-closed inconclusive.

#![cfg(feature = "cli")]

use grillo_verifier::{parse_bundle, verify_bundle, MutationManifest, Verdict, Violation};

const CICADA_MANIFEST: &str =
    include_str!("../../grillo-manifest/tests/fixtures/hopper-cicada.manifest.json");

const SHARD: u8 = 2;
const SHARD_LEN: usize = 9_216;

fn manifest() -> MutationManifest {
    MutationManifest::from_json(CICADA_MANIFEST).expect("real cicada manifest parses")
}

fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// The seven executor-column cells execute_intent writes for `slot`.
fn executor_cells(slot: u32) -> Vec<(u32, u32)> {
    const COLUMNS: [(u32, u32); 7] = [
        (7140, 1),
        (7160, 32),
        (7800, 8),
        (7960, 8),
        (8120, 8),
        (8280, 32),
        (8920, 8),
    ];
    COLUMNS
        .iter()
        .map(|&(base, size)| (base + slot * size, size))
        .collect()
}

/// Build a v1 touch-map wire blob for `write` records `(offset, size)` on
/// the shard, matching the format `decode_touch_map` consumes.
fn touch_blob(writes: &[(u32, u32)]) -> Vec<u8> {
    let mut out = vec![0x7a, 0x01, 0x00, writes.len() as u8];
    for &(off, size) in writes {
        out.push(SHARD);
        out.extend_from_slice(&(off | 0x8000_0000).to_le_bytes()); // bit31 = write
        out.extend_from_slice(&size.to_le_bytes());
    }
    out
}

fn bundle_json(slot: u16, pre: &[u8], post: &[u8], writes: &[(u32, u32)]) -> String {
    format!(
        r#"{{
          "instruction": "execute_intent",
          "argumentPayload": "{}",
          "touchMap": "{}",
          "accounts": [ {{ "index": {SHARD}, "pre": "{}", "post": "{}" }} ]
        }}"#,
        hex(&slot.to_le_bytes()),
        hex(&touch_blob(writes)),
        hex(pre),
        hex(post),
    )
}

#[test]
fn a_real_slot3_execute_bundle_verifies_as_a_scoped_pass() {
    let slot = 3u32;
    let pre = vec![0u8; SHARD_LEN];
    let mut post = pre.clone();
    let cells = executor_cells(slot);
    for (off, size) in &cells {
        for b in *off..*off + *size {
            post[b as usize] = 0xAB;
        }
    }
    let json = bundle_json(slot as u16, &pre, &post, &cells);
    let bundle = parse_bundle(&json).expect("bundle parses");
    let verdict = verify_bundle(&manifest(), &bundle).expect("verifies");
    assert!(verdict.is_pass(), "scoped PASS:\n{}", verdict.render());
}

#[test]
fn a_neighbor_slot_write_bundle_is_a_violation() {
    let pre = vec![0u8; SHARD_LEN];
    let mut post = pre.clone();
    let neighbor = 7140 + 4; // slot-4 status under a slot-3 authorization
    post[neighbor] = 1;
    let json = bundle_json(3, &pre, &post, &[(neighbor as u32, 1)]);
    let bundle = parse_bundle(&json).expect("parses");
    match verify_bundle(&manifest(), &bundle).expect("verifies") {
        Verdict::Violation(v) => assert!(v.contains(&Violation::UnauthorizedAcquisition {
            account_index: SHARD,
            offset: neighbor as u32,
            size: 1,
        })),
        other => panic!("expected a violation:\n{}", other.render()),
    }
}

#[test]
fn an_immutable_user_column_write_bundle_is_a_violation() {
    let pre = vec![0u8; SHARD_LEN];
    let mut post = pre.clone();
    for b in 176..208 {
        post[b] = 1; // owners slot-3 cell — execute has no authority there
    }
    let json = bundle_json(3, &pre, &post, &[(176, 32)]);
    let bundle = parse_bundle(&json).expect("parses");
    assert!(matches!(
        verify_bundle(&manifest(), &bundle).expect("verifies"),
        Verdict::Violation(_)
    ));
}

#[test]
fn a_parametric_bundle_without_argument_payload_is_inconclusive() {
    let pre = vec![0u8; SHARD_LEN];
    // No argumentPayload: the resolver cannot run, so the verifier must
    // refuse rather than guess against the static envelope.
    let json = format!(
        r#"{{ "instruction": "execute_intent", "touchMap": "{}",
              "accounts": [ {{ "index": {SHARD}, "pre": "{}", "post": "{}" }} ] }}"#,
        hex(&touch_blob(&[])),
        hex(&pre),
        hex(&pre),
    );
    let bundle = parse_bundle(&json).expect("parses");
    assert!(matches!(
        verify_bundle(&manifest(), &bundle).expect("verifies"),
        Verdict::Inconclusive(_)
    ));
}

#[test]
fn malformed_hex_is_a_bundle_error_never_a_verdict() {
    let json = format!(
        r#"{{ "instruction": "execute_intent", "argumentPayload": "0300", "touchMap": "{}",
              "accounts": [ {{ "index": {SHARD}, "pre": "zz", "post": "00" }} ] }}"#,
        hex(&touch_blob(&[])),
    );
    let bundle = parse_bundle(&json).expect("parses");
    assert!(verify_bundle(&manifest(), &bundle).is_err());
}
