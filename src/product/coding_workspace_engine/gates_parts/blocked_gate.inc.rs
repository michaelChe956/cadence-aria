impl CodingWorkspaceEngine {
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
        let code_review_provider_interrupted =
            super::failed_review_recovery::is_code_review_provider_interrupted_gate(&gate);
        if code_review_provider_interrupted && action_id != "retry_review" {
            return Err(CodingWorkspaceEngineError::ProviderStream(
                "coding_failed_review_recovery_action_not_allowed".to_string(),
            ));
        }
        let action = gate
            .available_actions
            .iter()
            .find(|action| action.action_id == action_id)
            .ok_or_else(|| {
                CodingWorkspaceEngineError::ProviderStream(
                    "coding_gate_action_not_allowed".to_string(),
                )
            })?;
        if action.action_type == CodingGateActionType::RetryReview
            && code_review_provider_interrupted
        {
            return Err(CodingWorkspaceEngineError::ProviderStream(
                "coding_failed_review_recovery_requires_reservation".to_string(),
            ));
        }
        let should_resolve_gate =
            !matches!(action.action_type, CodingGateActionType::ProvideContext);

        let current = self.store.get_attempt(project_id, issue_id, attempt_id)?;
        let updated = match action.action_type {
            CodingGateActionType::Abort => {
                self.handle_abort(project_id, issue_id, attempt_id).await?
            }
            CodingGateActionType::RetryCoding => {
                let coder_provider = self
                    .store
                    .get_role_provider_config_snapshot(
                        &current.project_id,
                        &current.issue_id,
                        &current.id,
                    )?
                    .coder;
                let cleared = self.clear_attempt_provider_conversation(
                    &current,
                    &CodingProviderRole::Coder,
                    &coder_provider,
                )?;
                let resumed =
                    self.resume_blocked_attempt_at_stage(&cleared, CodingExecutionStage::Coding)?;
                if let Some(failed) = self.store.latest_role_run(
                    &resumed.project_id,
                    &resumed.issue_id,
                    &resumed.id,
                    CodingExecutionStage::Coding,
                    CodingProviderRole::Coder,
                )? && failed.status == CodingRoleRunStatus::Failed
                {
                    self.store.create_manual_retry_role_run(
                        &resumed,
                        CodingExecutionStage::Coding,
                        CodingProviderRole::Coder,
                        &failed,
                        gate.reason_code.clone(),
                    )?;
                }
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
            CodingGateActionType::RetryGroupReviewShard
            | CodingGateActionType::RetryGroupReduction => self.resume_blocked_attempt_at_stage(
                &current,
                CodingExecutionStage::InternalPrReview,
            )?,
            CodingGateActionType::SendToCoder => {
                if is_code_review_feedback_gate(&gate) {
                    self.send_code_review_feedback_to_coder(&current, extra_context)?
                } else {
                    self.send_review_limit_feedback_to_coder(&current, extra_context)?
                }
            }
            CodingGateActionType::ProvideContext => {
                if let Some(content) = extra_context
                    && !content.trim().is_empty()
                {
                    self.store.create_context_note(&current, content)?;
                }
                let running = if current.status == CodingAttemptStatus::Blocked {
                    self.store.admit_and_transition_attempt_to_executable(
                        project_id, issue_id, attempt_id,
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
                self.continue_attempt_with_manual_context(&current, &gate, operator_context)?
            }
            _ => {
                return Err(CodingWorkspaceEngineError::ProviderStream(
                    "coding_gate_action_not_allowed".to_string(),
                ));
            }
        };
        if should_resolve_gate {
            // C2 Task 4：记录解决动作——manual_continue／accept_risk 是续跑
            // runner 跳过已完成 reviewer 的持久化证据。
            self.store.resolve_blocked_gate_with_action(
                project_id,
                issue_id,
                attempt_id,
                gate_id,
                Some(action_id),
            )?;
        }
        Ok(updated)
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
            self.store.admit_and_transition_attempt_to_executable(
                &current.project_id,
                &current.issue_id,
                &current.id,
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

    // ─── C2 Task 8（REQ-CVT-03/04，#19／BYPASS-19）：验证处理旁路入口 ───

    /// manual_continue 语义共享续跑效果：context note＋质量豁免审计＋
    /// admission CAS 回 Running（门解决由调用侧统一执行）。Task 4 门动作
    /// 与 Task 8 验证处理结论批准复用同一应用效果。
    pub(crate) fn continue_attempt_with_manual_context(
        &self,
        current: &CodingExecutionAttempt,
        gate: &CodingGateRequired,
        operator_context: String,
    ) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
        self.store
            .create_context_note(current, operator_context.clone())?;
        self.store.create_quality_bypass_audit(
            current,
            CreateQualityBypassAuditInput {
                attempt_id: current.id.clone(),
                gate_id: gate.gate_id.clone(),
                stage: gate.stage.clone().unwrap_or_else(|| current.stage.clone()),
                reason_code: gate.reason_code.clone(),
                operator_context,
            },
        )?;
        if current.status == CodingAttemptStatus::Blocked {
            Ok(self.store.admit_and_transition_attempt_to_executable(
                &current.project_id,
                &current.issue_id,
                &current.id,
            )?)
        } else {
            Ok(current.clone())
        }
    }
}
