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

// F-31（P1 功能回归，v35 issue_0001/session_0001 现场锚）：story/design 的 AuthorConfirm
// 门确认通路是 HTTP confirm 端点（WS confirm 帧被矩阵拒收）。C3-T5 删除
// handle_author_decision 时连带删掉了「确认后触发 review」（原实现：reviewer_enabled_at_start
// 命中 → complete node + start_review），F-20 只补偿了确认通道，未补偿 review 触发——
// review 启用（review_rounds>0 && reviewer_provider.is_some()）的 story 会话确认后
// 直接定稿，ReviewerRun 从不创建（v35 时间线只有 start_generation/author_run/author_confirm
// 三节点）。修复后：确认时若 review 启用且本轮产物尚未评审 → 进入 cross_review 并跑
// ReviewOnly；review 完成回 AuthorConfirm，再次确认才定稿。
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

    // 第一次确认：review 启用且本轮未评审 → 进入 cross_review（不得直接定稿）。
    let (status, body) = request_json(
        app.clone(),
        Method::POST,
        "/api/workspace-sessions/workspace_session_0001/confirm",
        json!({"confirmed_by": "user"}),
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

    // 第一次确认：review 未跑 → 接管本轮进入 cross_review（durable 状态同口径落 running）。
    let (status, body) = request_json(
        app.clone(),
        Method::POST,
        "/api/workspace-sessions/workspace_session_0001/confirm",
        json!({"confirmed_by": "user"}),
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

    // 第一次确认：review 未跑 → Fake reviewer 快速路径（Skipped）落 HumanConfirm。
    let (status, body) = request_json(
        app.clone(),
        Method::POST,
        "/api/workspace-sessions/workspace_session_0001/confirm",
        json!({"confirmed_by": "user"}),
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
