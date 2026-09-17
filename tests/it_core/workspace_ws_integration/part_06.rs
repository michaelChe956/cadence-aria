use cadence_aria::web::workspace_session::WorkspaceSessionRegistry;

/// T1 单实例：多个 attachment 必须复用同一个 session-owned manager/engine。
#[tokio::test]
async fn workspace_ws_multiple_connections_share_one_session_manager() {
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
    let mut sockets = Vec::new();
    for _ in 0..3 {
        let (ws, _) = connect_async(url.clone()).await.expect("connect ws");
        sockets.push(ws);
    }
    for ws in &mut sockets {
        assert!(matches!(recv_json(ws).await, WsOutMessage::SessionState { .. }));
    }
    assert_eq!(
        state.workspace_sessions.session_ids().await,
        vec!["workspace_session_0001".to_string()],
        "三个连接必须共享同一 session manager（engine 单实例）"
    );

    drop(sockets);
    server.abort();
}

/// T1/F3：run 持有唯一 engine 时，第二连接仍须由只读 durable 投影器在时限内 attach。
#[tokio::test]
async fn workspace_ws_second_connection_attaches_while_run_holds_engine_lock() {
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
    let (mut driver, _) = connect_async(url.clone()).await.expect("driver ws");
    assert!(matches!(recv_json(&mut driver).await, WsOutMessage::SessionState { .. }));
    send_json(
        &mut driver,
        &WsInMessage::UserMessage {
            content: long_message("attach_during_run"),
        },
    )
    .await;
    let _chunk = recv_until_stream_chunk(&mut driver).await;

    let durable_before = durable_tree_snapshot(root.path());
    let manager = state
        .workspace_sessions
        .get("workspace_session_0001")
        .await
        .expect("driver attach creates session manager");
    let run_before = manager.active_run().await.map(|run| run.token);
    assert!(run_before.is_some(), "driver run must be registered before observer attaches");

    let attach_started = tokio::time::Instant::now();
    let (mut observer, _) = connect_async(url.clone()).await.expect("observer ws");
    let state_message = tokio::time::timeout(Duration::from_secs(2), recv_json(&mut observer))
        .await
        .expect("run 进行中第二连接 attach 被阻塞超过 2s") ;
    assert!(matches!(state_message, WsOutMessage::SessionState { .. }));
    assert!(attach_started.elapsed() < Duration::from_secs(2));

    assert_eq!(state.workspace_sessions.session_ids().await.len(), 1);
    assert_eq!(
        durable_tree_snapshot(root.path()),
        durable_before,
        "投影器必须是零 durable 写入的纯渲染器"
    );
    assert_eq!(
        manager.active_run().await.map(|run| run.token),
        run_before,
        "投影器不得触发 start_run/supersede（不入 run 面）"
    );
    assert!(
        state
            .workspace_runs
            .provider_drive_in_progress("workspace_session_0001"),
        "drive-depth 仍为 driver 单一 run（投影器不注册 provider drive）"
    );

    complete.notify_one();
    drop(observer);
    drop(driver);
    server.abort();
}

/// T1：engine runtime 事件必须 fan-out 到所有在线 attachment，而非只发起 run 的 socket。
#[tokio::test]
async fn workspace_ws_passive_connection_receives_live_stream_events() {
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
    let (mut driver, _) = connect_async(url.clone()).await.expect("driver ws");
    assert!(matches!(recv_json(&mut driver).await, WsOutMessage::SessionState { .. }));
    send_json(
        &mut driver,
        &WsInMessage::UserMessage {
            content: long_message("broadcast_to_passive"),
        },
    )
    .await;
    let _chunk = recv_until_stream_chunk(&mut driver).await;

    let (mut observer, _) = connect_async(url.clone()).await.expect("observer ws");
    assert!(matches!(recv_json(&mut observer).await, WsOutMessage::SessionState { .. }));
    complete.notify_one();

    let live_event = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match recv_json(&mut observer).await {
                message @ (WsOutMessage::MessageComplete { .. }
                | WsOutMessage::StageChange { .. }
                | WsOutMessage::StreamChunk { .. }) => return message,
                WsOutMessage::Error { message } => panic!("observer ws error: {message}"),
                _ => {}
            }
        }
    })
    .await
    .expect("被动连接未在 10s 内收到实时事件（广播缺失）");
    assert!(matches!(
        live_event,
        WsOutMessage::MessageComplete { .. }
            | WsOutMessage::StageChange { .. }
            | WsOutMessage::StreamChunk { .. }
    ));

    drop(observer);
    drop(driver);
    server.abort();
}

fn durable_tree_snapshot(root: &std::path::Path) -> Vec<(std::path::PathBuf, Vec<u8>)> {
    fn visit(base: &std::path::Path, current: &std::path::Path, out: &mut Vec<(std::path::PathBuf, Vec<u8>)>) {
        let mut entries = std::fs::read_dir(current)
            .unwrap_or_else(|error| panic!("read durable directory {}: {error}", current.display()))
            .collect::<Result<Vec<_>, _>>()
            .unwrap_or_else(|error| panic!("read durable entry {}: {error}", current.display()));
        entries.sort_by_key(|entry| entry.path());
        for entry in entries {
            let path = entry.path();
            if path.is_dir() {
                visit(base, &path, out);
            } else {
                out.push((
                    path.strip_prefix(base).expect("relative durable path").to_path_buf(),
                    std::fs::read(&path)
                        .unwrap_or_else(|error| panic!("read durable file {}: {error}", path.display())),
                ));
            }
        }
    }

    let durable = root.join(".aria");
    let mut files = Vec::new();
    visit(&durable, &durable, &mut files);
    files
}

#[allow(dead_code)]
fn _registry_type_is_public(_: WorkspaceSessionRegistry) {}
