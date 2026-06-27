#[async_trait::async_trait]
impl StreamingProviderAdapter for HangingStreamingProvider {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let (event_tx, event_rx) = mpsc::channel(8);
        let (command_tx, mut command_rx) = mpsc::channel::<ProviderCommand>(8);
        tokio::spawn(async move {
            let _ = event_tx
                .send(ProviderEvent::TextDelta {
                    content: "# Draft".to_string(),
                })
                .await;
            loop {
                tokio::select! {
                    _ = cancel.cancelled() => return,
                    command = command_rx.recv() => {
                        match command {
                            Some(ProviderCommand::Abort) | None => return,
                            Some(ProviderCommand::PermissionResponse { .. })
                            | Some(ProviderCommand::ChoiceResponse { .. })
                            | Some(ProviderCommand::ToolResult(_)) => {}
                        }
                    }
                }
            }
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
    ) -> Result<
        mpsc::Receiver<cadence_aria::cross_cutting::streaming_provider::StreamChunk>,
        ProviderAdapterError,
    > {
        Err(ProviderAdapterError::execution_failed(
            None,
            String::new(),
            "run_streaming is not used by workspace websocket",
            0,
        ))
    }
}

struct ChoiceThenHangingStreamingProvider;

#[async_trait::async_trait]
impl StreamingProviderAdapter for ChoiceThenHangingStreamingProvider {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let (event_tx, event_rx) = mpsc::channel(8);
        let (command_tx, mut command_rx) = mpsc::channel::<ProviderCommand>(8);
        tokio::spawn(async move {
            let _ = event_tx
                .send(ProviderEvent::ChoiceRequest(ChoiceRequestData {
                    id: "choice_hanging_001".to_string(),
                    prompt: "继续方式？".to_string(),
                    options: vec![ChoiceOptionData {
                        id: "opt_0".to_string(),
                        label: "继续 author".to_string(),
                        description: None,
                    }],
                    allow_multiple: false,
                    allow_free_text: false,
                    source: ChoiceRequestSource::ProviderChoice,
                }))
                .await;
            loop {
                tokio::select! {
                    _ = cancel.cancelled() => return,
                    command = command_rx.recv() => {
                        match command {
                            Some(ProviderCommand::Abort) | None => return,
                            Some(ProviderCommand::ChoiceResponse { .. }) => {
                                let _ = event_tx
                                    .send(ProviderEvent::StatusChanged(ProviderStatus::Running))
                                    .await;
                            }
                            Some(ProviderCommand::PermissionResponse { .. })
                            | Some(ProviderCommand::ToolResult(_)) => {}
                        }
                    }
                }
            }
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
            "run_streaming is not used by workspace websocket",
            0,
        ))
    }
}

struct ChoiceThenCompletingStreamingProvider;

#[async_trait::async_trait]
impl StreamingProviderAdapter for ChoiceThenCompletingStreamingProvider {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let (event_tx, event_rx) = mpsc::channel(8);
        let (command_tx, mut command_rx) = mpsc::channel::<ProviderCommand>(8);
        tokio::spawn(async move {
            let _ = event_tx
                .send(ProviderEvent::ChoiceRequest(ChoiceRequestData {
                    id: "choice_completing_001".to_string(),
                    prompt: "继续方式？".to_string(),
                    options: vec![ChoiceOptionData {
                        id: "opt_0".to_string(),
                        label: "继续 author".to_string(),
                        description: None,
                    }],
                    allow_multiple: false,
                    allow_free_text: false,
                    source: ChoiceRequestSource::ProviderChoice,
                }))
                .await;
            loop {
                tokio::select! {
                    _ = cancel.cancelled() => return,
                    command = command_rx.recv() => {
                        match command {
                            Some(ProviderCommand::ChoiceResponse { .. }) => {
                                let _ = event_tx
                                    .send(ProviderEvent::Completed {
                                        full_output: VALID_STORY_SPEC.to_string(),
                                        provider_session_id: Some(
                                            "choice-completing-session".to_string(),
                                        ),
                                    })
                                    .await;
                                return;
                            }
                            Some(ProviderCommand::Abort) | None => return,
                            Some(ProviderCommand::PermissionResponse { .. })
                            | Some(ProviderCommand::ToolResult(_)) => {}
                        }
                    }
                }
            }
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
            "run_streaming is not used by workspace websocket",
            0,
        ))
    }
}

#[derive(Default)]
struct SequencedChoiceCompletingProvider {
    next_choice: Mutex<u32>,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for SequencedChoiceCompletingProvider {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let id = {
            let mut next = self.next_choice.lock().unwrap();
            *next += 1;
            format!("choice_sequence_{next:03}")
        };
        let (event_tx, event_rx) = mpsc::channel(8);
        let (command_tx, mut command_rx) = mpsc::channel::<ProviderCommand>(8);
        tokio::spawn(async move {
            let _ = event_tx
                .send(ProviderEvent::ChoiceRequest(ChoiceRequestData {
                    id,
                    prompt: "继续方式？".to_string(),
                    options: vec![ChoiceOptionData {
                        id: "opt_0".to_string(),
                        label: "继续 author".to_string(),
                        description: None,
                    }],
                    allow_multiple: false,
                    allow_free_text: false,
                    source: ChoiceRequestSource::ProviderChoice,
                }))
                .await;
            loop {
                tokio::select! {
                    _ = cancel.cancelled() => return,
                    command = command_rx.recv() => {
                        match command {
                            Some(ProviderCommand::ChoiceResponse { .. }) => {
                                let _ = event_tx
                                    .send(ProviderEvent::Completed {
                                        full_output: VALID_STORY_SPEC.to_string(),
                                        provider_session_id: Some(
                                            "choice-sequence-session".to_string(),
                                        ),
                                    })
                                    .await;
                                return;
                            }
                            Some(ProviderCommand::Abort) | None => return,
                            Some(ProviderCommand::PermissionResponse { .. })
                            | Some(ProviderCommand::ToolResult(_)) => {}
                        }
                    }
                }
            }
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
            "run_streaming is not used by workspace websocket",
            0,
        ))
    }
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for ScriptedStreamingProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        self.prompts.lock().unwrap().push(input.prompt);
        let output = self
            .outputs
            .lock()
            .unwrap()
            .pop_front()
            .expect("scripted provider output");
        let (event_tx, event_rx) = mpsc::channel(8);
        let (command_tx, _command_rx) = mpsc::channel::<ProviderCommand>(8);
        tokio::spawn(async move {
            let _ = event_tx
                .send(ProviderEvent::TextDelta {
                    content: output.clone(),
                })
                .await;
            let _ = event_tx
                .send(ProviderEvent::Completed {
                    full_output: output,
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
    ) -> Result<
        mpsc::Receiver<cadence_aria::cross_cutting::streaming_provider::StreamChunk>,
        ProviderAdapterError,
    > {
        Err(ProviderAdapterError::execution_failed(
            None,
            String::new(),
            "run_streaming is not used by workspace websocket",
            0,
        ))
    }
}

async fn create_workspace_session_fixture(root: &TempDir) -> TempDir {
    create_workspace_session_fixture_with_author(root, "fake").await
}

async fn create_workspace_session_fixture_with_author(
    root: &TempDir,
    author_provider: &str,
) -> TempDir {
    create_workspace_session_fixture_with_providers(root, author_provider, "fake", 1).await
}

async fn create_workspace_session_fixture_with_providers(
    root: &TempDir,
    author_provider: &str,
    reviewer_provider: &str,
    review_rounds: u32,
) -> TempDir {
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));

    request_json(
        app.clone(),
        Method::POST,
        "/api/projects",
        json!({"name":"Lifecycle","description":null}),
    )
    .await;
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/repositories",
        json!({"name":"Repo","path":repo.path()}),
    )
    .await;
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues",
        json!({"title":"登录会话过期","description":"描述","repository_id":"repository_0001"}),
    )
    .await;
    let (status, story_response) = request_json(
        app,
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/story-specs:generate",
        json!({
            "title":"登录会话过期提示",
            "author_provider":author_provider,
            "reviewer_provider":reviewer_provider,
            "review_rounds":review_rounds,
            "superpowers_enabled":true,
            "openspec_enabled":true
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        story_response["workspace_session"]["workspace_session_id"],
        "workspace_session_0001"
    );
    repo
}

fn clear_workspace_session_messages(root: &std::path::Path) {
    replace_workspace_session_messages(root, json!([]));
}

fn replace_workspace_session_messages(root: &std::path::Path, messages: Value) {
    let session_path = root.join(
        ".aria/projects/project_0001/issues/issue_0001/workspace-sessions/workspace_session_0001.json",
    );
    let mut session: Value =
        serde_json::from_str(&fs::read_to_string(&session_path).expect("workspace session json"))
            .expect("workspace session value");
    session["messages"] = messages;
    fs::write(
        &session_path,
        serde_json::to_string_pretty(&session).expect("workspace session json"),
    )
    .expect("write workspace session");
}

async fn request_json(
    app: axum::Router,
    method: Method,
    uri: &str,
    body: Value,
) -> (StatusCode, Value) {
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("request");
    let response = app.oneshot(request).await.expect("response");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, value)
}

async fn lifecycle_json(root: &std::path::Path) -> Value {
    let app = build_web_router(WebAppState::new(
        root.to_path_buf(),
        WebRuntime::new_fake(root.to_path_buf()),
    ));
    let (status, lifecycle) = request_json(
        app,
        Method::GET,
        "/api/issues/issue_0001/lifecycle?project_id=project_0001",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    lifecycle
}

async fn send_json(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    message: &WsInMessage,
) {
    ws.send(Message::Text(
        serde_json::to_string(message).unwrap().into(),
    ))
    .await
    .expect("send ws message");
}

async fn accept_author_output(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) {
    assert_eq!(
        recv_until_stage(ws, "author_confirm").await,
        "author_confirm"
    );
    send_json(
        ws,
        &WsInMessage::AuthorDecision {
            decision: AuthorDecision::Accept,
        },
    )
    .await;
}

async fn recv_json(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) -> WsOutMessage {
    let message = timeout(Duration::from_secs(3), ws.next())
        .await
        .expect("ws message timeout")
        .expect("ws message")
        .expect("valid ws message");
    match message {
        Message::Text(text) => serde_json::from_str(&text).expect("ws json"),
        other => panic!("expected text ws message, got {other:?}"),
    }
}

async fn recv_until_message_complete(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) -> String {
    for _ in 0..600 {
        match recv_json(ws).await {
            WsOutMessage::MessageComplete { checkpoint_id, .. } => return checkpoint_id,
            WsOutMessage::Error { message } => panic!("ws error: {message}"),
            _ => {}
        }
    }
    panic!("message_complete not received");
}

async fn recv_until_stage(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    expected: &str,
) -> String {
    for _ in 0..40 {
        match recv_json(ws).await {
            WsOutMessage::StageChange { stage } if stage == expected => return stage,
            WsOutMessage::Error { message } => panic!("ws error: {message}"),
            _ => {}
        }
    }
    panic!("stage_change {expected} not received");
}

async fn recv_until_session_state(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) -> WsOutMessage {
    for _ in 0..40 {
        match recv_json(ws).await {
            state @ WsOutMessage::SessionState { .. } => return state,
            WsOutMessage::Error { message } => panic!("ws error: {message}"),
            _ => {}
        }
    }
    panic!("session_state not received");
}

async fn recv_until_stream_chunk(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) -> String {
    for _ in 0..40 {
        match recv_json(ws).await {
            WsOutMessage::StreamChunk { content, .. } => return content,
            WsOutMessage::Error { message } => panic!("ws error: {message}"),
            _ => {}
        }
    }
    panic!("stream_chunk not received");
}

#[derive(Debug)]
struct PermissionRequestSeen {
    id: String,
    tool_name: String,
    description: String,
}

async fn recv_until_permission_request(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) -> PermissionRequestSeen {
    for _ in 0..40 {
        match recv_json(ws).await {
            WsOutMessage::PermissionRequest {
                id,
                tool_name,
                description,
                ..
            } => {
                return PermissionRequestSeen {
                    id,
                    tool_name,
                    description,
                };
            }
            WsOutMessage::Error { message } => panic!("ws error: {message}"),
            _ => {}
        }
    }
    panic!("permission_request not received");
}

#[derive(Debug)]
struct ChoiceRequestSeen {
    id: String,
    prompt: String,
    options: Vec<cadence_aria::web::workspace_ws_types::ChoiceOption>,
    source: Option<String>,
}

async fn recv_until_choice_request(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) -> ChoiceRequestSeen {
    for _ in 0..600 {
        match recv_json(ws).await {
            WsOutMessage::ChoiceRequest {
                id,
                prompt,
                options,
                source,
                ..
            } => {
                return ChoiceRequestSeen {
                    id,
                    prompt,
                    options,
                    source: Some(source),
                };
            }
            WsOutMessage::MessageComplete { .. } => {
                panic!("author question was completed as artifact before choice_request")
            }
            WsOutMessage::StageChange { stage } if stage == "cross_review" => {
                panic!("reviewer started before author choice_request")
            }
            WsOutMessage::Error { message } => panic!("ws error: {message}"),
            _ => {}
        }
    }
    panic!("choice_request not received");
}

async fn recv_until_protocol_error(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) -> WsOutMessage {
    for _ in 0..40 {
        match recv_json(ws).await {
            event @ WsOutMessage::ProtocolError { .. } => return event,
            WsOutMessage::Error { message } => panic!("ws error: {message}"),
            _ => {}
        }
    }
    panic!("protocol_error not received");
}

fn long_message(token: &str) -> String {
    (0..80)
        .map(|idx| format!("{token}_{idx}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn executable_fixture(relative_path: &str) -> PathBuf {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(relative_path);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let metadata = std::fs::metadata(&path)
            .unwrap_or_else(|error| panic!("fixture metadata {}: {error}", path.display()));
        let mut permissions = metadata.permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&path, permissions)
            .unwrap_or_else(|error| panic!("chmod fixture {}: {error}", path.display()));
    }
    path
}

fn git_repo() -> TempDir {
    let dir = tempdir().expect("repo");
    let status = Command::new("git")
        .args(["init", "--initial-branch", "main"])
        .current_dir(dir.path())
        .status()
        .expect("git init");
    assert!(status.success());
    dir
}
