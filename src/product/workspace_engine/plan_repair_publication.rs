use chrono::Utc;

use crate::product::json_store::ProductStoreError;
use crate::product::models::{
    PlanAmendmentConfirmation, PlanAmendmentManifest, PlanRepairRequestStatus,
    PlanRepairSessionSnapshotDto, PlanRepairSessionStage, WorkspaceSessionStatus,
};
use crate::product::plan_repair::{PlanRepairEngine, PlanRepairError};
use crate::product::work_item_revision_store::WorkItemRevisionStore;
use crate::web::workspace_ws_types::TimelineNodeStatus;

use super::{WorkspaceEngine, WorkspaceStage};

impl WorkspaceEngine {
    pub async fn confirm_and_publish_plan_amendment(
        &mut self,
        amendment_id: &str,
        confirmed_by: &str,
    ) -> Result<PlanAmendmentManifest, PlanRepairError> {
        let snapshot = self.refresh_plan_repair_publication_state()?;
        match snapshot.stage {
            PlanRepairSessionStage::AwaitingConfirmation => {
                if !self.plan_amendment_publication_started(&snapshot, amendment_id)? {
                    self.confirm_plan_amendment(amendment_id).await?;
                }
                self.publish_confirmed_plan_amendment(amendment_id, confirmed_by)
            }
            PlanRepairSessionStage::Published
            | PlanRepairSessionStage::ApplyingAmendment
            | PlanRepairSessionStage::AmendmentApplyFailed
            | PlanRepairSessionStage::Completed => {
                self.published_plan_amendment_replay(&snapshot, amendment_id)
            }
            _ => Err(PlanRepairError::ConfirmationRequired),
        }
    }

    fn publish_confirmed_plan_amendment(
        &mut self,
        amendment_id: &str,
        confirmed_by: &str,
    ) -> Result<PlanAmendmentManifest, PlanRepairError> {
        let expected = self.require_plan_repair_snapshot()?.clone();
        let package_identity = expected.package_identity.as_ref().ok_or_else(|| {
            PlanRepairError::InvalidRepairTarget(
                "plan repair package identity is missing at publication".to_string(),
            )
        })?;
        if package_identity.amendment_id != amendment_id {
            return Err(PlanRepairError::AmendmentConflict {
                expected: package_identity.amendment_id.clone(),
                actual: amendment_id.to_string(),
            });
        }
        let lifecycle = self.persistent_lifecycle()?;
        let revision_store = WorkItemRevisionStore::new(lifecycle.app_paths());
        let plan = revision_store
            .get_plan_lineage(
                &self.session.project_id,
                &self.session.issue_id,
                &expected.request.plan_id,
            )
            .map_err(PlanRepairError::Store)?;
        let prepared = PlanRepairEngine::new(revision_store.clone(), plan.clone())
            .load_prepared_amendment(&package_identity.candidate_package_artifact_id)?;
        let attestation = revision_store
            .get_plan_repair_review_attestation(&plan, &package_identity.review_attestation_id)
            .map_err(PlanRepairError::Store)?;
        let existing_publication = revision_store
            .find_plan_amendment_publication_journal(&plan, amendment_id)
            .map_err(PlanRepairError::Store)?;
        let confirmation = match existing_publication {
            Some(journal)
                if journal.request_id == expected.request.id
                    && journal.amendment_id == amendment_id =>
            {
                journal
                    .confirmation
                    .ok_or_else(|| plan_repair_publication_identity_error(&journal.id))?
            }
            Some(journal) => return Err(plan_repair_publication_identity_error(&journal.id)),
            None => PlanAmendmentConfirmation {
                amendment_id: attestation.amendment_id.clone(),
                base_plan_revision_id: attestation.base_plan_revision_id.clone(),
                accepted_impact_scope: attestation.accepted_impact_scope.clone(),
                risk_acceptance_reason: attestation.risk_acceptance_reason.clone(),
                review_attestation_id: Some(attestation.id),
                confirmed_by: confirmed_by.to_string(),
                confirmed_at: Utc::now().to_rfc3339(),
            },
        };
        let manifest = PlanRepairEngine::new(revision_store.clone(), plan)
            .publish_amendment(prepared, confirmation)?;
        let published_plan = revision_store
            .get_plan_lineage(
                &self.session.project_id,
                &self.session.issue_id,
                &expected.request.plan_id,
            )
            .map_err(PlanRepairError::Store)?;
        let request = revision_store
            .get_repair_request(&published_plan, &expected.request.id)
            .map_err(PlanRepairError::Store)?;
        if request.status != PlanRepairRequestStatus::Published {
            return Err(plan_repair_publication_identity_error(amendment_id));
        }
        let mut published = expected.clone();
        published.request = request;
        published.stage = PlanRepairSessionStage::Published;
        published.amendment = Some(manifest.clone());
        published.error = None;
        lifecycle
            .compare_and_save_plan_repair_session_state(
                &self.session.project_id,
                &self.session.issue_id,
                &self.session.session_id,
                &expected,
                &published,
            )
            .map_err(PlanRepairError::Store)?;
        let session = lifecycle
            .get_workspace_session(&self.session.session_id)
            .map_err(PlanRepairError::Store)?;
        lifecycle
            .compare_and_update_workspace_session_status(
                &session,
                WorkspaceSessionStatus::WaitingForHuman,
            )
            .map_err(PlanRepairError::Store)?;
        self.apply_refreshed_plan_repair_state(published);
        Ok(manifest)
    }

    fn plan_amendment_publication_started(
        &self,
        snapshot: &PlanRepairSessionSnapshotDto,
        amendment_id: &str,
    ) -> Result<bool, PlanRepairError> {
        let package_identity = snapshot
            .package_identity
            .as_ref()
            .ok_or_else(|| plan_repair_publication_identity_error(amendment_id))?;
        if package_identity.amendment_id != amendment_id {
            return Err(plan_repair_publication_identity_error(amendment_id));
        }
        let lifecycle = self.persistent_lifecycle()?;
        let revision_store = WorkItemRevisionStore::new(lifecycle.app_paths());
        let plan = revision_store
            .get_plan_lineage(
                &self.session.project_id,
                &self.session.issue_id,
                &snapshot.request.plan_id,
            )
            .map_err(PlanRepairError::Store)?;
        let Some(journal) = revision_store
            .find_plan_amendment_publication_journal(&plan, amendment_id)
            .map_err(PlanRepairError::Store)?
        else {
            return Ok(false);
        };
        if journal.request_id != snapshot.request.id
            || journal.base_plan_revision_id != package_identity.base_plan_revision_id
            || journal.new_plan_revision_id != package_identity.next_plan_revision_id
            || journal.confirmation.is_none()
        {
            return Err(plan_repair_publication_identity_error(&journal.id));
        }
        Ok(true)
    }

    fn published_plan_amendment_replay(
        &self,
        snapshot: &PlanRepairSessionSnapshotDto,
        amendment_id: &str,
    ) -> Result<PlanAmendmentManifest, PlanRepairError> {
        let manifest = snapshot
            .amendment
            .as_ref()
            .filter(|manifest| manifest.id == amendment_id)
            .cloned()
            .ok_or_else(|| plan_repair_publication_identity_error(amendment_id))?;
        let lifecycle = self.persistent_lifecycle()?;
        let store = WorkItemRevisionStore::new(lifecycle.app_paths());
        let plan = store
            .get_plan_lineage(
                &self.session.project_id,
                &self.session.issue_id,
                &snapshot.request.plan_id,
            )
            .map_err(PlanRepairError::Store)?;
        let stored = store
            .get_amendment_manifest(&plan, amendment_id)
            .map_err(PlanRepairError::Store)?;
        if stored != manifest {
            return Err(plan_repair_publication_identity_error(amendment_id));
        }
        Ok(manifest)
    }

    fn refresh_plan_repair_publication_state(
        &mut self,
    ) -> Result<PlanRepairSessionSnapshotDto, PlanRepairError> {
        self.recover_pending_plan_repair_transition()?;
        let lifecycle = self.persistent_lifecycle()?;
        let mut snapshot = lifecycle
            .load_plan_repair_session_state(
                &self.session.project_id,
                &self.session.issue_id,
                &self.session.session_id,
            )
            .map_err(PlanRepairError::Store)?
            .ok_or_else(|| plan_repair_publication_identity_error(&self.session.session_id))?;
        let revision_store = WorkItemRevisionStore::new(lifecycle.app_paths());
        let plan = revision_store
            .get_plan_lineage(
                &self.session.project_id,
                &self.session.issue_id,
                &snapshot.request.plan_id,
            )
            .map_err(PlanRepairError::Store)?;
        let request = revision_store
            .get_repair_request(&plan, &snapshot.request.id)
            .map_err(PlanRepairError::Store)?;
        if snapshot.stage == PlanRepairSessionStage::AwaitingConfirmation
            && matches!(
                request.status,
                PlanRepairRequestStatus::Published | PlanRepairRequestStatus::Applied
            )
        {
            let expected = snapshot.clone();
            let amendment_id = request
                .amendment_id
                .as_deref()
                .ok_or_else(|| plan_repair_publication_identity_error(&request.id))?;
            let manifest = revision_store
                .get_amendment_manifest(&plan, amendment_id)
                .map_err(PlanRepairError::Store)?;
            snapshot.request = request.clone();
            snapshot.stage = if request.status == PlanRepairRequestStatus::Applied {
                PlanRepairSessionStage::Completed
            } else {
                PlanRepairSessionStage::Published
            };
            snapshot.amendment = Some(manifest);
            snapshot.error = None;
            lifecycle
                .compare_and_save_plan_repair_session_state(
                    &self.session.project_id,
                    &self.session.issue_id,
                    &self.session.session_id,
                    &expected,
                    &snapshot,
                )
                .map_err(PlanRepairError::Store)?;
        } else if snapshot.request != request {
            return Err(plan_repair_publication_identity_error(&snapshot.request.id));
        }
        if snapshot.link.child_session_id != self.session.session_id {
            return Err(plan_repair_publication_identity_error(
                &self.session.session_id,
            ));
        }
        self.apply_refreshed_plan_repair_state(snapshot.clone());
        Ok(snapshot)
    }

    fn apply_refreshed_plan_repair_state(&mut self, snapshot: PlanRepairSessionSnapshotDto) {
        self.timeline_nodes = snapshot.timeline_nodes.clone();
        self.active_node_id = self
            .timeline_nodes
            .iter()
            .find(|node| {
                matches!(
                    node.status,
                    TimelineNodeStatus::Active | TimelineNodeStatus::Paused
                )
            })
            .map(|node| node.node_id.clone());
        self.session.stage = match snapshot.stage {
            PlanRepairSessionStage::Completed | PlanRepairSessionStage::Failed => {
                WorkspaceStage::Completed
            }
            PlanRepairSessionStage::Triaging
            | PlanRepairSessionStage::AuthoringRevision
            | PlanRepairSessionStage::ValidatingContract
            | PlanRepairSessionStage::GeneratingProjections
            | PlanRepairSessionStage::PlanReview
            | PlanRepairSessionStage::ApplyingAmendment => WorkspaceStage::Running,
            PlanRepairSessionStage::AwaitingConfirmation
            | PlanRepairSessionStage::Published
            | PlanRepairSessionStage::AmendmentConflict
            | PlanRepairSessionStage::AmendmentApplyFailed => WorkspaceStage::HumanConfirm,
        };
        self.plan_repair_snapshot = Some(snapshot);
    }
}

fn plan_repair_publication_identity_error(id: &str) -> PlanRepairError {
    PlanRepairError::Store(ProductStoreError::IdentityMismatch {
        kind: "plan_repair_publication",
        id: id.to_string(),
    })
}
