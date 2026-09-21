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

// 退役留档（T5/REQ-RET-02）：`workspace_ws_review_report_feedback_revision_runs_second_review` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

// 退役留档（T5/REQ-RET-02）：`workspace_ws_rollback_truncates_persistent_messages` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

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

// F-30（v34 复验，标本 issue_0003/session_0008）：已确认终态会话上「开始生成」
// 必须整链拒绝。用户门可观察契约：收到明确 protocol_error（stage=completed
// 不放行 start_generation，协议矩阵先拦；引擎侧另有 F-30 终态守卫兜底直调路径），
// 零 ProviderLocked、provider 零调用、durable 状态与 timeline 零改写。
#[tokio::test]
async fn workspace_ws_start_generation_on_confirmed_session_is_rejected_without_side_effects() {
    let root = tempdir().expect("root");
    create_workspace_session_fixture(&root).await;
    set_workspace_session_status_field(&root, "confirmed");
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

    match recv_json(&mut ws).await {
        WsOutMessage::ProtocolError { code, message, .. } => {
            assert_eq!(code, "INVALID_MESSAGE_FOR_STAGE");
            assert!(
                message.contains("start_generation") && message.contains("completed"),
                "rejection should be diagnosable, got: {message}"
            );
        }
        other => panic!("expected protocol_error rejection, got {other:?}"),
    }

    // provider 未被启动（无 prompt 进 provider），durable 零改写。
    assert!(
        author_prompts.lock().unwrap().is_empty(),
        "author provider must never be prompted on a terminal session"
    );
    let lifecycle = LifecycleStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let session = lifecycle
        .get_workspace_session("workspace_session_0001")
        .expect("workspace session");
    assert_eq!(
        session.status,
        cadence_aria::product::models::WorkspaceSessionStatus::Confirmed
    );
    let timeline = lifecycle
        .load_timeline_nodes("workspace_session_0001")
        .expect("timeline nodes");
    assert!(
        timeline
            .iter()
            .all(|node| node.node_type != TimelineNodeType::StartGeneration),
        "no StartGeneration node may be persisted for a terminal session"
    );

    drop(ws);
    server.abort();
}

fn set_workspace_session_status_field(root: &TempDir, status: &str) {
    let session_path = root.path().join(
        ".aria/projects/project_0001/issues/issue_0001/workspace-sessions/workspace_session_0001.json",
    );
    let mut session: Value =
        serde_json::from_str(&fs::read_to_string(&session_path).expect("workspace session json"))
            .expect("workspace session value");
    assert!(
        session.get("status").is_some(),
        "fixture session json should carry a status field"
    );
    session["status"] = json!(status);
    fs::write(&session_path, serde_json::to_string_pretty(&session).unwrap())
        .expect("write workspace session");
}

// 退役留档（T5/REQ-RET-02）：`workspace_ws_author_decision_accept_starts_reviewer` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

// spec-design-dialog-revision T9：Reject 在 Story/Design 已移除推倒重来出口——返回引导性错误，
// 阶段与产物保持不变（改用反馈修订表达重写意图），重连后仍处于 AuthorConfirm 且产物保留。
// 退役留档（T5/REQ-RET-02）：`workspace_ws_author_decision_reject_returns_guidance_error_and_survives_reconnect` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。
