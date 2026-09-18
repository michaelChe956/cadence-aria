/// T3：run 在最后一个 attachment 断开后完成时，manager 必须从 registry 回收；再次
/// attach 必须从 durable 重建，且不得新增断连终态审计节点。
#[tokio::test]
async fn workspace_session_manager_recycled_after_terminal_without_subscribers() {
    let root = tempdir().expect("root");
    let _repo = create_workspace_session_fixture(&root).await;
    let complete = Arc::new(Notify::new());
    let mut registry = ProviderRegistry::new();
    registry.register(
        ProviderName::Fake,
        Arc::new(SignalledCompletionStreamingProvider {
            complete: complete.clone(),
        }),
    );
    let state = WebAppState::with_provider_registry(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
        registry,
    );
    let app = build_web_router(state.clone());
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move { axum::serve(listener, app).await.expect("serve") });
    let url = format!("ws://{addr}/api/workspace-sessions/workspace_session_0001/ws");

    let (mut ws, _) = connect_async(url.clone()).await.expect("ws");
    let _initial = recv_json(&mut ws).await;
    send_json(
        &mut ws,
        &WsInMessage::UserMessage {
            content: long_message("recycle_probe"),
        },
    )
    .await;
    let _chunk = recv_until_stream_chunk(&mut ws).await;
    assert!(
        state
            .test_controls
            .drop_workspace_socket("workspace_session_0001")
            .await,
        "测试必须先让服务端完成 detach，再通知 run 终态"
    );
    tokio::time::timeout(Duration::from_secs(2), ws.next())
        .await
        .expect("服务端 test drop 超时")
        .expect("服务端 test drop 应返回 close frame")
        .expect("有效 close frame");
    drop(ws);
    assert_eq!(state.workspace_sessions.session_ids().await.len(), 1);

    complete.notify_one();
    let recycled = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if state.workspace_sessions.session_ids().await.is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    });
    assert!(recycled.await.is_ok(), "终态且无订阅者后 manager 应被回收");

    let (mut again, _) = connect_async(url).await.expect("reconnect ws");
    match recv_json(&mut again).await {
        WsOutMessage::SessionState { timeline_nodes, .. } => {
            assert_eq!(
                timeline_nodes
                    .iter()
                    .filter(|node| node.node_type == TimelineNodeType::AbortedByDisconnect)
                    .count(),
                0,
                "重建不得新增断连审计节点"
            );
        }
        other => panic!("expected session_state, got {other:?}"),
    }
    drop(again);
    server.abort();
}

/// REQ-WCR-02：第二个 legacy driver 接管会话 lease；被接管的旧连接迟到写必须
/// 以可诊断的协议错误拒绝，而当前 holder 仍可中止同一活动 run。
#[tokio::test]
async fn workspace_ws_lease_takeover_and_stale_write_rejection() {
    let root = tempdir().expect("root");
    let _repo = create_workspace_session_fixture(&root).await;
    let complete = Arc::new(Notify::new());
    let mut registry = ProviderRegistry::new();
    registry.register(
        ProviderName::Fake,
        Arc::new(SignalledCompletionStreamingProvider {
            complete: complete.clone(),
        }),
    );
    let app = build_web_router(WebAppState::with_provider_registry(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
        registry,
    ));
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move { axum::serve(listener, app).await.expect("serve") });
    let url = format!("ws://{addr}/api/workspace-sessions/workspace_session_0001/ws");

    let (mut first_driver, _) = connect_async(url.clone()).await.expect("first driver");
    let _initial = recv_json(&mut first_driver).await;
    send_json(
        &mut first_driver,
        &WsInMessage::UserMessage {
            content: long_message("lease_takeover"),
        },
    )
    .await;
    let _chunk = recv_until_stream_chunk(&mut first_driver).await;

    let (mut second_driver, _) = connect_async(url).await.expect("second driver");
    let _second_initial = recv_json(&mut second_driver).await;

    send_json(&mut first_driver, &WsInMessage::Abort).await;
    match recv_json(&mut first_driver).await {
        WsOutMessage::ProtocolError { code, .. } => assert_eq!(code, "STALE_DRIVER_LEASE"),
        other => panic!("stale driver write must be rejected, got {other:?}"),
    }

    drop(first_driver);
    send_json(&mut second_driver, &WsInMessage::Abort).await;
    loop {
        match recv_json(&mut second_driver).await {
            WsOutMessage::ProviderStatus { status } => {
                assert_eq!(status, WsProviderStatus::Aborted);
                break;
            }
            // 该连接 attach 时活跃 run 已在推进：初帧基线之后的 journal 补发帧
            // 先于 abort 响应到达，跳过直至 holder 的中止状态。
            WsOutMessage::Error { message } => panic!("second driver ws error: {message}"),
            _ => continue,
        }
    }

    drop(second_driver);
    complete.notify_one();
    server.abort();
}

/// REQ-WCR-02：driver close 只撤销连接持有型 lease，不取消正在运行的 provider，
/// 后续真实完成仍写入业务终态且不产生断连中止 marker。
#[tokio::test]
async fn workspace_ws_driver_close_revokes_lease_run_completes() {
    let (_lock, _controls_env) = ConnectionDiagnosticTestControlsGuard::enable().await;
    let root = tempdir().expect("root");
    let _repo = create_workspace_session_fixture(&root).await;
    let complete = Arc::new(Notify::new());
    let mut registry = ProviderRegistry::new();
    registry.register(
        ProviderName::Fake,
        Arc::new(SignalledCompletionStreamingProvider {
            complete: complete.clone(),
        }),
    );
    let state = WebAppState::with_provider_registry(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
        registry,
    );
    let controls = state.test_controls.clone();
    let app = build_web_router(state);
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move { axum::serve(listener, app).await.expect("serve") });
    let url = format!("ws://{addr}/api/workspace-sessions/workspace_session_0001/ws");

    let (mut driver, _) = connect_async(url).await.expect("driver");
    let _initial = recv_json(&mut driver).await;
    send_json(
        &mut driver,
        &WsInMessage::UserMessage {
            content: long_message("driver_close_revokes_lease"),
        },
    )
    .await;
    let _chunk = recv_until_stream_chunk(&mut driver).await;
    drop(driver);

    let diagnostic = wait_for_connection_diagnostic(&controls, "eof").await;
    assert_eq!(diagnostic["role"], "driver");
    assert!(diagnostic["current_run_token"].as_u64().is_some());

    complete.notify_one();
    let mut completed = false;
    for _ in 0..100 {
        tokio::time::sleep(Duration::from_millis(50)).await;
        let nodes = LifecycleStore::new(ProductAppPaths::new(root.path().join(".aria")))
            .load_timeline_nodes("workspace_session_0001")
            .expect("timeline nodes");
        if persisted_workspace_messages(root.path())
            .iter()
            .any(|message| message.role == "assistant" && message.content.contains("# Story Spec"))
        {
            assert!(
                nodes
                    .iter()
                    .all(|node| node.node_type != TimelineNodeType::AbortedByDisconnect),
                "driver close must not write disconnect-aborted terminal"
            );
            completed = true;
            break;
        }
    }
    assert!(
        completed,
        "revoking a lease must leave the run to its business completion"
    );
    server.abort();
}

/// REQ-WCR-02/F1：显式 observer 的 Hello 不得接管 driver lease；读面的 initial
/// snapshot 仍可用，而现任 driver 必须继续能够中止 run。
#[tokio::test]
async fn workspace_ws_observer_connection_does_not_take_over_lease() {
    let root = tempdir().expect("root");
    let _repo = create_workspace_session_fixture(&root).await;
    let complete = Arc::new(Notify::new());
    let mut registry = ProviderRegistry::new();
    registry.register(
        ProviderName::Fake,
        Arc::new(SignalledCompletionStreamingProvider {
            complete: complete.clone(),
        }),
    );
    let app = build_web_router(WebAppState::with_provider_registry(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
        registry,
    ));
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move { axum::serve(listener, app).await.expect("serve") });
    let url = format!("ws://{addr}/api/workspace-sessions/workspace_session_0001/ws");

    let (mut driver, _) = connect_async(url.clone()).await.expect("driver");
    let _initial = recv_json(&mut driver).await;
    send_json(
        &mut driver,
        &WsInMessage::UserMessage {
            content: long_message("observer_does_not_take_lease"),
        },
    )
    .await;
    let _chunk = recv_until_stream_chunk(&mut driver).await;

    let (mut observer, _) = connect_async(url).await.expect("observer");
    send_json(
        &mut observer,
        &WsInMessage::Hello {
            session_id: "workspace_session_0001".into(),
            last_seen_node_id: None,
            role: Some(HelloRole::Observer),
            after_event_seq: None,
        },
    )
    .await;
    assert!(matches!(
        recv_json(&mut observer).await,
        WsOutMessage::SessionState { .. }
    ));

    send_json(&mut driver, &WsInMessage::Abort).await;
    match recv_json(&mut driver).await {
        WsOutMessage::ProviderStatus { status } => assert_eq!(status, WsProviderStatus::Aborted),
        other => panic!("observer must not stale the driver lease, got {other:?}"),
    }

    drop(observer);
    drop(driver);
    complete.notify_one();
    server.abort();
}

/// REQ-WCR-04/T9：manager 只分配一次序号；attach 基线和所有在线 attachment 的直播
/// 事件必须携带同一递增 `event_seq`，而非 socket-local 序号。
#[tokio::test]
async fn workspace_ws_fanout_events_have_shared_monotonic_event_seq() {
    let root = tempdir().expect("root");
    let _repo = create_workspace_session_fixture(&root).await;
    let complete = Arc::new(Notify::new());
    let mut registry = ProviderRegistry::new();
    registry.register(
        ProviderName::Fake,
        Arc::new(SignalledCompletionStreamingProvider {
            complete: complete.clone(),
        }),
    );
    let app = build_web_router(WebAppState::with_provider_registry(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
        registry,
    ));
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move { axum::serve(listener, app).await.expect("serve") });
    let url = format!("ws://{addr}/api/workspace-sessions/workspace_session_0001/ws");

    let (mut driver, _) = connect_async(url.clone()).await.expect("driver");
    let initial = recv_json_value(&mut driver).await;
    assert_eq!(initial["type"], "session_state");
    assert_eq!(
        initial["event_seq"], 0,
        "first attach establishes seq baseline"
    );

    let (mut observer, _) = connect_async(url).await.expect("observer");
    let observer_initial = recv_json_value(&mut observer).await;
    assert_eq!(observer_initial["type"], "session_state");
    assert_eq!(
        observer_initial["event_seq"], 0,
        "all attachments share baseline"
    );
    send_json(
        &mut observer,
        &WsInMessage::Hello {
            session_id: "workspace_session_0001".into(),
            last_seen_node_id: None,
            role: Some(HelloRole::Observer),
            after_event_seq: None,
        },
    )
    .await;

    send_json(
        &mut driver,
        &WsInMessage::UserMessage {
            content: long_message("event_seq_fanout"),
        },
    )
    .await;
    let driver_chunk = recv_until_stream_chunk_value(&mut driver).await;
    let observer_chunk = recv_until_stream_chunk_value(&mut observer).await;
    let driver_seq = driver_chunk["event_seq"]
        .as_u64()
        .expect("driver event seq");
    let observer_seq = observer_chunk["event_seq"]
        .as_u64()
        .expect("observer event seq");
    assert_eq!(
        driver_seq, observer_seq,
        "fan-out shares the stamped event identity"
    );
    assert!(driver_seq > 0, "live event advances attach baseline");

    complete.notify_one();
    drop(observer);
    drop(driver);
    server.abort();
}

/// REQ-WCR-04：一个不消费的 observer 不得反压 provider，快 observer 保持精确直播流。
#[tokio::test]
async fn workspace_ws_slow_subscriber_degrades_without_backpressure() {
    let root = tempdir().expect("root");
    create_workspace_session_fixture(&root).await;
    let step = Arc::new(Notify::new());
    let complete = Arc::new(Notify::new());
    let mut registry = ProviderRegistry::new();
    registry.register(
        ProviderName::Fake,
        Arc::new(GatedChunkStreamingProvider {
            step: step.clone(),
            complete: complete.clone(),
            chunk_bytes: 0,
        }),
    );
    let app = build_web_router(WebAppState::with_provider_registry(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
        registry,
    ));
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move { axum::serve(listener, app).await.expect("serve") });
    let url = format!("ws://{addr}/api/workspace-sessions/workspace_session_0001/ws");

    let (mut driver, _) = connect_async(url.clone()).await.expect("driver");
    assert_eq!(recv_json_value(&mut driver).await["type"], "session_state");

    let (mut slow_observer, _) = connect_async(url.clone()).await.expect("slow observer");
    send_json(
        &mut slow_observer,
        &WsInMessage::Hello {
            session_id: "workspace_session_0001".into(),
            last_seen_node_id: None,
            role: Some(HelloRole::Observer),
            after_event_seq: None,
        },
    )
    .await;

    let (mut fast_observer, _) = connect_async(url.clone()).await.expect("fast observer");
    send_json(
        &mut fast_observer,
        &WsInMessage::Hello {
            session_id: "workspace_session_0001".into(),
            last_seen_node_id: None,
            role: Some(HelloRole::Observer),
            after_event_seq: None,
        },
    )
    .await;
    assert_eq!(
        recv_json_value(&mut fast_observer).await["type"],
        "session_state"
    );

    assert_eq!(
        timeout(Duration::from_secs(3), recv_json_value(&mut slow_observer))
            .await
            .expect("slow observer initial snapshot")["type"],
        "session_state"
    );

    send_json(
        &mut driver,
        &WsInMessage::UserMessage {
            content: long_message("slow_subscriber"),
        },
    )
    .await;
    let initial_driver = recv_until_stream_chunk_value(&mut driver).await;
    assert!(
        initial_driver["content"]
            .as_str()
            .is_some_and(|content| content.starts_with("gated chunk 0"))
    );
    let initial_fast = recv_until_stream_chunk_value(&mut fast_observer).await;
    assert!(
        initial_fast["content"]
            .as_str()
            .is_some_and(|content| content.starts_with("gated chunk 0"))
    );

    for index in 1..=200 {
        step.notify_one();
        let chunk = timeout(
            Duration::from_secs(2),
            recv_until_stream_chunk_value(&mut driver),
        )
        .await
        .expect("slow observer must not backpressure driver");
        assert!(
            chunk["content"]
                .as_str()
                .is_some_and(|content| content.starts_with(&format!("gated chunk {index}"))),
            "driver must receive stepped chunk {index}"
        );
    }

    // 满队列降级和单发 `resync_required` 由 manager 单测精确覆盖；此处保持端到端的
    // router 不反压和快订阅者全量接收验证，避免本机 TCP 缓冲大小污染协议测试。
    for index in 1..=200 {
        let chunk = timeout(
            Duration::from_secs(2),
            recv_until_stream_chunk_value(&mut fast_observer),
        )
        .await
        .expect("fast observer must receive every stepped live chunk");
        assert!(
            chunk["content"]
                .as_str()
                .is_some_and(|content| content.starts_with(&format!("gated chunk {index}"))),
            "fast observer must receive stepped chunk {index}"
        );
    }
    complete.notify_one();
    drop(slow_observer);
    drop(driver);
    server.abort();
}

/// REQ-WCR-04：关闭单一 attachment 只摘除该连接，其他连接的事件流和 run 不受影响。
#[tokio::test]
async fn matrix4_observer_close_zero_impact_on_driver_run() {
    let root = tempdir().expect("root");
    create_workspace_session_fixture(&root).await;
    let step = Arc::new(Notify::new());
    let complete = Arc::new(Notify::new());
    let mut registry = ProviderRegistry::new();
    registry.register(
        ProviderName::Fake,
        Arc::new(GatedChunkStreamingProvider {
            step: step.clone(),
            complete: complete.clone(),
            chunk_bytes: 0,
        }),
    );
    let app = build_web_router(WebAppState::with_provider_registry(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
        registry,
    ));
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move { axum::serve(listener, app).await.expect("serve") });
    let url = format!("ws://{addr}/api/workspace-sessions/workspace_session_0001/ws");

    let (mut driver, _) = connect_async(url.clone()).await.expect("driver");
    assert_eq!(recv_json_value(&mut driver).await["type"], "session_state");
    let (mut closing_observer, _) = connect_async(url.clone()).await.expect("closing observer");
    assert_eq!(
        recv_json_value(&mut closing_observer).await["type"],
        "session_state"
    );
    send_json(
        &mut closing_observer,
        &WsInMessage::Hello {
            session_id: "workspace_session_0001".into(),
            last_seen_node_id: None,
            role: Some(HelloRole::Observer),
            after_event_seq: None,
        },
    )
    .await;
    let (mut observing_observer, _) = connect_async(url.clone())
        .await
        .expect("observing observer");
    assert_eq!(
        recv_json_value(&mut observing_observer).await["type"],
        "session_state"
    );
    send_json(
        &mut observing_observer,
        &WsInMessage::Hello {
            session_id: "workspace_session_0001".into(),
            last_seen_node_id: None,
            role: Some(HelloRole::Observer),
            after_event_seq: None,
        },
    )
    .await;

    send_json(
        &mut driver,
        &WsInMessage::UserMessage {
            content: long_message("close_one_connection"),
        },
    )
    .await;
    assert_eq!(
        recv_until_stream_chunk_value(&mut driver).await["content"],
        "gated chunk 0"
    );
    assert_eq!(
        recv_until_stream_chunk_value(&mut closing_observer).await["content"],
        "gated chunk 0"
    );
    assert_eq!(
        recv_until_stream_chunk_value(&mut observing_observer).await["content"],
        "gated chunk 0"
    );

    closing_observer.close(None).await.expect("close observer");
    drop(closing_observer);
    tokio::time::sleep(Duration::from_millis(50)).await;

    for index in 1..=2 {
        step.notify_one();
        assert_eq!(
            recv_until_stream_chunk_value(&mut driver).await["content"],
            format!("gated chunk {index}"),
            "driver stream must continue after a peer closes"
        );
        assert_eq!(
            recv_until_stream_chunk_value(&mut observing_observer).await["content"],
            format!("gated chunk {index}"),
            "remaining observer stream must continue after a peer closes"
        );
    }
    complete.notify_one();
    let _ = recv_until_message_complete(&mut driver).await;

    drop(observing_observer);
    drop(driver);
    server.abort();
}
async fn recv_json_value(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) -> Value {
    let message = timeout(Duration::from_secs(3), ws.next())
        .await
        .expect("ws message timeout")
        .expect("ws message")
        .expect("valid ws message");
    match message {
        Message::Text(text) => serde_json::from_str(&text).expect("ws json"),
        other => panic!("expected text ws message, got {other:?}"),
    }
}

async fn recv_until_stream_chunk_value(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) -> Value {
    for _ in 0..40 {
        let message = recv_json_value(ws).await;
        match message["type"].as_str() {
            Some("stream_chunk") => return message,
            Some("error") => panic!("ws error: {}", message["message"]),
            _ => {}
        }
    }
    panic!("stream_chunk not received");
}

async fn recv_until_close_frame(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    context: &str,
) {
    for _ in 0..20 {
        match timeout(Duration::from_secs(7), ws.next())
            .await
            .unwrap_or_else(|_| panic!("{context} timeout"))
        {
            Some(Ok(Message::Close(_))) => return,
            Some(Err(error))
                if error
                    .to_string()
                    .contains("reset without closing handshake") =>
            {
                // 当前 axum socket 关闭会先记录 server_idle 并发送 Close；对端在 close
                // 握手完成前复位时，tungstenite 会报告此错误而不是可见 Close frame。
                // 诊断断言随后仍验证服务端实际按 idle 关闭。
                return;
            }
            Some(Ok(Message::Text(text))) => {
                let json: Value = serde_json::from_str(&text).expect("ws json before close");
                assert_ne!(
                    json["type"], "error",
                    "{context} received protocol error before idle close: {json}"
                );
            }
            Some(Ok(_)) => {}
            Some(Err(error)) => panic!("{context} websocket error: {error}"),
            None => panic!("{context} websocket ended before close frame"),
        }
    }
    panic!("{context} did not receive close frame or reset");
}

/// RCA §6 矩阵②服务端半面：human_confirm 静默超过 idle 阈值后，服务器可以回收
/// 连接，但不能写入任何终态；重连仍必须投影原来的门等待。
#[tokio::test]
async fn matrix2_gate_silence_idle_close_writes_no_terminal() {
    let (_lock, _controls_env) = ConnectionDiagnosticTestControlsGuard::enable().await;
    let root = tempdir().expect("root");
    let _repo = create_workspace_session_fixture(&root).await;
    let state = WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    );
    let controls = state.test_controls.clone();
    controls
        .set_server_idle_timeout(Duration::from_millis(30))
        .await;
    let app = build_web_router(state);
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move { axum::serve(listener, app).await.expect("serve") });
    let url = format!("ws://{addr}/api/workspace-sessions/workspace_session_0001/ws");

    let (mut ws, _) = connect_async(url.clone()).await.expect("ws");
    let _initial = recv_json(&mut ws).await;
    send_json(
        &mut ws,
        &WsInMessage::UserMessage {
            content: long_message("matrix2_gate_silence"),
        },
    )
    .await;
    let _checkpoint = recv_until_message_complete(&mut ws).await;
    accept_author_output(&mut ws).await;
    assert_eq!(
        recv_until_stage(&mut ws, "human_confirm").await,
        "human_confirm"
    );

    let durable_before_idle_close = durable_tree_snapshot(root.path());
    recv_until_close_frame(&mut ws, "human gate idle close").await;
    let diagnostic = wait_for_connection_diagnostic(&controls, "server_idle").await;
    assert_eq!(diagnostic["idle_timeout_triggered"], true);
    assert!(
        diagnostic["current_run_token"].is_null(),
        "human_confirm 窗口不能保留 active run"
    );

    let lifecycle = LifecycleStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let session = lifecycle
        .get_workspace_session("workspace_session_0001")
        .expect("workspace session");
    assert_eq!(
        session.status,
        cadence_aria::product::models::WorkspaceSessionStatus::WaitingForHuman
    );
    let nodes = lifecycle
        .load_timeline_nodes("workspace_session_0001")
        .expect("timeline nodes");
    assert!(
        nodes
            .iter()
            .all(|node| node.node_type != TimelineNodeType::AbortedByDisconnect),
        "idle 回收 human_confirm 连接不得写入断连伪终态"
    );
    assert_ne!(
        nodes.last().expect("human gate timeline node").status,
        TimelineNodeStatus::Failed,
        "idle 回收不得将等待门覆盖为 Failed"
    );
    assert_eq!(
        durable_tree_snapshot(root.path()),
        durable_before_idle_close,
        "idle close 不得产生任何 durable 终态写入"
    );

    let (mut probe, _) = connect_async(url).await.expect("reconnect probe");
    match recv_json(&mut probe).await {
        WsOutMessage::SessionState {
            stage,
            session_status,
            timeline_nodes,
            ..
        } => {
            assert_eq!(
                session_status,
                cadence_aria::product::models::WorkspaceSessionStatus::WaitingForHuman
            );
            assert!(
                matches!(stage.as_str(), "author_confirm" | "human_confirm"),
                "重连必须恢复一个待人工处理的 gate，而非由 idle close 改写为终态：{stage}"
            );
            assert!(
                timeline_nodes
                    .iter()
                    .all(|node| node.node_type != TimelineNodeType::AbortedByDisconnect)
            );
        }
        other => panic!("expected session_state after idle reconnect, got {other:?}"),
    }

    drop(probe);
    drop(ws);
    server.abort();
}

/// RCA §6 矩阵③：所有可在 active run 期间由客户端触发的关闭形态都必须留下唯一的
/// 中性诊断，并让 provider 自然写入业务终态；server-idle 在 active run 中由矩阵①
/// 的 guard 禁止触发，空闲 gate 的 server-idle 回收由矩阵②覆盖。
#[tokio::test]
async fn matrix3_non_idle_closes_during_run_keep_business_terminal() {
    let (_lock, _controls_env) = ConnectionDiagnosticTestControlsGuard::enable().await;
    for close_kind in [
        Matrix3ClientCloseKind::StaleSocket4000,
        Matrix3ClientCloseKind::PageUnload1000,
        Matrix3ClientCloseKind::TcpDrop,
    ] {
        let root = tempdir().expect("root");
        let _repo = create_workspace_session_fixture(&root).await;
        let complete = Arc::new(Notify::new());
        let mut registry = ProviderRegistry::new();
        registry.register(
            ProviderName::Fake,
            Arc::new(SignalledCompletionStreamingProvider {
                complete: complete.clone(),
            }),
        );
        let state = WebAppState::with_provider_registry(
            root.path().to_path_buf(),
            WebRuntime::new_fake(root.path().to_path_buf()),
            registry,
        );
        let controls = state.test_controls.clone();
        let app = build_web_router(state);
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("local addr");
        let server = tokio::spawn(async move { axum::serve(listener, app).await.expect("serve") });
        let url = format!("ws://{addr}/api/workspace-sessions/workspace_session_0001/ws");

        let (ws, _) = connect_async(url).await.expect("ws");
        let mut ws = Some(ws);
        let _initial = recv_json(ws.as_mut().expect("active ws")).await;
        send_json(
            ws.as_mut().expect("active ws"),
            &WsInMessage::UserMessage {
                content: long_message(close_kind.message_token()),
            },
        )
        .await;
        let _chunk = recv_until_stream_chunk(ws.as_mut().expect("active ws")).await;

        let receiver_exit = close_kind.close(ws.as_mut().expect("active ws")).await;
        if close_kind.is_tcp_drop() {
            drop(ws.take());
        }
        let diagnostic = wait_for_connection_diagnostic(&controls, receiver_exit).await;
        assert!(
            diagnostic["current_run_token"].as_u64().is_some(),
            "{close_kind:?} 的诊断必须保留 close 时 run 在途的 token：{diagnostic}"
        );
        assert_eq!(diagnostic["idle_timeout_triggered"], false);
        close_kind.assert_diagnostic(&diagnostic);

        complete.notify_one();
        wait_for_business_terminal(root.path(), close_kind.message_token()).await;
        let nodes = LifecycleStore::new(ProductAppPaths::new(root.path().join(".aria")))
            .load_timeline_nodes("workspace_session_0001")
            .expect("timeline nodes");
        assert!(
            nodes
                .iter()
                .all(|node| node.node_type != TimelineNodeType::AbortedByDisconnect),
            "{close_kind:?} 不得写入 aborted_by_disconnect"
        );

        drop(ws.take());
        server.abort();
    }
}

#[derive(Debug, Clone, Copy)]
enum Matrix3ClientCloseKind {
    StaleSocket4000,
    PageUnload1000,
    TcpDrop,
}

impl Matrix3ClientCloseKind {
    fn message_token(self) -> &'static str {
        match self {
            Self::StaleSocket4000 => "matrix3_close_4000",
            Self::PageUnload1000 => "matrix3_close_1000",
            Self::TcpDrop => "matrix3_tcp_drop",
        }
    }

    async fn close(
        self,
        ws: &mut tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
    ) -> &'static str {
        match self {
            Self::StaleSocket4000 => {
                ws.send(Message::Close(Some(tokio_tungstenite::tungstenite::protocol::CloseFrame {
                    code: tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode::Library(4000),
                    reason: "matrix stale socket".into(),
                })))
                .await
                .expect("send 4000 close");
                "close_frame"
            }
            Self::PageUnload1000 => {
                ws.close(None).await.expect("send 1000 close");
                "close_frame"
            }
            Self::TcpDrop => {
                // 将底层连接留给函数末尾 drop；此处停止发送即可模拟无 close frame 的 EOF。
                "eof"
            }
        }
    }

    fn is_tcp_drop(self) -> bool {
        matches!(self, Self::TcpDrop)
    }

    fn assert_diagnostic(self, diagnostic: &Value) {
        match self {
            Self::StaleSocket4000 => {
                assert_eq!(diagnostic["close_code"], 4000);
                assert_eq!(diagnostic["close_reason"], "matrix stale socket");
            }
            Self::PageUnload1000 => {
                assert!(
                    diagnostic["close_code"].is_null() || diagnostic["close_code"] == 1000,
                    "page unload must preserve its 1000-or-empty close diagnostic: {diagnostic}"
                );
            }
            Self::TcpDrop => {
                assert!(
                    diagnostic["close_code"].is_null(),
                    "TCP EOF must not be encoded as a close frame: {diagnostic}"
                );
            }
        }
    }
}

async fn wait_for_business_terminal(root: &std::path::Path, close_kind: &str) {
    for _ in 0..100 {
        if persisted_workspace_messages(root)
            .iter()
            .any(|message| message.role == "assistant" && message.content.contains("# Story Spec"))
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("{close_kind} 后 provider 业务终态未落盘");
}
