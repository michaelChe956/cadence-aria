use std::collections::BTreeSet;

use chrono::Utc;

use super::*;
use crate::product::coding_models::{
    CodingAmendmentApplicationJournal, CodingAmendmentApplicationPhase, CodingAttemptScope,
    CodingPlanAmendmentDeliveryStatus, PlanAmendmentContext, PlanAmendmentContextStatus,
};
use crate::product::models::{
    AmendmentResumeMode, PlanAmendmentManifest, PlanRepairRequest, PlanRepairRequestStatus,
    PlanRepairSessionSnapshotDto, PlanRepairSessionStage, WorkspaceSessionStatus,
};
use crate::product::work_item_revision_store::WorkItemRevisionStore;
use crate::web::coding_ws_handler::delivery_ack::register_plan_amendment_socket_write;

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
        let _arbitration = self
            .store
            .acquire_amendment_application_arbitration(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
            )
            .await?;
        self.apply_plan_amendment_locked(attempt, manifest).await
    }

    async fn apply_plan_amendment_locked(
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
        self.validate_amendment_application_identity(
            &current,
            manifest,
            journal.phase == CodingAmendmentApplicationPhase::Completed,
        )?;
        self.validate_amendment_application_prefix(&current, manifest, &journal)?;
        if journal.phase != CodingAmendmentApplicationPhase::Completed {
            self.ensure_plan_amendment_worktree_ready(&current).await?;
        }
        let result = self
            .apply_plan_amendment_from_journal(&current, manifest, &mut journal)
            .await;
        match result {
            Ok(updated) => Ok(updated),
            Err(error) => {
                if matches!(
                    error,
                    CodingWorkspaceEngineError::Store(
                        ProductStoreError::IdentityMismatch { .. }
                            | ProductStoreError::Ambiguous { .. }
                    )
                ) {
                    return Err(error);
                }
                self.record_amendment_application_failure(&current, manifest, &error);
                Err(error)
            }
        }
    }

    pub async fn recover_plan_amendment(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
        self.recover_plan_amendment_with_history_session(attempt)
            .await
            .map(|(attempt, _)| attempt)
    }

    /// REQ-GCE-03: resume the ORIGINAL group attempt after an amendment is
    /// approved. The explicit `PlanAmendmentContext` anchors the original plan
    /// session gate, the previous plan revision and the manifest-driven resume
    /// target; incompatible revisions fail the context closed (durable) and
    /// never switch to another plan, legacy admission, or a new attempt.
    pub(crate) async fn resume_group_after_plan_amendment(
        &self,
        attempt: &CodingExecutionAttempt,
        context: &PlanAmendmentContext,
        manifest: &PlanAmendmentManifest,
    ) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
        let _ = (attempt, context, manifest);
        Err(CodingWorkspaceEngineError::Store(
            ProductStoreError::NotFound {
                kind: "coding_plan_amendment_context",
                id: "resume_not_wired".to_string(),
            },
        ))
    }

    pub(crate) async fn recover_plan_amendment_with_history_session(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<(CodingExecutionAttempt, String), CodingWorkspaceEngineError> {
        let _arbitration = self
            .store
            .acquire_amendment_application_arbitration(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
            )
            .await?;
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
            None => self.current_amendment_journal_id(&current, &plan, &revision_store)?,
        };
        let manifest = revision_store.get_amendment_manifest(&plan, &amendment_id)?;
        let updated = self
            .apply_plan_amendment_locked(&current, &manifest)
            .await?;
        let child_session_id = self.completed_amendment_child_session_id(&current, &manifest)?;
        Ok((updated, child_session_id))
    }

    fn current_amendment_journal_id(
        &self,
        attempt: &CodingExecutionAttempt,
        plan: &crate::product::models::WorkItemPlanLineage,
        revision_store: &WorkItemRevisionStore,
    ) -> Result<String, CodingWorkspaceEngineError> {
        let mut candidates = Vec::new();
        for journal in self.store.list_amendment_application_journals(attempt)? {
            let manifest = revision_store.get_amendment_manifest(plan, &journal.amendment_id)?;
            let is_current_revision =
                plan.active_revision_id.as_deref() == Some(manifest.new_plan_revision_id.as_str());
            let delivered_current = journal.phase == CodingAmendmentApplicationPhase::Completed
                && attempt.status == CodingAttemptStatus::Running
                && self
                    .store
                    .get_plan_amendment_delivery(attempt, &journal.amendment_id)
                    .is_ok_and(|delivery| {
                        delivery.status == CodingPlanAmendmentDeliveryStatus::Delivered
                    });
            let is_unfinished = journal.phase != CodingAmendmentApplicationPhase::Completed
                || matches!(
                    attempt.status,
                    CodingAttemptStatus::ApplyingPlanAmendment
                        | CodingAttemptStatus::AmendmentApplyFailed
                )
                || (attempt.status == CodingAttemptStatus::AwaitingPlanAmendment
                    && manifest.resume_target.mode == AmendmentResumeMode::AwaitHandoff)
                || delivered_current;
            if is_current_revision && is_unfinished {
                candidates.push(journal.amendment_id);
            }
        }
        match candidates.as_slice() {
            [amendment_id] => Ok(amendment_id.clone()),
            _ => Err(amendment_identity_error(&attempt.id).into()),
        }
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
            let revision_store = WorkItemRevisionStore::new(self.store.paths());
            let materialization_head_commit = journal.materialization_head_commit.clone();
            *journal = revision_store.with_active_amendment_identity(
                &authority.plan,
                &manifest.id,
                &manifest.new_plan_revision_id,
                || {
                    self.store.materialize_unit_runs_from_manifest(
                        attempt,
                        manifest,
                        materialization_head_commit.as_deref(),
                    )?;
                    self.store.advance_amendment_application_journal(
                        attempt,
                        &manifest.id,
                        CodingAmendmentApplicationPhase::UnitRunsWritten,
                        None,
                        Utc::now().to_rfc3339(),
                    )
                },
            )?;
        }
        if journal.phase.order() < CodingAmendmentApplicationPhase::ResumeTargetWritten.order() {
            let revision_store = WorkItemRevisionStore::new(self.store.paths());
            *journal = revision_store.with_active_amendment_identity(
                &authority.plan,
                &manifest.id,
                &manifest.new_plan_revision_id,
                || {
                    self.store
                        .set_resume_target_from_manifest(attempt, manifest)?;
                    self.store.advance_amendment_application_journal(
                        attempt,
                        &manifest.id,
                        CodingAmendmentApplicationPhase::ResumeTargetWritten,
                        None,
                        Utc::now().to_rfc3339(),
                    )
                },
            )?;
        }
        if journal.phase.order() < CodingAmendmentApplicationPhase::Completed.order() {
            let revision_store = WorkItemRevisionStore::new(self.store.paths());
            *journal = revision_store.with_active_amendment_identity(
                &authority.plan,
                &manifest.id,
                &manifest.new_plan_revision_id,
                || {
                    self.store.advance_amendment_application_journal(
                        attempt,
                        &manifest.id,
                        CodingAmendmentApplicationPhase::Completed,
                        None,
                        Utc::now().to_rfc3339(),
                    )
                },
            )?;
        }

        self.finalize_completed_amendment_application(
            attempt,
            manifest,
            &authority.plan,
            &authority.request,
        )?;
        self.reconcile_plan_amendment_delivery(attempt, manifest)
            .await?;
        self.store
            .resume_attempt_after_amendment(attempt, manifest)
            .map_err(CodingWorkspaceEngineError::from)
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

    fn validate_amendment_application_prefix(
        &self,
        attempt: &CodingExecutionAttempt,
        manifest: &PlanAmendmentManifest,
        journal: &CodingAmendmentApplicationJournal,
    ) -> Result<(), CodingWorkspaceEngineError> {
        if journal.phase.order() >= CodingAmendmentApplicationPhase::PlanBindingWritten.order() {
            let binding = self.store.get_plan_binding(attempt)?;
            if binding.bound_plan_revision_id != manifest.new_plan_revision_id
                || binding.applied_amendment_ids.last() != Some(&manifest.id)
            {
                return Err(amendment_identity_error(&manifest.id).into());
            }
        }
        if journal.phase.order() >= CodingAmendmentApplicationPhase::UnitRunsWritten.order() {
            self.store.validate_materialized_unit_runs_from_manifest(
                attempt,
                manifest,
                journal.phase.order()
                    >= CodingAmendmentApplicationPhase::ResumeTargetWritten.order(),
                journal.phase == CodingAmendmentApplicationPhase::Completed,
                journal.materialization_head_commit.as_deref(),
            )?;
        }
        if journal.phase == CodingAmendmentApplicationPhase::Completed {
            self.validate_completed_amendment_session_prefix(attempt, manifest)?;
        }
        Ok(())
    }

    fn validate_completed_amendment_session_prefix(
        &self,
        attempt: &CodingExecutionAttempt,
        manifest: &PlanAmendmentManifest,
    ) -> Result<(), CodingWorkspaceEngineError> {
        self.completed_amendment_child_session_id(attempt, manifest)
            .map(|_| ())
    }

    fn completed_amendment_child_session_id(
        &self,
        attempt: &CodingExecutionAttempt,
        manifest: &PlanAmendmentManifest,
    ) -> Result<String, CodingWorkspaceEngineError> {
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
        let snapshot = lifecycle
            .load_plan_repair_session_state(
                &attempt.project_id,
                &attempt.issue_id,
                &link.child_session_id,
            )?
            .ok_or_else(|| amendment_identity_error(&link.child_session_id))?;
        if snapshot.link != link
            || snapshot.amendment.as_ref() != Some(manifest)
            || snapshot.request.id != manifest.repair_request_id
            || snapshot.request.trigger_attempt_id != attempt.id
            || snapshot.request.amendment_id.as_deref() != Some(manifest.id.as_str())
        {
            return Err(amendment_identity_error(&link.child_session_id).into());
        }
        Ok(link.child_session_id)
    }

    fn finalize_completed_amendment_application(
        &self,
        attempt: &CodingExecutionAttempt,
        manifest: &PlanAmendmentManifest,
        plan: &crate::product::models::WorkItemPlanLineage,
        request: &PlanRepairRequest,
    ) -> Result<(), CodingWorkspaceEngineError> {
        let revision_store = WorkItemRevisionStore::new(self.store.paths());
        let current_plan =
            revision_store.get_plan_lineage(&attempt.project_id, &attempt.issue_id, &plan.id)?;
        let applied =
            revision_store.compare_and_mark_repair_request_applied(&current_plan, request)?;
        self.finalize_plan_repair_session(attempt, manifest, &applied)?;
        let current_plan =
            revision_store.get_plan_lineage(&attempt.project_id, &attempt.issue_id, &plan.id)?;
        revision_store.compare_and_release_applied_amendment(
            &current_plan,
            &manifest.id,
            &manifest.new_plan_revision_id,
        )?;
        Ok(())
    }

    async fn reconcile_plan_amendment_delivery(
        &self,
        attempt: &CodingExecutionAttempt,
        manifest: &PlanAmendmentManifest,
    ) -> Result<(), CodingWorkspaceEngineError> {
        let delivery = self
            .store
            .load_or_prepare_plan_amendment_delivery(attempt, &manifest.id)?;
        if delivery.status == CodingPlanAmendmentDeliveryStatus::Delivered {
            return Ok(());
        }
        let socket_write = register_plan_amendment_socket_write(&delivery.event_id)?;
        self.event_tx
            .send(CodingWsOutMessage::PlanAmendmentUpdated {
                event_id: delivery.event_id.clone(),
                amendment: Box::new(manifest.clone()),
            })
            .await
            .map_err(|_| {
                ProductStoreError::Io("plan_amendment_delivery_send_failed".to_string())
            })?;
        tokio::select! {
            result = socket_write.wait_or_channel_closed(self.event_tx.raw_sender()) => result?,
            _ = self.cancellation.cancelled() => {
                return Err(CodingWorkspaceEngineError::Aborted);
            }
        }
        self.store.mark_plan_amendment_delivery_delivered(
            attempt,
            &manifest.id,
            &delivery.event_id,
        )?;
        Ok(())
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
        let session = lifecycle.get_workspace_session(&link.child_session_id)?;
        if snapshot.link != link
            || snapshot.amendment.as_ref() != Some(manifest)
            || snapshot.request.id != applied.id
            || session.project_id != attempt.project_id
            || session.issue_id != attempt.issue_id
        {
            return Err(amendment_identity_error(&link.child_session_id).into());
        }
        let expected_snapshot = snapshot.clone();
        snapshot.request = applied.clone();
        snapshot.stage = PlanRepairSessionStage::Completed;
        snapshot.error = None;
        lifecycle.compare_and_save_plan_repair_session_state(
            &attempt.project_id,
            &attempt.issue_id,
            &link.child_session_id,
            &expected_snapshot,
            &snapshot,
        )?;
        lifecycle.compare_and_update_workspace_session_status(
            &session,
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
    let replacement_sources = manifest
        .replacement_units
        .keys()
        .cloned()
        .collect::<BTreeSet<_>>();
    if stale.len() != manifest.stale_units.len()
        || revalidation.len() != manifest.revalidation_required_units.len()
        || unaffected.len() != manifest.unaffected_units.len()
        || !stale.is_disjoint(&revalidation)
        || !stale.is_disjoint(&unaffected)
        || !revalidation.is_disjoint(&unaffected)
        || !revised.is_disjoint(&unaffected)
        || !revised.is_disjoint(&replacement_sources)
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
