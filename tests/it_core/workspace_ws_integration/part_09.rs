// v38 复验 #2/#3（恢复 C3 前原意，spec-design-dialog-revision T3 Revise 语义）：
// story/design 在 AuthorConfirm 门上提交修订意见（RequestRevision）→ 引擎进入
// Revision 阶段并发起修订 run（author 反馈修订 prompt 携带反馈全文）→ 完成后
// 回 AuthorConfirm 门（可再评审/确认定稿）。
// 修复前红锚：socket 矩阵把非 WorkItemPlan 的 author_confirm RequestRevision
// 拒为 INVALID_MESSAGE_FOR_STAGE；即使放行，inbound 非 WIP 臂也回
// REQUEST_REVISION_WORKSPACE_INVALID——本测试两条都必须不再发生。

/// 持续读到 stage_change==expected，途中任何 protocol/error 帧立即失败
/// （红测锚：修复前的 INVALID_MESSAGE_FOR_STAGE / REQUEST_REVISION_WORKSPACE_INVALID
/// 都必须在此立刻暴露，而不是等 stage 超时）。
async fn recv_stage_fail_on_protocol_error(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    expected: &str,
) -> String {
    for _ in 0..600 {
        match recv_json(ws).await {
            WsOutMessage::StageChange { stage } if stage == expected => return stage,
            WsOutMessage::ProtocolError { code, message, .. } => {
                panic!("protocol error while waiting for stage {expected}: {code}: {message}")
            }
            WsOutMessage::Error { message } => {
                panic!("ws error while waiting for stage {expected}: {message}")
            }
            _ => {}
        }
    }
    panic!("stage_change {expected} not received");
}

#[tokio::test]
async fn story_author_confirm_request_revision_runs_revision_and_returns_to_gate() {
    let root = tempdir().expect("root");
    create_workspace_session_fixture(&root).await;
    let author_prompts = Arc::new(Mutex::new(Vec::new()));
    let mut registry = ProviderRegistry::new();
    registry.register(
        ProviderName::Fake,
        Arc::new(ScriptedStreamingProvider::new(
            [VALID_STORY_SPEC, VALID_STORY_SPEC],
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
        recv_stage_fail_on_protocol_error(&mut ws, "author_confirm").await,
        "author_confirm"
    );

    // 门上提交修订意见（「采纳 Review 意见」预填后的发送语义）。
    send_json(
        &mut ws,
        &WsInMessage::RequestRevision {
            feedback: cadence_aria::web::workspace_ws_types::StructuredFeedback {
                feedback_types: vec!["revision".to_string()],
                description: "按以下 review 意见修订：\n\n第二段缺少冲突".to_string(),
                target_artifact_version: None,
            },
        },
    )
    .await;

    // 接受（非协议错误）→ 进入 Revision 阶段 → 修订 run 完成后回门。
    assert_eq!(
        recv_stage_fail_on_protocol_error(&mut ws, "revision").await,
        "revision"
    );
    assert_eq!(
        recv_stage_fail_on_protocol_error(&mut ws, "author_confirm").await,
        "author_confirm"
    );

    // 修订 run 的 prompt 必须是 author 反馈修订 prompt：携带反馈全文，
    // 且不含 reviewer 返修段（I-1：post-review 新反馈清空 verdict，走 T4 分流）。
    let prompts = author_prompts.lock().unwrap();
    assert!(
        prompts.len() >= 2,
        "author 生成 run 与修订 run 各一次，got {} prompts",
        prompts.len()
    );
    let revision_prompt = &prompts[1];
    assert!(
        revision_prompt.contains("第二段缺少冲突"),
        "修订 prompt 必须携带用户反馈全文"
    );
    assert!(
        !revision_prompt.contains("Reviewer 审核意见"),
        "无 verdict 的反馈修订必须走 author 反馈 prompt，而非 reviewer 返修 prompt"
    );
    drop(prompts);

    // durable 时间线：Revision 节点 + 反馈全文落盘节点 detail（T7 fix1 重连恢复语义）
    // + 门节点重建（回门）。
    let lifecycle = LifecycleStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let nodes = lifecycle
        .load_timeline_nodes("workspace_session_0001")
        .expect("timeline nodes");
    let revision_node = nodes
        .iter()
        .find(|node| node.node_type == TimelineNodeType::Revision)
        .expect("revision timeline node");
    assert_eq!(revision_node.status, TimelineNodeStatus::Completed);
    assert!(
        nodes
            .iter()
            .filter(|node| node.node_type == TimelineNodeType::AuthorConfirm)
            .count()
            >= 2,
        "修订完成后必须回 AuthorConfirm 门（第二个门节点），got {nodes:?}"
    );
    let detail = lifecycle
        .load_node_detail("workspace_session_0001", &revision_node.node_id)
        .expect("revision node detail");
    assert_eq!(
        detail.revision_feedback.as_deref(),
        Some("按以下 review 意见修订：\n\n第二段缺少冲突"),
        "反馈全文必须落盘节点 detail（断线重连 retry 重建 pending_revision_context）"
    );

    drop(ws);
    server.abort();
}

/// v38 复验 #2 完整用户链：author 生成 → 确认并评审 → review 回门（此时
/// latest_review_verdict=Some）→「采纳 Review 意见」预填后发送 RequestRevision。
/// I-1 语义锚：post-review 新反馈提交必须清空 verdict，修订 prompt 走 author
/// 反馈修订路径（携带反馈全文、无 reviewer 返修段），完成后回门。
#[tokio::test]
async fn post_review_request_revision_uses_author_feedback_prompt_not_reviewer_repair() {
    let root = tempdir().expect("root");
    create_workspace_session_fixture_with_providers(&root, "fake", "codex", 1).await;
    let author_prompts = Arc::new(Mutex::new(Vec::new()));
    let reviewer_prompts = Arc::new(Mutex::new(Vec::new()));
    let mut registry = ProviderRegistry::new();
    registry.register(
        ProviderName::Fake,
        Arc::new(ScriptedStreamingProvider::new(
            [VALID_STORY_SPEC, VALID_STORY_SPEC],
            author_prompts.clone(),
        )),
    );
    registry.register(
        ProviderName::Codex,
        Arc::new(ScriptedStreamingProvider::new(
            ["审核意见：第二段缺少冲突。\n```json\n{\"verdict\":\"revise\",\"summary\":\"需要返修\"}\n```"],
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
        recv_stage_fail_on_protocol_error(&mut ws, "author_confirm").await,
        "author_confirm"
    );

    // 确认并评审（F-31：HTTP confirm with_review=true 接管进 CrossReview）。
    let (status, body) = request_json(
        app.clone(),
        Method::POST,
        "/api/workspace-sessions/workspace_session_0001/confirm",
        json!({"confirmed_by": "user", "with_review": true}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        recv_stage_fail_on_protocol_error(&mut ws, "author_confirm").await,
        "author_confirm",
        "review 完成后必须回 AuthorConfirm 门（route_review_report_to_author_confirm）"
    );
    assert_eq!(
        reviewer_prompts.lock().unwrap().len(),
        1,
        "确认并评审必须发起一次 ReviewOnly run"
    );

    // 门上「采纳 Review 意见」→ 发送 RequestRevision（预填文本即反馈全文）。
    send_json(
        &mut ws,
        &WsInMessage::RequestRevision {
            feedback: cadence_aria::web::workspace_ws_types::StructuredFeedback {
                feedback_types: vec!["revision".to_string()],
                description: "按以下 review 意见修订：\n\n第二段缺少冲突".to_string(),
                target_artifact_version: None,
            },
        },
    )
    .await;
    assert_eq!(
        recv_stage_fail_on_protocol_error(&mut ws, "revision").await,
        "revision"
    );
    assert_eq!(
        recv_stage_fail_on_protocol_error(&mut ws, "author_confirm").await,
        "author_confirm",
        "修订完成后必须回门（可再评审/确认定稿）"
    );

    let prompts = author_prompts.lock().unwrap();
    let revision_prompt = &prompts[1];
    assert!(
        revision_prompt.contains("第二段缺少冲突"),
        "修订 prompt 必须携带用户反馈全文"
    );
    assert!(
        !revision_prompt.contains("Reviewer 审核意见"),
        "I-1：post-review 新反馈必须清空 verdict 走 author 反馈修订 prompt，而非 reviewer 返修 prompt"
    );

    drop(ws);
    server.abort();
}

/// 回归锚（WorkItemPlan plan-repair 现有放行不破坏）：非 story/design 的门上
/// RequestRevision 仍被拒——WorkItem 载荷的会话（非 WorkItemPlan）在
/// author_confirm 发 RequestRevision 必须收到错误而非进入修订。
/// 注：WorkItem 会话实际不落 author_confirm 门（无生成流），此处以
/// prepare_context 阶段的 story 会话为负例锚：非门阶段发 RequestRevision
/// 必须 INVALID_MESSAGE_FOR_STAGE（修订只在门上放行）。
#[tokio::test]
async fn request_revision_outside_author_confirm_gate_is_rejected() {
    let root = tempdir().expect("root");
    create_workspace_session_fixture(&root).await;
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let served_app = app.clone();
    let server = tokio::spawn(async move {
        axum::serve(listener, served_app).await.expect("serve");
    });

    let (mut ws, _) = connect_async(format!(
        "ws://{addr}/api/workspace-sessions/workspace_session_0001/ws"
    ))
    .await
    .expect("connect ws");
    let _initial = recv_json(&mut ws).await;

    // prepare_context（非门阶段）发 RequestRevision → 必须拒收且零副作用。
    send_json(
        &mut ws,
        &WsInMessage::RequestRevision {
            feedback: cadence_aria::web::workspace_ws_types::StructuredFeedback {
                feedback_types: vec!["revision".to_string()],
                description: "不该被接受的门外语义".to_string(),
                target_artifact_version: None,
            },
        },
    )
    .await;
    let mut rejected = false;
    for _ in 0..40 {
        match recv_json(&mut ws).await {
            WsOutMessage::ProtocolError { code, message, .. } => {
                assert_eq!(
                    code, "INVALID_MESSAGE_FOR_STAGE",
                    "门外 RequestRevision 必须按阶段矩阵拒收，got {code}: {message}"
                );
                rejected = true;
                break;
            }
            WsOutMessage::Error { message } => panic!("unexpected ws error: {message}"),
            _ => {}
        }
    }
    assert!(rejected, "门外 RequestRevision 必须收到 protocol error");
    // 零副作用：连接仍可应答 ping（stage 不漂移）。
    send_json(&mut ws, &WsInMessage::Ping).await;
    let mut pong = false;
    for _ in 0..20 {
        match recv_json(&mut ws).await {
            WsOutMessage::Pong => {
                pong = true;
                break;
            }
            _ => {}
        }
    }
    assert!(
        pong,
        "connection must stay alive after out-of-gate rejection"
    );

    drop(ws);
    server.abort();
}
