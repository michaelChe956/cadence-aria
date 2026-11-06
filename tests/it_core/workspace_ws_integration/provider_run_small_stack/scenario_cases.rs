//! C 层真实 WS 场景执行体（任务10）：Story 空 Author / 最重 plan 路径 /
//! SC manual 修订门。
//!
//! 经 `provider_run_small_stack.rs` 顶层 `#[path]` 声明挂载——`use super::*`
//! 可达父 harness 模块私有 helper（真实 fixture / WS 收发工具）。共享件
//! （场景替身 / 帧收集器 / WS 辅助）在 `scenario_support.rs`。
use super::ws_scenario_support::{
    FrameCollector, PLAN_OUTLINE_AUTHOR_OUTPUT, PLAN_REVIEW_OUTPUT, ScenarioScriptedProvider,
    ScenarioStep, author_terminal_variant, connect_ws_with_origin, hello_driver_admission,
    http_health_check, recv_frame_bounded, serve_ws_app, start_generation_message,
};
use super::ws_small_stack::verify_ws_smoke_evidence;
use super::*;

/// 场景 1（Story 空 Author，D1 事故路径）：fresh Story 会话 StartGeneration
/// 驱动 Author 臂——run1 挂起后同连接 Abort（durable Failed 收口、失败回显
/// ≤1），run2 再生成完整收口；随后 /api/health + 重连 WS 证明服务未 abort。
pub(super) async fn run_story_author_scenario() -> Result<(), String> {
    let root = tempdir().expect("story scenario root");
    create_workspace_session_fixture_with_providers(&root, "claude_code", "fake", 1).await;
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let session_id = "workspace_session_0001".to_string();
    let prompts = Arc::new(Mutex::new(Vec::new()));
    let mut registry = ProviderRegistry::new();
    registry.register(
        ProviderName::ClaudeCode,
        Arc::new(ScenarioScriptedProvider {
            steps: Mutex::new(
                [
                    ScenarioStep::HangAfterFirstDelta,
                    ScenarioStep::Complete(VALID_STORY_SPEC),
                ]
                .into_iter()
                .collect::<VecDeque<_>>(),
            ),
            prompts: prompts.clone(),
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
    let (addr, server) = serve_ws_app(app).await;
    let url = format!("ws://{addr}/api/workspace-sessions/{session_id}/ws");
    let origin = format!("http://{addr}");

    let mut ws = connect_ws_with_origin(&url, &origin).await;
    let mut collector = FrameCollector::new(app_paths, &session_id, prompts.clone());
    let initial = collector.next(&mut ws).await;
    assert!(
        matches!(initial, WsOutMessage::SessionState { .. }),
        "expected initial session_state, got {initial:?}"
    );
    hello_driver_admission(&mut ws, &session_id).await;

    // run1：StartGeneration → locked 前奏 → 真实 start 的首个输出 → Abort。
    send_json(&mut ws, &start_generation_message()).await;
    collector
        .until(
            &mut ws,
            &|frame: &WsOutMessage| matches!(frame, WsOutMessage::StreamChunk { .. }),
            "run1 first stream_chunk（真实 provider start 后的本 run 输出）",
        )
        .await;
    // 可信 provider-start 探针：run1 至少一次真实 validated start（locked/登记
    // run 不计——探针只数 adapter 的真实 start_validated 调用）。
    let starts_run1 = prompts.lock().unwrap().len();
    assert!(
        starts_run1 >= 1,
        "run1 必须有可信 validated start（探针计数）"
    );
    assert!(
        collector.provider_locked_at < collector.first_output_at,
        "locked 前奏必须先于本 run 输出（frames={:?}）",
        collector.frames
    );
    send_json(&mut ws, &WsInMessage::Abort).await;
    collector.abort_sent = true;
    collector
        .until(
            &mut ws,
            &|frame: &WsOutMessage| {
                matches!(frame, WsOutMessage::StageChange { stage } if stage == "prepare_context")
            },
            "abort 后 stage 回 prepare_context（取消收口）",
        )
        .await;
    assert!(
        collector.completed_echo_at.is_none(),
        "被 Abort 的 run1 不得出现 message_complete（frames={:?}）",
        collector.frames
    );
    assert!(
        collector.cancel_failure_echoes <= 1,
        "取消失败回显必须 ≤1（frames={:?}）",
        collector.frames
    );
    let cancel_failed_durable = collector.durable_run_node_failed();
    println!(
        "[ws-2mib][story_author] run1 cancelled: durable_failed_node={cancel_failed_durable} \
         cancel_echoes={} frames={:?}",
        collector.cancel_failure_echoes, collector.frames
    );

    // run2（取消后再生成）：同一连接再次 StartGeneration，完整收口。
    send_json(&mut ws, &start_generation_message()).await;
    collector
        .until(
            &mut ws,
            &|frame: &WsOutMessage| matches!(frame, WsOutMessage::MessageComplete { .. }),
            "run2 message_complete（再生成完整收口）",
        )
        .await;
    // 可信 provider-start 探针（再生成）：run2 必须再次真实 validated start
    // （计数相对 run1 严格增加——证明取消后可再生成，不是复用旧登记）。
    let starts_run2 = prompts.lock().unwrap().len();
    assert!(
        starts_run2 > starts_run1,
        "run2 必须再次真实 validated start（取消后可再生成）：run1={starts_run1}, run2={starts_run2}"
    );
    let evidence = collector.into_evidence("story_author", starts_run2, true);
    // durable Completed 必须先于 completed 回显可观察（帧后检查点同序）。
    verify_ws_smoke_evidence(&evidence)
        .map_err(|error| format!("story_author 四条件证据不成立: {error}"))?;

    // ① 服务未 abort：同一 socket 的 /api/health + 全新 WS 重连。
    http_health_check(addr)
        .await
        .map_err(|error| format!("story_author health check failed: {error}"))?;
    let mut reconnected = connect_ws_with_origin(&url, &origin).await;
    let reload = recv_frame_bounded(&mut reconnected)
        .await
        .expect("C 层场景重连后必须收到 session_state 帧");
    assert!(
        matches!(reload, WsOutMessage::SessionState { .. }),
        "reconnect must yield session_state, got {reload:?}"
    );
    println!("[ws-2mib][story_author] evidence={evidence:?}");
    drop(ws);
    drop(reconnected);
    server.abort();
    Ok(())
}

/// Legacy WorkItemPlan 会话创建（part_08 同源手法：plan 实体 + Legacy flow
/// 会话；本场景 author/reviewer 均 ClaudeCode 挂场景替身）。
async fn create_legacy_plan_session(
    root: &TempDir,
    flow_kind_single_candidate: bool,
    review_rounds: u32,
) -> Result<String, String> {
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle = LifecycleStore::new(app_paths.clone());
    let plan = lifecycle
        .create_issue_work_item_plan(
            cadence_aria::product::lifecycle_store::CreateIssueWorkItemPlanInput {
                id: Some(if flow_kind_single_candidate {
                    "issue_work_item_plan_ws_2mib_sc".to_string()
                } else {
                    "issue_work_item_plan_ws_2mib_legacy".to_string()
                }),
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
        .map_err(|error| format!("create plan failed: {error}"))?;
    let session = lifecycle
        .create_workspace_session(
            cadence_aria::product::lifecycle_store::CreateWorkspaceSessionInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                entity_id: plan.id,
                workspace_type: cadence_aria::product::models::WorkspaceType::WorkItemPlan,
                author_provider: ProviderName::ClaudeCode,
                reviewer_provider: Some(ProviderName::ClaudeCode),
                review_rounds,
                superpowers_enabled: false,
                openspec_enabled: false,
                work_item_plan_options: Some(
                    cadence_aria::product::lifecycle_store::WorkItemPlanSessionOptions {
                        flow_kind: if flow_kind_single_candidate {
                            cadence_aria::product::work_item_plan_policy::WorkItemPlanFlowKind::SingleCandidate
                        } else {
                            cadence_aria::product::work_item_plan_policy::WorkItemPlanFlowKind::Legacy
                        },
                        run_policy:
                            cadence_aria::product::work_item_plan_policy::RunPolicy::Interactive,
                        rollout_snapshot: true,
                    },
                ),
            },
        )
        .map_err(|error| format!("create plan session failed: {error}"))?;
    Ok(session.id)
}

/// 场景 2（最重 plan 路径 + followups 接力）：Legacy WorkItemPlan 会话
/// StartGeneration 走 plan 子驱动深链（author outline → review → outline/
/// draft 接力，尽脚本化可达深度）；断言以真实帧/durable 为准。
pub(super) async fn run_plan_relay_scenario() -> Result<(), String> {
    let root = tempdir().expect("plan relay scenario root");
    create_workspace_session_fixture_with_providers(&root, "claude_code", "claude_code", 1).await;
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let session_id = create_legacy_plan_session(&root, false, 1).await?;
    let prompts = Arc::new(Mutex::new(Vec::new()));
    let mut registry = ProviderRegistry::new();
    registry.register(
        ProviderName::ClaudeCode,
        Arc::new(ScenarioScriptedProvider {
            steps: Mutex::new(
                [
                    // run1：挂起（首 delta 后等待取消）——让 Abort 落在**在途**
                    // legacy outline 臂上（否则 outline 已收口、无 active run，
                    // Abort 走 typed 拒收而非取消收口）。
                    ScenarioStep::HangAfterFirstDelta,
                    // run2（取消后再生成）：完整 outline 输出（sentinel 全文）。
                    ScenarioStep::Complete(PLAN_OUTLINE_AUTHOR_OUTPUT),
                    // 后续轮次（若接力再触发）：review 结构化输出。
                    ScenarioStep::Complete(PLAN_REVIEW_OUTPUT),
                    ScenarioStep::Complete(PLAN_OUTLINE_AUTHOR_OUTPUT),
                ]
                .into_iter()
                .collect::<VecDeque<_>>(),
            ),
            prompts: prompts.clone(),
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
    let (addr, server) = serve_ws_app(app).await;
    let url = format!("ws://{addr}/api/workspace-sessions/{session_id}/ws");
    let origin = format!("http://{addr}");

    let mut ws = connect_ws_with_origin(&url, &origin).await;
    let mut collector = FrameCollector::new(app_paths, &session_id, prompts.clone());
    let initial = collector.next(&mut ws).await;
    assert!(
        matches!(initial, WsOutMessage::SessionState { .. }),
        "expected initial session_state, got {initial:?}"
    );
    hello_driver_admission(&mut ws, &session_id).await;

    // plan 深链 run1：StartGeneration → locked → 真实 validated start → 首个
    // 输出（脚本化 provider 在首 delta 后挂起，使 Abort 落在在途臂上）。
    send_json(&mut ws, &start_generation_message()).await;
    collector
        .until(
            &mut ws,
            &|frame: &WsOutMessage| matches!(frame, WsOutMessage::StreamChunk { .. }),
            "plan author 首个 stream_chunk（真实 start 后的本 run 输出）",
        )
        .await;
    // 可信 provider-start 探针（run1）：至少一次真实 validated start。
    let starts_run1 = prompts.lock().unwrap().len();
    assert!(
        starts_run1 >= 1,
        "plan author 必须真实 validated start（探针计数）"
    );
    assert!(
        collector.provider_locked_at < collector.first_output_at,
        "locked 前奏必须先于本 run 输出（frames={:?}）",
        collector.frames
    );

    // 取消收口：Abort 落在在途 legacy outline 臂上 → stage 回 prepare_context。
    send_json(&mut ws, &WsInMessage::Abort).await;
    collector.abort_sent = true;
    collector
        .until(
            &mut ws,
            &|frame: &WsOutMessage| {
                matches!(frame, WsOutMessage::StageChange { stage } if stage == "prepare_context")
            },
            "abort 后 stage 回 prepare_context（plan 取消收口）",
        )
        .await;
    assert!(
        collector.completed_echo_at.is_none(),
        "被 Abort 的 plan run1 不得出现 message_complete（frames={:?}）",
        collector.frames
    );
    assert!(
        collector.cancel_failure_echoes <= 1,
        "plan 取消失败回显必须 ≤1（frames={:?}）",
        collector.frames
    );
    println!(
        "[ws-2mib][plan_relay] run1 cancelled: starts={starts_run1} cancel_echoes={} \
         frames={:?}",
        collector.cancel_failure_echoes, collector.frames
    );

    // 取消后再生成（run2）：同一连接再次 StartGeneration 必须再次真实 start，
    // 并完整收口（author outline 轮终态：message_complete / stage_change /
    // typed error —— 三者都是完整深链终态，按实际帧如实断言）。
    let starts_before_regen = prompts.lock().unwrap().len();
    send_json(&mut ws, &start_generation_message()).await;
    let run2_terminal = collector
        .until(
            &mut ws,
            &|frame: &WsOutMessage| {
                matches!(frame, WsOutMessage::MessageComplete { .. })
                    || matches!(frame, WsOutMessage::StageChange { .. })
                    || matches!(frame, WsOutMessage::Error { .. })
            },
            "plan 再生成 run 的终态（message_complete / stage_change / typed error）",
        )
        .await;
    // start 探针先于任何输出帧递增（start_validated 返回会话后才有 chunk）。
    let total_starts = prompts.lock().unwrap().len();
    assert!(
        total_starts > starts_before_regen,
        "plan 再生成必须再次真实 start（before={starts_before_regen}, after={total_starts}）"
    );
    println!(
        "[ws-2mib][plan_relay] run2 terminal: {}（starts={}）",
        author_terminal_variant(&run2_terminal),
        total_starts
    );
    let evidence = collector.into_evidence("plan_relay", total_starts, true);
    verify_ws_smoke_evidence(&evidence)
        .map_err(|error| format!("plan_relay 四条件证据不成立: {error}"))?;

    http_health_check(addr)
        .await
        .map_err(|error| format!("plan_relay health check failed: {error}"))?;
    let mut reconnected = connect_ws_with_origin(&url, &origin).await;
    let reload = recv_frame_bounded(&mut reconnected)
        .await
        .expect("C 层场景重连后必须收到 session_state 帧");
    assert!(
        matches!(reload, WsOutMessage::SessionState { .. }),
        "reconnect must yield session_state, got {reload:?}"
    );
    println!("[ws-2mib][plan_relay] evidence={evidence:?}");
    drop(ws);
    drop(reconnected);
    server.abort();
    Ok(())
}

/// 场景 3（SC manual 修订臂）：SC WorkItemPlan 会话手术到 Approval 门，
/// HumanGateFeedback 触发 manual revision 臂真实 start（part_04 已验证
/// feedback 派发可达 HumanGateTurnOpen；本场景延伸到修订 provider start 与
/// turn 收口），并验证取消后再生成（第二 feedback turn 再次真实 start）。
pub(super) async fn run_sc_manual_gate_scenario() -> Result<(), String> {
    let root = tempdir().expect("sc manual scenario root");
    create_workspace_session_fixture(&root).await;
    let session_id = create_legacy_plan_session(&root, true, 0).await?;
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle = LifecycleStore::new(app_paths.clone());
    let mut record = lifecycle
        .get_workspace_session(&session_id)
        .map_err(|error| format!("load SC session failed: {error}"))?;
    record.status = cadence_aria::product::models::WorkspaceSessionStatus::WaitingForHuman;
    record.single_candidate_phase =
        Some(cadence_aria::product::models::SingleCandidatePhase::Approval);
    record.human_gate_snapshot = Some(
        cadence_aria::product::work_item_plan_policy::HumanGateSnapshot {
            findings: vec![],
            repeated_fingerprints: vec![],
            attempts_used: 0,
            manual_repairs_remaining: 2,
            trigger: cadence_aria::product::work_item_plan_policy::HumanReason::NativeHumanRequired,
            resumable: false,
            accepted_feedback_turns: None,
            candidate_recovery: None,
        },
    );
    record
        .messages
        .push(cadence_aria::product::models::WorkspaceMessageRecord {
            role: "assistant".to_string(),
            content: SC_HUMAN_GATE_CANDIDATE.to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
        });
    let session_path = app_paths
        .issue_root(&record.project_id, &record.issue_id)
        .join("workspace-sessions")
        .join(format!("{}.json", record.id));
    cadence_aria::product::json_store::write_json(&session_path, &record)
        .map_err(|error| format!("persist SC approval state failed: {error}"))?;

    let prompts = Arc::new(Mutex::new(Vec::new()));
    let mut registry = ProviderRegistry::new();
    registry.register(
        ProviderName::ClaudeCode,
        Arc::new(ScenarioScriptedProvider {
            steps: Mutex::new(
                [
                    // turn1：完整候选输出（turn 收口后门仍在 human_confirm，
                    // 可继续开第二回合）。
                    ScenarioStep::Complete(SC_HUMAN_GATE_CANDIDATE),
                    // turn2（再生成）：第二回合再次真实 start。
                    ScenarioStep::Complete(SC_HUMAN_GATE_CANDIDATE),
                ]
                .into_iter()
                .collect::<VecDeque<_>>(),
            ),
            prompts: prompts.clone(),
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
    let (addr, server) = serve_ws_app(app).await;
    let url = format!("ws://{addr}/api/workspace-sessions/{session_id}/ws");
    let origin = format!("http://{addr}");

    let mut ws = connect_ws_with_origin(&url, &origin).await;
    let mut collector = FrameCollector::new(app_paths, &session_id, prompts.clone());
    let initial = collector.next(&mut ws).await;
    assert!(
        matches!(initial, WsOutMessage::SessionState { .. }),
        "expected initial session_state, got {initial:?}"
    );
    hello_driver_admission(&mut ws, &session_id).await;

    // HumanGateFeedback → manual revision 臂：turn open → 真实 start → 收口。
    send_json(
        &mut ws,
        &WsInMessage::HumanGateFeedback {
            command_id: "cmd-ws-2mib-sc-manual-1".to_string(),
            feedback: "请保留完整候选，仅修正追踪关系".to_string(),
            excluded_finding_ids: vec![],
        },
    )
    .await;
    collector
        .until(
            &mut ws,
            &|frame: &WsOutMessage| {
                matches!(frame, WsOutMessage::HumanGateTurnOpen { .. })
                    || matches!(frame, WsOutMessage::HumanGateTurnRejected { .. })
                    || matches!(frame, WsOutMessage::HumanGateBusy { .. })
            },
            "human gate turn open / typed 拒收",
        )
        .await;
    // turn1：真实 provider start（可信探针）+ 首个输出帧 + turn 终态。
    // 用单次收集（until_both）——门内 typed 拒收帧可能先于输出帧到达，
    // 分两次 until 会把先到的终态帧吃掉。
    let (turn1_output, turn1_terminal) = collector
        .until_both(
            &mut ws,
            &|frame: &WsOutMessage| matches!(frame, WsOutMessage::StreamChunk { .. }),
            &|frame: &WsOutMessage| {
                matches!(frame, WsOutMessage::HumanGateTurnCompleted { .. })
                    || matches!(frame, WsOutMessage::HumanGateTurnFailed { .. })
                    || matches!(frame, WsOutMessage::HumanGateTurnRejected { .. })
                    || matches!(frame, WsOutMessage::HumanGateClosed { .. })
                    || matches!(frame, WsOutMessage::ProtocolError { .. })
                    || matches!(frame, WsOutMessage::Error { .. })
            },
            "manual revision turn1（输出帧 + turn 终态）",
        )
        .await;
    // 可信 provider-start 探针（turn1）：至少一次真实 validated start。
    let starts = prompts.lock().unwrap().len();
    assert!(
        starts >= 1,
        "SC manual 修订臂必须真实 validated start（starts={starts}）"
    );
    assert!(
        matches!(turn1_output, WsOutMessage::StreamChunk { .. }),
        "turn1 必须观察到真实 provider 输出帧"
    );
    println!(
        "[ws-2mib][sc_manual_gate] turn1 terminal: {} starts={starts}",
        author_terminal_variant(&turn1_terminal)
    );
    assert!(
        collector.cancel_failure_echoes <= 1,
        "SC 取消失败回显必须 ≤1（frames={:?}）",
        collector.frames
    );

    // 再生成（门内语义）：第二 feedback turn 必须再次真实 start。
    let starts_before_regen = prompts.lock().unwrap().len();
    send_json(
        &mut ws,
        &WsInMessage::HumanGateFeedback {
            command_id: "cmd-ws-2mib-sc-manual-2".to_string(),
            feedback: "再次修订：补齐成功标准".to_string(),
            excluded_finding_ids: vec![],
        },
    )
    .await;
    let (turn2_output, _turn2_terminal) = collector
        .until_both(
            &mut ws,
            &|frame: &WsOutMessage| matches!(frame, WsOutMessage::StreamChunk { .. }),
            &|frame: &WsOutMessage| {
                matches!(frame, WsOutMessage::HumanGateTurnCompleted { .. })
                    || matches!(frame, WsOutMessage::HumanGateTurnFailed { .. })
                    || matches!(frame, WsOutMessage::HumanGateTurnRejected { .. })
                    || matches!(frame, WsOutMessage::HumanGateClosed { .. })
                    || matches!(frame, WsOutMessage::HumanGateBusy { .. })
                    || matches!(frame, WsOutMessage::ProtocolError { .. })
                    || matches!(frame, WsOutMessage::Error { .. })
            },
            "第二修订 turn（输出帧 + turn 终态）",
        )
        .await;
    assert!(
        matches!(turn2_output, WsOutMessage::StreamChunk { .. }),
        "第二修订 turn 必须观察到真实 provider 输出帧"
    );
    let total_starts = prompts.lock().unwrap().len();
    assert!(
        total_starts > starts_before_regen,
        "SC 取消后必须能再次真实 start（before={starts_before_regen}, after={total_starts}）"
    );
    let evidence = collector.into_evidence("sc_manual_gate", total_starts, false);
    verify_ws_smoke_evidence(&evidence)
        .map_err(|error| format!("sc_manual_gate 四条件证据不成立: {error}"))?;

    http_health_check(addr)
        .await
        .map_err(|error| format!("sc_manual_gate health check failed: {error}"))?;
    let mut reconnected = connect_ws_with_origin(&url, &origin).await;
    let reload = recv_frame_bounded(&mut reconnected)
        .await
        .expect("C 层场景重连后必须收到 session_state 帧");
    assert!(
        matches!(reload, WsOutMessage::SessionState { .. }),
        "reconnect must yield session_state, got {reload:?}"
    );
    println!(
        "[ws-2mib][sc_manual_gate] evidence={evidence:?} second_turn_starts={}",
        total_starts - starts
    );
    drop(ws);
    drop(reconnected);
    server.abort();
    Ok(())
}
