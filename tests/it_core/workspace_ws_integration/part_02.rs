#[tokio::test]
async fn workspace_session_detail_http_api_returns_full_persisted_content() {
    let root = tempdir().expect("root");
    create_workspace_session_fixture(&root).await;
    let lifecycle = LifecycleStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let detail = NodeDetail {
        node_id: "author_run_001".to_string(),
        session_id: "workspace_session_0001".to_string(),
        node_type: TimelineNodeType::AuthorRun,
        status: TimelineNodeStatus::Completed,
        agent_role: Some(AgentRole::Author),
        provider: Some(ProviderSnapshot {
            name: "fake".to_string(),
            model: "fixture-model".to_string(),
        }),
        prompt: Some("完整 Provider Prompt 文本\n包含第二行".to_string()),
        messages: vec![json!({"role":"user","content":"请生成完整产物"})],
        streaming_content: "完整输出\n包含工具结果".to_string(),
        execution_events: vec![
            json!({"event_id":"event_output_001","kind":"output","output":"完整输出\n包含工具结果"}),
            json!({"event_id":"event_without_output","kind":"output","output":null}),
        ],
        permission_events: Vec::new(),
        verdict: Some(json!({"verdict":"pass","summary":"可确认"})),
        artifact_ref: None,
        is_revision: false,
        revision_feedback: None,
        base_artifact_ref: None,
        started_at: "2026-05-20T14:30:00Z".to_string(),
        ended_at: Some("2026-05-20T14:35:00Z".to_string()),
    };
    lifecycle
        .save_node_detail("workspace_session_0001", "author_run_001", &detail)
        .expect("save node detail");
    lifecycle
        .append_artifact_version(
            "workspace_session_0001",
            ArtifactVersion {
                version: 3,
                payload: ArtifactPayload::Markdown {
                    markdown: "# Artifact v3\n\n完整 Markdown".to_string(),
                    diff: None,
                },
                generated_by: ProviderName::Fake,
                reviewed_by: Some(ProviderName::Fake),
                review_verdict: Some(ReviewVerdictType::Pass),
                confirmed_by: None,
                is_current: true,
                created_at: "2026-05-20T14:36:00Z".to_string(),
                source_node_id: "author_run_001".to_string(),
            },
        )
        .expect("append artifact version");

    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));

    let (status, node_detail) = request_json(
        app.clone(),
        Method::GET,
        "/api/workspace-sessions/workspace_session_0001/timeline-node-details/author_run_001",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(node_detail["node_id"], "author_run_001");
    assert_eq!(
        node_detail["prompt"],
        "完整 Provider Prompt 文本\n包含第二行"
    );
    assert_eq!(node_detail["streaming_content"], "完整输出\n包含工具结果");
    assert_eq!(node_detail["messages"][0]["content"], "请生成完整产物");

    let (status, prompt) = request_json(
        app.clone(),
        Method::GET,
        "/api/workspace-sessions/workspace_session_0001/timeline-node-details/author_run_001/prompt",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(prompt["node_id"], "author_run_001");
    assert_eq!(prompt["prompt"], "完整 Provider Prompt 文本\n包含第二行");

    let (status, output) = request_json(
        app.clone(),
        Method::GET,
        "/api/workspace-sessions/workspace_session_0001/timeline-node-details/author_run_001/events/event_output_001/output",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(output["node_id"], "author_run_001");
    assert_eq!(output["event_id"], "event_output_001");
    assert_eq!(output["output"], "完整输出\n包含工具结果");

    let (status, artifact) = request_json(
        app.clone(),
        Method::GET,
        "/api/workspace-sessions/workspace_session_0001/artifact-versions/3",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(artifact["version"], 3);
    assert_eq!(artifact["markdown"], "# Artifact v3\n\n完整 Markdown");

    let (missing_node_status, _) = request_json(
        app.clone(),
        Method::GET,
        "/api/workspace-sessions/workspace_session_0001/timeline-node-details/missing_node",
        json!({}),
    )
    .await;
    assert_eq!(missing_node_status, StatusCode::NOT_FOUND);

    let (missing_event_status, _) = request_json(
        app.clone(),
        Method::GET,
        "/api/workspace-sessions/workspace_session_0001/timeline-node-details/author_run_001/events/missing_event/output",
        json!({}),
    )
    .await;
    assert_eq!(missing_event_status, StatusCode::NOT_FOUND);
    let (missing_output_status, _) = request_json(
        app.clone(),
        Method::GET,
        "/api/workspace-sessions/workspace_session_0001/timeline-node-details/author_run_001/events/event_without_output/output",
        json!({}),
    )
    .await;
    assert_eq!(missing_output_status, StatusCode::NOT_FOUND);

    let (missing_artifact_status, _) = request_json(
        app,
        Method::GET,
        "/api/workspace-sessions/workspace_session_0001/artifact-versions/99",
        json!({}),
    )
    .await;
    assert_eq!(missing_artifact_status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn workspace_ws_review_report_feedback_revision_runs_second_review() {
    let root = tempdir().expect("root");
    create_workspace_session_fixture_with_providers(&root, "fake", "codex", 2).await;
    let author_prompts = Arc::new(Mutex::new(Vec::new()));
    let reviewer_prompts = Arc::new(Mutex::new(Vec::new()));
    let mut registry = ProviderRegistry::new();
    registry.register(
        ProviderName::Fake,
        Arc::new(ScriptedStreamingProvider::new(
            [INITIAL_STORY_SPEC, REVISED_STORY_SPEC],
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
            reviewer_prompts.clone(),
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
    let (mut ws, _) = connect_async(url).await.expect("connect ws");
    let _initial = recv_json(&mut ws).await;

    send_json(
        &mut ws,
        &WsInMessage::UserMessage {
            content: "生成 Story Spec".to_string(),
        },
    )
    .await;

    // spec-design-dialog-revision T9：旧「ReviewDecision continue_with_context」四段路径迁移为
    // 对话式循环：review revise 报告回对话流 → AuthorConfirm 反馈修订 → 再次确认送审 → 二次 review。
    let mut saw_first_review = false;
    for _ in 0..600 {
        match recv_json(&mut ws).await {
            WsOutMessage::StageChange { stage } if stage == "author_confirm" && !saw_first_review => {
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
                saw_first_review = true;
            }
            WsOutMessage::StageChange { stage } if stage == "author_confirm" && saw_first_review => {
                // 首轮 review 完成后的 AuthorConfirm：改用反馈修订而非旧 ReviewDecisionResponse。
                send_json(
                    &mut ws,
                    &WsInMessage::AuthorDecision {
                        decision: AuthorDecision::Revise {
                            feedback: "补充登录错误码".to_string(),
                        },
                    },
                )
                .await;
                break;
            }
            WsOutMessage::Error { message } => panic!("ws error: {message}"),
            _ => {}
        }
    }
    assert!(saw_first_review, "first review should report revise verdict");

    let mut saw_revision_stream = false;
    let mut saw_post_revision_confirm = false;
    for _ in 0..600 {
        match recv_json(&mut ws).await {
            WsOutMessage::StreamChunk { content, .. }
                if content.contains("# Revised Story Spec") =>
            {
                saw_revision_stream = true;
            }
            WsOutMessage::StageChange { stage } if stage == "author_confirm" => {
                saw_post_revision_confirm = true;
                send_json(
                    &mut ws,
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
        "revision completion should return to author_confirm"
    );

    let mut saw_review_pass = false;
    let mut saw_final_author_confirm = false;
    for _ in 0..600 {
        match recv_json(&mut ws).await {
            WsOutMessage::ReviewComplete { summary, .. } if summary == "可以确认" => {
                saw_review_pass = true;
            }
            WsOutMessage::StageChange { stage } if stage == "author_confirm" && saw_review_pass => {
                saw_final_author_confirm = true;
                break;
            }
            WsOutMessage::Error { message } => panic!("ws error: {message}"),
            _ => {}
        }
    }

    assert!(
        saw_revision_stream,
        "revision output should stream to websocket"
    );
    assert!(saw_review_pass, "second review should pass");
    assert!(
        saw_final_author_confirm,
        "second review pass should return to author_confirm (T5: 不再自动进入 human_confirm)"
    );
    let prompts = author_prompts.lock().unwrap();
    let revision_prompt = prompts.get(1).expect("revision author prompt");
    // T4：反馈修订走 build_author_revision_prompt（产物全文 + 用户反馈），不再携带 reviewer 返修 preamble。
    assert!(revision_prompt.contains("## 用户反馈"));
    assert!(revision_prompt.contains("补充登录错误码"));
    assert_eq!(reviewer_prompts.lock().unwrap().len(), 2);

    drop(ws);
    server.abort();
}

#[tokio::test]
async fn workspace_ws_rollback_truncates_persistent_messages() {
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
            content: "first".to_string(),
        },
    )
    .await;
    let first_checkpoint = recv_until_message_complete(&mut ws).await;
    accept_author_output(&mut ws).await;
    let _ = recv_until_stage(&mut ws, "human_confirm").await;

    send_json(
        &mut ws,
        &WsInMessage::UserMessage {
            content: "second".to_string(),
        },
    )
    .await;
    let _second_checkpoint = recv_until_message_complete(&mut ws).await;
    accept_author_output(&mut ws).await;
    let _ = recv_until_stage(&mut ws, "human_confirm").await;

    send_json(
        &mut ws,
        &WsInMessage::Rollback {
            checkpoint_id: first_checkpoint,
        },
    )
    .await;

    let rolled_back = recv_until_session_state(&mut ws).await;
    match rolled_back {
        WsOutMessage::SessionState {
            messages, stage, ..
        } => {
            assert_eq!(stage, "author_confirm");
            assert_eq!(messages.len(), 3);
            assert!(messages.iter().any(|message| message.role == "system"));
            assert!(messages.iter().any(|message| message.content == "first"));
            assert!(!messages.iter().any(|message| message.content == "second"));
        }
        other => panic!("expected session_state, got {other:?}"),
    }

    let messages = persisted_workspace_messages(root.path());
    assert_eq!(messages.len(), 3);
    assert!(messages.iter().any(|message| message.role == "system"));
    assert!(!messages.iter().any(|message| message.content == "second"));

    drop(ws);
    server.abort();
}

#[tokio::test]
async fn workspace_ws_provider_selection_persists_across_reconnect() {
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
    let (mut ws, _) = connect_async(url.clone()).await.expect("connect ws");
    let _initial = recv_json(&mut ws).await;

    send_json(
        &mut ws,
        &WsInMessage::ProviderSelect {
            role: "author".to_string(),
            provider: cadence_aria::product::models::ProviderName::Codex,
        },
    )
    .await;
    let updated = recv_until_session_state(&mut ws).await;
    match updated {
        WsOutMessage::SessionState { providers, .. } => {
            assert_eq!(
                serde_json::to_value(providers.author).unwrap(),
                json!("codex")
            );
        }
        other => panic!("expected session_state, got {other:?}"),
    }
    drop(ws);

    let (mut reconnected, _) = connect_async(url).await.expect("reconnect ws");
    let reloaded = recv_json(&mut reconnected).await;
    match reloaded {
        WsOutMessage::SessionState { providers, .. } => {
            assert_eq!(
                serde_json::to_value(providers.author).unwrap(),
                json!("codex")
            );
        }
        other => panic!("expected session_state, got {other:?}"),
    }

    drop(reconnected);
    server.abort();
}

#[tokio::test]
async fn workspace_ws_start_generation_includes_context_note_in_author_prompt() {
    let root = tempdir().expect("root");
    create_workspace_session_fixture_with_providers(&root, "fake", "fake", 1).await;
    let author_prompts = Arc::new(Mutex::new(Vec::new()));
    let mut registry = ProviderRegistry::new();
    registry.register(
        ProviderName::Fake,
        Arc::new(ScriptedStreamingProvider::new(
            [VALID_STORY_SPEC],
            author_prompts.clone(),
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
    let (mut ws, _) = connect_async(url).await.expect("connect ws");
    let _initial = recv_json(&mut ws).await;

    send_json(
        &mut ws,
        &WsInMessage::ContextNote {
            content: "用户补充：必须覆盖 n=10 -> 89。".to_string(),
        },
    )
    .await;
    send_json(
        &mut ws,
        &WsInMessage::StartGeneration {
            provider_config: ProviderConfigSnapshot {
                author: ProviderName::Fake,
                reviewer: None,
                review_rounds: 0,
                permission_modes: cadence_aria::product::models::WorkspaceRolePermissionModes::default(),
            },
            reviewer_enabled: false,
        },
    )
    .await;

    let _checkpoint = recv_until_message_complete(&mut ws).await;
    let prompt = author_prompts
        .lock()
        .unwrap()
        .first()
        .expect("author prompt")
        .clone();
    assert!(
        prompt.contains("用户补充：必须覆盖 n=10 -> 89。"),
        "author prompt should include context note, got: {prompt}"
    );

    let messages = persisted_workspace_messages(root.path());
    assert!(messages.iter().any(|message| {
        message.role == "user" && message.content == "用户补充：必须覆盖 n=10 -> 89。"
    }));

    drop(ws);
    server.abort();
}

#[tokio::test]
async fn workspace_ws_author_decision_accept_starts_reviewer() {
    let root = tempdir().expect("root");
    create_workspace_session_fixture_with_providers(&root, "fake", "codex", 1).await;
    let author_prompts = Arc::new(Mutex::new(Vec::new()));
    let reviewer_prompts = Arc::new(Mutex::new(Vec::new()));
    let mut registry = ProviderRegistry::new();
    registry.register(
        ProviderName::Fake,
        Arc::new(ScriptedStreamingProvider::new(
            [VALID_STORY_SPEC],
            author_prompts,
        )),
    );
    registry.register(
        ProviderName::Codex,
        Arc::new(ScriptedStreamingProvider::new(
            ["审核通过。\n```json\n{\"verdict\":\"pass\",\"summary\":\"可进入人工确认\"}\n```"],
            reviewer_prompts.clone(),
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
    let (mut ws, _) = connect_async(url).await.expect("connect ws");
    let _initial = recv_json(&mut ws).await;

    send_json(
        &mut ws,
        &WsInMessage::StartGeneration {
            provider_config: ProviderConfigSnapshot {
                author: ProviderName::Fake,
                reviewer: Some(ProviderName::Codex),
                review_rounds: 1,
                permission_modes: cadence_aria::product::models::WorkspaceRolePermissionModes::default(),
            },
            reviewer_enabled: true,
        },
    )
    .await;

    let _checkpoint = recv_until_message_complete(&mut ws).await;
    assert_eq!(
        recv_until_stage(&mut ws, "author_confirm").await,
        "author_confirm"
    );

    send_json(
        &mut ws,
        &WsInMessage::AuthorDecision {
            decision: AuthorDecision::Accept,
        },
    )
    .await;

    assert_eq!(
        recv_until_stage(&mut ws, "cross_review").await,
        "cross_review"
    );
    // spec-design-dialog-revision T5：review pass 不自动定稿，统一回 AuthorConfirm 等待用户确认。
    for _ in 0..600 {
        match recv_json(&mut ws).await {
            WsOutMessage::ReviewComplete { verdict, .. } => {
                assert_eq!(verdict, ReviewVerdictType::Pass);
                break;
            }
            WsOutMessage::Error { message } => panic!("ws error: {message}"),
            _ => {}
        }
    }
    assert_eq!(
        recv_until_stage(&mut ws, "author_confirm").await,
        "author_confirm"
    );
    let prompts = reviewer_prompts.lock().unwrap();
    assert_eq!(prompts.len(), 1);
    assert!(prompts[0].contains("当前 Artifact"));
    assert!(prompts[0].contains("# Story Spec"));

    drop(ws);
    server.abort();
}

// spec-design-dialog-revision T9：Reject 在 Story/Design 已移除推倒重来出口——返回引导性错误，
// 阶段与产物保持不变（改用反馈修订表达重写意图），重连后仍处于 AuthorConfirm 且产物保留。
#[tokio::test]
async fn workspace_ws_author_decision_reject_returns_guidance_error_and_survives_reconnect() {
    let root = tempdir().expect("root");
    create_workspace_session_fixture_with_providers(&root, "fake", "codex", 1).await;
    let reviewer_prompts = Arc::new(Mutex::new(Vec::new()));
    let mut registry = ProviderRegistry::new();
    registry.register(
        ProviderName::Fake,
        Arc::new(ScriptedStreamingProvider::new(
            [VALID_STORY_SPEC],
            Arc::new(Mutex::new(Vec::new())),
        )),
    );
    registry.register(
        ProviderName::Codex,
        Arc::new(ScriptedStreamingProvider::new(
            ["reviewer should not run when author decision is rejected"],
            reviewer_prompts.clone(),
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
        &WsInMessage::StartGeneration {
            provider_config: ProviderConfigSnapshot {
                author: ProviderName::Fake,
                reviewer: Some(ProviderName::Codex),
                review_rounds: 1,
                permission_modes: cadence_aria::product::models::WorkspaceRolePermissionModes::default(),
            },
            reviewer_enabled: true,
        },
    )
    .await;

    let _checkpoint = recv_until_message_complete(&mut ws).await;
    assert_eq!(
        recv_until_stage(&mut ws, "author_confirm").await,
        "author_confirm"
    );
    send_json(
        &mut ws,
        &WsInMessage::AuthorDecision {
            decision: AuthorDecision::Reject,
        },
    )
    .await;

    let mut reject_error = None;
    for _ in 0..600 {
        match recv_json(&mut ws).await {
            WsOutMessage::ProtocolError { code, message, .. } => {
                reject_error = Some((code, message));
                break;
            }
            WsOutMessage::Error { message } => panic!("ws error: {message}"),
            _ => {}
        }
    }
    let (code, message) = reject_error.expect("reject should return guidance protocol error");
    assert_eq!(code, "INVALID_AUTHOR_DECISION");
    assert!(
        message.contains("反馈"),
        "reject guidance should point to feedback revision: {message}"
    );
    assert!(
        reviewer_prompts.lock().unwrap().is_empty(),
        "reviewer must not run when author decision is rejected"
    );

    drop(ws);
    let (mut reconnected, _) = connect_async(url).await.expect("reconnect ws");
    match recv_json(&mut reconnected).await {
        WsOutMessage::SessionState {
            stage,
            artifact,
            artifact_versions,
            artifact_version_summaries,
            messages,
            ..
        } => {
            assert_eq!(stage, "author_confirm");
            assert!(
                artifact.is_some(),
                "reject guidance error must not discard the artifact"
            );
            // Story 会话 SessionState 的 artifact_versions 恒空（版本史走 artifact_version_summaries）。
            assert_eq!(artifact_versions.len(), 0);
            assert_eq!(artifact_version_summaries.len(), 1);
            assert!(
                artifact_version_summaries[0].is_current,
                "reject 不再作废产物，当前稿保持 current"
            );
            assert!(messages.iter().any(|message| {
                message.role == "assistant" && message.content.contains("# Story Spec")
            }));
        }
        other => panic!("expected reconnected session state, got {other:?}"),
    }

    drop(reconnected);
    server.abort();
}
