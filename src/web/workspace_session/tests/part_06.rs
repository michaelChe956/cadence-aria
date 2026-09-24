// ── change human-gate-termination-reliability（C3 Task 3）─────────────────────
// REQ-HTR-03：degraded 连接关键帧投递（有界等待 + resync_required）与
// degraded 转移 append-only 打点。V0 结论（f53-v0-frame-order.md）：try_send
// 满即丢 + 门开后引擎静默 = 连接 stale 至 reload——本组用例钉死两条恢复腿。

/// REQ-HTR-03 场景 1（等待式投递腿）：degraded 连接的关键帧不被静默吞——
/// 队列恢复容量后，即使引擎此后静默（无后续广播触发既有恢复机制），有界
/// 等待任务也必须把恢复基线送达该连接。
#[tokio::test]
async fn degraded_attachment_keyframe_recovers_via_bounded_waiting_delivery() {
    let manager = WorkspaceSessionManager::test_fixture("session_keyframe_bounded_wait");
    let (slow_tx, mut slow_rx) = mpsc::channel(1);
    manager.attach("slow", slow_tx).await;
    // 容量 1：首帧入队占满，第二帧触发降级（enter 打点）。
    manager
        .broadcast_test_event(crate::web::workspace_ws_types::WsProviderStatus::Starting)
        .await;
    manager
        .broadcast_test_event(crate::web::workspace_ws_types::WsProviderStatus::Running)
        .await;
    assert!(
        manager.attachment_is_degraded("slow"),
        "满队列 attachment 必须被标为 degraded"
    );

    // 关键帧到达：恢复 try_send 仍失败（队列满）→ 有界等待任务接管。
    manager.broadcast_test_keyframe(crate::web::workspace_ws_types::WsOutMessage::StageChange {
        stage: "human_confirm".to_string(),
    });

    // 客户端恢复读取：腾出队列容量——此后不再有任何新广播（引擎静默）。
    let first = slow_rx.recv().await.expect("queued provider_status frame");
    assert!(matches!(first, OutboundControl::Text(json) if json.contains("provider_status")));
    let recovered = tokio::time::timeout(std::time::Duration::from_secs(3), slow_rx.recv())
        .await
        .expect("关键帧的有界等待投递必须在引擎静默期送达恢复基线（不依赖后续广播）")
        .expect("attachment channel stays open");
    match recovered {
        OutboundControl::Text(json) => {
            let value: serde_json::Value = serde_json::from_str(&json).expect("baseline JSON");
            assert_eq!(
                value["type"], "session_state",
                "等待式投递送达的是恢复基线（全量 session_state）"
            );
            assert_eq!(value["event_seq"], 3, "基线必须携带关键帧广播的 event_seq");
        }
        other => panic!("unexpected outbound control: {other:?}"),
    }
}

/// REQ-HTR-03 场景 1（resync 腿）：持续满队列的关键帧在两次有界退避尝试
/// 后必须显式发出 resync_required——客户端收到后主动重连拉取全量，不出现
/// 「帧被吞且无任何信号」的 stale 停留。
#[tokio::test]
async fn degraded_attachment_keyframe_escalates_to_resync_required() {
    let manager = WorkspaceSessionManager::test_fixture("session_keyframe_resync");
    let (slow_tx, mut slow_rx) = mpsc::channel(1);
    manager.attach("slow", slow_tx).await;
    manager
        .broadcast_test_event(crate::web::workspace_ws_types::WsProviderStatus::Starting)
        .await;
    manager
        .broadcast_test_event(crate::web::workspace_ws_types::WsProviderStatus::Running)
        .await;
    assert!(manager.attachment_is_degraded("slow"));

    manager.broadcast_test_keyframe(crate::web::workspace_ws_types::WsOutMessage::StageChange {
        stage: "human_confirm".to_string(),
    });

    // 等待两次有界尝试（750ms×2）+ 退避（250ms）全部超时：resync 发送进入
    // 停驻等待容量。真实时间等待（~2.6s > 1.75s 上界 + 余量）。
    tokio::time::sleep(std::time::Duration::from_millis(2600)).await;

    // 腾出容量 → 停驻的 resync_required 必达（最后防线，等待不设界）。
    let _first = slow_rx.recv().await.expect("queued provider_status frame");
    let resync = tokio::time::timeout(std::time::Duration::from_secs(3), slow_rx.recv())
        .await
        .expect("有界等待失败后必须发出 resync_required（客户端拉取信号）")
        .expect("attachment channel stays open");
    match resync {
        OutboundControl::Text(json) => {
            let value: serde_json::Value = serde_json::from_str(&json).expect("resync JSON");
            assert_eq!(value["type"], "resync_required");
            assert_eq!(value["event_seq"], 3, "resync 携带关键帧广播的 event_seq");
        }
        other => panic!("expected resync_required, got {other:?}"),
    }
}

/// REQ-HTR-03 场景 2：degraded 转移打点（进入/退出/原因/帧序）落会话级
/// append-only jsonl；同一降级期只打一次 enter（高频轮询连接防打点风暴）。
#[tokio::test]
async fn degraded_transitions_are_recorded_to_append_only_diagnostics() {
    let session_id = "session_degraded_diagnostics";
    let diagnostics_path = std::env::temp_dir()
        .join(session_id)
        .join("degraded-diagnostics.jsonl");
    let _ = std::fs::remove_file(&diagnostics_path);

    let manager = WorkspaceSessionManager::test_fixture(session_id);
    let (slow_tx, mut slow_rx) = mpsc::channel(1);
    manager.attach("slow", slow_tx).await;
    manager
        .broadcast_test_event(crate::web::workspace_ws_types::WsProviderStatus::Starting)
        .await;
    manager
        .broadcast_test_event(crate::web::workspace_ws_types::WsProviderStatus::Running)
        .await;
    assert!(manager.attachment_is_degraded("slow"));
    // 已降级期间的再次广播（恢复尝试失败）不得重复打 enter。
    manager
        .broadcast_test_event(crate::web::workspace_ws_types::WsProviderStatus::Starting)
        .await;

    let _first = slow_rx.recv().await.expect("queued provider_status frame");
    manager
        .broadcast_test_event(crate::web::workspace_ws_types::WsProviderStatus::Completed)
        .await;
    assert!(
        !manager.attachment_is_degraded("slow"),
        "队列恢复容量后必须以基线恢复直播"
    );
    let _baseline = slow_rx.recv().await.expect("recovery baseline frame");

    let content =
        std::fs::read_to_string(&diagnostics_path).expect("degraded-diagnostics.jsonl 必须存在");
    let events: Vec<serde_json::Value> = content
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();
    let enters: Vec<&serde_json::Value> = events
        .iter()
        .filter(|event| event["event"] == "degraded_enter")
        .collect();
    let exits: Vec<&serde_json::Value> = events
        .iter()
        .filter(|event| event["event"] == "degraded_exit")
        .collect();
    assert_eq!(enters.len(), 1, "同一降级期只打一次 enter: {content}");
    assert_eq!(exits.len(), 1, "恢复只打一次 exit: {content}");
    assert_eq!(enters[0]["connection_id"], "slow");
    assert_eq!(enters[0]["reason"], "outbound_queue_full");
    assert_eq!(enters[0]["event_seq"], 2, "enter 携带被丢帧的 event_seq");
    assert_eq!(exits[0]["reason"], "session_state_baseline_delivered");
    assert_eq!(exits[0]["event_seq"], 4, "exit 携带恢复基线的 event_seq");
}
