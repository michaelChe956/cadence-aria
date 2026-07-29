use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Arc;

use tokio::sync::mpsc;

use super::seed::{core_v2_contract, fixture_paths};
use super::{
    PlanRepairDirtyGateSnapshot, PlanRepairFixtureError, PlanRepairFixtureRecovered,
    PlanRepairIdentitySnapshot,
};
use crate::product::checkpoint_store::CheckpointStore;
use crate::product::coding_attempt_store::CodingAttemptStore;
use crate::product::coding_models::{
    CodingAmendmentApplicationPhase, CodingAttemptStatus, CodingExecutionUnitStatus,
    CodingUnitRunStatus,
};
use crate::product::coding_workspace_engine::{CodingWorkspaceEngine, CodingWorkspaceEngineError};
use crate::product::git_workspace_service::GitWorkspaceService;
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::models::{
    HandoffRevision, PlanAmendmentConfirmation, PlanRepairAwaitingConfirmationPackage,
    PlanRepairPackageIdentity, PlanRepairRequest, PlanRepairRequestStatus,
    PlanRepairReviewAttestation, PlanRepairSessionStage, PlanRevisionReason, WorkItemDraftRevision,
    WorkspaceSessionStatus,
};
use crate::product::plan_repair::{PlanRepairEngine, PlanRepairError, PreparedPlanAmendment};
use crate::product::work_item_revision_store::WorkItemRevisionStore;
use crate::product::workspace_engine::{
    EngineEvent, WorkspaceEngine, WorkspaceSession, canonical_plan_repair_parent_session,
};
use crate::web::coding_ws_handler::delivery_ack::confirm_plan_amendment_socket_write;
use crate::web::workspace_ws_types::{
    ArtifactPayload, WorkItemPlanReviewAction, WorkItemPlanReviewComplete, WorkItemPlanReviewScope,
    WorkItemPlanReviewVerdict,
};

const PROJECT_ID: &str = "project_0001";
const ISSUE_ID: &str = "issue_plan_0001";
const PLAN_ID: &str = "work_item_plan_0001";
const CREATED_AT: &str = "2026-07-20T00:00:10Z";

pub(super) async fn confirm_publish_apply_and_resume(
    root: &Path,
) -> Result<PlanRepairFixtureRecovered, PlanRepairFixtureError> {
    ensure_review_is_routed(root).await?;
    let (prepared, attestation, mut child_engine) = prepare_review_and_awaiting(root).await?;
    child_engine
        .confirm_plan_amendment(&prepared.manifest.id)
        .await
        .map_err(fixture_error)?;
    let manifest = publish(root, prepared, &attestation)?;
    persist_published_child_snapshot(root, &manifest)?;
    apply_amendment(root, &manifest).await?;
    write_resume_target(root, &manifest)?;
    let handoff = persist_completed_core_handoff(root, &manifest)?;
    finalize_application_and_propagate(root, &manifest, &handoff).await?;
    recovered_snapshot(root, &manifest)
}

pub(super) fn plan_repair_request_count(root: &Path) -> Result<usize, PlanRepairFixtureError> {
    let revision_store = WorkItemRevisionStore::new(fixture_paths(root));
    let plan = revision_store
        .get_plan_lineage(PROJECT_ID, ISSUE_ID, PLAN_ID)
        .map_err(fixture_error)?;
    revision_store
        .list_repair_requests(&plan)
        .map(|requests| requests.len())
        .map_err(fixture_error)
}

pub(super) fn plan_repair_identity(
    root: &Path,
) -> Result<PlanRepairIdentitySnapshot, PlanRepairFixtureError> {
    let paths = fixture_paths(root);
    let revision_store = WorkItemRevisionStore::new(paths.clone());
    let plan = revision_store
        .get_plan_lineage(PROJECT_ID, ISSUE_ID, PLAN_ID)
        .map_err(fixture_error)?;
    let request = unique_repair_request(&revision_store, &plan)?;
    let amendment_id = request
        .amendment_id
        .clone()
        .ok_or_else(|| fixture_error("repair request amendment is missing"))?;
    let link = unique_repair_link(&LifecycleStore::new(paths))?;
    if plan.active_amendment_id.as_deref() != Some(amendment_id.as_str())
        || link.trigger.repair_request_id != request.id
        || link.trigger.fingerprint != request.fingerprint
    {
        return Err(fixture_error("active plan repair identity is inconsistent"));
    }
    Ok(PlanRepairIdentitySnapshot {
        request_id: request.id,
        amendment_id,
        child_session_id: link.child_session_id,
    })
}

#[cfg(test)]
pub(super) fn authoritative_plan_repair_request(
    root: &Path,
) -> Result<PlanRepairRequest, PlanRepairFixtureError> {
    let revision_store = WorkItemRevisionStore::new(fixture_paths(root));
    let plan = revision_store
        .get_plan_lineage(PROJECT_ID, ISSUE_ID, PLAN_ID)
        .map_err(fixture_error)?;
    unique_repair_request(&revision_store, &plan)
}

pub(super) async fn start_stale_base_plan_repair(root: &Path) -> Result<(), PlanRepairError> {
    let paths = fixture_paths(root);
    let revision_store = WorkItemRevisionStore::new(paths.clone());
    let plan = revision_store
        .get_plan_lineage(PROJECT_ID, ISSUE_ID, PLAN_ID)
        .map_err(PlanRepairError::Store)?;
    let mut request = unique_repair_request(&revision_store, &plan)
        .map_err(|error| PlanRepairError::InvalidRepairTarget(error.to_string()))?;
    request.base_plan_revision_id = "plan_revision_0000".to_string();
    let lifecycle = LifecycleStore::new(fixture_paths(root));
    let amendment_id = request.amendment_id.as_deref().ok_or_else(|| {
        PlanRepairError::InvalidRepairTarget("repair request amendment is missing".to_string())
    })?;
    let parent = canonical_plan_repair_parent_session(
        &lifecycle,
        PROJECT_ID,
        ISSUE_ID,
        PLAN_ID,
        amendment_id,
    )?;
    let (event_tx, _event_rx) = mpsc::channel::<EngineEvent>(16);
    let mut engine = WorkspaceEngine::new_persistent(
        Arc::new(CheckpointStore::new(root.join("repair-checkpoints"))),
        lifecycle,
        event_tx,
        WorkspaceSession::from_record(parent),
    );
    engine.start_plan_repair(request).await.map(|_| ())
}

pub(super) async fn publish_then_attempt_dirty_worktree_apply(
    root: &Path,
) -> Result<PlanRepairDirtyGateSnapshot, PlanRepairFixtureError> {
    ensure_review_is_routed(root).await?;
    let (prepared, attestation, mut child_engine) = prepare_review_and_awaiting(root).await?;
    child_engine
        .confirm_plan_amendment(&prepared.manifest.id)
        .await
        .map_err(fixture_error)?;
    let manifest = publish(root, prepared, &attestation)?;
    persist_published_child_snapshot(root, &manifest)?;

    let store = CodingAttemptStore::new(fixture_paths(root));
    let attempt = fixture_attempt(&store)?;
    let binding_before = store.get_plan_binding(&attempt).map_err(fixture_error)?;
    std::fs::write(root.join("worktree/dirty-before-amendment.txt"), "dirty\n")
        .map_err(fixture_error)?;
    let (event_tx, _event_rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), event_tx);
    match engine.apply_plan_amendment(&attempt, &manifest).await {
        Err(CodingWorkspaceEngineError::SharedWorktreeDirtyManualGate(reason))
            if reason == "worktree_dirty_before_plan_amendment" => {}
        Err(error) => return Err(fixture_error(error)),
        Ok(_) => {
            return Err(fixture_error(
                "dirty worktree amendment unexpectedly applied",
            ));
        }
    }

    let gates = store
        .list_open_blocked_gates(PROJECT_ID, ISSUE_ID, &attempt.id)
        .map_err(fixture_error)?;
    let open_gate_reason_codes = gates
        .iter()
        .filter_map(|gate| gate.reason_code.clone())
        .collect::<Vec<_>>();
    let application_journal_count = store
        .list_amendment_application_journals(&attempt)
        .map_err(fixture_error)?
        .len();
    let binding_after = store.get_plan_binding(&attempt).map_err(fixture_error)?;
    if binding_after.bound_plan_revision_id != binding_before.bound_plan_revision_id
        || binding_after.applied_amendment_ids != binding_before.applied_amendment_ids
    {
        return Err(fixture_error(
            "dirty worktree gate mutated the coding plan binding",
        ));
    }
    Ok(PlanRepairDirtyGateSnapshot {
        open_gate_count: gates.len(),
        open_gate_reason_codes,
        application_journal_count,
        bound_plan_revision_id: binding_after.bound_plan_revision_id,
        applied_amendment_count: binding_after.applied_amendment_ids.len(),
    })
}

pub(super) async fn ensure_review_is_routed(root: &Path) -> Result<(), PlanRepairFixtureError> {
    let store = CodingAttemptStore::new(fixture_paths(root));
    let attempt = fixture_attempt(&store)?;
    if matches!(
        attempt.status,
        crate::product::coding_models::CodingAttemptStatus::AwaitingPlanAmendment
            | crate::product::coding_models::CodingAttemptStatus::ApplyingPlanAmendment
            | crate::product::coding_models::CodingAttemptStatus::AmendmentApplyFailed
    ) {
        return Ok(());
    }
    super::seed::route_upstream_contract_invalid(root)
        .await
        .map(|_| ())
}

pub(super) async fn prepare_review_and_awaiting(
    root: &Path,
) -> Result<
    (
        PreparedPlanAmendment,
        PlanRepairReviewAttestation,
        WorkspaceEngine,
    ),
    PlanRepairFixtureError,
> {
    let paths = fixture_paths(root);
    let revision_store = WorkItemRevisionStore::new(paths.clone());
    let plan = revision_store
        .get_plan_lineage(PROJECT_ID, ISSUE_ID, PLAN_ID)
        .map_err(fixture_error)?;
    let request = unique_repair_request(&revision_store, &plan)?;
    let draft = WorkItemDraftRevision {
        id: "work_item_draft_revision_wi_core_0002".to_string(),
        logical_work_item_id: "wi_core".to_string(),
        revision_no: 2,
        supersedes: Some("work_item_draft_revision_wi_core_0001".to_string()),
        revision_reason: PlanRevisionReason::RepairUpstreamContract,
        canonical_contract_candidate: core_v2_contract(),
        trigger_repair_request_id: Some(request.id.clone()),
        created_at: CREATED_AT.to_string(),
    };
    revision_store
        .put_draft_revision(&plan, &draft)
        .map_err(fixture_error)?;
    let engine = PlanRepairEngine::new(revision_store.clone(), plan.clone())
        .with_candidate_drafts(vec![draft])
        .with_created_at(CREATED_AT);
    let prepared = engine.prepare_amendment(&request).map_err(fixture_error)?;
    engine.persist_candidate(&prepared).map_err(fixture_error)?;
    enter_prepared_awaiting(root, revision_store, plan, request, prepared).await
}

pub(super) async fn enter_prepared_awaiting(
    root: &Path,
    revision_store: WorkItemRevisionStore,
    plan: crate::product::models::WorkItemPlanLineage,
    request: PlanRepairRequest,
    prepared: PreparedPlanAmendment,
) -> Result<
    (
        PreparedPlanAmendment,
        PlanRepairReviewAttestation,
        WorkspaceEngine,
    ),
    PlanRepairFixtureError,
> {
    let review = passing_review();
    let attestation = PlanRepairReviewAttestation {
        id: format!(
            "plan_repair_review_attestation_{}_{}",
            prepared.manifest.id, review.generation_round_id
        ),
        request_id: request.id.clone(),
        amendment_id: prepared.manifest.id.clone(),
        plan_id: plan.id.clone(),
        base_plan_revision_id: prepared.base_plan_revision_id.clone(),
        reviewed_plan_revision_id: prepared.next_plan_revision.id.clone(),
        plan_projection_bundle_id: prepared.plan_projection_bundle.id.clone(),
        generation_round_id: review.generation_round_id.clone(),
        accepted_impact_scope: minimum_impact_scope(&prepared),
        risk_acceptance_reason: None,
        candidate_package_artifact_id: prepared.candidate_package.id.clone(),
        candidate_package_fingerprint: prepared
            .candidate_package
            .candidate_package_fingerprint
            .clone(),
        review: review.clone(),
        created_at: CREATED_AT.to_string(),
    };
    revision_store
        .put_plan_repair_review_attestation(&plan, &attestation)
        .map_err(fixture_error)?;

    let lifecycle = LifecycleStore::new(fixture_paths(root));
    let link = unique_repair_link(&lifecycle)?;
    let mut snapshot = lifecycle
        .load_plan_repair_session_state(PROJECT_ID, ISSUE_ID, &link.child_session_id)
        .map_err(fixture_error)?
        .ok_or_else(|| fixture_error("plan repair snapshot is missing"))?;
    snapshot.candidate_package_artifact_id = Some(prepared.candidate_package.id.clone());
    lifecycle
        .save_plan_repair_session_state(PROJECT_ID, ISSUE_ID, &link.child_session_id, &snapshot)
        .map_err(fixture_error)?;
    let child = lifecycle
        .get_workspace_session(&link.child_session_id)
        .map_err(fixture_error)?;
    let (event_tx, _event_rx) = mpsc::channel::<EngineEvent>(16);
    let mut child_engine = WorkspaceEngine::new_persistent(
        Arc::new(CheckpointStore::new(root.join("repair-checkpoints"))),
        lifecycle,
        event_tx,
        WorkspaceSession::from_record(child),
    );
    let package = PlanRepairAwaitingConfirmationPackage {
        package_identity: PlanRepairPackageIdentity {
            request_id: request.id,
            amendment_id: prepared.manifest.id.clone(),
            plan_id: plan.id,
            base_plan_revision_id: prepared.base_plan_revision_id.clone(),
            next_plan_revision_id: prepared.next_plan_revision.id.clone(),
            projection_bundle_id: prepared.plan_projection_bundle.id.clone(),
            validation_report_id: prepared.validation_report.id.clone(),
            review_attestation_id: attestation.id.clone(),
            reviewed_plan_revision_id: prepared.next_plan_revision.id.clone(),
            review_generation_round_id: review.generation_round_id.clone(),
            candidate_package_artifact_id: prepared.candidate_package.id.clone(),
            candidate_package_fingerprint: prepared
                .candidate_package
                .candidate_package_fingerprint
                .clone(),
        },
        projection: prepared.plan_projection_bundle.clone(),
        amendment: prepared.manifest.clone(),
        validation: prepared.validation_report.clone(),
        impact: prepared.impact_report.clone(),
        plan_review: review,
    };
    child_engine
        .enter_plan_repair_awaiting_confirmation(package)
        .await
        .map_err(fixture_error)?;
    Ok((prepared, attestation, child_engine))
}

pub(super) fn publish(
    root: &Path,
    prepared: PreparedPlanAmendment,
    attestation: &PlanRepairReviewAttestation,
) -> Result<crate::product::models::PlanAmendmentManifest, PlanRepairFixtureError> {
    let revision_store = WorkItemRevisionStore::new(fixture_paths(root));
    let plan = revision_store
        .get_plan_lineage(PROJECT_ID, ISSUE_ID, PLAN_ID)
        .map_err(fixture_error)?;
    PlanRepairEngine::new(revision_store, plan)
        .with_created_at(CREATED_AT)
        .publish_amendment(
            prepared,
            PlanAmendmentConfirmation {
                amendment_id: attestation.amendment_id.clone(),
                base_plan_revision_id: attestation.base_plan_revision_id.clone(),
                accepted_impact_scope: attestation.accepted_impact_scope.clone(),
                risk_acceptance_reason: None,
                review_attestation_id: Some(attestation.id.clone()),
                confirmed_by: "fixture_user".to_string(),
                confirmed_at: "2026-07-20T00:00:11Z".to_string(),
            },
        )
        .map_err(fixture_error)
}

pub(super) fn persist_published_child_snapshot(
    root: &Path,
    manifest: &crate::product::models::PlanAmendmentManifest,
) -> Result<(), PlanRepairFixtureError> {
    let paths = fixture_paths(root);
    let revision_store = WorkItemRevisionStore::new(paths.clone());
    let plan = revision_store
        .get_plan_lineage(PROJECT_ID, ISSUE_ID, PLAN_ID)
        .map_err(fixture_error)?;
    let request = revision_store
        .get_repair_request(&plan, &manifest.repair_request_id)
        .map_err(fixture_error)?;
    if request.status != PlanRepairRequestStatus::Published {
        return Err(fixture_error("published request status is missing"));
    }
    let lifecycle = LifecycleStore::new(paths);
    let link = unique_repair_link(&lifecycle)?;
    let mut snapshot = lifecycle
        .load_plan_repair_session_state(PROJECT_ID, ISSUE_ID, &link.child_session_id)
        .map_err(fixture_error)?
        .ok_or_else(|| fixture_error("plan repair snapshot is missing"))?;
    snapshot.request = request;
    snapshot.stage = PlanRepairSessionStage::Published;
    snapshot.amendment = Some(manifest.clone());
    lifecycle
        .save_plan_repair_session_state(PROJECT_ID, ISSUE_ID, &link.child_session_id, &snapshot)
        .map_err(fixture_error)?;
    lifecycle
        .update_workspace_session_status(
            &link.child_session_id,
            WorkspaceSessionStatus::WaitingForHuman,
        )
        .map_err(fixture_error)?;
    Ok(())
}

pub(super) async fn apply_amendment(
    root: &Path,
    manifest: &crate::product::models::PlanAmendmentManifest,
) -> Result<(), PlanRepairFixtureError> {
    let store = CodingAttemptStore::new(fixture_paths(root));
    let attempt = fixture_attempt(&store)?;
    store
        .load_or_prepare_amendment_application(&attempt, manifest)
        .map_err(fixture_error)?;
    store
        .update_attempt_status(
            PROJECT_ID,
            ISSUE_ID,
            &attempt.id,
            CodingAttemptStatus::ApplyingPlanAmendment,
        )
        .map_err(fixture_error)?;
    store
        .update_plan_binding_from_manifest(&attempt, manifest)
        .map_err(fixture_error)?;
    store
        .advance_amendment_application_journal(
            &attempt,
            &manifest.id,
            CodingAmendmentApplicationPhase::PlanBindingWritten,
            None,
            "2026-07-20T00:00:13Z".to_string(),
        )
        .map_err(fixture_error)?;
    store
        .materialize_unit_runs_from_manifest(&attempt, manifest, attempt.head_commit.as_deref())
        .map_err(fixture_error)?;
    store
        .advance_amendment_application_journal(
            &attempt,
            &manifest.id,
            CodingAmendmentApplicationPhase::UnitRunsWritten,
            None,
            "2026-07-20T00:00:14Z".to_string(),
        )
        .map_err(fixture_error)?;
    Ok(())
}

pub(super) fn write_resume_target(
    root: &Path,
    manifest: &crate::product::models::PlanAmendmentManifest,
) -> Result<(), PlanRepairFixtureError> {
    let store = CodingAttemptStore::new(fixture_paths(root));
    let attempt = fixture_attempt(&store)?;
    let journal = store
        .get_amendment_application_journal(&attempt, &manifest.id)
        .map_err(fixture_error)?;
    if journal.phase.order() >= CodingAmendmentApplicationPhase::ResumeTargetWritten.order() {
        return Ok(());
    }
    store
        .set_resume_target_from_manifest(&attempt, manifest)
        .map_err(fixture_error)?;
    store
        .advance_amendment_application_journal(
            &attempt,
            &manifest.id,
            CodingAmendmentApplicationPhase::ResumeTargetWritten,
            None,
            "2026-07-20T00:00:15Z".to_string(),
        )
        .map_err(fixture_error)?;
    Ok(())
}

pub(super) fn persist_completed_core_handoff(
    root: &Path,
    manifest: &crate::product::models::PlanAmendmentManifest,
) -> Result<HandoffRevision, PlanRepairFixtureError> {
    let paths = fixture_paths(root);
    let store = CodingAttemptStore::new(paths.clone());
    let revision_store = WorkItemRevisionStore::new(paths.clone());
    let attempt = fixture_attempt(&store)?;
    let plan = revision_store
        .get_plan_lineage(PROJECT_ID, ISSUE_ID, PLAN_ID)
        .map_err(fixture_error)?;
    if let Ok(existing) =
        revision_store.get_handoff_revision(&plan, "wi_core", "handoff_revision_0002")
    {
        return Ok(existing);
    }
    let units = store
        .list_coding_units(PROJECT_ID, ISSUE_ID, &attempt.id)
        .map_err(fixture_error)?;
    let core = units
        .iter()
        .find(|unit| unit.logical_work_item_id == "wi_core")
        .ok_or_else(|| fixture_error("core unit is missing"))?;
    let core_run = store
        .list_coding_unit_runs(&attempt, &core.id)
        .map_err(fixture_error)?
        .into_iter()
        .find(|run| run.work_item_revision_id == "work_item_revision_wi_core_0002")
        .ok_or_else(|| fixture_error("amendment core run is missing"))?;
    store
        .set_materialized_amendment_unit_run_status(
            &attempt,
            manifest,
            "wi_core",
            CodingUnitRunStatus::Running,
        )
        .map_err(fixture_error)?;
    let completed = store
        .complete_coding_unit_run(&attempt, &core_run.id, "commit_core_v2")
        .map_err(fixture_error)?;
    store
        .update_coding_unit_status(
            PROJECT_ID,
            ISSUE_ID,
            &attempt.id,
            &core.id,
            CodingExecutionUnitStatus::Completed,
            Some("core revision 2 completed".to_string()),
        )
        .map_err(fixture_error)?;
    let handoff = HandoffRevision {
        id: "handoff_revision_0002".to_string(),
        logical_work_item_id: "wi_core".to_string(),
        work_item_revision_id: completed.work_item_revision_id,
        coding_unit_run_id: completed.id,
        provided_contracts: vec!["contract.workflow".to_string()],
        provided_capabilities: BTreeMap::from([(
            "contract.workflow".to_string(),
            vec![
                "failure_message".to_string(),
                "finalization_failure".to_string(),
                "workflow_explicit_completion".to_string(),
            ],
        )]),
        contract_hash: "contract_hash_v2".to_string(),
        commit_sha: "commit_core_v2".to_string(),
        created_at: "2026-07-20T00:00:12Z".to_string(),
    };
    revision_store
        .put_handoff_revision(&plan, &handoff)
        .map_err(fixture_error)?;
    store
        .update_coding_unit_latest_handoff_revision_id(
            PROJECT_ID,
            ISSUE_ID,
            &attempt.id,
            &core.id,
            Some(handoff.id.clone()),
        )
        .map_err(fixture_error)?;
    Ok(handoff)
}

pub(super) async fn finalize_application_and_propagate(
    root: &Path,
    manifest: &crate::product::models::PlanAmendmentManifest,
    handoff: &HandoffRevision,
) -> Result<(), PlanRepairFixtureError> {
    let store = CodingAttemptStore::new(fixture_paths(root));
    let attempt = fixture_attempt(&store)?;
    let journal = store
        .get_amendment_application_journal(&attempt, &manifest.id)
        .map_err(fixture_error)?;
    if journal.phase != CodingAmendmentApplicationPhase::Completed {
        store
            .advance_amendment_application_journal(
                &attempt,
                &manifest.id,
                CodingAmendmentApplicationPhase::Completed,
                None,
                "2026-07-20T00:00:16Z".to_string(),
            )
            .map_err(fixture_error)?;
    }
    let (event_tx, mut event_rx) = mpsc::channel(16);
    tokio::spawn(async move {
        while let Some(event) = event_rx.recv().await {
            confirm_plan_amendment_socket_write(&event);
        }
    });
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), event_tx);
    let resumed = engine
        .recover_plan_amendment(&fixture_attempt(&store)?)
        .await
        .map_err(fixture_error)?;
    engine
        .apply_completed_handoff(&resumed, handoff)
        .await
        .map_err(fixture_error)?;
    Ok(())
}

pub(super) fn recovered_snapshot(
    root: &Path,
    manifest: &crate::product::models::PlanAmendmentManifest,
) -> Result<PlanRepairFixtureRecovered, PlanRepairFixtureError> {
    let paths = fixture_paths(root);
    let store = CodingAttemptStore::new(paths.clone());
    let revision_store = WorkItemRevisionStore::new(paths.clone());
    let attempt = fixture_attempt(&store)?;
    let plan = revision_store
        .get_plan_lineage(PROJECT_ID, ISSUE_ID, PLAN_ID)
        .map_err(fixture_error)?;
    let binding = store.get_plan_binding(&attempt).map_err(fixture_error)?;
    let units = store
        .list_coding_units(PROJECT_ID, ISSUE_ID, &attempt.id)
        .map_err(fixture_error)?;
    let active = units
        .iter()
        .find(|unit| attempt.active_unit_id.as_deref() == Some(unit.id.as_str()))
        .ok_or_else(|| fixture_error("active unit is missing"))?;
    let active_run = store.get_active_unit_run(&attempt).map_err(fixture_error)?;
    let mut logical_active_revision_ids = BTreeMap::new();
    for logical_id in ["wi_core", "wi_registration", "wi_unrelated"] {
        let logical = revision_store
            .get_logical_work_item(&plan, logical_id)
            .map_err(fixture_error)?;
        logical_active_revision_ids.insert(
            logical_id.to_string(),
            logical
                .active_revision_id
                .ok_or_else(|| fixture_error("logical active revision is missing"))?,
        );
    }
    let repair_requests = revision_store
        .list_repair_requests(&plan)
        .map_err(fixture_error)?;
    let repair_request_count = repair_requests.len();
    let mut amendment_reference_ids = repair_requests
        .into_iter()
        .filter_map(|request| request.amendment_id)
        .collect::<Vec<_>>();
    amendment_reference_ids.sort();
    let unique_amendment_reference_ids = amendment_reference_ids
        .iter()
        .collect::<BTreeSet<_>>()
        .len();
    let lifecycle = LifecycleStore::new(paths);
    let repair_link = unique_repair_link(&lifecycle)?;
    let mut amendment_artifact_ids = lifecycle
        .list_artifact_versions(&repair_link.child_session_id)
        .map_err(fixture_error)?
        .into_iter()
        .filter_map(|version| match version.payload {
            ArtifactPayload::PlanAmendmentManifest { manifest } => Some(manifest.id),
            _ => None,
        })
        .collect::<Vec<_>>();
    amendment_artifact_ids.sort();
    let unique_amendment_artifact_ids =
        amendment_artifact_ids.iter().collect::<BTreeSet<_>>().len();
    let mut unit_run_ids = Vec::new();
    for unit in &units {
        unit_run_ids.extend(
            store
                .list_coding_unit_runs(&attempt, &unit.id)
                .map_err(fixture_error)?
                .into_iter()
                .map(|run| run.id),
        );
    }
    unit_run_ids.sort();
    let unique_unit_run_ids = unit_run_ids.iter().collect::<BTreeSet<_>>().len();
    let handoff_revision_ids = revision_store
        .list_handoff_revisions(&plan, "wi_core")
        .map_err(fixture_error)?
        .into_iter()
        .map(|handoff| handoff.id)
        .collect::<Vec<_>>();
    let unique_handoff_revision_ids = handoff_revision_ids.iter().collect::<BTreeSet<_>>().len();
    Ok(PlanRepairFixtureRecovered {
        bound_plan_revision_id: binding.bound_plan_revision_id,
        active_plan_revision_id: plan.active_revision_id.unwrap_or_default(),
        active_amendment_id: plan.active_amendment_id,
        logical_active_revision_ids,
        current_work_item_revision_id: active.work_item_revision_id.clone(),
        current_resolved_handoff_revision_ids: active_run.resolved_handoff_revision_ids,
        rewritten_logical_work_item_ids: manifest.revised_work_items.keys().cloned().collect(),
        revalidated_logical_work_item_ids: manifest.revalidation_required_units.clone(),
        repair_request_count,
        amendment_reference_ids,
        unique_amendment_reference_ids,
        amendment_artifact_ids,
        unique_amendment_artifact_ids,
        unit_run_ids,
        unique_unit_run_ids,
        handoff_revision_ids,
        unique_handoff_revision_ids,
    })
}

pub(super) fn unique_repair_request(
    store: &WorkItemRevisionStore,
    plan: &crate::product::models::WorkItemPlanLineage,
) -> Result<PlanRepairRequest, PlanRepairFixtureError> {
    let requests = store.list_repair_requests(plan).map_err(fixture_error)?;
    match requests.as_slice() {
        [request] => Ok(request.clone()),
        _ => Err(fixture_error("repair request is not unique")),
    }
}

pub(super) fn unique_repair_link(
    lifecycle: &LifecycleStore,
) -> Result<crate::product::models::WorkspaceSessionLink, PlanRepairFixtureError> {
    let links = lifecycle
        .list_session_links(PROJECT_ID, ISSUE_ID)
        .map_err(fixture_error)?;
    match links.as_slice() {
        [link] => Ok(link.clone()),
        _ => Err(fixture_error("plan repair link is not unique")),
    }
}

pub(super) fn fixture_attempt(
    store: &CodingAttemptStore,
) -> Result<crate::product::coding_models::CodingExecutionAttempt, PlanRepairFixtureError> {
    store
        .get_attempt_for_work_item_group(PROJECT_ID, ISSUE_ID, PLAN_ID)
        .map_err(fixture_error)?
        .ok_or_else(|| fixture_error("fixture attempt is missing"))
}

pub(super) fn passing_review() -> WorkItemPlanReviewComplete {
    WorkItemPlanReviewComplete {
        verdict: WorkItemPlanReviewVerdict::Pass,
        review_scope: WorkItemPlanReviewScope::Outline,
        target_outline_id: None,
        generation_round_id: "repair_round_0001".to_string(),
        draft_id: None,
        batch_id: None,
        review_action: WorkItemPlanReviewAction::Continue,
        gates: Vec::new(),
        affects_items: Vec::new(),
        warnings: Vec::new(),
    }
}

pub(super) fn minimum_impact_scope(prepared: &PreparedPlanAmendment) -> Vec<String> {
    prepared
        .manifest
        .revalidation_required_units
        .iter()
        .chain(prepared.manifest.stale_units.iter())
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

pub(super) fn fixture_error(error: impl std::fmt::Debug) -> PlanRepairFixtureError {
    PlanRepairFixtureError {
        message: format!("plan_repair_fixture_failed: {error:?}"),
        fault_point: None,
    }
}
