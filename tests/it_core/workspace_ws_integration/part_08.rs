// F-27（v33 复验 3）：choice 卡依赖单帧事件送达，挂起 choice 不在 session_state
// 全量投影里——degraded 恢复/attach 快照永远缺卡。本组测试锚定治本契约：
// session_state 帧顶层 `pending_choice_requests`（元素与 choice_request 帧字段
// 同构），provider pending（run 持锁窗口经 durable 投影）与 TextFallback
// pending（引擎 pending_author_choice，run 结束后活引擎）双来源统一入投影。


/// 直接以 author-choice 文本完成的 provider：Completed 内容命中
/// detect_author_choice_request（Story 面文本提问回退），驱动引擎走
/// TextFallback choice 路径（pending_author_choice，非 provider pending 集）。
struct TextFallbackChoiceCompletingProvider;

const TEXT_FALLBACK_CHOICE_OUTPUT: &str = "感谢提供项目上下文。\n\n\
    在生成 Story Spec 之前，我有几个问题需要确认：\n\n\
    **问题 1：弹窗触发时机**\n\n\
    根据 Issue 描述，弹窗是在\"启动 aria 后\"触发。请问这里的\"启动 aria\"具体指什么时机？\n\n\
    - **A)** 用户运行 `aria` 命令启动 daemon 时（Rust 后端启动时）\n\
    - **B)** 用户打开 Web 工作台页面时（前端首次加载时）\n\
    - **C)** 两者都需要（后端启动时检测，前端展示弹窗）\n";

#[async_trait::async_trait]
impl StreamingProviderAdapter for TextFallbackChoiceCompletingProvider {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, cadence_aria::cross_cutting::provider_adapter::ProviderAdapterError>
    {
        let (event_tx, event_rx) = mpsc::channel(8);
        let (command_tx, _command_rx) = mpsc::channel::<ProviderCommand>(8);
        tokio::spawn(async move {
            let _ = event_tx
                .send(ProviderEvent::TextDelta {
                    content: TEXT_FALLBACK_CHOICE_OUTPUT.to_string(),
                })
                .await;
            let _ = event_tx
                .send(ProviderEvent::Completed(
                    cadence_aria::cross_cutting::streaming_provider::ProviderCompletion::plain(
                        TEXT_FALLBACK_CHOICE_OUTPUT.to_string(),
                        Some("text-fallback-choice-session".to_string()),
                    ),
                ))
                .await;
            cancel.cancelled().await;
        });
        Ok(ProviderSession {
            native_session_id: None,
            events: event_rx,
            commands: command_tx,
        })
    }

    async fn run_streaming(
        &self,
        _input: &cadence_aria::protocol::contracts::AdapterInput,
        _cancel: CancellationToken,
    ) -> Result<
        mpsc::Receiver<cadence_aria::cross_cutting::streaming_provider::StreamChunk>,
        cadence_aria::cross_cutting::provider_adapter::ProviderAdapterError,
    > {
        Err(cadence_aria::cross_cutting::provider_adapter::ProviderAdapterError::execution_failed(
            None,
            String::new(),
            "run_streaming is not used by workspace websocket",
            0,
        ))
    }
}

async fn recv_raw_value(
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

/// 持续读到 type==session_state 的原始 JSON（attach 初帧即 session_state，
/// 这里容错跳过其他帧防止时序抖动）。
async fn recv_until_session_state_value(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) -> Value {
    for _ in 0..40 {
        let value = recv_raw_value(ws).await;
        if value["type"] == "session_state" {
            return value;
        }
    }
    panic!("session_state not received");
}

/// F-27 主锚：run 挂起 provider choice 期间（engine 锁被 run 任务持有），
/// 次连接 attach 的 session_state 初帧经 durable 投影临时 engine 组装——
/// `pending_choice_requests` 必须带全量字段（id/prompt/options/
/// allow_multiple/allow_free_text/questions/source）。
#[tokio::test]
async fn workspace_ws_session_state_projects_pending_provider_choice_during_active_run() {
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
            content: "run choice provider".to_string(),
        },
    )
    .await;
    let _choice = recv_until_choice_request(&mut primary).await;

    // run 仍挂起 choice（等待应答）——次连接 attach 初帧必须含挂起 choice 投影。
    let (mut secondary, _) = connect_async(url).await.expect("connect secondary");
    let state = recv_until_session_state_value(&mut secondary).await;
    let pending = state["pending_choice_requests"]
        .as_array()
        .expect("session_state must project pending_choice_requests during pending choice");
    let projected = pending
        .iter()
        .find(|entry| entry["id"] == json!("choice_completing_001"))
        .expect("pending provider choice must be projected")
        .clone();
    assert_eq!(projected["prompt"], json!("继续方式？"));
    assert_eq!(
        projected["options"],
        json!([{ "id": "opt_0", "label": "继续 author", "description": null }]),
        "options must be isomorphic to the choice_request frame (same ChoiceOption DTO)"
    );
    assert_eq!(projected["allow_multiple"], json!(false));
    assert_eq!(projected["allow_free_text"], json!(false));
    assert_eq!(
        projected["questions"],
        json!([{
            "id": "default",
            "prompt": "继续方式？",
            "options": [{ "id": "opt_0", "label": "继续 author", "description": null }],
            "allow_multiple": false,
            "allow_free_text": false,
        }]),
        "questions must use effective_questions like the live frame"
    );
    assert_eq!(projected["source"], json!("provider_choice"));
    // F-59（缺陷 2）：等待提示条数据源——挂起起始时刻与发问角色随投影带出
    //（已等待时长/901s 超时倒计时的锚点，跨刷新不失真）。
    let created_at_ms = projected["created_at_ms"]
        .as_u64()
        .expect("provider pending choice must carry created_at_ms (F-59)");
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("epoch")
        .as_millis() as u64;
    assert!(
        created_at_ms <= now_ms && now_ms.saturating_sub(created_at_ms) < 60_000,
        "created_at_ms 必须是登记时刻（epoch ms），实际 {created_at_ms} vs now {now_ms}"
    );
    assert_eq!(projected["role"], json!("author"));

    drop(secondary);
    drop(primary);
    server.abort();
}

/// F-27 TextFallback 锚：author 以文本提问完成后 run 结束、会话暂停在
/// AuthorConfirm——此时无活跃 run、engine 锁空闲，attach 初帧走活引擎
/// build_session_state。TextFallback choice（pending_author_choice）必须同样
/// 进 pending_choice_requests 投影（router 侧 F-24 挂起帧不登记 TextFallback，
/// 投影是它唯一的恢复来源）。
#[tokio::test]
async fn workspace_ws_session_state_projects_text_fallback_pending_choice() {
    let root = tempdir().expect("root");
    create_workspace_session_fixture(&root).await;
    let mut registry = ProviderRegistry::new();
    registry.register(
        ProviderName::Fake,
        Arc::new(TextFallbackChoiceCompletingProvider),
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
            content: "run text fallback choice provider".to_string(),
        },
    )
    .await;

    // 等到 TextFallback choice_request 帧到达（source=text_fallback）。
    let fallback_frame = loop {
        let value = recv_raw_value(&mut primary).await;
        if value["type"] == json!("choice_request") && value["source"] == json!("text_fallback") {
            break value;
        }
        if value["type"] == json!("error") {
            panic!("ws error: {}", value["message"]);
        }
    };
    let fallback_choice_id = fallback_frame["id"].as_str().expect("choice id").to_string();

    // run 已结束（paused 等待应答）——次连接 attach 的 session_state 必须含
    // TextFallback pending choice 投影。
    let (mut secondary, _) = connect_async(url).await.expect("connect secondary");
    let state = recv_until_session_state_value(&mut secondary).await;
    let pending = state["pending_choice_requests"]
        .as_array()
        .expect("session_state must project pending_choice_requests for text fallback");
    let projected = pending
        .iter()
        .find(|entry| entry["id"] == json!(fallback_choice_id))
        .expect("text fallback pending choice must be projected")
        .clone();
    assert_eq!(projected["source"], json!("text_fallback"));
    assert_eq!(projected["allow_multiple"], json!(false));
    // F-59：TextFallback 发问角色同为 author；无 provider 等待界，
    // created_at_ms 不得携带（前端回退首见时刻）。
    assert_eq!(projected["role"], json!("author"));
    assert!(
        projected["created_at_ms"].is_null(),
        "text_fallback 无等待界，投影不得携带 created_at_ms"
    );
    assert_eq!(projected["allow_free_text"], json!(true));
    assert_eq!(
        projected["questions"][0]["id"],
        json!("default"),
        "text fallback projection mirrors the live frame's default question"
    );
    assert!(
        projected["options"]
            .as_array()
            .expect("options array")
            .len()
            >= 2,
        "text fallback projection carries the detected options"
    );

    drop(secondary);
    drop(primary);
    server.abort();
}

/// F-27 防泄漏锚：provider choice 被应答、run 完成后（DriveGuard Drop 清空
/// pending 登记），后续 attach 的 session_state 不得再投影该 choice id。
#[tokio::test]
async fn workspace_ws_pending_choice_projection_clears_after_run_completes() {
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
    let (mut ws, _) = connect_async(url.clone()).await.expect("connect ws");
    let _initial = recv_json(&mut ws).await;
    send_json(
        &mut ws,
        &WsInMessage::UserMessage {
            content: "run choice provider then complete".to_string(),
        },
    )
    .await;
    let choice = recv_until_choice_request(&mut ws).await;
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
    let _checkpoint = recv_until_message_complete(&mut ws).await;

    // run 完成（DriveGuard 已 Drop）——新连接的 session_state 不得残留该 choice。
    let (mut after, _) = connect_async(url).await.expect("connect after");
    let state = recv_until_session_state_value(&mut after).await;
    let pending = state["pending_choice_requests"].as_array();
    assert!(
        pending.is_none_or(|entries| {
            !entries
                .iter()
                .any(|entry| entry["id"] == json!("choice_completing_001"))
        }),
        "answered/completed choice must not stay projected after the run ends"
    );

    drop(after);
    drop(ws);
    server.abort();
}

// ---------------------------------------------------------------------------
// F-27R2（v34 复验收敛轮）：choice pending 变更即广播 session_state
// ---------------------------------------------------------------------------

/// F-27R2 专属挂起 provider：发出唯一 id 的 provider choice 后挂起等应答，
/// 应答后仅回 StatusChanged 继续挂起（run 持锁存活到测试结束）。唯一 id
/// 隔离进程级挂起登记簿——it_core 多测试共用同一 session_id，广播帧的
/// 投影读同一登记簿，共享 id 会让「不含已答 id」断言被并行测试污染。
struct ChoiceHangingBroadcastProvider {
    choice_id: &'static str,
}

const CHOICE_BROADCAST_PENDING_ID: &str = "choice_broadcast_r2_001";
const CHOICE_ANSWER_BROADCAST_PENDING_ID: &str = "choice_broadcast_r2_002";

#[async_trait::async_trait]
impl StreamingProviderAdapter for ChoiceHangingBroadcastProvider {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let (event_tx, event_rx) = mpsc::channel(8);
        let (command_tx, mut command_rx) = mpsc::channel::<ProviderCommand>(8);
        let pending_id = self.choice_id.to_string();
        tokio::spawn(async move {
            let _ = event_tx
                .send(ProviderEvent::ChoiceRequest(ChoiceRequestData {
                    id: pending_id,
                    prompt: "继续方式？".to_string(),
                    options: vec![ChoiceOptionData {
                        id: "opt_0".to_string(),
                        label: "继续 author".to_string(),
                        description: None,
                    }],
                    allow_multiple: false,
                    allow_free_text: false,
                    questions: vec![],
                    source: ChoiceRequestSource::ProviderChoice,
                }))
                .await;
            loop {
                tokio::select! {
                    _ = cancel.cancelled() => return,
                    command = command_rx.recv() => {
                        match command {
                            Some(ProviderCommand::Abort) | None => return,
                            Some(ProviderCommand::ChoiceResponse { .. }) => {
                                let _ = event_tx
                                    .send(ProviderEvent::StatusChanged(
                                        ProviderStatus::Running,
                                    ))
                                    .await;
                            }
                            Some(ProviderCommand::PermissionResponse { .. })
                            | Some(ProviderCommand::ToolResult(_)) => {}
                        }
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

/// 读到 type 匹配的原始 JSON 帧（容错跳过其他帧；error 帧直接失败）。
async fn recv_until_type(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    frame_type: &str,
) -> Value {
    for _ in 0..40 {
        let value = recv_raw_value(ws).await;
        if value["type"] == json!(frame_type) {
            return value;
        }
        if value["type"] == json!("error") {
            panic!("ws error: {}", value["message"]);
        }
    }
    panic!("{frame_type} frame not received");
}

/// F-27R2 主锚（结构化源）：provider choice 登记挂起后，已连接 attachment
/// 必须收到一帧带 pending_choice_requests 的 session_state——这是
/// choice_request 帧降级丢失后前端对账（reconcilePendingChoiceRequests）
/// 唯一的触发面。帧序锚：choice_request 帧先入 journal，session_state 的
/// event_seq 必须严格更大。挂起 provider 令 run 持锁存活——该帧不可能
/// 来自 run 结束/门事件，只能来自 pending 变更触发面。
#[tokio::test]
async fn workspace_ws_choice_pending_change_broadcasts_session_state_with_pending() {
    let root = tempdir().expect("root");
    create_workspace_session_fixture(&root).await;
    let mut registry = ProviderRegistry::new();
    registry.register(
        ProviderName::Fake,
        Arc::new(ChoiceHangingBroadcastProvider {
            choice_id: CHOICE_BROADCAST_PENDING_ID,
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
    let (mut primary, _) = connect_async(url).await.expect("connect primary");
    let _initial = recv_json(&mut primary).await;
    send_json(
        &mut primary,
        &WsInMessage::UserMessage {
            content: "run broadcast choice provider".to_string(),
        },
    )
    .await;

    let choice_frame = recv_until_type(&mut primary, "choice_request").await;
    assert_eq!(choice_frame["id"], json!(CHOICE_BROADCAST_PENDING_ID));
    let choice_seq = choice_frame["event_seq"]
        .as_u64()
        .expect("choice_request event_seq");

    let state = recv_until_type(&mut primary, "session_state").await;
    let state_seq = state["event_seq"]
        .as_u64()
        .expect("session_state event_seq");
    assert!(
        state_seq > choice_seq,
        "session_state 必须后于 choice_request 帧入 journal（choice={choice_seq}, state={state_seq}）"
    );
    let pending = state["pending_choice_requests"]
        .as_array()
        .expect("pending 变更必须广播带 pending_choice_requests 的 session_state");
    assert!(
        pending
            .iter()
            .any(|entry| entry["id"] == json!(CHOICE_BROADCAST_PENDING_ID)),
        "广播 session_state 必须携带该挂起 choice：{state}"
    );

    drop(primary);
    server.abort();
}

/// F-27R2 TextFallback 锚：文本回退 choice（pending 只活引擎内存，durable
/// 投影临时引擎上恒空）登记后，已连接 attachment 同样必须收到带该 pending
/// 的 session_state 帧（run 尾锁释放后由活引擎投影广播）。帧序锚同上。
#[tokio::test]
async fn workspace_ws_text_fallback_choice_pending_change_broadcasts_session_state() {
    let root = tempdir().expect("root");
    create_workspace_session_fixture(&root).await;
    let mut registry = ProviderRegistry::new();
    registry.register(
        ProviderName::Fake,
        Arc::new(TextFallbackChoiceCompletingProvider),
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
    let (mut primary, _) = connect_async(url).await.expect("connect primary");
    let _initial = recv_json(&mut primary).await;
    send_json(
        &mut primary,
        &WsInMessage::UserMessage {
            content: "run text fallback broadcast provider".to_string(),
        },
    )
    .await;

    let fallback_frame = loop {
        let value = recv_raw_value(&mut primary).await;
        if value["type"] == json!("choice_request") && value["source"] == json!("text_fallback")
        {
            break value;
        }
        if value["type"] == json!("error") {
            panic!("ws error: {}", value["message"]);
        }
    };
    let fallback_id = fallback_frame["id"].as_str().expect("choice id").to_string();
    let choice_seq = fallback_frame["event_seq"]
        .as_u64()
        .expect("choice_request event_seq");

    let state = recv_until_type(&mut primary, "session_state").await;
    let state_seq = state["event_seq"]
        .as_u64()
        .expect("session_state event_seq");
    assert!(
        state_seq > choice_seq,
        "session_state 必须后于 choice_request 帧入 journal（choice={choice_seq}, state={state_seq}）"
    );
    let pending = state["pending_choice_requests"]
        .as_array()
        .expect("TextFallback pending 变更必须广播带 pending_choice_requests 的 session_state");
    assert!(
        pending.iter().any(|entry| entry["id"] == json!(fallback_id)),
        "广播 session_state 必须携带 TextFallback 挂起 choice：{state}"
    );

    drop(primary);
    server.abort();
}

/// F-27R2 应答移除锚：ChoiceResponse 命中挂起 id 后，必须再广播一帧不含
/// 已答 id 的 session_state（挂起 provider 令 run 持锁存活中——帧只能来自
/// 移除触发面）。
#[tokio::test]
async fn workspace_ws_choice_answer_broadcasts_session_state_without_answered_pending() {
    let root = tempdir().expect("root");
    create_workspace_session_fixture(&root).await;
    let mut registry = ProviderRegistry::new();
    registry.register(
        ProviderName::Fake,
        Arc::new(ChoiceHangingBroadcastProvider {
            choice_id: CHOICE_ANSWER_BROADCAST_PENDING_ID,
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
    let (mut primary, _) = connect_async(url).await.expect("connect primary");
    let _initial = recv_json(&mut primary).await;
    send_json(
        &mut primary,
        &WsInMessage::UserMessage {
            content: "run answer broadcast choice provider".to_string(),
        },
    )
    .await;

    let _choice = recv_until_type(&mut primary, "choice_request").await;
    let registered = recv_until_type(&mut primary, "session_state").await;
    let registered_seq = registered["event_seq"]
        .as_u64()
        .expect("registered session_state event_seq");

    send_json(
        &mut primary,
        &WsInMessage::ChoiceResponse {
            id: CHOICE_ANSWER_BROADCAST_PENDING_ID.to_string(),
            selected_option_ids: vec!["opt_0".to_string()],
            free_text: None,
            answers: vec![],
        },
    )
    .await;

    let answered = recv_until_type(&mut primary, "session_state").await;
    let answered_seq = answered["event_seq"]
        .as_u64()
        .expect("answered session_state event_seq");
    assert!(
        answered_seq > registered_seq,
        "应答后的 session_state 必须是新的广播帧（registered={registered_seq}, answered={answered_seq}）"
    );
    assert!(
        answered["pending_choice_requests"]
            .as_array()
            .is_none_or(|entries| {
                !entries
                    .iter()
                    .any(|entry| entry["id"] == json!(CHOICE_ANSWER_BROADCAST_PENDING_ID))
            }),
        "已答 choice 必须移出广播 session_state 的挂起集：{answered}"
    );

    drop(primary);
    server.abort();
}

// ---------------------------------------------------------------------------
// F-27R3（f27r2-review-k3 P1 回归修复）：三驱动统一 PendingChoiceRequests
// 登记簿——plan 子驱动与 review 驱动投影锚
// ---------------------------------------------------------------------------

const CHOICE_PLAN_SUBDRIVE_PENDING_ID: &str = "choice_plan_subdrive_r3_001";
const CHOICE_REVIEW_DRIVE_PENDING_ID: &str = "choice_review_drive_r3_001";

/// F-27R3 plan 子驱动锚（k3 方案 a）：WorkItemPlan（Legacy 流）会话
/// StartGeneration 走 provider_drive/work_item_plan.rs 子驱动，其挂起
/// provider choice 与主驱动同源——choice_request 帧后确定性追加的
/// session_state 广播必须携带该 choice（pending_choice_requests 投影经
/// 进程级登记簿）。修复前该驱动不经登记簿，投影不含此卡：前端对账
/// （rebuildChatEntries）挂载即抹除且永不补回，最终 900s choice_wait_timeout。
#[tokio::test]
async fn workspace_ws_plan_subdrive_choice_pending_projects_into_session_state() {
    let root = tempdir().expect("root");
    create_workspace_session_fixture(&root).await;
    let lifecycle = LifecycleStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let plan = lifecycle
        .create_issue_work_item_plan(
            cadence_aria::product::lifecycle_store::CreateIssueWorkItemPlanInput {
                id: Some("issue_work_item_plan_r3_legacy".to_string()),
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                source_story_spec_ids: vec![],
                source_design_spec_ids: vec![],
                options: cadence_aria::product::models::IssueWorkItemPlanOptions {
                    include_integration_tests: false,
                    include_e2e_tests: false,
                    force_frontend_backend_split: false,
                    require_execution_plan_confirm: false,
                },
                status: cadence_aria::product::models::IssueWorkItemPlanStatus::Draft,
                work_item_ids: vec![],
                repository_profile_ref: None,
                verification_plan_ids: vec![],
                dependency_graph: vec![],
                created_from_provider_run: None,
                validator_findings: vec![],
            },
        )
        .expect("create plan");
    let session = lifecycle
        .create_workspace_session(
            cadence_aria::product::lifecycle_store::CreateWorkspaceSessionInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                entity_id: plan.id,
                workspace_type: cadence_aria::product::models::WorkspaceType::WorkItemPlan,
                author_provider: ProviderName::Fake,
                reviewer_provider: ProviderName::Fake,
                review_rounds: 0,
                superpowers_enabled: false,
                openspec_enabled: false,
                work_item_plan_options: Some(
                    cadence_aria::product::lifecycle_store::WorkItemPlanSessionOptions {
                        flow_kind:
                            cadence_aria::product::work_item_plan_policy::WorkItemPlanFlowKind::Legacy,
                        run_policy:
                            cadence_aria::product::work_item_plan_policy::RunPolicy::Interactive,
                        rollout_snapshot: true,
                    },
                ),
            },
        )
        .expect("create legacy plan session");
    let mut registry = ProviderRegistry::new();
    registry.register(
        ProviderName::Fake,
        Arc::new(ChoiceHangingBroadcastProvider {
            choice_id: CHOICE_PLAN_SUBDRIVE_PENDING_ID,
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

    let url = format!("ws://{addr}/api/workspace-sessions/{}/ws", session.id);
    let (mut ws, _) = connect_async(url.clone()).await.expect("connect ws");
    let _initial = recv_json(&mut ws).await;
    send_json(
        &mut ws,
        &WsInMessage::StartGeneration {
            provider_config: ProviderConfigSnapshot {
                author: ProviderName::Fake,
                reviewer: None,
                review_rounds: 0,
                permission_modes:
                    cadence_aria::product::models::WorkspaceRolePermissionModes::default(),
            },
            reviewer_enabled: false,
        },
    )
    .await;

    let choice_frame = recv_until_type(&mut ws, "choice_request").await;
    assert_eq!(choice_frame["id"], json!(CHOICE_PLAN_SUBDRIVE_PENDING_ID));
    let state = recv_until_type(&mut ws, "session_state").await;
    let pending = state["pending_choice_requests"].as_array().expect(
        "plan 子驱动挂起 choice 后必须广播带 pending_choice_requests 的 session_state",
    );
    assert!(
        pending
            .iter()
            .any(|entry| entry["id"] == json!(CHOICE_PLAN_SUBDRIVE_PENDING_ID)),
        "广播 session_state 必须携带 plan 子驱动挂起的 choice（三驱动统一登记簿）：{state}"
    );

    drop(ws);
    server.abort();
}

/// F-27R3 review 驱动锚（k3 方案 a）：断线中止的 review 经
/// RetryInterruptedRun 重启后由 review/drive.rs 驱动 reviewer 会话，其挂起
/// provider choice 同样必须进 session_state 投影（此前仅本地 HashSet，
/// 投影恒不含——同 plan 子驱动回归面）。
#[tokio::test]
async fn workspace_ws_review_drive_choice_pending_projects_into_session_state() {
    let root = tempdir().expect("root");
    create_workspace_session_fixture(&root).await;
    // 会话手术：author 产物已落盘（artifact 版本指向 author 节点）、reviewer
    // 节点断线中止（Failed + 紧随 AbortedByDisconnect）——构成
    // recoverable_shared_review 可恢复现场，RetryInterruptedRun 重启 review。
    let lifecycle = LifecycleStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let now = chrono::Utc::now().to_rfc3339();
    let stage_cross_review = cadence_aria::web::workspace_ws_types::WorkspaceStage::CrossReview;
    let stage_running = cadence_aria::web::workspace_ws_types::WorkspaceStage::Running;
    let stage_prepare_context =
        cadence_aria::web::workspace_ws_types::WorkspaceStage::PrepareContext;
    let snapshot = ProviderConfigSnapshot {
        author: ProviderName::Fake,
        reviewer: None,
        review_rounds: 0,
        permission_modes: cadence_aria::product::models::WorkspaceRolePermissionModes::default(),
    };
    lifecycle
        .save_timeline_nodes(
            "workspace_session_0001",
            &[
                cadence_aria::web::workspace_ws_types::TimelineNode {
                    node_id: "author_run_review_r3".to_string(),
                    node_type: TimelineNodeType::AuthorRun,
                    agent: Some(ProviderName::Fake),
                    stage: stage_running,
                    round: None,
                    status: TimelineNodeStatus::Completed,
                    title: "Author Run".to_string(),
                    summary: Some("生成完成".to_string()),
                    started_at: now.clone(),
                    completed_at: Some(now.clone()),
                    duration_ms: None,
                    artifact_ref: None,
                    provider_config_snapshot: snapshot.clone(),
                    retry: None,
                },
                cadence_aria::web::workspace_ws_types::TimelineNode {
                    node_id: "review_run_r3".to_string(),
                    node_type: TimelineNodeType::ReviewerRun,
                    agent: Some(ProviderName::Fake),
                    stage: stage_cross_review,
                    round: Some(1),
                    status: TimelineNodeStatus::Failed,
                    title: "Review Round 1".to_string(),
                    summary: Some("连接断开，运行已中止".to_string()),
                    started_at: now.clone(),
                    completed_at: Some(now.clone()),
                    duration_ms: None,
                    artifact_ref: None,
                    provider_config_snapshot: snapshot.clone(),
                    retry: None,
                },
                cadence_aria::web::workspace_ws_types::TimelineNode {
                    node_id: "abort_review_r3".to_string(),
                    node_type: TimelineNodeType::AbortedByDisconnect,
                    agent: None,
                    stage: stage_prepare_context,
                    round: None,
                    status: TimelineNodeStatus::Completed,
                    title: "连接断开".to_string(),
                    summary: None,
                    started_at: now.clone(),
                    completed_at: Some(now.clone()),
                    duration_ms: None,
                    artifact_ref: None,
                    provider_config_snapshot: snapshot,
                    retry: None,
                },
            ],
        )
        .expect("save review-retry timeline");
    lifecycle
        .append_artifact_version(
            "workspace_session_0001",
            ArtifactVersion {
                version: 1,
                payload: ArtifactPayload::Markdown {
                    markdown: VALID_STORY_SPEC.to_string(),
                    diff: None,
                },
                generated_by: ProviderName::Fake,
                reviewed_by: None,
                review_verdict: None,
                confirmed_by: None,
                is_current: true,
                created_at: now,
                source_node_id: "author_run_review_r3".to_string(),
            },
        )
        .expect("append artifact version");

    let mut registry = ProviderRegistry::new();
    registry.register(
        ProviderName::Fake,
        Arc::new(ChoiceHangingBroadcastProvider {
            choice_id: CHOICE_REVIEW_DRIVE_PENDING_ID,
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
    let (mut ws, _) = connect_async(url.clone()).await.expect("connect ws");
    let _initial = recv_json(&mut ws).await;
    send_json(
        &mut ws,
        &WsInMessage::RetryInterruptedRun {
            failed_node_id: "review_run_r3".to_string(),
        },
    )
    .await;

    let choice_frame = recv_until_type(&mut ws, "choice_request").await;
    assert_eq!(choice_frame["id"], json!(CHOICE_REVIEW_DRIVE_PENDING_ID));
    let state = recv_until_type(&mut ws, "session_state").await;
    let pending = state["pending_choice_requests"].as_array().expect(
        "review 驱动挂起 choice 后必须广播带 pending_choice_requests 的 session_state",
    );
    assert!(
        pending
            .iter()
            .any(|entry| entry["id"] == json!(CHOICE_REVIEW_DRIVE_PENDING_ID)),
        "广播 session_state 必须携带 review 驱动挂起的 choice（三驱动统一登记簿）：{state}"
    );

    drop(ws);
    server.abort();
}
