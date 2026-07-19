use std::collections::HashSet;
use std::sync::Arc;

use super::*;
use crate::product::checkpoint_store::CheckpointStore;
use crate::product::coding_models::CodingUnitRun;
use crate::product::models::{
    PlanDefectRoute, PlanRepairRequest, PlanRepairRequestStatus, PlanRepairSessionSnapshotDto,
    WorkItemPlanLineage, WorkspaceSessionLink, WorkspaceSessionStatus, WorkspaceType,
};
use crate::product::plan_repair::{PlanDefectFinding, PlanDefectSeverity, plan_defect_fingerprint};
use crate::product::work_item_projection::ReviewerWorkItemProjection;
use crate::product::work_item_revision_store::WorkItemRevisionStore;
use crate::product::workspace_engine::{EngineEvent, WorkspaceEngine, WorkspaceSession};

impl CodingWorkspaceEngine {
    pub(crate) async fn start_plan_repair_from_review(
        &self,
        attempt: &CodingExecutionAttempt,
        review_id: &str,
        finding_id: &str,
        finding: &ReviewFinding,
        reviewer_projection: &ReviewerWorkItemProjection,
    ) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
        let current = self.store.validate_attempt_lineage(attempt)?;
        let trigger_run = self.store.get_active_unit_run(&current)?;
        self.start_plan_repair_from_finding(
            &current,
            review_id,
            finding_id,
            finding,
            reviewer_projection,
            &trigger_run,
        )
        .await
    }

    pub(crate) async fn start_plan_repair_from_internal_review(
        &self,
        attempt: &CodingExecutionAttempt,
        review: &InternalPrReview,
    ) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
        let current = self.store.validate_attempt_lineage(attempt)?;
        let mut findings = review.findings.iter().enumerate().filter(|(_, finding)| {
            matches!(
                finding.defect_class,
                crate::product::models::PlanDefectClass::CurrentWorkItemInvalid
                    | crate::product::models::PlanDefectClass::UpstreamContractInvalid
                    | crate::product::models::PlanDefectClass::DependencyGraphInvalid
            )
        });
        let (finding_index, finding) = findings.next().ok_or_else(|| {
            plan_repair_start_error("group review Plan Repair finding is missing".to_string())
        })?;
        if findings.next().is_some() {
            return Err(plan_repair_start_error(
                "group review Plan Repair finding is ambiguous".to_string(),
            ));
        }
        let bindings = self.authoritative_group_reviewer_bindings(&current)?;
        let trigger = unique_authoritative_group_reviewer_binding(finding, &bindings)?;
        self.start_plan_repair_from_finding(
            &current,
            &review.id,
            &format!("{}_finding_{:04}", review.id, finding_index + 1),
            finding,
            &trigger.projection_binding.projection,
            &trigger.run,
        )
        .await
    }

    async fn start_plan_repair_from_finding(
        &self,
        attempt: &CodingExecutionAttempt,
        review_id: &str,
        finding_id: &str,
        finding: &ReviewFinding,
        reviewer_projection: &ReviewerWorkItemProjection,
        trigger_run: &CodingUnitRun,
    ) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
        validate_plan_defect_finding(finding, reviewer_projection)
            .map_err(|error| plan_repair_start_error(format!("invalid finding: {error:?}")))?;
        if finding.recommended_route != PlanDefectRoute::PlanRepair {
            return Err(plan_repair_start_error(
                "finding does not route to Plan Repair".to_string(),
            ));
        }
        let current = self.store.validate_attempt_lineage(attempt)?;
        let latest = self
            .store
            .list_coding_unit_runs(&current, &trigger_run.unit_id)?
            .into_iter()
            .max_by_key(|run| run.execution_no)
            .ok_or_else(|| plan_repair_start_error("trigger UnitRun is missing".to_string()))?;
        if latest.id != trigger_run.id
            || reviewer_projection.work_item_revision_id != trigger_run.work_item_revision_id
        {
            return Err(plan_repair_start_error(
                "trigger UnitRun projection binding mismatch".to_string(),
            ));
        }
        let plan_id = current.work_item_group_id.as_deref().ok_or_else(|| {
            plan_repair_start_error("Plan Repair requires a WorkItemGroup attempt".to_string())
        })?;
        let revision_store = WorkItemRevisionStore::new(self.store.paths());
        let plan =
            revision_store.get_plan_lineage(&current.project_id, &current.issue_id, plan_id)?;
        let base_plan_revision_id = plan.active_revision_id.clone().ok_or_else(|| {
            plan_repair_start_error("active plan revision is missing".to_string())
        })?;
        let canonical_finding = canonical_plan_defect_finding(finding_id, finding)?;
        let fingerprint = plan_defect_fingerprint(&base_plan_revision_id, &canonical_finding);
        let open_requests = revision_store.list_open_repair_requests(&plan)?;
        let existing = open_requests
            .iter()
            .find(|request| request.fingerprint == fingerprint)
            .cloned();
        let now = Utc::now().to_rfc3339();
        let request = PlanRepairRequest {
            id: existing
                .as_ref()
                .map(|request| request.id.clone())
                .unwrap_or(allocate_plan_repair_request_id(&revision_store, &plan)?),
            plan_id: plan.id.clone(),
            base_plan_revision_id,
            trigger_attempt_id: current.id.clone(),
            trigger_unit_run_id: trigger_run.id.clone(),
            trigger_review_id: Some(review_id.to_string()),
            trigger_finding_id: finding_id.to_string(),
            amendment_id: existing
                .as_ref()
                .and_then(|request| request.amendment_id.clone()),
            defect_class: canonical_finding.defect_class.clone(),
            reason_code: canonical_finding.reason_code.clone(),
            repair_target: canonical_finding.repair_target.clone().ok_or_else(|| {
                plan_repair_start_error("Plan Repair finding target is missing".to_string())
            })?,
            contract_refs: canonical_finding.contract_refs.clone(),
            capability_refs: canonical_finding.capability_refs.clone(),
            evidence: canonical_finding.evidence.clone(),
            fingerprint,
            status: PlanRepairRequestStatus::Open,
            created_at: existing
                .as_ref()
                .map(|request| request.created_at.clone())
                .unwrap_or_else(|| now.clone()),
            updated_at: now,
        };
        let lifecycle = LifecycleStore::new(self.store.paths());
        let parent = unique_plan_repair_parent_session(&lifecycle, &current, &plan.id)?;
        let checkpoint_store = Arc::new(CheckpointStore::new(
            self.store
                .paths()
                .issue_lifecycle_root(&current.project_id, &current.issue_id),
        ));
        let (workspace_tx, _workspace_rx) = mpsc::channel::<EngineEvent>(8);
        let mut workspace_engine = WorkspaceEngine::new_persistent(
            checkpoint_store,
            lifecycle.clone(),
            workspace_tx,
            WorkspaceSession::from_record(parent),
        );
        let child = workspace_engine
            .start_plan_repair(request.clone())
            .await
            .map_err(|error| plan_repair_start_error(format!("{error:?}")))?;
        let session_link = lifecycle.get_session_link(&child.id)?;
        let snapshot = lifecycle
            .load_plan_repair_session_state(&current.project_id, &current.issue_id, &child.id)?
            .ok_or_else(|| {
                plan_repair_start_error("Plan Repair child snapshot is missing".to_string())
            })?;
        validate_linked_plan_repair_snapshot(
            &current,
            &snapshot,
            &session_link,
            &request,
            existing.as_ref(),
        )?;
        let reconciliation = self.store.reconcile_linked_plan_repair_pause(&current)?;
        let updated = reconciliation.attempt;
        if reconciliation.timeline_created {
            let node = reconciliation.timeline_node.ok_or_else(|| {
                plan_repair_start_error(
                    "Plan Repair timeline reconciliation is missing".to_string(),
                )
            })?;
            let _ = self
                .event_tx
                .send(CodingWsOutMessage::CodingTimelineNodeCreated { node })
                .await;
        }
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::PlanRepairRequired {
                request: Box::new(snapshot.request),
                session_link: Some(session_link),
            })
            .await;
        Ok(updated)
    }
}

fn canonical_plan_defect_finding(
    finding_id: &str,
    finding: &ReviewFinding,
) -> Result<PlanDefectFinding, CodingWorkspaceEngineError> {
    let severity = match finding.severity {
        FindingSeverity::Error => PlanDefectSeverity::Error,
        FindingSeverity::Warning => PlanDefectSeverity::Warning,
        FindingSeverity::Info => {
            return Err(plan_repair_start_error(
                "Plan Repair finding severity cannot be info".to_string(),
            ));
        }
    };
    Ok(PlanDefectFinding {
        finding_id: finding_id.to_string(),
        severity,
        defect_class: finding.defect_class.clone(),
        reason_code: finding.reason_code.clone().ok_or_else(|| {
            plan_repair_start_error("Plan Repair finding reason code is missing".to_string())
        })?,
        message: finding.message.clone(),
        evidence: finding.plan_defect_evidence.clone(),
        contract_refs: finding.contract_refs.clone(),
        capability_refs: finding.capability_refs.clone(),
        repair_target: finding.repair_target.clone(),
        recommended_route: finding.recommended_route.clone(),
        confidence: finding.confidence.clone().ok_or_else(|| {
            plan_repair_start_error("Plan Repair finding confidence is missing".to_string())
        })?,
    })
}

fn allocate_plan_repair_request_id(
    revision_store: &WorkItemRevisionStore,
    plan: &WorkItemPlanLineage,
) -> Result<String, CodingWorkspaceEngineError> {
    for sequence in 1..=10_000 {
        let id = format!("plan_repair_request_{sequence:04}");
        match revision_store.get_repair_request(plan, &id) {
            Err(ProductStoreError::NotFound { .. }) => return Ok(id),
            Ok(_) => continue,
            Err(error) => return Err(error.into()),
        }
    }
    Err(plan_repair_start_error(
        "Plan Repair request id space exhausted".to_string(),
    ))
}

fn unique_plan_repair_parent_session(
    lifecycle: &LifecycleStore,
    attempt: &CodingExecutionAttempt,
    plan_id: &str,
) -> Result<crate::product::models::WorkspaceSessionRecord, CodingWorkspaceEngineError> {
    let child_ids = lifecycle
        .list_session_links(&attempt.project_id, &attempt.issue_id)?
        .into_iter()
        .map(|link| link.child_session_id)
        .collect::<HashSet<_>>();
    let mut candidates = lifecycle
        .list_workspace_sessions(&attempt.project_id, &attempt.issue_id)?
        .into_iter()
        .filter(|session| {
            session.entity_id == plan_id
                && session.workspace_type == WorkspaceType::WorkItemPlan
                && session.status != WorkspaceSessionStatus::Terminated
                && !child_ids.contains(&session.id)
        });
    let parent = candidates.next().ok_or_else(|| {
        plan_repair_start_error("Plan Repair parent WorkItemPlan workspace is missing".to_string())
    })?;
    if candidates.next().is_some() {
        return Err(plan_repair_start_error(
            "Plan Repair parent WorkItemPlan workspace is ambiguous".to_string(),
        ));
    }
    Ok(parent)
}

fn validate_linked_plan_repair_snapshot(
    attempt: &CodingExecutionAttempt,
    snapshot: &PlanRepairSessionSnapshotDto,
    session_link: &WorkspaceSessionLink,
    requested: &PlanRepairRequest,
    existing: Option<&PlanRepairRequest>,
) -> Result<(), CodingWorkspaceEngineError> {
    if snapshot.link != *session_link
        || snapshot.request.id != session_link.trigger.repair_request_id
        || snapshot.request.id != requested.id
        || snapshot.request.fingerprint != requested.fingerprint
        || snapshot.request.trigger_attempt_id != attempt.id
        || snapshot.request.trigger_unit_run_id != requested.trigger_unit_run_id
        || snapshot.request.trigger_unit_run_id != session_link.trigger.unit_run_id
        || snapshot.request.fingerprint != session_link.trigger.fingerprint
        || existing.is_some_and(|expected| {
            snapshot.request.id != expected.id
                || snapshot.request.trigger_attempt_id != expected.trigger_attempt_id
                || snapshot.request.trigger_unit_run_id != expected.trigger_unit_run_id
                || snapshot.request.trigger_review_id != expected.trigger_review_id
                || snapshot.request.trigger_finding_id != expected.trigger_finding_id
                || snapshot.request.base_plan_revision_id != expected.base_plan_revision_id
                || snapshot.request.amendment_id != expected.amendment_id
                || snapshot.request.fingerprint != expected.fingerprint
        })
        || (existing.is_none()
            && (snapshot.request.trigger_review_id != requested.trigger_review_id
                || snapshot.request.trigger_finding_id != requested.trigger_finding_id))
    {
        return Err(plan_repair_start_error(
            "Plan Repair linked snapshot identity mismatch".to_string(),
        ));
    }
    Ok(())
}

fn plan_repair_start_error(message: String) -> CodingWorkspaceEngineError {
    CodingWorkspaceEngineError::ProviderStream(format!("plan_repair_start_failed: {message}"))
}
