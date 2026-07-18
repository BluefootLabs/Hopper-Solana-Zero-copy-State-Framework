//! Parse and RESOLVE the real `hopper-cicada` manifest.
//!
//! This is the first end-to-end coverage of the invocation-parametric Effect
//! ABI path against a manifest Hopper actually EMITS. The sentinel fixture
//! ([`parse_real_manifest`]) is static-only, so before this test the whole
//! producer→JSON→parser→resolver seam for `parametricWriteRanges` was
//! exercised only by hand-built `InstructionContract`s — a field-name drift
//! between `hopper_schema::codama` and `grillo_manifest`'s serde model would
//! have gone uncaught.
//!
//! The fixture was generated ONCE from source with:
//!
//! ```text
//! cargo run -p hopper-cli -- compile --emit manifest \
//!     --package hopper-cicada \
//!     --out crates/grillo-manifest/tests/fixtures/hopper-cicada.manifest.json
//! ```
//!
//! i.e. it is the exact `hopper_schema::codama::ManifestJson` render of the
//! SAME `PARAMETRIC_WRITE_RANGES` statics the runtime installs as its write
//! policy — published == enforced.

use grillo_manifest::MutationManifest;

const CICADA_MANIFEST: &str = include_str!("fixtures/hopper-cicada.manifest.json");

fn manifest() -> MutationManifest {
    MutationManifest::from_json(CICADA_MANIFEST).expect("real cicada manifest must parse")
}

#[test]
fn cicada_manifest_parses_with_parametric_instructions() {
    let m = manifest();
    assert_eq!(m.program_name, "hopper-cicada");
    let parametric: Vec<&str> = m
        .instructions
        .iter()
        .filter(|i| !i.parametric.is_empty())
        .map(|i| i.name.as_str())
        .collect();
    // The column-oriented lifecycle handlers all carry exact-cell rules.
    for expected in [
        "claim_intent",
        "release_claim",
        "cancel_intent",
        "execute_intent",
        "reclaim_intent",
    ] {
        assert!(
            parametric.contains(&expected),
            "expected parametric rules on {expected}"
        );
    }
}

/// `execute_intent` publishes exactly the seven executor-column exact-cell
/// rules, all keyed on the `slot` u16 selector at compact argument index 0
/// over the shard account (index 2), each with `count == INTENTS_PER_SHARD`.
#[test]
fn execute_intent_publishes_the_executor_column_cell_rules() {
    let m = manifest();
    let ix = m
        .instruction("execute_intent")
        .expect("execute_intent is a published instruction");
    assert!(ix.strict_writes);

    // The selector's wire descriptor: `slot` is the first fixed u16 argument.
    let slot_arg = ix
        .args
        .iter()
        .find(|a| a.name == "slot")
        .expect("slot arg descriptor");
    assert_eq!(slot_arg.canonical_type, "u16");
    assert_eq!(slot_arg.size, 2);

    let segments: Vec<&str> = ix
        .parametric
        .iter()
        .map(|p| p.segment_name.as_str())
        .collect();
    for expected in [
        "statuses",
        "claimants",
        "claim_expiries",
        "settled_inputs",
        "settled_outputs",
        "settlement_hashes",
        "revisions",
    ] {
        assert!(
            segments.contains(&expected),
            "missing executor-column rule `{expected}`"
        );
    }
    // No IMMUTABLE user column (owners/route_programs/max_inputs/…) appears —
    // execute publishes no authority to rewrite the user's constraints.
    for forbidden in [
        "owners",
        "route_programs",
        "max_inputs",
        "min_outputs",
        "vault_authorities",
    ] {
        assert!(
            !segments.contains(&forbidden),
            "execute must not govern `{forbidden}`"
        );
    }

    for rule in &ix.parametric {
        assert_eq!(rule.account_index, 2, "executor columns live on the shard");
        assert_eq!(rule.argument_name, "slot");
        assert_eq!(rule.argument_index, 0);
        assert_eq!(rule.count, 20);
        // Hopper array columns are packed, so stride == cell size.
        assert_eq!(rule.stride, rule.cell_size);
        assert!(rule.cell_size > 0 && rule.count > 0);
    }
}

/// Resolving `execute_intent` for slot 3 narrows every executor envelope to
/// the slot-3 cell and — the crown-jewel invariant, checked on real emitted
/// data — NEVER broadens beyond the published static authorization.
#[test]
fn resolving_execute_intent_narrows_to_the_selected_cell_without_broadening() {
    let m = manifest();
    let ix = m.instruction("execute_intent").unwrap();

    // The real post-discriminator payload begins with the u16 `slot`; the
    // resolver stops after the last selector, so the trailing route_data /
    // route_meta_flags bytes are irrelevant to resolution.
    let resolved = ix
        .resolve_effects(&3u16.to_le_bytes())
        .expect("resolves for slot 3");

    let on_shard = |byte: u32| {
        resolved
            .authorized_ranges()
            .iter()
            .any(|r| r.account_index == 2 && r.contains_byte(byte))
    };
    // slot-3 status cell [7143,7144) authorized; slot-4's [7144,..) is NOT.
    assert!(on_shard(7143));
    assert!(!on_shard(7144));
    // slot-3 claimant cell [7256,7288) authorized; slot-2's [7224,7256) is NOT.
    assert!(on_shard(7256));
    assert!(!on_shard(7224));

    // NO broadening: every resolved shard byte lies inside the union of the
    // published static writeRanges for the shard. The resolver may only narrow.
    let static_shard: Vec<(u64, u64)> = ix
        .authorized
        .iter()
        .filter(|r| r.account_index == 2)
        .map(|r| (r.offset as u64, r.end()))
        .collect();
    for r in resolved
        .authorized_ranges()
        .iter()
        .filter(|r| r.account_index == 2)
    {
        let covered = static_shard
            .iter()
            .any(|&(s, e)| s <= r.offset as u64 && r.end() <= e);
        assert!(
            covered,
            "resolved range {r:?} broadens beyond static authorization"
        );
    }
}

/// The resolved certificate is selector-sensitive: a different slot yields a
/// different commitment, so a certificate can never be replayed for another
/// cell — while the source (published-rule) commitment stays constant.
#[test]
fn resolved_commitment_is_selector_sensitive() {
    let m = manifest();
    let ix = m.instruction("execute_intent").unwrap();
    let a = ix.resolve_effects(&3u16.to_le_bytes()).unwrap();
    let b = ix.resolve_effects(&4u16.to_le_bytes()).unwrap();
    assert_ne!(
        a.commitment(),
        b.commitment(),
        "different slot ⇒ different certificate"
    );
    assert_eq!(
        a.source_commitment(),
        b.source_commitment(),
        "same published rule set"
    );
}

/// `reclaim_intent` is the one handler authorized to zero the whole record, so
/// it publishes an exact-cell rule for every column — the immutable user
/// columns included — and resolving it narrows each to the selected slot.
#[test]
fn reclaim_intent_publishes_full_column_cell_rules() {
    let m = manifest();
    let ix = m.instruction("reclaim_intent").unwrap();
    assert!(
        ix.parametric.len() >= 20,
        "reclaim clears the full column set"
    );
    let resolved = ix
        .resolve_effects(&5u16.to_le_bytes())
        .expect("resolves for slot 5");
    // owners column base 80, stride 32 → slot-5 owner cell at 80 + 5*32 = 240.
    assert!(resolved
        .authorized_ranges()
        .iter()
        .any(|r| r.account_index == 2 && r.contains_byte(240)));
    // ...but slot 6's owner cell (272) is not authorized by a slot-5 reclaim.
    assert!(!resolved
        .authorized_ranges()
        .iter()
        .any(|r| r.account_index == 2 && r.contains_byte(272)));
}

/// An out-of-range selector fails closed against real published rules.
#[test]
fn out_of_range_slot_fails_closed() {
    let m = manifest();
    let ix = m.instruction("execute_intent").unwrap();
    // count is 20, so slot 20 is out of range.
    assert!(ix.resolve_effects(&20u16.to_le_bytes()).is_err());
    // A truncated payload cannot locate the selector.
    assert!(ix.resolve_effects(&[7]).is_err());
}
