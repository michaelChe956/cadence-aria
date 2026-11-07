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
                reviewer_provider: Some(ProviderName::Fake),

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
            accepted_feedback_turns: None,
            candidate_recovery: None,
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
        excluded_finding_ids: vec![], },
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

/// REQ-WSC-13（k3 P0-2A 修法 A）：排除-only 帧（`feedback` 为空 + 平台
/// `finding_key`）必须被**受理**——落 durable 已处置清单、回受理回执、
/// **不发 `INVALID_HUMAN_GATE_FEEDBACK`**（现场：门卡已显示「已排除」，聊天里
/// 却冒出原始错误码 = 本 change 要消灭的「用户不知道如何继续」）。
#[tokio::test]
async fn workspace_ws_sc_exclude_only_feedback_is_accepted_without_gate_turn() {
    let root = tempdir().expect("root");
    let _repo = create_workspace_session_fixture(&root).await;
    let lifecycle = LifecycleStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let plan = lifecycle
        .create_issue_work_item_plan(
            cadence_aria::product::lifecycle_store::CreateIssueWorkItemPlanInput {
                id: Some("issue_work_item_plan_sc_exclude_only".to_string()),
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
                reviewer_provider: Some(ProviderName::Fake),
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
            accepted_feedback_turns: None,
            candidate_recovery: None,
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
            command_id: "cmd-socket-exclude-only".to_string(),
            feedback: String::new(),
            excluded_finding_ids: vec!["0123456789abcdef".to_string()],
        },
    )
    .await;

    match recv_json(&mut ws).await {
        WsOutMessage::HumanGateExclusionsAccepted {
            command_id,
            excluded_finding_ids,
        } => {
            assert_eq!(command_id, "cmd-socket-exclude-only");
            assert_eq!(
                excluded_finding_ids,
                vec!["0123456789abcdef".to_string()],
                "受理回执回传同一批平台 key"
            );
        }
        other => panic!("排除-only 帧必须被受理（无错误帧），实际 {other:?}"),
    }

    // durable 已处置清单已落盘（引擎受理事实；重启后同一清单可读）。
    let dispositions = lifecycle
        .list_human_dispositions(&session.id)
        .expect("read human dispositions");
    assert_eq!(dispositions.len(), 1);
    assert_eq!(dispositions[0].finding_key, "0123456789abcdef");

    // 未建修订 turn：没有任何 HumanGateTurn 记录被开出。
    assert!(
        lifecycle
            .list_human_gate_turns(&session.id)
            .expect("list human gate turns")
            .is_empty(),
        "排除-only 帧不得建修订 turn"
    );

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
    let app = build_web_router(with_available_provider_health(
        WebAppState::with_provider_registry(
            root.path().to_path_buf(),
            WebRuntime::new_fake(root.path().to_path_buf()),
            registry,
        ),
        root.path(),
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
    // 「workspace 以 Pi 在 Auto 权限模式下发起 author run」在可观察层断言。
    // LC gateway validated 启动面按设计只外借 launch_view()（role/working_dir/
    // prompt），provider_type/permission_mode 不外借（into_parts 为
    // pub(crate)），adapter 侧不可观察；改为断言引擎构造 launch input 的同一
    // durable 事实源——build_streaming_input 读 session.author_provider ×
    // session.permission_modes.author，经 provider_type_for_name /
    // permission_mode_for_provider 映射成 input 的 (Pi, Auto)（Pi 恒 Auto），
    // 外加引擎启动登记的 durable provider_start_ledger（启动前原子落盘的
    // actual-launch 观察）。三项合并后 (Pi, Auto) 语义不减。
    let durable_session = LifecycleStore::new(ProductAppPaths::new(root.path().join(".aria")))
        .get_workspace_session("workspace_session_0001")
        .expect("persisted workspace session");
    assert_eq!(
        durable_session.author_provider,
        cadence_aria::product::models::ProviderName::Pi,
        "workspace must invoke Pi in Auto mode (durable author provider)"
    );
    assert_eq!(
        durable_session.permission_modes.author,
        cadence_aria::cross_cutting::streaming_provider::ProviderPermissionMode::Auto,
        "workspace must invoke Pi in Auto mode (durable author permission mode)"
    );
    let actual_launches: Vec<&str> = durable_session
        .provider_start_ledger
        .iter()
        .filter(|entry| entry.is_actual_launch_observation())
        .map(|entry| entry.provider.as_deref().unwrap_or("<missing>"))
        .collect();
    assert_eq!(
        actual_launches,
        vec!["pi"],
        "engine must have registered exactly one actual Pi author launch"
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
                        assert_eq!(last.status, TimelineNodeStatus::Aborted);
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
    // LC 会话必经 gateway：替身挂 ClaudeCode 名（Fake 不被 gateway 支持）。
    create_workspace_session_fixture_with_author(&root, "claude_code").await;
    let mut registry = ProviderRegistry::new();
    registry.register(
        ProviderName::ClaudeCode,
        Arc::new(ChoiceThenHangingStreamingProvider),
    );
    let app = build_web_router(with_available_provider_health(
        WebAppState::with_provider_registry(
            root.path().to_path_buf(),
            WebRuntime::new_fake(root.path().to_path_buf()),
            registry,
        ),
        root.path(),
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
            command_id: None,
            expected_run_id: None,
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
    let _repo = create_workspace_session_fixture_with_author(&root, "claude_code").await;
    let state = with_available_provider_health(
        WebAppState::new(
            root.path().to_path_buf(),
            WebRuntime::new_fake(root.path().to_path_buf()),
        ),
        root.path(),
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

    let app = build_web_router(with_available_provider_health(
        WebAppState::with_provider_registry(
            root.path().to_path_buf(),
            WebRuntime::new_fake(root.path().to_path_buf()),
            registry,
        ),
        root.path(),
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

impl WorkingDirRecordingStreamingProvider {
    /// 裸 `start`（单仓 direct 历史路径）与 `start_validated`（LC
    /// gateway 路径）共用的会话构造。
    fn recording_session(&self, working_dir: PathBuf) -> ProviderSession {
        *self.observed_working_dir.lock().unwrap() = Some(working_dir);
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
        ProviderSession {
            native_session_id: None,
            events: event_rx,
            commands: command_tx,
        }
    }
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for WorkingDirRecordingStreamingProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        Ok(self.recording_session(input.working_dir))
    }

    /// LC 会话经 gateway validated 启动：观察点从 adapter 侧的裸 input
    /// 迁到 validated 启动事实的只读投影（`launch_view().working_dir`）——
    /// 投影借出的正是本次 launch 的实际 spawn cwd，语义不减；按 trait
    /// 契约不回调裸 start。
    async fn start_validated(
        &self,
        validated: cadence_aria::cross_cutting::session_launch::ValidatedStreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let working_dir = validated.launch_view().working_dir.to_path_buf();
        Ok(self.recording_session(working_dir))
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
    /// 替身自己发出的原生会话 id（completion 的 provider_session_id）——
    /// validated 启动面不外借 resume_provider_session_id，续接事实只能由
    /// 替身自身的上一轮会话 id 派生。
    last_native_session_id: Mutex<Option<String>>,
}

struct ChoiceThenArtifactProvider {
    state: Arc<ChoiceThenArtifactProviderState>,
}

impl ChoiceThenArtifactProvider {
    /// 裸 `start`（单仓 direct 历史路径）与 `start_validated`（LC
    /// gateway 路径）共用的会话构造。
    fn choice_then_artifact_session(
        &self,
        resume_provider_session_id: Option<String>,
        prompt: String,
    ) -> ProviderSession {
        self.state
            .resume_ids
            .lock()
            .unwrap()
            .push(resume_provider_session_id);
        self.state.prompts.lock().unwrap().push(prompt);
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
        let native_session_id = "author-provider-session-1".to_string();
        let emitted_session_id = native_session_id.clone();
        tokio::spawn(async move {
            let output = output.to_string();
            let _ = event_tx
                .send(ProviderEvent::TextDelta {
                    content: output.clone(),
                })
                .await;
            let _ = event_tx
                .send(ProviderEvent::Completed(cadence_aria::cross_cutting::streaming_provider::ProviderCompletion::plain(output, Some(emitted_session_id))))
                .await;
        });
        *self.state.last_native_session_id.lock().unwrap() = Some(native_session_id);
        ProviderSession {
            native_session_id: None,
            events: event_rx,
            commands: command_tx,
        }
    }
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for ChoiceThenArtifactProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        Ok(self.choice_then_artifact_session(
            input.resume_provider_session_id.clone(),
            input.prompt,
        ))
    }

    /// LC 会话经 gateway validated 启动：观察点从 adapter 侧的裸 input
    /// 迁到 validated 启动事实的只读投影（`launch_view().prompt`）——投影
    /// 借出的正是本次 launch 的实际 prompt，语义不减；resume 事实由替身
    /// 自身上一轮发出的原生会话 id 派生（validated 面不外借该字段）。
    /// 按 trait 契约不回调裸 start。
    async fn start_validated(
        &self,
        validated: cadence_aria::cross_cutting::session_launch::ValidatedStreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let resume_provider_session_id =
            self.state.last_native_session_id.lock().unwrap().clone();
        let prompt = validated.launch_view().prompt.to_string();
        Ok(self.choice_then_artifact_session(resume_provider_session_id, prompt))
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
