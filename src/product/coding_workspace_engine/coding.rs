use super::*;

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
        let role_run = self.store.create_role_run(
            &attempt,
            CodingExecutionStage::Coding,
            CodingProviderRole::Coder,
            CodingRoleRunTrigger::Initial,
            Some(node.id.clone()),
        )?;

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
        let full_prompt = build_coding_prompt(
            &attempt,
            context,
            rework_instruction.as_ref(),
            coding_context_notes,
        );
        let prompt_mode = if resume_provider_session_id.is_some() {
            CodingPromptMode::DeltaOnly
        } else {
            CodingPromptMode::FullConversation
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
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::CodingExecutionEvent {
                event: provider_prompt_event(
                    &node.id,
                    &coder_provider,
                    prompt.clone(),
                    prompt_mode.event_detail(),
                ),
            })
            .await;
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

        let legacy_input = AdapterInput {
            provider_type: provider_type_for_name(&coder_provider),
            role: AdapterRole::Executor,
            worktree_path: Some(worktree_path.to_string_lossy().to_string()),
            prompt,
            context_files: Vec::new(),
            output_schema: "coding_workspace_markdown".to_string(),
            timeout: DEFAULT_PROVIDER_TIMEOUT_SECS,
            max_retries: 0,
        };
        let permission_mode =
            role_permission_mode_for_attempt(&self.store, &attempt, CodingProviderRole::Coder)?;
        let input = StreamingProviderInput {
            provider_type: legacy_input.provider_type.clone(),
            role: legacy_input.role.clone(),
            prompt: legacy_input.prompt.clone(),
            working_dir: worktree_path.clone(),
            workspace_session_id: Some(attempt.id.clone()),
            resume_provider_session_id: resume_provider_session_id.clone(),
            permission_mode,
            structured_output_contract: None,
            env_vars: BTreeMap::new(),
            timeout_secs: legacy_input.timeout,
        };
        let fresh_retry = (coder_provider == ProviderName::Codex
            && resume_provider_session_id.is_some())
        .then(|| {
            let fresh_legacy_input = AdapterInput {
                prompt: full_prompt.clone(),
                ..legacy_input.clone()
            };
            let mut fresh_input = input.clone();
            fresh_input.prompt = full_prompt;
            fresh_input.resume_provider_session_id = None;
            CodingProviderFreshRetry {
                legacy_input: fresh_legacy_input,
                input: fresh_input,
            }
        });
        let stream_result = self
            .run_provider_stream_to_completion(CodingProviderStreamRun {
                attempt: &attempt,
                node_id: &node.id,
                role_run: Some(&role_run),
                provider,
                legacy_input: &legacy_input,
                input,
                provider_name: &coder_provider,
                provider_role: CodingProviderRole::Coder,
                command_rx,
                allow_legacy_stream_fallback: true,
                fresh_retry,
                timeout: None,
                timeout_reason_code: None,
            })
            .await;
        let full_output = match stream_result {
            Ok(output) => output,
            Err(error) => {
                let (status, reason_code) = coder_role_run_failure_status(&error);
                let _ = self.store.update_role_run_status(
                    &attempt.project_id,
                    &attempt.issue_id,
                    &attempt.id,
                    &role_run.id,
                    status,
                    reason_code,
                );
                return Err(error);
            }
        };
        let raw_provider_output_ref = self.store.save_provider_raw_output(
            &attempt.id,
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
        })
        .await;
        Ok(attempt)
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
                "started_at": role_run.started_at,
                "completed_at": completed_at
            })),
            created_at: completed_at,
        };
        self.save_and_emit_chat_entry(entry).await;
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

fn coder_role_run_failure_status(
    error: &CodingWorkspaceEngineError,
) -> (CodingRoleRunStatus, Option<String>) {
    match error {
        CodingWorkspaceEngineError::Aborted => (
            CodingRoleRunStatus::Aborted,
            Some("abort_attempt".to_string()),
        ),
        CodingWorkspaceEngineError::ProviderStream(message)
            if message == "provider_choice_unresolved" =>
        {
            (
                CodingRoleRunStatus::Blocked,
                Some("provider_choice_unresolved".to_string()),
            )
        }
        CodingWorkspaceEngineError::ProviderStream(message) => {
            (CodingRoleRunStatus::Failed, Some(message.clone()))
        }
        other => (CodingRoleRunStatus::Failed, Some(other.to_string())),
    }
}
