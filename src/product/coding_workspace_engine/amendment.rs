use std::collections::BTreeSet;

use chrono::Utc;

use super::*;
use crate::product::coding_models::{
    CodingAmendmentApplicationJournal, CodingAmendmentApplicationPhase, CodingAttemptScope,
};
use crate::product::models::{
    PlanAmendmentManifest, PlanRepairRequest, PlanRepairRequestStatus,
    PlanRepairSessionSnapshotDto, PlanRepairSessionStage, WorkspaceSessionStatus,
};
use crate::product::work_item_revision_store::WorkItemRevisionStore;

struct AmendmentApplicationAuthority {
    plan: crate::product::models::WorkItemPlanLineage,
    request: PlanRepairRequest,
}

impl CodingWorkspaceEngine {
    pub async fn apply_plan_amendment(
        &self,
        attempt: &CodingExecutionAttempt,
        manifest: &PlanAmendmentManifest,
    ) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
        let current = self.store.validate_attempt_lineage(attempt)?;
        let mut journal = match self
            .store
            .get_amendment_application_journal(&current, &manifest.id)
        {
            Ok(journal) => journal,
            Err(ProductStoreError::NotFound { .. }) => {
                self.validate_amendment_application_identity(&current, manifest, false)?;
                self.ensure_plan_amendment_worktree_ready(&current).await?;
                self.store
                    .load_or_prepare_amendment_application(&current, manifest)?
            }
            Err(error) => return Err(error.into()),
        };
        if journal.phase != CodingAmendmentApplicationPhase::Completed {
            self.ensure_plan_amendment_worktree_ready(&current).await?;
        }
        let result = self
            .apply_plan_amendment_from_journal(&current, manifest, &mut journal)
            .await;
        match result {
            Ok(updated) => Ok(updated),
            Err(error) => {
                self.record_amendment_application_failure(&current, manifest, &error);
                Err(error)
            }
        }
    }

    pub async fn recover_plan_amendment(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
        let current = self.store.validate_attempt_lineage(attempt)?;
        let binding = self.store.get_plan_binding(&current)?;
        let revision_store = WorkItemRevisionStore::new(self.store.paths());
        let plan = revision_store.get_plan_lineage(
            &current.project_id,
            &current.issue_id,
            &binding.plan_id,
        )?;
        let amendment_id = match plan.active_amendment_id.clone() {
            Some(id) => id,
            None => binding
                .applied_amendment_ids
                .last()
                .cloned()
                .ok_or_else(|| amendment_identity_error(&current.id))?,
        };
        let manifest = revision_store.get_amendment_manifest(&plan, &amendment_id)?;
        self.apply_plan_amendment(&current, &manifest).await
    }

    async fn apply_plan_amendment_from_journal(
        &self,
        attempt: &CodingExecutionAttempt,
        manifest: &PlanAmendmentManifest,
        journal: &mut CodingAmendmentApplicationJournal,
    ) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
        let authority = self.validate_amendment_application_identity(
            attempt,
            manifest,
            journal.phase == CodingAmendmentApplicationPhase::Completed,
        )?;
        if journal.error.is_some() {
            *journal = self
                .store
                .clear_amendment_application_error(attempt, &manifest.id)?;
        }
        if journal.phase != CodingAmendmentApplicationPhase::Completed {
            let current =
                self.store
                    .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
            if current.status != CodingAttemptStatus::ApplyingPlanAmendment {
                self.store.update_attempt_status(
                    &attempt.project_id,
                    &attempt.issue_id,
                    &attempt.id,
                    CodingAttemptStatus::ApplyingPlanAmendment,
                )?;
            }
        }

        if journal.phase.order() < CodingAmendmentApplicationPhase::PlanBindingWritten.order() {
            self.ensure_active_amendment_lock(&authority.plan, manifest)?;
            self.store
                .update_plan_binding_from_manifest(attempt, manifest)?;
            *journal = self.store.advance_amendment_application_journal(
                attempt,
                &manifest.id,
                CodingAmendmentApplicationPhase::PlanBindingWritten,
                None,
                Utc::now().to_rfc3339(),
            )?;
        }
        if journal.phase.order() < CodingAmendmentApplicationPhase::UnitRunsWritten.order() {
            self.ensure_active_amendment_lock(&authority.plan, manifest)?;
            self.store
                .materialize_unit_runs_from_manifest(attempt, manifest)?;
            *journal = self.store.advance_amendment_application_journal(
                attempt,
                &manifest.id,
                CodingAmendmentApplicationPhase::UnitRunsWritten,
                None,
                Utc::now().to_rfc3339(),
            )?;
        }
        if journal.phase.order() < CodingAmendmentApplicationPhase::ResumeTargetWritten.order() {
            self.ensure_active_amendment_lock(&authority.plan, manifest)?;
            self.store
                .set_resume_target_from_manifest(attempt, manifest)?;
            *journal = self.store.advance_amendment_application_journal(
                attempt,
                &manifest.id,
                CodingAmendmentApplicationPhase::ResumeTargetWritten,
                None,
                Utc::now().to_rfc3339(),
            )?;
        }
        if journal.phase.order() < CodingAmendmentApplicationPhase::Completed.order() {
            self.ensure_active_amendment_lock(&authority.plan, manifest)?;
            *journal = self.store.advance_amendment_application_journal(
                attempt,
                &manifest.id,
                CodingAmendmentApplicationPhase::Completed,
                None,
                Utc::now().to_rfc3339(),
            )?;
        }

        let updated = self.finalize_completed_amendment_application(
            attempt,
            manifest,
            &authority.plan,
            &authority.request,
        )?;
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::PlanAmendmentUpdated {
                amendment: Box::new(manifest.clone()),
            })
            .await;
        Ok(updated)
    }

    fn validate_amendment_application_identity(
        &self,
        attempt: &CodingExecutionAttempt,
        manifest: &PlanAmendmentManifest,
        completed: bool,
    ) -> Result<AmendmentApplicationAuthority, CodingWorkspaceEngineError> {
        if attempt.scope != CodingAttemptScope::WorkItemGroup {
            return Err(amendment_identity_error(&attempt.id).into());
        }
        let binding = self.store.get_plan_binding(attempt)?;
        let revision_store = WorkItemRevisionStore::new(self.store.paths());
        let plan = revision_store.get_plan_lineage(
            &attempt.project_id,
            &attempt.issue_id,
            &binding.plan_id,
        )?;
        if plan.active_revision_id.as_deref() != Some(manifest.new_plan_revision_id.as_str()) {
            return Err(amendment_identity_error(&manifest.id).into());
        }
        match plan.active_amendment_id.as_deref() {
            Some(id) if id == manifest.id => {}
            None if completed => {}
            _ => return Err(amendment_identity_error(&manifest.id).into()),
        }
        let stored_manifest = revision_store.get_amendment_manifest(&plan, &manifest.id)?;
        if stored_manifest != *manifest {
            return Err(amendment_identity_error(&manifest.id).into());
        }
        let request = revision_store.get_repair_request(&plan, &manifest.repair_request_id)?;
        let allowed_status = if completed {
            matches!(
                request.status,
                PlanRepairRequestStatus::Published | PlanRepairRequestStatus::Applied
            )
        } else {
            request.status == PlanRepairRequestStatus::Published
        };
        if !allowed_status
            || request.plan_id != plan.id
            || request.base_plan_revision_id != manifest.previous_plan_revision_id
            || request.trigger_attempt_id != attempt.id
            || request.amendment_id.as_deref() != Some(manifest.id.as_str())
        {
            return Err(amendment_identity_error(&manifest.repair_request_id).into());
        }
        let previous = revision_store.get_plan_revision(
            &attempt.project_id,
            &attempt.issue_id,
            &plan.id,
            &manifest.previous_plan_revision_id,
        )?;
        let next = revision_store.get_plan_revision(
            &attempt.project_id,
            &attempt.issue_id,
            &plan.id,
            &manifest.new_plan_revision_id,
        )?;
        if next.supersedes.as_deref() != Some(previous.id.as_str()) {
            return Err(amendment_identity_error(&manifest.new_plan_revision_id).into());
        }
        for (logical_id, replacement) in &manifest.revised_work_items {
            if previous.work_item_bindings.get(logical_id)
                != Some(&replacement.previous_revision_id)
                || next.work_item_bindings.get(logical_id) != Some(&replacement.next_revision_id)
            {
                return Err(amendment_identity_error(logical_id).into());
            }
        }
        validate_manifest_partitions(manifest, &next.work_item_bindings.keys().cloned().collect())?;
        if !completed {
            let snapshot = self
                .store
                .linked_active_plan_repair_snapshot(attempt)?
                .ok_or_else(|| amendment_identity_error(&manifest.id))?;
            validate_linked_snapshot(attempt, manifest, &request, &snapshot)?;
        }
        Ok(AmendmentApplicationAuthority { plan, request })
    }

    fn ensure_active_amendment_lock(
        &self,
        authority: &crate::product::models::WorkItemPlanLineage,
        manifest: &PlanAmendmentManifest,
    ) -> Result<(), CodingWorkspaceEngineError> {
        let plan = WorkItemRevisionStore::new(self.store.paths()).get_plan_lineage(
            &authority.project_id,
            &authority.issue_id,
            &authority.id,
        )?;
        if plan.active_revision_id.as_deref() != Some(manifest.new_plan_revision_id.as_str())
            || plan.active_amendment_id.as_deref() != Some(manifest.id.as_str())
        {
            return Err(amendment_identity_error(&manifest.id).into());
        }
        Ok(())
    }

    fn finalize_completed_amendment_application(
        &self,
        attempt: &CodingExecutionAttempt,
        manifest: &PlanAmendmentManifest,
        plan: &crate::product::models::WorkItemPlanLineage,
        request: &PlanRepairRequest,
    ) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
        let revision_store = WorkItemRevisionStore::new(self.store.paths());
        let current_plan =
            revision_store.get_plan_lineage(&attempt.project_id, &attempt.issue_id, &plan.id)?;
        let applied = revision_store.update_repair_request_status(
            &current_plan,
            &request.id,
            PlanRepairRequestStatus::Applied,
        )?;
        self.finalize_plan_repair_session(attempt, manifest, &applied)?;
        let current_plan =
            revision_store.get_plan_lineage(&attempt.project_id, &attempt.issue_id, &plan.id)?;
        match current_plan.active_amendment_id.as_deref() {
            Some(id) if id == manifest.id => {
                revision_store.release_active_amendment(&current_plan, &manifest.id)?;
            }
            None => {}
            _ => return Err(amendment_identity_error(&manifest.id).into()),
        }
        self.store
            .resume_attempt_after_amendment(attempt, manifest)
            .map_err(CodingWorkspaceEngineError::from)
    }

    fn finalize_plan_repair_session(
        &self,
        attempt: &CodingExecutionAttempt,
        manifest: &PlanAmendmentManifest,
        applied: &PlanRepairRequest,
    ) -> Result<(), CodingWorkspaceEngineError> {
        let lifecycle = LifecycleStore::new(self.store.paths());
        let mut links = lifecycle
            .list_session_links(&attempt.project_id, &attempt.issue_id)?
            .into_iter()
            .filter(|link| {
                link.trigger.attempt_id == attempt.id
                    && link.trigger.repair_request_id == manifest.repair_request_id
                    && link.trigger.amendment_id == manifest.id
            });
        let link = links
            .next()
            .ok_or_else(|| amendment_identity_error(&manifest.id))?;
        if links.next().is_some() {
            return Err(ProductStoreError::Ambiguous {
                kind: "coding_plan_amendment_session_link",
                id: manifest.id.clone(),
            }
            .into());
        }
        let mut snapshot = lifecycle
            .load_plan_repair_session_state(
                &attempt.project_id,
                &attempt.issue_id,
                &link.child_session_id,
            )?
            .ok_or_else(|| amendment_identity_error(&link.child_session_id))?;
        if snapshot.link != link
            || snapshot.amendment.as_ref() != Some(manifest)
            || snapshot.request.id != applied.id
        {
            return Err(amendment_identity_error(&link.child_session_id).into());
        }
        snapshot.request = applied.clone();
        snapshot.stage = PlanRepairSessionStage::Completed;
        snapshot.error = None;
        lifecycle.save_plan_repair_session_state(
            &attempt.project_id,
            &attempt.issue_id,
            &link.child_session_id,
            &snapshot,
        )?;
        lifecycle.update_workspace_session_status(
            &link.child_session_id,
            WorkspaceSessionStatus::Terminated,
        )?;
        Ok(())
    }

    async fn ensure_plan_amendment_worktree_ready(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<(), CodingWorkspaceEngineError> {
        let path = attempt
            .worktree_path
            .as_ref()
            .ok_or_else(|| CodingWorkspaceEngineError::MissingWorktree(attempt.id.clone()))?;
        if self._git_service.git_status(path).await?.is_empty() {
            return Ok(());
        }
        let already_open = self
            .store
            .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)?
            .iter()
            .any(|gate| {
                gate.reason_code.as_deref() == Some("worktree_dirty_before_plan_amendment")
            });
        if !already_open {
            self.store.create_blocked_gate(
                attempt,
                CreateBlockedGateInput {
                    attempt_id: attempt.id.clone(),
                    stage: attempt.stage.clone(),
                    node_id: None,
                    role: None,
                    title: "Worktree must be checkpointed before Plan Amendment".to_string(),
                    description:
                        "Commit or discard the current worktree diff before applying the amendment"
                            .to_string(),
                    reason_code: Some("worktree_dirty_before_plan_amendment".to_string()),
                    evidence_refs: Vec::new(),
                    raw_provider_output_ref: None,
                    available_actions: vec![
                        coding_gate_action_for_id("manual_continue")
                            .expect("manual continue action"),
                        coding_gate_action_for_id("abort").expect("abort action"),
                    ],
                },
            )?;
        }
        Err(CodingWorkspaceEngineError::SharedWorktreeDirtyManualGate(
            "worktree_dirty_before_plan_amendment".to_string(),
        ))
    }

    fn record_amendment_application_failure(
        &self,
        attempt: &CodingExecutionAttempt,
        manifest: &PlanAmendmentManifest,
        error: &CodingWorkspaceEngineError,
    ) {
        let Ok(journal) = self
            .store
            .get_amendment_application_journal(attempt, &manifest.id)
        else {
            return;
        };
        let _ =
            self.store
                .mark_amendment_application_failed(attempt, &manifest.id, error.to_string());
        let current = self
            .store
            .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id);
        if let Ok(current) = current
            && current.status == CodingAttemptStatus::ApplyingPlanAmendment
        {
            let _ = self.store.update_attempt_status(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                CodingAttemptStatus::AmendmentApplyFailed,
            );
        }
        if journal.phase != CodingAmendmentApplicationPhase::Completed {
            self.mark_plan_repair_session_apply_failed(attempt, manifest, error);
        }
    }

    fn mark_plan_repair_session_apply_failed(
        &self,
        attempt: &CodingExecutionAttempt,
        manifest: &PlanAmendmentManifest,
        error: &CodingWorkspaceEngineError,
    ) {
        let lifecycle = LifecycleStore::new(self.store.paths());
        let Ok(links) = lifecycle.list_session_links(&attempt.project_id, &attempt.issue_id) else {
            return;
        };
        let Some(link) = links.into_iter().find(|link| {
            link.trigger.attempt_id == attempt.id && link.trigger.amendment_id == manifest.id
        }) else {
            return;
        };
        let Ok(Some(mut snapshot)) = lifecycle.load_plan_repair_session_state(
            &attempt.project_id,
            &attempt.issue_id,
            &link.child_session_id,
        ) else {
            return;
        };
        snapshot.stage = PlanRepairSessionStage::AmendmentApplyFailed;
        snapshot.error = Some(error.to_string());
        let _ = lifecycle.save_plan_repair_session_state(
            &attempt.project_id,
            &attempt.issue_id,
            &link.child_session_id,
            &snapshot,
        );
    }
}

fn validate_manifest_partitions(
    manifest: &PlanAmendmentManifest,
    known_units: &BTreeSet<String>,
) -> Result<(), CodingWorkspaceEngineError> {
    let revised = manifest
        .revised_work_items
        .keys()
        .cloned()
        .collect::<BTreeSet<_>>();
    let stale = manifest
        .stale_units
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let revalidation = manifest
        .revalidation_required_units
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let unaffected = manifest
        .unaffected_units
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    if stale.len() != manifest.stale_units.len()
        || revalidation.len() != manifest.revalidation_required_units.len()
        || unaffected.len() != manifest.unaffected_units.len()
        || !stale.is_disjoint(&revalidation)
        || !stale.is_disjoint(&unaffected)
        || !revalidation.is_disjoint(&unaffected)
        || !revised.is_disjoint(&unaffected)
        || stale
            .iter()
            .chain(revalidation.iter())
            .chain(unaffected.iter())
            .any(|id| !known_units.contains(id))
        || !known_units.contains(&manifest.resume_target.logical_work_item_id)
    {
        return Err(amendment_identity_error(&manifest.id).into());
    }
    Ok(())
}

fn validate_linked_snapshot(
    attempt: &CodingExecutionAttempt,
    manifest: &PlanAmendmentManifest,
    request: &PlanRepairRequest,
    snapshot: &PlanRepairSessionSnapshotDto,
) -> Result<(), CodingWorkspaceEngineError> {
    let valid_stage = snapshot.stage == PlanRepairSessionStage::Published
        || (attempt.status == CodingAttemptStatus::AmendmentApplyFailed
            && snapshot.stage == PlanRepairSessionStage::AmendmentApplyFailed
            && snapshot.error.is_some());
    if snapshot.request != *request
        || snapshot.amendment.as_ref() != Some(manifest)
        || snapshot.link.trigger.attempt_id != attempt.id
        || snapshot.link.trigger.repair_request_id != request.id
        || snapshot.link.trigger.amendment_id != manifest.id
        || !valid_stage
    {
        return Err(amendment_identity_error(&manifest.id).into());
    }
    Ok(())
}

fn amendment_identity_error(id: &str) -> ProductStoreError {
    ProductStoreError::IdentityMismatch {
        kind: "coding_plan_amendment_application",
        id: id.to_string(),
    }
}
