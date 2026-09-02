use super::*;
use crate::product::coding_models::{CodingAttemptScope, CodingExecutionUnitStatus};
use crate::product::coding_workspace_engine::group_dependency_gate::{
    GroupUnitSelectionOutcome, dependency_gate_applies,
};

#[allow(dead_code)]
pub(crate) enum GroupUnitFailureOutcome {
    RetrySameUnit {
        attempt_id: String,
        unit_id: String,
        run_no: u32,
    },
    AwaitingManualRecovery {
        attempt_id: String,
        reason_code: String,
    },
    AwaitingPlanAmendment {
        attempt_id: String,
        unit_id: String,
        finding_id: String,
    },
    Aborted {
        attempt_id: String,
        reason_code: String,
    },
}

impl CodingWorkspaceEngine {
    pub(crate) async fn handle_group_unit_failure(
        &self,
        attempt: &CodingExecutionAttempt,
        unit_id: &str,
        failure: ProviderFailureClassification,
    ) -> Result<GroupUnitFailureOutcome, CodingWorkspaceEngineError> {
        let current = self.store.validate_attempt_lineage(attempt)?;
        if current.scope != CodingAttemptScope::WorkItemGroup {
            return Err(CodingWorkspaceEngineError::ProviderProtocol(
                "group_unit_failure_requires_group_attempt".to_string(),
            ));
        }
        if current.status == CodingAttemptStatus::Aborted {
            return Ok(GroupUnitFailureOutcome::Aborted {
                attempt_id: current.id,
                reason_code: "abort_attempt".to_string(),
            });
        }
        if current.stage != CodingExecutionStage::Coding {
            return Err(CodingWorkspaceEngineError::ProviderProtocol(
                "group_unit_failure_requires_coding_stage".to_string(),
            ));
        }
        let unit = self
            .store
            .list_coding_units(&current.project_id, &current.issue_id, &current.id)?
            .into_iter()
            .find(|unit| unit.id == unit_id)
            .ok_or_else(|| ProductStoreError::NotFound {
                kind: "coding_execution_unit",
                id: unit_id.to_string(),
            })?;
        if !matches!(current.status, CodingAttemptStatus::Running) {
            let reason_code = current
                .manual_recovery_reason
                .clone()
                .unwrap_or_else(|| "attempt_blocked".to_string());
            return Ok(GroupUnitFailureOutcome::AwaitingManualRecovery {
                attempt_id: current.id,
                reason_code,
            });
        }
        let active = self.store.get_active_unit_run(&current)?;
        if active.unit_id != unit.id {
            return Err(CodingWorkspaceEngineError::Store(
                ProductStoreError::IdentityMismatch {
                    kind: "coding_active_unit_run",
                    id: unit_id.to_string(),
                },
            ));
        }
        match failure {
            ProviderFailureClassification::Retryable { .. }
                if active.operational_retry_count + 1 < MAX_PROVIDER_INVOCATIONS_PER_CYCLE =>
            {
                let retry = self
                    .store
                    .create_retry_coding_unit_run(&current, unit_id, &active.id)?;
                Ok(GroupUnitFailureOutcome::RetrySameUnit {
                    attempt_id: current.id,
                    unit_id: unit.id,
                    run_no: retry.execution_no,
                })
            }
            ProviderFailureClassification::Retryable { reason_code, .. } => {
                self.store
                    .fail_coding_unit_run(&current, unit_id, &active.id)?;
                self.store.update_coding_unit_status(
                    &current.project_id,
                    &current.issue_id,
                    &current.id,
                    unit_id,
                    CodingExecutionUnitStatus::Failed,
                    Some(reason_code.clone()),
                )?;
                self.store
                    .transition_to_awaiting_manual_recovery(&current.id, &reason_code)
                    .map_err(|error| {
                        CodingWorkspaceEngineError::ProviderProtocol(error.to_string())
                    })?;
                Ok(GroupUnitFailureOutcome::AwaitingManualRecovery {
                    attempt_id: current.id,
                    reason_code,
                })
            }
            ProviderFailureClassification::NonRetryable { reason_code, .. } => {
                self.store
                    .fail_coding_unit_run(&current, unit_id, &active.id)?;
                self.store.update_coding_unit_status(
                    &current.project_id,
                    &current.issue_id,
                    &current.id,
                    unit_id,
                    CodingExecutionUnitStatus::Blocked,
                    Some(reason_code.clone()),
                )?;
                let blocked = self.store.update_attempt_status(
                    &current.project_id,
                    &current.issue_id,
                    &current.id,
                    CodingAttemptStatus::Blocked,
                )?;
                let gate = self.store.create_blocked_gate(
                    &blocked,
                    CreateBlockedGateInput {
                        attempt_id: blocked.id.clone(),
                        stage: blocked.stage.clone(),
                        node_id: None,
                        role: Some(CodingProviderRole::Coder),
                        title: "Group unit 执行需要人工处理".to_string(),
                        description: format!("unit {} failure: {}", unit.id, reason_code),
                        reason_code: Some(reason_code.clone()),
                        evidence_refs: Vec::new(),
                        raw_provider_output_ref: None,
                        available_actions: vec![
                            coding_gate_action_for_id("retry_coding").expect("retry coding action"),
                            coding_gate_action_for_id("abort").expect("abort action"),
                        ],
                    },
                )?;
                let _ = self
                    .event_tx
                    .send(CodingWsOutMessage::CodingGateRequired { gate })
                    .await;
                Ok(GroupUnitFailureOutcome::AwaitingManualRecovery {
                    attempt_id: blocked.id,
                    reason_code,
                })
            }
        }
    }
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

        let next = if dependency_gate_applies(attempt) {
            let outcome = self.select_next_sc_group_unit(attempt)?;
            match outcome {
                GroupUnitSelectionOutcome::Ready { unit_id, audit } => {
                    let next = units.iter().find(|unit| unit.id == unit_id).cloned();
                    let Some(next) = next else {
                        let failed = GroupUnitSelectionOutcome::FailedClosed {
                            reason_code: "SC_GROUP_DEPENDENCY_UNKNOWN".to_string(),
                            message: "SC dependency selector selected an unknown unit".to_string(),
                            audit,
                        };
                        self.persist_sc_group_dependency_gate_outcome(attempt, &failed)?;
                        return self
                            .store
                            .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
                            .map_err(CodingWorkspaceEngineError::from);
                    };
                    if !self
                        .store
                        .start_pending_coding_unit_run(attempt, &next.id)?
                    {
                        let failed = GroupUnitSelectionOutcome::FailedClosed {
                            reason_code: "SC_GROUP_UNIT_RUN_NOT_STARTABLE".to_string(),
                            message: format!(
                                "SC dependency-selected unit {} was not startable",
                                next.id
                            ),
                            audit,
                        };
                        self.persist_sc_group_dependency_gate_outcome(attempt, &failed)?;
                        return self
                            .store
                            .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
                            .map_err(CodingWorkspaceEngineError::from);
                    }
                    let ready = GroupUnitSelectionOutcome::Ready {
                        unit_id: next.id.clone(),
                        audit,
                    };
                    self.persist_sc_group_dependency_gate_outcome(attempt, &ready)?;
                    Some(next)
                }
                GroupUnitSelectionOutcome::Waiting { .. }
                | GroupUnitSelectionOutcome::FailedClosed { .. } => {
                    self.persist_sc_group_dependency_gate_outcome(attempt, &outcome)?;
                    let mut persisted = self.store.get_attempt(
                        &attempt.project_id,
                        &attempt.issue_id,
                        &attempt.id,
                    )?;
                    // Dependency waiting/fail-closed is not an executable unit state. Clear
                    // only the execution-authority pointer; current_work_item_id remains the
                    // progress/display pointer for the next pending work item.
                    persisted.active_unit_id = None;
                    // Controller ruling for 6.1: retain the caller-facing progress pointer while
                    // clearing execution authority; this keeps the waiting/fail-closed view
                    // continuous even though store-side completion synchronizes durable pointers.
                    persisted.current_work_item_id = Some(current_work_item_id.clone());
                    persisted.updated_at = Utc::now().to_rfc3339();
                    self.store.update_attempt_non_status_fields(&persisted)?;
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
            let status_result = self.store.update_coding_unit_status(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                &next.id,
                CodingExecutionUnitStatus::Running,
                Some("进入下一个 Work Item".to_string()),
            );
            if let Err(ProductStoreError::Io(message)) = &status_result
                && message.starts_with("active_coding_unit_exists:")
            {
                return self
                    .store
                    .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
                    .map_err(CodingWorkspaceEngineError::from);
            }
            status_result?;

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
