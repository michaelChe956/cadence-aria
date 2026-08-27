use crate::product::work_item_plan_policy::{FindingFingerprint, ReviewInvocationScope};
use std::collections::BTreeSet;

#[test]
fn single_candidate_initial_prompt_is_derived_from_server_scope() {
    let scope = ReviewInvocationScope::initial("revision-001");
    let instructions = crate::product::workspace_engine::review_scope_instructions(&scope)
        .expect("initial scope instructions");

    assert!(instructions.contains("Initial"));
    assert!(instructions.contains("revision-001"));
    assert!(instructions.contains("一次全候选评估"));
    assert!(instructions.contains("category"));
    assert!(instructions.contains("class_hint"));
    assert!(instructions.contains("must_fix"));
    assert!(instructions.contains(scope.scope_digest()));
    assert!(!instructions.contains("review_invocation_scope"));
}

#[test]
fn single_candidate_verification_prompt_replays_only_original_fingerprints() {
    let fingerprint = FindingFingerprint::for_finding(
        Some(crate::product::work_item_plan_policy::ReviewFindingCategory::ContractGap),
        crate::product::work_item_plan_policy::FindingClass::Repairable,
        "original",
        Some("contract.field"),
    );
    let scope = ReviewInvocationScope::verification(
        BTreeSet::from([fingerprint.clone()]),
        "revision-002",
        "project/issue/plan/mechanical_report/report-002",
    );
    let instructions = crate::product::workspace_engine::review_scope_instructions(&scope)
        .expect("verification scope instructions");

    assert!(instructions.contains("Verification"));
    assert!(instructions.contains("revision-002"));
    assert!(instructions.contains("mechanical_report"));
    assert!(instructions.contains(fingerprint.0.as_str()));
    assert!(instructions.contains("仅复核原 fingerprints"));
    assert!(instructions.contains("advisory"));
    assert!(instructions.contains(scope.scope_digest()));
}

#[test]
fn single_candidate_scope_instructions_reject_invalid_digest() {
    let mut value = serde_json::to_value(ReviewInvocationScope::initial("revision-001")).unwrap();
    value["scope_digest"] = serde_json::Value::String("review_scope_v1:invalid".to_string());
    let scope = serde_json::from_value::<ReviewInvocationScope>(value);
    assert!(scope.is_err());
}

#[test]
fn single_candidate_scope_instructions_reject_empty_verification_report() {
    let scope = ReviewInvocationScope::verification(BTreeSet::new(), "revision-002", "");
    let error = crate::product::workspace_engine::review_scope_instructions(&scope)
        .expect_err("missing mechanical report must be fatal");
    assert!(error.contains("mechanical report"));
}
