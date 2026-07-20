use std::collections::BTreeMap;
use std::path::Path;

use super::recovery::{enter_prepared_awaiting, fixture_error, unique_repair_request};
use super::seed::{fixture_paths, registration_contract, start_plan_repair_finding, status_name};
use super::{PlanRepairFixtureError, PlanRepairFixtureWaiting};
use crate::product::coding_attempt_store::CodingAttemptStore;
use crate::product::coding_models::{
    CodingAttemptStatus, CodingExecutionStage, FindingSeverity, ReviewFinding,
};
use crate::product::models::{
    PlanDefectClass, PlanDefectEvidence, PlanDefectRoute, PlanRepairReviewAttestation,
    PlanRevisionReason, RepairTarget, RepairTargetKind, WorkItemDraftRevision,
};
use crate::product::plan_repair::{
    PlanDefectConfidence, PlanRepairEngine, PreparedPlanAmendment, SubgraphReplanRequest,
};
use crate::product::work_item_contract::CanonicalWorkItemContract;
use crate::product::work_item_revision_store::WorkItemRevisionStore;
use crate::product::workspace_engine::WorkspaceEngine;

const PROJECT_ID: &str = "project_0001";
const ISSUE_ID: &str = "issue_plan_0001";
const PLAN_ID: &str = "work_item_plan_0001";
const CREATED_AT: &str = "2026-07-20T00:00:10Z";

pub(super) async fn route_dependency_graph_invalid(
    root: &Path,
) -> Result<PlanRepairFixtureWaiting, PlanRepairFixtureError> {
    let store = CodingAttemptStore::new(fixture_paths(root));
    let attempt = store
        .get_attempt_for_work_item_group(PROJECT_ID, ISSUE_ID, PLAN_ID)
        .map_err(fixture_error)?
        .ok_or_else(|| PlanRepairFixtureError::not_implemented("attempt_missing"))?;
    if attempt.status != CodingAttemptStatus::AwaitingPlanAmendment {
        start_plan_repair_finding(
            root,
            "code_review_report_0001_finding_topology_0001",
            dependency_graph_finding("code_review_report_0001_finding_topology_0001"),
        )
        .await?;
    }
    let waiting = store
        .get_attempt(PROJECT_ID, ISSUE_ID, &attempt.id)
        .map_err(fixture_error)?;
    let unit = store
        .get_active_coding_unit(PROJECT_ID, ISSUE_ID, &attempt.id)
        .map_err(fixture_error)?
        .ok_or_else(|| PlanRepairFixtureError::not_implemented("active_unit_missing"))?;
    let run = store.get_active_unit_run(&waiting).map_err(fixture_error)?;
    Ok(PlanRepairFixtureWaiting {
        attempt_status: status_name(&waiting.status).to_string(),
        active_logical_work_item_id: unit.logical_work_item_id,
        active_unit_rework_count: run.unit_rework_count,
    })
}

pub(super) async fn prepare_topology_review_and_awaiting(
    root: &Path,
) -> Result<
    (
        PreparedPlanAmendment,
        PlanRepairReviewAttestation,
        WorkspaceEngine,
    ),
    PlanRepairFixtureError,
> {
    let revision_store = WorkItemRevisionStore::new(fixture_paths(root));
    let plan = revision_store
        .get_plan_lineage(PROJECT_ID, ISSUE_ID, PLAN_ID)
        .map_err(fixture_error)?;
    let request = unique_repair_request(&revision_store, &plan)?;
    let contract = registration_topology_contract();
    let draft = WorkItemDraftRevision {
        id: "work_item_draft_revision_wi_registration_0002".to_string(),
        logical_work_item_id: "wi_registration".to_string(),
        revision_no: 2,
        supersedes: Some("work_item_draft_revision_wi_registration_0001".to_string()),
        revision_reason: PlanRevisionReason::SubgraphReplan,
        canonical_contract_candidate: contract.clone(),
        trigger_repair_request_id: Some(request.id.clone()),
        created_at: CREATED_AT.to_string(),
    };
    revision_store
        .put_draft_revision(&plan, &draft)
        .map_err(fixture_error)?;
    let subgraph_request = SubgraphReplanRequest {
        plan_id: plan.id.clone(),
        base_plan_revision_id: request.base_plan_revision_id.clone(),
        repair_request_id: request.id.clone(),
        changed_logical_work_item_ids: vec!["wi_registration".to_string()],
        replacement_contracts: vec![contract],
        replacement_mapping: BTreeMap::from([(
            "wi_registration".to_string(),
            vec!["wi_registration".to_string()],
        )]),
        story_spec_refs_changed: false,
        design_spec_refs_changed: false,
    };
    let engine = PlanRepairEngine::new(revision_store.clone(), plan.clone())
        .with_candidate_drafts(vec![draft])
        .with_subgraph_replan_request(subgraph_request)
        .with_created_at(CREATED_AT);
    let prepared = engine.prepare_amendment(&request).map_err(fixture_error)?;
    engine.persist_candidate(&prepared).map_err(fixture_error)?;
    enter_prepared_awaiting(root, revision_store, plan, request, prepared).await
}

fn dependency_graph_finding(finding_id: &str) -> ReviewFinding {
    ReviewFinding {
        severity: FindingSeverity::Error,
        file_path: Some("src/registration.rs".to_string()),
        line: Some(1),
        message: "workflow dependency topology requires replanning".to_string(),
        required_action: Some("replan the affected dependency subgraph".to_string()),
        source_stage: CodingExecutionStage::CodeReview,
        evidence: vec!["src/registration.rs:1".to_string()],
        plan_defect_evidence: vec![PlanDefectEvidence {
            kind: "review_finding".to_string(),
            source_ref: format!("code_review_report_0001#{finding_id}"),
            message: "dependency graph contract topology is invalid".to_string(),
        }],
        related_requirements: Vec::new(),
        related_design_constraints: Vec::new(),
        related_work_item_tasks: Vec::new(),
        defect_class: PlanDefectClass::DependencyGraphInvalid,
        reason_code: Some("dependency_graph_invalid".to_string()),
        contract_refs: vec!["contract.workflow".to_string()],
        capability_refs: vec!["finalization_failure".to_string()],
        repair_target: Some(RepairTarget {
            kind: RepairTargetKind::Subgraph,
            logical_work_item_ids: vec!["wi_registration".to_string()],
            work_item_revision_ids: vec!["work_item_revision_wi_registration_0001".to_string()],
        }),
        recommended_route: PlanDefectRoute::PlanRepair,
        confidence: Some(PlanDefectConfidence::High),
    }
}

fn registration_topology_contract() -> CanonicalWorkItemContract {
    let mut contract = registration_contract();
    contract.input_contracts[0].required_capabilities =
        vec!["workflow_explicit_completion".to_string()];
    contract
}
