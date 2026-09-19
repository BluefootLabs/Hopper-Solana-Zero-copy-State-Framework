//! Authority diff over the real emitted Sentinel and Cicada manifests.
//!
//! The unit tests in `src/authority.rs` pin each rule on small hand-written
//! manifests. These tests run the same rules over the checked-in renders of
//! `hopper_schema::codama::ManifestJson`, so a key rename in the producer
//! (`hasOne`, `seeds`, `discriminatorBytes`, `parametricWriteRanges`) cannot
//! silently turn the gate into a no-op that reports every upgrade as safe.

use grillo_manifest::authority::{AuthorityDiff, AuthorityImpact, AuthorityVerdict};
use serde_json::Value;

const SENTINEL: &str = include_str!("fixtures/hopper-sentinel.manifest.json");
const CICADA: &str = include_str!("fixtures/hopper-cicada.manifest.json");

fn mutate(json: &str, edit: impl FnOnce(&mut Value)) -> String {
    let mut value: Value = serde_json::from_str(json).unwrap();
    edit(&mut value);
    serde_json::to_string_pretty(&value).unwrap()
}

fn instruction<'a>(value: &'a mut Value, name: &str) -> &'a mut Value {
    value["instructions"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|ix| ix["name"] == name)
        .unwrap_or_else(|| panic!("instruction `{name}` in fixture"))
}

fn codes(
    report: &grillo_manifest::authority::AuthorityReport,
    impact: AuthorityImpact,
) -> Vec<String> {
    report.with_impact(impact).map(|f| f.code.clone()).collect()
}

#[test]
fn real_manifests_do_not_widen_against_themselves() {
    for fixture in [SENTINEL, CICADA] {
        let report = AuthorityDiff::between_json(fixture, fixture).unwrap();
        assert!(report.findings.is_empty(), "{}", report.render());
        assert_eq!(report.verdict(), AuthorityVerdict::NotWidened);
    }
}

#[test]
fn sentinel_dropped_signer_and_wider_range_are_named() {
    let upgraded = mutate(SENTINEL, |m| {
        let ix = instruction(m, "honest_pause");
        ix["accounts"][0]["signer"] = Value::Bool(false);
        ix["writeRanges"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({ "accountIndex": 1, "offset": 112, "size": 2 }));
    });
    let report = AuthorityDiff::between_json(SENTINEL, &upgraded).unwrap();
    assert_eq!(report.verdict(), AuthorityVerdict::Widened);
    assert_eq!(
        codes(&report, AuthorityImpact::Widened),
        ["signer_dropped", "write_range_widened"]
    );
    let range = report
        .findings
        .iter()
        .find(|f| f.code == "write_range_widened")
        .unwrap();
    assert_eq!(range.account.as_deref(), Some("config"));
    assert_eq!(range.detail, "gains `fee_bps`");
}

#[test]
fn cicada_context_relations_are_compared() {
    let upgraded = mutate(CICADA, |m| {
        for context in m["contexts"].as_array_mut().unwrap() {
            if context["name"] != "ClaimIntent" {
                continue;
            }
            for account in context["accounts"].as_array_mut().unwrap() {
                account["hasOne"] = serde_json::json!([]);
            }
        }
    });
    let report = AuthorityDiff::between_json(CICADA, &upgraded).unwrap();
    assert_eq!(
        report.verdict(),
        AuthorityVerdict::Widened,
        "{}",
        report.render()
    );
    assert!(report
        .findings
        .iter()
        .all(|f| f.instruction == "claim_intent" && f.code == "has_one_removed"));
}

#[test]
fn cicada_exact_cell_rules_are_compared() {
    let upgraded = mutate(CICADA, |m| {
        let ix = instruction(m, "claim_intent");
        assert!(
            !ix["parametricWriteRanges"].as_array().unwrap().is_empty(),
            "fixture must carry exact-cell rules for this test to mean anything"
        );
        ix["parametricWriteRanges"] = serde_json::json!([]);
    });
    let report = AuthorityDiff::between_json(CICADA, &upgraded).unwrap();
    let widened = codes(&report, AuthorityImpact::Widened);
    assert!(
        !widened.is_empty() && widened.iter().all(|c| c == "exact_cell_rule_removed"),
        "{}",
        report.render()
    );
}
