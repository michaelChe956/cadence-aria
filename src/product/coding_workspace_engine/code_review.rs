use super::*;

impl CodingWorkspaceEngine {
    pub async fn execute_code_review(
        &self,
        attempt: &CodingExecutionAttempt,
        provider: &dyn StreamingProviderAdapter,
    ) -> Result<CodeReviewReport, CodingWorkspaceEngineError> {
        let (_command_tx, mut command_rx) = mpsc::channel(1);
        self.execute_code_review_with_commands(attempt, provider, &mut command_rx)
            .await
    }

    pub async fn execute_code_review_with_commands(
        &self,
        attempt: &CodingExecutionAttempt,
        provider: &dyn StreamingProviderAdapter,
        command_rx: &mut mpsc::Receiver<CodingRunnerCommand>,
    ) -> Result<CodeReviewReport, CodingWorkspaceEngineError> {
        let attempt = self.store.ensure_provider_run_allowed(attempt)?;
        let Some(worktree_path) = attempt.worktree_path.as_ref() else {
            return Err(CodingWorkspaceEngineError::MissingWorktree(
                attempt.id.clone(),
            ));
        };
        let attempt = self.store.update_attempt_stage(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingExecutionStage::CodeReview,
        )?;
        let node = self.create_code_review_timeline_node(&attempt)?;
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::CodingTimelineNodeCreated { node: node.clone() })
            .await;

        let role_run = match self.store.latest_role_run(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingExecutionStage::CodeReview,
            CodingProviderRole::CodeReviewer,
        )? {
            Some(run) if run.status == CodingRoleRunStatus::Running && run.node_id.is_none() => {
                self.store.attach_role_run_node(
                    &attempt.project_id,
                    &attempt.issue_id,
                    &attempt.id,
                    &run.id,
                    node.id.clone(),
                )?
            }
            _ => self.store.create_role_run(
                &attempt,
                CodingExecutionStage::CodeReview,
                CodingProviderRole::CodeReviewer,
                CodingRoleRunTrigger::Initial,
                Some(node.id.clone()),
            )?,
        };

        let reviewer = self
            .store
            .get_role_provider_config_snapshot(&attempt.project_id, &attempt.issue_id, &attempt.id)?
            .code_reviewer;
        let retry_diagnostic = self.retry_diagnostic_for_previous_run(&attempt, &role_run)?;
        let prompt = match self.render_reviewer_unit_run_context(&attempt, &reviewer)? {
            Some(rendered) => rendered.text,
            None => {
                self.build_code_review_prompt(&attempt, worktree_path, retry_diagnostic.as_deref())
                    .await?
            }
        };
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::CodingExecutionEvent {
                event: provider_prompt_event(
                    &node.id,
                    &reviewer,
                    prompt.clone(),
                    CodingPromptMode::FullConversation.event_detail(),
                ),
            })
            .await;
        let input = AdapterInput {
            provider_type: provider_type_for_name(&reviewer),
            role: AdapterRole::Reviewer,
            worktree_path: Some(worktree_path.to_string_lossy().to_string()),
            prompt,
            context_files: Vec::new(),
            output_schema: "coding_workspace_code_review_json".to_string(),
            timeout: DEFAULT_PROVIDER_TIMEOUT_SECS,
            max_retries: 0,
        };
        let resume_provider_session_id = self.provider_resume_session_id_for_attempt(
            &attempt,
            &CodingProviderRole::CodeReviewer,
            &reviewer,
        );
        let mut provider_input = streaming_input_from_adapter(&input, worktree_path.clone());
        provider_input.workspace_session_id = Some(attempt.id.clone());
        provider_input.resume_provider_session_id = resume_provider_session_id;
        provider_input.permission_mode = role_permission_mode_for_attempt(
            &self.store,
            &attempt,
            CodingProviderRole::CodeReviewer,
        )?;
        let full_output = self
            .run_provider_stream_to_completion(CodingProviderStreamRun {
                attempt: &attempt,
                node_id: &node.id,
                role_run: Some(&role_run),
                provider,
                legacy_input: &input,
                input: provider_input,
                provider_name: &reviewer,
                provider_role: CodingProviderRole::CodeReviewer,
                command_rx,
                allow_legacy_stream_fallback: true,
                fresh_retry: None,
                timeout: None,
                timeout_reason_code: None,
            })
            .await?;
        let raw_provider_output_ref = self.store.save_provider_raw_output(
            &attempt,
            CodingExecutionStage::CodeReview,
            "code_review",
            &full_output,
        )?;
        self.store.update_role_run_refs(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &role_run.id,
            vec![raw_provider_output_ref.clone()],
            Vec::new(),
        )?;
        let report = self.build_code_review_report(
            &attempt,
            &full_output,
            Some(raw_provider_output_ref.clone()),
            &role_run,
        )?;
        let plan_defect_source = PlanDefectSource::CodeReviewer;
        let reviewer_projection = self.reviewer_projection_for_attempt(&attempt)?;
        let plan_defect_route = code_review_flow_decision(&report, &reviewer_projection);
        self.store.save_code_review_report(&attempt, &report)?;
        self.emit_code_review_chat_entry(
            &attempt,
            &node.id,
            &report,
            plan_defect_source.label(),
            plan_defect_route.label(),
        )
        .await;
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::CodeReviewComplete {
                report: Box::new(report.clone()),
            })
            .await;
        let (node_status, summary, role_run_status, reason_code) = match report.verdict {
            ReviewVerdict::Approve => (
                CodingTimelineNodeStatus::Completed,
                Some("code review 通过".to_string()),
                CodingRoleRunStatus::Completed,
                None,
            ),
            ReviewVerdict::RequestChanges => (
                CodingTimelineNodeStatus::Failed,
                Some("code review 要求修改".to_string()),
                CodingRoleRunStatus::Completed,
                None,
            ),
            ReviewVerdict::Blocked => (
                CodingTimelineNodeStatus::Blocked,
                Some("code review 被阻塞".to_string()),
                CodingRoleRunStatus::Blocked,
                Some("code_review_blocked".to_string()),
            ),
        };
        self.complete_timeline_node(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &node.id,
            node_status,
            summary,
        )
        .await?;
        self.store.update_role_run_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &role_run.id,
            role_run_status,
            reason_code,
        )?;
        if report.verdict == ReviewVerdict::Blocked
            && !code_review_report_has_actionable_findings(&report)
        {
            self.create_review_blocked_gate(ReviewBlockedGateInput {
                attempt: &attempt,
                node_id: &node.id,
                stage: CodingExecutionStage::CodeReview,
                role: CodingProviderRole::CodeReviewer,
                title: "Code review blocked".to_string(),
                description: report.summary.clone(),
                reason_code: "code_review_blocked",
                evidence_refs: vec![report.id.clone()],
                raw_provider_output_ref: Some(raw_provider_output_ref.clone()),
            })
            .await?;
        }
        // 既有的 code_review_blocked 门禁已落地时，不再重复落地分诊门禁。
        // 既有分支硬条件是 verdict==Blocked && !actionable，该情形已被 plan_defect.rs
        // 判定为 StopForHumanTriage（plan_defect.rs:177-178），由上面分支统一落
        // code_review_blocked gate；新分诊门禁必须跳过，避免 double-gate。
        let code_review_blocked_landed = report.verdict == ReviewVerdict::Blocked
            && !code_review_report_has_actionable_findings(&report);
        if !code_review_blocked_landed {
            // 仅处理三个分诊决策；RunCoderFix / StartPlanRepair /
            // ContinueAfterApprove / StartStoryAmendment / StartDesignAmendment 由
            // runner 后续处理，不在此落门禁。门禁动作不触发 plan repair。
            let triage_reason_code: Option<(&'static str, &'static str)> = match plan_defect_route {
                CodeReviewFlowDecision::StopForHumanTriage => Some((
                    "code_review_output_human_triage",
                    "Code Review 结论需人工分诊",
                )),
                CodeReviewFlowDecision::RetryVerification => Some((
                    "code_review_verification_incomplete",
                    "Code Review 验证证据不完整",
                )),
                CodeReviewFlowDecision::OpenOperationalGate => Some((
                    "code_review_operational_blocker",
                    "Code Review 命中运维阻塞",
                )),
                _ => None,
            };
            if let Some((reason_code, title)) = triage_reason_code {
                self.create_review_blocked_gate(ReviewBlockedGateInput {
                    attempt: &attempt,
                    node_id: &node.id,
                    stage: CodingExecutionStage::CodeReview,
                    role: CodingProviderRole::CodeReviewer,
                    title: title.to_string(),
                    description: report.summary.clone(),
                    reason_code,
                    evidence_refs: vec![report.id.clone()],
                    raw_provider_output_ref: Some(raw_provider_output_ref.clone()),
                })
                .await?;
            }
        }
        Ok(report)
    }
}
