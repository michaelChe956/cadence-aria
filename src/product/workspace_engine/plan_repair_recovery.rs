use chrono::Utc;

use crate::product::json_store::ProductStoreError;
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::models::{
    PlanDefectClass, PlanRepairRequest, PlanRepairRequestStatus, PlanRepairSessionSnapshotDto,
    PlanRepairSessionStage, RepairTarget, RepairTargetKind, WorkspaceReturnContext,
    WorkspaceSessionLink, WorkspaceSessionLinkTrigger, WorkspaceSessionRecord,
    WorkspaceSessionRelation, WorkspaceSessionStatus, WorkspaceType,
};
use crate::product::plan_repair::PlanRepairError;
use crate::product::work_item_revision_store::WorkItemRevisionStore;
use crate::web::workspace_ws_types::{
    ProviderConfigSnapshot, TimelineNode, TimelineNodeStatus, TimelineNodeType,
    WorkspaceStage as WsWorkspaceStage,
};

use super::{
    WorkspaceSession, awaiting_confirmation_package_from_snapshot,
    validate_persisted_awaiting_confirmation_package,
};

pub(crate) fn initial_plan_repair_timeline(session: &WorkspaceSessionRecord) -> Vec<TimelineNode> {
    vec![TimelineNode {
        node_id: "timeline_node_001".to_string(),
        node_type: TimelineNodeType::PlanRepairAuthoringRevision,
        agent: Some(session.author_provider.clone()),
        stage: WsWorkspaceStage::Running,
        round: None,
        status: TimelineNodeStatus::Active,
        title: "生成 Work Item 修订".to_string(),
        summary: Some("基于 Plan Defect 生成修订".to_string()),
        started_at: Utc::now().to_rfc3339(),
        completed_at: None,
        duration_ms: None,
        artifact_ref: None,
        provider_config_snapshot: ProviderConfigSnapshot {
            author: session.author_provider.clone(),
            reviewer: Some(session.reviewer_provider.clone()),
            review_rounds: session.review_rounds,
        },
        retry: None,
    }]
}

pub(crate) fn reconcile_plan_repair_child(
    lifecycle: &LifecycleStore,
    project_id: &str,
    issue_id: &str,
    request: &PlanRepairRequest,
    link: WorkspaceSessionLink,
    child: WorkspaceSessionRecord,
) -> Result<WorkspaceSessionRecord, PlanRepairError> {
    let existing_snapshot = lifecycle
        .load_plan_repair_session_state(project_id, issue_id, &child.id)
        .map_err(PlanRepairError::Store)?;
    if existing_snapshot
        .as_ref()
        .is_some_and(|snapshot| snapshot.link != link)
    {
        return Err(PlanRepairError::Store(
            ProductStoreError::IdentityMismatch {
                kind: "plan_repair_session_state",
                id: child.id,
            },
        ));
    }
    let persisted_timeline = lifecycle
        .load_timeline_nodes_for_issue_session(project_id, issue_id, &child.id)
        .map_err(PlanRepairError::Store)?;
    let timeline_nodes = if persisted_timeline.is_empty() {
        existing_snapshot
            .as_ref()
            .map(|snapshot| snapshot.timeline_nodes.clone())
            .filter(|nodes| !nodes.is_empty())
            .unwrap_or_else(|| initial_plan_repair_timeline(&child))
    } else {
        persisted_timeline
    };
    lifecycle
        .save_timeline_nodes(&child.id, &timeline_nodes)
        .map_err(PlanRepairError::Store)?;
    let mut snapshot = existing_snapshot.unwrap_or_else(|| PlanRepairSessionSnapshotDto {
        request: request.clone(),
        link: link.clone(),
        stage: PlanRepairSessionStage::AuthoringRevision,
        projection: None,
        amendment: None,
        validation: None,
        impact: None,
        plan_review: None,
        package_identity: None,
        candidate_package_artifact_id: None,
        impact_scope_review: None,
        timeline_nodes: timeline_nodes.clone(),
        error: None,
    });
    snapshot.request = request.clone();
    snapshot.link = link;
    snapshot.timeline_nodes = timeline_nodes;
    lifecycle
        .save_plan_repair_session_state(project_id, issue_id, &child.id, &snapshot)
        .map_err(PlanRepairError::Store)?;
    let status = match snapshot.stage {
        PlanRepairSessionStage::AwaitingConfirmation
        | PlanRepairSessionStage::Published
        | PlanRepairSessionStage::AmendmentConflict
        | PlanRepairSessionStage::AmendmentApplyFailed => WorkspaceSessionStatus::WaitingForHuman,
        PlanRepairSessionStage::Completed | PlanRepairSessionStage::Failed => {
            WorkspaceSessionStatus::Terminated
        }
        PlanRepairSessionStage::Triaging
        | PlanRepairSessionStage::AuthoringRevision
        | PlanRepairSessionStage::ValidatingContract
        | PlanRepairSessionStage::GeneratingProjections
        | PlanRepairSessionStage::PlanReview
        | PlanRepairSessionStage::ApplyingAmendment => WorkspaceSessionStatus::Running,
    };
    lifecycle
        .update_workspace_session_status(&child.id, status)
        .map_err(PlanRepairError::Store)
}

pub(crate) fn load_plan_repair_snapshot_fail_closed(
    lifecycle: &LifecycleStore,
    session: &WorkspaceSession,
    timeline_nodes: &mut Vec<TimelineNode>,
    transition_recovery_error: Option<String>,
) -> Option<PlanRepairSessionSnapshotDto> {
    let session_id_signals_repair = session
        .session_id
        .starts_with("workspace_session_plan_amendment_");
    let (link, link_error) = match lifecycle.get_session_link(&session.session_id) {
        Ok(link) => (Some(link), None),
        Err(error) => (None, Some(format!("plan repair link load failed: {error}"))),
    };
    let (snapshot, snapshot_error) = match lifecycle.load_plan_repair_session_state(
        &session.project_id,
        &session.issue_id,
        &session.session_id,
    ) {
        Ok(snapshot) => (snapshot, None),
        Err(error) => (
            None,
            Some(format!("plan repair snapshot load failed: {error}")),
        ),
    };
    let repair_expected = session_id_signals_repair
        || link
            .as_ref()
            .is_some_and(|link| link.relation == WorkspaceSessionRelation::PlanRepair)
        || snapshot.is_some();
    if !repair_expected {
        return None;
    }

    let recovery_error = transition_recovery_error
        .or_else(|| {
            link.as_ref().map_or_else(
                || Some(link_error.unwrap_or_else(|| "plan repair link missing".to_string())),
                |link| {
                    (link.relation != WorkspaceSessionRelation::PlanRepair)
                        .then(|| "plan repair link identity mismatch".to_string())
                },
            )
        })
        .or_else(|| {
            snapshot.as_ref().map_or_else(
                || {
                    Some(
                        snapshot_error
                            .unwrap_or_else(|| "plan repair linked snapshot missing".to_string()),
                    )
                },
                |_| None,
            )
        })
        .or_else(|| {
            validate_refresh_identity(
                lifecycle,
                session,
                link.as_ref().expect("repair link checked above"),
                snapshot.as_ref().expect("repair snapshot checked above"),
            )
            .err()
        });

    if let Some(error) = recovery_error {
        return Some(failed_recovery_snapshot(
            lifecycle,
            session,
            timeline_nodes,
            snapshot,
            link,
            error,
        ));
    }
    snapshot
}

fn validate_refresh_identity(
    lifecycle: &LifecycleStore,
    session: &WorkspaceSession,
    link: &WorkspaceSessionLink,
    snapshot: &PlanRepairSessionSnapshotDto,
) -> Result<(), String> {
    let request = &snapshot.request;
    let amendment_id = request
        .amendment_id
        .as_deref()
        .ok_or_else(|| "plan repair snapshot identity mismatch: amendment missing".to_string())?;
    let expected_route = format!(
        "/workbench/projects/{}/issues/{}/coding/{}",
        session.project_id, session.issue_id, request.trigger_attempt_id
    );
    if session.workspace_type != WorkspaceType::WorkItemPlan
        || session.entity_id != request.plan_id
        || link.parent_session_id != request.trigger_attempt_id
        || link.child_session_id != session.session_id
        || link.id != format!("workspace_session_link_{amendment_id}")
        || link.child_session_id != format!("workspace_session_{amendment_id}")
        || snapshot.link != *link
        || link.trigger.repair_request_id != request.id
        || link.trigger.amendment_id != amendment_id
        || link.trigger.fingerprint != request.fingerprint
        || link.trigger.base_plan_revision_id != request.base_plan_revision_id
        || link.trigger.attempt_id != request.trigger_attempt_id
        || link.trigger.unit_run_id != request.trigger_unit_run_id
        || link.trigger.review_id != request.trigger_review_id
        || link.trigger.finding_id != request.trigger_finding_id
        || link.return_context.original_attempt_id != request.trigger_attempt_id
        || link.return_context.original_unit_run_id != request.trigger_unit_run_id
        || link.return_context.timeline_anchor_id != request.trigger_finding_id
        || link.return_context.original_route != expected_route
    {
        return Err("plan repair snapshot identity mismatch".to_string());
    }
    let revision_store = WorkItemRevisionStore::new(lifecycle.app_paths());
    let plan = revision_store
        .get_plan_lineage(&session.project_id, &session.issue_id, &request.plan_id)
        .map_err(|error| format!("plan repair lineage load failed: {error}"))?;
    let stored_request = revision_store
        .get_repair_request(&plan, &request.id)
        .map_err(|error| format!("plan repair request load failed: {error}"))?;
    if stored_request != *request {
        return Err("plan repair request identity mismatch".to_string());
    }
    if snapshot.stage == PlanRepairSessionStage::AwaitingConfirmation {
        if request.status != PlanRepairRequestStatus::AwaitingConfirmation {
            return Err("plan repair awaiting request status mismatch".to_string());
        }
        let package = awaiting_confirmation_package_from_snapshot(snapshot)
            .map_err(|error| format!("plan repair awaiting package invalid: {error:?}"))?;
        validate_persisted_awaiting_confirmation_package(
            &revision_store,
            snapshot,
            &plan,
            &package,
        )
        .map_err(|error| format!("plan repair awaiting package invalid: {error:?}"))?;
    }
    Ok(())
}

fn failed_recovery_snapshot(
    lifecycle: &LifecycleStore,
    session: &WorkspaceSession,
    timeline_nodes: &mut Vec<TimelineNode>,
    snapshot: Option<PlanRepairSessionSnapshotDto>,
    link: Option<WorkspaceSessionLink>,
    error: String,
) -> PlanRepairSessionSnapshotDto {
    let now = Utc::now().to_rfc3339();
    let fallback_link = link.unwrap_or_else(|| fallback_repair_link(session, &now));
    let request = load_linked_repair_request(lifecycle, session, &fallback_link)
        .unwrap_or_else(|| fallback_repair_request(session, &fallback_link, &now));
    append_recovery_failure_timeline(session, timeline_nodes, &error, &now);
    let mut failed = snapshot.unwrap_or(PlanRepairSessionSnapshotDto {
        request: request.clone(),
        link: fallback_link,
        stage: PlanRepairSessionStage::Failed,
        projection: None,
        amendment: None,
        validation: None,
        impact: None,
        plan_review: None,
        package_identity: None,
        candidate_package_artifact_id: None,
        impact_scope_review: None,
        timeline_nodes: Vec::new(),
        error: None,
    });
    failed.request = request;
    failed.stage = PlanRepairSessionStage::Failed;
    failed.timeline_nodes = timeline_nodes.clone();
    failed.error = Some(error);
    failed
}

fn load_linked_repair_request(
    lifecycle: &LifecycleStore,
    session: &WorkspaceSession,
    link: &WorkspaceSessionLink,
) -> Option<PlanRepairRequest> {
    let revision_store = WorkItemRevisionStore::new(lifecycle.app_paths());
    let plan = revision_store
        .get_plan_lineage(&session.project_id, &session.issue_id, &session.entity_id)
        .ok()?;
    let request = revision_store
        .get_repair_request(&plan, &link.trigger.repair_request_id)
        .ok()?;
    let amendment_id = request.amendment_id.as_deref()?;
    let expected_route = format!(
        "/workbench/projects/{}/issues/{}/coding/{}",
        session.project_id, session.issue_id, request.trigger_attempt_id
    );
    if session.workspace_type != WorkspaceType::WorkItemPlan
        || request.plan_id != session.entity_id
        || link.relation != WorkspaceSessionRelation::PlanRepair
        || link.id != format!("workspace_session_link_{amendment_id}")
        || link.parent_session_id != request.trigger_attempt_id
        || link.child_session_id != session.session_id
        || link.child_session_id != format!("workspace_session_{amendment_id}")
        || link.trigger.attempt_id != request.trigger_attempt_id
        || link.trigger.unit_run_id != request.trigger_unit_run_id
        || link.trigger.review_id != request.trigger_review_id
        || link.trigger.finding_id != request.trigger_finding_id
        || link.trigger.repair_request_id != request.id
        || link.trigger.amendment_id != amendment_id
        || link.trigger.fingerprint != request.fingerprint
        || link.trigger.base_plan_revision_id != request.base_plan_revision_id
        || link.return_context.original_attempt_id != request.trigger_attempt_id
        || link.return_context.original_unit_run_id != request.trigger_unit_run_id
        || link.return_context.timeline_anchor_id != request.trigger_finding_id
        || link.return_context.original_route != expected_route
    {
        return None;
    }
    Some(request)
}

fn fallback_repair_link(session: &WorkspaceSession, now: &str) -> WorkspaceSessionLink {
    let amendment_id = session
        .session_id
        .strip_prefix("workspace_session_")
        .unwrap_or("plan_amendment_recovery_unknown")
        .to_string();
    WorkspaceSessionLink {
        id: format!("workspace_session_link_{amendment_id}"),
        relation: WorkspaceSessionRelation::PlanRepair,
        parent_session_id: "plan_repair_recovery_unknown".to_string(),
        child_session_id: session.session_id.clone(),
        trigger: WorkspaceSessionLinkTrigger {
            attempt_id: "plan_repair_recovery_unknown".to_string(),
            unit_run_id: "plan_repair_recovery_unknown".to_string(),
            review_id: None,
            finding_id: "plan_repair_recovery_unknown".to_string(),
            repair_request_id: "plan_repair_recovery_unknown".to_string(),
            amendment_id,
            fingerprint: "plan_repair_recovery_unknown".to_string(),
            base_plan_revision_id: "plan_repair_recovery_unknown".to_string(),
        },
        return_context: WorkspaceReturnContext {
            original_attempt_id: "plan_repair_recovery_unknown".to_string(),
            original_unit_run_id: "plan_repair_recovery_unknown".to_string(),
            timeline_anchor_id: "plan_repair_recovery_unknown".to_string(),
            original_route: String::new(),
        },
        created_at: now.to_string(),
    }
}

fn fallback_repair_request(
    session: &WorkspaceSession,
    link: &WorkspaceSessionLink,
    now: &str,
) -> PlanRepairRequest {
    PlanRepairRequest {
        id: link.trigger.repair_request_id.clone(),
        plan_id: session.entity_id.clone(),
        base_plan_revision_id: link.trigger.base_plan_revision_id.clone(),
        trigger_attempt_id: link.trigger.attempt_id.clone(),
        trigger_unit_run_id: link.trigger.unit_run_id.clone(),
        trigger_review_id: link.trigger.review_id.clone(),
        trigger_finding_id: link.trigger.finding_id.clone(),
        amendment_id: Some(link.trigger.amendment_id.clone()),
        defect_class: PlanDefectClass::DependencyGraphInvalid,
        reason_code: "plan_repair_recovery_failed".to_string(),
        repair_target: RepairTarget {
            kind: RepairTargetKind::CurrentWorkItem,
            logical_work_item_ids: Vec::new(),
            work_item_revision_ids: Vec::new(),
        },
        contract_refs: Vec::new(),
        capability_refs: Vec::new(),
        evidence: Vec::new(),
        fingerprint: link.trigger.fingerprint.clone(),
        status: PlanRepairRequestStatus::InProgress,
        created_at: now.to_string(),
        updated_at: now.to_string(),
    }
}

fn append_recovery_failure_timeline(
    session: &WorkspaceSession,
    timeline_nodes: &mut Vec<TimelineNode>,
    error: &str,
    now: &str,
) {
    timeline_nodes.push(TimelineNode {
        node_id: format!("timeline_node_{:03}", timeline_nodes.len() + 1),
        node_type: TimelineNodeType::PlanRepairAuthoringRevision,
        agent: None,
        stage: WsWorkspaceStage::Completed,
        round: None,
        status: TimelineNodeStatus::Failed,
        title: "Plan Repair 恢复失败".to_string(),
        summary: Some(error.to_string()),
        started_at: now.to_string(),
        completed_at: Some(now.to_string()),
        duration_ms: Some(0),
        artifact_ref: None,
        provider_config_snapshot: ProviderConfigSnapshot {
            author: session.author_provider.clone(),
            reviewer: session.reviewer_provider.clone(),
            review_rounds: session.review_rounds,
        },
        retry: None,
    });
}
