use super::*;
use crate::product::coding_models::CodingAttemptScope;

impl CodingWorkspaceEngine {
    fn current_work_item_id_for_handoff<'a>(&self, attempt: &'a CodingExecutionAttempt) -> &'a str {
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

        self.generate_and_save_work_item_handoff_if_missing(&current)
            .await?;
        self.run_completion_gates(&current).await?;

        let updated = self.store.update_attempt_status(
            project_id,
            issue_id,
            attempt_id,
            CodingAttemptStatus::Completed,
        )?;
        if updated.scope == CodingAttemptScope::WorkItemGroup {
            self.mark_completed_group_work_items_if_present(&updated)?;
            let current_work_item_id = self.current_work_item_id_for_handoff(&current).to_string();
            self.release_issue_shared_worktree_lock_if_holder(
                project_id,
                issue_id,
                &current_work_item_id,
            )?;
        } else {
            let current_work_item_id = self.current_work_item_id_for_handoff(&updated);
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
        self.ensure_issue_shared_worktree_clean(
            project_id,
            issue_id,
            attempt_id,
            &current.work_item_id,
        )
        .await?;

        let updated = self.store.update_attempt_status(
            project_id,
            issue_id,
            attempt_id,
            CodingAttemptStatus::Aborted,
        )?;
        self.release_issue_shared_worktree_lock_if_holder(
            project_id,
            issue_id,
            &updated.work_item_id,
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
            project_id,
            issue_id,
            attempt_id,
            &updated.work_item_id,
        )
        .await?;
        self.release_issue_shared_worktree_lock_if_holder(
            project_id,
            issue_id,
            &updated.work_item_id,
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
        self.ensure_issue_shared_worktree_clean(
            project_id,
            issue_id,
            attempt_id,
            &current.work_item_id,
        )
        .await?;
        let updated = self.store.update_attempt_status(
            project_id,
            issue_id,
            attempt_id,
            CodingAttemptStatus::Aborted,
        )?;
        self.release_issue_shared_worktree_lock_if_holder(
            project_id,
            issue_id,
            &updated.work_item_id,
        )?;
        Ok(())
    }

    pub(crate) async fn generate_and_save_work_item_handoff_if_missing(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<(), CodingWorkspaceEngineError> {
        if attempt.scope == CodingAttemptScope::WorkItemGroup {
            let Some(active) = self.store.get_active_coding_unit(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
            )?
            else {
                if self.store.get_visible_work_item_handoff(attempt)?.is_some() {
                    return Ok(());
                }
                return Err(CodingWorkspaceEngineError::WorkItemHandoffMissing(
                    attempt.id.clone(),
                ));
            };
            let handoff_ref = format!("units/{}/work-item-handoff.json", active.id);
            if self
                .store
                .get_coding_unit_handoff(
                    &attempt.project_id,
                    &attempt.issue_id,
                    &attempt.id,
                    &active.id,
                )?
                .is_some()
            {
                if active.handoff_ref.as_deref() != Some(handoff_ref.as_str()) {
                    self.store.update_coding_unit_handoff_ref(
                        &attempt.project_id,
                        &attempt.issue_id,
                        &attempt.id,
                        &active.id,
                        Some(handoff_ref),
                    )?;
                }
                return Ok(());
            }

            let handoff = if let Some(provider) = self.provider.as_ref() {
                self.generate_work_item_handoff_from_provider(provider, attempt)
                    .await?
            } else {
                self.generate_placeholder_work_item_handoff(attempt).await?
            };
            self.store.save_coding_unit_handoff(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                &active.id,
                &handoff,
            )?;
            self.store.update_coding_unit_handoff_ref(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                &active.id,
                Some(handoff_ref.clone()),
            )?;

            let lifecycle = LifecycleStore::new(self.store.paths());
            let current_work_item_id = self.current_work_item_id_for_handoff(attempt);
            if lifecycle
                .list_work_items(&attempt.project_id, &attempt.issue_id)?
                .iter()
                .any(|item| item.id == current_work_item_id)
            {
                lifecycle.update_work_item_handoff_summary(
                    &attempt.project_id,
                    &attempt.issue_id,
                    current_work_item_id,
                    Some(handoff_ref),
                    handoff.commit_sha.clone(),
                )?;
            }

            return Ok(());
        }

        if self
            .store
            .get_work_item_handoff(&attempt.project_id, &attempt.issue_id, &attempt.id)?
            .is_some()
        {
            return Ok(());
        }

        let handoff = if let Some(provider) = self.provider.as_ref() {
            self.generate_work_item_handoff_from_provider(provider, attempt)
                .await?
        } else {
            self.generate_placeholder_work_item_handoff(attempt).await?
        };

        self.store.save_work_item_handoff(&handoff)?;

        let lifecycle = LifecycleStore::new(self.store.paths());
        let current_work_item_id = self.current_work_item_id_for_handoff(attempt);
        if lifecycle
            .list_work_items(&attempt.project_id, &attempt.issue_id)?
            .iter()
            .any(|item| item.id == current_work_item_id)
        {
            lifecycle.update_work_item_handoff_summary(
                &attempt.project_id,
                &attempt.issue_id,
                current_work_item_id,
                Some(format!(
                    "projects/{}/issues/{}/coding-attempts/{}/work-item-handoff.json",
                    attempt.project_id, attempt.issue_id, attempt.id
                )),
                attempt.head_commit.clone(),
            )?;
        }

        Ok(())
    }

    pub(crate) async fn generate_placeholder_work_item_handoff(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<WorkItemHandoff, CodingWorkspaceEngineError> {
        let testing_reports =
            self.store
                .list_testing_reports(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
        let tests_run: Vec<String> = testing_reports
            .iter()
            .flat_map(|report| report.commands.iter().map(|cmd| cmd.command.join(" ")))
            .collect();
        let test_result_summary = testing_reports
            .last()
            .map(|report| format!("{:?}", report.overall_status))
            .unwrap_or_else(|| "no testing report".to_string());

        let review_requests =
            self.store
                .list_review_requests(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
        let review_summary = review_requests
            .last()
            .map(|r| format!("{:?}", r.push_status));

        Ok(WorkItemHandoff {
            id: format!(
                "work_item_handoff_{}_{}_{}",
                attempt.project_id, attempt.issue_id, attempt.id
            ),
            project_id: attempt.project_id.clone(),
            issue_id: attempt.issue_id.clone(),
            work_item_id: self.current_work_item_id_for_handoff(attempt).to_string(),
            attempt_id: attempt.id.clone(),
            provider_run_ref: None,
            summary: "Handoff generated from attempt artifacts".to_string(),
            files_changed: Vec::new(),
            commit_sha: attempt.head_commit.clone(),
            diff_summary: String::new(),
            tests_run,
            test_result_summary,
            review_summary,
            api_or_contract_changes: Vec::new(),
            open_risks: Vec::new(),
            next_work_item_notes: Vec::new(),
            created_at: Utc::now().to_rfc3339(),
        })
    }

    pub(crate) async fn generate_work_item_handoff_from_provider(
        &self,
        provider: &Arc<dyn ProviderAdapter + Send + Sync>,
        attempt: &CodingExecutionAttempt,
    ) -> Result<WorkItemHandoff, CodingWorkspaceEngineError> {
        let worktree_path = self.attempt_worktree_path(attempt).await?;
        let provider_type = provider_type_for_name(&attempt.provider_config_snapshot.author);
        let output_schema = serde_json::json!({
            "type": "object",
            "properties": {
                "summary": {"type": "string"},
                "files_changed": {"type": "array", "items": {"type": "string"}},
                "diff_summary": {"type": "string"},
                "tests_run": {"type": "array", "items": {"type": "string"}},
                "test_result_summary": {"type": "string"},
                "api_or_contract_changes": {"type": "array", "items": {"type": "string"}},
                "next_work_item_notes": {"type": "array", "items": {"type": "string"}}
            },
            "required": ["summary"]
        });
        let input = AdapterInput {
            provider_type,
            role: AdapterRole::Handoff,
            prompt: "Generate a concise handoff summary for the completed work item.".to_string(),
            worktree_path: Some(worktree_path.to_string_lossy().to_string()),
            context_files: Vec::new(),
            output_schema: output_schema.to_string(),
            timeout: DEFAULT_PROVIDER_TIMEOUT_SECS,
            max_retries: 0,
        };

        let output = tokio::task::spawn_blocking({
            let provider = Arc::clone(provider);
            move || provider.run(&input)
        })
        .await
        .map_err(|error| CodingWorkspaceEngineError::ProviderStream(error.to_string()))??;

        let structured = output.structured_output.unwrap_or_default();
        Ok(WorkItemHandoff {
            id: format!(
                "work_item_handoff_{}_{}_{}",
                attempt.project_id, attempt.issue_id, attempt.id
            ),
            project_id: attempt.project_id.clone(),
            issue_id: attempt.issue_id.clone(),
            work_item_id: self.current_work_item_id_for_handoff(attempt).to_string(),
            attempt_id: attempt.id.clone(),
            provider_run_ref: None,
            summary: structured
                .get("summary")
                .and_then(|v| v.as_str())
                .unwrap_or("Completed work item")
                .to_string(),
            files_changed: structured
                .get("files_changed")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default(),
            commit_sha: attempt.head_commit.clone(),
            diff_summary: structured
                .get("diff_summary")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            tests_run: structured
                .get("tests_run")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default(),
            test_result_summary: structured
                .get("test_result_summary")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            review_summary: None,
            api_or_contract_changes: structured
                .get("api_or_contract_changes")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default(),
            open_risks: Vec::new(),
            next_work_item_notes: structured
                .get("next_work_item_notes")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default(),
            created_at: Utc::now().to_rfc3339(),
        })
    }

    pub(crate) async fn complete_attempt_after_final_rework(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
        self.generate_and_save_work_item_handoff_if_missing(attempt)
            .await?;
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
            &attempt.work_item_id,
        )?;
        let node = self.create_completed_final_confirm_timeline_node(&completed)?;
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::CodingTimelineNodeCreated { node })
            .await;
        Ok(completed)
    }

    pub async fn complete_group_unit_after_code_review(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
        self.generate_and_save_work_item_handoff_if_missing(attempt)
            .await?;
        self.complete_current_group_unit(attempt, Some("当前 Work Item 已完成".to_string()))
            .await
    }

    fn mark_completed_group_work_items_if_present(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<(), CodingWorkspaceEngineError> {
        let lifecycle = LifecycleStore::new(self.store.paths());
        let existing_work_item_ids = lifecycle
            .list_work_items(&attempt.project_id, &attempt.issue_id)?
            .into_iter()
            .map(|work_item| work_item.id)
            .collect::<std::collections::HashSet<_>>();
        let completed_units =
            self.store
                .list_coding_units(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
        for unit in completed_units.into_iter().filter(|unit| {
            unit.status == crate::product::coding_models::CodingExecutionUnitStatus::Completed
        }) {
            if existing_work_item_ids.contains(&unit.work_item_id) {
                lifecycle.update_work_item_execution_status(
                    &attempt.project_id,
                    &attempt.issue_id,
                    &unit.work_item_id,
                    WorkItemStatus::Completed,
                )?;
                self.mark_issue_shared_worktree_completed_if_present(
                    &attempt.project_id,
                    &attempt.issue_id,
                    &unit.work_item_id,
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
