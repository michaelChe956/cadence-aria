use std::path::Path;
use std::sync::Arc;

use tokio::sync::mpsc;

use super::recovery::{
    finalize_application_and_propagate, fixture_attempt, fixture_error, minimum_impact_scope,
    passing_review, persist_completed_core_handoff, persist_published_child_snapshot, publish,
    recovered_snapshot, unique_repair_link, unique_repair_request, write_resume_target,
};
use super::seed::{core_v2_contract, fixture_paths, route_upstream_contract_invalid};
use super::{PlanRepairFaultPoint, PlanRepairFixtureError, PlanRepairFixtureRecovered};
use crate::product::checkpoint_store::CheckpointStore;
use crate::product::coding_attempt_store::CodingAttemptStore;
use crate::product::coding_models::{CodingAmendmentApplicationPhase, CodingAttemptStatus};
use crate::product::json_store::ProductStoreError;
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::models::{
    PlanAmendmentPublicationPhase, PlanRepairRequestStatus, PlanRepairReviewAttestation,
    PlanRepairSessionStage, PlanRevisionReason, WorkItemDraftRevision,
};
use crate::product::plan_repair::{PlanRepairEngine, PreparedPlanAmendment};
use crate::product::work_item_revision_store::WorkItemRevisionStore;
use crate::product::workspace_engine::{EngineEvent, WorkspaceEngine, WorkspaceSession};

const PROJECT_ID: &str = "project_0001";
const ISSUE_ID: &str = "issue_plan_0001";
const PLAN_ID: &str = "work_item_plan_0001";
const CREATED_AT: &str = "2026-07-20T00:00:10Z";

pub(super) async fn drive_until_fault(
    root: &Path,
    fault_point: PlanRepairFaultPoint,
) -> Result<(), PlanRepairFixtureError> {
    route_upstream_contract_invalid(root).await?;
    match fault_point {
        PlanRepairFaultPoint::AfterDraftSaved => {
            save_candidate_draft(root)?;
        }
        PlanRepairFaultPoint::AfterProjectionGenerated => {
            persist_candidate(root)?;
        }
        PlanRepairFaultPoint::AfterPlanReview => {
            let prepared = persist_candidate(root)?;
            persist_review(root, &prepared)?;
        }
        PlanRepairFaultPoint::AfterAmendmentPrepared => {
            super::recovery::prepare_review_and_awaiting(root).await?;
        }
        PlanRepairFaultPoint::AfterPlanPublished => {
            publish_from_current_state(root).await?;
        }
        PlanRepairFaultPoint::AfterPlanBindingWritten => {
            let manifest = publish_from_current_state(root).await?;
            ensure_plan_binding_written(root, &manifest)?;
        }
        PlanRepairFaultPoint::AfterUnitRunsWritten => {
            let manifest = publish_from_current_state(root).await?;
            ensure_unit_runs_written(root, &manifest)?;
        }
        PlanRepairFaultPoint::AfterResumeTargetWritten => {
            let manifest = publish_from_current_state(root).await?;
            ensure_unit_runs_written(root, &manifest)?;
            write_resume_target(root, &manifest)?;
        }
        PlanRepairFaultPoint::AfterHandoffPublished => {
            let manifest = publish_from_current_state(root).await?;
            ensure_unit_runs_written(root, &manifest)?;
            write_resume_target(root, &manifest)?;
            persist_completed_core_handoff(root, &manifest)?;
        }
    }
    verify_fault_boundary(root, fault_point)?;
    Err(PlanRepairFixtureError::injected(fault_point))
}

pub(super) async fn recover_to_completion(
    root: &Path,
) -> Result<PlanRepairFixtureRecovered, PlanRepairFixtureError> {
    let revision_store = WorkItemRevisionStore::new(fixture_paths(root));
    let plan = revision_store
        .get_plan_lineage(PROJECT_ID, ISSUE_ID, PLAN_ID)
        .map_err(fixture_error)?;
    if revision_store
        .list_repair_requests(&plan)
        .map_err(fixture_error)?
        .is_empty()
    {
        route_upstream_contract_invalid(root).await?;
    }
    let manifest = ensure_plan_published(root).await?;
    ensure_unit_runs_written(root, &manifest)?;
    write_resume_target(root, &manifest)?;
    let handoff = persist_completed_core_handoff(root, &manifest)?;
    finalize_application_and_propagate(root, &manifest, &handoff).await?;
    recovered_snapshot(root, &manifest)
}

fn save_candidate_draft(root: &Path) -> Result<WorkItemDraftRevision, PlanRepairFixtureError> {
    let revision_store = WorkItemRevisionStore::new(fixture_paths(root));
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
        trigger_repair_request_id: Some(request.id),
        created_at: CREATED_AT.to_string(),
    };
    revision_store
        .put_draft_revision(&plan, &draft)
        .map_err(fixture_error)?;
    Ok(draft)
}

fn persist_candidate(root: &Path) -> Result<PreparedPlanAmendment, PlanRepairFixtureError> {
    let revision_store = WorkItemRevisionStore::new(fixture_paths(root));
    let plan = revision_store
        .get_plan_lineage(PROJECT_ID, ISSUE_ID, PLAN_ID)
        .map_err(fixture_error)?;
    let request = unique_repair_request(&revision_store, &plan)?;
    let draft = save_candidate_draft(root)?;
    let engine = PlanRepairEngine::new(revision_store, plan)
        .with_candidate_drafts(vec![draft])
        .with_created_at(CREATED_AT);
    let prepared = engine.prepare_amendment(&request).map_err(fixture_error)?;
    engine.persist_candidate(&prepared).map_err(fixture_error)?;
    Ok(prepared)
}

fn persist_review(
    root: &Path,
    prepared: &PreparedPlanAmendment,
) -> Result<PlanRepairReviewAttestation, PlanRepairFixtureError> {
    let revision_store = WorkItemRevisionStore::new(fixture_paths(root));
    let plan = revision_store
        .get_plan_lineage(PROJECT_ID, ISSUE_ID, PLAN_ID)
        .map_err(fixture_error)?;
    let request = unique_repair_request(&revision_store, &plan)?;
    let review = passing_review();
    let attestation = PlanRepairReviewAttestation {
        id: format!(
            "plan_repair_review_attestation_{}_{}",
            prepared.manifest.id, review.generation_round_id
        ),
        request_id: request.id,
        amendment_id: prepared.manifest.id.clone(),
        plan_id: plan.id.clone(),
        base_plan_revision_id: prepared.base_plan_revision_id.clone(),
        reviewed_plan_revision_id: prepared.next_plan_revision.id.clone(),
        plan_projection_bundle_id: prepared.plan_projection_bundle.id.clone(),
        generation_round_id: review.generation_round_id.clone(),
        accepted_impact_scope: minimum_impact_scope(prepared),
        risk_acceptance_reason: None,
        candidate_package_artifact_id: prepared.candidate_package.id.clone(),
        candidate_package_fingerprint: prepared
            .candidate_package
            .candidate_package_fingerprint
            .clone(),
        review,
        created_at: CREATED_AT.to_string(),
    };
    revision_store
        .put_plan_repair_review_attestation(&plan, &attestation)
        .map_err(fixture_error)?;
    Ok(attestation)
}

async fn publish_from_current_state(
    root: &Path,
) -> Result<crate::product::models::PlanAmendmentManifest, PlanRepairFixtureError> {
    let (prepared, attestation, mut child_engine) =
        super::recovery::prepare_review_and_awaiting(root).await?;
    child_engine
        .confirm_plan_amendment(&prepared.manifest.id)
        .await
        .map_err(fixture_error)?;
    let manifest = publish(root, prepared, &attestation)?;
    persist_published_child_snapshot(root, &manifest)?;
    Ok(manifest)
}

async fn ensure_plan_published(
    root: &Path,
) -> Result<crate::product::models::PlanAmendmentManifest, PlanRepairFixtureError> {
    let revision_store = WorkItemRevisionStore::new(fixture_paths(root));
    let plan = revision_store
        .get_plan_lineage(PROJECT_ID, ISSUE_ID, PLAN_ID)
        .map_err(fixture_error)?;
    if plan.active_revision_id.as_deref() == Some("plan_revision_0002") {
        let request = unique_repair_request(&revision_store, &plan)?;
        let amendment_id = request
            .amendment_id
            .ok_or_else(|| fixture_error("published amendment id is missing"))?;
        return revision_store
            .get_amendment_manifest(&plan, &amendment_id)
            .map_err(fixture_error);
    }
    let lifecycle = LifecycleStore::new(fixture_paths(root));
    let link = unique_repair_link(&lifecycle)?;
    let snapshot = lifecycle
        .load_plan_repair_session_state(PROJECT_ID, ISSUE_ID, &link.child_session_id)
        .map_err(fixture_error)?
        .ok_or_else(|| fixture_error("plan repair snapshot is missing"))?;
    if snapshot.stage != PlanRepairSessionStage::AwaitingConfirmation {
        return publish_from_current_state(root).await;
    }
    let prepared = load_prepared_from_snapshot(root)?;
    let attestation_id = snapshot
        .package_identity
        .as_ref()
        .ok_or_else(|| fixture_error("package identity is missing"))?
        .review_attestation_id
        .clone();
    let attestation = revision_store
        .get_plan_repair_review_attestation(&plan, &attestation_id)
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
    child_engine
        .confirm_plan_amendment(&prepared.manifest.id)
        .await
        .map_err(fixture_error)?;
    let manifest = publish(root, prepared, &attestation)?;
    persist_published_child_snapshot(root, &manifest)?;
    Ok(manifest)
}

fn load_prepared_from_snapshot(
    root: &Path,
) -> Result<PreparedPlanAmendment, PlanRepairFixtureError> {
    let revision_store = WorkItemRevisionStore::new(fixture_paths(root));
    let plan = revision_store
        .get_plan_lineage(PROJECT_ID, ISSUE_ID, PLAN_ID)
        .map_err(fixture_error)?;
    let lifecycle = LifecycleStore::new(fixture_paths(root));
    let link = unique_repair_link(&lifecycle)?;
    let snapshot = lifecycle
        .load_plan_repair_session_state(PROJECT_ID, ISSUE_ID, &link.child_session_id)
        .map_err(fixture_error)?
        .ok_or_else(|| fixture_error("plan repair snapshot is missing"))?;
    let candidate_id = snapshot
        .candidate_package_artifact_id
        .ok_or_else(|| fixture_error("candidate package id is missing"))?;
    let candidate = revision_store
        .get_plan_repair_candidate_package(&plan, &candidate_id)
        .map_err(fixture_error)?;
    let next_plan_revision = revision_store
        .get_plan_revision(
            PROJECT_ID,
            ISSUE_ID,
            PLAN_ID,
            &candidate.new_plan_revision_id,
        )
        .map_err(fixture_error)?;
    let publication_ids = revision_store
        .allocate_plan_amendment_publication_ids(
            &plan,
            &candidate.amendment_id,
            next_plan_revision.revision_no,
            &candidate
                .minimum_manifest
                .revised_work_items
                .keys()
                .cloned()
                .collect::<Vec<_>>(),
        )
        .map_err(fixture_error)?;
    let mut drafts = Vec::new();
    let mut revisions = Vec::new();
    let mut verification = Vec::new();
    let mut logical = Vec::new();
    for (logical_id, replacement) in &candidate.minimum_manifest.revised_work_items {
        let revision = revision_store
            .get_work_item_revision(&plan, logical_id, &replacement.next_revision_id)
            .map_err(fixture_error)?;
        drafts.push(
            revision_store
                .get_draft_revision(&plan, &revision.source_draft_revision_id)
                .map_err(fixture_error)?,
        );
        verification.push(
            revision_store
                .get_verification_plan_revision(&plan, &revision.verification_plan_revision_id)
                .map_err(fixture_error)?,
        );
        logical.push(
            revision_store
                .get_logical_work_item(&plan, logical_id)
                .map_err(fixture_error)?,
        );
        revisions.push(revision);
    }
    let dependency_graph_revision = revision_store
        .get_dependency_graph_revision(&plan, &next_plan_revision.dependency_graph_revision_id)
        .map_err(fixture_error)?;
    Ok(PreparedPlanAmendment {
        base_plan_revision_id: candidate.base_plan_revision_id.clone(),
        publication_ids,
        next_plan_revision,
        draft_revisions: drafts,
        revised_work_items: revisions,
        verification_plan_revisions: verification,
        work_item_projection_bundles: candidate.work_item_projection_bundles.clone(),
        logical_work_items: logical,
        dependency_graph_revision,
        plan_projection_bundle: candidate.plan_projection_bundle.clone(),
        validation_report: candidate.validation_report.clone(),
        contract_deltas: candidate.minimum_manifest.contract_deltas.clone(),
        impact_report: candidate.impact_report.clone(),
        manifest: candidate.minimum_manifest.clone(),
        candidate_package: candidate,
        subgraph_replan: None,
    })
}

fn ensure_plan_binding_written(
    root: &Path,
    manifest: &crate::product::models::PlanAmendmentManifest,
) -> Result<(), PlanRepairFixtureError> {
    let store = CodingAttemptStore::new(fixture_paths(root));
    let attempt = fixture_attempt(&store)?;
    let journal = match store.get_amendment_application_journal(&attempt, &manifest.id) {
        Ok(journal) => journal,
        Err(ProductStoreError::NotFound { .. }) => store
            .load_or_prepare_amendment_application(&attempt, manifest)
            .map_err(fixture_error)?,
        Err(error) => return Err(fixture_error(error)),
    };
    if journal.phase.order() >= CodingAmendmentApplicationPhase::PlanBindingWritten.order() {
        return Ok(());
    }
    let current = fixture_attempt(&store)?;
    if current.status != CodingAttemptStatus::ApplyingPlanAmendment {
        store
            .update_attempt_status(
                PROJECT_ID,
                ISSUE_ID,
                &current.id,
                CodingAttemptStatus::ApplyingPlanAmendment,
            )
            .map_err(fixture_error)?;
    }
    store
        .update_plan_binding_from_manifest(&current, manifest)
        .map_err(fixture_error)?;
    store
        .advance_amendment_application_journal(
            &current,
            &manifest.id,
            CodingAmendmentApplicationPhase::PlanBindingWritten,
            None,
            "2026-07-20T00:00:13Z".to_string(),
        )
        .map_err(fixture_error)?;
    Ok(())
}

fn ensure_unit_runs_written(
    root: &Path,
    manifest: &crate::product::models::PlanAmendmentManifest,
) -> Result<(), PlanRepairFixtureError> {
    ensure_plan_binding_written(root, manifest)?;
    let store = CodingAttemptStore::new(fixture_paths(root));
    let attempt = fixture_attempt(&store)?;
    let journal = store
        .get_amendment_application_journal(&attempt, &manifest.id)
        .map_err(fixture_error)?;
    if journal.phase.order() >= CodingAmendmentApplicationPhase::UnitRunsWritten.order() {
        return Ok(());
    }
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

fn verify_fault_boundary(
    root: &Path,
    fault_point: PlanRepairFaultPoint,
) -> Result<(), PlanRepairFixtureError> {
    let paths = fixture_paths(root);
    let revision_store = WorkItemRevisionStore::new(paths.clone());
    let plan = revision_store
        .get_plan_lineage(PROJECT_ID, ISSUE_ID, PLAN_ID)
        .map_err(fixture_error)?;
    let request = unique_repair_request(&revision_store, &plan)?;
    let amendment_id = request
        .amendment_id
        .clone()
        .ok_or_else(|| fixture_error("fault boundary amendment id is missing"))?;
    let lifecycle = LifecycleStore::new(paths.clone());
    let link = unique_repair_link(&lifecycle)?;
    let snapshot = lifecycle
        .load_plan_repair_session_state(PROJECT_ID, ISSUE_ID, &link.child_session_id)
        .map_err(fixture_error)?
        .ok_or_else(|| fixture_error("fault boundary snapshot is missing"))?;
    match fault_point {
        PlanRepairFaultPoint::AfterDraftSaved => {
            revision_store
                .get_draft_revision(&plan, "work_item_draft_revision_wi_core_0002")
                .map_err(fixture_error)?;
            if revision_store
                .get_plan_revision(PROJECT_ID, ISSUE_ID, PLAN_ID, "plan_revision_0002")
                .is_ok()
            {
                return Err(fixture_error(
                    "draft boundary already exposed plan revision 2",
                ));
            }
        }
        PlanRepairFaultPoint::AfterProjectionGenerated => {
            revision_store
                .get_plan_revision(PROJECT_ID, ISSUE_ID, PLAN_ID, "plan_revision_0002")
                .map_err(fixture_error)?;
            let attestation_id =
                format!("plan_repair_review_attestation_{amendment_id}_repair_round_0001");
            if revision_store
                .get_plan_repair_review_attestation(&plan, &attestation_id)
                .is_ok()
            {
                return Err(fixture_error("projection boundary already exposed review"));
            }
        }
        PlanRepairFaultPoint::AfterPlanReview => {
            revision_store
                .get_plan_repair_review_attestation(
                    &plan,
                    &format!("plan_repair_review_attestation_{amendment_id}_repair_round_0001"),
                )
                .map_err(fixture_error)?;
            if snapshot.stage == PlanRepairSessionStage::AwaitingConfirmation {
                return Err(fixture_error("review boundary already awaits confirmation"));
            }
        }
        PlanRepairFaultPoint::AfterAmendmentPrepared => {
            if snapshot.stage != PlanRepairSessionStage::AwaitingConfirmation
                || request.status != PlanRepairRequestStatus::AwaitingConfirmation
            {
                return Err(fixture_error("awaiting-confirmation boundary mismatch"));
            }
        }
        PlanRepairFaultPoint::AfterPlanPublished => {
            if plan.active_revision_id.as_deref() != Some("plan_revision_0002") {
                return Err(fixture_error("published boundary active plan mismatch"));
            }
            let journal = revision_store
                .get_plan_amendment_publication_journal(
                    &plan,
                    &format!("{amendment_id}_publication_journal"),
                )
                .map_err(fixture_error)?;
            if journal.phase != PlanAmendmentPublicationPhase::PlanPublished {
                return Err(fixture_error("publication journal boundary mismatch"));
            }
        }
        PlanRepairFaultPoint::AfterPlanBindingWritten
        | PlanRepairFaultPoint::AfterUnitRunsWritten
        | PlanRepairFaultPoint::AfterResumeTargetWritten => {
            let store = CodingAttemptStore::new(paths);
            let attempt = fixture_attempt(&store)?;
            let journal = store
                .get_amendment_application_journal(&attempt, &amendment_id)
                .map_err(fixture_error)?;
            let expected = match fault_point {
                PlanRepairFaultPoint::AfterPlanBindingWritten => {
                    CodingAmendmentApplicationPhase::PlanBindingWritten
                }
                PlanRepairFaultPoint::AfterUnitRunsWritten => {
                    CodingAmendmentApplicationPhase::UnitRunsWritten
                }
                PlanRepairFaultPoint::AfterResumeTargetWritten => {
                    CodingAmendmentApplicationPhase::ResumeTargetWritten
                }
                _ => unreachable!(),
            };
            if journal.phase != expected {
                return Err(fixture_error("application journal boundary mismatch"));
            }
        }
        PlanRepairFaultPoint::AfterHandoffPublished => {
            revision_store
                .get_handoff_revision(&plan, "wi_core", "handoff_revision_0002")
                .map_err(fixture_error)?;
            let store = CodingAttemptStore::new(paths);
            let attempt = fixture_attempt(&store)?;
            let core = store
                .list_coding_units(PROJECT_ID, ISSUE_ID, &attempt.id)
                .map_err(fixture_error)?
                .into_iter()
                .find(|unit| unit.logical_work_item_id == "wi_core")
                .ok_or_else(|| fixture_error("fault boundary core unit is missing"))?;
            if core.latest_handoff_revision_id.as_deref() != Some("handoff_revision_0002") {
                return Err(fixture_error("handoff pointer boundary mismatch"));
            }
            let journal = store
                .get_amendment_application_journal(&attempt, &amendment_id)
                .map_err(fixture_error)?;
            if journal.phase != CodingAmendmentApplicationPhase::ResumeTargetWritten {
                return Err(fixture_error(
                    "handoff boundary advanced application journal",
                ));
            }
        }
    }
    Ok(())
}
