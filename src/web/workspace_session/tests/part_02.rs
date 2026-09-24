// ---------------------------------------------------------------------------
// F-24（choice 卡不送达用户）：挂起 provider choice 的可靠重投面
// ---------------------------------------------------------------------------

fn provider_choice_frame(id: &str) -> crate::web::workspace_ws_types::WsOutMessage {
    use crate::web::workspace_ws_types::{ChoiceOption, WsOutMessage};

    WsOutMessage::ChoiceRequest {
        id: id.to_string(),
        prompt: "验收口径歧义需要用户裁定".to_string(),
        options: vec![ChoiceOption {
            id: "option-a".to_string(),
            label: "按全局口径".to_string(),
            description: None,
        }],
        allow_multiple: false,
        allow_free_text: false,
        questions: Vec::new(),
        source: "provider_choice".to_string(),
    }
}

fn frames_contain_choice(frames: &[String], id: &str) -> bool {
    frames.iter().any(|json| {
        let value: serde_json::Value =
            serde_json::from_str(json).unwrap_or(serde_json::Value::Null);
        value["type"] == "choice_request" && value["id"] == id
    })
}

/// F-24 现场锚（0484）：满队列 attachment 降级期间广播的 choice 帧被 try_send 丢弃，
/// 恢复只补 session_state 基线（不含 choice）→ 用户全程在线也永远看不到卡。
/// 修复后：degraded 恢复必须在基线之外补发挂起 choice 帧。
#[tokio::test]
async fn degraded_attachment_recovery_redelivers_pending_provider_choice() {
    use crate::web::workspace_ws_types::WsProviderStatus;

    let manager = WorkspaceSessionManager::test_fixture("session_choice_degraded_redelivery");
    let (slow_tx, mut slow_rx) = mpsc::channel(1);
    manager.attach("slow", slow_tx).await;
    manager
        .broadcast_test_event(WsProviderStatus::Starting)
        .await;
    manager
        .broadcast_test_event(WsProviderStatus::Running)
        .await;
    assert!(
        manager.attachment_is_degraded("slow"),
        "满队列 attachment 必须被标为 degraded"
    );

    manager
        .start_run(ProviderRunKind::ReviewOnly, None)
        .await
        .expect("active run for pending choice");
    manager.register_pending_choice_frame(provider_choice_frame("choice_degraded_1"));

    let _first = slow_rx.recv().await.expect("first queued event");
    manager
        .broadcast_test_event(WsProviderStatus::Completed)
        .await;
    assert!(!manager.attachment_is_degraded("slow"));

    let mut saw_baseline = false;
    let mut saw_choice = false;
    for _ in 0..4 {
        match tokio::time::timeout(std::time::Duration::from_millis(500), slow_rx.recv()).await {
            Ok(Some(OutboundControl::Text(json))) => {
                let value: serde_json::Value =
                    serde_json::from_str(&json).expect("recovery frame JSON");
                match value["type"].as_str() {
                    Some("session_state") => saw_baseline = true,
                    Some("choice_request") if value["id"] == "choice_degraded_1" => {
                        saw_choice = true;
                    }
                    _ => {}
                }
            }
            _ => break,
        }
    }
    assert!(saw_baseline, "degraded 恢复必须先发 session_state 基线");
    assert!(
        saw_choice,
        "degraded 恢复必须补发挂起 choice 卡（F-24：卡片丢失即 run 永久楔死）"
    );
}

/// F-24：初帧激活与 cursor 重订阅（snapshot 分支）都必须补发活跃 run 的挂起
/// choice——`pending_author_choice_request_message` 只覆盖 TextFallback 面。
#[tokio::test]
async fn attach_and_cursor_resubscribe_redeliver_pending_provider_choices() {
    let manager = WorkspaceSessionManager::test_fixture("session_choice_attach_redelivery");
    manager
        .start_run(ProviderRunKind::ReviewOnly, None)
        .await
        .expect("active run");
    manager.register_pending_choice_frame(provider_choice_frame("choice_attach_1"));

    let (session_state, _text_fallback) = manager.attached_session_state().await;
    let frames = manager.activate_attachment_with_initial_frames(
        "conn-attach",
        session_state,
        None,
        manager.attach_baseline_seq(),
    );
    assert!(
        frames_contain_choice(&frames, "choice_attach_1"),
        "初帧激活必须补发挂起 provider choice：{frames:?}"
    );

    let (cursor_tx, mut cursor_rx) = mpsc::channel(16);
    manager.register_attachment("conn-cursor", cursor_tx.clone());
    manager.resubscribe(&cursor_tx, "conn-cursor", 9_999).await;
    let mut resubscribed = Vec::new();
    while let Ok(control) = cursor_rx.try_recv() {
        let OutboundControl::Text(json) = control else {
            panic!("resubscribe must only send text frames");
        };
        resubscribed.push(json);
    }
    assert!(
        frames_contain_choice(&resubscribed, "choice_attach_1"),
        "cursor snapshot 重订阅必须补发挂起 provider choice：{resubscribed:?}"
    );
}

