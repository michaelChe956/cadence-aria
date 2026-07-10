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
            .try_send(ProviderEvent::Completed(cadence_aria::cross_cutting::streaming_provider::ProviderCompletion::plain("done".to_string(), None)))
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
            .try_send(ProviderEvent::Completed(cadence_aria::cross_cutting::streaming_provider::ProviderCompletion::plain("done".to_string(), None)))
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
                            .send(ProviderEvent::Completed(cadence_aria::cross_cutting::streaming_provider::ProviderCompletion::plain("approved".to_string(), None)))
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
                            .send(ProviderEvent::Completed(cadence_aria::cross_cutting::streaming_provider::ProviderCompletion::plain("selected backend_first".to_string(), None)))
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
            .try_send(ProviderEvent::Completed(cadence_aria::cross_cutting::streaming_provider::ProviderCompletion::plain("done".to_string(), None)))
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
            .try_send(ProviderEvent::Completed(cadence_aria::cross_cutting::streaming_provider::ProviderCompletion::plain(self.output.clone(), None)))
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
            .try_send(ProviderEvent::Completed(cadence_aria::cross_cutting::streaming_provider::ProviderCompletion::plain(r#"{"verdict":"approve","summary":"review ok","findings":[]}"#
                .to_string(), Some("review-session-0001".to_string()))))
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
