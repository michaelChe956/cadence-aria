use cadence_aria::web::workspace_session::WorkspaceSessionRegistry;

/// 逐步放行流式文本，供 cursor 回放与活跃 run 窗口恢复用例精确控制事件窗口。
struct GatedChunkStreamingProvider {
    step: Arc<Notify>,
    complete: Arc<Notify>,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for GatedChunkStreamingProvider {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let (event_tx, event_rx) = mpsc::channel(8);
        let (command_tx, _command_rx) = mpsc::channel::<ProviderCommand>(8);
        let step = self.step.clone();
        let complete = self.complete.clone();
        tokio::spawn(async move {
            let mut index = 0_u64;
            let _ = event_tx
                .send(ProviderEvent::TextDelta {
                    content: format!("gated chunk {index}"),
                })
                .await;
            loop {
                tokio::select! {
                    _ = cancel.cancelled() => return,
                    _ = step.notified() => {
                        index += 1;
                        if event_tx.send(ProviderEvent::TextDelta {
                            content: format!("gated chunk {index}"),
                        }).await.is_err() {
                            return;
                        }
                    }
                    _ = complete.notified() => {
                        let _ = event_tx.send(ProviderEvent::Completed(
                            cadence_aria::cross_cutting::streaming_provider::ProviderCompletion::plain(
                                VALID_STORY_SPEC.to_string(),
                                None,
                            ),
                        )).await;
                        return;
                    }
                }
            }
        });
        Ok(ProviderSession {
            native_session_id: None,
            events: event_rx,
            commands: command_tx,
        })
    }

    async fn run_streaming(
        &self,
        _input: &AdapterInput,
        _cancel: CancellationToken,
    ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
        Err(ProviderAdapterError::execution_failed(
            None,
            String::new(),
            "run_streaming is not used by workspace websocket",
            0,
        ))
    }
}

/// REQ-WCR-04：携带可回放 cursor 的重连仅收到严格晚于 cursor 的事件，且 attach
/// snapshot 基线不得先于回放帧到达。
#[tokio::test]
async fn workspace_ws_reconnect_with_cursor_replays_without_snapshot_baseline() {
    let root = tempdir().expect("root");
    let _repo = create_workspace_session_fixture(&root).await;
    let step = Arc::new(Notify::new());
    let complete = Arc::new(Notify::new());
    let mut registry = ProviderRegistry::new();
    registry.register(
        ProviderName::Fake,
        Arc::new(GatedChunkStreamingProvider {
            step: step.clone(),
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

    let (mut ws, _) = connect_async(url.clone()).await.expect("ws");
    assert_eq!(recv_json_value(&mut ws).await["type"], "session_state");
    send_json(
        &mut ws,
        &WsInMessage::UserMessage {
            content: long_message("cursor_replay"),
        },
    )
    .await;
    let initial = recv_until_stream_chunk_value(&mut ws).await;
    let cursor = initial["event_seq"].as_u64().expect("initial chunk seq");

    drop(ws);
    for _ in 0..3 {
        step.notify_one();
        tokio::time::sleep(Duration::from_millis(30)).await;
    }

    let (mut ws2, _) = connect_async(url.clone()).await.expect("reconnect");
    send_json(
        &mut ws2,
        &WsInMessage::Hello {
            session_id: "workspace_session_0001".into(),
            last_seen_node_id: None,
            role: Some(HelloRole::Driver),
            after_event_seq: Some(cursor),
        },
    )
    .await;

    let mut replayed = Vec::new();
    for _ in 0..3 {
        let message = recv_json_value(&mut ws2).await;
        assert_ne!(
            message["type"], "session_state",
            "cursor 重连的回放前不得注入 snapshot 基线"
        );
        assert_eq!(message["type"], "stream_chunk");
        replayed.push(
            message["content"]
                .as_str()
                .expect("chunk content")
                .to_string(),
        );
    }
    assert_eq!(
        replayed,
        vec!["gated chunk 1", "gated chunk 2", "gated chunk 3"],
        "断连窗口严格回放一次且顺序不变"
    );

    step.notify_one();
    assert_eq!(
        recv_until_stream_chunk_value(&mut ws2).await["content"],
        "gated chunk 4",
        "回放结束后无缝续接直播"
    );
    complete.notify_one();
    drop(ws2);
    server.abort();
}

/// REQ-WCR-04/F2：cursor 已落在上一个 run 的尾窗外时，活跃 run 不能只发送快照；
/// 它必须以本 run 窗口起点减一建立基线并补发已产生的流式文本。
#[tokio::test]
async fn workspace_ws_stale_cursor_during_active_run_replays_current_run_window() {
    let root = tempdir().expect("root");
    let _repo = create_workspace_session_fixture(&root).await;
    let step = Arc::new(Notify::new());
    let complete = Arc::new(Notify::new());
    let mut registry = ProviderRegistry::new();
    registry.register(
        ProviderName::Fake,
        Arc::new(GatedChunkStreamingProvider {
            step: step.clone(),
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

    let (mut ws, _) = connect_async(url.clone()).await.expect("ws");
    let _initial = recv_json_value(&mut ws).await;
    send_json(
        &mut ws,
        &WsInMessage::UserMessage {
            content: long_message("stale_cursor_active_run"),
        },
    )
    .await;
    let old_cursor = recv_until_stream_chunk_value(&mut ws).await["event_seq"]
        .as_u64()
        .expect("old cursor");

    for _ in 0..1_030 {
        step.notify_one();
        let _ = recv_until_stream_chunk_value(&mut ws).await;
    }
    complete.notify_one();
    for _ in 0..40 {
        if recv_json_value(&mut ws).await["type"] == "message_complete" {
            break;
        }
    }

    send_json(
        &mut ws,
        &WsInMessage::UserMessage {
            content: long_message("active_run_window"),
        },
    )
    .await;
    let _run_two_initial = recv_until_stream_chunk_value(&mut ws).await;
    step.notify_one();
    let _run_two_second = recv_until_stream_chunk_value(&mut ws).await;
    drop(ws);
    let (mut ws2, _) = connect_async(url.clone()).await.expect("reconnect");
    send_json(
        &mut ws2,
        &WsInMessage::Hello {
            session_id: "workspace_session_0001".into(),
            last_seen_node_id: None,
            role: Some(HelloRole::Driver),
            after_event_seq: Some(old_cursor),
        },
    )
    .await;

    let baseline = recv_json_value(&mut ws2).await;
    assert_eq!(baseline["type"], "session_state");
    let baseline_seq = baseline["event_seq"]
        .as_u64()
        .expect("snapshot baseline seq");
    let first_replayed = recv_until_stream_chunk_value(&mut ws2).await;
    assert!(
        baseline_seq < first_replayed["event_seq"].as_u64().expect("replayed seq"),
        "活跃 run 快照基线不得抬高至补发事件之后"
    );
    assert_eq!(first_replayed["content"], "gated chunk 0");
    assert_eq!(
        recv_until_stream_chunk_value(&mut ws2).await["content"],
        "gated chunk 1",
        "当前活跃 run 已流出的第二条文本也必须补发"
    );

    step.notify_one();
    assert_eq!(
        recv_until_stream_chunk_value(&mut ws2).await["content"],
        "gated chunk 2",
        "补发后继续接收直播"
    );
    complete.notify_one();
    drop(ws2);
    server.abort();
}

/// REQ-WCR-04：非活跃 session 的过旧 cursor 退化为当前 event_seq 的 snapshot 基线，
/// 之后连接继续接收新的直播帧。
#[tokio::test]
async fn workspace_ws_stale_cursor_without_active_run_falls_back_to_snapshot_baseline() {
    let root = tempdir().expect("root");
    let _repo = create_workspace_session_fixture(&root).await;
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move { axum::serve(listener, app).await.expect("serve") });
    let url = format!("ws://{addr}/api/workspace-sessions/workspace_session_0001/ws");

    let (mut ws, _) = connect_async(url.clone()).await.expect("ws");
    let initial = recv_json_value(&mut ws).await;
    let baseline = initial["event_seq"].as_u64().expect("initial baseline");
    drop(ws);

    let (mut ws2, _) = connect_async(url).await.expect("reconnect");
    send_json(
        &mut ws2,
        &WsInMessage::Hello {
            session_id: "workspace_session_0001".into(),
            last_seen_node_id: None,
            role: Some(HelloRole::Driver),
            after_event_seq: Some(baseline.saturating_add(1)),
        },
    )
    .await;
    let snapshot = recv_json_value(&mut ws2).await;
    assert_eq!(snapshot["type"], "session_state");
    assert_eq!(snapshot["event_seq"], baseline);

    drop(ws2);
    server.abort();
}
/// REQ-WCR-04：cursor 已确认原 choice 事件时，重连仍须补发 pending 门卡，
/// 否则前端丢失门卡后 run 将永远等待选择。
#[tokio::test]
async fn workspace_ws_cursor_reconnect_replays_pending_choice_after_replay_window() {
    let root = tempdir().expect("root");
    create_workspace_session_fixture(&root).await;
    let provider_state = Arc::new(ChoiceThenArtifactProviderState::default());
    let mut registry = ProviderRegistry::new();
    registry.register(
        ProviderName::Fake,
        Arc::new(ChoiceThenArtifactProvider {
            state: provider_state,
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

    let (mut ws, _) = connect_async(url.clone()).await.expect("ws");
    assert_eq!(recv_json_value(&mut ws).await["type"], "session_state");
    send_json(
        &mut ws,
        &WsInMessage::UserMessage {
            content: "开始生成".to_string(),
        },
    )
    .await;
    let choice = loop {
        let message = recv_json_value(&mut ws).await;
        if message["type"] == "choice_request" {
            break message;
        }
    };
    let cursor = choice["event_seq"].as_u64().expect("choice event seq");
    let choice_id = choice["id"].as_str().expect("choice id").to_string();

    let (mut ws2, _) = connect_async(url).await.expect("reconnect");
    send_json(
        &mut ws2,
        &WsInMessage::Hello {
            session_id: "workspace_session_0001".into(),
            last_seen_node_id: None,
            role: Some(HelloRole::Driver),
            after_event_seq: Some(cursor),
        },
    )
    .await;
    let replayed_choice = recv_json_value(&mut ws2).await;
    assert_eq!(replayed_choice["type"], "choice_request");
    assert_eq!(replayed_choice["id"], choice_id);
    assert!(
        replayed_choice.get("event_seq").is_none(),
        "重发门卡不属于 journal 回放事件，不能干扰 cursor 去重"
    );

    drop(ws2);
    server.abort();
}

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
        assert!(matches!(
            recv_json(ws).await,
            WsOutMessage::SessionState { .. }
        ));
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
    assert!(matches!(
        recv_json(&mut driver).await,
        WsOutMessage::SessionState { .. }
    ));
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
    assert!(
        run_before.is_some(),
        "driver run must be registered before observer attaches"
    );

    let attach_started = tokio::time::Instant::now();
    let (mut observer, _) = connect_async(url.clone()).await.expect("observer ws");
    let state_message = tokio::time::timeout(Duration::from_secs(2), recv_json(&mut observer))
        .await
        .expect("run 进行中第二连接 attach 被阻塞超过 2s");
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
    assert!(matches!(
        recv_json(&mut driver).await,
        WsOutMessage::SessionState { .. }
    ));
    send_json(
        &mut driver,
        &WsInMessage::UserMessage {
            content: long_message("broadcast_to_passive"),
        },
    )
    .await;
    let _chunk = recv_until_stream_chunk(&mut driver).await;

    let (mut observer, _) = connect_async(url.clone()).await.expect("observer ws");
    assert!(matches!(
        recv_json(&mut observer).await,
        WsOutMessage::SessionState { .. }
    ));
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

/// REQ-WCR-02：显式 observer 的写命令必须在进入 engine 前被拒绝；缺席 role 的
/// legacy driver 兼容语义由 part_03 的 secondary abort 用例持续覆盖。
#[tokio::test]
async fn workspace_ws_observer_write_commands_are_rejected_and_run_untouched() {
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
    let _state = recv_json(&mut driver).await;
    send_json(
        &mut driver,
        &WsInMessage::UserMessage {
            content: long_message("observer_reject_probe"),
        },
    )
    .await;
    let _chunk = recv_until_stream_chunk(&mut driver).await;

    let (mut observer, _) = connect_async(url.clone()).await.expect("observer");
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
    let _snapshot = recv_json(&mut observer).await;

    for message in [
        WsInMessage::Abort,
        WsInMessage::UserMessage {
            content: "observer write".into(),
        },
        WsInMessage::Advance {
            command_id: "cmd-obs-1".into(),
        },
        WsInMessage::ContextNote {
            content: "future write family defaults to rejected".into(),
        },
    ] {
        send_json(&mut observer, &message).await;
        match recv_json(&mut observer).await {
            WsOutMessage::ProtocolError {
                code,
                context: Some(context),
                ..
            } => {
                assert_eq!(code, "OBSERVER_WRITE_REJECTED");
                assert_eq!(context["role"], "observer");
                assert_eq!(context["received"], message_type_for_test(&message));
            }
            other => panic!("observer write must be rejected diagnostically, got {other:?}"),
        }
    }

    complete.notify_one();
    let mut completed = false;
    for _ in 0..200 {
        match recv_json(&mut driver).await {
            WsOutMessage::MessageComplete { .. } | WsOutMessage::StageChange { .. } => {
                completed = true;
                break;
            }
            WsOutMessage::Error { message } => panic!("driver ws error: {message}"),
            _ => continue,
        }
    }
    assert!(completed, "observer 被拒写不得影响 driver run 至完成");

    drop(observer);
    drop(driver);
    server.abort();
}

fn message_type_for_test(message: &WsInMessage) -> &'static str {
    match message {
        WsInMessage::Abort => "abort",
        WsInMessage::UserMessage { .. } => "user_message",
        WsInMessage::Advance { .. } => "advance",
        WsInMessage::ContextNote { .. } => "context_note",
        _ => unreachable!("test only constructs observer write commands"),
    }
}

fn durable_tree_snapshot(root: &std::path::Path) -> Vec<(std::path::PathBuf, Vec<u8>)> {
    fn visit(
        base: &std::path::Path,
        current: &std::path::Path,
        out: &mut Vec<(std::path::PathBuf, Vec<u8>)>,
    ) {
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
                    path.strip_prefix(base)
                        .expect("relative durable path")
                        .to_path_buf(),
                    std::fs::read(&path).unwrap_or_else(|error| {
                        panic!("read durable file {}: {error}", path.display())
                    }),
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

/// 以历史 durable 形状植入断连标记，验证读取、往返和关闭都不会把历史事实改写成
/// 新一次断连终态。
fn seed_historical_aborted_marker(root: &TempDir) {
    let lifecycle = LifecycleStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let mut nodes = lifecycle
        .load_timeline_nodes("workspace_session_0001")
        .expect("load existing timeline nodes");
    nodes.push(cadence_aria::web::workspace_ws_types::TimelineNode {
        node_id: "timeline_node_historical_disconnect".to_string(),
        node_type: TimelineNodeType::AbortedByDisconnect,
        agent: None,
        stage: cadence_aria::web::workspace_ws_types::WorkspaceStage::PrepareContext,
        round: None,
        status: TimelineNodeStatus::Failed,
        title: "历史连接断开".to_string(),
        summary: Some("连接断开，运行已中止".to_string()),
        started_at: "2026-01-01T00:00:00Z".to_string(),
        completed_at: Some("2026-01-01T00:00:01Z".to_string()),
        duration_ms: Some(1),
        artifact_ref: None,
        provider_config_snapshot: ProviderConfigSnapshot {
            author: ProviderName::Fake,
            reviewer: None,
            review_rounds: 0,
            permission_modes: cadence_aria::product::models::WorkspaceRolePermissionModes::default(
            ),
        },
        retry: None,
    });
    lifecycle
        .save_timeline_nodes("workspace_session_0001", &nodes)
        .expect("seed historical disconnect marker");
}

/// REQ-WCR-03：provider 完成和服务端关闭清理并发发生时，关闭路径不得追加
/// `aborted_by_disconnect`；唯一 terminal 来自业务完成。
#[tokio::test]
async fn workspace_ws_completion_and_close_race_yields_single_terminal() {
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

    let (mut ws, _) = connect_async(url).await.expect("ws");
    let _initial = recv_json(&mut ws).await;
    send_json(
        &mut ws,
        &WsInMessage::UserMessage {
            content: long_message("race_terminal"),
        },
    )
    .await;
    let _chunk = recv_until_stream_chunk(&mut ws).await;

    complete.notify_one();
    assert!(
        state
            .test_controls
            .drop_workspace_socket("workspace_session_0001")
            .await,
        "完成与关闭必须在同一活动 run 窗口交错"
    );
    drop(ws);

    let mut terminal = false;
    for _ in 0..100 {
        tokio::time::sleep(Duration::from_millis(50)).await;
        let nodes = LifecycleStore::new(ProductAppPaths::new(root.path().join(".aria")))
            .load_timeline_nodes("workspace_session_0001")
            .expect("timeline nodes");
        assert!(
            nodes
                .iter()
                .all(|node| node.node_type != TimelineNodeType::AbortedByDisconnect),
            "完成与关闭并发不得写入断连伪终态"
        );
        if persisted_workspace_messages(root.path())
            .iter()
            .any(|message| message.role == "assistant" && message.content.contains("# Story Spec"))
        {
            terminal = true;
            break;
        }
    }
    assert!(terminal, "provider 完成必须落真实业务终态");
    server.abort();
}

/// REQ-WCR-06：存量 `aborted_by_disconnect` 是不可变历史事实；新会话必须照常服务，
/// 且关闭后既不误判也不改写该 durable 标记。
#[tokio::test]
async fn workspace_ws_historical_disconnect_marker_is_preserved_not_misjudged() {
    let (_lock, _controls_env) = ConnectionDiagnosticTestControlsGuard::enable().await;
    let root = tempdir().expect("root");
    let _repo = create_workspace_session_fixture(&root).await;
    seed_historical_aborted_marker(&root);

    let state = WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    );
    let controls = state.test_controls.clone();
    let app = build_web_router(state);
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move { axum::serve(listener, app).await.expect("serve") });
    let url = format!("ws://{addr}/api/workspace-sessions/workspace_session_0001/ws");

    let (mut ws, _) = connect_async(url).await.expect("ws");
    match recv_json(&mut ws).await {
        WsOutMessage::SessionState { timeline_nodes, .. } => {
            assert!(
                timeline_nodes
                    .iter()
                    .any(|node| node.node_type == TimelineNodeType::AbortedByDisconnect),
                "历史标记必须原样呈现，而非被当作当前连接终态"
            );
        }
        other => panic!("expected session_state, got {other:?}"),
    }
    let durable_after_read = durable_tree_snapshot(root.path());

    send_json(&mut ws, &WsInMessage::Ping).await;
    assert!(matches!(recv_json(&mut ws).await, WsOutMessage::Pong));
    drop(ws);
    let _diagnostic = wait_for_connection_diagnostic(&controls, "eof").await;

    assert_eq!(
        durable_tree_snapshot(root.path()),
        durable_after_read,
        "正常往返和关闭不得改写含历史标记的 durable"
    );
    let nodes = LifecycleStore::new(ProductAppPaths::new(root.path().join(".aria")))
        .load_timeline_nodes("workspace_session_0001")
        .expect("timeline nodes");
    assert_eq!(
        nodes
            .iter()
            .filter(|node| node.node_type == TimelineNodeType::AbortedByDisconnect)
            .count(),
        1,
        "历史断连标记必须保持单个，不得被误判为新标记"
    );
    server.abort();
}

/// REQ-WCR-03（0437 形态根治）：human_confirm 门刚开启即断连，
/// 活动节点不得被标「连接断开，运行已中止」，门等待正常呈现。
#[tokio::test]
async fn workspace_ws_close_right_after_gate_opens_does_not_mark_failed() {
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
    let server = tokio::spawn(async move { axum::serve(listener, app).await.expect("serve") });
    let url = format!("ws://{addr}/api/workspace-sessions/workspace_session_0001/ws");

    let (mut ws, _) = connect_async(url.clone()).await.expect("ws");
    let _initial = recv_json(&mut ws).await;
    send_json(
        &mut ws,
        &WsInMessage::UserMessage {
            content: long_message("gate_window_close"),
        },
    )
    .await;
    let _checkpoint = recv_until_message_complete(&mut ws).await;
    accept_author_output(&mut ws).await; // → human_confirm 门开
    assert_eq!(
        recv_until_stage(&mut ws, "human_confirm").await,
        "human_confirm"
    );
    drop(ws); // 门开即刻断连（0437 窗口）
    let _diagnostic = wait_for_connection_diagnostic(&controls, "eof").await;

    // 重连：门等待正常呈现，无断连审计/无 Failed 覆盖/无 prepare_context 整流。
    let (mut probe, _) = connect_async(url).await.expect("probe");
    match recv_json(&mut probe).await {
        WsOutMessage::SessionState {
            stage,
            timeline_nodes,
            active_run_id,
            session_status,
            ..
        } => {
            assert_ne!(
                stage, "prepare_context",
                "断连不得把 session 整流回 prepare_context"
            );
            assert_eq!(
                session_status,
                cadence_aria::product::models::WorkspaceSessionStatus::WaitingForHuman
            );
            assert_eq!(active_run_id, None);
            assert!(
                timeline_nodes
                    .iter()
                    .all(|node| node.node_type != TimelineNodeType::AbortedByDisconnect)
            );
            let last = timeline_nodes.last().expect("node");
            assert_ne!(
                last.status,
                TimelineNodeStatus::Failed,
                "门刚开即断连不得把活动节点标 Failed（0437）"
            );
        }
        other => panic!("expected session_state, got {other:?}"),
    }
    drop(probe);
    server.abort();
}

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
    match recv_json(&mut second_driver).await {
        WsOutMessage::ProviderStatus { status } => assert_eq!(status, WsProviderStatus::Aborted),
        other => panic!("lease holder abort should cancel the active run, got {other:?}"),
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
