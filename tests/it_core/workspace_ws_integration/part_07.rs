// T5 Step 2（REQ-WSC-08 scenario「已删除消息协议错误拒绝」/REQ-RET-03 scenario
// 「在途 legacy 会话决策拒绝」）：退役消息族经真实 socket 发送 → stage-specific
// protocol error（LEGACY_MESSAGE_RETIRED，含 stage 上下文）且零副作用（会话
// 状态与事件流不变）。红测先行：本文件先于 in_.rs 删除落地（彼时消息仍被接受，
// 断言失败=红）；变体删除后 parse 面自然拒收 → 绿。

#[tokio::test]
async fn retired_legacy_decision_messages_receive_stage_specific_protocol_error_without_side_effects()
 {
    let root = tempdir().expect("tempdir");
    let _repo = create_workspace_session_fixture(&root).await;

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let serve_root = root.path().to_path_buf();
    tokio::spawn(async move {
        let app = build_web_router(WebAppState::new(
            serve_root.clone(),
            WebRuntime::new_fake(serve_root),
        ));
        axum::serve(listener, app).await.expect("serve");
    });

    let (mut ws, _) = connect_async(format!(
        "ws://{addr}/api/workspace-sessions/workspace_session_0001/ws"
    ))
    .await
    .expect("connect workspace ws");
    // 吸收连接建立期的 session_state 推送，取其 stage 作零副作用基线。
    let mut initial_stage = None;
    for _ in 0..20 {
        let message = timeout(Duration::from_secs(5), ws.next())
            .await
            .expect("initial message timeout")
            .expect("initial message stream")
            .expect("initial message");
        let Message::Text(text) = message else {
            continue;
        };
        let value: Value = serde_json::from_str(&text).expect("initial message json");
        if value["type"] == "session_state" {
            initial_stage = value["stage"].as_str().map(str::to_string);
            break;
        }
    }
    let baseline_stage = initial_stage.expect("session_state stage before retired sends");

    let retired_payloads: [(&str, &str); 10] = [
        (
            "review_decision_response",
            r#"{"type":"review_decision_response","decision":"continue","extra_context":null}"#,
        ),
        (
            "author_decision",
            r#"{"type":"author_decision","decision":"accept"}"#,
        ),
        (
            "select_work_item_generation_mode",
            r#"{"type":"select_work_item_generation_mode","mode":"serial"}"#,
        ),
        (
            "select_revision_path",
            r#"{"type":"select_revision_path","path":"revise","extra_context":null}"#,
        ),
        (
            "request_outline_revision",
            r#"{"type":"request_outline_revision","feedback":"revise outline"}"#,
        ),
        (
            "work_item_draft_decision",
            r#"{"type":"work_item_draft_decision","outline_id":"outline_0001","decision":"accept","feedback":null}"#,
        ),
        (
            "work_item_batch_decision",
            r#"{"type":"work_item_batch_decision","decision":"accept_all","feedback":null,"first_affected_outline_id":null}"#,
        ),
        (
            "save_human_presentation_revision",
            r#"{"type":"save_human_presentation_revision","source_projection_bundle_id":"bundle_0001","scope":"plan","supersedes":null,"human_summary":"summary","why_split":null,"dependency_explanation":[],"risk_explanation":[],"source_refs":[]}"#,
        ),
        (
            "human_confirm",
            r#"{"type":"human_confirm","decision":"confirm","payload":null}"#,
        ),
        (
            "revert_work_item",
            r#"{"type":"revert_work_item","work_item_id":"work_item_0001","feedback":null,"clear":false}"#,
        ),
    ];

    for (wire_name, raw) in retired_payloads {
        let before_messages = persisted_workspace_messages(root.path());
        ws.send(Message::Text(raw.to_string().into()))
            .await
            .expect("send retired message");

        let mut protocol_error = None;
        for _ in 0..20 {
            let message = timeout(Duration::from_secs(5), ws.next())
                .await
                .expect("retired response timeout")
                .expect("retired response stream")
                .expect("retired response");
            let Message::Text(text) = message else {
                continue;
            };
            let value: Value = serde_json::from_str(&text).expect("retired response json");
            if value["type"] == "protocol_error" && value["code"] == "LEGACY_MESSAGE_RETIRED" {
                protocol_error = Some(value);
                break;
            }
            if value["type"] == "error" {
                panic!(
                    "retired message {wire_name} must produce stage-specific protocol error, got generic error: {value}"
                );
            }
        }
        let error = protocol_error
            .unwrap_or_else(|| panic!("retired message {wire_name} must be rejected"));
        assert_eq!(error["code"], "LEGACY_MESSAGE_RETIRED", "{wire_name}");
        assert_eq!(error["context"]["received"], wire_name, "{wire_name}");
        assert!(
            error["context"]["stage"].as_str().is_some(),
            "{wire_name}: protocol error must carry stage context"
        );

        // 零副作用：durable 会话消息零新增（事件流不变）。
        let after_messages = persisted_workspace_messages(root.path());
        assert_eq!(
            after_messages.len(),
            before_messages.len(),
            "{wire_name}: retired message must not append workspace messages"
        );
    }

    // 零副作用终检：stage 不因退役消息漂移（连接保持可继续应答 ping）。
    ws.send(Message::Text(r#"{"type":"ping"}"#.into()))
        .await
        .expect("ping after retired sends");
    let mut pong = false;
    for _ in 0..20 {
        let message = timeout(Duration::from_secs(5), ws.next())
            .await
            .expect("pong timeout")
            .expect("pong stream")
            .expect("pong");
        let Message::Text(text) = message else {
            continue;
        };
        let value: Value = serde_json::from_str(&text).expect("pong json");
        if value["type"] == "pong" {
            pong = true;
            break;
        }
        if value["type"] == "session_state" {
            let stage = value["stage"].as_str().expect("stage");
            assert_eq!(
                stage, baseline_stage,
                "stage must not drift after retired sends"
            );
        }
    }
    assert!(pong, "connection must stay alive after retired sends");
}

// F-25b（0497 现场锚）：story/design AuthorConfirm 的确认通路是 HTTP confirm 端点
// （WS confirm 帧被矩阵拒收），此前端点只写 durable——已连接的其他 tab/重连连接
// 只能整页 reload 才能看到 confirmed，会话页动作条与门卡停在「等待确认产物」。
// 修复后：confirm 完成 durable 写即向全部 attachment 广播含 session_status=
// confirmed 的全量 session_state（对照 F-09 HumanGateOpened 广播先例）。
#[tokio::test]
async fn http_confirm_broadcasts_confirmed_session_state_to_connected_ws() {
    let root = tempdir().expect("root");
    let _repo = create_workspace_session_fixture(&root).await;
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let serve_app = app.clone();
    let server = tokio::spawn(async move {
        axum::serve(listener, serve_app).await.expect("serve");
    });

    let (mut ws, _) = connect_async(format!(
        "ws://{addr}/api/workspace-sessions/workspace_session_0001/ws"
    ))
    .await
    .expect("connect workspace ws");
    // 吸收连接建立期的初帧，确认基线尚未 confirmed。
    let mut baseline_status = None;
    for _ in 0..20 {
        let message = timeout(Duration::from_secs(5), ws.next())
            .await
            .expect("initial message timeout")
            .expect("initial message stream")
            .expect("initial message");
        let Message::Text(text) = message else {
            continue;
        };
        let value: Value = serde_json::from_str(&text).expect("initial message json");
        if value["type"] == "session_state" {
            baseline_status = value["session_status"].as_str().map(str::to_string);
            break;
        }
    }
    assert_ne!(
        baseline_status.as_deref(),
        Some("confirmed"),
        "fixture session must start unconfirmed"
    );

    // HTTP confirm 打在同一 state（同 registry/同 manager）上。
    let (status, body) = request_json(
        app,
        Method::POST,
        "/api/workspace-sessions/workspace_session_0001/confirm",
        json!({"confirmed_by": "user"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let mut confirmed_frame = None;
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
            confirmed_frame = Some(value);
            break;
        }
    }
    let frame = confirmed_frame.expect(
        "HTTP confirm must broadcast a session_state frame with session_status=confirmed to the connected WS",
    );
    // fix round 1（k3 P2）：stage 必须随 status 一致收敛——Confirmed 会话的内存
    // stage 同步为 completed（与 from_record→workspace_stage_for_status 投影一致），
    // 否则已连接 tab 收到 confirmed+author_confirm 组合而重连 tab 看到 completed，
    // 两 tab 投影互相矛盾。
    assert_eq!(
        frame["stage"], "completed",
        "confirmed session_state must carry stage=completed"
    );
    // 审计消息同样随帧投影（其他 tab 无需 reload 即可看到确认审计行）。
    assert!(
        frame["messages"]
            .as_array()
            .expect("session_state messages")
            .iter()
            .any(|message| message["content"]
                .as_str()
                .is_some_and(|content| content.contains("确认当前 Workspace 产物"))),
        "broadcast session_state must carry the confirm audit message"
    );

    drop(ws);
    server.abort();
}
