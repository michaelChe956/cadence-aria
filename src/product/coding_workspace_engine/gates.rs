use super::*;

impl CodingWorkspaceEngine {
    pub(crate) async fn fail_provider_stream<T>(
        &self,
        attempt: &CodingExecutionAttempt,
        node_id: &str,
        message: String,
    ) -> Result<T, CodingWorkspaceEngineError> {
        self.store.update_attempt_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingAttemptStatus::Failed,
        )?;
        self.complete_timeline_node(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            node_id,
            CodingTimelineNodeStatus::Failed,
            Some(message.clone()),
        )
        .await?;
        self.handle_attempt_failed(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .await?;
        Err(CodingWorkspaceEngineError::ProviderStream(message))
    }

    pub(crate) async fn fail_provider_stream_ended<T>(
        &self,
        attempt: &CodingExecutionAttempt,
        node_id: &str,
    ) -> Result<T, CodingWorkspaceEngineError> {
        self.fail_provider_stream(
            attempt,
            node_id,
            "provider stream ended before completion".to_string(),
        )
        .await
    }

    pub(crate) async fn emit_permission_request(
        &self,
        node_id: &str,
        provider: &ProviderName,
        request: PermissionRequestData,
    ) {
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::CodingExecutionEvent {
                event: ws_event_from_permission_request(node_id, provider, &request),
            })
            .await;
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::CodingPermissionRequest {
                id: request.id,
                tool_name: request.tool_name,
                description: request.description,
                risk_level: ws_permission_risk_level(request.risk_level),
            })
            .await;
    }

    pub(crate) async fn emit_choice_request(
        &self,
        attempt: &CodingExecutionAttempt,
        node_id: &str,
        stage: CodingExecutionStage,
        role: CodingProviderRole,
        provider: &ProviderName,
        request: ChoiceRequestData,
    ) -> Result<(), CodingWorkspaceEngineError> {
        let source = request.source.as_str().to_string();
        self.store.create_choice_gate(CreateChoiceGateInput {
            attempt_id: attempt.id.clone(),
            choice_id: request.id.clone(),
            stage,
            node_id: Some(node_id.to_string()),
            role,
            provider: provider.clone(),
            source: source.clone(),
            prompt: request.prompt.clone(),
            options: request
                .options
                .iter()
                .map(|option| CodingChoiceOption {
                    id: option.id.clone(),
                    label: option.label.clone(),
                    description: option.description.clone(),
                })
                .collect(),
            allow_multiple: request.allow_multiple,
            allow_free_text: request.allow_free_text,
        })?;
        let current =
            self.store
                .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
        if current.status == CodingAttemptStatus::Running {
            self.store.update_attempt_status(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                CodingAttemptStatus::WaitingForHuman,
            )?;
        }
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::CodingExecutionEvent {
                event: ws_event_from_choice_request(node_id, provider, &request),
            })
            .await;
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::CodingChoiceRequest {
                id: request.id,
                prompt: request.prompt,
                source,
                options: request
                    .options
                    .into_iter()
                    .map(|option| ChoiceOption {
                        id: option.id,
                        label: option.label,
                        description: option.description,
                    })
                    .collect(),
                allow_multiple: request.allow_multiple,
                allow_free_text: request.allow_free_text,
            })
            .await;
        Ok(())
    }

    pub(crate) async fn ensure_issue_shared_worktree_clean(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        work_item_id: &str,
    ) -> Result<(), CodingWorkspaceEngineError> {
        let lifecycle = LifecycleStore::new(self.store.paths());
        let shared = match lifecycle.get_issue_shared_worktree(project_id, issue_id)? {
            Some(shared) => shared,
            None => return Ok(()),
        };
        if shared.current_active_work_item_id.as_deref() != Some(work_item_id) {
            return Ok(());
        }
        let worktree_path = shared.worktree_path;
        if !worktree_path.exists() {
            return Ok(());
        }
        let status = self._git_service.git_status(&worktree_path).await?;
        if !status.is_empty() {
            self.store.create_blocked_gate(CreateBlockedGateInput {
                attempt_id: attempt_id.to_string(),
                stage: CodingExecutionStage::FinalConfirm,
                node_id: None,
                role: None,
                title: "Shared worktree has uncommitted changes".to_string(),
                description: "Issue shared worktree has uncommitted changes and must be cleaned up manually before the active lock can be released".to_string(),
                reason_code: Some("shared_worktree_dirty_manual_gate".to_string()),
                evidence_refs: Vec::new(),
                raw_provider_output_ref: None,
                available_actions: vec![
                    coding_gate_action_for_id("manual_continue").expect("manual continue action"),
                    coding_gate_action_for_id("abort").expect("abort action"),
                ],
            })?;
            return Err(CodingWorkspaceEngineError::SharedWorktreeDirtyManualGate(
                worktree_path.to_string_lossy().to_string(),
            ));
        }
        Ok(())
    }

    pub(crate) async fn run_completion_gates(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<CompletionGateReport, CodingWorkspaceEngineError> {
        if attempt.head_commit.is_none() {
            return Err(CodingWorkspaceEngineError::CompletionCommitMissing(
                attempt.id.clone(),
            ));
        }

        let lifecycle = LifecycleStore::new(self.store.paths());
        let work_item = lifecycle
            .list_work_items(&attempt.project_id, &attempt.issue_id)?
            .into_iter()
            .find(|item| item.id == attempt.work_item_id)
            .ok_or_else(|| CodingWorkspaceEngineError::FinalConfirmNotReady(attempt.id.clone()))?;

        let changed_files = self.changed_files_for_attempt(attempt, &work_item).await?;
        let worktree_path = self.attempt_worktree_path(attempt).await.ok();
        for relative_path in &changed_files {
            let candidate = std::path::Path::new(relative_path);
            if work_item
                .forbidden_write_scopes
                .iter()
                .any(|scope| scope_allows_path(scope, relative_path, true))
            {
                return Err(CodingWorkspaceEngineError::WorkItemDiffScopeViolation(
                    relative_path.clone(),
                ));
            }
            if !work_item.exclusive_write_scopes.is_empty()
                && let Some(ref base) = worktree_path
            {
                let _ =
                    validate_write_path(base, &work_item.exclusive_write_scopes, candidate, true)
                        .map_err(|_| {
                        CodingWorkspaceEngineError::WorkItemDiffScopeViolation(
                            relative_path.clone(),
                        )
                    })?;
            }
        }

        if let Some(plan_ref) = &work_item.verification_plan_ref {
            let verification_plan = lifecycle.get_verification_plan(
                &attempt.project_id,
                &attempt.issue_id,
                plan_ref,
            )?;
            if !verification_plan.required_gates.is_empty() {
                let reports = self.store.list_testing_reports(
                    &attempt.project_id,
                    &attempt.issue_id,
                    &attempt.id,
                )?;
                let passed = reports.iter().any(|report| {
                    report.overall_status == TestingOverallStatus::Passed
                        || report.overall_status == TestingOverallStatus::PassedWithWarnings
                });
                if !passed {
                    return Err(CodingWorkspaceEngineError::VerificationGateResultMissing(
                        attempt.id.clone(),
                    ));
                }
            }
        }

        if self.store.get_visible_work_item_handoff(attempt)?.is_none() {
            return Err(CodingWorkspaceEngineError::WorkItemHandoffMissing(
                attempt.id.clone(),
            ));
        }

        self.ensure_issue_shared_worktree_clean(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &attempt.work_item_id,
        )
        .await?;

        Ok(CompletionGateReport)
    }

    pub(crate) async fn changed_files_for_attempt(
        &self,
        attempt: &CodingExecutionAttempt,
        _work_item: &LifecycleWorkItemRecord,
    ) -> Result<Vec<String>, CodingWorkspaceEngineError> {
        let worktree_path = match self.attempt_worktree_path(attempt).await {
            Ok(path) => path,
            Err(CodingWorkspaceEngineError::MissingWorktree(_)) => return Ok(Vec::new()),
            Err(error) => return Err(error),
        };
        if !worktree_path.exists() {
            return Ok(Vec::new());
        }
        match self._git_service.git_status(&worktree_path).await {
            Ok(status) => Ok(status.into_iter().map(|file| file.path).collect()),
            Err(_) => Ok(Vec::new()),
        }
    }

    pub(crate) async fn attempt_worktree_path(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<PathBuf, CodingWorkspaceEngineError> {
        if let Some(path) = attempt.worktree_path.as_ref() {
            return Ok(path.clone());
        }
        let lifecycle = LifecycleStore::new(self.store.paths());
        let shared = lifecycle.get_issue_shared_worktree(&attempt.project_id, &attempt.issue_id)?;
        match shared {
            Some(shared) if shared.worktree_path.exists() => Ok(shared.worktree_path),
            _ => Err(CodingWorkspaceEngineError::MissingWorktree(
                attempt.id.clone(),
            )),
        }
    }

    pub(crate) fn release_issue_shared_worktree_lock_if_holder(
        &self,
        project_id: &str,
        issue_id: &str,
        work_item_id: &str,
    ) -> Result<(), CodingWorkspaceEngineError> {
        let lifecycle = LifecycleStore::new(self.store.paths());
        let shared = match lifecycle.get_issue_shared_worktree(project_id, issue_id)? {
            Some(shared) => shared,
            None => return Ok(()),
        };
        if shared.current_active_work_item_id.as_deref() == Some(work_item_id) {
            lifecycle.release_issue_worktree_lock(project_id, issue_id, work_item_id)?;
        }
        Ok(())
    }

    pub(crate) fn mark_issue_shared_worktree_completed_if_present(
        &self,
        project_id: &str,
        issue_id: &str,
        work_item_id: &str,
    ) -> Result<(), CodingWorkspaceEngineError> {
        let lifecycle = LifecycleStore::new(self.store.paths());
        if lifecycle
            .get_issue_shared_worktree(project_id, issue_id)?
            .is_some()
        {
            lifecycle.mark_issue_worktree_completed_item(project_id, issue_id, work_item_id)?;
        }
        Ok(())
    }

    pub(crate) async fn release_active_lock_if_shared_worktree_clean(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        work_item_id: &str,
    ) -> Result<(), CodingWorkspaceEngineError> {
        match self
            .ensure_issue_shared_worktree_clean(project_id, issue_id, attempt_id, work_item_id)
            .await
        {
            Ok(()) => self.release_issue_shared_worktree_lock_if_holder(
                project_id,
                issue_id,
                work_item_id,
            ),
            Err(CodingWorkspaceEngineError::SharedWorktreeDirtyManualGate(_)) => Ok(()),
            Err(error) => Err(error),
        }
    }

    pub async fn handle_blocked_gate_response(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        gate_id: &str,
        action_id: &str,
        extra_context: Option<String>,
    ) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
        let Some(gate) = self
            .store
            .list_open_blocked_gates(project_id, issue_id, attempt_id)?
            .into_iter()
            .find(|gate| gate.gate_id == gate_id)
        else {
            return Ok(self.store.get_attempt(project_id, issue_id, attempt_id)?);
        };
        let action = gate
            .available_actions
            .iter()
            .find(|action| action.action_id == action_id)
            .ok_or_else(|| {
                CodingWorkspaceEngineError::ProviderStream(
                    "coding_gate_action_not_allowed".to_string(),
                )
            })?;
        let should_resolve_gate =
            !matches!(action.action_type, CodingGateActionType::ProvideContext);

        let current = self.store.get_attempt(project_id, issue_id, attempt_id)?;
        let updated = match action.action_type {
            CodingGateActionType::Abort => {
                self.handle_abort(project_id, issue_id, attempt_id).await?
            }
            CodingGateActionType::RetryTestPlan
            | CodingGateActionType::RerunMissingSteps
            | CodingGateActionType::RerunTesting => {
                let trigger = match action.action_type {
                    CodingGateActionType::RetryTestPlan => CodingRoleRunTrigger::RetryTestPlan,
                    CodingGateActionType::RerunMissingSteps => {
                        CodingRoleRunTrigger::RerunMissingSteps
                    }
                    CodingGateActionType::RerunTesting => CodingRoleRunTrigger::ManualRerun,
                    _ => CodingRoleRunTrigger::ManualRerun,
                };
                let resumed =
                    self.resume_blocked_attempt_at_stage(&current, CodingExecutionStage::Testing)?;
                self.store.supersede_latest_role_run_and_create(
                    &resumed,
                    CodingExecutionStage::Testing,
                    CodingProviderRole::Tester,
                    trigger,
                    None,
                    gate.reason_code.clone(),
                )?;
                resumed
            }
            CodingGateActionType::RetryReview => {
                if gate.stage == Some(CodingExecutionStage::InternalPrReview)
                    || gate.role == Some(CodingProviderRole::InternalReviewer)
                {
                    let resumed = self.resume_blocked_attempt_at_stage(
                        &current,
                        CodingExecutionStage::InternalPrReview,
                    )?;
                    self.store.supersede_latest_role_run_and_create(
                        &resumed,
                        CodingExecutionStage::InternalPrReview,
                        CodingProviderRole::InternalReviewer,
                        CodingRoleRunTrigger::RetryInternalReview,
                        None,
                        gate.reason_code.clone(),
                    )?;
                    resumed
                } else {
                    let resumed = self.resume_blocked_attempt_at_stage(
                        &current,
                        CodingExecutionStage::CodeReview,
                    )?;
                    self.store.supersede_latest_role_run_and_create(
                        &resumed,
                        CodingExecutionStage::CodeReview,
                        CodingProviderRole::CodeReviewer,
                        CodingRoleRunTrigger::RetryReview,
                        None,
                        gate.reason_code.clone(),
                    )?;
                    resumed
                }
            }
            CodingGateActionType::RetryInternalReview => {
                let resumed = self.resume_blocked_attempt_at_stage(
                    &current,
                    CodingExecutionStage::InternalPrReview,
                )?;
                self.store.supersede_latest_role_run_and_create(
                    &resumed,
                    CodingExecutionStage::InternalPrReview,
                    CodingProviderRole::InternalReviewer,
                    CodingRoleRunTrigger::RetryInternalReview,
                    None,
                    gate.reason_code.clone(),
                )?;
                resumed
            }
            CodingGateActionType::RetryAnalyst => {
                let previous = self.store.latest_role_run(
                    &current.project_id,
                    &current.issue_id,
                    &current.id,
                    CodingExecutionStage::Rework,
                    CodingProviderRole::Analyst,
                )?;
                let resumed =
                    self.resume_blocked_attempt_at_stage(&current, CodingExecutionStage::Rework)?;
                let new_run = self.store.supersede_latest_role_run_and_create(
                    &resumed,
                    CodingExecutionStage::Rework,
                    CodingProviderRole::Analyst,
                    CodingRoleRunTrigger::RetryAnalyst,
                    None,
                    gate.reason_code.clone(),
                )?;
                if let Some(previous) = previous {
                    self.store.update_role_run_refs(
                        &resumed.project_id,
                        &resumed.issue_id,
                        &resumed.id,
                        &new_run.id,
                        Vec::new(),
                        previous.artifact_refs,
                    )?;
                }
                resumed
            }
            CodingGateActionType::AcceptTestingResult => {
                self.accept_testing_result_for_analyst(&current, &gate)?
            }
            CodingGateActionType::ContinueRework => {
                self.continue_rework_after_limit_for_attempt(&current, extra_context)?
            }
            CodingGateActionType::SendRawOutputToAnalyst => {
                self.resume_blocked_attempt_at_stage(&current, CodingExecutionStage::Rework)?
            }
            CodingGateActionType::ProvideContext => {
                if let Some(content) = extra_context
                    && !content.trim().is_empty()
                {
                    self.store.create_context_note(&current.id, content)?;
                }
                let running = if current.status == CodingAttemptStatus::Blocked {
                    self.store.update_attempt_status(
                        project_id,
                        issue_id,
                        attempt_id,
                        CodingAttemptStatus::Running,
                    )?
                } else {
                    current
                };
                self.store.update_attempt_status(
                    &running.project_id,
                    &running.issue_id,
                    &running.id,
                    CodingAttemptStatus::WaitingForHuman,
                )?
            }
            CodingGateActionType::ManualContinue | CodingGateActionType::AcceptRisk => {
                let operator_context = extra_context
                    .map(|content| content.trim().to_string())
                    .filter(|content| !content.is_empty())
                    .ok_or_else(|| {
                        CodingWorkspaceEngineError::ProviderStream(
                            "coding_gate_extra_context_required".to_string(),
                        )
                    })?;
                self.store
                    .create_context_note(&current.id, operator_context.clone())?;
                self.store
                    .create_quality_bypass_audit(CreateQualityBypassAuditInput {
                        attempt_id: current.id.clone(),
                        gate_id: gate.gate_id.clone(),
                        stage: gate.stage.clone().unwrap_or_else(|| current.stage.clone()),
                        reason_code: gate.reason_code.clone(),
                        skipped_required_steps: self.latest_missing_required_steps(&current)?,
                        operator_context,
                    })?;
                if current.status == CodingAttemptStatus::Blocked {
                    self.store.update_attempt_status(
                        project_id,
                        issue_id,
                        attempt_id,
                        CodingAttemptStatus::Running,
                    )?
                } else {
                    current
                }
            }
            _ => {
                return Err(CodingWorkspaceEngineError::ProviderStream(
                    "coding_gate_action_not_allowed".to_string(),
                ));
            }
        };
        if should_resolve_gate {
            self.store
                .resolve_blocked_gate(project_id, issue_id, attempt_id, gate_id)?;
        }
        Ok(updated)
    }

    pub(crate) fn latest_missing_required_steps(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<Vec<String>, ProductStoreError> {
        let Some(report) = self
            .store
            .list_testing_reports(&attempt.project_id, &attempt.issue_id, &attempt.id)?
            .into_iter()
            .last()
        else {
            return Ok(Vec::new());
        };
        let mut steps = Vec::new();
        for step in report
            .missing_required_steps
            .into_iter()
            .chain(report.skipped_required_steps)
        {
            if !steps.contains(&step) {
                steps.push(step);
            }
        }
        Ok(steps)
    }

    pub(crate) fn accept_testing_result_for_analyst(
        &self,
        current: &CodingExecutionAttempt,
        gate: &CodingGateRequired,
    ) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
        let report = self.testing_report_for_gate(current, gate)?;
        let running = if current.status == CodingAttemptStatus::Blocked {
            self.store.update_attempt_status(
                &current.project_id,
                &current.issue_id,
                &current.id,
                CodingAttemptStatus::Running,
            )?
        } else {
            current.clone()
        };
        let role_run = self.store.create_role_run(
            &running,
            CodingExecutionStage::Rework,
            CodingProviderRole::Analyst,
            CodingRoleRunTrigger::Initial,
            None,
        )?;
        let evidence = testing_report_to_analyst_evidence(&report);
        let evidence_ref = self.store.save_analyst_evidence(&running.id, &evidence)?;
        self.store.update_role_run_refs(
            &running.project_id,
            &running.issue_id,
            &running.id,
            &role_run.id,
            Vec::new(),
            vec![evidence_ref],
        )?;
        Ok(running)
    }

    pub(crate) fn testing_report_for_gate(
        &self,
        attempt: &CodingExecutionAttempt,
        gate: &CodingGateRequired,
    ) -> Result<TestingReport, CodingWorkspaceEngineError> {
        if let Some(report_id) = gate
            .evidence_refs
            .iter()
            .rev()
            .find_map(|reference| reference.strip_suffix(".json"))
        {
            return Ok(self.store.get_testing_report(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                report_id,
            )?);
        }
        self.store
            .list_testing_reports(&attempt.project_id, &attempt.issue_id, &attempt.id)?
            .into_iter()
            .last()
            .ok_or_else(|| {
                CodingWorkspaceEngineError::ProviderStream(
                    "testing_result_review_missing_report".to_string(),
                )
            })
    }

    pub(crate) fn resume_blocked_attempt_at_stage(
        &self,
        current: &CodingExecutionAttempt,
        stage: CodingExecutionStage,
    ) -> Result<CodingExecutionAttempt, ProductStoreError> {
        let mut updated = if matches!(
            current.status,
            CodingAttemptStatus::Blocked | CodingAttemptStatus::WaitingForHuman
        ) {
            self.store.update_attempt_status(
                &current.project_id,
                &current.issue_id,
                &current.id,
                CodingAttemptStatus::Running,
            )?
        } else {
            current.clone()
        };
        if updated.stage != stage {
            updated = self.store.update_attempt_stage(
                &updated.project_id,
                &updated.issue_id,
                &updated.id,
                stage,
            )?;
        }
        Ok(updated)
    }
}
