#[tokio::test]
async fn workspace_ws_abort_after_choice_response_returns_prepare_context() {
    let root = tempdir().expect("root");
    create_workspace_session_fixture(&root).await;
    let mut registry = ProviderRegistry::new();
    registry.register(
        ProviderName::Fake,
        Arc::new(ChoiceThenHangingStreamingProvider),
    );
    let app = build_web_router(WebAppState::with_provider_registry(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
        registry,
    ));
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    let url = format!("ws://{addr}/api/workspace-sessions/workspace_session_0001/ws");
    let (mut ws, _) = connect_async(url).await.expect("connect ws");
    let _initial = recv_json(&mut ws).await;

    send_json(
        &mut ws,
        &WsInMessage::UserMessage {
            content: "run choice and hang provider".to_string(),
        },
    )
    .await;

    let choice = recv_until_choice_request(&mut ws).await;
    send_json(
        &mut ws,
        &WsInMessage::ChoiceResponse {
            id: choice.id,
            selected_option_ids: vec!["opt_0".to_string()],
            free_text: None,
            answers: vec![],
        },
    )
    .await;
    send_json(&mut ws, &WsInMessage::Abort).await;

    let mut saw_aborted_status = false;
    for _ in 0..80 {
        match recv_json(&mut ws).await {
            WsOutMessage::ProviderStatus {
                status: WsProviderStatus::Aborted,
            } => saw_aborted_status = true,
            WsOutMessage::StageChange { stage } if stage == "prepare_context" => {
                assert!(saw_aborted_status);
                drop(ws);
                server.abort();
                return;
            }
            WsOutMessage::MessageComplete { .. } => {
                panic!("aborted choice run should not complete")
            }
            WsOutMessage::Error { message } => panic!("ws error: {message}"),
            _ => {}
        }
    }
    panic!("abort after choice response did not return workspace to prepare_context");
}

#[tokio::test]
async fn workspace_ws_test_permission_fixture_emits_permission_request_for_fake_provider() {
    let root = tempdir().expect("root");
    let _repo = create_workspace_session_fixture(&root).await;
    let state = WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    );
    state
        .test_controls
        .enable_permission_fixture("workspace_session_0001".to_string())
        .await;
    let app = build_web_router(state);
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    let url = format!("ws://{addr}/api/workspace-sessions/workspace_session_0001/ws");
    let (mut ws, _) = connect_async(url).await.expect("connect ws");
    let _initial = recv_json(&mut ws).await;

    send_json(
        &mut ws,
        &WsInMessage::UserMessage {
            content: "run permission fixture".to_string(),
        },
    )
    .await;

    let permission = recv_until_permission_request(&mut ws).await;
    assert_eq!(permission.tool_name, "Bash");
    assert_eq!(permission.description, "E2E permission fixture request");

    drop(ws);
    server.abort();
}

#[tokio::test]
async fn workspace_ws_human_confirm_v2_completes_workspace() {
    let root = tempdir().expect("root");
    let _repo = create_workspace_session_fixture(&root).await;
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    let url = format!("ws://{addr}/api/workspace-sessions/workspace_session_0001/ws");
    let (mut ws, _) = connect_async(url).await.expect("connect ws");
    let _initial = recv_json(&mut ws).await;

    send_json(
        &mut ws,
        &WsInMessage::UserMessage {
            content: "confirm with v2 message".to_string(),
        },
    )
    .await;
    accept_author_output(&mut ws).await;
    assert_eq!(
        recv_until_stage(&mut ws, "human_confirm").await,
        "human_confirm"
    );

    send_json(
        &mut ws,
        &WsInMessage::HumanConfirm {
            decision: cadence_aria::web::workspace_ws_types::HumanConfirmDecision::Confirm,
            payload: None,
        },
    )
    .await;

    assert_eq!(recv_until_stage(&mut ws, "completed").await, "completed");

    drop(ws);
    server.abort();
}

#[tokio::test]
async fn workspace_ws_unmatched_permission_response_returns_protocol_error() {
    let root = tempdir().expect("root");
    let _repo = create_workspace_session_fixture_with_author(&root, "claude_code").await;
    let mut registry = ProviderRegistry::new();
    registry.register(ProviderName::Fake, Arc::new(FakeStreamingProvider));
    registry.register(
        ProviderName::ClaudeCode,
        Arc::new(ClaudeCodeProvider::new(executable_fixture(
            "tests/fixtures/provider/claude_stream_json_fixture.sh",
        ))),
    );

    let app = build_web_router(WebAppState::with_provider_registry(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
        registry,
    ));
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    let url = format!("ws://{addr}/api/workspace-sessions/workspace_session_0001/ws");
    let (mut ws, _) = connect_async(url).await.expect("connect ws");
    let _initial = recv_json(&mut ws).await;

    send_json(
        &mut ws,
        &WsInMessage::UserMessage {
            content: "run supervised provider".to_string(),
        },
    )
    .await;

    let permission = recv_until_permission_request(&mut ws).await;
    send_json(
        &mut ws,
        &WsInMessage::PermissionResponse {
            id: "permission_not_pending".to_string(),
            approved: true,
            reason: Some("wrong request".to_string()),
        },
    )
    .await;

    match recv_until_protocol_error(&mut ws).await {
        WsOutMessage::ProtocolError { code, context, .. } => {
            assert_eq!(code, "PERMISSION_ID_UNMATCHED");
            assert_eq!(
                context
                    .as_ref()
                    .and_then(|value| value.get("permission_id"))
                    .and_then(|value| value.as_str()),
                Some("permission_not_pending")
            );
        }
        other => panic!("expected protocol_error, got {other:?}"),
    }

    send_json(
        &mut ws,
        &WsInMessage::PermissionResponse {
            id: permission.id,
            approved: true,
            reason: None,
        },
    )
    .await;

    let checkpoint = recv_until_message_complete(&mut ws).await;
    assert!(checkpoint.starts_with("cp_"));

    drop(ws);
    server.abort();
}

#[tokio::test]
async fn workspace_ws_codex_current_protocol_completes_from_repository_path() {
    let root = tempdir().expect("root");
    let repo = create_workspace_session_fixture_with_author(&root, "codex").await;
    let mut registry = ProviderRegistry::new();
    registry.register(ProviderName::Fake, Arc::new(FakeStreamingProvider));
    registry.register(
        ProviderName::Codex,
        Arc::new(CodexProvider::new(executable_fixture(
            "tests/fixtures/provider/codex_app_server_current_fixture.sh",
        ))),
    );

    let app = build_web_router(WebAppState::with_provider_registry(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
        registry,
    ));
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    let url = format!("ws://{addr}/api/workspace-sessions/workspace_session_0001/ws");
    let (mut ws, _) = connect_async(url).await.expect("connect ws");
    let initial = recv_json(&mut ws).await;
    match initial {
        WsOutMessage::SessionState { messages, .. } => {
            assert_eq!(messages.len(), 1);
            assert_eq!(messages[0].role, "system");
            assert!(messages[0].content.contains("Workspace 生成任务已准备"));
            assert!(messages[0].content.contains("OpenSpec"));
            assert!(messages[0].content.contains("using-superpowers"));
            assert!(messages[0].content.contains("Repository 路径"));
            assert!(
                messages[0]
                    .content
                    .contains(&repo.path().display().to_string())
            );
        }
        other => panic!("expected session_state, got {other:?}"),
    }

    send_json(
        &mut ws,
        &WsInMessage::UserMessage {
            content: "run codex current protocol".to_string(),
        },
    )
    .await;

    let expected_repo_path = repo
        .path()
        .canonicalize()
        .expect("repo canonical")
        .to_string_lossy()
        .to_string();
    let mut checkpoint = None;
    let mut saw_command_started = false;
    let mut saw_command_completed = false;
    for _ in 0..600 {
        match recv_json(&mut ws).await {
            WsOutMessage::ExecutionEvent { event } if event.event_id == "command_cmd_001" => {
                assert_eq!(serde_json::to_value(&event.kind).unwrap(), json!("command"));
                assert_eq!(event.command.as_deref(), Some("pwd"));
                assert_eq!(event.cwd.as_deref(), Some(expected_repo_path.as_str()));
                match serde_json::to_value(&event.status).unwrap() {
                    value if value == json!("started") => saw_command_started = true,
                    value if value == json!("completed") => {
                        assert_eq!(event.exit_code, Some(0));
                        assert!(
                            event
                                .output
                                .as_deref()
                                .unwrap_or_default()
                                .contains(expected_repo_path.as_str())
                        );
                        saw_command_completed = true;
                    }
                    other => panic!("unexpected command status: {other}"),
                }
            }
            WsOutMessage::MessageComplete {
                checkpoint_id: next_checkpoint,
                ..
            } => {
                checkpoint = Some(next_checkpoint);
                break;
            }
            WsOutMessage::Error { message } => panic!("ws error: {message}"),
            _ => {}
        }
    }
    assert!(
        saw_command_started,
        "websocket did not emit command started"
    );
    assert!(
        saw_command_completed,
        "websocket did not emit command completed"
    );
    assert!(checkpoint.as_deref().unwrap_or_default().starts_with("cp_"));
    accept_author_output(&mut ws).await;
    let stage = recv_until_stage(&mut ws, "human_confirm").await;
    assert_eq!(stage, "human_confirm");

    drop(ws);
    server.abort();
}

#[tokio::test]
async fn workspace_ws_reconnect_during_review_decision_can_still_run_revision() {
    let root = tempdir().expect("root");
    create_workspace_session_fixture_with_providers(&root, "fake", "codex", 2).await;
    let author_prompts = Arc::new(Mutex::new(Vec::new()));
    let mut registry = ProviderRegistry::new();
    registry.register(
        ProviderName::Fake,
        Arc::new(ScriptedStreamingProvider::new(
            [INITIAL_STORY_SPEC, REVISED_AFTER_RECONNECT_STORY_SPEC],
            author_prompts.clone(),
        )),
    );
    registry.register(
        ProviderName::Codex,
        Arc::new(ScriptedStreamingProvider::new(
            [
                r#"需要补充失败路径。

```json
{
  "verdict": "revise",
  "summary": "补充失败路径",
  "findings": [
    {
      "severity": "must_fix",
      "message": "缺少失败路径",
      "evidence": "Artifact 未覆盖失败路径",
      "impact": "下一阶段无法验收异常流程",
      "required_action": "补充失败路径说明"
    }
  ]
}
```"#,
                "审核通过。\n\n```json\n{\"verdict\":\"pass\",\"summary\":\"可以确认\"}\n```",
            ],
            Arc::new(Mutex::new(Vec::new())),
        )),
    );
    let app = build_web_router(WebAppState::with_provider_registry(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
        registry,
    ));
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    let url = format!("ws://{addr}/api/workspace-sessions/workspace_session_0001/ws");
    let (mut ws, _) = connect_async(url.clone()).await.expect("connect ws");
    let _initial = recv_json(&mut ws).await;
    send_json(
        &mut ws,
        &WsInMessage::UserMessage {
            content: "生成 Story Spec".to_string(),
        },
    )
    .await;
    for _ in 0..600 {
        match recv_json(&mut ws).await {
            WsOutMessage::StageChange { stage } if stage == "author_confirm" => {
                send_json(
                    &mut ws,
                    &WsInMessage::AuthorDecision {
                        decision: AuthorDecision::Accept,
                    },
                )
                .await;
            }
            WsOutMessage::ReviewDecisionRequired { .. } => break,
            WsOutMessage::Error { message } => panic!("ws error: {message}"),
            _ => {}
        }
    }
    drop(ws);

    let (mut reconnected, _) = connect_async(url).await.expect("reconnect ws");
    let _state = recv_json(&mut reconnected).await;
    send_json(
        &mut reconnected,
        &WsInMessage::ReviewDecisionResponse {
            decision: "continue_with_context".to_string(),
            extra_context: Some("重连后补充".to_string()),
        },
    )
    .await;

    let mut saw_revision = false;
    let mut saw_human_confirm = false;
    for _ in 0..600 {
        match recv_json(&mut reconnected).await {
            WsOutMessage::StreamChunk { content, .. }
                if content.contains("# Revised After Reconnect") =>
            {
                saw_revision = true;
            }
            WsOutMessage::StageChange { stage } if stage == "author_confirm" => {
                send_json(
                    &mut reconnected,
                    &WsInMessage::AuthorDecision {
                        decision: AuthorDecision::Accept,
                    },
                )
                .await;
            }
            WsOutMessage::StageChange { stage } if stage == "human_confirm" => {
                saw_human_confirm = true;
                break;
            }
            WsOutMessage::Error { message } => panic!("ws error: {message}"),
            _ => {}
        }
    }
    assert!(saw_revision);
    assert!(saw_human_confirm);
    let prompts = author_prompts.lock().unwrap();
    assert!(prompts[1].contains("需要补充失败路径"));
    assert!(prompts[1].contains("重连后补充"));

    drop(reconnected);
    server.abort();
}

struct WorkingDirRecordingStreamingProvider {
    observed_working_dir: Arc<Mutex<Option<PathBuf>>>,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for WorkingDirRecordingStreamingProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        *self.observed_working_dir.lock().unwrap() = Some(input.working_dir);
        let (event_tx, event_rx) = mpsc::channel(8);
        let (command_tx, _command_rx) = mpsc::channel::<ProviderCommand>(8);
        tokio::spawn(async move {
            let _ = event_tx
                .send(ProviderEvent::TextDelta {
                    content: VALID_STORY_SPEC.to_string(),
                })
                .await;
            let _ = event_tx
                .send(ProviderEvent::Completed {
                    full_output: VALID_STORY_SPEC.to_string(),
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

struct ScriptedStreamingProvider {
    outputs: Mutex<VecDeque<String>>,
    prompts: Arc<Mutex<Vec<String>>>,
}

impl ScriptedStreamingProvider {
    fn new<const N: usize>(outputs: [&str; N], prompts: Arc<Mutex<Vec<String>>>) -> Self {
        Self {
            outputs: Mutex::new(outputs.into_iter().map(ToOwned::to_owned).collect()),
            prompts,
        }
    }
}

#[derive(Default)]
struct ChoiceThenArtifactProviderState {
    calls: Mutex<u32>,
    resume_ids: Mutex<Vec<Option<String>>>,
    prompts: Mutex<Vec<String>>,
}

struct ChoiceThenArtifactProvider {
    state: Arc<ChoiceThenArtifactProviderState>,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for ChoiceThenArtifactProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        self.state
            .resume_ids
            .lock()
            .unwrap()
            .push(input.resume_provider_session_id.clone());
        self.state.prompts.lock().unwrap().push(input.prompt);
        let mut calls = self.state.calls.lock().unwrap();
        *calls += 1;
        let call_no = *calls;
        drop(calls);

        let output = if call_no == 1 {
            "需要先确认一个边界条件，然后我再生成最终 Story Spec：\n\
             `climb_stairs(n)` 对 `n <= 0` 应该如何处理？\n\
             - **A)** 返回 `0`，仅把正整数楼梯数视为有效输入\n\
             - **B)** 抛出异常，例如 `ValueError`\n\
             - **C)** 不定义该行为，Story Spec 只覆盖 issue 明确要求的 `n >= 1` 场景"
        } else {
            "# Story Spec\n\n\
             ## 范围\n\
             来源 source id: Issue issue_0001；实现 climb_stairs。\n\n\
             ## 用户故事\n\
             作为调用方，我需要计算爬楼梯方法数。\n\n\
             ## 功能需求\n\
             - [REQ-001] 实现 `climb_stairs(n: i32) -> i32`。\n\n\
             ## 成功标准\n\
             - [AC-001] 覆盖 n=1、n=2、n=3、n=5、n=10。\n\n\
             ## 待确认项\n\
             无\n\n\
             ## 非功能需求\n\
             使用 Python 实现。"
        };
        let (event_tx, event_rx) = mpsc::channel(8);
        let (command_tx, _command_rx) = mpsc::channel::<ProviderCommand>(8);
        tokio::spawn(async move {
            let output = output.to_string();
            let _ = event_tx
                .send(ProviderEvent::TextDelta {
                    content: output.clone(),
                })
                .await;
            let _ = event_tx
                .send(ProviderEvent::Completed {
                    full_output: output,
                    provider_session_id: Some("author-provider-session-1".to_string()),
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
            "run_streaming is not used by workspace websocket",
            0,
        ))
    }
}

#[derive(Default)]
struct RoleResumeRecordingProviderState {
    author_resume_ids: Mutex<Vec<Option<String>>>,
    reviewer_resume_ids: Mutex<Vec<Option<String>>>,
}

struct RoleResumeRecordingProvider {
    state: Arc<RoleResumeRecordingProviderState>,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for RoleResumeRecordingProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let (output, provider_session_id) = match input.role {
            AdapterRole::Reviewer => {
                self.state
                    .reviewer_resume_ids
                    .lock()
                    .unwrap()
                    .push(input.resume_provider_session_id.clone());
                (
                    "审核通过。\n```json\n{\"verdict\":\"pass\",\"summary\":\"ok\"}\n```",
                    Some("reviewer-provider-session-1".to_string()),
                )
            }
            _ => {
                self.state
                    .author_resume_ids
                    .lock()
                    .unwrap()
                    .push(input.resume_provider_session_id.clone());
                (
                    "# Story Spec\n\n\
                     ## 范围\n来源 source id: Issue issue_0001；实现登录会话过期提示。\n\n\
                     ## 用户故事\n作为用户，我希望登录会话过期时获得清晰提示。\n\n\
                     ## 功能需求\n- [REQ-001] 实现登录会话过期提示。\n\n\
                     ## 成功标准\n- [AC-001] 会话过期时提示用户重新登录。\n\n\
                     ## 待确认项\n无\n\n\
                     ## 非功能需求\n无\n",
                    Some("author-provider-session-1".to_string()),
                )
            }
        };
        let (event_tx, event_rx) = mpsc::channel(8);
        let (command_tx, _command_rx) = mpsc::channel::<ProviderCommand>(8);
        tokio::spawn(async move {
            let _ = event_tx
                .send(ProviderEvent::Completed {
                    full_output: output.to_string(),
                    provider_session_id,
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
            "run_streaming is not used by workspace websocket",
            0,
        ))
    }
}

struct HangingStreamingProvider;
