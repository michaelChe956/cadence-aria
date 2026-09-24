// 退役留档（T5/REQ-RET-02）：`workspace_ws_reconnect_restores_message_checkpoint_ids` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

#[tokio::test]
async fn workspace_ws_user_message_interrupts_active_stream_before_completion() {
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
    let (mut ws, _) = connect_async(url).await.expect("connect ws");
    let _initial = recv_json(&mut ws).await;

    send_json(
        &mut ws,
        &WsInMessage::UserMessage {
            content: long_message("old_instruction"),
        },
    )
    .await;
    let _first_chunk = recv_until_stream_chunk(&mut ws).await;

    send_json(
        &mut ws,
        &WsInMessage::UserMessage {
            content: "second_override".to_string(),
        },
    )
    .await;

    for _ in 0..200 {
        match recv_json(&mut ws).await {
            WsOutMessage::StreamChunk { content, .. } if content.contains("second_override") => {
                drop(ws);
                server.abort();
                return;
            }
            WsOutMessage::MessageComplete { .. } => {
                panic!("active stream completed before the interrupting message was applied")
            }
            WsOutMessage::Error { message } => panic!("ws error: {message}"),
            _ => {}
        }
    }
    panic!("interrupting message was not streamed");
}

#[tokio::test]
async fn workspace_ws_abort_discards_partial_stream_without_completion() {
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
    let (mut ws, _) = connect_async(url).await.expect("connect ws");
    let _initial = recv_json(&mut ws).await;

    send_json(
        &mut ws,
        &WsInMessage::UserMessage {
            content: long_message("abort_instruction"),
        },
    )
    .await;
    let _first_chunk = recv_until_stream_chunk(&mut ws).await;
    send_json(&mut ws, &WsInMessage::Abort).await;

    for _ in 0..80 {
        match recv_json(&mut ws).await {
            WsOutMessage::StageChange { stage } if stage == "prepare_context" => {
                let messages = persisted_workspace_messages(root.path());
                assert_eq!(messages.len(), 2);
                assert!(messages.iter().any(|message| message.role == "system"));
                assert!(messages.iter().any(|message| {
                    message.role == "user" && message.content == long_message("abort_instruction")
                }));
                drop(ws);
                server.abort();
                return;
            }
            WsOutMessage::MessageComplete { .. } => {
                panic!("aborted stream should not complete a partial assistant message")
            }
            WsOutMessage::Error { message } => panic!("ws error: {message}"),
            _ => {}
        }
    }
    panic!("abort did not return workspace to prepare_context");
}

static CONNECTION_DIAGNOSTIC_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn wait_for_connection_diagnostic(
    controls: &TestControls,
    expected_receiver_exit: &str,
) -> Value {
    for _ in 0..160 {
        if let Some(diagnostic) = controls
            .connection_diagnostics("workspace_session_0001")
            .into_iter()
            .find(|diagnostic| diagnostic["receiver_exit"] == expected_receiver_exit)
        {
            assert!(diagnostic["connection_id"].as_str().is_some_and(|id| !id.is_empty()));
            assert!(diagnostic["last_client_activity_at"].as_str().is_some());
            assert!(diagnostic["last_server_activity_at"].as_str().is_some());
            assert!(diagnostic["provider_drive_depth"].as_u64().is_some());
            return diagnostic;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("connection diagnostic with receiver_exit={expected_receiver_exit} was not recorded");
}

struct ConnectionDiagnosticTestControlsGuard;

impl ConnectionDiagnosticTestControlsGuard {
    async fn enable() -> (tokio::sync::MutexGuard<'static, ()>, Self) {
        let lock = CONNECTION_DIAGNOSTIC_TEST_LOCK.lock().await;
        unsafe {
            std::env::set_var("ARIA_E2E_TEST_CONTROLS", "1");
        }
        (lock, Self)
    }
}

impl Drop for ConnectionDiagnosticTestControlsGuard {
    fn drop(&mut self) {
        unsafe {
            std::env::remove_var("ARIA_E2E_TEST_CONTROLS");
        }
    }
}

#[tokio::test]
async fn workspace_ws_idle_timeout_records_server_idle_connection_diagnostic() {
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
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });
    let url = format!("ws://{addr}/api/workspace-sessions/workspace_session_0001/ws");
    let (mut ws, _) = connect_async(url).await.expect("connect ws");
    let _initial = recv_json(&mut ws).await;

    let closed = timeout(Duration::from_secs(7), ws.next())
        .await
        .expect("server idle close timeout")
        .expect("server idle close result")
        .expect("server idle close frame");
    assert!(matches!(closed, Message::Close(_)));
    let diagnostic = wait_for_connection_diagnostic(&controls, "server_idle").await;
    assert_eq!(diagnostic["idle_timeout_triggered"], true);
    assert_eq!(diagnostic["role"], "driver");

    server.abort();
}

#[tokio::test]
async fn workspace_ws_client_close_4000_records_connection_diagnostic() {
    let (_lock, _controls_env) = ConnectionDiagnosticTestControlsGuard::enable().await;
    let root = tempdir().expect("root");
    let _repo = create_workspace_session_fixture(&root).await;
    let state = WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    );
    let controls = state.test_controls.clone();
    let app = build_web_router(state);
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });
    let url = format!("ws://{addr}/api/workspace-sessions/workspace_session_0001/ws");
    let (mut ws, _) = connect_async(url).await.expect("connect ws");
    let _initial = recv_json(&mut ws).await;
    ws.send(Message::Close(Some(tokio_tungstenite::tungstenite::protocol::CloseFrame {
        code: tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode::Library(4000),
        reason: "stale socket".into(),
    })))
    .await
    .expect("send client close");

    let diagnostic = wait_for_connection_diagnostic(&controls, "close_frame").await;
    assert_eq!(diagnostic["idle_timeout_triggered"], false);
    assert_eq!(diagnostic["close_code"], 4000);
    assert_eq!(diagnostic["close_reason"], "stale socket");
    assert_eq!(diagnostic["role"], "driver");

    server.abort();
}

#[tokio::test]
async fn workspace_ws_page_unload_close_1000_records_connection_diagnostic() {
    let (_lock, _controls_env) = ConnectionDiagnosticTestControlsGuard::enable().await;
    let root = tempdir().expect("root");
    let _repo = create_workspace_session_fixture(&root).await;
    let state = WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    );
    let controls = state.test_controls.clone();
    let app = build_web_router(state);
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });
    let url = format!("ws://{addr}/api/workspace-sessions/workspace_session_0001/ws");
    let (mut ws, _) = connect_async(url).await.expect("connect ws");
    let _initial = recv_json(&mut ws).await;
    ws.close(None).await.expect("send page unload close");

    let diagnostic = wait_for_connection_diagnostic(&controls, "close_frame").await;
    assert_eq!(diagnostic["idle_timeout_triggered"], false);
    assert!(diagnostic["close_code"].is_null() || diagnostic["close_code"] == 1000);

    server.abort();
}

#[tokio::test]
async fn workspace_ws_tcp_drop_records_eof_connection_diagnostic() {
    let (_lock, _controls_env) = ConnectionDiagnosticTestControlsGuard::enable().await;
    let root = tempdir().expect("root");
    let _repo = create_workspace_session_fixture(&root).await;
    let state = WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    );
    let controls = state.test_controls.clone();
    let app = build_web_router(state);
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });
    let url = format!("ws://{addr}/api/workspace-sessions/workspace_session_0001/ws");
    let (mut ws, _) = connect_async(url).await.expect("connect ws");
    let _initial = recv_json(&mut ws).await;
    drop(ws);

    let diagnostic = wait_for_connection_diagnostic(&controls, "eof").await;
    assert_eq!(diagnostic["idle_timeout_triggered"], false);

    server.abort();
}

/// REQ-WCR-03（0429 形态根治）：run 驱动至自然终态后连接关闭/读循环结束，
/// 不得追加 aborted_by_disconnect、不得把 session 拉回 prepare_context。
/// 原过渡期用例（connection_id 写入 marker detail）随 close 写入路径移除而改写；
/// 归因由中性 ConnectionDiagnostic 承担（D9）。
#[tokio::test]
async fn workspace_ws_disconnect_during_active_run_writes_no_disconnect_terminal() {
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
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });
    let url = format!("ws://{addr}/api/workspace-sessions/workspace_session_0001/ws");

    let (mut ws, _) = connect_async(url).await.expect("connect ws");
    let _initial = recv_json(&mut ws).await;
    send_json(
        &mut ws,
        &WsInMessage::UserMessage {
            content: long_message("close_no_terminal"),
        },
    )
    .await;
    let _chunk = recv_until_stream_chunk(&mut ws).await;
    drop(ws); // 连接关闭：run 仍在跑
    let _diagnostic = wait_for_connection_diagnostic(&controls, "eof").await;
    complete.notify_one(); // close 清理已开始；run 驱动至自然完成（业务终态）

    let mut business_terminal = false;
    for _ in 0..100 {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let nodes = LifecycleStore::new(ProductAppPaths::new(root.path().join(".aria")))
            .load_timeline_nodes("workspace_session_0001")
            .expect("timeline nodes");
        let no_marker = nodes
            .iter()
            .all(|node| node.node_type != TimelineNodeType::AbortedByDisconnect);
        let assistant_done = persisted_workspace_messages(root.path())
            .iter()
            .any(|message| message.role == "assistant" && message.content.contains("# Story Spec"));
        if assistant_done {
            assert!(
                no_marker,
                "断连后业务终态落盘，但断连审计节点仍被写入：nodes={:?}",
                nodes
                    .iter()
                    .map(|node| (&node.node_type, &node.status))
                    .collect::<Vec<_>>()
            );
            business_terminal = true;
            break;
        }
    }
    assert!(business_terminal, "run 应驱动至完成并落盘 assistant 消息");
    server.abort();
}

#[tokio::test]
async fn workspace_ws_disconnect_does_not_cancel_active_provider_run() {
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
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    let url = format!("ws://{addr}/api/workspace-sessions/workspace_session_0001/ws");
    let (mut ws, _) = connect_async(url).await.expect("connect ws");
    let _initial = recv_json(&mut ws).await;

    send_json(
        &mut ws,
        &WsInMessage::UserMessage {
            content: long_message("disconnect_survivor"),
        },
    )
    .await;
    let _first_chunk = recv_until_stream_chunk(&mut ws).await;

    // 断连：旧清理路径会无条件取消 runner token（claude×轻 慢握手被杀的同构链）。
    drop(ws);

    // 断连后触发 provider 完成：run 未被取消 → 驱动至完成并落盘 assistant 消息。
    complete.notify_one();
    let mut survived = false;
    for _ in 0..100 {
        tokio::time::sleep(Duration::from_millis(50)).await;
        if persisted_workspace_messages(root.path()).iter().any(|message| {
            message.role == "assistant" && message.content.contains("# Story Spec")
        }) {
            survived = true;
            break;
        }
    }
    assert!(
        survived,
        "断连清理不应取消进行中的 workspace run（run 应驱动至完成并落盘 assistant 消息）：messages={:?} nodes={:?}",
        persisted_workspace_messages(root.path())
            .iter()
            .map(|message| (message.role.clone(), message.content.len()))
            .collect::<Vec<_>>(),
        LifecycleStore::new(ProductAppPaths::new(root.path().join(".aria")))
            .load_timeline_nodes("workspace_session_0001")
            .expect("timeline nodes")
            .iter()
            .map(|node| (node.node_type.clone(), node.status.clone()))
            .collect::<Vec<_>>()
    );

    server.abort();
}

#[tokio::test]
async fn workspace_ws_idle_timeout_holds_reconnect_while_surviving_run_drives() {
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
    state
        .test_controls
        .set_server_idle_timeout(Duration::from_millis(30))
        .await;
    let app = build_web_router(state);
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    let url = format!("ws://{addr}/api/workspace-sessions/workspace_session_0001/ws");
    let (mut primary, _) = connect_async(url.clone()).await.expect("connect primary ws");
    let _initial = recv_json(&mut primary).await;

    send_json(
        &mut primary,
        &WsInMessage::UserMessage {
            content: long_message("idle_hold_survivor"),
        },
    )
    .await;
    let _first_chunk = recv_until_stream_chunk(&mut primary).await;

    // 断连：run 存活并继续驱动（provider 挂起，不触发完成）。
    drop(primary);

    // 重连 socket：本 socket 无 current_run、registry 已摘除——provider drive 期
    // 服务器不得主动 idle 关闭（idle 计时器 tick 粒度 5s，需覆盖首个 5s tick）。
    let (mut reconnected, _) = connect_async(url).await.expect("reconnect ws");
    let _state = recv_json(&mut reconnected).await;
    // 排空初始突发后进入静默窗口。
    for _ in 0..20 {
        if timeout(Duration::from_millis(100), reconnected.next()).await.is_err() {
            break;
        }
    }
    let leaked_close = timeout(Duration::from_millis(5500), reconnected.next()).await;
    assert!(
        leaked_close.is_err(),
        "provider drive 进行中，服务器不得对无 current_run 的重连 socket 主动 idle 关闭"
    );

    // run 结束后 idle 守卫应恢复：下一个 tick 后连接被正常回收（Close 帧）。
    complete.notify_one();
    let mut closed = false;
    for _ in 0..240 {
        match timeout(Duration::from_millis(100), reconnected.next()).await {
            Ok(Some(Ok(Message::Close(_)))) => {
                closed = true;
                break;
            }
            Ok(Some(Ok(_))) => {}
            Ok(Some(Err(_))) | Ok(None) => {
                closed = true;
                break;
            }
            Err(_) => {}
        }
    }
    assert!(
        closed,
        "provider drive 结束后 idle 回收应恢复（连接应被正常关闭）"
    );

    drop(reconnected);
    server.abort();
}

#[tokio::test]
async fn workspace_ws_second_connection_does_not_mark_active_run_stale() {
    let root = tempdir().expect("root");
    create_workspace_session_fixture(&root).await;
    let mut registry = ProviderRegistry::new();
    registry.register(ProviderName::Fake, Arc::new(HangingStreamingProvider));
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
    let (mut primary, _) = connect_async(url.clone())
        .await
        .expect("connect primary ws");
    let _initial = recv_json(&mut primary).await;

    send_json(
        &mut primary,
        &WsInMessage::UserMessage {
            content: long_message("primary_instruction"),
        },
    )
    .await;
    let _first_chunk = recv_until_stream_chunk(&mut primary).await;

    let (mut secondary, _) = connect_async(url.clone())
        .await
        .expect("connect secondary ws");
    send_json(
        &mut secondary,
        &WsInMessage::Hello {
            session_id: "workspace_session_0001".into(),
            last_seen_node_id: None,
            role: Some(HelloRole::Observer),
            after_event_seq: None,
        },
    )
    .await;

    match recv_json(&mut secondary).await {
        WsOutMessage::SessionState {
            stage,
            timeline_nodes,
            active_run_id,
            ..
        } => {
            let last = timeline_nodes.last().expect("timeline node");
            assert_eq!(stage, "running");
            assert!(active_run_id.is_none());
            assert_ne!(last.node_type, TimelineNodeType::AbortedByDisconnect);
            assert_ne!(last.status, TimelineNodeStatus::Failed);
        }
        other => panic!("expected session_state, got {other:?}"),
    }

    send_json(&mut primary, &WsInMessage::Abort).await;
    let _stage = recv_until_stage(&mut primary, "prepare_context").await;

    drop(secondary);
    drop(primary);
    server.abort();
}

/// REQ-DLS-04：次连接经显式 hello（无 role 缺席归一为 Driver）接管 lease 后
/// 仍可中止活动 run——attach 对租约零效应后这是唯一接管路径；被接管的主
/// 连接此后任何迟到写均须被拒绝，保持旧脚本的次连接 abort 行为同时收紧
/// 单活写面。
#[tokio::test]
async fn workspace_ws_secondary_connection_takes_lease_and_can_abort_active_run_started_by_primary() {
    let root = tempdir().expect("root");
    create_workspace_session_fixture(&root).await;
    let mut registry = ProviderRegistry::new();
    registry.register(ProviderName::Fake, Arc::new(HangingStreamingProvider));
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
    let (mut primary, _) = connect_async(url.clone())
        .await
        .expect("connect primary ws");
    let _initial = recv_json(&mut primary).await;

    send_json(
        &mut primary,
        &WsInMessage::UserMessage {
            content: long_message("primary_instruction"),
        },
    )
    .await;
    let _first_chunk = recv_until_stream_chunk(&mut primary).await;

    let (mut secondary, _) = connect_async(url.clone())
        .await
        .expect("connect secondary ws");
    let _secondary_state = recv_json(&mut secondary).await;
    // REQ-DLS-04：次连接必须显式 hello 才接管（无 role 归一 Driver）。
    send_json(
        &mut secondary,
        &WsInMessage::Hello {
            session_id: "workspace_session_0001".into(),
            last_seen_node_id: None,
            role: None,
            after_event_seq: None,
        },
    )
    .await;
    // 同连接读循环顺序：Pong 返回即 hello（bind_role 接管）已生效；活跃
    // run 持有 engine 锁期间 hello 的 session_state 回复不可达，不能作屏障。
    send_json(&mut secondary, &WsInMessage::Ping).await;
    loop {
        match recv_json(&mut secondary).await {
            WsOutMessage::Pong => break,
            WsOutMessage::Error { message } => panic!("secondary ws error: {message}"),
            _ => continue,
        }
    }
    send_json(&mut primary, &WsInMessage::Abort).await;
    match recv_json(&mut primary).await {
        WsOutMessage::ProtocolError { code, .. } => assert_eq!(code, "STALE_DRIVER_LEASE"),
        other => panic!("primary stale write must be rejected after secondary takeover, got {other:?}"),
    }

    send_json(&mut secondary, &WsInMessage::Abort).await;
    tokio::time::timeout(
        Duration::from_secs(5),
        recv_until_stage(&mut primary, "prepare_context"),
    )
    .await
    .expect("secondary abort should stop the primary active run");

    let (mut refreshed, _) = connect_async(url).await.expect("connect refreshed ws");
    match recv_json(&mut refreshed).await {
        WsOutMessage::SessionState {
            stage,
            active_run_id,
            timeline_nodes,
            ..
        } => {
            assert_eq!(stage, "prepare_context");
            assert!(active_run_id.is_none());
            let last = timeline_nodes.last().expect("timeline node");
            assert_eq!(last.status, TimelineNodeStatus::Failed);
            assert_eq!(last.summary.as_deref(), Some("运行已中止"));
        }
        other => panic!("expected session_state, got {other:?}"),
    }

    drop(refreshed);
    drop(secondary);
    drop(primary);
    server.abort();
}

#[tokio::test]
async fn workspace_ws_idle_timeout_does_not_close_socket_during_active_run() {
    let root = tempdir().expect("root");
    let _repo = create_workspace_session_fixture(&root).await;
    let mut registry = ProviderRegistry::new();
    registry.register(ProviderName::Fake, Arc::new(HangingStreamingProvider));
    let state = WebAppState::with_provider_registry(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
        registry,
    );
    state
        .test_controls
        .set_server_idle_timeout(Duration::from_millis(30))
        .await;
    let app = build_web_router(state);
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
        &WsInMessage::UserMessage {
            content: "start long running provider".to_string(),
        },
    )
    .await;
    let _first_chunk = recv_until_stream_chunk(&mut ws).await;

    let next_message = timeout(Duration::from_millis(120), ws.next()).await;
    assert!(
        next_message.is_err(),
        "idle timeout must not close the socket while a provider run is active"
    );

    drop(ws);
    server.abort();
}

#[tokio::test]
async fn workspace_ws_test_control_drop_closes_registered_socket() {
    let root = tempdir().expect("root");
    let _repo = create_workspace_session_fixture(&root).await;
    let state = WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    );
    let controls = state.test_controls.clone();
    let app = build_web_router(state);
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    let url = format!("ws://{addr}/api/workspace-sessions/workspace_session_0001/ws");
    let (mut ws, _) = connect_async(url).await.expect("connect ws");
    let _initial = recv_json(&mut ws).await;

    assert!(
        controls
            .drop_workspace_socket("workspace_session_0001")
            .await
    );

    let closed = timeout(Duration::from_secs(3), ws.next())
        .await
        .expect("socket close timeout")
        .expect("socket close frame")
        .expect("valid close frame");
    assert!(matches!(closed, Message::Close(_)));

    drop(ws);
    server.abort();
}

// 退役留档（T5/REQ-RET-02）：`workspace_ws_supervised_permission_allows_real_stream_to_complete` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

// 退役留档（T5/REQ-RET-02）：`workspace_ws_claude_author_ask_user_question_choice_continues_same_provider` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

#[tokio::test]
async fn workspace_ws_hello_during_pending_choice_does_not_block_choice_response() {
    let root = tempdir().expect("root");
    create_workspace_session_fixture(&root).await;
    let mut registry = ProviderRegistry::new();
    registry.register(
        ProviderName::Fake,
        Arc::new(ChoiceThenCompletingStreamingProvider),
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
        &WsInMessage::UserMessage {
            content: "run choice provider".to_string(),
        },
    )
    .await;

    let choice = recv_until_choice_request(&mut ws).await;
    send_json(
        &mut ws,
        &WsInMessage::Hello {
            session_id: "workspace_session_0001".to_string(),
            last_seen_node_id: None,
            role: None,
            after_event_seq: None,
        },
    )
    .await;
    send_json(
        &mut ws,
        &WsInMessage::ChoiceResponse {
            id: choice.id,
            selected_option_ids: vec!["opt_0".to_string()],
            free_text: None,
            answers: vec![],
        },
    )
    .await;

    let checkpoint = recv_until_message_complete(&mut ws).await;
    assert!(checkpoint.starts_with("cp_"));

    drop(ws);
    server.abort();
}

/// F-24 现场锚（0484）：provider 挂起 choice 期间另一连接接入（页面刷新/断线
/// 重连后 cockpit 重开），必须能收到挂起 choice 卡并代答——否则引擎等待界
/// （F-22 900s）只能把卡死降级为超时失败，用户全程看不到问题。
#[tokio::test]
async fn workspace_ws_second_connection_receives_and_answers_pending_choice() {
    let root = tempdir().expect("root");
    create_workspace_session_fixture(&root).await;
    let mut registry = ProviderRegistry::new();
    registry.register(
        ProviderName::Fake,
        Arc::new(ChoiceThenCompletingStreamingProvider),
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
    let (mut primary, _) = connect_async(url.clone()).await.expect("connect primary");
    let _initial = recv_json(&mut primary).await;
    send_json(
        &mut primary,
        &WsInMessage::UserMessage {
            content: "run choice provider for takeover".to_string(),
        },
    )
    .await;
    let _choice = recv_until_choice_request(&mut primary).await;

    // 次连接重连：初帧（grace/首入站激活）必须带回挂起 choice 卡。
    let (mut secondary, _) = connect_async(url).await.expect("connect secondary");
    let _state = recv_json(&mut secondary).await;
    let replayed_choice = recv_until_choice_request(&mut secondary).await;

    // REQ-DLS-04：attach 对租约零效应——次连接代答前必须显式 hello（无 role
    // 缺席归一为 Driver）接管 lease，方可写；F-24 用户语义（重连代答）不变。
    send_json(
        &mut secondary,
        &WsInMessage::Hello {
            session_id: "workspace_session_0001".into(),
            last_seen_node_id: None,
            role: None,
            after_event_seq: None,
        },
    )
    .await;
    assert!(matches!(
        recv_json(&mut secondary).await,
        WsOutMessage::SessionState { .. }
    ));
    send_json(
        &mut secondary,
        &WsInMessage::ChoiceResponse {
            id: replayed_choice.id,
            selected_option_ids: vec!["opt_0".to_string()],
            free_text: None,
            answers: vec![],
        },
    )
    .await;
    let checkpoint = recv_until_message_complete(&mut secondary).await;
    assert!(checkpoint.starts_with("cp_"));

    drop(secondary);
    drop(primary);
    server.abort();
}

#[tokio::test]
async fn workspace_ws_stale_choice_response_after_new_run_is_rejected_before_provider() {
    let root = tempdir().expect("root");
    create_workspace_session_fixture(&root).await;
    let mut registry = ProviderRegistry::new();
    registry.register(
        ProviderName::Fake,
        Arc::new(SequencedChoiceCompletingProvider::default()),
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
        &WsInMessage::UserMessage {
            content: "run first choice provider".to_string(),
        },
    )
    .await;
    let first_choice = recv_until_choice_request(&mut ws).await;

    send_json(
        &mut ws,
        &WsInMessage::UserMessage {
            content: "replace with second choice provider".to_string(),
        },
    )
    .await;
    let second_choice = recv_until_choice_request(&mut ws).await;
    assert_ne!(first_choice.id, second_choice.id);

    send_json(
        &mut ws,
        &WsInMessage::ChoiceResponse {
            id: first_choice.id.clone(),
            selected_option_ids: vec!["opt_0".to_string()],
            free_text: None,
            answers: vec![],
        },
    )
    .await;

    match recv_until_protocol_error(&mut ws).await {
        WsOutMessage::ProtocolError { code, message, .. } => {
            assert_eq!(code, "CHOICE_ID_UNMATCHED");
            assert!(message.contains(&first_choice.id));
        }
        other => panic!("expected protocol_error, got {other:?}"),
    }

    send_json(
        &mut ws,
        &WsInMessage::ChoiceResponse {
            id: second_choice.id,
            selected_option_ids: vec!["opt_0".to_string()],
            free_text: None,
            answers: vec![],
        },
    )
    .await;
    let checkpoint = recv_until_message_complete(&mut ws).await;
    assert!(checkpoint.starts_with("cp_"));

    drop(ws);
    server.abort();
}
