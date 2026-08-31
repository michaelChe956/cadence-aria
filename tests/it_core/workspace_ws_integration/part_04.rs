#[tokio::test]
async fn workspace_ws_sc_human_gate_feedback_reaches_dispatch_after_socket_stage_gate() {
    let root = tempdir().expect("root");
    let _repo = create_workspace_session_fixture(&root).await;
    let lifecycle = LifecycleStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let plan = lifecycle
        .create_issue_work_item_plan(
            cadence_aria::product::lifecycle_store::CreateIssueWorkItemPlanInput {
                id: Some("issue_work_item_plan_sc_socket".to_string()),
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                source_story_spec_ids: vec![],
                source_design_spec_ids: vec![],
                options: cadence_aria::product::models::IssueWorkItemPlanOptions {
                    include_integration_tests: false,
                    include_e2e_tests: false,
                    force_frontend_backend_split: false,
                    require_execution_plan_confirm: false,
                },
                status: cadence_aria::product::models::IssueWorkItemPlanStatus::Draft,
                work_item_ids: vec![],
                repository_profile_ref: None,
                verification_plan_ids: vec![],
                dependency_graph: vec![],
                created_from_provider_run: None,
                validator_findings: vec![],
            },
        )
        .expect("create plan");
    let session = lifecycle
        .create_workspace_session(
            cadence_aria::product::lifecycle_store::CreateWorkspaceSessionInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                entity_id: plan.id,
                workspace_type: cadence_aria::product::models::WorkspaceType::WorkItemPlan,
                author_provider: ProviderName::Fake,
                reviewer_provider: ProviderName::Fake,
                review_rounds: 0,
                superpowers_enabled: false,
                openspec_enabled: false,
                work_item_plan_options: Some(
                    cadence_aria::product::lifecycle_store::WorkItemPlanSessionOptions {
                        flow_kind:
                            cadence_aria::product::work_item_plan_policy::WorkItemPlanFlowKind::SingleCandidate,
                        run_policy: cadence_aria::product::work_item_plan_policy::RunPolicy::Interactive,
                        rollout_snapshot: true,
                    },
                ),
            },
        )
        .expect("create SC session");
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    let url = format!("ws://{addr}/api/workspace-sessions/{}/ws", session.id);
    let (mut ws, _) = connect_async(url).await.expect("connect ws");
    assert!(matches!(recv_json(&mut ws).await, WsOutMessage::SessionState { .. }));
    send_json(
        &mut ws,
        &WsInMessage::HumanGateFeedback {
            command_id: "cmd-socket-gate".to_string(),
            feedback: "请保留完整候选，仅修正此处".to_string(),
        },
    )
    .await;

    let response = recv_json(&mut ws).await;
    assert!(matches!(
        response,
        WsOutMessage::ProtocolError { code, .. }
            if code == "WORK_ITEM_PLAN_HUMAN_GATE_STAGE_INVALID"
    ));

    let _ = ws.close(None).await;
    server.abort();
}

#[tokio::test]
async fn workspace_ws_abort_with_pi_reaches_cancelled_state_and_stops_output() {
    use std::sync::atomic::{AtomicBool, Ordering};

    let root = tempdir().expect("root");
    create_workspace_session_fixture_with_author(&root, "pi").await;
    let started = Arc::new(AtomicBool::new(false));
    let abort_observed = Arc::new(Notify::new());
    let seen_inputs = Arc::new(Mutex::new(Vec::new()));
    let mut registry = ProviderRegistry::new();
    registry.register(
        ProviderName::Pi,
        Arc::new(PiHangingStreamingProvider {
            started: started.clone(),
            abort_observed: abort_observed.clone(),
            seen_inputs: seen_inputs.clone(),
        }),
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
            content: "run Pi then cancel".to_string(),
        },
    )
    .await;
    assert_eq!(recv_until_stream_chunk(&mut ws).await, "Pi partial output");
    assert!(started.load(Ordering::SeqCst), "Pi author adapter must start");
    assert_eq!(
        seen_inputs.lock().expect("Pi input recorder").as_slice(),
        &[(
            cadence_aria::protocol::contracts::ProviderType::Pi,
            cadence_aria::cross_cutting::streaming_provider::ProviderPermissionMode::Auto,
        )],
        "workspace must invoke Pi in Auto mode"
    );

    send_json(&mut ws, &WsInMessage::Abort).await;

    let mut saw_aborted_status = false;
    for _ in 0..80 {
        match recv_json(&mut ws).await {
            WsOutMessage::ProviderStatus {
                status: WsProviderStatus::Aborted,
            } => saw_aborted_status = true,
            WsOutMessage::StageChange { stage } if stage == "prepare_context" => {
                assert!(saw_aborted_status, "frontend must receive the existing aborted status");
                timeout(Duration::from_secs(1), abort_observed.notified())
                    .await
                    .expect("Pi provider must observe Abort before emitting post-abort events");
                let session = LifecycleStore::new(ProductAppPaths::new(root.path().join(".aria")))
                    .get_workspace_session("workspace_session_0001")
                    .expect("persisted workspace session");
                assert_eq!(session.status, cadence_aria::product::models::WorkspaceSessionStatus::Open);
                let no_trailing_output = timeout(Duration::from_millis(150), ws.next()).await;
                match no_trailing_output {
                    Err(_) => {}
                    Ok(Some(Ok(Message::Text(text)))) => {
                        let message: WsOutMessage =
                            serde_json::from_str(&text).expect("ws json after Abort");
                        assert!(
                            !matches!(message, WsOutMessage::StreamChunk { ref content, .. } if content.contains(PI_POST_ABORT_OUTPUT)),
                            "cancelled Pi run must suppress provider output emitted after Abort"
                        );
                        panic!("cancelled Pi run must not emit frontend output after Abort: {message:?}");
                    }
                    Ok(Some(Ok(other))) => {
                        panic!("cancelled Pi run must not emit websocket frame after Abort: {other:?}")
                    }
                    Ok(Some(Err(error))) => {
                        panic!("websocket read failed while checking post-abort output: {error}")
                    }
                    Ok(None) => panic!("websocket closed while checking post-abort output"),
                }
                drop(ws);

                let (mut reconnected, _) = connect_async(url).await.expect("reconnect ws");
                match recv_json(&mut reconnected).await {
                    WsOutMessage::SessionState {
                        stage,
                        active_run_id,
                        timeline_nodes,
                        ..
                    } => {
                        assert_eq!(stage, "prepare_context");
                        assert!(active_run_id.is_none());
                        let last = timeline_nodes.last().expect("cancelled Pi timeline node");
                        assert_eq!(last.status, TimelineNodeStatus::Failed);
                        assert_eq!(last.summary.as_deref(), Some("运行已中止"));
                    }
                    other => panic!("expected cancellation session state, got {other:?}"),
                }
                drop(reconnected);
                server.abort();
                return;
            }
            WsOutMessage::StreamChunk { .. } | WsOutMessage::MessageComplete { .. } => {
                panic!("cancelled Pi run must not emit output after Abort")
            }
            WsOutMessage::Error { message } => panic!("ws error: {message}"),
            _ => {}
        }
    }
    panic!("Pi abort did not return workspace to prepare_context");
}

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
    set_workspace_author_permission_mode_to_supervised(&root);
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

// spec-design-dialog-revision T9：旧「重连时处在 ReviewDecision 阶段」场景迁移——ReviewDecision 已从
// Story/Design 退役，等价场景为：review 完成回 AuthorConfirm 后断线，重连提交反馈修订仍可运行，
// 并可再次送审完成第二轮 review。
#[tokio::test]
async fn workspace_ws_reconnect_after_review_can_still_run_revision() {
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
      "message": "缺少失败路径\n影响：下一阶段无法验收异常流程",
      "evidence": "Artifact 未覆盖失败路径",
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
    let mut saw_review_revise = false;
    for _ in 0..600 {
        match recv_json(&mut ws).await {
            WsOutMessage::StageChange { stage } if stage == "author_confirm" && !saw_review_revise => {
                send_json(
                    &mut ws,
                    &WsInMessage::AuthorDecision {
                        decision: AuthorDecision::Accept,
                    },
                )
                .await;
            }
            WsOutMessage::ReviewComplete {
                verdict: ReviewVerdictType::Revise,
                ..
            } => {
                saw_review_revise = true;
            }
            WsOutMessage::StageChange { stage } if stage == "author_confirm" && saw_review_revise => {
                // review 完成回到 AuthorConfirm：在此处断线（等价旧 ReviewDecision 暂停点）。
                break;
            }
            WsOutMessage::Error { message } => panic!("ws error: {message}"),
            _ => {}
        }
    }
    assert!(saw_review_revise, "review should report revise verdict");
    drop(ws);

    let (mut reconnected, _) = connect_async(url).await.expect("reconnect ws");
    let _state = recv_json(&mut reconnected).await;
    send_json(
        &mut reconnected,
        &WsInMessage::AuthorDecision {
            decision: AuthorDecision::Revise {
                feedback: "重连后补充".to_string(),
            },
        },
    )
    .await;

    let mut saw_revision = false;
    let mut saw_post_revision_confirm = false;
    for _ in 0..600 {
        match recv_json(&mut reconnected).await {
            WsOutMessage::StreamChunk { content, .. }
                if content.contains("# Revised After Reconnect") =>
            {
                saw_revision = true;
            }
            WsOutMessage::StageChange { stage } if stage == "author_confirm" && saw_revision => {
                saw_post_revision_confirm = true;
                send_json(
                    &mut reconnected,
                    &WsInMessage::AuthorDecision {
                        decision: AuthorDecision::Accept,
                    },
                )
                .await;
                break;
            }
            WsOutMessage::Error { message } => panic!("ws error: {message}"),
            _ => {}
        }
    }
    assert!(
        saw_post_revision_confirm,
        "revision after reconnect should complete back to author_confirm"
    );

    let mut saw_second_review_pass = false;
    let mut saw_final_author_confirm = false;
    for _ in 0..600 {
        match recv_json(&mut reconnected).await {
            WsOutMessage::ReviewComplete { summary, .. } if summary == "可以确认" => {
                saw_second_review_pass = true;
            }
            WsOutMessage::StageChange { stage } if stage == "author_confirm" && saw_second_review_pass => {
                saw_final_author_confirm = true;
                break;
            }
            WsOutMessage::Error { message } => panic!("ws error: {message}"),
            _ => {}
        }
    }
    assert!(saw_second_review_pass, "second review after reconnect should pass");
    assert!(
        saw_final_author_confirm,
        "second review pass should return to author_confirm (T5)"
    );
    let prompts = author_prompts.lock().unwrap();
    // T4：反馈修订走 build_author_revision_prompt（产物全文 + 用户反馈）。
    assert!(prompts[1].contains("## 用户反馈"));
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
                .send(ProviderEvent::Completed(cadence_aria::cross_cutting::streaming_provider::ProviderCompletion::plain(VALID_STORY_SPEC.to_string(), None)))
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
                .send(ProviderEvent::Completed(cadence_aria::cross_cutting::streaming_provider::ProviderCompletion::plain(output, Some("author-provider-session-1".to_string()))))
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
                .send(ProviderEvent::Completed(cadence_aria::cross_cutting::streaming_provider::ProviderCompletion::plain(output.to_string(), provider_session_id)))
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
