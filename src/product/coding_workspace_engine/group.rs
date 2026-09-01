use super::*;
use crate::product::coding_models::{
    CodingAdmissionKind, CodingAttemptScope, CodingExecutionUnitStatus,
};
use crate::product::coding_workspace_engine::group_dependency_gate::GroupUnitSelectionOutcome;

impl CodingWorkspaceEngine {
    pub async fn complete_current_group_unit(
        &self,
        attempt: &CodingExecutionAttempt,
        summary: Option<String>,
    ) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
        if attempt.scope != CodingAttemptScope::WorkItemGroup {
            return Ok(attempt.clone());
        }

        self.validate_attempt_issue_shared_worktree_lock_if_present(attempt)?;

        let active = self
            .store
            .get_active_coding_unit(&attempt.project_id, &attempt.issue_id, &attempt.id)?
            .ok_or_else(|| CodingWorkspaceEngineError::FinalConfirmNotReady(attempt.id.clone()))?;
        self.store.update_coding_unit_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &active.id,
            CodingExecutionUnitStatus::Completed,
            summary,
        )?;

        self.advance_to_next_group_unit(attempt).await
    }

    pub async fn advance_to_next_group_unit(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
        let current_work_item_id = self.active_work_item_id_for_attempt(attempt).to_string();
        let units =
            self.store
                .list_coding_units(&attempt.project_id, &attempt.issue_id, &attempt.id)?;

        let next = if attempt.admission_kind == CodingAdmissionKind::ScAdvance {
            let outcome = self.select_next_sc_group_unit(attempt)?;
            self.persist_sc_group_dependency_gate_outcome(attempt, &outcome)?;
            match outcome {
                GroupUnitSelectionOutcome::Ready { unit_id } => {
                    let next = units.iter().find(|unit| unit.id == unit_id).cloned();
                    let Some(next) = next else {
                        return Err(CodingWorkspaceEngineError::ProviderProtocol(
                            "SC dependency selector selected an unknown unit".to_string(),
                        ));
                    };
                    if !self
                        .store
                        .start_pending_coding_unit_run(attempt, &next.id)?
                    {
                        return self
                            .store
                            .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
                            .map_err(CodingWorkspaceEngineError::from);
                    }
                    Some(next)
                }
                GroupUnitSelectionOutcome::Waiting { .. }
                | GroupUnitSelectionOutcome::FailedClosed { .. } => {
                    return self
                        .store
                        .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
                        .map_err(CodingWorkspaceEngineError::from);
                }
                GroupUnitSelectionOutcome::Complete => None,
            }
        } else {
            let mut pending_units = units
                .iter()
                .filter(|unit| unit.status == CodingExecutionUnitStatus::Pending)
                .collect::<Vec<_>>();
            pending_units.sort_by_key(|unit| unit.order_index);
            let mut next = None;
            for candidate in pending_units {
                if self
                    .store
                    .start_pending_coding_unit_run(attempt, &candidate.id)?
                {
                    next = Some(candidate.clone());
                    break;
                }
            }
            next
        };
        if let Some(next) = next {
            self.store.update_coding_unit_status(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                &next.id,
                CodingExecutionUnitStatus::Running,
                Some("进入下一个 Work Item".to_string()),
            )?;

            let mut updated =
                self.store
                    .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
            let needs_execution_admission = updated.status != CodingAttemptStatus::Running;
            updated.current_work_item_id = Some(next.logical_work_item_id.clone());
            updated.active_unit_id = Some(next.id.clone());
            updated.stage = CodingExecutionStage::PrepareContext;
            updated.updated_at = Utc::now().to_rfc3339();
            self.store.update_attempt_non_status_fields(&updated)?;
            let updated = if needs_execution_admission {
                self.store.admit_and_transition_attempt_to_executable(
                    &attempt.project_id,
                    &attempt.issue_id,
                    &attempt.id,
                )?
            } else {
                self.store
                    .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)?
            };
            let lifecycle = LifecycleStore::new(self.store.paths());
            if lifecycle
                .get_issue_shared_worktree(&attempt.project_id, &attempt.issue_id)?
                .is_some()
            {
                lifecycle.transfer_issue_worktree_lock(
                    &attempt.project_id,
                    &attempt.issue_id,
                    &current_work_item_id,
                    &next.logical_work_item_id,
                    &attempt.id,
                )?;
            }
            return Ok(updated);
        }

        self.store.update_attempt_stage(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingExecutionStage::ReviewRequest,
        )?;
        self.store
            .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .map_err(CodingWorkspaceEngineError::from)
    }

    pub fn group_attempt_ready_for_final_review(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<bool, CodingWorkspaceEngineError> {
        if attempt.scope != CodingAttemptScope::WorkItemGroup {
            return Ok(false);
        }

        Ok(self
            .store
            .list_coding_units(&attempt.project_id, &attempt.issue_id, &attempt.id)?
            .iter()
            .all(|unit| unit.status == CodingExecutionUnitStatus::Completed))
    }
}
