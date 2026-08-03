use serde_json::json;

use crate::product::work_item_contract::{AcceptanceCriterion, EvidenceKind};

#[test]
fn scalar_required_evidence_is_normalized_to_single_element_list() {
    let value = json!({
        "criterion_id": "crt_ac_001",
        "statement": "demo criterion",
        "required_evidence": "non_zero_test_execution"
    });

    let parsed: AcceptanceCriterion = serde_json::from_value(value).unwrap();

    assert_eq!(
        parsed.required_evidence,
        vec![EvidenceKind::NonZeroTestExecution]
    );
}

#[test]
fn array_required_evidence_parses_unchanged() {
    let value = json!({
        "criterion_id": "crt_ac_002",
        "statement": "demo criterion",
        "required_evidence": ["source_diff", "manual_check"]
    });

    let parsed: AcceptanceCriterion = serde_json::from_value(value).unwrap();

    assert_eq!(
        parsed.required_evidence,
        vec![EvidenceKind::SourceDiff, EvidenceKind::ManualCheck]
    );
}

#[test]
fn invalid_scalar_required_evidence_is_rejected() {
    let value = json!({
        "criterion_id": "crt_ac_003",
        "statement": "demo criterion",
        "required_evidence": "not_a_real_evidence_kind"
    });

    assert!(serde_json::from_value::<AcceptanceCriterion>(value).is_err());
}

#[test]
fn invalid_element_in_required_evidence_array_is_rejected() {
    let value = json!({
        "criterion_id": "crt_ac_004",
        "statement": "demo criterion",
        "required_evidence": ["source_diff", "not_a_real_evidence_kind"]
    });

    assert!(serde_json::from_value::<AcceptanceCriterion>(value).is_err());
}
