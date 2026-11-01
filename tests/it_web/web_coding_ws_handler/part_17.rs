// F-19：coding runner 事件流单连接绑定无广播。此前 runner 绑定驱动 socket 的
// 私有 event channel（socket.rs spawn 时传入本连接 event_tx），唯一消费者断开
// 即死、外部恢复后 campaign 驱动失明。本组测试钉死两面绿契约：
// 1) 多消费者：第二连接（观察/接管连接）必须收到 runner 实时事件；
// 2) 断开存活：驱动连接断开后事件流不得死亡，仍存活的连接继续收到 runner
//    后续事件（含 fail-closed 终态可见）。
// 对照 workspace 面 P2 先例（workspace_session manager router broadcast），coding
// 面以 socket registry per-attempt hub fan-out 达成同款语义。

/// F-19 红面一：第二连接收不到 runner 事件。同一 attempt 建立两条 WS，连接一
/// 发 StartCoding，连接二必须实时收到 WorktreePrepare stage change——单连接
/// 绑定下连接二在初始快照后一片死寂。
#[tokio::test]
async fn coding_ws_runner_events_broadcast_to_second_connection() {
    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let app = app_with_attempt(root.path());
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    let url = format!("ws://{addr}/ws/coding-attempts/coding_attempt_0001");
    let (mut ws1, _) = connect_async(url.clone()).await.expect("connect ws1");
    let _snapshot1 = recv_json(&mut ws1).await;
    let (mut ws2, _) = connect_async(url).await.expect("connect ws2");
    let _snapshot2 = recv_json(&mut ws2).await;

    send_json(&mut ws1, &CodingWsInMessage::StartCoding).await;

    assert_eq!(
        recv_json(&mut ws2).await,
        CodingWsOutMessage::CodingStageChange {
            stage: CodingExecutionStage::WorktreePrepare
        },
        "F-19: 第二连接必须收到 runner 实时事件（多消费者广播）"
    );
    // 驱动连接自身事件面零回归：仍按原序收到同一事件。
    assert_eq!(
        recv_json(&mut ws1).await,
        CodingWsOutMessage::CodingStageChange {
            stage: CodingExecutionStage::WorktreePrepare
        },
        "驱动连接不得因广播引入回归"
    );

    ws1.close(None).await.expect("close ws1");
    ws2.close(None).await.expect("close ws2");
    server.abort();
}

/// F-19 红面二：唯一消费者（驱动 WS）断开即死。连接一发 StartCoding 后立即
/// 断开，仍存活的连接二必须继续收到 runner 后续事件直至 fail-closed 终态
/// （protocol error 可见）——单连接绑定下 runner 事件随驱动连接一起蒸发。
#[tokio::test]
async fn coding_ws_runner_event_stream_survives_driver_disconnect() {
    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let app = app_with_attempt(root.path());
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    let url = format!("ws://{addr}/ws/coding-attempts/coding_attempt_0001");
    let (mut ws1, _) = connect_async(url.clone()).await.expect("connect ws1");
    let _snapshot1 = recv_json(&mut ws1).await;
    let (mut ws2, _) = connect_async(url).await.expect("connect ws2");
    let _snapshot2 = recv_json(&mut ws2).await;

    send_json(&mut ws1, &CodingWsInMessage::StartCoding).await;
    ws1.close(None).await.expect("close ws1");

    // 连接二必须先收到 runner 起步事件（事件流未随驱动连接死亡）。
    assert_eq!(
        recv_json(&mut ws2).await,
        CodingWsOutMessage::CodingStageChange {
            stage: CodingExecutionStage::WorktreePrepare
        },
        "F-19: 驱动连接断开后事件流必须继续向存活连接广播"
    );
    // 并收到 fail-closed 终态可见帧（本 fixture worktree prepare 确定性失败 →
    // AwaitingManualRecovery → coding_start_failed protocol error）。
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        if std::time::Instant::now() > deadline {
            panic!("F-19: 驱动连接断开后存活连接必须收到 runner 终态事件");
        }
        match recv_json(&mut ws2).await {
            CodingWsOutMessage::CodingProtocolError { code, .. } => {
                assert_eq!(code, "coding_start_failed");
                break;
            }
            _ => continue,
        }
    }

    ws2.close(None).await.expect("close ws2");
    server.abort();
}
