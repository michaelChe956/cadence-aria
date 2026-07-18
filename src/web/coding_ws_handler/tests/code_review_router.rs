use super::*;
use crate::product::plan_repair::PlanDefectConfidence;
use crate::product::work_item_contract::{
    BlockerRoute, BlockerRule, ContractCompatibilityPolicy, RequiredInputContract,
    WorkItemWritePolicy,
};
use crate::product::work_item_projection::ReviewerWorkItemProjection;

#[test]
fn coding_plan_repair_router_never_sends_plan_defect_to_coder() {
    let report =
        code_review_report_with(ReviewVerdict::Blocked, vec![upstream_plan_defect_finding()]);

    assert_eq!(
        code_review_flow_decision(&report, &reviewer_projection_fixture()),
        CodeReviewFlowDecision::StartPlanRepair
    );
}

#[test]
fn coding_plan_repair_router_invalid_plan_finding_stops_for_human_triage() {
    let report =
        code_review_report_with(ReviewVerdict::Blocked, vec![upstream_plan_defect_finding()]);
    let mut projection = reviewer_projection_fixture();
    projection.blocker_routing.clear();

    assert_eq!(
        code_review_flow_decision(&report, &projection),
        CodeReviewFlowDecision::StopForHumanTriage
    );
}

#[test]
fn coding_plan_repair_router_preserves_implementation_and_approve_behavior() {
    let projection = reviewer_projection_fixture();
    assert_eq!(
        code_review_flow_decision(
            &code_review_report_with(
                ReviewVerdict::RequestChanges,
                vec![implementation_finding()],
            ),
            &projection,
        ),
        CodeReviewFlowDecision::RunCoderFix
    );
    assert_eq!(
        code_review_flow_decision(
            &code_review_report_with(ReviewVerdict::Approve, Vec::new()),
            &projection,
        ),
        CodeReviewFlowDecision::ContinueAfterApprove
    );
}

#[test]
fn coding_plan_repair_router_rejects_polluted_implementation_findings() {
    let projection = reviewer_projection_fixture();
    let mut with_plan_target = implementation_finding();
    with_plan_target.repair_target = Some(crate::product::models::RepairTarget {
        kind: crate::product::models::RepairTargetKind::CurrentWorkItem,
        logical_work_item_ids: vec!["work_item_0001".to_string()],
        work_item_revision_ids: vec!["work_item_revision_0001".to_string()],
    });
    let mut with_wrong_route = implementation_finding();
    with_wrong_route.recommended_route = crate::product::models::PlanDefectRoute::PlanRepair;
    let mut with_typed_evidence = implementation_finding();
    with_typed_evidence.plan_defect_evidence = vec![crate::product::models::PlanDefectEvidence {
        kind: "projection".to_string(),
        source_ref: "work_item_revision_0001".to_string(),
        message: "typed plan evidence must not be attached to implementation defects".to_string(),
    }];
    with_typed_evidence.confidence = Some(PlanDefectConfidence::High);

    for (case, finding) in [
        ("plan target", with_plan_target),
        ("wrong route", with_wrong_route),
        ("typed evidence and confidence", with_typed_evidence),
    ] {
        assert_eq!(
            code_review_flow_decision(
                &code_review_report_with(ReviewVerdict::RequestChanges, vec![finding]),
                &projection,
            ),
            CodeReviewFlowDecision::StopForHumanTriage,
            "{case} must fail closed",
        );
    }
}

fn upstream_plan_defect_finding() -> ReviewFinding {
    ReviewFinding {
        severity: FindingSeverity::Error,
        file_path: None,
        line: None,
        message: "missing finalization_failure".to_string(),
        required_action: None,
        source_stage: CodingExecutionStage::CodeReview,
        evidence: Vec::new(),
        plan_defect_evidence: Vec::new(),
        related_requirements: Vec::new(),
        related_design_constraints: Vec::new(),
        related_work_item_tasks: Vec::new(),
        defect_class: crate::product::models::PlanDefectClass::UpstreamContractInvalid,
        reason_code: Some("upstream_contract_capability_missing".to_string()),
        contract_refs: vec!["repository_initialization_finalization".to_string()],
        capability_refs: vec!["finalization_failure".to_string()],
        repair_target: Some(crate::product::models::RepairTarget {
            kind: crate::product::models::RepairTargetKind::UpstreamWorkItem,
            logical_work_item_ids: vec!["wi_core".to_string()],
            work_item_revision_ids: vec!["work_item_revision_0001".to_string()],
        }),
        recommended_route: crate::product::models::PlanDefectRoute::PlanRepair,
        confidence: Some(PlanDefectConfidence::High),
    }
}

pub(super) fn implementation_finding() -> ReviewFinding {
    ReviewFinding {
        severity: FindingSeverity::Error,
        file_path: Some("src/lib.rs".to_string()),
        line: Some(42),
        message: "missing validation".to_string(),
        required_action: Some("add validation".to_string()),
        source_stage: CodingExecutionStage::CodeReview,
        evidence: Vec::new(),
        plan_defect_evidence: Vec::new(),
        related_requirements: Vec::new(),
        related_design_constraints: Vec::new(),
        related_work_item_tasks: Vec::new(),
        defect_class: crate::product::models::PlanDefectClass::ImplementationDefect,
        reason_code: None,
        contract_refs: Vec::new(),
        capability_refs: Vec::new(),
        repair_target: None,
        recommended_route: crate::product::models::PlanDefectRoute::CoderRework,
        confidence: None,
    }
}

pub(super) fn reviewer_projection_fixture() -> ReviewerWorkItemProjection {
    ReviewerWorkItemProjection {
        work_item_revision_id: "work_item_revision_0002".to_string(),
        criterion_refs: Vec::new(),
        requirement_matrix: Vec::new(),
        scope_policy: WorkItemWritePolicy {
            exclusive_scopes: Vec::new(),
            forbidden_scopes: Vec::new(),
        },
        input_contract_checks: vec![RequiredInputContract {
            contract_id: "repository_initialization_finalization".to_string(),
            provider_logical_work_item_id: "wi_core".to_string(),
            required_capabilities: vec!["finalization_failure".to_string()],
            compatibility_policy: ContractCompatibilityPolicy::RequireAll,
        }],
        output_contract_checks: Vec::new(),
        verification_evidence_rules: Vec::new(),
        blocker_routing: vec![BlockerRule {
            reason_code: "upstream_contract_capability_missing".to_string(),
            route: BlockerRoute::PlanRepairUpstream,
            target_contract_refs: vec!["repository_initialization_finalization".to_string()],
        }],
    }
}
