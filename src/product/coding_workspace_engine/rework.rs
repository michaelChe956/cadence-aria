use super::*;

impl CodingWorkspaceEngine {
    pub async fn execute_rework(
        &self,
        attempt: &CodingExecutionAttempt,
        evidence: &str,
        provider: &dyn StreamingProviderAdapter,
    ) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
        let (_command_tx, mut command_rx) = mpsc::channel(1);
        self.execute_rework_with_commands(attempt, evidence, provider, &mut command_rx)
            .await
    }

    pub async fn execute_rework_with_commands(
        &self,
        attempt: &CodingExecutionAttempt,
        evidence: &str,
        provider: &dyn StreamingProviderAdapter,
        command_rx: &mut mpsc::Receiver<CodingRunnerCommand>,
    ) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
        let Some(worktree_path) = attempt.worktree_path.as_ref() else {
            return Err(CodingWorkspaceEngineError::MissingWorktree(
                attempt.id.clone(),
            ));
        };
        let current =
            self.store
                .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
        let source_stage = current.stage.clone();
        let rework_round = current.rework_count + 1;
        if current.status != CodingAttemptStatus::Running {
            self.store.update_attempt_status(
                &current.project_id,
                &current.issue_id,
                &current.id,
                CodingAttemptStatus::Running,
            )?;
        }
        let attempt = self.store.update_attempt_stage(
            &current.project_id,
            &current.issue_id,
            &current.id,
            CodingExecutionStage::Rework,
        )?;
        let node = self.create_rework_timeline_node(&attempt, rework_round)?;
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::CodingTimelineNodeCreated { node: node.clone() })
            .await;

        let role_run = match self.store.latest_role_run(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingExecutionStage::Rework,
            CodingProviderRole::Analyst,
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
                CodingExecutionStage::Rework,
                CodingProviderRole::Analyst,
                CodingRoleRunTrigger::Initial,
                Some(node.id.clone()),
            )?,
        };
        let evidence_ref = self.store.save_analyst_evidence(&attempt.id, evidence)?;
        self.store.update_role_run_refs(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &role_run.id,
            Vec::new(),
            vec![evidence_ref.clone()],
        )?;

        let notes = self.store.list_unconsumed_context_notes(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
        )?;
        let note_ids = notes.iter().map(|note| note.id.clone()).collect::<Vec<_>>();
        let context_note_input =
            format_rework_context_notes(&notes, REWORK_CONTEXT_NOTE_CHAR_LIMIT);
        let evaluation_context_json =
            self.evaluation_context_json_for_role(&attempt, EvaluationContextRole::Analyst)?;
        let analyst_provider = self
            .store
            .get_role_provider_config_snapshot(&attempt.project_id, &attempt.issue_id, &attempt.id)?
            .analyst;
        let retry_diagnostic = self.retry_diagnostic_for_previous_run(&attempt, &role_run)?;
        let prompt = build_rework_prompt(
            &attempt,
            evidence,
            &source_stage,
            rework_round,
            &context_note_input,
            &evaluation_context_json,
            retry_diagnostic.as_deref(),
        );
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::CodingExecutionEvent {
                event: provider_prompt_event(
                    &node.id,
                    &analyst_provider,
                    prompt.clone(),
                    CodingPromptMode::FullConversation.event_detail(),
                ),
            })
            .await;

        let input = AdapterInput {
            provider_type: provider_type_for_name(&analyst_provider),
            role: AdapterRole::Reviewer,
            worktree_path: Some(worktree_path.to_string_lossy().to_string()),
            prompt,
            context_files: Vec::new(),
            output_schema: "coding_workspace_analyst_verdict_json".to_string(),
            timeout: DEFAULT_PROVIDER_TIMEOUT_SECS,
            max_retries: 0,
        };
        let resume_provider_session_id = self.provider_resume_session_id_for_attempt(
            &attempt,
            &CodingProviderRole::Analyst,
            &analyst_provider,
        );
        let mut provider_input = streaming_input_from_adapter(&input, worktree_path.clone());
        provider_input.workspace_session_id = Some(attempt.id.clone());
        provider_input.resume_provider_session_id = resume_provider_session_id;
        provider_input.permission_mode =
            role_permission_mode_for_attempt(&self.store, &attempt, CodingProviderRole::Analyst)?;
        let full_output = self
            .run_provider_stream_to_completion(CodingProviderStreamRun {
                attempt: &attempt,
                node_id: &node.id,
                role_run: Some(&role_run),
                provider,
                legacy_input: &input,
                input: provider_input,
                provider_name: &analyst_provider,
                provider_role: CodingProviderRole::Analyst,
                command_rx,
                allow_legacy_stream_fallback: true,
                timeout: None,
                timeout_reason_code: None,
            })
            .await?;
        let analyst_raw_ref = self.store.save_provider_raw_output(
            &attempt.id,
            CodingExecutionStage::Rework,
            "analyst_decision",
            &full_output,
        )?;
        self.store.update_role_run_refs(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &role_run.id,
            vec![analyst_raw_ref.clone()],
            Vec::new(),
        )?;
        if !note_ids.is_empty() {
            self.store.mark_context_notes_consumed(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                &note_ids,
                rework_round,
            )?;
        }
        let mut decision = parse_analyst_verdict(&full_output, &source_stage);
        if decision.parse_error.is_some()
            && !decision
                .raw_provider_output_refs
                .iter()
                .any(|reference| reference == &analyst_raw_ref)
        {
            decision.raw_provider_output_refs.push(analyst_raw_ref);
        }
        let existing_decisions = self.store.list_analyst_decisions(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
        )?;
        let decision_record = AnalystDecisionRecord {
            id: next_sequential_id("analyst_decision", existing_decisions.len()),
            attempt_id: attempt.id.clone(),
            source_stage: source_stage.clone(),
            rework_round,
            verdict: decision.structured_verdict.clone(),
            next_stage: decision.next_stage.clone().unwrap_or_else(|| {
                default_next_stage_for_legacy_verdict(&decision.structured_verdict, &source_stage)
            }),
            reason: decision.reason.clone(),
            evidence_refs: decision.evidence_refs.clone(),
            raw_provider_output_refs: decision.raw_provider_output_refs.clone(),
            rework_instructions: decision.rework_instructions.clone(),
            human_gate: decision.human_gate.clone(),
            created_at: Utc::now().to_rfc3339(),
            parse_error: decision.parse_error.clone(),
            role_run_id: Some(role_run.id.clone()),
            run_no: Some(role_run.run_no),
        };
        self.store.save_analyst_decision(&decision_record)?;
        self.emit_analyst_verdict_entry(
            &attempt,
            &node.id,
            rework_round,
            &source_stage,
            &decision,
            &role_run,
        )
        .await;
        let (updated, node_status, summary) = self
            .apply_analyst_decision(&attempt, &node.id, &source_stage, rework_round, &decision)
            .await?;
        let role_run_status = if node_status == CodingTimelineNodeStatus::Blocked {
            CodingRoleRunStatus::Blocked
        } else {
            CodingRoleRunStatus::Completed
        };
        self.store.update_role_run_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &role_run.id,
            role_run_status,
            decision.parse_error.clone().or_else(|| {
                if node_status == CodingTimelineNodeStatus::Blocked {
                    Some("analyst_human_gate".to_string())
                } else {
                    None
                }
            }),
        )?;
        self.complete_timeline_node(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &node.id,
            node_status,
            Some(summary),
        )
        .await?;
        Ok(updated)
    }

    pub async fn execute_reviewer_driven_rework(
        &self,
        attempt: &CodingExecutionAttempt,
        review_report: &CodeReviewReport,
        context: &CodingExecutionContext,
        provider: &dyn StreamingProviderAdapter,
        command_rx: &mut mpsc::Receiver<CodingRunnerCommand>,
    ) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
        let current =
            self.store
                .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
        let rework_round = current.rework_count + 1;
        if current.rework_count >= current.max_auto_rework {
            let gate = self.store.create_blocked_gate(CreateBlockedGateInput {
                attempt_id: current.id.clone(),
                stage: CodingExecutionStage::Rework,
                node_id: None,
                role: Some(CodingProviderRole::Coder),
                title: "Code Review 返修超上限".to_string(),
                description: format!(
                    "code review 连续要求修改 {} 次，已达上限，请人工介入。\n\n最新 findings:\n{}",
                    current.rework_count,
                    review_findings_summary(review_report)
                ),
                reason_code: Some("reviewer_rework_limit_reached".to_string()),
                evidence_refs: review_report_evidence_refs(review_report),
                raw_provider_output_ref: review_report.raw_provider_output_ref.clone(),
                available_actions: Vec::new(),
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
                    CodingAttemptStatus::Blocked,
                )
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
        self.store.save_rework_instruction(&instruction)?;

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
        let rework_attempt = self.store.update_attempt_stage(
            &running.project_id,
            &running.issue_id,
            &running.id,
            CodingExecutionStage::Rework,
        )?;
        let updated = self.store.increment_attempt_rework_count(
            &rework_attempt.project_id,
            &rework_attempt.issue_id,
            &rework_attempt.id,
        )?;
        let node = self.create_rework_timeline_node(&updated, rework_round)?;
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
        let prompt = build_coding_delta_prompt(&updated, context, Some(&instruction), None);
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::CodingExecutionEvent {
                event: provider_prompt_event(
                    &node.id,
                    &coder_provider_name,
                    prompt.clone(),
                    CodingPromptMode::DeltaOnly.event_detail(),
                ),
            })
            .await;

        let role_run = self.store.create_role_run(
            &updated,
            CodingExecutionStage::Rework,
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
        let resume_provider_session_id = self.provider_resume_session_id_for_attempt(
            &updated,
            &CodingProviderRole::Coder,
            &coder_provider_name,
        );
        let mut provider_input = streaming_input_from_adapter(&input, worktree_path);
        provider_input.workspace_session_id = Some(updated.id.clone());
        provider_input.resume_provider_session_id = resume_provider_session_id;
        provider_input.permission_mode =
            role_permission_mode_for_attempt(&self.store, &updated, CodingProviderRole::Coder)?;
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
                timeout: None,
                timeout_reason_code: None,
            })
            .await?;
        let raw_provider_output_ref = self.store.save_provider_raw_output(
            &updated.id,
            CodingExecutionStage::Rework,
            "coder_rework",
            &full_output,
        )?;
        self.store.update_role_run_refs(
            &updated.project_id,
            &updated.issue_id,
            &updated.id,
            &role_run.id,
            vec![raw_provider_output_ref],
            Vec::new(),
        )?;
        self.store.update_role_run_status(
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
            Some(format!("reviewer 返修 round {}", rework_round)),
        )
        .await?;

        self.store
            .update_attempt_stage(
                &updated.project_id,
                &updated.issue_id,
                &updated.id,
                CodingExecutionStage::CodeReview,
            )
            .map_err(CodingWorkspaceEngineError::from)
    }

    pub fn continue_rework_after_limit(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        extra_context: Option<String>,
    ) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
        let current = self.store.get_attempt(project_id, issue_id, attempt_id)?;
        self.continue_rework_after_limit_for_attempt(&current, extra_context)
    }

    pub(crate) fn continue_rework_after_limit_for_attempt(
        &self,
        current: &CodingExecutionAttempt,
        extra_context: Option<String>,
    ) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
        if current.stage != CodingExecutionStage::Rework
            || !matches!(
                current.status,
                CodingAttemptStatus::Blocked | CodingAttemptStatus::WaitingForHuman
            )
            || current.rework_count < current.max_auto_rework
        {
            return Err(CodingWorkspaceEngineError::ProviderStream(
                "continue_rework_not_available".to_string(),
            ));
        }

        if let Some(content) = extra_context
            && !content.trim().is_empty()
        {
            self.store
                .create_context_note(&current.id, content.trim().to_string())?;
        }

        let decision = self
            .store
            .latest_analyst_decision(&current.project_id, &current.issue_id, &current.id)?
            .ok_or_else(|| {
                CodingWorkspaceEngineError::ProviderStream(
                    "continue_rework_missing_analyst_decision".to_string(),
                )
            })?;
        if decision.verdict != AnalystDecisionVerdict::NeedsFix
            || decision.next_stage != AnalystDecisionNextStage::Coding
        {
            return Err(CodingWorkspaceEngineError::ProviderStream(
                "continue_rework_latest_decision_not_coding".to_string(),
            ));
        }

        let existing = self.store.list_rework_instructions(
            &current.project_id,
            &current.issue_id,
            &current.id,
        )?;
        let (summary, fix_hints) = rework_instruction_fields_from_analyst_record(&decision);
        let instruction = CodingReworkInstruction {
            id: next_sequential_id("coding_rework_instruction", existing.len()),
            attempt_id: current.id.clone(),
            source_stage: decision.source_stage.clone(),
            rework_round: decision.rework_round,
            summary,
            fix_hints,
            questions: Vec::new(),
            created_at: Utc::now().to_rfc3339(),
            consumed_by_node_id: None,
            consumed_at: None,
        };
        self.store.save_rework_instruction(&instruction)?;

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
        let updated = self.store.increment_attempt_rework_count(
            &running.project_id,
            &running.issue_id,
            &running.id,
        )?;
        self.store
            .update_attempt_stage(
                &updated.project_id,
                &updated.issue_id,
                &updated.id,
                CodingExecutionStage::Coding,
            )
            .map_err(CodingWorkspaceEngineError::from)
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
