/// F-43 后端红面：coding attempt 页刷新/新开连接必须能拿到并答掉挂起 provider
/// choice，否则 coder 等 UI、UI 无入口 —— 刷新即永久死锁。
///
/// 两条根因、两层钉法：
/// 1. 「看不到卡」：attach 初帧只有会话快照投影（`pending_choices`），没有可作答
///    的 `coding_choice_request` 帧 —— 新连接拿不到选择入口。
/// 2. 「点了没反应」：新连接不持有 runner 句柄（句柄只留给启动 runner 的那条连接），
///    `ChoiceResponse` 被拒（permission 更是静默丢弃）—— 帧补上了仍是死锁。
const PENDING_CHOICE_ID: &str = "provider_choice_pending_0001";
const PENDING_CHOICE_PROMPT: &str = "⚠️ Dangerous command:\n\n  rm -rf /tmp/aria-fixture\n\nAllow?";

type CodingWsStream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// 与 it_product `ChoiceAwaitingProvider` 同形的 provider：先抛 choice 挂在
/// 引擎里，收到 `ProviderCommand::ChoiceResponse` 才完成本轮 coding。
struct F43ChoiceAwaitingProvider;

#[async_trait::async_trait]
impl cadence_aria::cross_cutting::streaming_provider::StreamingProviderAdapter
    for F43ChoiceAwaitingProvider
{
    async fn start(
        &self,
        _input: cadence_aria::cross_cutting::streaming_provider::StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<
        cadence_aria::cross_cutting::streaming_provider::ProviderSession,
        cadence_aria::cross_cutting::provider_adapter::ProviderAdapterError,
    > {
        use cadence_aria::cross_cutting::streaming_provider::{
            ChoiceOptionData, ChoiceRequestData, ChoiceRequestSource, ProviderCommand,
            ProviderCompletion, ProviderEvent, ProviderSession,
        };
        let (event_tx, event_rx) = mpsc::channel(16);
        let (command_tx, mut command_rx) = mpsc::channel(8);
        tokio::spawn(async move {
            let _ = event_tx
                .send(ProviderEvent::ChoiceRequest(ChoiceRequestData {
                    id: PENDING_CHOICE_ID.to_string(),
                    prompt: PENDING_CHOICE_PROMPT.to_string(),
                    options: vec![
                        ChoiceOptionData {
                            id: "Yes".to_string(),
                            label: "Yes".to_string(),
                            description: None,
                        },
                        ChoiceOptionData {
                            id: "No".to_string(),
                            label: "No".to_string(),
                            description: None,
                        },
                    ],
                    allow_multiple: false,
                    allow_free_text: true,
                    questions: Vec::new(),
                    source: ChoiceRequestSource::ProviderChoice,
                }))
                .await;
            while let Some(command) = command_rx.recv().await {
                match command {
                    ProviderCommand::ChoiceResponse {
                        id,
                        selected_option_ids,
                        ..
                    } if id == PENDING_CHOICE_ID
                        && selected_option_ids == vec!["Yes".to_string()] =>
                    {
                        let _ = event_tx
                            .send(ProviderEvent::Completed(ProviderCompletion::plain(
                                "approved pending choice".to_string(),
                                None,
                            )))
                            .await;
                        return;
                    }
                    ProviderCommand::Abort => return,
                    _ => {}
                }
            }
        });
        Ok(ProviderSession {
            native_session_id: None,
            events: event_rx,
            commands: command_tx,
        })
    }
}

fn app_and_state_with_pending_choice(
    root_path: &std::path::Path,
) -> (axum::Router, WebAppState, CodingExecutionAttempt) {
    let app_paths = ProductAppPaths::new(root_path.join(".aria"));
    let repo = root_path.join("repo");
    init_simple_git_repo(&repo);
    let worktree = root_path.join("worktree");
    std::fs::create_dir_all(&worktree).expect("create worktree fixture");
    let repository = RepositoryStore::new(app_paths.clone())
        .create(CreateRepositoryInput {
            project_id: "project_0001".to_string(),
            name: "repo".to_string(),
            path: repo,
            default_policy_preset: Some("manual-write".to_string()),
            default_provider_mode: Some("fake".to_string()),
            idempotency_key: "coding-ws-part18-pending-choice-repository".to_string(),
        })
        .expect("create repository");
    LifecycleStore::new(app_paths.clone())
        .create_work_item(CreateWorkItemInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: repository.id,
            story_spec_ids: Vec::new(),
            design_spec_ids: Vec::new(),
            title: "Coding work item".to_string(),
            ..Default::default()
        })
        .expect("create work item");
    let store = CodingAttemptStore::new(app_paths);
    create_legacy_coding_attempt_fixture(
        &store,
        CreateCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            work_item_id: "work_item_0001".to_string(),
            base_branch: "main".to_string(),
            branch_name: "aria/work-items/work_item_0001/attempt-1".to_string(),
            worktree_path: Some(worktree),
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Fake,
                reviewer: Some(ProviderName::Fake),
                review_rounds: 1,
                permission_modes:
                    cadence_aria::product::models::WorkspaceRolePermissionModes::default(),
            },
            target_snapshot: None,
            max_auto_rework: 2,
        },
    );
    let attempt = crate::seed_coding_attempt_running(
        &store,
        "project_0001",
        "issue_0001",
        "coding_attempt_0001",
    );
    let state = WebAppState::new(
        root_path.to_path_buf(),
        WebRuntime::new_fake(root_path.to_path_buf()),
    );
    (build_web_router(state.clone()), state, attempt)
}

/// 引擎已挂起 choice 的落盘态（`WaitingForHuman` + choice-gates 下一条 Open gate）；
/// 只用于「已答不重发」用例，构造成本远低于真跑一轮 provider。
fn seed_pending_choice_gate(
    store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
) -> CodingExecutionAttempt {
    let mut waiting = attempt.clone();
    waiting.status = CodingAttemptStatus::WaitingForHuman;
    waiting.stage = CodingExecutionStage::Coding;
    crate::seed_coding_attempt_record(store, &waiting);
    store
        .create_choice_gate(
            &waiting,
            CreateChoiceGateInput {
                attempt_id: waiting.id.clone(),
                choice_id: PENDING_CHOICE_ID.to_string(),
                stage: CodingExecutionStage::Coding,
                node_id: Some("coding_node_0006".to_string()),
                role: CodingProviderRole::Coder,
                provider: ProviderName::Fake,
                source: "provider_choice".to_string(),
                prompt: PENDING_CHOICE_PROMPT.to_string(),
                options: vec![
                    CodingChoiceOption {
                        id: "Yes".to_string(),
                        label: "Yes".to_string(),
                        description: None,
                    },
                    CodingChoiceOption {
                        id: "No".to_string(),
                        label: "No".to_string(),
                        description: None,
                    },
                ],
                allow_multiple: false,
                allow_free_text: true,
            },
        )
        .expect("create pending choice gate");
    waiting
}

async fn recv_coding_choice_request(ws: &mut CodingWsStream) -> serde_json::Value {
    for _ in 0..20 {
        let value = recv_json_value(ws).await;
        assert_ne!(
            value["type"], "coding_protocol_error",
            "unexpected protocol error while waiting for pending choice frame: {value}"
        );
        if value["type"] == "coding_choice_request" {
            return value;
        }
    }
    panic!("new coding ws connection never received coding_choice_request");
}

/// F-43 红面贯穿链：真引擎挂起 choice（durable gate）→ 新 WS 连接 attach 拿到
/// 选择帧 → 作答经注册表投递回**仍在等待的真引擎**并被执行（gate 关闭、ack 发出）。
#[tokio::test]
async fn coding_ws_new_connection_receives_and_answers_pending_choice() {
    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let (app, state, attempt) = app_and_state_with_pending_choice(root.path());
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt_key = CodingAttemptRunKey::from_attempt(&attempt);

    // runner 侧：注册表持有的 command_tx 就是引擎 command_rx 的对端（生产由
    // `spawn_coding_runner` 建立同一对通道；此处测试直接驱动引擎）。
    let (command_tx, mut command_rx) = mpsc::channel(8);
    state
        .coding_runs
        .insert_cancellable(&attempt_key, command_tx)
        .expect("register coding runner");

    let (engine_event_tx, mut engine_event_rx) = mpsc::channel(32);
    let engine = cadence_aria::product::coding_workspace_engine::CodingWorkspaceEngine::new(
        store.clone(),
        cadence_aria::product::git_workspace_service::GitWorkspaceService::new(),
        engine_event_tx,
    );
    let provider = F43ChoiceAwaitingProvider;
    let context = cadence_aria::product::coding_workspace_engine::CodingExecutionContext::default();
    let execute =
        engine.execute_coding_with_commands(&attempt, &provider, &context, &mut command_rx);
    tokio::pin!(execute);

    // 1) 引擎跑到挂起 choice：durable gate 已落盘（refresh 前的真实状态）。
    timeout(Duration::from_secs(20), async {
        loop {
            tokio::select! {
                result = &mut execute => panic!("coding run ended before choice was answered: {result:?}"),
                event = engine_event_rx.recv() => {
                    if let Some(CodingWsOutMessage::CodingChoiceRequest { id, .. }) = event
                        && id == PENDING_CHOICE_ID
                    {
                        break;
                    }
                }
            }
        }
    })
    .await
    .expect("engine must reach the pending choice");

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    // 2) 页面刷新：新连接（不持有 runner 句柄）attach。
    let url = format!(
        "ws://{addr}/ws/projects/project_0001/issues/issue_0001/coding-attempts/coding_attempt_0001"
    );
    let (mut ws, _) = connect_async(url).await.expect("connect ws");

    let snapshot = recv_json_value(&mut ws).await;
    assert_eq!(snapshot["type"], "coding_session_state");
    assert_eq!(
        snapshot["pending_choices"][0]["choice_id"],
        PENDING_CHOICE_ID
    );

    // 3) attach 补帧：新连接必须拿到可直接作答的 choice 帧。
    let frame = recv_coding_choice_request(&mut ws).await;
    assert_eq!(frame["id"], PENDING_CHOICE_ID);
    assert_eq!(frame["source"], "provider_choice");
    assert_eq!(frame["prompt"], PENDING_CHOICE_PROMPT);
    assert_eq!(frame["options"][0]["id"], "Yes");
    assert_eq!(frame["options"][1]["id"], "No");
    assert_eq!(frame["allow_multiple"], false);
    assert_eq!(frame["allow_free_text"], true);

    // 4) 新连接作答 → 必须送达仍在等待的真引擎并被消费（gate 关闭 + ack 发出 +
    //    provider 收到 ChoiceResponse 后完成本轮）。
    send_json(
        &mut ws,
        &CodingWsInMessage::ChoiceResponse {
            id: PENDING_CHOICE_ID.to_string(),
            selected_option_ids: vec!["Yes".to_string()],
            free_text: None,
        },
    )
    .await;

    let mut saw_ack = false;
    let outcome = timeout(Duration::from_secs(20), async {
        loop {
            tokio::select! {
                result = &mut execute => break result,
                event = engine_event_rx.recv() => {
                    if let Some(CodingWsOutMessage::CodingChoiceResponseAck { id, .. }) = event
                        && id == PENDING_CHOICE_ID
                    {
                        saw_ack = true;
                    }
                }
            }
        }
    })
    .await
    .expect("engine must consume the pending choice response routed from the new connection");
    assert!(
        saw_ack,
        "engine must emit choice response ack for the resumed answer"
    );
    assert!(
        outcome.is_ok(),
        "engine must consume the choice: {outcome:?}"
    );
    assert!(
        store
            .list_open_choice_gates("project_0001", "issue_0001", "coding_attempt_0001")
            .expect("open choice gates")
            .is_empty(),
        "answered choice gate must be resolved by the engine"
    );

    ws.close(None).await.expect("close ws");
    server.abort();
}

#[tokio::test]
async fn coding_ws_new_connection_does_not_resend_answered_choice() {
    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let (app, state, attempt) = app_and_state_with_pending_choice(root.path());
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let waiting = seed_pending_choice_gate(&store, &attempt);
    store
        .resolve_choice_gate(
            "project_0001",
            "issue_0001",
            &waiting.id,
            PENDING_CHOICE_ID,
            vec!["Yes".to_string()],
            None,
        )
        .expect("resolve pending choice gate");

    let attempt_key = CodingAttemptRunKey::from_attempt(&waiting);
    let (runner_tx, mut runner_rx) = mpsc::channel(8);
    state
        .coding_runs
        .insert_cancellable(&attempt_key, runner_tx)
        .expect("register runner");

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    let url = format!(
        "ws://{addr}/ws/projects/project_0001/issues/issue_0001/coding-attempts/coding_attempt_0001"
    );
    let (mut ws, _) = connect_async(url).await.expect("connect ws");

    let snapshot = recv_json_value(&mut ws).await;
    assert_eq!(snapshot["type"], "coding_session_state");
    assert_eq!(
        snapshot["pending_choices"].as_array().map(Vec::len),
        Some(0),
        "answered choice must not stay in the session projection"
    );

    // 已答/过期不重发：静默窗口内不得再冒出同一张选择卡。
    loop {
        match timeout(Duration::from_millis(300), ws.next()).await {
            Err(_) => break,
            Ok(Some(Ok(Message::Text(text)))) => {
                let value: serde_json::Value = serde_json::from_str(&text).expect("ws text json");
                assert_ne!(
                    value["type"], "coding_choice_request",
                    "answered choice must not be resent to a new connection: {value}"
                );
            }
            Ok(Some(Ok(_))) => {}
            Ok(other) => panic!("unexpected ws stream state: {other:?}"),
        }
    }
    assert!(
        runner_rx.try_recv().is_err(),
        "new connection must not deliver commands for an answered choice"
    );

    ws.close(None).await.expect("close ws");
    server.abort();
}
