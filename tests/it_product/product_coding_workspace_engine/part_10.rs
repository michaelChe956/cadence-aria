#[async_trait::async_trait]
impl StreamingProviderAdapter for EventEmittingCodingProvider {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let (event_tx, event_rx) = mpsc::channel(16);
        let (command_tx, _command_rx) = mpsc::channel(8);
        event_tx
            .try_send(ProviderEvent::TextDelta {
                content: "working".to_string(),
            })
            .expect("send text");
        event_tx
            .try_send(ProviderEvent::Execution(ProviderExecutionEvent {
                event_id: "command_0001".to_string(),
                kind: ProviderExecutionEventKind::Command,
                status: ProviderExecutionEventStatus::Completed,
                title: "Run tests".to_string(),
                detail: Some("Executed verification command".to_string()),
                command: Some("uv run pytest".to_string()),
                cwd: None,
                output: Some("1 passed".to_string()),
                exit_code: Some(0),
            }))
            .expect("send execution event");
        event_tx
            .try_send(ProviderEvent::ToolCall(ProviderToolCall {
                id: "tool_0001".to_string(),
                tool_name: "run_command".to_string(),
                input: serde_json::json!({ "command": "uv run pytest" }),
            }))
            .expect("send tool call");
        event_tx
            .try_send(ProviderEvent::ToolResult(ProviderToolResult {
                tool_use_id: "tool_0001".to_string(),
                output: "1 passed".to_string(),
                is_error: false,
            }))
            .expect("send tool result");
        event_tx
            .try_send(ProviderEvent::Completed {
                full_output: "done".to_string(),
                provider_session_id: None,
            })
            .expect("send completed");

        Ok(ProviderSession {
            events: event_rx,
            commands: command_tx,
        })
    }
}

struct ControlEventCodingProvider;

#[async_trait::async_trait]
impl StreamingProviderAdapter for ControlEventCodingProvider {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let (event_tx, event_rx) = mpsc::channel(16);
        let (command_tx, _command_rx) = mpsc::channel(8);
        event_tx
            .try_send(ProviderEvent::StatusChanged(ProviderStatus::Running))
            .expect("send status");
        event_tx
            .try_send(ProviderEvent::PermissionRequest(PermissionRequestData {
                id: "permission_0001".to_string(),
                tool_name: "shell".to_string(),
                description: "Run uv test command".to_string(),
                risk_level: RiskLevel::High,
            }))
            .expect("send permission");
        event_tx
            .try_send(ProviderEvent::ChoiceRequest(ChoiceRequestData {
                id: "choice_0001".to_string(),
                prompt: "Select implementation strategy".to_string(),
                options: vec![ChoiceOptionData {
                    id: "dp".to_string(),
                    label: "Dynamic programming".to_string(),
                    description: Some("Iterative solution".to_string()),
                }],
                allow_multiple: false,
                allow_free_text: true,
                questions: vec![],
                source: ChoiceRequestSource::ProviderChoice,
            }))
            .expect("send choice");
        event_tx
            .try_send(ProviderEvent::Completed {
                full_output: "done".to_string(),
                provider_session_id: None,
            })
            .expect("send completed");

        Ok(ProviderSession {
            events: event_rx,
            commands: command_tx,
        })
    }
}

struct PermissionAwaitingProvider;

#[async_trait::async_trait]
impl StreamingProviderAdapter for PermissionAwaitingProvider {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let (event_tx, event_rx) = mpsc::channel(16);
        let (command_tx, mut command_rx) = mpsc::channel(8);
        tokio::spawn(async move {
            let _ = event_tx
                .send(ProviderEvent::PermissionRequest(PermissionRequestData {
                    id: "permission_0001".to_string(),
                    tool_name: "shell".to_string(),
                    description: "Run uv test command".to_string(),
                    risk_level: RiskLevel::High,
                }))
                .await;
            while let Some(command) = command_rx.recv().await {
                match command {
                    cadence_aria::cross_cutting::streaming_provider::ProviderCommand::PermissionResponse {
                        id,
                        approved,
                        ..
                    } if id == "permission_0001" && approved => {
                        let _ = event_tx
                            .send(ProviderEvent::Completed {
                                full_output: "approved".to_string(),
                                provider_session_id: None,
                            })
                            .await;
                        return;
                    }
                    cadence_aria::cross_cutting::streaming_provider::ProviderCommand::Abort => {
                        return;
                    }
                    _ => {}
                }
            }
        });
        Ok(ProviderSession {
            events: event_rx,
            commands: command_tx,
        })
    }
}

struct ChoiceAwaitingProvider;

#[async_trait::async_trait]
impl StreamingProviderAdapter for ChoiceAwaitingProvider {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let (event_tx, event_rx) = mpsc::channel(16);
        let (command_tx, mut command_rx) = mpsc::channel(8);
        tokio::spawn(async move {
            let _ = event_tx
                .send(ProviderEvent::ChoiceRequest(ChoiceRequestData {
                    id: "choice_0001".to_string(),
                    prompt: "Select implementation strategy".to_string(),
                    options: vec![ChoiceOptionData {
                        id: "backend_first".to_string(),
                        label: "先做后端".to_string(),
                        description: Some("TASK-001 到 TASK-009".to_string()),
                    }],
                    allow_multiple: false,
                    allow_free_text: true,
                    questions: vec![],
                    source: ChoiceRequestSource::RequestUserInput,
                }))
                .await;
            while let Some(command) = command_rx.recv().await {
                match command {
                    cadence_aria::cross_cutting::streaming_provider::ProviderCommand::ChoiceResponse {
                        id,
                        selected_option_ids,
                        ..
                    } if id == "choice_0001"
                        && selected_option_ids == vec!["backend_first".to_string()] =>
                    {
                        let _ = event_tx
                            .send(ProviderEvent::Completed {
                                full_output: "selected backend_first".to_string(),
                                provider_session_id: None,
                            })
                            .await;
                        return;
                    }
                    cadence_aria::cross_cutting::streaming_provider::ProviderCommand::Abort => {
                        return;
                    }
                    _ => {}
                }
            }
        });
        Ok(ProviderSession {
            events: event_rx,
            commands: command_tx,
        })
    }
}

struct ChoiceThenPermissionProvider;

#[async_trait::async_trait]
impl StreamingProviderAdapter for ChoiceThenPermissionProvider {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let (event_tx, event_rx) = mpsc::channel(16);
        let (command_tx, _command_rx) = mpsc::channel(8);
        event_tx
            .try_send(ProviderEvent::ChoiceRequest(ChoiceRequestData {
                id: "choice_0001".to_string(),
                prompt: "Select implementation strategy".to_string(),
                options: vec![ChoiceOptionData {
                    id: "backend_first".to_string(),
                    label: "先做后端".to_string(),
                    description: Some("TASK-001 到 TASK-009".to_string()),
                }],
                allow_multiple: false,
                allow_free_text: true,
                questions: vec![],
                source: ChoiceRequestSource::RequestUserInput,
            }))
            .expect("send choice");
        event_tx
            .try_send(ProviderEvent::PermissionRequest(PermissionRequestData {
                id: "permission_0001".to_string(),
                tool_name: "shell".to_string(),
                description: "Run tests".to_string(),
                risk_level: RiskLevel::High,
            }))
            .expect("send permission");
        event_tx
            .try_send(ProviderEvent::Completed {
                full_output: "done".to_string(),
                provider_session_id: None,
            })
            .expect("send completed");

        Ok(ProviderSession {
            events: event_rx,
            commands: command_tx,
        })
    }
}

struct EventThenCompletedProvider {
    output: String,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for EventThenCompletedProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let (event_tx, event_rx) = mpsc::channel(16);
        let (command_tx, _command_rx) = mpsc::channel(8);
        event_tx
            .try_send(ProviderEvent::Execution(ProviderExecutionEvent {
                event_id: "provider_command_0001".to_string(),
                kind: ProviderExecutionEventKind::Command,
                status: ProviderExecutionEventStatus::Completed,
                title: "Provider command".to_string(),
                detail: Some("Provider emitted a command event".to_string()),
                command: Some("git diff --stat".to_string()),
                cwd: Some(input.working_dir.display().to_string()),
                output: Some("changed files".to_string()),
                exit_code: Some(0),
            }))
            .expect("send execution event");
        event_tx
            .try_send(ProviderEvent::Completed {
                full_output: self.output.clone(),
                provider_session_id: None,
            })
            .expect("send completed");
        Ok(ProviderSession {
            events: event_rx,
            commands: command_tx,
        })
    }
}

struct ReviewControlEventProvider;

#[async_trait::async_trait]
impl StreamingProviderAdapter for ReviewControlEventProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let (event_tx, event_rx) = mpsc::channel(16);
        let (command_tx, _command_rx) = mpsc::channel(8);
        event_tx
            .try_send(ProviderEvent::TextDelta {
                content: "reviewing".to_string(),
            })
            .expect("send text");
        event_tx
            .try_send(ProviderEvent::Execution(ProviderExecutionEvent {
                event_id: "review_command_0001".to_string(),
                kind: ProviderExecutionEventKind::Command,
                status: ProviderExecutionEventStatus::Completed,
                title: "Review command".to_string(),
                detail: Some("Ran review helper".to_string()),
                command: Some("cargo test --locked".to_string()),
                cwd: Some(input.working_dir.display().to_string()),
                output: Some("review ok".to_string()),
                exit_code: Some(0),
            }))
            .expect("send execution");
        event_tx
            .try_send(ProviderEvent::ToolCall(ProviderToolCall {
                id: "review_tool_0001".to_string(),
                tool_name: "run_command".to_string(),
                input: serde_json::json!({ "command": "cargo test --locked" }),
            }))
            .expect("send tool call");
        event_tx
            .try_send(ProviderEvent::ToolResult(ProviderToolResult {
                tool_use_id: "review_tool_0001".to_string(),
                output: "tool ok".to_string(),
                is_error: false,
            }))
            .expect("send tool result");
        event_tx
            .try_send(ProviderEvent::StatusChanged(ProviderStatus::Running))
            .expect("send status");
        event_tx
            .try_send(ProviderEvent::PermissionRequest(PermissionRequestData {
                id: "permission_review_0001".to_string(),
                tool_name: "shell".to_string(),
                description: "Inspect diff".to_string(),
                risk_level: RiskLevel::High,
            }))
            .expect("send permission");
        event_tx
            .try_send(ProviderEvent::Completed {
                full_output: r#"{"verdict":"approve","summary":"review ok","findings":[]}"#
                    .to_string(),
                provider_session_id: Some("review-session-0001".to_string()),
            })
            .expect("send completed");
        Ok(ProviderSession {
            events: event_rx,
            commands: command_tx,
        })
    }
}

struct ReviewPermissionTimeoutProvider;

#[async_trait::async_trait]
impl StreamingProviderAdapter for ReviewPermissionTimeoutProvider {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let (event_tx, event_rx) = mpsc::channel(8);
        let (command_tx, _command_rx) = mpsc::channel(8);
        event_tx
            .try_send(ProviderEvent::PermissionTimeout {
                permission_id: "permission_review_timeout".to_string(),
            })
            .expect("send permission timeout");
        Ok(ProviderSession {
            events: event_rx,
            commands: command_tx,
        })
    }
}

struct StartFailingProvider;

#[async_trait::async_trait]
impl StreamingProviderAdapter for StartFailingProvider {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        Err(ProviderAdapterError::command_missing(
            "provider failed to start",
        ))
    }
}

struct ReviewStreamingProvider;

#[async_trait::async_trait]
impl StreamingProviderAdapter for ReviewStreamingProvider {
    async fn run_streaming(
        &self,
        _input: &AdapterInput,
        _cancel: CancellationToken,
    ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
        let (tx, rx) = mpsc::channel(8);
        tx.try_send(StreamChunk::Text("reviewing diff".to_string()))
            .expect("send review chunk");
        tx.try_send(StreamChunk::Done {
            full_output: r#"{"verdict":"approve","summary":"review ok","findings":[]}"#.to_string(),
        })
        .expect("send review done");
        Ok(rx)
    }
}

fn drain_events(rx: &mut mpsc::Receiver<CodingWsOutMessage>) -> Vec<CodingWsOutMessage> {
    let mut events = Vec::new();
    while let Ok(event) = rx.try_recv() {
        events.push(event);
    }
    events
}

fn assert_provider_command_event(events: &[CodingWsOutMessage]) {
    assert!(
        events.iter().any(|event| {
            matches!(
                event,
                CodingWsOutMessage::CodingExecutionEvent { event }
                    if event.title == "Provider command"
                        && event.kind == WsExecutionEventKind::Command
                        && event.status == WsExecutionEventStatus::Completed
                        && event.command.as_deref() == Some("git diff --stat")
                        && event.output.as_deref() == Some("changed files")
            )
        }),
        "expected provider command execution event, got {events:?}"
    );
}

struct AnalystStreamingProvider {
    prompt: Arc<Mutex<Option<String>>>,
    output: String,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for AnalystStreamingProvider {
    async fn run_streaming(
        &self,
        input: &AdapterInput,
        _cancel: CancellationToken,
    ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
        *self.prompt.lock().expect("prompt lock") = Some(input.prompt.clone());
        let (tx, rx) = mpsc::channel(8);
        tx.try_send(StreamChunk::Done {
            full_output: self.output.clone(),
        })
        .expect("send rework done");
        Ok(rx)
    }
}

struct InternalReviewStreamingProvider;

#[async_trait::async_trait]
impl StreamingProviderAdapter for InternalReviewStreamingProvider {
    async fn run_streaming(
        &self,
        _input: &AdapterInput,
        _cancel: CancellationToken,
    ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
        let (tx, rx) = mpsc::channel(8);
        tx.try_send(StreamChunk::Text("reviewing pushed branch".to_string()))
            .expect("send internal review chunk");
        tx.try_send(StreamChunk::Done {
            full_output: r#"{"verdict":"approve","summary":"internal review ok","findings":[],"impact_scope":["src"],"pr_description":"实现 work item","commit_message_suggestion":"feat: implement work item"}"#.to_string(),
        })
        .expect("send internal review done");
        Ok(rx)
    }
}

#[tokio::test]
async fn analyst_human_gate_offers_retry_analyst_action() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    init_repo(&worktree);
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let mut input = create_input();
    input.worktree_path = Some(worktree);
    let attempt = store.create_attempt(input).expect("create attempt");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    let (tx, _rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider = InputCapturingProvider {
        input: Arc::new(Mutex::new(None)),
        output: "Analyst prose without JSON".to_string(),
    };

    engine
        .execute_rework(&attempt, "testing blocked evidence", &provider)
        .await
        .expect("execute analyst");

    let gates = store
        .list_open_blocked_gates("project_0001", "issue_0001", &attempt.id)
        .expect("gates");
    assert_eq!(gates.len(), 1);
    assert_eq!(gates[0].role, Some(CodingProviderRole::Analyst));
    assert!(gates[0].available_actions.iter().any(|action| {
        action.action_id == "retry_analyst"
            && action.action_type == CodingGateActionType::RetryAnalyst
    }));
}

#[tokio::test]
async fn provide_context_keeps_analyst_human_gate_open_for_retry() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    init_repo(&worktree);
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let mut input = create_input();
    input.worktree_path = Some(worktree);
    let attempt = store.create_attempt(input).expect("create attempt");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Blocked,
        )
        .expect("blocked");
    store
        .update_attempt_stage(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingExecutionStage::Rework,
        )
        .expect("rework");
    store
        .create_blocked_gate(CreateBlockedGateInput {
            attempt_id: attempt.id.clone(),
            stage: CodingExecutionStage::Rework,
            node_id: Some("coding_node_0002".to_string()),
            role: Some(CodingProviderRole::Analyst),
            title: "Analyst human gate".to_string(),
            description: "需要重跑 Analyst".to_string(),
            reason_code: Some("analyst_human_gate".to_string()),
            evidence_refs: Vec::new(),
            raw_provider_output_ref: Some(
                "provider-raw/rework/analyst_decision_0001.txt".to_string(),
            ),
            available_actions: vec![
                CodingGateAction {
                    action_id: "retry_analyst".to_string(),
                    label: "重试 Analyst".to_string(),
                    action_type: CodingGateActionType::RetryAnalyst,
                },
                CodingGateAction {
                    action_id: "provide_context".to_string(),
                    label: "补充上下文".to_string(),
                    action_type: CodingGateActionType::ProvideContext,
                },
                CodingGateAction {
                    action_id: "abort".to_string(),
                    label: "终止".to_string(),
                    action_type: CodingGateActionType::Abort,
                },
            ],
        })
        .expect("create gate");
    let (tx, _rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

    let updated = engine
        .handle_blocked_gate_response(
            "project_0001",
            "issue_0001",
            &attempt.id,
            "coding_blocked_gate_0001",
            "provide_context",
            Some("请按系统支持的 Analyst JSON schema 重试".to_string()),
        )
        .await
        .expect("provide context");

    assert_eq!(updated.status, CodingAttemptStatus::WaitingForHuman);
    assert_eq!(updated.stage, CodingExecutionStage::Rework);
    let notes = store
        .list_context_notes("project_0001", "issue_0001", &attempt.id)
        .expect("context notes");
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].consumed_by_rework_round, None);
    let gates = store
        .list_open_blocked_gates("project_0001", "issue_0001", &attempt.id)
        .expect("open gates");
    assert_eq!(gates.len(), 1);
    assert_eq!(gates[0].gate_id, "coding_blocked_gate_0001");
    assert!(gates[0].available_actions.iter().any(|action| {
        action.action_id == "retry_analyst"
            && action.action_type == CodingGateActionType::RetryAnalyst
    }));
}
