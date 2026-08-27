use std::collections::BTreeSet;

use super::{
    FindingClass, FindingFingerprint, ReviewCycleState, ReviewInvocationScope, RunBudgets,
    RunHistory,
};

#[test]
fn run_history_defaults_missing_fields_and_rejects_unknown_fields() {
    let recovered = serde_json::from_value::<RunHistory>(serde_json::json!({})).unwrap();
    assert_eq!(recovered, RunHistory::default());

    let partial = serde_json::from_value::<RunHistory>(serde_json::json!({
        "repairs_used": 1,
        "seen_fingerprints": [],
    }))
    .unwrap();
    assert_eq!(partial.repairs_used, 1);
    assert_eq!(partial.manual_repairs_used, 0);
    assert_eq!(partial.transitions_used, 0);
    assert_eq!(partial.initial_review_count, 0);
    assert_eq!(partial.verification_review_count, 0);
    assert!(partial.review_cycles.is_empty());

    let cycles = serde_json::from_value::<RunHistory>(serde_json::json!({
        "review_cycles": {
            "draft:outline_001": {
                "initial_count": 1,
                "verification_count": 1
            }
        }
    }))
    .unwrap();
    assert_eq!(
        cycles.review_cycles["draft:outline_001"],
        ReviewCycleState {
            repairs_used: 0,
            initial_count: 1,
            verification_count: 1,
        },
        "old cycle JSON without repairs_used must recover a zero cycle budget"
    );

    assert!(
        serde_json::from_value::<RunHistory>(serde_json::json!({
            "unexpected": true,
        }))
        .is_err()
    );
}

#[test]
fn run_budgets_default_to_fixed_policy_limits() {
    assert_eq!(
        RunBudgets::default(),
        RunBudgets {
            max_repairs: 1,
            max_transitions: 12,
            max_manual_repairs: 3,
        }
    );
}

#[test]
fn review_invocation_scope_digest_is_canonical_and_validated() {
    let first = ReviewInvocationScope::initial("revision-001");
    let second = ReviewInvocationScope::initial("revision-001");
    assert_eq!(first, second);
    first.validate_digest().unwrap();

    let original_fingerprints = BTreeSet::from([
        FindingFingerprint::for_finding(None, FindingClass::Repairable, "first", Some("field_one")),
        FindingFingerprint::for_finding(
            None,
            FindingClass::Repairable,
            "second",
            Some("field_two"),
        ),
    ]);
    let verification = ReviewInvocationScope::verification(
        original_fingerprints,
        "revision-002",
        "mechanical-report-001",
    );
    verification.validate_digest().unwrap();

    let empty_verification = ReviewInvocationScope::verification(
        BTreeSet::new(),
        "revision-002",
        "mechanical-report-001",
    );
    assert_eq!(
        serde_json::to_value(&empty_verification).unwrap()["phase"],
        "verification"
    );
    empty_verification.validate_digest().unwrap();

    let serialized = serde_json::to_value(&verification).unwrap();
    assert_eq!(serialized["phase"], "verification");
    assert!(serialized.get("initial_revision_id").is_none());
    assert!(
        serde_json::from_value::<ReviewInvocationScope>(serialized)
            .unwrap()
            .validate_digest()
            .is_ok()
    );
}

#[test]
fn review_invocation_scope_rejects_invalid_digest_shape_or_recalculation() {
    let scope = ReviewInvocationScope::initial("revision-001");

    let mut invalid_initial_shape = serde_json::to_value(&scope).unwrap();
    invalid_initial_shape["mechanical_report_ref"] = serde_json::json!("not applicable");
    assert!(serde_json::from_value::<ReviewInvocationScope>(invalid_initial_shape).is_err());

    for digest in [
        "not_a_scope_digest",
        "review_scope_v1:too_short",
        "review_scope_v1:A0d52e876560346f9c9fd0777ad620c166699c1d43eb6f42b0134e7506d4b2f8",
        "review_scope_v1:g0d52e876560346f9c9fd0777ad620c166699c1d43eb6f42b0134e7506d4b2f8",
        "review_scope_v1:0000000000000000000000000000000000000000000000000000000000000000",
    ] {
        let mut invalid = serde_json::to_value(&scope).unwrap();
        invalid["scope_digest"] = serde_json::json!(digest);
        assert!(
            serde_json::from_value::<ReviewInvocationScope>(invalid).is_err(),
            "scope digest {digest:?} should be rejected"
        );
    }
}
