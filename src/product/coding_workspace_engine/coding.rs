use super::*;

pub(crate) struct CoderExecutionOutcome {
    pub(crate) attempt: CodingExecutionAttempt,
    pub(crate) plan_defect_decision: Option<CodeReviewFlowDecision>,
    pub(crate) plan_defect_report: Option<ExecutionPlanDefectReport>,
}

impl CodingWorkspaceEngine {
    pub async fn execute_coding(
        &self,
        attempt: &CodingExecutionAttempt,
        provider: &dyn StreamingProviderAdapter,
        context: &CodingExecutionContext,
    ) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
        let (_command_tx, mut command_rx) = mpsc::channel(1);
        self.execute_coding_with_commands(attempt, provider, context, &mut command_rx)
            .await
    }

    pub async fn execute_coding_with_commands(
        &self,
        attempt: &CodingExecutionAttempt,
        provider: &dyn StreamingProviderAdapter,
        context: &CodingExecutionContext,
        command_rx: &mut mpsc::Receiver<CodingRunnerCommand>,
    ) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
        Ok(self
            .execute_coding_with_commands_outcome(attempt, provider, context, command_rx)
            .await?
            .attempt)
    }

    pub(crate) async fn execute_coding_with_commands_outcome(
        &self,
        attempt: &CodingExecutionAttempt,
        provider: &dyn StreamingProviderAdapter,
        context: &CodingExecutionContext,
        command_rx: &mut mpsc::Receiver<CodingRunnerCommand>,
    ) -> Result<CoderExecutionOutcome, CodingWorkspaceEngineError> {
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
            CodingExecutionStage::Coding,
        )?;
        let node = self.create_coding_timeline_node(&attempt)?;
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::CodingTimelineNodeCreated { node: node.clone() })
            .await;
        let role_run = match self.store.latest_role_run(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingExecutionStage::Coding,
            CodingProviderRole::Coder,
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
                CodingExecutionStage::Coding,
                CodingProviderRole::Coder,
                CodingRoleRunTrigger::Initial,
                Some(node.id.clone()),
            )?,
        };

        let coder_provider = self
            .store
            .get_role_provider_config_snapshot(&attempt.project_id, &attempt.issue_id, &attempt.id)?
            .coder;
        let mut resume_provider_session_id = self.provider_resume_session_id_for_attempt(
            &attempt,
            &CodingProviderRole::Coder,
            &coder_provider,
        );
        if resume_provider_session_id.is_some()
            && !self.should_resume_coder_session_for_role_run(&attempt, &role_run.id)?
        {
            resume_provider_session_id = None;
        }
        let rework_instruction = self.store.latest_unconsumed_rework_instruction(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
        )?;
        let context_notes = self.store.list_unconsumed_context_notes(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
        )?;
        let context_note_ids = context_notes
            .iter()
            .map(|note| note.id.clone())
            .collect::<Vec<_>>();
        let context_note_input =
            format_rework_context_notes(&context_notes, REWORK_CONTEXT_NOTE_CHAR_LIMIT);
        let coding_context_notes = (!context_note_ids.is_empty()).then_some(&context_note_input);
        let rendered_context = self.render_coder_unit_run_context(
            &attempt,
            &coder_provider,
            rework_instruction
                .as_ref()
                .map(|instruction| instruction.summary.clone()),
        )?;
        let full_prompt = rendered_context
            .as_ref()
            .map(|rendered| rendered.text.clone())
            .unwrap_or_else(|| {
                build_coding_prompt(
                    &attempt,
                    context,
                    rework_instruction.as_ref(),
                    coding_context_notes,
                )
            });
        let prompt_mode = if rendered_context.is_some() || resume_provider_session_id.is_none() {
            CodingPromptMode::FullConversation
        } else {
            CodingPromptMode::DeltaOnly
        };
        let prompt = match prompt_mode {
            CodingPromptMode::FullConversation => full_prompt.clone(),
            CodingPromptMode::DeltaOnly => build_coding_delta_prompt(
                &attempt,
                context,
                rework_instruction.as_ref(),
                coding_context_notes,
            ),
        };
        if let Some(instruction) = rework_instruction.as_ref() {
            self.store.mark_rework_instruction_consumed(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                &instruction.id,
                &node.id,
            )?;
        }
        if !context_note_ids.is_empty() {
            self.store.mark_context_notes_consumed(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                &context_note_ids,
                attempt.rework_count,
            )?;
        }

        let retry_success = self
            .run_coder_with_retry_cycle(CoderRetryCycleInput {
                attempt: &attempt,
                initial_node: node,
                initial_role_run: role_run,
                provider,
                provider_name: &coder_provider,
                worktree_path,
                initial_prompt: prompt,
                fresh_prompt: full_prompt,
                initial_prompt_mode: prompt_mode,
                initial_resume_provider_session_id: resume_provider_session_id,
                command_rx,
            })
            .await?;
        let full_output = retry_success.outcome.full_output;
        let role_run = retry_success.role_run;
        let node = retry_success.node;
        let (plan_defect_report, plan_defect_decision, plan_defect_error) =
            match parse_execution_plan_defects(PlanDefectSource::Coder, &full_output) {
                Ok(report) if report.findings.is_empty() => (None, None, None),
                Ok(report) => {
                    let projection = self.reviewer_projection_for_attempt(&attempt)?;
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
            &attempt,
            CodingExecutionStage::Coding,
            "coder_output",
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
        let completed_role_run = self.store.update_role_run_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &role_run.id,
            CodingRoleRunStatus::Completed,
            None,
        )?;
        self.complete_timeline_node(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &node.id,
            CodingTimelineNodeStatus::Completed,
            Some("代码编写完成".to_string()),
        )
        .await?;
        self.emit_coder_output_chat_entry(CoderOutputChatEntryInput {
            attempt: &attempt,
            node_id: &node.id,
            provider_name: &coder_provider,
            role_run: &completed_role_run,
            full_output: &full_output,
            raw_provider_output_ref: &raw_provider_output_ref,
            source: "coding",
            plan_defect_route,
        })
        .await;
        let attempt = if plan_defect_decision == Some(CodeReviewFlowDecision::StopForHumanTriage) {
            self.open_coding_output_human_triage_gate(
                &attempt,
                &node.id,
                plan_defect_report.as_ref(),
                plan_defect_error.as_deref(),
                Some(raw_provider_output_ref.clone()),
            )?
        } else {
            attempt
        };
        Ok(CoderExecutionOutcome {
            attempt,
            plan_defect_decision,
            plan_defect_report,
        })
    }

    fn should_resume_coder_session_for_role_run(
        &self,
        attempt: &CodingExecutionAttempt,
        current_role_run_id: &str,
    ) -> Result<bool, CodingWorkspaceEngineError> {
        if attempt.scope != crate::product::coding_models::CodingAttemptScope::WorkItemGroup {
            return Ok(true);
        }

        let Some(active_unit) = self.store.get_active_coding_unit(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
        )?
        else {
            return Ok(false);
        };
        let Some(unit_started_at) = active_unit.started_at.as_deref() else {
            return Ok(false);
        };

        Ok(self
            .store
            .list_role_runs(&attempt.project_id, &attempt.issue_id, &attempt.id)?
            .into_iter()
            .any(|run| {
                run.id != current_role_run_id
                    && run.stage == CodingExecutionStage::Coding
                    && run.role == CodingProviderRole::Coder
                    && started_at_or_after(&run.started_at, unit_started_at)
            }))
    }

    pub(crate) async fn emit_coder_output_chat_entry(&self, input: CoderOutputChatEntryInput<'_>) {
        let CoderOutputChatEntryInput {
            attempt,
            node_id,
            provider_name,
            role_run,
            full_output,
            raw_provider_output_ref,
            source,
            plan_defect_route,
        } = input;
        let completed_at = role_run
            .completed_at
            .clone()
            .unwrap_or_else(|| Utc::now().to_rfc3339());
        let entry = CodingChatEntry {
            id: format!("{node_id}_coder_output"),
            attempt_id: attempt.id.clone(),
            node_id: Some(node_id.to_string()),
            role: CodingAgentRole::Author,
            entry_type: CodingEntryType::AssistantMessage,
            content: Some(full_output.to_string()),
            metadata: Some(serde_json::json!({
                "source": source,
                "provider": provider_name,
                "role_run_id": role_run.id,
                "run_no": role_run.run_no,
                "raw_provider_output_ref": raw_provider_output_ref,
                "plan_defect_route": plan_defect_route,
                "started_at": role_run.started_at,
                "completed_at": completed_at
            })),
            created_at: completed_at,
        };
        self.save_and_emit_chat_entry(attempt, entry).await;
    }
}

fn started_at_or_after(left: &str, right: &str) -> bool {
    match (
        chrono::DateTime::parse_from_rfc3339(left),
        chrono::DateTime::parse_from_rfc3339(right),
    ) {
        (Ok(left), Ok(right)) => left >= right,
        _ => left >= right,
    }
}
