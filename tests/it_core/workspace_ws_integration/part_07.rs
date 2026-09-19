// T5 Step 2（REQ-WSC-08 scenario「已删除消息协议错误拒绝」/REQ-RET-03 scenario
// 「在途 legacy 会话决策拒绝」）：退役消息族经真实 socket 发送 → stage-specific
// protocol error（LEGACY_MESSAGE_RETIRED，含 stage 上下文）且零副作用（会话
// 状态与事件流不变）。红测先行：本文件先于 in_.rs 删除落地（彼时消息仍被接受，
// 断言失败=红）；变体删除后 parse 面自然拒收 → 绿。
use super::*;

#[tokio::test]
async fn retired_legacy_decision_messages_receive_stage_specific_protocol_error_without_side_effects(
) {
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

    let (mut ws, _) =
        connect_async(format!("ws://{addr}/api/workspace-sessions/workspace_session_0001/ws"))
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
        let Message::Text(text) = message else { continue };
        let value: Value = serde_json::from_str(&text).expect("initial message json");
        if value["type"] == "session_state" {
            initial_stage = value["stage"].as_str().map(str::to_string);
            break;
        }
    }
    let baseline_stage = initial_stage.expect("session_state stage before retired sends");

    let retired_payloads: [(&str, &str); 10] = [
        ("review_decision_response", r#"{"type":"review_decision_response","decision":"continue","extra_context":null}"#),
        ("author_decision", r#"{"type":"author_decision","decision":"accept"}"#),
        ("select_work_item_generation_mode", r#"{"type":"select_work_item_generation_mode","mode":"serial"}"#),
        ("select_revision_path", r#"{"type":"select_revision_path","path":"revise","extra_context":null}"#),
        ("request_outline_revision", r#"{"type":"request_outline_revision","feedback":"revise outline"}"#),
        ("work_item_draft_decision", r#"{"type":"work_item_draft_decision","outline_id":"outline_0001","decision":"accept","feedback":null}"#),
        ("work_item_batch_decision", r#"{"type":"work_item_batch_decision","decision":"accept_all","feedback":null,"first_affected_outline_id":null}"#),
        ("save_human_presentation_revision", r#"{"type":"save_human_presentation_revision","source_projection_bundle_id":"bundle_0001","scope":"plan","supersedes":null,"human_summary":"summary","why_split":null,"dependency_explanation":[],"risk_explanation":[],"source_refs":[]}"#),
        ("human_confirm", r#"{"type":"human_confirm","decision":"confirm","payload":null}"#),
        ("revert_work_item", r#"{"type":"revert_work_item","work_item_id":"work_item_0001","feedback":null,"clear":false}"#),
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
            let Message::Text(text) = message else { continue };
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
        let Message::Text(text) = message else { continue };
        let value: Value = serde_json::from_str(&text).expect("pong json");
        if value["type"] == "pong" {
            pong = true;
            break;
        }
        if value["type"] == "session_state" {
            let stage = value["stage"].as_str().expect("stage");
            assert_eq!(stage, baseline_stage, "stage must not drift after retired sends");
        }
    }
    assert!(pong, "connection must stay alive after retired sends");
}
