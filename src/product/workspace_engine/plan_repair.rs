use chrono::Utc;

use crate::product::models::{
    PlanAmendmentManifest, PlanAmendmentPublicationPhase, PlanRepairAwaitingConfirmationPackage,
    PlanRepairRequest, PlanRepairRequestStatus, PlanRepairSessionSnapshotDto,
    PlanRepairSessionStage, WorkspaceReturnContext, WorkspaceSessionLink,
    WorkspaceSessionLinkTrigger, WorkspaceSessionRelation,
};
use crate::product::plan_repair::PlanRepairError;

use super::*;

impl WorkspaceEngine {
    pub async fn start_plan_repair(
        &mut self,
        request: PlanRepairRequest,
    ) -> Result<WorkspaceSessionRecord, PlanRepairError> {
        if self.session.workspace_type != WorkspaceType::WorkItemPlan
            || self.session.entity_id != request.plan_id
        {
            return Err(PlanRepairError::InvalidRepairTarget(
                "plan repair requires the matching WorkItemPlan workspace".to_string(),
            ));
        }
        let lifecycle = self.lifecycle_store.as_ref().ok_or_else(|| {
            PlanRepairError::Store(ProductStoreError::Io(
                "plan repair requires a persistent workspace engine".to_string(),
            ))
        })?;
        let revision_store = WorkItemRevisionStore::new(lifecycle.app_paths());
        let plan = revision_store
            .get_plan_lineage(
                &self.session.project_id,
                &self.session.issue_id,
                &request.plan_id,
            )
            .map_err(PlanRepairError::Store)?;
        if plan.active_revision_id.as_deref() != Some(request.base_plan_revision_id.as_str()) {
            return Err(PlanRepairError::AmendmentConflict {
                expected: request.base_plan_revision_id,
                actual: plan.active_revision_id.unwrap_or_default(),
            });
        }

        let open_requests = revision_store
            .list_open_repair_requests(&plan)
            .map_err(PlanRepairError::Store)?;
        let mut selected = open_requests
            .iter()
            .find(|existing| existing.fingerprint == request.fingerprint)
            .cloned();
        if let Some(existing) = selected.as_ref() {
            selected = Some(
                revision_store
                    .merge_repair_request_evidence(&plan, &existing.id, request.evidence.clone())
                    .map_err(PlanRepairError::Store)?,
            );
        }

        if selected.is_none()
            && let Some(active_amendment_id) = plan.active_amendment_id.as_deref()
        {
            let requested_amendment_id = amendment_id_for(&request.fingerprint);
            if active_amendment_id == requested_amendment_id {
                let mut recovering = request.clone();
                recovering.amendment_id = Some(active_amendment_id.to_string());
                selected = Some(recovering);
            } else {
                let active_request = open_requests
                    .iter()
                    .find(|existing| existing.amendment_id.as_deref() == Some(active_amendment_id))
                    .ok_or_else(|| PlanRepairError::ActiveAmendmentExists {
                        amendment_id: active_amendment_id.to_string(),
                    })?;
                let (link, child) = linked_child_session(
                    lifecycle,
                    &self.session.project_id,
                    &self.session.issue_id,
                    active_request,
                )
                .ok_or_else(|| PlanRepairError::ActiveAmendmentExists {
                    amendment_id: active_amendment_id.to_string(),
                })?;
                return reconcile_plan_repair_child(
                    lifecycle,
                    &self.session.project_id,
                    &self.session.issue_id,
                    active_request,
                    link,
                    child,
                );
            }
        }

        if let Some(existing) = selected.as_ref()
            && let Some((link, child)) = linked_child_session(
                lifecycle,
                &self.session.project_id,
                &self.session.issue_id,
                existing,
            )
        {
            return reconcile_plan_repair_child(
                lifecycle,
                &self.session.project_id,
                &self.session.issue_id,
                existing,
                link,
                child,
            );
        }

        let mut selected = selected.unwrap_or(request);
        let amendment_id = selected
            .amendment_id
            .clone()
            .unwrap_or_else(|| amendment_id_for(&selected.fingerprint));
        revision_store
            .acquire_active_amendment(&plan, &amendment_id)
            .map_err(|error| match plan.active_amendment_id.as_deref() {
                Some(existing) => PlanRepairError::ActiveAmendmentExists {
                    amendment_id: existing.to_string(),
                },
                None => PlanRepairError::Store(error),
            })?;

        if selected.amendment_id.is_none() {
            selected.amendment_id = Some(amendment_id.clone());
        }
        selected.status = PlanRepairRequestStatus::InProgress;
        selected.updated_at = Utc::now().to_rfc3339();
        match revision_store.get_repair_request(&plan, &selected.id) {
            Ok(_) => {
                selected = revision_store
                    .assign_repair_request_amendment(&plan, &selected.id, &amendment_id)
                    .map_err(PlanRepairError::Store)?;
                selected = revision_store
                    .update_repair_request_status(
                        &plan,
                        &selected.id,
                        PlanRepairRequestStatus::InProgress,
                    )
                    .map_err(PlanRepairError::Store)?;
            }
            Err(ProductStoreError::NotFound { .. }) => revision_store
                .put_repair_request(&plan, &selected)
                .map_err(PlanRepairError::Store)?,
            Err(error) => return Err(PlanRepairError::Store(error)),
        }

        let child_session_id = child_session_id_for(&amendment_id);
        let child = lifecycle
            .create_workspace_session_with_id(
                CreateWorkspaceSessionInput {
                    project_id: self.session.project_id.clone(),
                    issue_id: self.session.issue_id.clone(),
                    entity_id: request_plan_id(&selected),
                    workspace_type: WorkspaceType::WorkItemPlan,
                    author_provider: self.session.author_provider.clone(),
                    reviewer_provider: self
                        .session
                        .reviewer_provider
                        .clone()
                        .unwrap_or_else(|| self.session.author_provider.clone()),
                    review_rounds: self.session.review_rounds,
                    superpowers_enabled: self.session.superpowers_enabled,
                    openspec_enabled: self.session.openspec_enabled,
                },
                child_session_id,
            )
            .map_err(PlanRepairError::Store)?;
        let link = WorkspaceSessionLink {
            id: link_id_for(&amendment_id),
            relation: WorkspaceSessionRelation::PlanRepair,
            parent_session_id: selected.trigger_attempt_id.clone(),
            child_session_id: child.id.clone(),
            trigger: WorkspaceSessionLinkTrigger {
                attempt_id: selected.trigger_attempt_id.clone(),
                unit_run_id: selected.trigger_unit_run_id.clone(),
                review_id: selected.trigger_review_id.clone(),
                finding_id: selected.trigger_finding_id.clone(),
                repair_request_id: selected.id.clone(),
                amendment_id: amendment_id.clone(),
                fingerprint: selected.fingerprint.clone(),
                base_plan_revision_id: selected.base_plan_revision_id.clone(),
            },
            return_context: WorkspaceReturnContext {
                original_attempt_id: selected.trigger_attempt_id.clone(),
                original_unit_run_id: selected.trigger_unit_run_id.clone(),
                timeline_anchor_id: selected.trigger_finding_id.clone(),
                original_route: format!(
                    "/workbench/projects/{}/issues/{}/coding/{}",
                    self.session.project_id, self.session.issue_id, selected.trigger_attempt_id
                ),
            },
            created_at: Utc::now().to_rfc3339(),
        };
        lifecycle
            .put_session_link(&self.session.project_id, &self.session.issue_id, &link)
            .map_err(PlanRepairError::Store)?;
        reconcile_plan_repair_child(
            lifecycle,
            &self.session.project_id,
            &self.session.issue_id,
            &selected,
            link,
            child,
        )
    }

    pub fn plan_repair_session_state(&self) -> Option<&PlanRepairSessionSnapshotDto> {
        self.plan_repair_snapshot.as_ref()
    }

    pub async fn enter_plan_repair_awaiting_confirmation(
        &mut self,
        package: PlanRepairAwaitingConfirmationPackage,
    ) -> Result<(), PlanRepairError> {
        self.recover_pending_plan_repair_transition()?;
        let snapshot = self.require_plan_repair_snapshot()?.clone();
        let lifecycle = self.persistent_lifecycle()?;
        let revision_store = WorkItemRevisionStore::new(lifecycle.app_paths());
        let plan = revision_store
            .get_plan_lineage(
                &self.session.project_id,
                &self.session.issue_id,
                &snapshot.request.plan_id,
            )
            .map_err(PlanRepairError::Store)?;
        validate_awaiting_confirmation_package(&snapshot, &plan, &package)?;
        if snapshot.stage == PlanRepairSessionStage::AwaitingConfirmation {
            if snapshot.projection.as_ref() == Some(&package.projection)
                && snapshot.amendment.as_ref() == Some(&package.amendment)
                && snapshot.validation.as_ref() == Some(&package.validation)
                && snapshot.impact.as_ref() == Some(&package.impact)
                && snapshot.plan_review.as_ref() == Some(&package.plan_review)
            {
                return Ok(());
            }
            return Err(PlanRepairError::InvalidRepairTarget(
                "awaiting-confirmation package conflicts with persisted repair state".to_string(),
            ));
        }
        let journal = awaiting_confirmation_transition(self, snapshot, package);
        self.commit_plan_repair_transition(journal)
    }

    pub async fn confirm_plan_amendment(
        &mut self,
        amendment_id: &str,
    ) -> Result<PlanAmendmentManifest, PlanRepairError> {
        self.recover_pending_plan_repair_transition()?;
        let snapshot = self.require_plan_repair_snapshot()?.clone();
        if snapshot.stage != PlanRepairSessionStage::AwaitingConfirmation {
            return Err(PlanRepairError::ConfirmationRequired);
        }
        let amendment = snapshot
            .amendment
            .clone()
            .ok_or(PlanRepairError::ConfirmationRequired)?;
        if amendment.id != amendment_id {
            return Err(PlanRepairError::AmendmentConflict {
                expected: amendment.id,
                actual: amendment_id.to_string(),
            });
        }
        let confirmation = self
            .timeline_nodes
            .iter()
            .find(|node| node.node_type == TimelineNodeType::PlanAmendmentConfirmation)
            .cloned()
            .ok_or(PlanRepairError::ConfirmationRequired)?;
        if confirmation.status != TimelineNodeStatus::Completed {
            let journal = confirmation_transition(self, snapshot);
            self.commit_plan_repair_transition(journal)?;
        }
        Ok(amendment)
    }

    pub async fn cancel_plan_amendment(
        &mut self,
        amendment_id: &str,
        reason: Option<String>,
    ) -> Result<(), PlanRepairError> {
        self.recover_pending_plan_repair_transition()?;
        let original_snapshot = self.require_plan_repair_snapshot()?.clone();
        let amendment = original_snapshot
            .amendment
            .as_ref()
            .ok_or(PlanRepairError::ConfirmationRequired)?;
        if amendment.id != amendment_id {
            return Err(PlanRepairError::AmendmentConflict {
                expected: amendment.id.clone(),
                actual: amendment_id.to_string(),
            });
        }
        if original_snapshot.request.status == PlanRepairRequestStatus::Cancelled {
            return Ok(());
        }
        if matches!(
            original_snapshot.stage,
            PlanRepairSessionStage::Published
                | PlanRepairSessionStage::ApplyingAmendment
                | PlanRepairSessionStage::AmendmentApplyFailed
                | PlanRepairSessionStage::Completed
        ) || matches!(
            original_snapshot.request.status,
            PlanRepairRequestStatus::Published | PlanRepairRequestStatus::Applied
        ) {
            return Err(plan_published_conflict());
        }

        let lifecycle = self.persistent_lifecycle()?;
        let revision_store = WorkItemRevisionStore::new(lifecycle.app_paths());
        let plan = revision_store
            .get_plan_lineage(
                &self.session.project_id,
                &self.session.issue_id,
                &original_snapshot.request.plan_id,
            )
            .map_err(PlanRepairError::Store)?;
        match revision_store.find_plan_amendment_publication_journal(&plan, amendment_id) {
            Ok(Some(journal)) if journal.phase == PlanAmendmentPublicationPhase::PlanPublished => {
                return Err(plan_published_conflict());
            }
            Ok(_) => {}
            Err(error) => return Err(PlanRepairError::Store(error)),
        }

        let cancel_summary = reason
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .map(|value| format!("用户取消：{value}"))
            .unwrap_or_else(|| "用户取消 Plan Amendment".to_string());
        let journal = cancellation_transition(self, original_snapshot, cancel_summary);
        self.commit_plan_repair_transition(journal)
    }

    fn require_plan_repair_snapshot(
        &self,
    ) -> Result<&PlanRepairSessionSnapshotDto, PlanRepairError> {
        self.plan_repair_snapshot.as_ref().ok_or_else(|| {
            PlanRepairError::InvalidRepairTarget(
                "workspace session is not a plan repair child".to_string(),
            )
        })
    }

    fn persistent_lifecycle(&self) -> Result<LifecycleStore, PlanRepairError> {
        self.lifecycle_store.clone().ok_or_else(|| {
            PlanRepairError::Store(ProductStoreError::Io(
                "plan repair requires a persistent workspace engine".to_string(),
            ))
        })
    }
}

fn amendment_id_for(fingerprint: &str) -> String {
    format!("plan_amendment_{fingerprint}")
}

fn child_session_id_for(amendment_id: &str) -> String {
    format!("workspace_session_{amendment_id}")
}

fn link_id_for(amendment_id: &str) -> String {
    format!("workspace_session_link_{amendment_id}")
}

fn plan_published_conflict() -> PlanRepairError {
    PlanRepairError::AmendmentConflict {
        expected: "before_plan_published".to_string(),
        actual: "plan_published".to_string(),
    }
}

fn request_plan_id(request: &PlanRepairRequest) -> String {
    request.plan_id.clone()
}

fn linked_child_session(
    lifecycle: &LifecycleStore,
    project_id: &str,
    issue_id: &str,
    request: &PlanRepairRequest,
) -> Option<(WorkspaceSessionLink, WorkspaceSessionRecord)> {
    lifecycle
        .list_session_links(project_id, issue_id)
        .ok()?
        .into_iter()
        .find(|link| {
            link.relation == WorkspaceSessionRelation::PlanRepair
                && link.trigger.attempt_id == request.trigger_attempt_id
                && link.trigger.unit_run_id == request.trigger_unit_run_id
                && link.trigger.review_id == request.trigger_review_id
                && link.trigger.finding_id == request.trigger_finding_id
                && link.trigger.repair_request_id == request.id
                && request.amendment_id.as_deref() == Some(link.trigger.amendment_id.as_str())
                && link.trigger.fingerprint == request.fingerprint
                && link.trigger.base_plan_revision_id == request.base_plan_revision_id
                && link.child_session_id == child_session_id_for(&link.trigger.amendment_id)
                && link.id == link_id_for(&link.trigger.amendment_id)
        })
        .and_then(|link| {
            lifecycle
                .get_workspace_session(&link.child_session_id)
                .ok()
                .map(|session| (link, session))
        })
}
