/// SC 人工门反馈链的 durable assistant 候选（change `artifact-candidate-selection` Task 6）。
///
/// 原夹具用 stub `"# Work Item Plan\n"`——无 fence 且缺全部必需 heading，过不了
/// WorkItemPlan artifact gate；迁移到候选选择器后 reload 产物为空，
/// `build_sc_manual_revision_prompt_for_turn` 以 `HUMAN_GATE_REVISION_CANDIDATE_MISSING`
/// 拒绝反馈，本用例便在「WS 阶段门后 feedback 到达 dispatch」的断言处失败。生产 SC 会话的
/// durable assistant message 是完整候选 markdown（会话门修订基线的既有语义），故夹具升级为
/// gate 合规形态；测试意图与断言不变。
const SC_HUMAN_GATE_CANDIDATE: &str = "# Work Item Plan\n\n\
    ## 计划范围\n\
    本计划覆盖 Issue issue_0001 的单个可执行候选。\n\n\
    ## 任务拆分\n\
    - [TASK-001] 完成后端接入。\n\n\
    ## 依赖图\n\
    无。\n\n\
    ## 验证计划\n\
    cargo test --locked。\n\n\
    ## 执行顺序\n\
    先完成任务拆分中的唯一任务。\n\n\
    ## 风险\n\
    无。\n\n\
    ## 追踪关系\n\
    source ids: Story Spec story_spec_0001, Design Spec design_spec_0001。\n\
    [TASK-001] -> [REQ-001]\n";

#[tokio::test]
async fn workspace_ws_sc_human_gate_feedback_reaches_dispatch_after_socket_stage_gate() {
    let root = tempdir().expect("root");
    let _repo = create_workspace_session_fixture(&root).await;
    let lifecycle = LifecycleStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let plan = lifecycle
        .create_issue_work_item_plan(
            cadence_aria::product::lifecycle_store::CreateIssueWorkItemPlanInput {
                id: Some("issue_work_item_plan_sc_socket".to_string()),
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
                            cadence_aria::product::work_item_plan_policy::WorkItemPlanFlowKind::SingleCandidate,
                        run_policy: cadence_aria::product::work_item_plan_policy::RunPolicy::Interactive,
                        rollout_snapshot: true,
                    },
                ),
            },
        )
        .expect("create SC session");
    let mut session_record = lifecycle
        .get_workspace_session(&session.id)
        .expect("load SC session");
    session_record.status = cadence_aria::product::models::WorkspaceSessionStatus::WaitingForHuman;
    session_record.single_candidate_phase = Some(
        cadence_aria::product::models::SingleCandidatePhase::Approval,
    );
    session_record.human_gate_snapshot = Some(
        cadence_aria::product::work_item_plan_policy::HumanGateSnapshot {
            findings: vec![],
            repeated_fingerprints: vec![],
            attempts_used: 0,
            manual_repairs_remaining: 1,
            trigger: cadence_aria::product::work_item_plan_policy::HumanReason::NativeHumanRequired,
            resumable: false,
        },
    );
    session_record.messages.push(cadence_aria::product::models::WorkspaceMessageRecord {
        role: "assistant".to_string(),
        content: SC_HUMAN_GATE_CANDIDATE.to_string(),
        created_at: chrono::Utc::now().to_rfc3339(),
    });
    let session_path = lifecycle
        .app_paths()
        .issue_root(&session_record.project_id, &session_record.issue_id)
        .join("workspace-sessions")
        .join(format!("{}.json", session_record.id));
    cadence_aria::product::json_store::write_json(&session_path, &session_record)
        .expect("persist SC approval state");
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    let url = format!("ws://{addr}/api/workspace-sessions/{}/ws", session.id);
    let (mut ws, _) = connect_async(url).await.expect("connect ws");
    assert!(matches!(recv_json(&mut ws).await, WsOutMessage::SessionState { .. }));
    send_json(
        &mut ws,
        &WsInMessage::HumanGateFeedback {
            command_id: "cmd-socket-gate".to_string(),
            feedback: "请保留完整候选，仅修正此处".to_string(),
        },
    )
    .await;

    let response = recv_json(&mut ws).await;
    assert!(matches!(
        response,
        WsOutMessage::HumanGateTurnOpen {
            ref command_id,
            ..
        } if command_id == "cmd-socket-gate"
    ));

    let _ = ws.close(None).await;
    server.abort();
}

#[tokio::test]
async fn workspace_ws_abort_with_pi_reaches_cancelled_state_and_stops_output() {
    use std::sync::atomic::{AtomicBool, Ordering};

    let root = tempdir().expect("root");
    create_workspace_session_fixture_with_author(&root, "pi").await;
    let started = Arc::new(AtomicBool::new(false));
    let abort_observed = Arc::new(Notify::new());
    let seen_inputs = Arc::new(Mutex::new(Vec::new()));
    let mut registry = ProviderRegistry::new();
    registry.register(
        ProviderName::Pi,
        Arc::new(PiHangingStreamingProvider {
            started: started.clone(),
            abort_observed: abort_observed.clone(),
            seen_inputs: seen_inputs.clone(),
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
        &WsInMessage::UserMessage {
            content: "run Pi then cancel".to_string(),
        },
    )
    .await;
    assert_eq!(recv_until_stream_chunk(&mut ws).await, "Pi partial output");
    assert!(started.load(Ordering::SeqCst), "Pi author adapter must start");
    assert_eq!(
        seen_inputs.lock().expect("Pi input recorder").as_slice(),
        &[(
            cadence_aria::protocol::contracts::ProviderType::Pi,
            cadence_aria::cross_cutting::streaming_provider::ProviderPermissionMode::Auto,
        )],
        "workspace must invoke Pi in Auto mode"
    );

    send_json(&mut ws, &WsInMessage::Abort).await;

    let mut saw_aborted_status = false;
    for _ in 0..80 {
        match recv_json(&mut ws).await {
            WsOutMessage::ProviderStatus {
                status: WsProviderStatus::Aborted,
            } => saw_aborted_status = true,
            WsOutMessage::StageChange { stage } if stage == "prepare_context" => {
                assert!(saw_aborted_status, "frontend must receive the existing aborted status");
                timeout(Duration::from_secs(1), abort_observed.notified())
                    .await
                    .expect("Pi provider must observe Abort before emitting post-abort events");
                let session = LifecycleStore::new(ProductAppPaths::new(root.path().join(".aria")))
                    .get_workspace_session("workspace_session_0001")
                    .expect("persisted workspace session");
                assert_eq!(session.status, cadence_aria::product::models::WorkspaceSessionStatus::Open);
                let no_trailing_output = timeout(Duration::from_millis(150), ws.next()).await;
                match no_trailing_output {
                    Err(_) => {}
                    Ok(Some(Ok(Message::Text(text)))) => {
                        let message: WsOutMessage =
                            serde_json::from_str(&text).expect("ws json after Abort");
                        assert!(
                            !matches!(message, WsOutMessage::StreamChunk { ref content, .. } if content.contains(PI_POST_ABORT_OUTPUT)),
                            "cancelled Pi run must suppress provider output emitted after Abort"
                        );
                        panic!("cancelled Pi run must not emit frontend output after Abort: {message:?}");
                    }
                    Ok(Some(Ok(other))) => {
                        panic!("cancelled Pi run must not emit websocket frame after Abort: {other:?}")
                    }
                    Ok(Some(Err(error))) => {
                        panic!("websocket read failed while checking post-abort output: {error}")
                    }
                    Ok(None) => panic!("websocket closed while checking post-abort output"),
                }
                drop(ws);

                let (mut reconnected, _) = connect_async(url).await.expect("reconnect ws");
                match recv_json(&mut reconnected).await {
                    WsOutMessage::SessionState {
                        stage,
                        active_run_id,
                        timeline_nodes,
                        ..
                    } => {
                        assert_eq!(stage, "prepare_context");
                        assert!(active_run_id.is_none());
                        let last = timeline_nodes.last().expect("cancelled Pi timeline node");
                        assert_eq!(last.status, TimelineNodeStatus::Failed);
                        assert_eq!(last.summary.as_deref(), Some("运行已中止"));
                    }
                    other => panic!("expected cancellation session state, got {other:?}"),
                }
                drop(reconnected);
                server.abort();
                return;
            }
            WsOutMessage::StreamChunk { .. } | WsOutMessage::MessageComplete { .. } => {
                panic!("cancelled Pi run must not emit output after Abort")
            }
            WsOutMessage::Error { message } => panic!("ws error: {message}"),
            _ => {}
        }
    }
    panic!("Pi abort did not return workspace to prepare_context");
}

#[tokio::test]
async fn workspace_ws_abort_after_choice_response_returns_prepare_context() {
    let root = tempdir().expect("root");
    create_workspace_session_fixture(&root).await;
    let mut registry = ProviderRegistry::new();
    registry.register(
        ProviderName::Fake,
        Arc::new(ChoiceThenHangingStreamingProvider),
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
            content: "run choice and hang provider".to_string(),
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
    send_json(&mut ws, &WsInMessage::Abort).await;

    let mut saw_aborted_status = false;
    for _ in 0..80 {
        match recv_json(&mut ws).await {
            WsOutMessage::ProviderStatus {
                status: WsProviderStatus::Aborted,
            } => saw_aborted_status = true,
            WsOutMessage::StageChange { stage } if stage == "prepare_context" => {
                assert!(saw_aborted_status);
                drop(ws);
                server.abort();
                return;
            }
            WsOutMessage::MessageComplete { .. } => {
                panic!("aborted choice run should not complete")
            }
            WsOutMessage::Error { message } => panic!("ws error: {message}"),
            _ => {}
        }
    }
    panic!("abort after choice response did not return workspace to prepare_context");
}

#[tokio::test]
async fn workspace_ws_test_permission_fixture_emits_permission_request_for_fake_provider() {
    let root = tempdir().expect("root");
    let _repo = create_workspace_session_fixture(&root).await;
    let state = WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    );
    state
        .test_controls
        .enable_permission_fixture("workspace_session_0001".to_string())
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
            content: "run permission fixture".to_string(),
        },
    )
    .await;

    let permission = recv_until_permission_request(&mut ws).await;
    assert_eq!(permission.tool_name, "Bash");
    assert_eq!(permission.description, "E2E permission fixture request");

    drop(ws);
    server.abort();
}

// 退役留档（T5/REQ-RET-02）：`workspace_ws_human_confirm_v2_completes_workspace` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

#[tokio::test]
async fn workspace_ws_unmatched_permission_response_returns_protocol_error() {
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
    send_json(
        &mut ws,
        &WsInMessage::PermissionResponse {
            id: "permission_not_pending".to_string(),
            approved: true,
            reason: Some("wrong request".to_string()),
        },
    )
    .await;

    match recv_until_protocol_error(&mut ws).await {
        WsOutMessage::ProtocolError { code, context, .. } => {
            assert_eq!(code, "PERMISSION_ID_UNMATCHED");
            assert_eq!(
                context
                    .as_ref()
                    .and_then(|value| value.get("permission_id"))
                    .and_then(|value| value.as_str()),
                Some("permission_not_pending")
            );
        }
        other => panic!("expected protocol_error, got {other:?}"),
    }

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

    drop(ws);
    server.abort();
}

// 退役留档（T5/REQ-RET-02）：`workspace_ws_codex_current_protocol_completes_from_repository_path` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

// spec-design-dialog-revision T9：旧「重连时处在 ReviewDecision 阶段」场景迁移——ReviewDecision 已从
// Story/Design 退役，等价场景为：review 完成回 AuthorConfirm 后断线，重连提交反馈修订仍可运行，
// 并可再次送审完成第二轮 review。
// 退役留档（T5/REQ-RET-02）：`workspace_ws_reconnect_after_review_can_still_run_revision` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

struct WorkingDirRecordingStreamingProvider {
    observed_working_dir: Arc<Mutex<Option<PathBuf>>>,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for WorkingDirRecordingStreamingProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        *self.observed_working_dir.lock().unwrap() = Some(input.working_dir);
        let (event_tx, event_rx) = mpsc::channel(8);
        let (command_tx, _command_rx) = mpsc::channel::<ProviderCommand>(8);
        tokio::spawn(async move {
            let _ = event_tx
                .send(ProviderEvent::TextDelta {
                    content: VALID_STORY_SPEC.to_string(),
                })
                .await;
            let _ = event_tx
                .send(ProviderEvent::Completed(cadence_aria::cross_cutting::streaming_provider::ProviderCompletion::plain(VALID_STORY_SPEC.to_string(), None)))
                .await;
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
    ) -> Result<
        mpsc::Receiver<cadence_aria::cross_cutting::streaming_provider::StreamChunk>,
        ProviderAdapterError,
    > {
        Err(ProviderAdapterError::execution_failed(
            None,
            String::new(),
            "run_streaming is not used by workspace websocket",
            0,
        ))
    }
}

struct ScriptedStreamingProvider {
    outputs: Mutex<VecDeque<String>>,
    prompts: Arc<Mutex<Vec<String>>>,
}

impl ScriptedStreamingProvider {
    fn new<const N: usize>(outputs: [&str; N], prompts: Arc<Mutex<Vec<String>>>) -> Self {
        Self {
            outputs: Mutex::new(outputs.into_iter().map(ToOwned::to_owned).collect()),
            prompts,
        }
    }
}

#[derive(Default)]
struct ChoiceThenArtifactProviderState {
    calls: Mutex<u32>,
    resume_ids: Mutex<Vec<Option<String>>>,
    prompts: Mutex<Vec<String>>,
}

struct ChoiceThenArtifactProvider {
    state: Arc<ChoiceThenArtifactProviderState>,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for ChoiceThenArtifactProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        self.state
            .resume_ids
            .lock()
            .unwrap()
            .push(input.resume_provider_session_id.clone());
        self.state.prompts.lock().unwrap().push(input.prompt);
        let mut calls = self.state.calls.lock().unwrap();
        *calls += 1;
        let call_no = *calls;
        drop(calls);

        let output = if call_no == 1 {
            "需要先确认一个边界条件，然后我再生成最终 Story Spec：\n\
             `climb_stairs(n)` 对 `n <= 0` 应该如何处理？\n\
             - **A)** 返回 `0`，仅把正整数楼梯数视为有效输入\n\
             - **B)** 抛出异常，例如 `ValueError`\n\
             - **C)** 不定义该行为，Story Spec 只覆盖 issue 明确要求的 `n >= 1` 场景"
        } else {
            "# Story Spec\n\n\
             ## 范围\n\
             来源 source id: Issue issue_0001；实现 climb_stairs。\n\n\
             ## 用户故事\n\
             作为调用方，我需要计算爬楼梯方法数。\n\n\
             ## 功能需求\n\
             - [REQ-001] 实现 `climb_stairs(n: i32) -> i32`。\n\n\
             ## 成功标准\n\
             - [AC-001] 覆盖 n=1、n=2、n=3、n=5、n=10。\n\n\
             ## 待确认项\n\
             无\n\n\
             ## 非功能需求\n\
             使用 Python 实现。"
        };
        let (event_tx, event_rx) = mpsc::channel(8);
        let (command_tx, _command_rx) = mpsc::channel::<ProviderCommand>(8);
        tokio::spawn(async move {
            let output = output.to_string();
            let _ = event_tx
                .send(ProviderEvent::TextDelta {
                    content: output.clone(),
                })
                .await;
            let _ = event_tx
                .send(ProviderEvent::Completed(cadence_aria::cross_cutting::streaming_provider::ProviderCompletion::plain(output, Some("author-provider-session-1".to_string()))))
                .await;
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

struct HangingStreamingProvider;
