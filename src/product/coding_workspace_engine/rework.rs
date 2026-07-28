use super::coding::CoderExecutionOutcome;
use super::*;

impl CodingWorkspaceEngine {
    pub async fn execute_coder_fix_from_review(
        &self,
        attempt: &CodingExecutionAttempt,
        review_report: &CodeReviewReport,
        context: &CodingExecutionContext,
        provider: &dyn StreamingProviderAdapter,
        command_rx: &mut mpsc::Receiver<CodingRunnerCommand>,
    ) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
        Ok(self
            .execute_coder_fix_from_review_outcome(
                attempt,
                review_report,
                context,
                provider,
                command_rx,
            )
            .await?
            .attempt)
    }

    pub(crate) async fn execute_coder_fix_from_review_outcome(
        &self,
        attempt: &CodingExecutionAttempt,
        review_report: &CodeReviewReport,
        context: &CodingExecutionContext,
        provider: &dyn StreamingProviderAdapter,
        command_rx: &mut mpsc::Receiver<CodingRunnerCommand>,
    ) -> Result<CoderExecutionOutcome, CodingWorkspaceEngineError> {
        let current = self.store.ensure_provider_run_allowed(attempt)?;
        let rework_round = current.rework_count + 1;
        if current.rework_count >= current.max_auto_rework {
            let actions = vec![
                coding_gate_action_for_id("provide_context").expect("provide context action"),
                coding_gate_action_for_id("send_to_coder").expect("send to coder action"),
                coding_gate_action_for_id("abort").expect("abort action"),
            ];
            let gate = self.store.create_blocked_gate(&current, CreateBlockedGateInput {
                attempt_id: current.id.clone(),
                stage: CodingExecutionStage::CodeReview,
                node_id: None,
                role: Some(CodingProviderRole::CodeReviewer),
                title: "Code Review 修复超上限".to_string(),
                description: format!(
                    "code review 连续要求修复 {} 次，已达上限，请人工介入。\n\n最新 findings:\n{}",
                    current.rework_count,
                    review_findings_summary(review_report)
                ),
                reason_code: Some("reviewer_rework_limit_reached".to_string()),
                evidence_refs: review_report_evidence_refs(review_report),
                raw_provider_output_ref: review_report.raw_provider_output_ref.clone(),
                available_actions: actions,
            })?;
            let _ = self
                .event_tx
                .send(CodingWsOutMessage::CodingGateRequired { gate })
                .await;
            return self
                .store
                .update_attempt_status(
                    &current.project_id,
                    &current.issue_id,
                    &current.id,
                    CodingAttemptStatus::WaitingForHuman,
                )
                .map(|attempt| CoderExecutionOutcome {
                    attempt,
                    plan_defect_decision: None,
                    plan_defect_report: None,
                })
                .map_err(CodingWorkspaceEngineError::from);
        }

        let Some(worktree_path) = current.worktree_path.clone() else {
            return Err(CodingWorkspaceEngineError::MissingWorktree(
                current.id.clone(),
            ));
        };

        let existing_instructions = self.store.list_rework_instructions(
            &current.project_id,
            &current.issue_id,
            &current.id,
        )?;
        let instruction = CodingReworkInstruction {
            id: next_sequential_id("coding_rework_instruction", existing_instructions.len()),
            attempt_id: current.id.clone(),
            source_stage: CodingExecutionStage::CodeReview,
            rework_round,
            summary: if review_report.summary.trim().is_empty() {
                format!("code review round {} 要求修改", review_report.round)
            } else {
                review_report.summary.clone()
            },
            fix_hints: review_findings_fix_hints(review_report),
            questions: Vec::new(),
            created_at: Utc::now().to_rfc3339(),
            consumed_by_node_id: None,
            consumed_at: None,
        };
        self.store.save_rework_instruction(&current, &instruction)?;

        let running = if current.status == CodingAttemptStatus::Running {
            current
        } else {
            self.store.update_attempt_status(
                &current.project_id,
                &current.issue_id,
                &current.id,
                CodingAttemptStatus::Running,
            )?
        };
        let coding_attempt = self.store.update_attempt_stage(
            &running.project_id,
            &running.issue_id,
            &running.id,
            CodingExecutionStage::Coding,
        )?;
        let updated = self.store.increment_attempt_rework_count(
            &coding_attempt.project_id,
            &coding_attempt.issue_id,
            &coding_attempt.id,
        )?;
        let node = self.create_coding_timeline_node(&updated)?;
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::CodingTimelineNodeCreated { node: node.clone() })
            .await;

        self.store.mark_rework_instruction_consumed(
            &updated.project_id,
            &updated.issue_id,
            &updated.id,
            &instruction.id,
            &node.id,
        )?;

        let coder_provider_name = self
            .store
            .get_role_provider_config_snapshot(&updated.project_id, &updated.issue_id, &updated.id)?
            .coder;
        let resume_provider_session_id = self.provider_resume_session_id_for_attempt(
            &updated,
            &CodingProviderRole::Coder,
            &coder_provider_name,
        );
        let rendered_context =
            self.render_coder_unit_run_context(&updated, &coder_provider_name, None)?;
        let delta_prompt = build_coding_delta_prompt(&updated, context, Some(&instruction), None);
        let full_prompt = rendered_context
            .as_ref()
            .map(|rendered| format!("{}\n\n{}", rendered.text, delta_prompt))
            .unwrap_or_else(|| build_coding_prompt(&updated, context, Some(&instruction), None));
        let prompt_mode = if rendered_context.is_some() || resume_provider_session_id.is_none() {
            CodingPromptMode::FullConversation
        } else {
            CodingPromptMode::DeltaOnly
        };
        let prompt = match prompt_mode {
            CodingPromptMode::FullConversation => full_prompt.clone(),
            CodingPromptMode::DeltaOnly => delta_prompt,
        };
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::CodingExecutionEvent {
                event: provider_prompt_event(
                    &node.id,
                    &coder_provider_name,
                    prompt.clone(),
                    prompt_mode.event_detail(),
                ),
            })
            .await;

        let role_run = self.store.create_role_run(
            &updated,
            CodingExecutionStage::Coding,
            CodingProviderRole::Coder,
            CodingRoleRunTrigger::Initial,
            Some(node.id.clone()),
        )?;
        let input = AdapterInput {
            provider_type: provider_type_for_name(&coder_provider_name),
            role: AdapterRole::Executor,
            worktree_path: Some(worktree_path.to_string_lossy().to_string()),
            prompt,
            context_files: Vec::new(),
            output_schema: "coding_workspace_markdown".to_string(),
            timeout: DEFAULT_PROVIDER_TIMEOUT_SECS,
            max_retries: 0,
        };
        let mut provider_input = streaming_input_from_adapter(&input, worktree_path);
        provider_input.workspace_session_id = Some(updated.id.clone());
        provider_input.resume_provider_session_id = resume_provider_session_id.clone();
        provider_input.permission_mode =
            role_permission_mode_for_attempt(&self.store, &updated, CodingProviderRole::Coder)?;
        let fresh_retry = (coder_provider_name == ProviderName::Codex
            && resume_provider_session_id.is_some())
        .then(|| {
            let fresh_legacy_input = AdapterInput {
                prompt: full_prompt.clone(),
                ..input.clone()
            };
            let mut fresh_input = provider_input.clone();
            fresh_input.prompt = full_prompt;
            fresh_input.resume_provider_session_id = None;
            CodingProviderFreshRetry {
                legacy_input: fresh_legacy_input,
                input: fresh_input,
            }
        });
        let full_output = self
            .run_provider_stream_to_completion(CodingProviderStreamRun {
                attempt: &updated,
                node_id: &node.id,
                role_run: Some(&role_run),
                provider,
                legacy_input: &input,
                input: provider_input,
                provider_name: &coder_provider_name,
                provider_role: CodingProviderRole::Coder,
                command_rx,
                allow_legacy_stream_fallback: true,
                fresh_retry,
                timeout: None,
                timeout_reason_code: None,
            })
            .await?;
        let (plan_defect_report, plan_defect_decision, plan_defect_error) =
            match parse_execution_plan_defects(PlanDefectSource::Coder, &full_output) {
                Ok(report) if report.findings.is_empty() => (None, None, None),
                Ok(report) => {
                    let projection = self.reviewer_projection_for_attempt(&updated)?;
                    let decision = execution_plan_defect_flow_decision(&report, &projection);
                    (Some(report), Some(decision), None)
                }
                Err(error) => (
                    None,
                    Some(CodeReviewFlowDecision::StopForHumanTriage),
                    Some(error.to_string()),
                ),
            };
        let plan_defect_route = plan_defect_decision.map(CodeReviewFlowDecision::label);
        let raw_provider_output_ref = self.store.save_provider_raw_output(
            &updated,
            CodingExecutionStage::Coding,
            "coder_output",
            &full_output,
        )?;
        self.store.update_role_run_refs(
            &updated.project_id,
            &updated.issue_id,
            &updated.id,
            &role_run.id,
            vec![raw_provider_output_ref.clone()],
            Vec::new(),
        )?;
        let completed_role_run = self.store.update_role_run_status(
            &updated.project_id,
            &updated.issue_id,
            &updated.id,
            &role_run.id,
            CodingRoleRunStatus::Completed,
            None,
        )?;
        self.complete_timeline_node(
            &updated.project_id,
            &updated.issue_id,
            &updated.id,
            &node.id,
            CodingTimelineNodeStatus::Completed,
            Some(format!("reviewer 修复 round {}", rework_round)),
        )
        .await?;
        self.emit_coder_output_chat_entry(CoderOutputChatEntryInput {
            attempt: &updated,
            node_id: &node.id,
            provider_name: &coder_provider_name,
            role_run: &completed_role_run,
            full_output: &full_output,
            raw_provider_output_ref: &raw_provider_output_ref,
            source: "coding",
            plan_defect_route,
        })
        .await;

        let attempt = if plan_defect_decision == Some(CodeReviewFlowDecision::StopForHumanTriage) {
            self.open_coding_output_human_triage_gate(
                &updated,
                &node.id,
                plan_defect_report.as_ref(),
                plan_defect_error.as_deref(),
                Some(raw_provider_output_ref.clone()),
            )?
        } else if plan_defect_decision
            .is_some_and(|decision| decision != CodeReviewFlowDecision::RunCoderFix)
        {
            updated
        } else {
            self.store
                .update_attempt_stage(
                    &updated.project_id,
                    &updated.issue_id,
                    &updated.id,
                    CodingExecutionStage::CodeReview,
                )
                .map_err(CodingWorkspaceEngineError::from)?
        };
        Ok(CoderExecutionOutcome {
            attempt,
            plan_defect_decision,
            plan_defect_report,
        })
    }

    pub fn send_review_limit_feedback_to_coder_for_attempt(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        extra_context: Option<String>,
    ) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
        let current = self.store.get_attempt(project_id, issue_id, attempt_id)?;
        self.send_review_limit_feedback_to_coder(&current, extra_context)
    }

    pub(crate) fn send_review_limit_feedback_to_coder(
        &self,
        current: &CodingExecutionAttempt,
        extra_context: Option<String>,
    ) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
        if current.stage != CodingExecutionStage::CodeReview
            || !matches!(
                current.status,
                CodingAttemptStatus::Blocked | CodingAttemptStatus::WaitingForHuman
            )
            || current.rework_count < current.max_auto_rework
        {
            return Err(CodingWorkspaceEngineError::ProviderStream(
                "send_to_coder_not_available".to_string(),
            ));
        }

        if let Some(content) = extra_context
            && !content.trim().is_empty()
        {
            self.store
                .create_context_note(current, content.trim().to_string())?;
        }

        let review_report = self
            .store
            .list_code_review_reports(&current.project_id, &current.issue_id, &current.id)?
            .into_iter()
            .last()
            .ok_or_else(|| {
                CodingWorkspaceEngineError::ProviderStream(
                    "send_to_coder_missing_code_review_report".to_string(),
                )
            })?;
        let can_continue_from_review = review_report.verdict == ReviewVerdict::RequestChanges
            || (review_report.verdict == ReviewVerdict::Blocked
                && code_review_report_has_actionable_findings(&review_report));
        if !can_continue_from_review {
            return Err(CodingWorkspaceEngineError::ProviderStream(
                "send_to_coder_latest_review_not_actionable".to_string(),
            ));
        }

        let existing = self.store.list_rework_instructions(
            &current.project_id,
            &current.issue_id,
            &current.id,
        )?;
        let instruction = CodingReworkInstruction {
            id: next_sequential_id("coding_rework_instruction", existing.len()),
            attempt_id: current.id.clone(),
            source_stage: CodingExecutionStage::CodeReview,
            rework_round: current.rework_count + 1,
            summary: if review_report.summary.trim().is_empty() {
                format!("code review round {} 要求修改", review_report.round)
            } else {
                review_report.summary.clone()
            },
            fix_hints: review_findings_fix_hints(&review_report),
            questions: Vec::new(),
            created_at: Utc::now().to_rfc3339(),
            consumed_by_node_id: None,
            consumed_at: None,
        };
        self.store.save_rework_instruction(current, &instruction)?;

        let running = if current.status == CodingAttemptStatus::Running {
            current.clone()
        } else {
            self.store.update_attempt_status(
                &current.project_id,
                &current.issue_id,
                &current.id,
                CodingAttemptStatus::Running,
            )?
        };
        let coding_attempt = self.store.update_attempt_stage(
            &running.project_id,
            &running.issue_id,
            &running.id,
            CodingExecutionStage::Coding,
        )?;
        let updated = self.store.increment_attempt_rework_count(
            &coding_attempt.project_id,
            &coding_attempt.issue_id,
            &coding_attempt.id,
        )?;
        Ok(updated)
    }

    pub(crate) fn send_code_review_feedback_to_coder(
        &self,
        current: &CodingExecutionAttempt,
        extra_context: Option<String>,
    ) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
        if current.stage != CodingExecutionStage::CodeReview
            || !matches!(
                current.status,
                CodingAttemptStatus::Blocked | CodingAttemptStatus::WaitingForHuman
            )
        {
            return Err(CodingWorkspaceEngineError::ProviderStream(
                "send_to_coder_not_available".to_string(),
            ));
        }

        let operator_context = extra_context
            .map(|content| content.trim().to_string())
            .filter(|content| !content.is_empty())
            .ok_or_else(|| {
                CodingWorkspaceEngineError::ProviderStream(
                    "coding_gate_extra_context_required".to_string(),
                )
            })?;
        self.store.create_context_note(current, operator_context)?;

        let review_report = self
            .store
            .list_code_review_reports(&current.project_id, &current.issue_id, &current.id)?
            .into_iter()
            .last()
            .ok_or_else(|| {
                CodingWorkspaceEngineError::ProviderStream(
                    "send_to_coder_missing_code_review_report".to_string(),
                )
            })?;
        if !matches!(
            review_report.verdict,
            ReviewVerdict::Blocked | ReviewVerdict::RequestChanges
        ) {
            return Err(CodingWorkspaceEngineError::ProviderStream(
                "send_to_coder_latest_review_not_actionable".to_string(),
            ));
        }

        let existing = self.store.list_rework_instructions(
            &current.project_id,
            &current.issue_id,
            &current.id,
        )?;
        let instruction = CodingReworkInstruction {
            id: next_sequential_id("coding_rework_instruction", existing.len()),
            attempt_id: current.id.clone(),
            source_stage: CodingExecutionStage::CodeReview,
            rework_round: current.rework_count + 1,
            summary: if review_report.summary.trim().is_empty() {
                format!("code review round {} 被阻塞", review_report.round)
            } else {
                review_report.summary.clone()
            },
            fix_hints: review_findings_fix_hints(&review_report),
            questions: Vec::new(),
            created_at: Utc::now().to_rfc3339(),
            consumed_by_node_id: None,
            consumed_at: None,
        };
        self.store.save_rework_instruction(current, &instruction)?;

        let running = if current.status == CodingAttemptStatus::Running {
            current.clone()
        } else {
            self.store.update_attempt_status(
                &current.project_id,
                &current.issue_id,
                &current.id,
                CodingAttemptStatus::Running,
            )?
        };
        let coding_attempt = self.store.update_attempt_stage(
            &running.project_id,
            &running.issue_id,
            &running.id,
            CodingExecutionStage::Coding,
        )?;
        let updated = self.store.increment_attempt_rework_count(
            &coding_attempt.project_id,
            &coding_attempt.issue_id,
            &coding_attempt.id,
        )?;
        Ok(updated)
    }
}

fn review_findings_summary(review_report: &CodeReviewReport) -> String {
    if review_report.findings.is_empty() {
        return "- reviewer 未提供结构化 findings".to_string();
    }
    review_report
        .findings
        .iter()
        .map(|finding| format!("- [{:?}] {}", finding.severity, finding.message))
        .collect::<Vec<_>>()
        .join("\n")
}

fn review_findings_fix_hints(review_report: &CodeReviewReport) -> Vec<String> {
    review_report
        .findings
        .iter()
        .filter(|finding| {
            matches!(
                finding.severity,
                crate::product::coding_models::FindingSeverity::Error
                    | crate::product::coding_models::FindingSeverity::Warning
            )
        })
        .map(review_finding_fix_hint)
        .collect()
}

fn review_finding_fix_hint(finding: &ReviewFinding) -> String {
    let location = match (&finding.file_path, finding.line) {
        (Some(path), Some(line)) => format!("{path}:{line} "),
        (Some(path), None) => format!("{path} "),
        _ => String::new(),
    };
    let action = finding
        .required_action
        .as_deref()
        .map(|action| format!(" -> {action}"))
        .unwrap_or_default();
    format!("{location}{}{action}", finding.message)
}

fn review_report_evidence_refs(review_report: &CodeReviewReport) -> Vec<String> {
    let mut refs = Vec::new();
    refs.extend(review_report.tested_evidence_refs.iter().cloned());
    refs.extend(review_report.diff_refs.iter().cloned());
    refs.extend(
        review_report
            .findings
            .iter()
            .flat_map(|finding| finding.evidence.iter().cloned()),
    );
    refs.sort();
    refs.dedup();
    refs
}
