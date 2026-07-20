use super::*;

pub(crate) struct ReviewBlockedGateInput<'a> {
    pub(crate) attempt: &'a CodingExecutionAttempt,
    pub(crate) node_id: &'a str,
    pub(crate) stage: CodingExecutionStage,
    pub(crate) role: CodingProviderRole,
    pub(crate) title: String,
    pub(crate) description: String,
    pub(crate) reason_code: &'static str,
    pub(crate) evidence_refs: Vec<String>,
    pub(crate) raw_provider_output_ref: Option<String>,
}

impl CodingWorkspaceEngine {
    pub(crate) async fn create_review_blocked_gate(
        &self,
        input: ReviewBlockedGateInput<'_>,
    ) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
        let ReviewBlockedGateInput {
            attempt,
            node_id,
            stage,
            role,
            title,
            description,
            reason_code,
            evidence_refs,
            raw_provider_output_ref,
        } = input;
        let retry_action = match stage {
            CodingExecutionStage::CodeReview => CodingGateAction {
                action_id: "retry_review".to_string(),
                label: "重试代码审查".to_string(),
                action_type: CodingGateActionType::RetryReview,
            },
            CodingExecutionStage::InternalPrReview => {
                coding_gate_action_for_id("retry_internal_review")
                    .expect("retry internal review action")
            }
            _ => coding_gate_action_for_id("retry_review").expect("retry review action"),
        };
        let available_actions = if reason_code == "code_review_provider_interrupted" {
            vec![retry_action]
        } else if matches!(
            reason_code,
            "internal_review_operational_blocker" | "internal_review_human_triage"
        ) {
            vec![
                retry_action,
                coding_gate_action_for_id("abort").expect("abort action"),
            ]
        } else {
            vec![
                retry_action,
                coding_gate_action_for_id("send_to_coder").expect("send to coder action"),
                coding_gate_action_for_id("abort").expect("abort action"),
            ]
        };
        let updated = self.store.update_attempt_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingAttemptStatus::Blocked,
        )?;
        let gate = self.store.create_blocked_gate(
            &updated,
            CreateBlockedGateInput {
                attempt_id: attempt.id.clone(),
                stage,
                node_id: Some(node_id.to_string()),
                role: Some(role),
                title,
                description,
                reason_code: Some(reason_code.to_string()),
                evidence_refs,
                raw_provider_output_ref,
                available_actions,
            },
        )?;
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::CodingGateRequired { gate })
            .await;
        Ok(updated)
    }

    pub(crate) async fn fail_provider_stream<T>(
        &self,
        attempt: &CodingExecutionAttempt,
        node_id: &str,
        message: String,
    ) -> Result<T, CodingWorkspaceEngineError> {
        self.validate_attempt_issue_shared_worktree_lock_if_present(attempt)?;
        #[cfg(test)]
        crate::product::coding_workspace_engine::mutation_test_pause::pause_coding_mutation_for_test(
            self.store.paths().root(),
            crate::product::coding_workspace_engine::mutation_test_pause::CodingMutationTestPoint::ProviderFailure,
        )
        .await;
        let current =
            self.store
                .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
        if !current.status.is_active() || current.stage != attempt.stage {
            return Err(CodingWorkspaceEngineError::ProviderStream(format!(
                "provider_failure_attempt_state_changed: {}",
                attempt.id
            )));
        }
        self.validate_attempt_issue_shared_worktree_lock_if_present(&current)?;
        let attempt = &current;
        if attempt.stage == CodingExecutionStage::CodeReview {
            self.complete_timeline_node(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                node_id,
                CodingTimelineNodeStatus::Failed,
                Some(message.clone()),
            )
            .await?;
            if let Some(role_run) = self.store.latest_role_run(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                CodingExecutionStage::CodeReview,
                CodingProviderRole::CodeReviewer,
            )? && role_run.status == CodingRoleRunStatus::Running
            {
                self.store.update_role_run_status(
                    &attempt.project_id,
                    &attempt.issue_id,
                    &attempt.id,
                    &role_run.id,
                    CodingRoleRunStatus::Failed,
                    Some("code_review_provider_interrupted".to_string()),
                )?;
            }
            self.create_review_blocked_gate(ReviewBlockedGateInput {
                attempt,
                node_id,
                stage: CodingExecutionStage::CodeReview,
                role: CodingProviderRole::CodeReviewer,
                title: "代码审查中断".to_string(),
                description: message.clone(),
                reason_code: "code_review_provider_interrupted",
                evidence_refs: Vec::new(),
                raw_provider_output_ref: None,
            })
            .await?;
            return Err(CodingWorkspaceEngineError::ProviderStream(message));
        }
        if attempt.stage == CodingExecutionStage::Coding {
            self.complete_timeline_node(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                node_id,
                CodingTimelineNodeStatus::Failed,
                Some(message.clone()),
            )
            .await?;
            if let Some(role_run) = self.store.latest_role_run(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                CodingExecutionStage::Coding,
                CodingProviderRole::Coder,
            )? && role_run.status == CodingRoleRunStatus::Running
            {
                self.store.update_role_run_status(
                    &attempt.project_id,
                    &attempt.issue_id,
                    &attempt.id,
                    &role_run.id,
                    CodingRoleRunStatus::Failed,
                    Some("coder_provider_interrupted".to_string()),
                )?;
            }
            self.store.update_attempt_status(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                CodingAttemptStatus::Blocked,
            )?;
            let gate = self.store.create_blocked_gate(
                attempt,
                CreateBlockedGateInput {
                    attempt_id: attempt.id.clone(),
                    stage: CodingExecutionStage::Coding,
                    node_id: Some(node_id.to_string()),
                    role: Some(CodingProviderRole::Coder),
                    title: "Coder 执行中断".to_string(),
                    description: message.clone(),
                    reason_code: Some("coder_provider_interrupted".to_string()),
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
            return Err(CodingWorkspaceEngineError::ProviderStream(message));
        }
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
}
