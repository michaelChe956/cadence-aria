use cadence_aria::web::workspace_ws_types::TimelineNode;

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
                permission_modes:
                    cadence_aria::product::models::WorkspaceRolePermissionModes::default(),
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
                permission_modes:
                    cadence_aria::product::models::WorkspaceRolePermissionModes::default(),
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

// F-31 纠正轮（红①）：review 启用的 story 会话，confirm 缺省 / with_review=false 必须
// 直接定稿——是否送审由用户选择。v36 过度实现：confirm 无条件接管进 Review（引擎
// author_confirm_requires_review 命中即 begin_review_after_author_confirm），用户失去
// 「不评审直接定稿」的选项。修复后：仅 with_review=true 才进 Review。
#[tokio::test]
async fn http_confirm_without_with_review_finalizes_directly_when_review_enabled() {
    for with_review_field in [None, Some(false)] {
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
                ["reviewer must not run without explicit with_review"],
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
        let served_app = app.clone();
        let server = tokio::spawn(async move {
            axum::serve(listener, served_app).await.expect("serve");
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
                    permission_modes:
                        cadence_aria::product::models::WorkspaceRolePermissionModes::default(),
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

        // review 启用 + 本轮未评审，但用户未要求送审（缺省 / 显式 false）→ 直接定稿。
        let mut confirm_body = json!({"confirmed_by": "user"});
        if let Some(with_review) = with_review_field {
            confirm_body["with_review"] = json!(with_review);
        }
        let (status, body) = request_json(
            app,
            Method::POST,
            "/api/workspace-sessions/workspace_session_0001/confirm",
            confirm_body,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(
            body["status"], "confirmed",
            "review 启用但用户未选择送审，确认必须直接定稿：{body}"
        );

        assert!(
            reviewer_prompts.lock().unwrap().is_empty(),
            "未要求送审不得启动任何 reviewer provider run"
        );
        let lifecycle = LifecycleStore::new(ProductAppPaths::new(root.path().join(".aria")));
        let nodes = lifecycle
            .load_timeline_nodes("workspace_session_0001")
            .expect("timeline nodes");
        assert!(
            nodes
                .iter()
                .all(|node| node.node_type != TimelineNodeType::ReviewerRun),
            "直接定稿不得创建 ReviewerRun 节点，got {nodes:?}"
        );
        assert!(
            nodes
                .iter()
                .any(|node| node.node_type == TimelineNodeType::Completed),
            "直接定稿必须建 Completed 节点，got {nodes:?}"
        );
        let story = lifecycle
            .list_story_specs("project_0001", "issue_0001")
            .expect("story specs")
            .into_iter()
            .find(|spec| spec.id == "story_spec_0001")
            .expect("story spec entity");
        assert_eq!(
            story.confirmation_status,
            cadence_aria::product::models::LifecycleConfirmationStatus::Confirmed,
            "直接定稿必须把实体落 Confirmed"
        );

        drop(ws);
        server.abort();
    }
}

// F-31（P1 功能回归，v35 issue_0001/session_0001 现场锚；v36 纠正轮）：story/design 的
// AuthorConfirm 门确认通路是 HTTP confirm 端点（WS confirm 帧被矩阵拒收）。C3-T5 删除
// handle_author_decision 时连带删掉了「确认后触发 review」（原实现：reviewer_enabled_at_start
// 命中 → complete node + start_review），F-20 只补偿了确认通道，未补偿 review 触发——
// review 启用（review_rounds>0 && reviewer_provider.is_some()）的 story 会话确认后
// 直接定稿，ReviewerRun 从不创建（v35 时间线只有 start_generation/author_run/author_confirm
// 三节点）。F-31 v36 修复曾走向过度实现：confirm 无条件接管进 Review；纠正轮改为用户可选——
// 请求带 with_review=true 才进入 cross_review 并跑 ReviewOnly；review 完成回 AuthorConfirm，
// 再次确认（缺省 with_review）定稿。
#[tokio::test]
async fn http_confirm_on_story_author_confirm_starts_reviewer_then_finalizes() {
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
    let served_app = app.clone();
    let server = tokio::spawn(async move {
        axum::serve(listener, served_app).await.expect("serve");
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
                permission_modes:
                    cadence_aria::product::models::WorkspaceRolePermissionModes::default(),
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

    // 第一次确认（用户显式选择送审 with_review=true）：review 启用且本轮未评审 →
    // 进入 cross_review（不得直接定稿）。
    let (status, body) = request_json(
        app.clone(),
        Method::POST,
        "/api/workspace-sessions/workspace_session_0001/confirm",
        json!({"confirmed_by": "user", "with_review": true}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_ne!(
        body["status"], "confirmed",
        "review 启用的 story 会话确认不得直接定稿，got {body}"
    );

    assert_eq!(
        recv_until_stage(&mut ws, "cross_review").await,
        "cross_review"
    );
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
    // spec-design-dialog-revision T5：review pass 不自动定稿，统一回 AuthorConfirm。
    assert_eq!(
        recv_until_stage(&mut ws, "author_confirm").await,
        "author_confirm"
    );
    assert_eq!(
        reviewer_prompts.lock().unwrap().len(),
        1,
        "ReviewOnly provider run must be initiated after the first confirm"
    );

    // ReviewerRun 节点锚：durable 时间线必须留下评审轮节点（v35 缺此节点=本回归）。
    let lifecycle = LifecycleStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let nodes = lifecycle
        .load_timeline_nodes("workspace_session_0001")
        .expect("timeline nodes");
    assert!(
        nodes
            .iter()
            .any(|node| node.node_type == TimelineNodeType::ReviewerRun),
        "confirm 后必须创建 ReviewerRun 节点，got {nodes:?}"
    );

    // 第二次确认：本轮已评审 → 定稿。
    let (status, body) = request_json(
        app,
        Method::POST,
        "/api/workspace-sessions/workspace_session_0001/confirm",
        json!({"confirmed_by": "user"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "confirmed", "{body}");

    drop(ws);
    server.abort();
}

// F-31 反向锚：未启用 review（生成时 reviewer_enabled=false）的 story 会话确认必须
// 维持既有定稿通路——零 ReviewerRun 节点、零 provider run，直接 Confirmed。
// 纠正轮（红③载体）：显式 with_review=true 时须如实 4xx（见下），缺省确认仍直接定稿。
#[tokio::test]
async fn http_confirm_on_story_without_reviewer_finalizes_without_review_node() {
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
            ["reviewer must not run without review"],
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
    let served_app = app.clone();
    let server = tokio::spawn(async move {
        axum::serve(listener, served_app).await.expect("serve");
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
                permission_modes:
                    cadence_aria::product::models::WorkspaceRolePermissionModes::default(),
            },
            reviewer_enabled: false,
        },
    )
    .await;
    let _checkpoint = recv_until_message_complete(&mut ws).await;
    assert_eq!(
        recv_until_stage(&mut ws, "author_confirm").await,
        "author_confirm"
    );

    // F-31 纠正轮（红③）：review 未启用的会话显式要求送审（with_review=true）必须如实 4xx，
    // 不得静默按定稿——否则用户以为已送审而实际跳过了评审。
    let (status, body) = request_json(
        app.clone(),
        Method::POST,
        "/api/workspace-sessions/workspace_session_0001/confirm",
        json!({"confirmed_by": "user", "with_review": true}),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "review 未启用时 with_review=true 必须如实拒绝：{body}"
    );
    assert_eq!(
        body["code"], "workspace_session_review_not_enabled",
        "{body}"
    );
    assert!(
        reviewer_prompts.lock().unwrap().is_empty(),
        "被拒的送审请求不得启动 reviewer provider run"
    );
    let lifecycle_after_reject =
        LifecycleStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let rejected_session = lifecycle_after_reject
        .get_workspace_session("workspace_session_0001")
        .expect("workspace session");
    assert_ne!(
        rejected_session.status,
        cadence_aria::product::models::WorkspaceSessionStatus::Confirmed,
        "被拒的送审不得落定稿"
    );

    let (status, body) = request_json(
        app,
        Method::POST,
        "/api/workspace-sessions/workspace_session_0001/confirm",
        json!({"confirmed_by": "user"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "confirmed", "{body}");

    let lifecycle = LifecycleStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let nodes = lifecycle
        .load_timeline_nodes("workspace_session_0001")
        .expect("timeline nodes");
    assert!(
        nodes
            .iter()
            .all(|node| node.node_type != TimelineNodeType::ReviewerRun),
        "未启用 review 的确认不得创建 ReviewerRun 节点，got {nodes:?}"
    );
    assert!(
        reviewer_prompts.lock().unwrap().is_empty(),
        "未启用 review 的确认不得启动任何 reviewer provider run"
    );
    // 直接定稿：实体（Story Spec）确认状态落 Confirmed，而非停在评审前的等待态。
    let story = lifecycle
        .list_story_specs("project_0001", "issue_0001")
        .expect("story specs")
        .into_iter()
        .find(|spec| spec.id == "story_spec_0001")
        .expect("story spec entity");
    assert_eq!(
        story.confirmation_status,
        cadence_aria::product::models::LifecycleConfirmationStatus::Confirmed,
        "未启用 review 的确认必须直接定稿"
    );

    drop(ws);
    server.abort();
}

// F-31 fix round（k3 P1）：评审在途（stage=cross_review / durable=running）时的第二次 HTTP
// confirm 必须被拒。修复前端点无 stage/status 守卫：`begin_review_after_author_confirm` 因
// stage!=author_confirm 返 false 即落到 confirm_workspace_entity + update_status(Confirmed)——
// 评审未回门就定稿，迟到 review 报告再经 route_review_report_to_author_confirm 把已 Confirmed
// 会话拖回 author_confirm（Running→Confirmed→WaitingForHuman，与 F-30 终态口径相悖）。
// 触发面：双击确认 / 多 tab 在评审窗口内再确认。
#[tokio::test]
async fn http_confirm_during_cross_review_is_rejected_without_finalizing() {
    let root = tempdir().expect("root");
    create_workspace_session_fixture_with_providers(&root, "fake", "codex", 1).await;
    let author_prompts = Arc::new(Mutex::new(Vec::new()));
    let mut registry = ProviderRegistry::new();
    registry.register(
        ProviderName::Fake,
        Arc::new(ScriptedStreamingProvider::new(
            [VALID_STORY_SPEC],
            author_prompts,
        )),
    );
    // 评审 provider 挂起不返回：把第二次 confirm 钉在「评审在途」窗口内。
    registry.register(ProviderName::Codex, Arc::new(HangingStreamingProvider));
    let app = build_web_router(WebAppState::with_provider_registry(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
        registry,
    ));
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let served_app = app.clone();
    let server = tokio::spawn(async move {
        axum::serve(listener, served_app).await.expect("serve");
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
                permission_modes:
                    cadence_aria::product::models::WorkspaceRolePermissionModes::default(),
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

    // 第一次确认（用户显式选择送审 with_review=true）：进入 cross_review
    // （durable 状态同口径落 running）。
    let (status, body) = request_json(
        app.clone(),
        Method::POST,
        "/api/workspace-sessions/workspace_session_0001/confirm",
        json!({"confirmed_by": "user", "with_review": true}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["status"], "running",
        "首次确认进入评审，durable 状态须与 cross_review 口径一致：{body}"
    );
    assert_eq!(
        recv_until_stage(&mut ws, "cross_review").await,
        "cross_review"
    );

    // 评审窗口内的第二次确认：必须立即 409——既不阻塞（run 任务持引擎锁跑完整段
    // provider drive，阻塞等锁会把确认挂到评审结束后才静默定稿），也不落定稿写入。
    let (status, body) = timeout(
        Duration::from_secs(5),
        request_json(
            app,
            Method::POST,
            "/api/workspace-sessions/workspace_session_0001/confirm",
            json!({"confirmed_by": "user"}),
        ),
    )
    .await
    .expect("评审在途的 confirm 必须在守卫处立即拒收，不得阻塞等 run 释放引擎锁");
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "评审在途的确认必须被拒（不得绕过评审定稿）：{body}"
    );
    assert_eq!(
        body["code"], "workspace_session_confirm_not_allowed",
        "{body}"
    );
    assert_eq!(body["details"]["stage"], "running", "{body}");
    assert_eq!(body["details"]["status"], "running", "{body}");

    let lifecycle = LifecycleStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let session = lifecycle
        .get_workspace_session("workspace_session_0001")
        .expect("workspace session");
    assert_eq!(
        session.status,
        cadence_aria::product::models::WorkspaceSessionStatus::Running,
        "被拒的确认不得改写 durable 会话状态"
    );
    let story = lifecycle
        .list_story_specs("project_0001", "issue_0001")
        .expect("story specs")
        .into_iter()
        .find(|spec| spec.id == "story_spec_0001")
        .expect("story spec entity");
    assert_eq!(
        story.confirmation_status,
        cadence_aria::product::models::LifecycleConfirmationStatus::Draft,
        "被拒的确认不得越过评审直接把实体落 Confirmed"
    );
    let nodes = lifecycle
        .load_timeline_nodes("workspace_session_0001")
        .expect("timeline nodes");
    assert!(
        nodes
            .iter()
            .all(|node| node.node_type != TimelineNodeType::Completed),
        "评审在途的确认不得建 Completed 节点，got {nodes:?}"
    );
    assert!(
        nodes.iter().any(|node| {
            node.node_type == TimelineNodeType::ReviewerRun
                && node.status == TimelineNodeStatus::Active
        }),
        "拒收确认后评审 run 必须仍在途（评审不被绕过/中断），got {nodes:?}"
    );

    drop(ws);
    server.abort();
}

// F-31 fix round（k3 P2）：Fake reviewer 快速路径的二次 HTTP confirm 必须走引擎同一定稿实现。
// 修复前：首次确认被接管后 `start_review` 走 Skipped 快速路径落 HumanConfirm
// （durable=waiting_for_human），第二次确认因 stage!=author_confirm 返 false 而落到 store-only
// 定稿——不经引擎 `finalize_current_artifact`，缺 `mark_latest_artifact_confirmed`
//（产物 confirmed_by 为空）与 Completed 节点，引擎 stage 停留门态，durable 时间线与引擎投影分叉。
#[tokio::test]
async fn http_confirm_after_fake_reviewer_skip_finalizes_through_engine() {
    let root = tempdir().expect("root");
    create_workspace_session_fixture_with_providers(&root, "fake", "fake", 1).await;
    let author_prompts = Arc::new(Mutex::new(Vec::new()));
    let mut registry = ProviderRegistry::new();
    registry.register(
        ProviderName::Fake,
        Arc::new(ScriptedStreamingProvider::new(
            [VALID_STORY_SPEC],
            author_prompts,
        )),
    );
    let app = build_web_router(WebAppState::with_provider_registry(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
        registry,
    ));
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let served_app = app.clone();
    let server = tokio::spawn(async move {
        axum::serve(listener, served_app).await.expect("serve");
    });

    let url = format!("ws://{addr}/api/workspace-sessions/workspace_session_0001/ws");
    let (mut ws, _) = connect_async(url).await.expect("connect ws");
    let _initial = recv_json(&mut ws).await;
    send_json(
        &mut ws,
        &WsInMessage::StartGeneration {
            provider_config: ProviderConfigSnapshot {
                author: ProviderName::Fake,
                reviewer: Some(ProviderName::Fake),
                review_rounds: 1,
                permission_modes:
                    cadence_aria::product::models::WorkspaceRolePermissionModes::default(),
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

    // 第一次确认（用户显式选择送审 with_review=true）：Fake reviewer 快速路径（Skipped）
    // 落 HumanConfirm。
    let (status, body) = request_json(
        app.clone(),
        Method::POST,
        "/api/workspace-sessions/workspace_session_0001/confirm",
        json!({"confirmed_by": "user", "with_review": true}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["status"], "waiting_for_human",
        "Fake reviewer 快速路径确认后停在人工确认门：{body}"
    );

    // 第二次确认：引擎同实现定稿（修复前 store-only，缺 confirmed_by / Completed 节点）。
    let (status, body) = request_json(
        app,
        Method::POST,
        "/api/workspace-sessions/workspace_session_0001/confirm",
        json!({"confirmed_by": "user"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "confirmed", "{body}");

    let lifecycle = LifecycleStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let versions = lifecycle
        .list_artifact_versions("workspace_session_0001")
        .expect("artifact versions");
    let current = versions
        .iter()
        .find(|version| version.is_current)
        .expect("current artifact version");
    assert_eq!(
        current.confirmed_by.as_deref(),
        Some("human"),
        "引擎定稿必须落 mark_latest_artifact_confirmed：{versions:?}"
    );
    let nodes = lifecycle
        .load_timeline_nodes("workspace_session_0001")
        .expect("timeline nodes");
    assert!(
        nodes
            .iter()
            .any(|node| node.node_type == TimelineNodeType::Completed),
        "引擎定稿必须建 Completed 节点，got {nodes:?}"
    );

    // stage 一致：引擎面收敛后的广播 session_state 必须落 completed（不得停在门态）。
    let mut confirmed_stage = None;
    for _ in 0..20 {
        let message = timeout(Duration::from_secs(5), ws.next())
            .await
            .expect("post-confirm frame timeout")
            .expect("post-confirm frame stream")
            .expect("post-confirm frame");
        let Message::Text(text) = message else {
            continue;
        };
        let value: Value = serde_json::from_str(&text).expect("post-confirm frame json");
        if value["type"] == "session_state" && value["session_status"] == "confirmed" {
            confirmed_stage = value["stage"].as_str().map(str::to_string);
            break;
        }
    }
    assert_eq!(
        confirmed_stage.as_deref(),
        Some("completed"),
        "定稿后的 session_state stage 必须与 Confirmed 同口径收敛为 completed"
    );

    drop(ws);
    server.abort();
}

// ── change plan-compile-gate-visibility（WP-B Task 2）─────────────────────────
// SC compile recovery action 的 WS 集成矩阵：REQ-CG-02「合法 recovery action 与
// 人工门命令并存」+「非 recovery 状态拒绝 recovery action」（spec
// openspec/changes/plan-compile-gate-visibility/specs/work-item-plan-conversational-gate）。
//
// fixture 形态与 `enter_work_item_plan_compile_recovery`
// （`src/product/workspace_engine/compile.rs:548`）落盘结果等价：会话为
// work_item_plan + single_candidate 流 + waiting_for_human，timeline 上唯一 active
// 的 `work_item_plan_compile_recovery` 节点，加一条 `RecoveryRequired` compile
// transaction。**真 compile 制品**（source/IR/mechanical-report/provenance 四 refs）
// 由 in-crate fixture（`workspace_engine::tests::single_candidate_recovery`）从
// outline/accepted-draft 现场生成，集成层无法复现，故 `continue` 的完整成功链由引擎
// 层用例（single_candidate_recovery.rs:727/:850/:1020、part_10）锁定；本文件的
// `continue` 行只钉「协议面放行 + 引擎 fail-closed 零副作用」。

const SC_RECOVERY_PLAN_ID: &str = "issue_work_item_plan_sc_recovery";
const SC_RECOVERY_COMPILE_ID: &str = "compile_recovery_0001";
const SC_RECOVERY_NODE_ID: &str = "timeline_node_compile_recovery";

struct ScCompileRecoveryFixture {
    session_id: String,
    app_paths: ProductAppPaths,
}

impl ScCompileRecoveryFixture {
    fn lifecycle(&self) -> LifecycleStore {
        LifecycleStore::new(self.app_paths.clone())
    }

    fn compile_transactions(
        &self,
    ) -> Vec<cadence_aria::product::models::WorkItemPlanCompileTransaction> {
        cadence_aria::product::work_item_plan_store::WorkItemPlanStore::new(self.app_paths.clone())
            .list_compile_transactions("project_0001", "issue_0001", SC_RECOVERY_PLAN_ID)
            .expect("list compile transactions")
    }

    fn timeline_nodes(&self) -> Vec<TimelineNode> {
        self.lifecycle()
            .load_timeline_nodes(&self.session_id)
            .expect("load timeline nodes")
    }

    fn session_status(&self) -> cadence_aria::product::models::WorkspaceSessionStatus {
        self.lifecycle()
            .get_workspace_session(&self.session_id)
            .expect("load workspace session")
            .status
    }
}

fn sc_recovery_node(
    node_id: &str,
    node_type: TimelineNodeType,
    stage: cadence_aria::web::workspace_ws_types::WorkspaceStage,
    status: TimelineNodeStatus,
) -> TimelineNode {
    TimelineNode {
        node_id: node_id.to_string(),
        node_type,
        agent: None,
        stage,
        round: None,
        status,
        title: "SC Final Compile".to_string(),
        summary: Some("fixture node".to_string()),
        started_at: "2026-09-22T00:00:00Z".to_string(),
        completed_at: None,
        duration_ms: None,
        artifact_ref: None,
        provider_config_snapshot: ProviderConfigSnapshot {
            author: ProviderName::Fake,
            reviewer: None,
            review_rounds: 0,
            permission_modes: cadence_aria::product::models::WorkspaceRolePermissionModes::default(
            ),
        },
        retry: None,
    }
}

/// 构造一个 SC WorkItemPlan 会话，铺上给定 timeline 节点（以及可选的
/// `RecoveryRequired` compile transaction）。
///
/// 前置复用 `create_workspace_session_fixture`：WS 会话工厂
/// （`WorkspaceSessionManager::create`）要求 project/repository/issue 三者可解析
/// （workspace context + `workspace_repository_for_session`），该夹具是既有集成层
/// 唯一已证实可达的铺法（part_04/part_08 同源）。
async fn create_sc_compile_recovery_fixture(
    root: &TempDir,
    nodes: Vec<TimelineNode>,
    session_status: cadence_aria::product::models::WorkspaceSessionStatus,
    recovery_transaction: bool,
) -> ScCompileRecoveryFixture {
    create_workspace_session_fixture(root).await;
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle = LifecycleStore::new(app_paths.clone());
    let plan = lifecycle
        .create_issue_work_item_plan(
            cadence_aria::product::lifecycle_store::CreateIssueWorkItemPlanInput {
                id: Some(SC_RECOVERY_PLAN_ID.to_string()),
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
        .expect("create SC work item plan");
    let session = lifecycle
        .create_workspace_session(
            cadence_aria::product::lifecycle_store::CreateWorkspaceSessionInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                entity_id: plan.id.clone(),
                workspace_type: cadence_aria::product::models::WorkspaceType::WorkItemPlan,
                author_provider: ProviderName::Fake,
                reviewer_provider: ProviderName::Fake,
                review_rounds: 0,
                superpowers_enabled: false,
                openspec_enabled: false,
                work_item_plan_options: Some(
                    cadence_aria::product::lifecycle_store::WorkItemPlanSessionOptions {
                        flow_kind: cadence_aria::product::work_item_plan_policy::WorkItemPlanFlowKind::SingleCandidate,
                        run_policy: cadence_aria::product::work_item_plan_policy::RunPolicy::Interactive,
                        rollout_snapshot: true,
                    },
                ),
            },
        )
        .expect("create SC work item plan session");
    let mut record = lifecycle
        .get_workspace_session(&session.id)
        .expect("load SC plan session");
    record.status = session_status;
    record.single_candidate_phase =
        Some(cadence_aria::product::models::SingleCandidatePhase::Approval);
    let session_path = app_paths
        .issue_root(&record.project_id, &record.issue_id)
        .join("workspace-sessions")
        .join(format!("{}.json", record.id));
    cadence_aria::product::json_store::write_json(&session_path, &record)
        .expect("persist SC plan session");
    lifecycle
        .save_timeline_nodes(&session.id, &nodes)
        .expect("persist fixture timeline");
    if recovery_transaction {
        let now = "2026-09-22T00:00:00Z".to_string();
        cadence_aria::product::work_item_plan_store::WorkItemPlanStore::new(app_paths.clone())
            .put_compile_transaction(&cadence_aria::product::models::WorkItemPlanCompileTransaction {
                compile_id: SC_RECOVERY_COMPILE_ID.to_string(),
                project_id: record.project_id.clone(),
                issue_id: record.issue_id.clone(),
                plan_id: plan.id.clone(),
                flow_kind: Some(
                    cadence_aria::product::work_item_plan_policy::WorkItemPlanFlowKind::SingleCandidate,
                ),
                source_revision_id: None,
                source_revision_ref: None,
                plan_candidate_ir_ref: None,
                mechanical_report_ref: None,
                publication_provenance_ref: None,
                publication_provenance_content_hash: None,
                generation_round_id: "generation_round_recovery".to_string(),
                outline_version_ref: "outline_version_recovery".to_string(),
                active_draft_ids: vec![],
                status: cadence_aria::product::models::WorkItemPlanCompileStatus::RecoveryRequired,
                plan_commit_state:
                    cadence_aria::product::models::WorkItemPlanCommitState::NotStarted,
                step_cursor: "compile_failed".to_string(),
                outline_to_work_item_id: std::collections::BTreeMap::new(),
                outline_to_verification_plan_id: std::collections::BTreeMap::new(),
                created_work_item_ids: vec![],
                created_verification_plan_ids: vec![],
                child_session_ids: vec![],
                validator_findings: vec![],
                abort_requested_at: None,
                failure_reason: Some("final compile failed".to_string()),
                previous_plan_snapshot: plan.clone(),
                created_at: now.clone(),
                updated_at: now,
                committed_at: None,
            })
            .expect("persist recovery compile transaction");
    }
    ScCompileRecoveryFixture {
        session_id: session.id,
        app_paths,
    }
}

async fn sc_recovery_ws_harness(
    root: &TempDir,
    session_id: &str,
) -> (
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    tokio::task::JoinHandle<()>,
) {
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });
    let (mut ws, _) = connect_async(format!(
        "ws://{addr}/api/workspace-sessions/{session_id}/ws"
    ))
    .await
    .expect("connect ws");
    let initial = recv_json(&mut ws).await;
    assert!(
        matches!(initial, WsOutMessage::SessionState { .. }),
        "unexpected initial frame: {initial:?}"
    );
    (ws, server)
}

/// 收帧到「结果信号」为止：拒绝路径等 `ProtocolError`，合法路径等首个业务帧
/// （stage_change / session_state）。两条路径都收到信号即返回，因此不依赖固定观察窗口。
async fn sc_recovery_await_outcome(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    expect_rejection: bool,
) -> WsOutMessage {
    for _ in 0..80 {
        let frame = recv_json(ws).await;
        match frame {
            WsOutMessage::ProtocolError { .. } if expect_rejection => return frame,
            WsOutMessage::ProtocolError { .. } => {
                panic!("recovery action must not be rejected, got {frame:?}")
            }
            WsOutMessage::Pong => {}
            other if !expect_rejection => return other,
            _ => {}
        }
    }
    panic!("no ws outcome observed (expect_rejection={expect_rejection})");
}

async fn sc_recovery_send_continue(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) {
    send_json(
        ws,
        &WsInMessage::WorkItemPlanCompileRecoveryAction {
            action:
                cadence_aria::web::workspace_ws_types::WorkItemPlanCompileRecoveryActionDto::Continue,
            reason: None,
        },
    )
    .await;
}

fn sc_recovery_assert_rejected_as_stage_error(frame: &WsOutMessage, expected_stage: &str) {
    match frame {
        WsOutMessage::ProtocolError {
            code,
            message,
            context,
        } => {
            assert_eq!(
                code, "INVALID_MESSAGE_FOR_STAGE",
                "非 HumanConfirm 阶段必须按阶段矩阵拒收（stage-specific protocol error）: {message}"
            );
            assert!(
                message.contains("work_item_plan_compile_recovery_action")
                    && message.contains(expected_stage),
                "拒绝必须可诊断到消息与阶段: {message}"
            );
            let context = context.as_ref().expect("stage rejection context");
            assert_eq!(context["stage"].as_str(), Some(expected_stage));
            assert_eq!(
                context["received"].as_str(),
                Some("work_item_plan_compile_recovery_action")
            );
        }
        other => panic!("expected stage-specific protocol error, got {other:?}"),
    }
}

fn sc_recovery_active_node_ids(nodes: &[TimelineNode]) -> Vec<String> {
    nodes
        .iter()
        .filter(|node| node.status == TimelineNodeStatus::Active)
        .map(|node| node.node_id.clone())
        .collect()
}

// 场景 1a（合法 recovery action：human_triage）：协议面放行 → 引擎按既有 recovery
// 语义执行 → 不新建第二个 compile，恢复节点收口、门回 human_confirm。
#[tokio::test]
async fn workspace_ws_sc_compile_recovery_human_triage_runs_through_engine() {
    use cadence_aria::product::models::{WorkItemPlanCompileStatus, WorkspaceSessionStatus};
    use cadence_aria::web::workspace_ws_types::{
        TimelineNodeStatus, TimelineNodeType, WorkItemPlanCompileRecoveryActionDto, WorkspaceStage,
    };

    let root = tempdir().expect("root");
    let fixture = create_sc_compile_recovery_fixture(
        &root,
        vec![sc_recovery_node(
            SC_RECOVERY_NODE_ID,
            TimelineNodeType::WorkItemPlanCompileRecovery,
            WorkspaceStage::HumanConfirm,
            TimelineNodeStatus::Active,
        )],
        WorkspaceSessionStatus::WaitingForHuman,
        true,
    )
    .await;
    let (mut ws, server) = sc_recovery_ws_harness(&root, &fixture.session_id).await;

    send_json(
        &mut ws,
        &WsInMessage::WorkItemPlanCompileRecoveryAction {
            action: WorkItemPlanCompileRecoveryActionDto::HumanTriage,
            reason: Some("需要人工整理依赖".to_string()),
        },
    )
    .await;
    let outcome = sc_recovery_await_outcome(&mut ws, false).await;
    assert!(
        !matches!(outcome, WsOutMessage::ProtocolError { .. }),
        "合法 SC recovery action 不得被协议面拒收: {outcome:?}"
    );

    // 引擎既有语义：同一 compile transaction 落原因（不新建第二个 compile），
    // 恢复节点收口，门回 human_confirm。
    let transactions = fixture.compile_transactions();
    assert_eq!(
        transactions.len(),
        1,
        "recovery 必须复用原 compile transaction，不得创建第二个 compile"
    );
    assert_eq!(
        transactions[0].status,
        WorkItemPlanCompileStatus::RecoveryRequired
    );
    assert_eq!(
        transactions[0].failure_reason.as_deref(),
        Some("需要人工整理依赖")
    );
    assert_eq!(transactions[0].step_cursor, "compile_failed");

    let nodes = fixture.timeline_nodes();
    let recovery = nodes
        .iter()
        .find(|node| node.node_id == SC_RECOVERY_NODE_ID)
        .expect("recovery node");
    assert_eq!(
        recovery.status,
        TimelineNodeStatus::Completed,
        "恢复节点必须在动作后收口"
    );
    assert!(
        nodes.iter().any(|node| {
            node.node_type == TimelineNodeType::HumanConfirm
                && node.status == TimelineNodeStatus::Active
        }),
        "恢复完成后必须回到 human_confirm 门: {nodes:?}"
    );
    assert_eq!(
        fixture.session_status(),
        WorkspaceSessionStatus::WaitingForHuman
    );

    drop(ws);
    server.abort();
}

// 场景 1b（合法 recovery action：abort_and_rollback）：同一合法门态下的回滚动作也
// 完整走引擎（事务落 Failed/rolled_back + previous plan 快照回写），证明放行不是
// 「静默吞掉」而是真接到既有执行面。
#[tokio::test]
async fn workspace_ws_sc_compile_recovery_abort_and_rollback_runs_through_engine() {
    use cadence_aria::product::models::{WorkItemPlanCompileStatus, WorkspaceSessionStatus};
    use cadence_aria::web::workspace_ws_types::{
        TimelineNodeStatus, TimelineNodeType, WorkItemPlanCompileRecoveryActionDto, WorkspaceStage,
    };

    let root = tempdir().expect("root");
    let fixture = create_sc_compile_recovery_fixture(
        &root,
        vec![sc_recovery_node(
            SC_RECOVERY_NODE_ID,
            TimelineNodeType::WorkItemPlanCompileRecovery,
            WorkspaceStage::HumanConfirm,
            TimelineNodeStatus::Active,
        )],
        WorkspaceSessionStatus::WaitingForHuman,
        true,
    )
    .await;
    let (mut ws, server) = sc_recovery_ws_harness(&root, &fixture.session_id).await;

    send_json(
        &mut ws,
        &WsInMessage::WorkItemPlanCompileRecoveryAction {
            action: WorkItemPlanCompileRecoveryActionDto::AbortAndRollback,
            reason: Some("放弃本次编译并回滚旧 Plan".to_string()),
        },
    )
    .await;
    let outcome = sc_recovery_await_outcome(&mut ws, false).await;
    assert!(
        !matches!(outcome, WsOutMessage::ProtocolError { .. }),
        "合法 SC recovery action 不得被协议面拒收: {outcome:?}"
    );

    let transactions = fixture.compile_transactions();
    assert_eq!(
        transactions.len(),
        1,
        "回滚不得创建第二个 compile transaction"
    );
    assert_eq!(transactions[0].status, WorkItemPlanCompileStatus::Failed);
    assert_eq!(transactions[0].step_cursor, "rolled_back");
    assert_eq!(
        transactions[0].failure_reason.as_deref(),
        Some("放弃本次编译并回滚旧 Plan")
    );
    assert!(
        fixture
            .lifecycle()
            .get_issue_work_item_plan("project_0001", "issue_0001", SC_RECOVERY_PLAN_ID)
            .is_ok(),
        "回滚必须把 previous plan 快照回写为现存 plan"
    );

    let nodes = fixture.timeline_nodes();
    assert_eq!(
        nodes
            .iter()
            .find(|node| node.node_id == SC_RECOVERY_NODE_ID)
            .expect("recovery node")
            .status,
        TimelineNodeStatus::Completed
    );
    assert!(
        nodes.iter().any(|node| {
            node.node_type == TimelineNodeType::HumanConfirm
                && node.status == TimelineNodeStatus::Active
        }),
        "回滚完成后必须回到 human_confirm 门: {nodes:?}"
    );

    drop(ws);
    server.abort();
}

// 场景 1c（continue）：协议面放行后由引擎裁决。集成层无法造出真 compile 制品
// （四 refs 由 in-crate fixture 生成），引擎在此 fail-closed——关键是收到的是
// 引擎错误码 `INVALID_COMPILE_RECOVERY_ACTION` 而非阶段错误，且零副作用。
#[tokio::test]
async fn workspace_ws_sc_compile_recovery_continue_reaches_engine_without_side_effects() {
    use cadence_aria::product::models::{WorkItemPlanCompileStatus, WorkspaceSessionStatus};
    use cadence_aria::web::workspace_ws_types::{
        TimelineNodeStatus, TimelineNodeType, WorkspaceStage,
    };

    let root = tempdir().expect("root");
    let fixture = create_sc_compile_recovery_fixture(
        &root,
        vec![sc_recovery_node(
            SC_RECOVERY_NODE_ID,
            TimelineNodeType::WorkItemPlanCompileRecovery,
            WorkspaceStage::HumanConfirm,
            TimelineNodeStatus::Active,
        )],
        WorkspaceSessionStatus::WaitingForHuman,
        true,
    )
    .await;
    let (mut ws, server) = sc_recovery_ws_harness(&root, &fixture.session_id).await;

    sc_recovery_send_continue(&mut ws).await;
    let outcome = sc_recovery_await_outcome(&mut ws, true).await;
    match &outcome {
        WsOutMessage::ProtocolError { code, message, .. } => {
            assert_eq!(
                code, "INVALID_COMPILE_RECOVERY_ACTION",
                "HumanConfirm + recovery 事实必须进入引擎裁决（而非阶段拒收）: {message}"
            );
            assert!(
                !message.contains("not allowed in stage"),
                "不得退化为阶段错误: {message}"
            );
            assert!(
                message.contains("source ref is missing"),
                "durable-only fixture 必须在引擎的单候选事务校验上 fail-closed: {message}"
            );
        }
        other => panic!("expected engine-level rejection, got {other:?}"),
    }

    // 零副作用：事务与 timeline 原样。
    let transactions = fixture.compile_transactions();
    assert_eq!(transactions.len(), 1, "fail-closed 不得创建第二个 compile");
    assert_eq!(
        transactions[0].status,
        WorkItemPlanCompileStatus::RecoveryRequired
    );
    assert_eq!(transactions[0].step_cursor, "compile_failed");
    assert_eq!(
        transactions[0].failure_reason.as_deref(),
        Some("final compile failed")
    );
    let nodes = fixture.timeline_nodes();
    assert_eq!(
        sc_recovery_active_node_ids(&nodes),
        vec![SC_RECOVERY_NODE_ID.to_string()],
        "fail-closed 不得改动 timeline: {nodes:?}"
    );

    drop(ws);
    server.abort();
}

// 场景 2/3/4（非 recovery 状态拒绝）：generate(Running) / 普通 HumanConfirm（缺
// recovery 事实）/ 终态 Confirmed 三类都必须零副作用拒收。
#[tokio::test]
async fn workspace_ws_compile_recovery_action_rejected_outside_recovery_state() {
    use cadence_aria::product::models::WorkspaceSessionStatus;
    use cadence_aria::web::workspace_ws_types::{
        TimelineNodeStatus, TimelineNodeType, WorkspaceStage,
    };

    // 非 recovery 状态的三类代表行：(1) 非门阶段（生成前 prepare_context）、
    // (2) AuthorConfirm 批次门、(3) 终态 Completed——矩阵均不放行 recovery action，
    // 必须零副作用拒收。
    //
    // 说明：Running/CrossReview/Revision 等 in-flight 阶段无法由 durable fixture 铺出
    // ——manager 创建期的 F-23 僵尸整流（`recover_stale_run_if_zombie`）会把「无活跃
    // run 却在 Running/CrossReview/Revision」的会话整回 prepare_context，故此处以
    // prepare_context 作非门阶段代表；in-flight 阶段的矩阵拒绝（返回 false）由 Task 1
    // 单测 `sc_human_confirm_accepts_compile_recovery_action_only_in_recovery_stage`
    // 逐阶段钉住。
    for (node_id, node_type, node_status, ws_stage, session_status, expected_stage) in [
        (
            "timeline_node_prepare_context",
            TimelineNodeType::PrepareContext,
            TimelineNodeStatus::Active,
            WorkspaceStage::PrepareContext,
            WorkspaceSessionStatus::Open,
            "prepare_context",
        ),
        (
            "timeline_node_batch_confirm",
            TimelineNodeType::WorkItemBatchConfirm,
            TimelineNodeStatus::Active,
            WorkspaceStage::AuthorConfirm,
            WorkspaceSessionStatus::WaitingForHuman,
            "author_confirm",
        ),
        (
            "timeline_node_completed",
            TimelineNodeType::Completed,
            TimelineNodeStatus::Completed,
            WorkspaceStage::Completed,
            WorkspaceSessionStatus::Confirmed,
            "completed",
        ),
    ] {
        let root = tempdir().expect("root");
        let fixture = create_sc_compile_recovery_fixture(
            &root,
            vec![sc_recovery_node(node_id, node_type, ws_stage, node_status)],
            session_status.clone(),
            false,
        )
        .await;
        let (mut ws, server) = sc_recovery_ws_harness(&root, &fixture.session_id).await;

        sc_recovery_send_continue(&mut ws).await;
        let outcome = sc_recovery_await_outcome(&mut ws, true).await;
        sc_recovery_assert_rejected_as_stage_error(&outcome, expected_stage);

        assert!(
            fixture.compile_transactions().is_empty(),
            "阶段拒收不得创建 compile transaction"
        );
        let nodes = fixture.timeline_nodes();
        assert_eq!(
            nodes.len(),
            1,
            "阶段拒收必须零副作用（timeline 不变）: {nodes:?}"
        );
        assert_eq!(fixture.session_status(), session_status);

        drop(ws);
        server.abort();
    }
}

// 场景 3（普通 HumanConfirm 门：无 recovery 事实）：矩阵放行（SC HumanConfirm 臂）
// → 引擎 durable 守卫 fail-closed `INVALID_COMPILE_RECOVERY_ACTION`，零副作用。
#[tokio::test]
async fn workspace_ws_human_confirm_without_recovery_fact_rejects_recovery_action() {
    use cadence_aria::product::models::WorkspaceSessionStatus;
    use cadence_aria::web::workspace_ws_types::{
        TimelineNodeStatus, TimelineNodeType, WorkspaceStage,
    };

    let root = tempdir().expect("root");
    let fixture = create_sc_compile_recovery_fixture(
        &root,
        vec![sc_recovery_node(
            "timeline_node_human_confirm",
            TimelineNodeType::HumanConfirm,
            WorkspaceStage::HumanConfirm,
            TimelineNodeStatus::Active,
        )],
        WorkspaceSessionStatus::WaitingForHuman,
        false,
    )
    .await;
    let (mut ws, server) = sc_recovery_ws_harness(&root, &fixture.session_id).await;

    sc_recovery_send_continue(&mut ws).await;
    let outcome = sc_recovery_await_outcome(&mut ws, true).await;
    match &outcome {
        WsOutMessage::ProtocolError { code, message, .. } => {
            assert_eq!(code, "INVALID_COMPILE_RECOVERY_ACTION");
            assert!(
                message.contains("requires active work_item_plan_compile_recovery node"),
                "缺 recovery 事实必须由引擎 durable 守卫拒收: {message}"
            );
        }
        other => panic!("expected engine-level rejection, got {other:?}"),
    }

    // 零副作用：无新节点、门/阶段不变、无 compile transaction、无子会话。
    let nodes = fixture.timeline_nodes();
    assert_eq!(nodes.len(), 1, "零副作用（timeline 不变）: {nodes:?}");
    assert_eq!(
        fixture.session_status(),
        WorkspaceSessionStatus::WaitingForHuman
    );
    assert!(fixture.compile_transactions().is_empty());
    let child_sessions: Vec<_> = fixture
        .lifecycle()
        .list_workspace_sessions("project_0001", "issue_0001")
        .expect("list workspace sessions")
        .into_iter()
        .filter(|session| {
            session.workspace_type == cadence_aria::product::models::WorkspaceType::WorkItem
        })
        .collect();
    assert!(
        child_sessions.is_empty(),
        "零副作用：不得创建子 WorkItem 会话: {child_sessions:?}"
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
    fs::write(
        &session_path,
        serde_json::to_string_pretty(&session).unwrap(),
    )
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
