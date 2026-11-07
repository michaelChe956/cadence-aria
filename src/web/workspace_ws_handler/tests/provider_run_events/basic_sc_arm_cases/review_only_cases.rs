//! P2（任务3，brief task-3 Step 1）：ReviewOnly 臂两路（gateway/传统）的
//! durable Completed 帧时序用例——自 `basic_sc_arm_cases.rs` 拆出
//! （行数门禁 1000~1200 继续拆）。
//!
//! - 本 turn 帧只在 durable Completed（verdict 面）之后发送；
//! - turn 未 Completed 不发；grant 已消费（迟到已完 turn 形态）不重复发；
//! - 夹具根（TempDir / ProviderRunFixture）必须活过臂内 LifecycleStore 读写。

use super::*;

/// grant/turn durable 种子：grant.turn_id 绑定指定 turn；turn 记录按
/// status/artifact 写入（HumanGateTurn 同源校验字段齐备）。
fn seed_review_only_turn(
    app_paths: &ProductAppPaths,
    record: &crate::product::models::WorkspaceSessionRecord,
    turn_id: &str,
    status: crate::product::models::HumanGateTurnStatus,
    artifact_ref: Option<String>,
    bind_grant: bool,
) {
    let mut next = record.clone();
    if bind_grant {
        next.structural_governance_state =
            Some(crate::product::models::StructuralGovernanceState {
                current_grant: Some(crate::product::models::StructuralRevisionGrant {
                    grant_id: format!("grant-{turn_id}"),
                    authorization_hash: String::new(),
                    turn_id: Some(turn_id.to_string()),
                    command_id: Some(format!("cmd-{turn_id}")),
                    candidate_source_revision_ref: None,
                    plan_candidate_ir_ref: None,
                    mechanical_report_ref: None,
                    phase: None,
                    attempt_no: 0,
                    prompt_ref: None,
                    prompt_digest: None,
                    instruction_digest: None,
                    failing_source_hash: None,
                    review_identity: None,
                    granted_at: "2026-11-06T00:00:00Z".to_string(),
                }),
                ..Default::default()
            });
    }
    let session_path = app_paths
        .issue_root(&next.project_id, &next.issue_id)
        .join("workspace-sessions")
        .join(format!("{}.json", next.id));
    crate::product::json_store::write_json(&session_path, &next).expect("rewrite session record");
    let turn_path = app_paths
        .issue_root(&next.project_id, &next.issue_id)
        .join("workspace-sessions")
        .join(&next.id)
        .join("human-gate-turns")
        .join(format!("{turn_id}.json"));
    let turn = crate::product::models::HumanGateTurn {
        turn_id: turn_id.to_string(),
        session_id: next.id.clone(),
        command_id: format!("cmd-{turn_id}"),
        feedback_text: "case 反馈".to_string(),
        status,
        attempt_no: 1,
        budget_reserved: 1,
        source_hash: "a".repeat(64),
        result_artifact_ref: artifact_ref,
        failure_class: None,
        created_at: "2026-11-06T00:00:00Z".to_string(),
        updated_at: "2026-11-06T00:00:00Z".to_string(),
    };
    crate::product::json_store::write_json(&turn_path, &turn).expect("write human gate turn");
}

/// 读取下一条 human_gate_turn_completed 帧（2s 超时 fail-closed，附 turn
/// 与 durable 现场诊断）。
async fn case_next_completed_turn_frame(
    outbound_rx: &mut mpsc::Receiver<OutboundControl>,
    turn_id: &str,
    app_paths: &ProductAppPaths,
    session_id: &str,
) -> serde_json::Value {
    match tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let outbound = outbound_rx.recv().await.expect("outbound control expected");
            let OutboundControl::Text(json) = outbound else {
                continue;
            };
            let value: serde_json::Value = serde_json::from_str(&json).expect("outbound json");
            if value["type"] == "human_gate_turn_completed" {
                return value;
            }
        }
    })
    .await
    {
        Ok(frame) => frame,
        Err(_) => {
            let record = LifecycleStore::new(app_paths.clone())
                .get_workspace_session(session_id)
                .expect("reload session for diagnostics");
            let grant_turn = record
                .structural_governance_state
                .as_ref()
                .and_then(|state| state.current_grant.as_ref())
                .and_then(|grant| grant.turn_id.clone());
            let turn_state = LifecycleStore::new(app_paths.clone())
                .get_human_gate_turn(session_id, turn_id)
                .map(|turn| (turn.status, turn.result_artifact_ref))
                .map_err(|error| error.to_string());
            panic!(
                "human_gate_turn_completed frame expected: turn={turn_id} \
                 durable grant_turn={grant_turn:?} turn_state={turn_state:?}"
            );
        }
    }
}

/// 观察窗口内不得出现 human_gate_turn_completed 帧。
async fn case_assert_no_completed_turn_frame(
    outbound_rx: &mut mpsc::Receiver<OutboundControl>,
    note: &str,
) {
    assert!(
        !matches!(
            tokio::time::timeout(std::time::Duration::from_millis(150), outbound_rx.recv()).await,
            Ok(Some(OutboundControl::Text(json)))
                if serde_json::from_str::<serde_json::Value>(&json)
                    .map(|value| value["type"] == "human_gate_turn_completed")
                    .unwrap_or(false)
        ),
        "{note}"
    );
}

/// ReviewOnly 两路：gateway/传统 review 收口后，本 turn 帧只在 durable
/// Completed（verdict 面）之后发送；turn 未 Completed 不发；grant 已消费
/// （迟到已完 turn 形态）不重复发。
#[tokio::test]
async fn review_only_completed_frame_waits_for_durable_and_skips_late_turns() {
    // —— 传统路（无 gateway）——
    // 路A：turn durable Completed + artifact + grant 绑定 → 恰一帧（值同源）。
    run_review_only_case(
        "turn-case-completed",
        crate::product::models::HumanGateTurnStatus::Completed,
        Some("artifact_case_002".to_string()),
        true,
        false,
    )
    .await;
    // 路B：turn 仍 Running（verdict 面未收口）→ 不发本 turn Completed。
    run_review_only_case(
        "turn-case-running",
        crate::product::models::HumanGateTurnStatus::Running,
        Some("artifact_case_pending".to_string()),
        true,
        false,
    )
    .await;
    // 路C：grant 已消费（迟到已完 turn 的消费后形态）→ 不重复发帧。
    run_review_only_case(
        "turn-case-late",
        crate::product::models::HumanGateTurnStatus::Completed,
        Some("artifact_case_late".to_string()),
        false,
        false,
    )
    .await;
    // —— gateway 路（engine 注入真实 gateway）——
    run_review_only_case(
        "turn-case-gateway",
        crate::product::models::HumanGateTurnStatus::Completed,
        Some("artifact_case_gateway".to_string()),
        true,
        true,
    )
    .await;
}

/// 单场景直驱：`gateway=false` 传统路（自持 TempDir 的 Story 夹具，engine
/// 无 gateway）/ `gateway=true` gateway 路（LC 夹具，engine 注入真实
/// gateway）；帧期望由 durable 种子形态推导（Completed+artifact+grant
/// 绑定缺一不发）。
async fn run_review_only_case(
    turn_id: &str,
    status: crate::product::models::HumanGateTurnStatus,
    artifact_ref: Option<String>,
    bind_grant: bool,
    gateway: bool,
) {
    let expect_frame = bind_grant
        && status == crate::product::models::HumanGateTurnStatus::Completed
        && artifact_ref.is_some();
    // 出站帧走专用存活通道（夹具自带的 outbound 接收端已丢弃，发送会静默
    // 失败——帧断言必须面向本用例自己的接收端）。
    let (outbound_tx, mut outbound_rx) = mpsc::channel::<OutboundControl>(64);
    // 夹具根存活句柄：TempDir / ProviderRunFixture 必须活过臂内 LifecycleStore
    // 读写（块内局部夹具在块尾即被 drop，durable 目录随之删除——臂读恒
    // NotFound，帧永不发）。以 `Box<dyn Any>` 保活至函数尾。
    let run_context;
    let root_holder: Box<dyn std::any::Any> = if gateway {
        // LC 夹具（成员规则齐备，engine 注入真实 gateway）→
        // drive_review_session_via_gateway 分支。
        let (input_tx, _input_rx) = mpsc::unbounded_channel();
        let provider = Arc::new(CaseAuthorProvider {
            output: Some(String::new()),
            starts: Arc::new(AtomicUsize::new(0)),
            inputs: input_tx,
        });
        let fixture = ProviderRunFixture::new_logical(true, provider.clone());
        let mut registry = ProviderRegistry::new();
        registry.register(ProviderName::ClaudeCode, provider);
        let mut fixture_run_context = ProviderRunContext::test_fixture(
            Arc::new(registry),
            fixture.engine.clone(),
            fixture.workspace_runs.clone(),
            fixture.record.id.clone(),
            fixture.app_paths.clone(),
            fixture.record.clone(),
        );
        fixture_run_context.manager = fixture.manager.clone();
        seed_review_only_turn(
            &fixture_run_context.app_paths,
            &fixture_run_context.session_record,
            turn_id,
            status,
            artifact_ref.clone(),
            bind_grant,
        );
        run_context = fixture_run_context;
        Box::new(fixture)
    } else {
        // 传统路（无 gateway）：自持 TempDir 的 Story 夹具——durable 种子与
        // 臂内 LifecycleStore 读必须面向存活目录（stack_budget_fixture 的
        // TempDir 在其返回时即被删除，仅适合纯内存量测）。
        let root = tempfile::tempdir().expect("temporary workspace root");
        let app_paths = ProductAppPaths::new(root.path().join(".aria"));
        let lifecycle = LifecycleStore::new(app_paths.clone());
        let session_record = lifecycle
            .create_workspace_session(CreateWorkspaceSessionInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                entity_id: "story_0001".to_string(),
                workspace_type: WorkspaceType::Story,
                author_provider: ProviderName::ClaudeCode,
                reviewer_provider: Some(ProviderName::Codex),
                review_rounds: 0,
                superpowers_enabled: false,
                openspec_enabled: false,
                work_item_plan_options: None,
            })
            .expect("workspace session");
        let (engine_tx, engine_rx) = mpsc::channel(64);
        drop(engine_rx);
        let engine = Arc::new(Mutex::new(WorkspaceEngine::new_persistent(
            Arc::new(CheckpointStore::new(root.path().join("checkpoints"))),
            lifecycle,
            engine_tx,
            WorkspaceSession::from_record(session_record.clone()),
        )));
        let (story_input_tx, _story_input_rx) = mpsc::unbounded_channel();
        let provider = Arc::new(CaseAuthorProvider {
            output: Some(String::new()),
            starts: Arc::new(AtomicUsize::new(0)),
            inputs: story_input_tx,
        });
        let mut registry = ProviderRegistry::new();
        registry.register(ProviderName::ClaudeCode, provider);
        let workspace_runs = WorkspaceRunRegistry::default();
        let story_run_context = ProviderRunContext::test_fixture(
            Arc::new(registry),
            engine,
            workspace_runs,
            session_record.id.clone(),
            app_paths,
            session_record,
        );
        seed_review_only_turn(
            &story_run_context.app_paths,
            &story_run_context.session_record,
            turn_id,
            status,
            artifact_ref.clone(),
            bind_grant,
        );
        run_context = story_run_context;
        Box::new(root)
    };
    let _root_holder = root_holder;
    let run_cancel = CancellationToken::new();
    let (_command_tx, command_rx) = mpsc::channel(8);
    let run_token = 1;
    let provider_for_run = run_context
        .provider_registry
        .get(&ProviderName::ClaudeCode)
        .expect("fixture provider");
    let engine_guard = run_context.engine.lock().await;
    let mut idle_guard = Some(
        run_context
            .workspace_runs
            .begin_provider_drive(&run_context.session_id),
    );
    let exit = super::run::drive_review_only_arm(arm_drive(
        engine_guard,
        command_rx,
        provider_for_run,
        &run_context,
        &outbound_tx,
        &mut idle_guard,
        &run_cancel,
        run_token,
    ))
    .await;
    assert!(
        matches!(exit, super::run::ArmExit::Continue(_)),
        "ReviewOnly 两路正常收口必须 Continue 交还 guard（turn_id={turn_id}）"
    );
    if expect_frame {
        let frame = case_next_completed_turn_frame(
            &mut outbound_rx,
            turn_id,
            &run_context.app_paths,
            &run_context.session_id,
        )
        .await;
        assert_eq!(
            frame["turn_id"].as_str(),
            Some(turn_id),
            "帧必须回呈本 turn：{frame}"
        );
        assert_eq!(
            frame["artifact_ref"].as_str(),
            artifact_ref.as_deref(),
            "帧 artifact 必须与 durable 同源：{frame}"
        );
    } else {
        case_assert_no_completed_turn_frame(
            &mut outbound_rx,
            "turn 未 durable Completed 或 grant 已消费（迟到已完 turn）不得发帧",
        )
        .await;
    }
}
