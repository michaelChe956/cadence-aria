use super::*;

pub(crate) enum ProviderTestingExecutionOutcome {
    EarlyReport(Box<TestingReport>),
    Completed(ProviderTestingExecutionPhase),
}

pub(crate) struct ProviderTestingExecutionPhase {
    pub(crate) full_output: String,
    pub(crate) step_results: Vec<TestingStepResult>,
    pub(crate) unplanned_commands: Vec<TestCommand>,
    pub(crate) unplanned_evidence: Vec<TestingUnplannedEvidence>,
    pub(crate) context_warnings: Vec<String>,
    pub(crate) blocked_summary: Option<String>,
    pub(crate) blocked_reason_code: Option<String>,
    pub(crate) chat_entry_sequence: usize,
}

pub(crate) struct ProviderTestingExecutionInput<'a> {
    pub(crate) attempt: CodingExecutionAttempt,
    pub(crate) node: CodingTimelineNode,
    pub(crate) role_run: CodingRoleRun,
    pub(crate) provider: &'a dyn StreamingProviderAdapter,
    pub(crate) worktree_path: PathBuf,
    pub(crate) tester_provider: ProviderName,
    pub(crate) plan: TestPlan,
    pub(crate) evaluation_context_json: String,
    pub(crate) chat_entry_sequence: usize,
    pub(crate) options: &'a TesterAgentOptions,
    pub(crate) command_rx: &'a mut mpsc::Receiver<CodingRunnerCommand>,
}

impl CodingWorkspaceEngine {
    pub(crate) async fn run_provider_testing_execution_phase(
        &self,
        input: ProviderTestingExecutionInput<'_>,
    ) -> Result<ProviderTestingExecutionOutcome, CodingWorkspaceEngineError> {
        let ProviderTestingExecutionInput {
            attempt,
            node,
            role_run,
            provider,
            worktree_path,
            tester_provider,
            plan,
            evaluation_context_json,
            chat_entry_sequence,
            options,
            command_rx,
        } = input;
        let mut chat_entry_sequence = chat_entry_sequence;
        let prompt = build_tester_execute_plan_prompt(&attempt, &plan, &evaluation_context_json);
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::CodingExecutionEvent {
                event: provider_prompt_event(
                    &node.id,
                    &tester_provider,
                    prompt.clone(),
                    "execute_test_plan",
                ),
            })
            .await;
        self.record_role_run_event(
            &attempt,
            Some(&role_run),
            CodingRoleRunEventType::ProviderPrompt,
            json!({
                "provider": tester_provider.clone(),
                "role": format!("{:?}", CodingProviderRole::Tester),
                "output_schema": "coding_workspace_execute_test_plan_json",
                "prompt": prompt.clone()
            }),
        );
        let resume_provider_session_id = self.provider_resume_session_id_for_attempt(
            &attempt,
            &CodingProviderRole::Tester,
            &tester_provider,
        );
        let input = StreamingProviderInput {
            provider_type: provider_type_for_name(&tester_provider),
            role: AdapterRole::Reviewer,
            prompt,
            working_dir: worktree_path.clone(),
            workspace_session_id: Some(attempt.id.clone()),
            resume_provider_session_id,
            permission_mode: role_permission_mode_for_attempt(
                &self.store,
                &attempt,
                CodingProviderRole::Tester,
            )?,
            env_vars: BTreeMap::new(),
            timeout_secs: options.timeout.as_secs().max(1),
        };
        let cancel = CancellationToken::new();
        let start_result = tokio::select! {
            result = provider.start(input, cancel.clone()) => result,
            _ = tokio::time::sleep(options.timeout) => {
                cancel.cancel();
                self.record_role_run_event(
                    &attempt,
                    Some(&role_run),
                    CodingRoleRunEventType::Timeout,
                    json!({
                        "phase": "execute_test_plan_start",
                        "reason_code": "execute_test_plan_timeout"
                    }),
                );
                let report_id = next_sequential_id(
                    "testing_report",
                    self.store
                        .list_testing_reports(&attempt.project_id, &attempt.issue_id, &attempt.id)?
                        .len(),
                );
                let mut report = build_plan_based_testing_report(
                    &report_id,
                    &attempt.id,
                    &plan,
                    Vec::new(),
                    Vec::new(),
                    None,
                    None,
                );
                report.overall_status = TestingOverallStatus::Blocked;
                report
                    .context_warnings
                    .push("execute_test_plan_timeout".to_string());
                return self
                    .save_blocked_testing_report_and_gate(
                        &attempt,
                        &node,
                        report,
                        BlockedTestingGateContext {
                            reason_code: "execute_test_plan_timeout".to_string(),
                            description: "Tester provider timed out starting execute_test_plan"
                                .to_string(),
                            raw_provider_output_ref: None,
                            role_run: Some(&role_run),
                        },
                    )
                    .await
                    .map(|report| ProviderTestingExecutionOutcome::EarlyReport(Box::new(report)));
            }
        };
        let mut session = match start_result {
            Ok(session) => {
                self.record_role_run_event(
                    &attempt,
                    Some(&role_run),
                    CodingRoleRunEventType::ProviderStart,
                    json!({
                        "provider": tester_provider.clone(),
                        "role": format!("{:?}", CodingProviderRole::Tester),
                        "phase": "execute_test_plan"
                    }),
                );
                session
            }
            Err(error) => {
                self.record_role_run_event(
                    &attempt,
                    Some(&role_run),
                    CodingRoleRunEventType::ProviderFailed,
                    json!({
                        "phase": "execute_test_plan",
                        "message": error.details.clone()
                    }),
                );
                let report_id = next_sequential_id(
                    "testing_report",
                    self.store
                        .list_testing_reports(&attempt.project_id, &attempt.issue_id, &attempt.id)?
                        .len(),
                );
                let mut report = build_plan_based_testing_report(
                    &report_id,
                    &attempt.id,
                    &plan,
                    Vec::new(),
                    Vec::new(),
                    None,
                    None,
                );
                report.overall_status = TestingOverallStatus::Blocked;
                report
                    .context_warnings
                    .push(format!("provider_start_failed:{error}"));
                return self
                    .save_blocked_testing_report_and_gate(
                        &attempt,
                        &node,
                        report,
                        BlockedTestingGateContext {
                            reason_code: "provider_start_failed".to_string(),
                            description: "Tester provider failed during execute_test_plan"
                                .to_string(),
                            raw_provider_output_ref: None,
                            role_run: Some(&role_run),
                        },
                    )
                    .await
                    .map(|report| ProviderTestingExecutionOutcome::EarlyReport(Box::new(report)));
            }
        };
        let timeout = tokio::time::sleep(options.timeout);
        tokio::pin!(timeout);
        let mut full_output = String::new();
        let mut step_results = Vec::new();
        let mut unplanned_commands = Vec::new();
        let mut unplanned_evidence = Vec::new();
        let mut context_warnings = Vec::new();
        let mut consecutive_failures = 0usize;
        let mut blocked_summary = None;
        let mut blocked_reason_code = None;
        let mut commands_open = true;
        let mut tool_call_titles = BTreeMap::new();
        let mut tool_call_commands = BTreeMap::new();
        let mut open_choice_ids = Vec::<String>::new();

        loop {
            tokio::select! {
                _ = &mut timeout => {
                    cancel.cancel();
                    self.record_role_run_event(
                        &attempt,
                        Some(&role_run),
                        CodingRoleRunEventType::Timeout,
                        json!({
                            "phase": "execute_test_plan",
                            "reason_code": "provider_stream_timeout"
                        }),
                    );
                    blocked_summary = Some("Tester Agent Loop 超时".to_string());
                    break;
                }
                command = command_rx.recv(), if commands_open => {
                    let Some(command) = command else {
                        commands_open = false;
                        continue;
                    };
                    match command {
                        CodingRunnerCommand::AbortAttempt => {
                            let _ = session.commands.send(ProviderCommand::Abort).await;
                            cancel.cancel();
                            let _ = self
                                .event_tx
                                .send(CodingWsOutMessage::CodingExecutionEvent {
                                    event: ws_event_from_provider_status(
                                        &node.id,
                                        &tester_provider,
                                        ProviderStatus::Aborted,
                                    ),
                                })
                                .await;
                            self.record_role_run_event(
                                &attempt,
                                Some(&role_run),
                                CodingRoleRunEventType::Aborted,
                                json!({
                                    "reason": "abort_attempt",
                                    "phase": "execute_test_plan"
                                }),
                            );
                            return Err(CodingWorkspaceEngineError::Aborted);
                        }
                        CodingRunnerCommand::ChoiceResponse {
                            id,
                            selected_option_ids,
                            free_text,
                        } => {
                            if !open_choice_ids.iter().any(|choice_id| choice_id == &id) {
                                let _ = self
                                    .event_tx
                                    .send(CodingWsOutMessage::CodingProtocolError {
                                        code: "coding_choice_gate_not_found".to_string(),
                                        message: format!(
                                            "ChoiceResponse id={id} not found in open coding choice gates"
                                        ),
                                    })
                                    .await;
                                continue;
                            }
                            if session
                                .commands
                                .send(ProviderCommand::ChoiceResponse {
                                    id: id.clone(),
                                    selected_option_ids: selected_option_ids.clone(),
                                    free_text: free_text.clone(),
                                })
                                .await
                                .is_ok()
                            {
                                let ack_selected_option_ids = selected_option_ids.clone();
                                let ack_free_text = free_text.clone();
                                let _ = self.store.resolve_choice_gate(
                                    &attempt.project_id,
                                    &attempt.issue_id,
                                    &attempt.id,
                                    &id,
                                    selected_option_ids,
                                    free_text,
                                )?;
                                open_choice_ids.retain(|choice_id| choice_id != &id);
                                let current = self.store.get_attempt(
                                    &attempt.project_id,
                                    &attempt.issue_id,
                                    &attempt.id,
                                )?;
                                if current.status == CodingAttemptStatus::WaitingForHuman {
                                    self.store.update_attempt_status(
                                        &attempt.project_id,
                                        &attempt.issue_id,
                                        &attempt.id,
                                        CodingAttemptStatus::Running,
                                    )?;
                                }
                                let _ = self
                                    .event_tx
                                    .send(CodingWsOutMessage::CodingChoiceResponseAck {
                                        id,
                                        selected_option_ids: ack_selected_option_ids,
                                        free_text: ack_free_text,
                                    })
                                    .await;
                            } else {
                                commands_open = false;
                            }
                        }
                        command => {
                            if !forward_runner_command_to_provider(command, &session.commands).await {
                                commands_open = false;
                            }
                        }
                    }
                }
                event = session.events.recv() => {
                    let Some(event) = event else {
                        if !open_choice_ids.is_empty() {
                            return Err(self.unresolved_provider_choice_error(
                                &attempt,
                                Some(&role_run),
                                "execute_test_plan_stream_closed",
                                &open_choice_ids,
                            ));
                        }
                        blocked_summary = Some("Tester Provider stream ended before completion".to_string());
                        break;
                    };
                    match event {
                        ProviderEvent::TextDelta { content } => {
                            if !open_choice_ids.is_empty() {
                                return Err(self.unresolved_provider_choice_error(
                                    &attempt,
                                    Some(&role_run),
                                    "execute_test_plan_text_delta",
                                    &open_choice_ids,
                                ));
                            }
                            let content_for_event = content.clone();
                            full_output.push_str(&content);
                            let _ = self
                                .event_tx
                                .send(CodingWsOutMessage::CodingStreamChunk {
                                    content,
                                    node_id: Some(node.id.clone()),
                                })
                                .await;
                            self.record_role_run_event(
                                &attempt,
                                Some(&role_run),
                                CodingRoleRunEventType::TextDelta,
                                json!({
                                    "content": content_for_event,
                                    "phase": "execute_test_plan"
                                }),
                            );
                        }
                        ProviderEvent::ToolCall(call) => {
                            if !open_choice_ids.is_empty() {
                                return Err(self.unresolved_provider_choice_error(
                                    &attempt,
                                    Some(&role_run),
                                    "execute_test_plan_tool_call",
                                    &open_choice_ids,
                                ));
                            }
                            let call_for_event = call.clone();
                            tool_call_titles.insert(call.id.clone(), call.tool_name.clone());
                            if let Some(command) = extract_tool_command(&call.input) {
                                tool_call_commands.insert(call.id.clone(), command);
                            }
                            let _ = self
                                .event_tx
                                .send(CodingWsOutMessage::CodingExecutionEvent {
                                    event: ws_event_from_tool_call(&node.id, &tester_provider, call.clone()),
                                })
                                .await;
                            let entry = tester_chat_entry(
                                &attempt,
                                &node.id,
                                &mut chat_entry_sequence,
                                CodingEntryType::ToolCall {
                                    tool_name: call.tool_name.clone(),
                                    input: call.input.clone(),
                                },
                                None,
                                Some(serde_json::json!({
                                    "tool_use_id": call.id.clone(),
                                    "role_run_id": role_run.id.clone(),
                                    "run_no": role_run.run_no
                                })),
                            );
                            self.save_and_emit_chat_entry(entry).await;
                            self.record_role_run_event(
                                &attempt,
                                Some(&role_run),
                                CodingRoleRunEventType::ToolCall,
                                json!({
                                    "id": call_for_event.id,
                                    "tool_name": call_for_event.tool_name,
                                    "input": call_for_event.input,
                                    "phase": "execute_test_plan"
                                }),
                            );

                            if let Some(reason_code) =
                                high_risk_test_step_block_reason(&plan, &call)
                            {
                                cancel.cancel();
                                blocked_reason_code = Some(reason_code.to_string());
                                blocked_summary = Some(
                                    "High risk TestPlan step requires permission".to_string(),
                                );
                                break;
                            }

                            let artifact_output_root = self.store.attempt_test_output_root(
                                &attempt.project_id,
                                &attempt.issue_id,
                                &attempt.id,
                            );
                            let outcome =
                                execute_tester_tool_call(
                                    &call,
                                    worktree_path.clone(),
                                    artifact_output_root,
                                )
                                .await?;
                            let command_result = outcome.command.clone();
                            let result = outcome.result;
                            record_tester_step_result(
                                &plan,
                                &call,
                                command_result,
                                &result,
                                TesterStepResultOutputs {
                                    step_results: &mut step_results,
                                    unplanned_commands: &mut unplanned_commands,
                                    unplanned_evidence: &mut unplanned_evidence,
                                    context_warnings: &mut context_warnings,
                                },
                            );
                            let is_error = result.is_error;
                            let result_for_event = result.clone();
                            let _ = session
                                .commands
                                .send(ProviderCommand::ToolResult(result.clone()))
                                .await;
                            let _ = self
                                .event_tx
                                .send(CodingWsOutMessage::CodingExecutionEvent {
                                    event: ws_event_from_tool_result(
                                        &node.id,
                                        &tester_provider,
                                        &call.tool_name,
                                        extract_tool_command(&call.input),
                                        result.clone(),
                                    ),
                                })
                                .await;
                            self.emit_tester_tool_result_entry(
                                &attempt,
                                &node.id,
                                &mut chat_entry_sequence,
                                Some(&role_run),
                                result,
                            )
                            .await;
                            self.record_role_run_event(
                                &attempt,
                                Some(&role_run),
                                CodingRoleRunEventType::ToolResult,
                                json!({
                                    "tool_use_id": result_for_event.tool_use_id,
                                    "output": result_for_event.output,
                                    "is_error": result_for_event.is_error,
                                    "phase": "execute_test_plan"
                                }),
                            );

                            if is_error {
                                consecutive_failures += 1;
                            } else {
                                consecutive_failures = 0;
                            }
                            if consecutive_failures >= options.failure_limit {
                                cancel.cancel();
                                blocked_summary = Some(format!(
                                    "Tester Agent Loop 连续 {} 次 tool_use 失败",
                                    options.failure_limit
                                ));
                                break;
                            }
                        }
                        ProviderEvent::ToolResult(result) => {
                            if !open_choice_ids.is_empty() {
                                return Err(self.unresolved_provider_choice_error(
                                    &attempt,
                                    Some(&role_run),
                                    "execute_test_plan_tool_result",
                                    &open_choice_ids,
                                ));
                            }
                            let result_for_event = result.clone();
                            let title = tool_call_titles
                                .get(&result.tool_use_id)
                                .cloned()
                                .unwrap_or_else(|| "Tool result".to_string());
                            let command = tool_call_commands.get(&result.tool_use_id).cloned();
                            let _ = self
                                .event_tx
                                .send(CodingWsOutMessage::CodingExecutionEvent {
                                    event: ws_event_from_tool_result(
                                        &node.id,
                                        &tester_provider,
                                        &title,
                                        command,
                                        result.clone(),
                                    ),
                                })
                                .await;
                            self.emit_tester_tool_result_entry(
                                &attempt,
                                &node.id,
                                &mut chat_entry_sequence,
                                Some(&role_run),
                                result,
                            )
                            .await;
                            self.record_role_run_event(
                                &attempt,
                                Some(&role_run),
                                CodingRoleRunEventType::ToolResult,
                                json!({
                                    "tool_use_id": result_for_event.tool_use_id,
                                    "output": result_for_event.output,
                                    "is_error": result_for_event.is_error,
                                    "phase": "execute_test_plan"
                                }),
                            );
                        }
                        ProviderEvent::Execution(event) => {
                            if !open_choice_ids.is_empty() {
                                return Err(self.unresolved_provider_choice_error(
                                    &attempt,
                                    Some(&role_run),
                                    "execute_test_plan_execution",
                                    &open_choice_ids,
                                ));
                            }
                            let event_for_record = event.clone();
                            let _ = self
                                .event_tx
                                .send(CodingWsOutMessage::CodingExecutionEvent {
                                    event: ws_event_from_provider_execution(
                                        event,
                                        &node.id,
                                        &tester_provider,
                                    ),
                                })
                                .await;
                            self.record_role_run_event(
                                &attempt,
                                Some(&role_run),
                                CodingRoleRunEventType::ExecutionEvent,
                                json!({
                                    "event_id": event_for_record.event_id,
                                    "kind": format!("{:?}", event_for_record.kind),
                                    "status": format!("{:?}", event_for_record.status),
                                    "title": event_for_record.title,
                                    "detail": event_for_record.detail,
                                    "command": event_for_record.command,
                                    "cwd": event_for_record.cwd,
                                    "output": event_for_record.output,
                                    "exit_code": event_for_record.exit_code,
                                    "phase": "execute_test_plan"
                                }),
                            );
                        }
                        ProviderEvent::Completed {
                            full_output: completed_output,
                            provider_session_id,
                        } => {
                            if !open_choice_ids.is_empty() {
                                return Err(self.unresolved_provider_choice_error(
                                    &attempt,
                                    Some(&role_run),
                                    "execute_test_plan_completed",
                                    &open_choice_ids,
                                ));
                            }
                            let provider_session_id_for_event = provider_session_id.clone();
                            let output_bytes = completed_output.len();
                            self.record_attempt_provider_session(
                                &attempt,
                                &CodingProviderRole::Tester,
                                tester_provider.clone(),
                                provider_session_id,
                                &node.id,
                            )?;
                            if !completed_output.trim().is_empty() {
                                full_output = completed_output;
                            }
                            let _ = self
                                .event_tx
                                .send(CodingWsOutMessage::CodingMessageComplete {
                                    node_id: Some(node.id.clone()),
                                })
                                .await;
                            self.record_role_run_event(
                                &attempt,
                                Some(&role_run),
                                CodingRoleRunEventType::MessageComplete,
                                json!({
                                    "provider_session_id": provider_session_id_for_event,
                                    "output_bytes": output_bytes,
                                    "phase": "execute_test_plan"
                                }),
                            );
                            break;
                        }
                        ProviderEvent::Failed { message } => {
                            self.record_role_run_event(
                                &attempt,
                                Some(&role_run),
                                CodingRoleRunEventType::ProviderFailed,
                                json!({
                                    "phase": "execute_test_plan",
                                    "message": message.clone()
                                }),
                            );
                            blocked_summary = Some(message);
                            break;
                        }
                        ProviderEvent::ProtocolError {
                            code,
                            message,
                            context,
                        } => {
                            self.record_role_run_event(
                                &attempt,
                                Some(&role_run),
                                CodingRoleRunEventType::ProviderFailed,
                                json!({
                                    "phase": "execute_test_plan",
                                    "code": code,
                                    "message": message.clone(),
                                    "context": context
                                }),
                            );
                            blocked_summary = Some(message);
                            break;
                        }
                        ProviderEvent::PermissionTimeout { permission_id } => {
                            if !open_choice_ids.is_empty() {
                                return Err(self.unresolved_provider_choice_error(
                                    &attempt,
                                    Some(&role_run),
                                    "execute_test_plan_permission_timeout",
                                    &open_choice_ids,
                                ));
                            }
                            let message = format!("Permission request {permission_id} timed out");
                            self.record_role_run_event(
                                &attempt,
                                Some(&role_run),
                                CodingRoleRunEventType::Timeout,
                                json!({
                                    "phase": "execute_test_plan",
                                    "reason": "permission_timeout",
                                    "permission_id": permission_id,
                                    "message": message.clone()
                                }),
                            );
                            blocked_summary = Some(message);
                            break;
                        }
                        ProviderEvent::PermissionRequest(request) => {
                            if !open_choice_ids.is_empty() {
                                return Err(self.unresolved_provider_choice_error(
                                    &attempt,
                                    Some(&role_run),
                                    "execute_test_plan_permission_request",
                                    &open_choice_ids,
                                ));
                            }
                            let request_for_event = request.clone();
                            self.emit_permission_request(&node.id, &tester_provider, request).await;
                            self.record_role_run_event(
                                &attempt,
                                Some(&role_run),
                                CodingRoleRunEventType::PermissionRequest,
                                json!({
                                    "id": request_for_event.id,
                                    "tool_name": request_for_event.tool_name,
                                    "description": request_for_event.description,
                                    "risk_level": format!("{:?}", request_for_event.risk_level),
                                    "phase": "execute_test_plan"
                                }),
                            );
                        }
                        ProviderEvent::ChoiceRequest(request) => {
                            let request_for_event = request.clone();
                            self.emit_choice_request(
                                &attempt,
                                &node.id,
                                CodingExecutionStage::Testing,
                                CodingProviderRole::Tester,
                                &tester_provider,
                                request,
                            )
                            .await?;
                            open_choice_ids.push(request_for_event.id.clone());
                            self.record_role_run_event(
                                &attempt,
                                Some(&role_run),
                                CodingRoleRunEventType::ChoiceRequest,
                                json!({
                                    "id": request_for_event.id,
                                    "prompt": request_for_event.prompt,
                                    "allow_multiple": request_for_event.allow_multiple,
                                    "allow_free_text": request_for_event.allow_free_text,
                                    "source": request_for_event.source.as_str(),
                                    "phase": "execute_test_plan"
                                }),
                            );
                        }
                        ProviderEvent::StatusChanged(status) => {
                            let status_for_event = status.clone();
                            let _ = self
                                .event_tx
                                .send(CodingWsOutMessage::CodingExecutionEvent {
                                    event: ws_event_from_provider_status(
                                        &node.id,
                                        &tester_provider,
                                        status,
                                    ),
                                })
                                .await;
                            self.record_role_run_event(
                                &attempt,
                                Some(&role_run),
                                CodingRoleRunEventType::StatusChanged,
                                json!({
                                    "status": format!("{status_for_event:?}"),
                                    "phase": "execute_test_plan"
                                }),
                            );
                        }
                    }
                }
            }
        }

        Ok(ProviderTestingExecutionOutcome::Completed(
            ProviderTestingExecutionPhase {
                full_output,
                step_results,
                unplanned_commands,
                unplanned_evidence,
                context_warnings,
                blocked_summary,
                blocked_reason_code,
                chat_entry_sequence,
            },
        ))
    }
}
