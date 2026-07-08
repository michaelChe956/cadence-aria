#[async_trait::async_trait]
impl StreamingProviderAdapter for DesignArtifactRetryProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        self.inputs.lock().unwrap().push(input);
        let mut calls = self.calls.lock().unwrap();
        *calls += 1;
        let call_no = *calls;
        drop(calls);

        let (event_tx, event_rx) = mpsc::channel(8);
        let (command_tx, _command_rx) = mpsc::channel(8);
        tokio::spawn(async move {
            if call_no == 1 {
                let output = "我先核对 reviewer 指出的几处代码锚点。\n";
                let _ = event_tx
                    .send(ProviderEvent::TextDelta {
                        content: output.to_string(),
                    })
                    .await;
                let _ = event_tx
                    .send(ProviderEvent::Completed {
                        full_output: output.to_string(),
                        provider_session_id: Some("design-retry-session-1".to_string()),
                    })
                    .await;
                return;
            }

            let output = format!(
                "```artifact\n{}```",
                complete_design_artifact(
                    "返修时直接输出完整设计产物。",
                    "ProviderDependencyDialog::submit。",
                )
                .replacen("# Design Spec", "# Retried Design Spec", 1)
            );
            let _ = event_tx
                .send(ProviderEvent::TextDelta {
                    content: output.clone(),
                })
                .await;
            let _ = event_tx
                .send(ProviderEvent::Completed {
                    full_output: output,
                    provider_session_id: Some("design-retry-session-2".to_string()),
                })
                .await;
        });
        Ok(ProviderSession {
            events: event_rx,
            commands: command_tx,
        })
    }

    async fn run_streaming(
        &self,
        _input: &AdapterInput,
        _cancel: CancellationToken,
    ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
        Err(ProviderAdapterError::execution_failed(
            None,
            String::new(),
            "run_streaming is not used by WorkspaceEngine",
            0,
        ))
    }
}

struct ExecutionEventStreamingProvider;

#[async_trait::async_trait]
impl StreamingProviderAdapter for ExecutionEventStreamingProvider {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let (event_tx, event_rx) = mpsc::channel(8);
        let (command_tx, _command_rx) = mpsc::channel(8);
        tokio::spawn(async move {
            let _ = event_tx
                .send(ProviderEvent::Execution(ProviderExecutionEvent {
                    event_id: "command_cmd_001".to_string(),
                    kind: ProviderExecutionEventKind::Command,
                    status: ProviderExecutionEventStatus::Completed,
                    title: "Command completed".to_string(),
                    detail: Some("exit code 0".to_string()),
                    command: Some("pwd".to_string()),
                    cwd: Some("/tmp/repo".to_string()),
                    output: Some("/tmp/repo\n".to_string()),
                    exit_code: Some(0),
                }))
                .await;
            let _ = event_tx
                .send(ProviderEvent::Completed {
                    full_output: "# Draft".to_string(),
                    provider_session_id: None,
                })
                .await;
        });
        Ok(ProviderSession {
            events: event_rx,
            commands: command_tx,
        })
    }

    async fn run_streaming(
        &self,
        _input: &AdapterInput,
        _cancel: CancellationToken,
    ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
        Err(ProviderAdapterError::execution_failed(
            None,
            String::new(),
            "run_streaming is not used by WorkspaceEngine",
            0,
        ))
    }
}

fn tool_event_provider_session(full_output: &str) -> ProviderSession {
    let (event_tx, event_rx) = mpsc::channel(8);
    let (command_tx, _command_rx) = mpsc::channel(8);
    event_tx
        .try_send(ProviderEvent::ToolCall(
            crate::cross_cutting::streaming_provider::ProviderToolCall {
                id: "tool_0001".to_string(),
                tool_name: "edit_file".to_string(),
                input: serde_json::json!({
                    "command": "apply_patch",
                    "path": "stairs.py"
                }),
            },
        ))
        .expect("send tool call");
    event_tx
        .try_send(ProviderEvent::ToolResult(
            crate::cross_cutting::streaming_provider::ProviderToolResult {
                tool_use_id: "tool_0001".to_string(),
                output: "updated stairs.py".to_string(),
                is_error: false,
            },
        ))
        .expect("send tool result");
    event_tx
        .try_send(ProviderEvent::Completed {
            full_output: full_output.to_string(),
            provider_session_id: None,
        })
        .expect("send completed");
    ProviderSession {
        events: event_rx,
        commands: command_tx,
    }
}

fn text_choice_provider_session(full_output: &str) -> ProviderSession {
    let (event_tx, event_rx) = mpsc::channel(8);
    let (command_tx, _command_rx) = mpsc::channel(8);
    event_tx
        .try_send(ProviderEvent::Completed {
            full_output: full_output.to_string(),
            provider_session_id: Some("provider-author-session-1".to_string()),
        })
        .expect("send completed");
    ProviderSession {
        events: event_rx,
        commands: command_tx,
    }
}

fn drain_engine_events(rx: &mut mpsc::Receiver<EngineEvent>) -> Vec<EngineEvent> {
    let mut events = Vec::new();
    while let Ok(event) = rx.try_recv() {
        events.push(event);
    }
    events
}

fn assert_tool_call_and_result_events(
    events: &[EngineEvent],
    expected_node_id: Option<&str>,
    expected_agent: ProviderName,
) {
    assert!(
        events.iter().any(|event| {
            matches!(
                event,
                EngineEvent::ExecutionEvent { event, node_id, agent }
                    if event.event_id == "tool_0001"
                        && event.kind == ProviderExecutionEventKind::Command
                        && event.status == ProviderExecutionEventStatus::Started
                        && event.title == "edit_file"
                        && event
                            .detail
                            .as_deref()
                            .is_some_and(|detail| detail.contains("stairs.py"))
                        && node_id.as_deref() == expected_node_id
                        && agent.as_ref() == Some(&expected_agent)
            )
        }),
        "expected visible tool call event, got {} engine events",
        events.len()
    );
    assert!(
        events.iter().any(|event| {
            matches!(
                event,
                EngineEvent::ExecutionEvent { event, node_id, agent }
                    if event.event_id == "tool_0001"
                        && event.kind == ProviderExecutionEventKind::Command
                        && event.status == ProviderExecutionEventStatus::Completed
                        && event.title == "edit_file"
                        && event.command.as_deref() == Some("apply_patch")
                        && event.output.as_deref() == Some("updated stairs.py")
                        && event.exit_code == Some(0)
                        && node_id.as_deref() == expected_node_id
                        && agent.as_ref() == Some(&expected_agent)
            )
        }),
        "expected visible tool result event, got {} engine events",
        events.len()
    );
}

#[tokio::test]
async fn handle_user_message_forwards_provider_execution_events() {
    let (_tmp, store) = setup();
    let (tx, mut rx) = mpsc::channel(64);
    let session = make_session("sess_007_exec");
    let mut engine = WorkspaceEngine::new(store, tx, session);

    engine
        .handle_user_message(
            "start".to_string(),
            Arc::new(ExecutionEventStreamingProvider),
            empty_provider_commands(),
        )
        .await;

    let mut saw_execution_event = false;
    while let Ok(event) = rx.try_recv() {
        if let EngineEvent::ExecutionEvent { event, .. } = event {
            if event.event_id != "command_cmd_001" {
                continue;
            }
            assert_eq!(event.event_id, "command_cmd_001");
            assert_eq!(event.kind, ProviderExecutionEventKind::Command);
            assert_eq!(event.status, ProviderExecutionEventStatus::Completed);
            assert_eq!(event.command.as_deref(), Some("pwd"));
            assert_eq!(event.output.as_deref(), Some("/tmp/repo\n"));
            saw_execution_event = true;
        }
    }

    assert!(
        saw_execution_event,
        "provider execution events should be forwarded to websocket layer"
    );
}

#[tokio::test]
async fn handle_user_message_emits_provider_prompt_event() {
    let (_tmp, store) = setup();
    let (tx, mut rx) = mpsc::channel(64);
    let session = make_session("sess_007_prompt");
    let mut engine = WorkspaceEngine::new(store, tx, session);

    engine
        .handle_user_message(
            "start".to_string(),
            Arc::new(ExecutionEventStreamingProvider),
            empty_provider_commands(),
        )
        .await;

    let events = drain_engine_events(&mut rx);
    assert!(
        events.iter().any(|event| {
            matches!(
                event,
                EngineEvent::ExecutionEvent { event, node_id, agent }
                    if event.title == "Provider Prompt"
                        && event.kind == ProviderExecutionEventKind::Output
                        && event.output.as_deref().is_some_and(|output| output.contains("[user]: start"))
                        && node_id.as_deref().is_some_and(|id| id.starts_with("timeline_node_"))
                && agent.as_ref() == Some(&ProviderName::ClaudeCode)
            )
        }),
        "expected provider prompt event"
    );
}

#[tokio::test]
async fn provider_session_forwards_tool_call_and_result_events() {
    let (tmp, checkpoint_store) = setup();
    let lifecycle_store = LifecycleStore::new(ProductAppPaths::new(tmp.path().join(".aria")));
    let (tx, mut rx) = mpsc::channel(64);
    let session_record = lifecycle_store
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: "story_spec_0001".to_string(),
            workspace_type: WorkspaceType::Story,
            author_provider: ProviderName::ClaudeCode,
            reviewer_provider: ProviderName::Codex,
            review_rounds: 2,
            superpowers_enabled: true,
            openspec_enabled: true,
        })
        .unwrap();
    let session = WorkspaceSession::from_record(session_record);
    let mut engine =
        WorkspaceEngine::new_persistent(checkpoint_store, lifecycle_store.clone(), tx, session);
    let node_id = create_author_run_node(&mut engine).await;

    engine
        .drive_provider_session(ProviderSessionDriveInput {
            session: Ok(tool_event_provider_session("# Draft")),
            command_rx: empty_provider_commands(),
            node_id: Some(node_id.clone()),
            agent: Some(ProviderName::ClaudeCode),
            role: ProviderConversationRole::Author,
            artifact_retry: None,
            revision_resume_fallback: None,
        })
        .await;

    let events = drain_engine_events(&mut rx);
    assert_tool_call_and_result_events(&events, Some(node_id.as_str()), ProviderName::ClaudeCode);

    let detail = lifecycle_store
        .load_node_detail(&engine.session().session_id, &node_id)
        .unwrap();
    let tool_events = detail
        .execution_events
        .iter()
        .filter(|event| event["event_id"] == "tool_0001")
        .collect::<Vec<_>>();
    assert_eq!(
        tool_events.len(),
        1,
        "same provider execution event id should be persisted once, got {detail:?}"
    );
    assert!(
        tool_events[0]["status"] == "completed" && tool_events[0]["output"] == "updated stairs.py",
        "tool result should be persisted to node detail, got {detail:?}"
    );
}

#[tokio::test]
async fn reviewer_provider_session_forwards_tool_call_and_result_events() {
    let (_tmp, store) = setup();
    let (tx, mut rx) = mpsc::channel(64);
    let session = make_session("sess_007_review_tools");
    let mut engine = WorkspaceEngine::new(store, tx, session);
    let node_id = create_reviewer_run_node(&mut engine).await;

    engine
        .drive_reviewer_provider_session(
            Ok(tool_event_provider_session(
                r#"{"verdict":"pass","summary":"审核通过"}"#,
            )),
            empty_provider_commands(),
            ProviderName::Codex,
        )
        .await;

    let events = drain_engine_events(&mut rx);
    assert_tool_call_and_result_events(&events, Some(node_id.as_str()), ProviderName::Codex);
}

#[tokio::test]
async fn handle_user_message_from_human_confirm_reenters_running_stage() {
    let (_tmp, store) = setup();
    let (tx, mut rx) = mpsc::channel(64);
    let mut session = make_session("sess_007");
    session.stage = WorkspaceStage::HumanConfirm;
    let mut engine = WorkspaceEngine::new(store, tx, session);
    engine
        .handle_user_message(
            "revise".to_string(),
            Arc::new(FakeStreamingProvider),
            empty_provider_commands(),
        )
        .await;

    engine
        .handle_author_decision(AuthorDecision::Accept)
        .await
        .unwrap();

    let mut saw_running = false;
    while let Ok(event) = rx.try_recv() {
        if matches!(event, EngineEvent::StageChange { stage } if stage == "running") {
            saw_running = true;
        }
    }
    assert!(
        saw_running,
        "manual intervention should restart the run stage"
    );
    assert_eq!(engine.session().stage, WorkspaceStage::CrossReview);
}

struct ErrorStreamingProvider;

#[async_trait::async_trait]
impl StreamingProviderAdapter for ErrorStreamingProvider {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let (event_tx, event_rx) = mpsc::channel(8);
        let (command_tx, _command_rx) = mpsc::channel(8);
        tokio::spawn(async move {
            let _ = event_tx
                .send(ProviderEvent::Failed {
                    message: "provider unavailable".to_string(),
                })
                .await;
        });
        Ok(ProviderSession {
            events: event_rx,
            commands: command_tx,
        })
    }

    async fn run_streaming(
        &self,
        _input: &AdapterInput,
        _cancel: CancellationToken,
    ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
        Err(ProviderAdapterError::execution_failed(
            None,
            String::new(),
            "run_streaming is not used by WorkspaceEngine",
            0,
        ))
    }
}

struct EmptyCompletedStreamingProvider;

#[async_trait::async_trait]
impl StreamingProviderAdapter for EmptyCompletedStreamingProvider {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let (event_tx, event_rx) = mpsc::channel(8);
        let (command_tx, _command_rx) = mpsc::channel(8);
        tokio::spawn(async move {
            let _ = event_tx
                .send(ProviderEvent::Completed {
                    full_output: String::new(),
                    provider_session_id: Some("empty-session".to_string()),
                })
                .await;
        });
        Ok(ProviderSession {
            events: event_rx,
            commands: command_tx,
        })
    }

    async fn run_streaming(
        &self,
        _input: &AdapterInput,
        _cancel: CancellationToken,
    ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
        Err(ProviderAdapterError::execution_failed(
            None,
            String::new(),
            "run_streaming is not used by WorkspaceEngine",
            0,
        ))
    }
}

struct InvalidArtifactStreamingProvider;

#[async_trait::async_trait]
impl StreamingProviderAdapter for InvalidArtifactStreamingProvider {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let (event_tx, event_rx) = mpsc::channel(8);
        let (command_tx, _command_rx) = mpsc::channel(8);
        tokio::spawn(async move {
            let _ = event_tx
                .send(ProviderEvent::Completed {
                    full_output: "我还需要继续分析，目前没有生成 Story Spec。".to_string(),
                    provider_session_id: Some("invalid-artifact-session".to_string()),
                })
                .await;
        });
        Ok(ProviderSession {
            events: event_rx,
            commands: command_tx,
        })
    }

    async fn run_streaming(
        &self,
        _input: &AdapterInput,
        _cancel: CancellationToken,
    ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
        Err(ProviderAdapterError::execution_failed(
            None,
            String::new(),
            "run_streaming is not used by WorkspaceEngine",
            0,
        ))
    }
}

#[tokio::test]
async fn handle_user_message_rejects_non_artifact_author_output_without_review() {
    let (_tmp, store) = setup();
    let (tx, mut rx) = mpsc::channel(64);
    let session = make_session("sess_invalid_artifact");
    let mut engine = WorkspaceEngine::new(store, tx, session);

    engine
        .handle_user_message(
            "start".to_string(),
            Arc::new(InvalidArtifactStreamingProvider),
            empty_provider_commands(),
        )
        .await;

    assert_eq!(engine.session().stage, WorkspaceStage::PrepareContext);
    assert_eq!(engine.session().artifact, None);

    let events = drain_engine_events(&mut rx);
    assert!(
        events.iter().any(|event| matches!(
            event,
            EngineEvent::Error { message }
                if message.contains("未返回有效的 Story Spec artifact")
        )),
        "invalid author output should emit an explicit artifact error"
    );
    assert!(
        !events.iter().any(|event| matches!(
            event,
            EngineEvent::StageChange { stage } if stage == "cross_review"
        )),
        "invalid author output must not start reviewer"
    );
}

#[tokio::test]
async fn handle_user_message_empty_provider_output_marks_author_node_failed() {
    let (_tmp, store) = setup();
    let (tx, mut rx) = mpsc::channel(64);
    let session = make_session("sess_empty_output");
    let mut engine = WorkspaceEngine::new(store, tx, session);

    engine
        .handle_user_message(
            "start".to_string(),
            Arc::new(EmptyCompletedStreamingProvider),
            empty_provider_commands(),
        )
        .await;

    let author_node = engine
        .timeline_nodes
        .iter()
        .find(|node| node.node_type == TimelineNodeType::AuthorRun)
        .expect("author node");
    assert_eq!(author_node.status, TimelineNodeStatus::Failed);
    assert_eq!(
        author_node.summary.as_deref(),
        Some("Provider 未返回助手内容")
    );
    assert_eq!(engine.session().stage, WorkspaceStage::PrepareContext);
    assert_eq!(engine.session().messages.len(), 1);

    let mut saw_error = false;
    while let Ok(event) = rx.try_recv() {
        if let EngineEvent::Error { message } = event {
            saw_error = message == "Provider completed without assistant output";
        }
    }
    assert!(saw_error);
}

#[tokio::test]
async fn reviewer_empty_provider_output_marks_review_node_failed_without_human_confirm() {
    let (_tmp, store) = setup();
    let (tx, mut rx) = mpsc::channel(64);
    let session = make_session("sess_empty_review_output");
    let mut engine = WorkspaceEngine::new(store, tx, session);
    let node_id = create_reviewer_run_node(&mut engine).await;

    engine
        .drive_reviewer_provider_session(
            Ok(text_choice_provider_session("")),
            empty_provider_commands(),
            ProviderName::Codex,
        )
        .await;

    let review_node = engine
        .timeline_nodes
        .iter()
        .find(|node| node.node_id == node_id)
        .expect("review node");
    assert_eq!(review_node.status, TimelineNodeStatus::Failed);
    assert_eq!(
        review_node.summary.as_deref(),
        Some("Provider 未返回助手内容")
    );
    assert_eq!(engine.session().stage, WorkspaceStage::PrepareContext);
    assert!(
        !engine
            .timeline_nodes
            .iter()
            .any(|node| node.node_type == TimelineNodeType::HumanConfirm),
        "empty reviewer output must not create a human confirm node"
    );

    let events = drain_engine_events(&mut rx);
    assert!(
        events.iter().any(|event| matches!(
            event,
            EngineEvent::Error { message }
                if message == "Provider completed without assistant output"
        )),
        "empty reviewer output should emit an explicit provider error"
    );
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, EngineEvent::ReviewComplete { .. })),
        "empty reviewer output must not be converted into a review verdict"
    );
}

#[tokio::test]
async fn finish_active_run_with_failed_node_marks_outline_node_failed() {
    let (_tmp, lifecycle_store, mut engine) = persistent_test_engine();
    let node_id = engine.begin_work_item_plan_outline_run().await;
    engine.mark_active_run_started("outline-run");

    engine
        .finish_active_run_with_failed_node("Outline structured output parse failed")
        .await;

    let node = engine
        .timeline_nodes
        .iter()
        .find(|node| node.node_id == node_id)
        .expect("outline node");
    assert_eq!(node.status, TimelineNodeStatus::Failed);
    assert_eq!(
        node.summary.as_deref(),
        Some("Outline structured output parse failed")
    );
    assert_eq!(engine.active_run_id(), None);
    assert_eq!(engine.current_stage(), WorkspaceStage::PrepareContext);

    let detail = lifecycle_store
        .load_node_detail(&engine.session().session_id, &node_id)
        .expect("node detail");
    assert_eq!(detail.status, TimelineNodeStatus::Failed);
}
