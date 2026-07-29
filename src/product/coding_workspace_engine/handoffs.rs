use super::*;
use crate::product::coding_models::{CodingAttemptScope, CodingStageGateStatus};

impl CodingWorkspaceEngine {
    pub(crate) fn active_work_item_id_for_attempt<'a>(
        &self,
        attempt: &'a CodingExecutionAttempt,
    ) -> &'a str {
        attempt
            .current_work_item_id
            .as_deref()
            .unwrap_or(&attempt.work_item_id)
    }

    pub async fn handle_final_confirm(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
        let current = self.store.get_attempt(project_id, issue_id, attempt_id)?;
        if current.status != CodingAttemptStatus::WaitingForHuman
            || current.stage != CodingExecutionStage::FinalConfirm
        {
            return Err(CodingWorkspaceEngineError::FinalConfirmNotReady(
                attempt_id.to_string(),
            ));
        }
        if current.scope == CodingAttemptScope::WorkItemGroup
            && !self.group_attempt_ready_for_final_review(&current)?
        {
            return Err(CodingWorkspaceEngineError::FinalConfirmNotReady(
                attempt_id.to_string(),
            ));
        }
        self.validate_attempt_issue_shared_worktree_lock_if_present(&current)?;
        match current.scope {
            CodingAttemptScope::WorkItemGroup => {
                self.run_group_completion_gates(&current).await?;
            }
            CodingAttemptScope::WorkItem => {
                self.record_work_item_completion_commit_if_present(&current)?;
                self.run_completion_gates(&current).await?;
            }
        }

        let updated = self.store.update_attempt_status(
            project_id,
            issue_id,
            attempt_id,
            CodingAttemptStatus::Completed,
        )?;
        if updated.scope == CodingAttemptScope::WorkItemGroup {
            self.mark_completed_group_work_items_if_present(&updated)?;
            let current_work_item_id = self.active_work_item_id_for_attempt(&current).to_string();
            self.release_issue_shared_worktree_lock_if_holder(
                project_id,
                issue_id,
                &current_work_item_id,
                &updated.id,
            )?;
        } else {
            let current_work_item_id = self.active_work_item_id_for_attempt(&updated);
            LifecycleStore::new(self.store.paths()).update_work_item_execution_status(
                &updated.project_id,
                &updated.issue_id,
                current_work_item_id,
                WorkItemStatus::Completed,
            )?;
            self.mark_issue_shared_worktree_completed_if_present(
                project_id,
                issue_id,
                current_work_item_id,
                &updated.id,
            )?;
        }
        if let Some(node_id) =
            self.active_final_confirm_node_id(project_id, issue_id, attempt_id)?
        {
            let completed_at = Utc::now().to_rfc3339();
            self.store.update_timeline_node_status(
                project_id,
                issue_id,
                attempt_id,
                &node_id,
                CodingTimelineNodeStatus::Completed,
                Some("用户已确认完成".to_string()),
                Some(completed_at.clone()),
            )?;
            let _ = self
                .event_tx
                .send(CodingWsOutMessage::CodingTimelineNodeUpdated {
                    node_id,
                    status: CodingTimelineNodeStatus::Completed,
                    summary: Some("用户已确认完成".to_string()),
                    completed_at: Some(completed_at),
                })
                .await;
        }
        Ok(updated)
    }

    pub async fn handle_abort(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
        let current = self.store.get_attempt(project_id, issue_id, attempt_id)?;
        let current = self
            .reconcile_coding_git_operation_for_termination(&current)
            .await?;
        if current.status == CodingAttemptStatus::Aborted {
            return Ok(current);
        }
        let active_work_item_id = self.active_work_item_id_for_attempt(&current).to_string();
        self.validate_attempt_issue_shared_worktree_lock_if_present(&current)?;
        self.ensure_issue_shared_worktree_clean(&current, &active_work_item_id)
            .await?;

        for gate in self
            .store
            .list_open_stage_gates(project_id, issue_id, attempt_id)?
        {
            self.store.update_stage_gate_status(
                project_id,
                issue_id,
                attempt_id,
                &gate.gate_id,
                CodingStageGateStatus::Cancelled,
            )?;
        }

        let updated = self.store.update_attempt_status(
            project_id,
            issue_id,
            attempt_id,
            CodingAttemptStatus::Aborted,
        )?;
        self.release_issue_shared_worktree_lock_if_holder(
            project_id,
            issue_id,
            &active_work_item_id,
            attempt_id,
        )?;
        if let Some(node_id) = self.active_timeline_node_id(project_id, issue_id, attempt_id)? {
            let completed_at = Utc::now().to_rfc3339();
            self.store.update_timeline_node_status(
                project_id,
                issue_id,
                attempt_id,
                &node_id,
                CodingTimelineNodeStatus::Failed,
                Some("用户已中止".to_string()),
                Some(completed_at.clone()),
            )?;
            let _ = self
                .event_tx
                .send(CodingWsOutMessage::CodingTimelineNodeUpdated {
                    node_id,
                    status: CodingTimelineNodeStatus::Failed,
                    summary: Some("用户已中止".to_string()),
                    completed_at: Some(completed_at),
                })
                .await;
        }
        Ok(updated)
    }

    pub async fn handle_attempt_failed(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
        let current = self.store.get_attempt(project_id, issue_id, attempt_id)?;
        self.validate_attempt_issue_shared_worktree_lock_if_present(&current)?;
        let updated = if current.status != CodingAttemptStatus::Failed {
            self.store.update_attempt_status(
                project_id,
                issue_id,
                attempt_id,
                CodingAttemptStatus::Failed,
            )?
        } else {
            current
        };

        self.ensure_issue_shared_worktree_clean(
            &updated,
            self.active_work_item_id_for_attempt(&updated),
        )
        .await?;
        self.release_issue_shared_worktree_lock_if_holder(
            project_id,
            issue_id,
            self.active_work_item_id_for_attempt(&updated),
            &updated.id,
        )?;
        Ok(updated)
    }

    pub async fn handle_delete_attempt(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<(), CodingWorkspaceEngineError> {
        let current = self.store.get_attempt(project_id, issue_id, attempt_id)?;
        let active_work_item_id = self.active_work_item_id_for_attempt(&current).to_string();
        self.validate_attempt_issue_shared_worktree_lock_if_present(&current)?;
        if current.status.is_active() {
            self.store.update_attempt_status(
                project_id,
                issue_id,
                attempt_id,
                CodingAttemptStatus::Aborted,
            )?;
        }
        self.release_issue_shared_worktree_lock_if_holder(
            project_id,
            issue_id,
            &active_work_item_id,
            attempt_id,
        )?;
        Ok(())
    }

    /// 记录 work item 的完成 commit。
    ///
    /// 交接摘要移除后，`completion_commit` 仍需写入：它由依赖交接引用展示消费，
    /// 与被移除的摘要无关。
    pub(crate) fn record_work_item_completion_commit_if_present(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<(), CodingWorkspaceEngineError> {
        let lifecycle = LifecycleStore::new(self.store.paths());
        let current_work_item_id = self.active_work_item_id_for_attempt(attempt);
        if lifecycle
            .list_work_items(&attempt.project_id, &attempt.issue_id)?
            .iter()
            .any(|item| item.id == current_work_item_id)
        {
            lifecycle.update_work_item_completion_commit(
                &attempt.project_id,
                &attempt.issue_id,
                current_work_item_id,
                attempt.head_commit.clone(),
            )?;
        }
        Ok(())
    }

    pub(crate) async fn complete_attempt_after_final_rework(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
        self.validate_attempt_issue_shared_worktree_lock_if_present(attempt)?;
        self.record_work_item_completion_commit_if_present(attempt)?;
        self.run_completion_gates(attempt).await?;
        let staged = self.store.update_attempt_stage(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingExecutionStage::FinalConfirm,
        )?;
        let completed = self.store.update_attempt_status(
            &staged.project_id,
            &staged.issue_id,
            &staged.id,
            CodingAttemptStatus::Completed,
        )?;
        self.mark_work_item_completed_if_present(&completed)?;
        self.mark_issue_shared_worktree_completed_if_present(
            &attempt.project_id,
            &attempt.issue_id,
            self.active_work_item_id_for_attempt(attempt),
            &attempt.id,
        )?;
        let node = self.create_completed_final_confirm_timeline_node(&completed)?;
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::CodingTimelineNodeCreated { node })
            .await;
        Ok(completed)
    }

    pub(crate) async fn complete_attempt_after_review_request(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
        self.validate_attempt_issue_shared_worktree_lock_if_present(attempt)?;
        self.record_work_item_completion_commit_if_present(attempt)?;
        self.run_completion_gates(attempt).await?;
        let completed = self.store.update_attempt_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingAttemptStatus::Completed,
        )?;
        self.mark_work_item_completed_if_present(&completed)?;
        self.mark_issue_shared_worktree_completed_if_present(
            &completed.project_id,
            &completed.issue_id,
            self.active_work_item_id_for_attempt(&completed),
            &completed.id,
        )?;
        Ok(completed)
    }

    pub(crate) async fn complete_group_attempt_after_final_review(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
        if attempt.scope != CodingAttemptScope::WorkItemGroup {
            return Err(CodingWorkspaceEngineError::FinalConfirmNotReady(
                attempt.id.clone(),
            ));
        }
        self.validate_attempt_issue_shared_worktree_lock_if_present(attempt)?;
        self.run_group_completion_gates(attempt).await?;
        let completed = self.store.update_attempt_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingAttemptStatus::Completed,
        )?;
        self.mark_completed_group_work_items_if_present(&completed)?;
        let current_work_item_id = self.active_work_item_id_for_attempt(attempt).to_string();
        self.release_issue_shared_worktree_lock_if_holder(
            &attempt.project_id,
            &attempt.issue_id,
            &current_work_item_id,
            &attempt.id,
        )?;
        Ok(completed)
    }

    fn mark_completed_group_work_items_if_present(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<(), CodingWorkspaceEngineError> {
        let completed_units = self
            .store
            .list_coding_units(&attempt.project_id, &attempt.issue_id, &attempt.id)?
            .into_iter()
            .filter(|unit| {
                unit.status == crate::product::coding_models::CodingExecutionUnitStatus::Completed
            })
            .collect::<Vec<_>>();
        if self.schema_v2_group_plan_lineage(attempt)?.is_some() {
            if let Some(last_completed) = completed_units.iter().max_by_key(|unit| unit.order_index)
            {
                self.mark_issue_shared_worktree_completed_if_present(
                    &attempt.project_id,
                    &attempt.issue_id,
                    &last_completed.logical_work_item_id,
                    &attempt.id,
                )?;
            }
            return Ok(());
        }
        let lifecycle = LifecycleStore::new(self.store.paths());
        let existing_work_item_ids = lifecycle
            .list_work_items(&attempt.project_id, &attempt.issue_id)?
            .into_iter()
            .map(|work_item| work_item.id)
            .collect::<std::collections::HashSet<_>>();
        for unit in completed_units {
            if existing_work_item_ids.contains(&unit.logical_work_item_id) {
                lifecycle.update_work_item_execution_status(
                    &attempt.project_id,
                    &attempt.issue_id,
                    &unit.logical_work_item_id,
                    WorkItemStatus::Completed,
                )?;
                self.mark_issue_shared_worktree_completed_if_present(
                    &attempt.project_id,
                    &attempt.issue_id,
                    &unit.logical_work_item_id,
                    &attempt.id,
                )?;
            }
        }
        Ok(())
    }

    pub(crate) fn mark_work_item_completed_if_present(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<(), ProductStoreError> {
        let lifecycle = LifecycleStore::new(self.store.paths());
        let exists = lifecycle
            .list_work_items(&attempt.project_id, &attempt.issue_id)?
            .iter()
            .any(|work_item| work_item.id == attempt.work_item_id);
        if exists {
            lifecycle.update_work_item_execution_status(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.work_item_id,
                WorkItemStatus::Completed,
            )?;
        }
        Ok(())
    }
}
