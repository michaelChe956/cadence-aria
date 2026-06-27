use super::ClaudeCodeProvider;
use super::*;

pub(crate) async fn terminate_aborted_child(child: &mut AsyncGroupChild) {
    #[cfg(unix)]
    if let Some(pgid) = child.id() {
        unsafe {
            let _ = libc::killpg(pgid as i32, libc::SIGKILL);
        }
    }
    let _ = child.start_kill();
    let _ = child.inner().start_kill();
    if let Err(error) = child.wait().await {
        tracing::warn!(%error, "failed to wait for aborted Claude Code provider process");
    }
}

pub(crate) async fn emit_ask_user_question_protocol_error(
    event_tx: &mpsc::Sender<ProviderEvent>,
    source: &str,
    context: Value,
    details: &str,
) {
    let message = format!("AskUserQuestion {source} unresolved: {details}");
    // 直接使用 event_tx.send，因为失败原因可能是 cancel；send_provider_event 会在 cancel 时丢弃事件。
    let _ = event_tx
        .send(ProviderEvent::ProtocolError {
            code: "ask_user_question_unresolved".to_string(),
            message,
            context: Some(context),
        })
        .await;
}

pub(crate) async fn read_claude_stream(
    stdout: tokio::process::ChildStdout,
    stdin: Arc<Mutex<ChildStdin>>,
    bridge: ApprovalBridge,
    event_tx: mpsc::Sender<ProviderEvent>,
    cancel: CancellationToken,
) -> Result<ClaudeStreamOutcome, ProviderAdapterError> {
    let mut lines = BufReader::new(stdout).lines();
    let mut pending_tool_uses: HashMap<String, ToolUseBlock> = HashMap::new();
    let mut resolved_ask_user_questions: HashMap<String, ResolvedAskUserQuestion> = HashMap::new();
    let mut emitted_assistant_text = String::new();

    loop {
        let line = tokio::select! {
            _ = cancel.cancelled() => {
                let _ = event_tx
                    .send(ProviderEvent::StatusChanged(ProviderStatus::Aborted))
                    .await;
                return Ok(ClaudeStreamOutcome::Aborted);
            }
            line = lines.next_line() => line.map_err(|error| {
                ProviderAdapterError::execution_failed(None, String::new(), error.to_string(), 0)
            })?,
        };
        let Some(line) = line else {
            return Ok(ClaudeStreamOutcome::EofWithoutResult);
        };
        if line.trim().is_empty() {
            continue;
        }

        let value = serde_json::from_str::<Value>(&line).map_err(|error| {
            ProviderAdapterError::parse_error(
                format!("invalid Claude stream JSON: {error}"),
                line.clone(),
                String::new(),
            )
        })?;

        if let Some(content) = ClaudeCodeProvider::parse_stream_text_delta(&value) {
            emitted_assistant_text.push_str(&content);
            send_provider_event(&event_tx, ProviderEvent::TextDelta { content }, &cancel).await?;
            continue;
        }

        if let Some(assistant_text) = ClaudeCodeProvider::parse_assistant_text(&value)
            && let Some(content) =
                ClaudeCodeProvider::assistant_text_delta(&assistant_text, &emitted_assistant_text)
        {
            emitted_assistant_text.push_str(&content);
            send_provider_event(&event_tx, ProviderEvent::TextDelta { content }, &cancel).await?;
        }

        if let Some(request) = ClaudeCodeProvider::parse_control_request(&value) {
            if request.tool_name == "AskUserQuestion" {
                eprintln!(
                    "[aria-choice-diag] claude received control_request AskUserQuestion request_id={} tool_use_id={}",
                    request.request_id,
                    request.tool_use_id.as_deref().unwrap_or("<none>")
                );
                if let Some(resolved) = request
                    .tool_use_id
                    .as_deref()
                    .and_then(|tool_use_id| resolved_ask_user_questions.get(tool_use_id))
                {
                    eprintln!(
                        "[aria-choice-diag] claude reusing AskUserQuestion decision for control_request request_id={} tool_use_id={}",
                        request.request_id,
                        request.tool_use_id.as_deref().unwrap_or("<none>")
                    );
                    ClaudeCodeProvider::write_choice_control_response(
                        &stdin,
                        &request.request_id,
                        &request.input,
                        resolved.answers.clone(),
                    )
                    .await?;
                    continue;
                }
                let choice_request = ask_user_question::parse_ask_user_question_from_input(
                    &request.input,
                    &request.request_id,
                );
                let choice_decision =
                    match bridge.request_choice(choice_request, cancel.clone()).await {
                        Ok(decision) => decision,
                        Err(error) => {
                            emit_ask_user_question_protocol_error(
                                &event_tx,
                                "control_request",
                                json!({
                                    "request_id": request.request_id,
                                    "tool_use_id": request.tool_use_id,
                                }),
                                &error.details,
                            )
                            .await;
                            return Err(error);
                        }
                    };
                eprintln!(
                    "[aria-choice-diag] claude got choice decision for control_request request_id={} selected={:?} free_text_present={}",
                    request.request_id,
                    choice_decision.selected_option_ids,
                    choice_decision
                        .free_text
                        .as_ref()
                        .is_some_and(|text| !text.trim().is_empty())
                );
                let answers = ask_user_question::ask_user_question_answers_from_decision(
                    &request.input,
                    &choice_decision,
                );
                ClaudeCodeProvider::write_choice_control_response(
                    &stdin,
                    &request.request_id,
                    &request.input,
                    answers.clone(),
                )
                .await?;
                if let Some(tool_use_id) = request.tool_use_id {
                    resolved_ask_user_questions.insert(
                        tool_use_id,
                        ResolvedAskUserQuestion {
                            input: request.input,
                            answers,
                        },
                    );
                }
            } else {
                let decision = bridge
                    .request_tool(
                        &request.tool_name,
                        &request.description,
                        RiskLevel::High,
                        cancel.clone(),
                    )
                    .await?;
                ClaudeCodeProvider::write_control_response(
                    &stdin,
                    &request.request_id,
                    decision.approved,
                    decision.reason,
                )
                .await?;
            }
            continue;
        }

        if let Some(tool_uses) = ClaudeCodeProvider::parse_tool_use_from_assistant(&value) {
            for tool_use in tool_uses {
                if tool_use.name == "AskUserQuestion" {
                    eprintln!(
                        "[aria-choice-diag] claude received assistant tool_use AskUserQuestion tool_use_id={}",
                        tool_use.id
                    );
                    let resolved = match resolved_ask_user_questions.remove(&tool_use.id) {
                        Some(resolved) => resolved,
                        None => {
                            let choice_request =
                                ask_user_question::parse_ask_user_question_from_input(
                                    &tool_use.input,
                                    &tool_use.id,
                                );
                            let choice_decision =
                                match bridge.request_choice(choice_request, cancel.clone()).await {
                                    Ok(decision) => decision,
                                    Err(error) => {
                                        emit_ask_user_question_protocol_error(
                                            &event_tx,
                                            "tool_use",
                                            json!({ "tool_use_id": tool_use.id }),
                                            &error.details,
                                        )
                                        .await;
                                        return Err(error);
                                    }
                                };
                            eprintln!(
                                "[aria-choice-diag] claude got choice decision for assistant tool_use tool_use_id={} selected={:?} free_text_present={}",
                                tool_use.id,
                                choice_decision.selected_option_ids,
                                choice_decision
                                    .free_text
                                    .as_ref()
                                    .is_some_and(|text| !text.trim().is_empty())
                            );
                            ResolvedAskUserQuestion {
                                input: tool_use.input.clone(),
                                answers: ask_user_question::ask_user_question_answers_from_decision(
                                    &tool_use.input,
                                    &choice_decision,
                                ),
                            }
                        }
                    };
                    resolved_ask_user_questions.insert(tool_use.id.clone(), resolved.clone());
                    ClaudeCodeProvider::write_tool_result(
                        &stdin,
                        &tool_use.id,
                        &resolved.input,
                        &resolved.answers,
                    )
                    .await?;
                    continue;
                } else {
                    let description = tool::tool_use_description(&tool_use);
                    send_provider_event(
                        &event_tx,
                        ProviderEvent::Execution(ProviderExecutionEvent {
                            event_id: tool_use.id.clone(),
                            kind: ProviderExecutionEventKind::Command,
                            status: ProviderExecutionEventStatus::Started,
                            title: tool_use.name.clone(),
                            detail: Some(description),
                            command: tool::tool_use_command(&tool_use),
                            cwd: None,
                            output: None,
                            exit_code: None,
                        }),
                        &cancel,
                    )
                    .await?;
                    pending_tool_uses.insert(tool_use.id.clone(), tool_use);
                }
            }
            continue;
        }

        if let Some(results) = ClaudeCodeProvider::parse_tool_result(&value) {
            for result in results {
                if result.is_error && resolved_ask_user_questions.contains_key(&result.tool_use_id)
                {
                    emit_ask_user_question_protocol_error(
                        &event_tx,
                        "tool_result",
                        json!({
                            "tool_use_id": result.tool_use_id,
                            "output": result.output,
                        }),
                        &result.output,
                    )
                    .await;
                    return Err(ProviderAdapterError::execution_failed(
                        None,
                        result.output,
                        "AskUserQuestion tool_result reported error",
                        0,
                    ));
                }
                if let Some(tool_use) = pending_tool_uses.remove(&result.tool_use_id) {
                    let output_preview =
                        tool::output_preview(&result.output, TOOL_RESULT_PREVIEW_MAX_BYTES);
                    let command = tool::tool_use_command(&tool_use);
                    send_provider_event(
                        &event_tx,
                        ProviderEvent::Execution(ProviderExecutionEvent {
                            event_id: tool_use.id,
                            kind: ProviderExecutionEventKind::Command,
                            status: ProviderExecutionEventStatus::Completed,
                            title: tool_use.name,
                            detail: None,
                            command,
                            cwd: None,
                            output: Some(output_preview),
                            exit_code: Some(0),
                        }),
                        &cancel,
                    )
                    .await?;
                }
            }
            continue;
        }

        if value.get("type").and_then(Value::as_str) == Some("result") {
            let is_error = value
                .get("is_error")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if is_error {
                send_provider_event(
                    &event_tx,
                    ProviderEvent::Failed {
                        message: value
                            .get("result")
                            .and_then(Value::as_str)
                            .unwrap_or("Claude Code provider failed")
                            .to_string(),
                    },
                    &cancel,
                )
                .await?;
                return Ok(ClaudeStreamOutcome::TerminalEventEmitted);
            }

            let result_output = value
                .get("result")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let full_output =
                if result_output.trim().is_empty() && !emitted_assistant_text.trim().is_empty() {
                    emitted_assistant_text.clone()
                } else {
                    result_output
                };
            let provider_session_id = value
                .get("session_id")
                .and_then(Value::as_str)
                .map(ToString::to_string);
            send_provider_event(
                &event_tx,
                ProviderEvent::Execution(ProviderExecutionEvent {
                    event_id: "turn".to_string(),
                    kind: ProviderExecutionEventKind::Turn,
                    status: ProviderExecutionEventStatus::Completed,
                    title: "Turn completed".to_string(),
                    detail: None,
                    command: None,
                    cwd: None,
                    output: None,
                    exit_code: None,
                }),
                &cancel,
            )
            .await?;
            send_provider_event(
                &event_tx,
                ProviderEvent::StatusChanged(ProviderStatus::Completed),
                &cancel,
            )
            .await?;
            send_provider_event(
                &event_tx,
                ProviderEvent::Completed {
                    full_output,
                    provider_session_id,
                },
                &cancel,
            )
            .await?;
            return Ok(ClaudeStreamOutcome::TerminalEventEmitted);
        }
    }
}
pub(crate) async fn send_provider_event(
    event_tx: &mpsc::Sender<ProviderEvent>,
    event: ProviderEvent,
    cancel: &CancellationToken,
) -> Result<(), ProviderAdapterError> {
    tokio::select! {
        _ = cancel.cancelled() => Err(ProviderAdapterError::execution_failed(
            None,
            String::new(),
            "Claude Code provider cancelled",
            0,
        )),
        result = event_tx.send(event) => result.map_err(|_| {
            ProviderAdapterError::execution_failed(
                None,
                String::new(),
                "provider event receiver closed",
                0,
            )
        }),
    }
}
