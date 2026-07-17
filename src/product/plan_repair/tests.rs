use serde_json::json;

use crate::product::work_item_contract::BlockerRoute;

use super::{
    NormalizedPlanDefectRoute, PlanDefectClass, PlanDefectConfidence, PlanDefectEvidence,
    PlanDefectFinding, PlanDefectRoute, PlanDefectSeverity, PlanRepairError, PlanRepairRequest,
    RepairTarget, RepairTargetKind, default_route, normalize_blocker_route,
    plan_defect_fingerprint,
};

fn repair_request_json() -> serde_json::Value {
    json!({
        "id": "plan_repair_request_0001",
        "plan_id": "work_item_plan_0001",
        "base_plan_revision_id": "plan_revision_0001",
        "trigger_attempt_id": "coding_attempt_0001",
        "trigger_unit_run_id": "unit_run_0001",
        "trigger_review_id": "review_0001",
        "trigger_finding_id": "finding_0001",
        "amendment_id": null,
        "defect_class": "current_work_item_invalid",
        "reason_code": "contract_invalid",
        "repair_target": {
            "kind": "current_work_item",
            "logical_work_item_ids": ["logical_work_item_0001"],
            "work_item_revision_ids": ["work_item_revision_0001"]
        },
        "contract_refs": ["contract_0001"],
        "capability_refs": ["capability_0001"],
        "evidence": [{
            "kind": "review_finding",
            "source_ref": "review_0001#finding_0001",
            "message": "contract mismatch"
        }],
        "fingerprint": "fingerprint_0001",
        "status": "open",
        "created_at": "2026-07-17T00:00:04Z",
        "updated_at": "2026-07-17T00:00:04Z"
    })
}

#[test]
fn plan_repair_request_serde_is_closed_and_evidence_is_typed() {
    let mut unknown_request_field = repair_request_json();
    unknown_request_field["unexpected"] = json!(true);
    assert!(serde_json::from_value::<PlanRepairRequest>(unknown_request_field).is_err());

    let mut unknown_evidence_field = repair_request_json();
    unknown_evidence_field["evidence"][0]["unexpected"] = json!(true);
    assert!(serde_json::from_value::<PlanRepairRequest>(unknown_evidence_field).is_err());

    let mut weak_evidence = repair_request_json();
    weak_evidence["evidence"] = json!([{"kind": "review", "id": "finding_0001"}]);
    assert!(serde_json::from_value::<PlanRepairRequest>(weak_evidence).is_err());
}

fn target_fixture(
    kind: RepairTargetKind,
    logical_work_item_ids: &[&str],
    work_item_revision_ids: &[&str],
) -> RepairTarget {
    RepairTarget {
        kind,
        logical_work_item_ids: logical_work_item_ids
            .iter()
            .map(|value| (*value).to_string())
            .collect(),
        work_item_revision_ids: work_item_revision_ids
            .iter()
            .map(|value| (*value).to_string())
            .collect(),
    }
}

fn finding_fixture(
    defect_class: PlanDefectClass,
    repair_target: Option<RepairTarget>,
    contract_refs: &[&str],
    capability_refs: &[&str],
) -> PlanDefectFinding {
    let recommended_route = default_route(&defect_class);
    PlanDefectFinding {
        finding_id: "finding_0001".to_string(),
        severity: PlanDefectSeverity::Error,
        defect_class,
        reason_code: "contract_invalid".to_string(),
        message: "contract mismatch".to_string(),
        evidence: vec![PlanDefectEvidence {
            kind: "review_finding".to_string(),
            source_ref: "review_0001#finding_0001".to_string(),
            message: "contract mismatch".to_string(),
        }],
        contract_refs: contract_refs
            .iter()
            .map(|value| (*value).to_string())
            .collect(),
        capability_refs: capability_refs
            .iter()
            .map(|value| (*value).to_string())
            .collect(),
        repair_target,
        recommended_route,
        confidence: PlanDefectConfidence::High,
    }
}

#[test]
fn plan_repair_default_route_covers_every_defect_class() {
    let cases = [
        (
            PlanDefectClass::ImplementationDefect,
            PlanDefectRoute::CoderRework,
        ),
        (
            PlanDefectClass::VerificationIncomplete,
            PlanDefectRoute::VerificationRetry,
        ),
        (
            PlanDefectClass::CurrentWorkItemInvalid,
            PlanDefectRoute::PlanRepair,
        ),
        (
            PlanDefectClass::UpstreamContractInvalid,
            PlanDefectRoute::PlanRepair,
        ),
        (
            PlanDefectClass::DependencyGraphInvalid,
            PlanDefectRoute::PlanRepair,
        ),
        (
            PlanDefectClass::DesignAmendmentRequired,
            PlanDefectRoute::DesignAmendment,
        ),
        (
            PlanDefectClass::StoryAmendmentRequired,
            PlanDefectRoute::StoryAmendment,
        ),
        (
            PlanDefectClass::OperationalBlocker,
            PlanDefectRoute::OperationalGate,
        ),
    ];

    for (class, expected) in cases {
        assert_eq!(default_route(&class), expected);
    }
}

#[test]
fn plan_repair_normalizes_every_blocker_route_without_losing_target_kind() {
    let cases = [
        (
            BlockerRoute::CoderRework,
            PlanDefectRoute::CoderRework,
            None,
        ),
        (
            BlockerRoute::VerificationRetry,
            PlanDefectRoute::VerificationRetry,
            None,
        ),
        (
            BlockerRoute::PlanRepairCurrent,
            PlanDefectRoute::PlanRepair,
            Some(RepairTargetKind::CurrentWorkItem),
        ),
        (
            BlockerRoute::PlanRepairUpstream,
            PlanDefectRoute::PlanRepair,
            Some(RepairTargetKind::UpstreamWorkItem),
        ),
        (
            BlockerRoute::SubgraphReplan,
            PlanDefectRoute::PlanRepair,
            Some(RepairTargetKind::Subgraph),
        ),
        (
            BlockerRoute::StoryAmendment,
            PlanDefectRoute::StoryAmendment,
            None,
        ),
        (
            BlockerRoute::DesignAmendment,
            PlanDefectRoute::DesignAmendment,
            None,
        ),
        (
            BlockerRoute::OperationalGate,
            PlanDefectRoute::OperationalGate,
            None,
        ),
    ];

    for (route, expected_route, required_target_kind) in cases {
        assert_eq!(
            normalize_blocker_route(route),
            NormalizedPlanDefectRoute {
                route: expected_route,
                required_target_kind,
            }
        );
    }
}

#[test]
fn plan_repair_finding_validation_requires_consistent_route_class_and_target() {
    let valid_targets = [
        (
            PlanDefectClass::CurrentWorkItemInvalid,
            RepairTargetKind::CurrentWorkItem,
        ),
        (
            PlanDefectClass::UpstreamContractInvalid,
            RepairTargetKind::UpstreamWorkItem,
        ),
        (
            PlanDefectClass::DependencyGraphInvalid,
            RepairTargetKind::Subgraph,
        ),
    ];
    for (class, kind) in valid_targets {
        let finding = finding_fixture(
            class,
            Some(target_fixture(
                kind,
                &["logical_work_item_0001"],
                &["work_item_revision_0001"],
            )),
            &[],
            &[],
        );
        assert!(finding.validate().is_ok());
    }

    let implementation = finding_fixture(PlanDefectClass::ImplementationDefect, None, &[], &[]);
    assert!(implementation.validate().is_ok());

    let mut wrong_route = implementation.clone();
    wrong_route.recommended_route = PlanDefectRoute::PlanRepair;
    assert!(matches!(
        wrong_route.validate(),
        Err(PlanRepairError::InvalidFinding(_))
    ));

    let wrong_target = finding_fixture(
        PlanDefectClass::UpstreamContractInvalid,
        Some(target_fixture(
            RepairTargetKind::CurrentWorkItem,
            &["logical_work_item_0001"],
            &["work_item_revision_0001"],
        )),
        &[],
        &[],
    );
    assert!(matches!(
        wrong_target.validate(),
        Err(PlanRepairError::InvalidRepairTarget(_))
    ));

    let unexpected_target = finding_fixture(
        PlanDefectClass::ImplementationDefect,
        Some(target_fixture(
            RepairTargetKind::CurrentWorkItem,
            &["logical_work_item_0001"],
            &["work_item_revision_0001"],
        )),
        &[],
        &[],
    );
    assert!(matches!(
        unexpected_target.validate(),
        Err(PlanRepairError::InvalidRepairTarget(_))
    ));
}

#[test]
fn plan_repair_finding_serde_rejects_unknown_fields_and_routes() {
    let finding = finding_fixture(
        PlanDefectClass::CurrentWorkItemInvalid,
        Some(target_fixture(
            RepairTargetKind::CurrentWorkItem,
            &["logical_work_item_0001"],
            &["work_item_revision_0001"],
        )),
        &[],
        &[],
    );
    let mut unknown_field = serde_json::to_value(&finding).unwrap();
    unknown_field["unexpected"] = json!(true);
    assert!(serde_json::from_value::<PlanDefectFinding>(unknown_field).is_err());

    let mut unknown_route = serde_json::to_value(&finding).unwrap();
    unknown_route["recommended_route"] = json!("unknown_route");
    assert!(serde_json::from_value::<PlanDefectFinding>(unknown_route).is_err());

    let mut unknown_target_field = serde_json::to_value(&finding.repair_target).unwrap();
    unknown_target_field["unexpected"] = json!(true);
    assert!(serde_json::from_value::<RepairTarget>(unknown_target_field).is_err());
}

#[test]
fn plan_repair_fingerprint_is_stable_for_reordered_refs_and_target_ids() {
    let left = finding_fixture(
        PlanDefectClass::UpstreamContractInvalid,
        Some(target_fixture(
            RepairTargetKind::UpstreamWorkItem,
            &["logical_b", "logical_a"],
            &["revision_b", "revision_a"],
        )),
        &["contract_b", "contract_a"],
        &["capability_b", "capability_a"],
    );
    let right = finding_fixture(
        PlanDefectClass::UpstreamContractInvalid,
        Some(target_fixture(
            RepairTargetKind::UpstreamWorkItem,
            &["logical_a", "logical_b"],
            &["revision_a", "revision_b"],
        )),
        &["contract_a", "contract_b"],
        &["capability_a", "capability_b"],
    );

    let left_fingerprint = plan_defect_fingerprint("plan_revision_0001", &left);
    assert_eq!(
        left_fingerprint,
        plan_defect_fingerprint("plan_revision_0001", &right)
    );
    assert_eq!(left_fingerprint.len(), 64);
    assert!(
        left_fingerprint
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    );
}

#[test]
fn plan_repair_fingerprint_treats_duplicate_refs_as_set_members() {
    let duplicated = finding_fixture(
        PlanDefectClass::DependencyGraphInvalid,
        Some(target_fixture(
            RepairTargetKind::Subgraph,
            &["logical_a", "logical_a"],
            &["revision_a", "revision_a"],
        )),
        &["contract_a", "contract_a"],
        &["capability_a", "capability_a"],
    );
    let unique = finding_fixture(
        PlanDefectClass::DependencyGraphInvalid,
        Some(target_fixture(
            RepairTargetKind::Subgraph,
            &["logical_a"],
            &["revision_a"],
        )),
        &["contract_a"],
        &["capability_a"],
    );

    assert_eq!(
        plan_defect_fingerprint("plan_revision_0001", &duplicated),
        plan_defect_fingerprint("plan_revision_0001", &unique)
    );
}

#[test]
fn plan_repair_fingerprint_normalizes_reason_code() {
    let canonical = finding_fixture(
        PlanDefectClass::DependencyGraphInvalid,
        Some(target_fixture(
            RepairTargetKind::Subgraph,
            &["logical_a"],
            &["revision_a"],
        )),
        &[],
        &[],
    );
    let mut differently_formatted = canonical.clone();
    differently_formatted.reason_code = "  CONTRACT_INVALID  ".to_string();

    assert_eq!(
        plan_defect_fingerprint("plan_revision_0001", &canonical),
        plan_defect_fingerprint("plan_revision_0001", &differently_formatted)
    );
}

#[test]
fn plan_repair_fingerprint_changes_for_different_base_or_target_semantics() {
    let current = finding_fixture(
        PlanDefectClass::CurrentWorkItemInvalid,
        Some(target_fixture(
            RepairTargetKind::CurrentWorkItem,
            &["logical_a"],
            &["revision_a"],
        )),
        &["contract_a"],
        &["capability_a"],
    );
    let changed_target = finding_fixture(
        PlanDefectClass::CurrentWorkItemInvalid,
        Some(target_fixture(
            RepairTargetKind::CurrentWorkItem,
            &["logical_b"],
            &["revision_b"],
        )),
        &["contract_a"],
        &["capability_a"],
    );

    assert_ne!(
        plan_defect_fingerprint("plan_revision_0001", &current),
        plan_defect_fingerprint("plan_revision_0002", &current)
    );
    assert_ne!(
        plan_defect_fingerprint("plan_revision_0001", &current),
        plan_defect_fingerprint("plan_revision_0001", &changed_target)
    );
}

#[test]
fn plan_repair_fingerprint_ignores_non_identity_evidence() {
    let mut left = finding_fixture(
        PlanDefectClass::CurrentWorkItemInvalid,
        Some(target_fixture(
            RepairTargetKind::CurrentWorkItem,
            &["logical_a"],
            &["revision_a"],
        )),
        &["contract_a"],
        &["capability_a"],
    );
    let mut right = left.clone();
    right.message = "different presentation".to_string();
    right.evidence.push(PlanDefectEvidence {
        kind: "test_failure".to_string(),
        source_ref: "test_0001".to_string(),
        message: "failed".to_string(),
    });
    right.confidence = PlanDefectConfidence::Medium;
    left.severity = PlanDefectSeverity::Warning;

    assert_eq!(
        plan_defect_fingerprint("plan_revision_0001", &left),
        plan_defect_fingerprint("plan_revision_0001", &right)
    );
}
