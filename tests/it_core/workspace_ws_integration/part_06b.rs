// T3：run 在最后一个 attachment 断开后完成时，manager 必须从 registry 回收；再次
// attach 必须从 durable 重建，且不得新增断连终态审计节点。
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

// REQ-WCR-02/REQ-DLS-04：第二连接经显式 hello（无 role 缺席归一为 Driver）
// 接管会话 lease——attach 对租约零效应后这是唯一接管路径；被接管的旧连接
// 迟到写必须以可诊断的协议错误拒绝，而当前 holder 仍可中止同一活动 run。
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
    // REQ-DLS-04：第二连接必须显式 hello 才能接管（多 tab 驾驶切换语义保持）。
    send_json(
        &mut second_driver,
        &WsInMessage::Hello {
            session_id: "workspace_session_0001".into(),
            last_seen_node_id: None,
            role: None,
            after_event_seq: None,
        },
    )
    .await;
    // hello 的 session_state 回复在活跃 run 持有 engine 锁期间不可达；同连接
    // 读循环顺序处理入站，Pong 返回即 hello（bind_role 接管）已生效。
    send_json(&mut second_driver, &WsInMessage::Ping).await;
    loop {
        match recv_json(&mut second_driver).await {
            WsOutMessage::Pong => break,
            WsOutMessage::Error { message } => panic!("second driver ws error: {message}"),
            _ => continue,
        }
    }

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

// REQ-WCR-02：driver close 只撤销连接持有型 lease，不取消正在运行的 provider，
// 后续真实完成仍写入业务终态且不产生断连中止 marker。
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

// REQ-WCR-02/F1：显式 observer 的 Hello 不得接管 driver lease；读面的 initial
// snapshot 仍可用，而现任 driver 必须继续能够中止 run。
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

// REQ-DLS-01：悬空自愈——驾驶连接的租约被并发连接占走后悬空（holder=None，
// F-50-1 现场：偷窃连接断开），原连接的首条写消息必须无感恢复：不产生
// STALE_DRIVER_LEASE，操作直接生效（UserMessage 走 run 启动全路径）。
#[tokio::test]
async fn workspace_ws_dangling_lease_self_heals_on_first_write() {
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

    let (mut driver, _) = connect_async(url.clone()).await.expect("driver");
    let _initial = recv_json(&mut driver).await;
    send_json(
        &mut driver,
        &WsInMessage::UserMessage {
            content: long_message("dangling_self_heal_first"),
        },
    )
    .await;
    let _chunk = recv_until_stream_chunk(&mut driver).await;

    // 偷窃连接显式接管后立刻悬空（F-50-1 引爆链在新形态下为：hello(driver)
    // 接管 → 断开 → 无人持有；REQ-DLS-04 后 attach 不再抢占，显式 hello 是
    // 并发连接占走租约的唯一路径）。
    let (mut thief, _) = connect_async(url).await.expect("thief");
    let _thief_initial = recv_json(&mut thief).await;
    send_json(
        &mut thief,
        &WsInMessage::Hello {
            session_id: "workspace_session_0001".into(),
            last_seen_node_id: None,
            role: Some(HelloRole::Driver),
            after_event_seq: None,
        },
    )
    .await;
    // 同连接读循环顺序：Pong 返回即 hello 接管已生效（run 持锁时
    // session_state 回复不可达，不能用其做屏障）。
    send_json(&mut thief, &WsInMessage::Ping).await;
    loop {
        match recv_json(&mut thief).await {
            WsOutMessage::Pong => break,
            WsOutMessage::Error { message } => panic!("thief ws error: {message}"),
            _ => continue,
        }
    }
    drop(thief);
    let eof = wait_for_connection_diagnostic(&controls, "eof").await;
    assert!(
        eof["connection_id"]
            .as_str()
            .is_some_and(|id| !id.is_empty()),
        "eof 诊断必须先于自愈断言落到偷窃连接上"
    );

    // 原驾驶连接首写：不 STALE、不报错，run 直接启动并推进到流式输出。
    send_json(
        &mut driver,
        &WsInMessage::UserMessage {
            content: long_message("dangling_self_heal_second"),
        },
    )
    .await;
    let _healed_chunk = recv_until_stream_chunk(&mut driver).await;

    drop(driver);
    complete.notify_one();
    server.abort();
}

// REQ-DLS-03：只读诊断端点返回当前持有者与最近转移序列——偷窃可定案，
// 无需推测（F-50-1 诊断 §2.3 的结构性补面）。
#[tokio::test]
async fn workspace_ws_lease_diagnostics_endpoint_returns_holder_and_recent_events() {
    let root = tempdir().expect("root");
    let _repo = create_workspace_session_fixture(&root).await;
    let state = WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    );
    let app = build_web_router(state.clone());
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move { axum::serve(listener, app).await.expect("serve") });
    let url = format!("ws://{addr}/api/workspace-sessions/workspace_session_0001/ws");

    let (mut driver, _) = connect_async(url).await.expect("driver");
    let _initial = recv_json(&mut driver).await;
    send_json(
        &mut driver,
        &WsInMessage::Hello {
            session_id: "workspace_session_0001".into(),
            last_seen_node_id: None,
            role: Some(HelloRole::Driver),
            after_event_seq: None,
        },
    )
    .await;
    assert!(matches!(
        recv_json(&mut driver).await,
        WsOutMessage::SessionState { .. }
    ));

    let response = build_web_router(state)
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/workspace-sessions/workspace_session_0001/lease-diagnostics")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("endpoint response");
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("endpoint body");
    let payload: Value = serde_json::from_slice(&body).expect("endpoint json");
    assert!(
        payload["holder"].as_str().is_some_and(|holder| !holder.is_empty()),
        "端点必须暴露当前持有者连接，got {payload}"
    );
    let events = payload["events"].as_array().expect("events array");
    assert!(
        events
            .iter()
            .any(|event| event["event"] == "hold" && event["connection_id"].is_string()),
        "hello 显式获取必须可定案（hold 打点），got {events:?}"
    );

    drop(driver);
    server.abort();
}

// REQ-DLS-03：manager 已回收（无活跃连接）时，端点退化为 durable jsonl 尾读。
#[tokio::test]
async fn workspace_ws_lease_diagnostics_endpoint_reads_durable_tail_without_live_manager() {
    let root = tempdir().expect("root");
    let _repo = create_workspace_session_fixture(&root).await;
    let timelines_root = workspace_timelines_root(&root.path().join(".aria"));
    let diagnostics_path = timelines_root
        .join("workspace_session_0001")
        .join("lease-diagnostics.jsonl");
    std::fs::create_dir_all(diagnostics_path.parent().expect("parent")).expect("mkdir");
    std::fs::write(
        &diagnostics_path,
        concat!(
            "{\"schema_version\":1,\"event\":\"hold\",\"connection_id\":\"conn-1\",\"role\":\"driver\"}\n",
            "{\"schema_version\":1,\"event\":\"release\",\"connection_id\":\"conn-1\",\"role\":\"driver\"}\n",
        ),
    )
    .expect("seed diagnostics");

    let state = WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    );
    let response = build_web_router(state)
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/workspace-sessions/workspace_session_0001/lease-diagnostics")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("endpoint response");
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("endpoint body");
    let payload: Value = serde_json::from_slice(&body).expect("endpoint json");
    assert!(
        payload["holder"].is_null(),
        "无活跃 manager 时持有者为 null，got {payload}"
    );
    let events = payload["events"].as_array().expect("events array");
    assert_eq!(
        events.len(),
        2,
        "durable 尾读必须返回 jsonl 既有事件，got {events:?}"
    );
    assert_eq!(events[0]["event"], "hold");
    assert_eq!(events[1]["event"], "release");
}

fn workspace_timelines_root(aria_root: &std::path::Path) -> std::path::PathBuf {
    let projects = aria_root.join("projects");
    let project = fs::read_dir(&projects)
        .expect("projects dir")
        .next()
        .expect("project entry")
        .expect("project read")
        .path();
    let issues = project.join("issues");
    let issue = fs::read_dir(&issues)
        .expect("issues dir")
        .next()
        .expect("issue entry")
        .expect("issue read")
        .path();
    issue.join("workspace-timelines")
}

// REQ-WCR-04/T9：manager 只分配一次序号；attach 基线和所有在线 attachment 的直播
// 事件必须携带同一递增 `event_seq`，而非 socket-local 序号。
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

// REQ-WCR-04：一个不消费的 observer 不得反压 provider，快 observer 保持精确直播流。
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

// REQ-WCR-04：关闭单一 attachment 只摘除该连接，其他连接的事件流和 run 不受影响。
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

// RCA §6 矩阵②服务端半面：human_confirm 静默超过 idle 阈值后，服务器可以回收
// 连接，但不能写入任何终态；重连仍必须投影原来的门等待。
// 退役留档（T5/REQ-RET-02）：`matrix2_gate_silence_idle_close_writes_no_terminal` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

// RCA §6 矩阵③：所有可在 active run 期间由客户端触发的关闭形态都必须留下唯一的
// 中性诊断，并让 provider 自然写入业务终态；server-idle 在 active run 中由矩阵①
// 的 guard 禁止触发，空闲 gate 的 server-idle 回收由矩阵②覆盖。
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
