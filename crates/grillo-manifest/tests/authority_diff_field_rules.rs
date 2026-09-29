//! Authority diff over the `fieldRules` of a manifest: what a stored value
//! must satisfy for its account to bind. A rule that is removed or loosened
//! admits state the old release refused, and the gate names it the way it
//! names a dropped signer.

use grillo_manifest::authority::{AuthorityDiff, AuthorityImpact, AuthorityVerdict};
use serde_json::{json, Value};

const SENTINEL: &str = include_str!("fixtures/hopper-sentinel.manifest.json");

fn with_rules(rules: Value) -> String {
    let mut value: Value = serde_json::from_str(SENTINEL).unwrap();
    value["fieldRules"] = json!([{ "layout": "Pool", "rules": rules }]);
    serde_json::to_string_pretty(&value).unwrap()
}

fn rule(field: &str, text: &str, min: Option<i128>, max: Option<i128>, exact: bool) -> Value {
    json!({
        "field": field,
        "rule": text,
        "min": min.map(|v| v.to_string()),
        "max": max.map(|v| v.to_string()),
        "exact": exact,
    })
}

fn findings(old: &str, new: &str) -> Vec<(AuthorityImpact, String, String)> {
    AuthorityDiff::between_json(old, new)
        .unwrap()
        .findings
        .into_iter()
        .map(|f| (f.impact, f.code, f.account.unwrap_or_default()))
        .collect()
}

fn tier(text: &str, min: Option<i128>, max: Option<i128>) -> String {
    with_rules(json!([rule("tier", text, min, max, true)]))
}

#[test]
fn a_manifest_without_rules_and_the_same_rules_report_nothing() {
    assert!(findings(SENTINEL, SENTINEL).is_empty());
    let ruled = tier("value >= 1 && value <= 10", Some(1), Some(10));
    assert!(findings(&ruled, &ruled).is_empty());
}

#[test]
fn a_looser_bound_is_a_widening() {
    let old = tier("value >= 1 && value <= 10", Some(1), Some(10));
    for new in [
        tier("value >= 1 && value <= 100", Some(1), Some(100)),
        tier("value >= 0 && value <= 10", Some(0), Some(10)),
        tier("value >= 1", Some(1), None),
        // Tighter below, looser above: a refused value now passes.
        tier("value >= 5 && value <= 11", Some(5), Some(11)),
    ] {
        let report = AuthorityDiff::between_json(&old, &new).unwrap();
        assert_eq!(
            report.verdict(),
            AuthorityVerdict::Widened,
            "{}",
            report.render()
        );
        assert_eq!(
            findings(&old, &new),
            [(
                AuthorityImpact::Widened,
                "field_rule_widened".to_string(),
                "tier".to_string()
            )]
        );
    }
}

#[test]
fn a_removed_rule_is_a_widening() {
    let old = tier("value >= 1 && value <= 10", Some(1), Some(10));
    assert_eq!(
        findings(&old, SENTINEL),
        [(
            AuthorityImpact::Widened,
            "field_rule_removed".to_string(),
            "tier".to_string()
        )]
    );
    // An empty rule list is the same as no key.
    assert_eq!(findings(&old, &with_rules(json!([]))).len(), 1);
}

#[test]
fn tighter_and_added_rules_narrow_and_say_what_stops_binding() {
    let old = tier("value >= 1 && value <= 10", Some(1), Some(10));
    let new = tier("value >= 2 && value <= 9", Some(2), Some(9));
    let report = AuthorityDiff::between_json(&old, &new).unwrap();
    assert_eq!(report.verdict(), AuthorityVerdict::NotWidened);
    assert_eq!(report.findings[0].code, "field_rule_tightened");
    assert!(report.findings[0].detail.contains("no longer binds"));
    assert!(report.findings[0].detail.contains("[1, 10] -> [2, 9]"));

    let report = AuthorityDiff::between_json(SENTINEL, &old).unwrap();
    assert_eq!(report.verdict(), AuthorityVerdict::NotWidened);
    assert_eq!(report.findings[0].code, "field_rule_added");
    assert_eq!(report.findings[0].instruction, "layout Pool");
}

#[test]
fn the_same_bounds_spelled_differently_are_informational() {
    let old = tier("value >= 1 && value <= 10", Some(1), Some(10));
    let new = tier("0 < value && value < 11", Some(1), Some(10));
    assert_eq!(
        findings(&old, &new),
        [(
            AuthorityImpact::Info,
            "field_rule_respelled".to_string(),
            "tier".to_string()
        )]
    );
}

#[test]
fn a_rule_with_a_condition_beyond_its_bounds_needs_review_unless_it_only_adds() {
    let old = with_rules(json!([rule(
        "deposited",
        "value >= 100 && value <= self.cap.get()",
        Some(100),
        None,
        false
    )]));
    // The cap condition is gone: the bounds did not widen, and nothing
    // orders the two rules.
    let new = with_rules(json!([rule(
        "deposited",
        "value >= 100",
        Some(100),
        None,
        true
    )]));
    let report = AuthorityDiff::between_json(&old, &new).unwrap();
    assert_eq!(
        report.verdict(),
        AuthorityVerdict::Review,
        "{}",
        report.render()
    );
    assert_eq!(report.findings[0].code, "field_rule_rewritten");

    // Two rules on the field, the old one kept: the field only got stricter.
    let new = with_rules(json!([
        rule(
            "deposited",
            "value >= 100 && value <= self.cap.get()",
            Some(100),
            None,
            false
        ),
        rule("deposited", "value % 2 == 0", None, None, false),
    ]));
    let report = AuthorityDiff::between_json(&old, &new).unwrap();
    assert_eq!(
        report.verdict(),
        AuthorityVerdict::NotWidened,
        "{}",
        report.render()
    );
    assert_eq!(report.findings[0].code, "field_rule_tightened");

    // A lower floor is a widening whatever else the rule says.
    let new = with_rules(json!([rule(
        "deposited",
        "value >= 50 && value <= self.cap.get()",
        Some(50),
        None,
        false
    )]));
    assert_eq!(
        AuthorityDiff::between_json(&old, &new).unwrap().verdict(),
        AuthorityVerdict::Widened
    );
}

#[test]
fn several_rules_on_one_field_intersect() {
    let old = with_rules(json!([
        rule("tier", "value >= 1", Some(1), None, true),
        rule("tier", "value <= 10", None, Some(10), true),
    ]));
    // One rule of the two is dropped: the ceiling is gone.
    let new = with_rules(json!([rule("tier", "value >= 1", Some(1), None, true)]));
    assert_eq!(
        findings(&old, &new),
        [(
            AuthorityImpact::Widened,
            "field_rule_widened".to_string(),
            "tier".to_string()
        )]
    );
}

#[test]
fn an_approval_covers_exactly_the_reviewed_widening() {
    let old = tier("value >= 1 && value <= 10", Some(1), Some(10));
    let new = tier("value >= 1 && value <= 100", Some(1), Some(100));
    let report = AuthorityDiff::between_json(&old, &new).unwrap();
    assert!(report.check_approval(&report).is_ok());

    let wider = tier("value >= 1 && value <= 1000", Some(1), Some(1000));
    let other = AuthorityDiff::between_json(&old, &wider).unwrap();
    assert!(other.check_approval(&report).is_err());
}
