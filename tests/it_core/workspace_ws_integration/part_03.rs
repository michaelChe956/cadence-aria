#[tokio::test]
async fn workspace_ws_reconnect_restores_message_checkpoint_ids() {
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
    let (mut ws, _) = connect_async(url.clone()).await.expect("connect ws");
    let _initial = recv_json(&mut ws).await;

    send_json(
        &mut ws,
        &WsInMessage::UserMessage {
            content: "restore checkpoint ids".to_string(),
        },
    )
    .await;
    let checkpoint_id = recv_until_message_complete(&mut ws).await;
    accept_author_output(&mut ws).await;
    let _ = recv_until_stage(&mut ws, "human_confirm").await;
    drop(ws);

    let (mut reconnected, _) = connect_async(url).await.expect("reconnect ws");
    let reloaded = recv_json(&mut reconnected).await;
    match reloaded {
        WsOutMessage::SessionState { messages, .. } => {
            let assistant = messages
                .iter()
                .find(|message| message.role == "assistant")
                .expect("assistant message");
            assert_eq!(
                assistant.checkpoint_id.as_deref(),
                Some(checkpoint_id.as_str())
            );
        }
        other => panic!("expected session_state, got {other:?}"),
    }

    drop(reconnected);
    server.abort();
}

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

#[tokio::test]
async fn workspace_ws_disconnect_during_active_run_writes_aborted_by_disconnect() {
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
    let (mut ws, _) = connect_async(url.clone()).await.expect("connect ws");
    let _initial = recv_json(&mut ws).await;

    send_json(
        &mut ws,
        &WsInMessage::UserMessage {
            content: long_message("disconnect_instruction"),
        },
    )
    .await;
    let _first_chunk = recv_until_stream_chunk(&mut ws).await;
    drop(ws);

    // 断连清理不再取消 run：fake provider 驱动至自然完成后，断连清理才拿到
    // engine 锁追加 aborted_by_disconnect 审计节点——轮询重连直至审计节点落盘
    // （旧行为是取消后立即落盘，两条路径部应收敛到同一审计终态）。
    let mut verified = false;
    for _ in 0..100 {
        let (mut probe, _) = connect_async(url.clone()).await.expect("probe ws");
        match recv_json(&mut probe).await {
            WsOutMessage::SessionState {
                stage,
                timeline_nodes,
                active_run_id,
                ..
            } => {
                let last = timeline_nodes.last().expect("timeline node");
                if last.node_type == TimelineNodeType::AbortedByDisconnect {
                    assert_eq!(stage, "prepare_context");
                    assert_eq!(active_run_id, None);
                    assert_eq!(last.status, TimelineNodeStatus::Failed);
                    assert!(
                        last.summary
                            .as_deref()
                            .is_some_and(|summary| summary.contains("run-1"))
                    );
                    verified = true;
                }
            }
            other => panic!("expected session_state, got {other:?}"),
        }
        drop(probe);
        if verified {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        verified,
        "断连后 aborted_by_disconnect 审计节点应在 run 结束后落盘且语义保留"
    );

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

#[tokio::test]
async fn workspace_ws_secondary_connection_can_abort_active_run_started_by_primary() {
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

#[tokio::test]
async fn workspace_ws_supervised_permission_allows_real_stream_to_complete() {
    let root = tempdir().expect("root");
    let _repo = create_workspace_session_fixture_with_author(&root, "claude_code").await;
    set_workspace_author_permission_mode_to_supervised(&root);
    let mut registry = ProviderRegistry::new();
    registry.register(ProviderName::Fake, Arc::new(FakeStreamingProvider));
    registry.register(
        ProviderName::ClaudeCode,
        Arc::new(ClaudeCodeProvider::new(executable_fixture(
            "tests/fixtures/provider/claude_stream_json_fixture.sh",
        ))),
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
            content: "run supervised provider".to_string(),
        },
    )
    .await;

    let permission = recv_until_permission_request(&mut ws).await;
    assert_eq!(permission.tool_name, "Bash");

    send_json(
        &mut ws,
        &WsInMessage::PermissionResponse {
            id: permission.id,
            approved: true,
            reason: None,
        },
    )
    .await;

    let checkpoint = recv_until_message_complete(&mut ws).await;
    assert!(checkpoint.starts_with("cp_"));
    accept_author_output(&mut ws).await;
    let stage = recv_until_stage(&mut ws, "human_confirm").await;
    assert_eq!(stage, "human_confirm");

    drop(ws);
    server.abort();
}

#[tokio::test]
async fn workspace_ws_claude_author_ask_user_question_choice_continues_same_provider() {
    let root = tempdir().expect("root");
    let _repo = create_workspace_session_fixture_with_author(&root, "claude_code").await;
    let mut registry = ProviderRegistry::new();
    registry.register(ProviderName::Fake, Arc::new(FakeStreamingProvider));
    registry.register(
        ProviderName::ClaudeCode,
        Arc::new(ClaudeCodeProvider::new(executable_fixture(
            "tests/fixtures/provider/claude_ask_user_question_fixture.sh",
        ))),
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
            content: "run claude ask user question provider".to_string(),
        },
    )
    .await;

    let choice = recv_until_choice_request(&mut ws).await;
    assert_eq!(choice.source.as_deref(), Some("ask_user_question"));
    assert_eq!(choice.id, "ask_req_001");
    assert_eq!(choice.prompt, "Drink?");
    assert_eq!(choice.options[0].id, "opt_0");
    assert_eq!(choice.options[0].label, "Tea");

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
    accept_author_output(&mut ws).await;
    let stage = recv_until_stage(&mut ws, "human_confirm").await;
    assert_eq!(stage, "human_confirm");

    drop(ws);
    server.abort();
}

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
