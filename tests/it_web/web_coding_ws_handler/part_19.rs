/// F-44（可用面）：中止终态经显式 `restart_coding` 重开——重走 admission CAS 回
/// Running，并由既有 runner 阶段链续跑（阶段门 + provider 角色运行）。
/// `work_item` 作用域 attempt 无 group unit 目标约束，是重开的完整可用面。
#[tokio::test]
async fn aborted_work_item_attempt_restart_coding_reopens_and_resumes_runner_pipeline() {
    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let (app, _state, _repo_path) = app_with_coding_ws_resume_fixture(root.path(), true, true, true);
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            "coding_attempt_0001",
            CodingAttemptStatus::Aborted,
        )
        .expect("abort work item attempt");

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    let url = format!("ws://{addr}/ws/coding-attempts/coding_attempt_0001");
    let (mut ws, _) = connect_async(url).await.expect("connect ws");
    let _initial = recv_json(&mut ws).await;
    send_json(&mut ws, &CodingWsInMessage::RestartCoding).await;

    // 1) wire：不得出现终态拒绝（重开前为 coding_message_not_allowed），且 runner
    //    既有阶段链推进到 Coding 阶段门。
    let mut saw_stage_gate = false;
    for _ in 0..50 {
        match recv_json(&mut ws).await {
            CodingWsOutMessage::CodingProtocolError { code, message } => {
                panic!("restart_coding must not be rejected: {code}: {message}");
            }
            CodingWsOutMessage::CodingGateRequired { gate }
                if gate.kind == CodingGateKind::StageGate
                    && gate.stage == Some(CodingExecutionStage::Coding) =>
            {
                saw_stage_gate = true;
                break;
            }
            _ => {}
        }
    }
    assert!(
        saw_stage_gate,
        "重开后 runner 既有阶段链必须推进到 Coding 阶段门"
    );

    // 2) durable：CAS 回 Running 且终态时间戳清除。
    let reopened = store
        .get_attempt("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("attempt after restart");
    assert_eq!(reopened.status, CodingAttemptStatus::Running);
    assert_eq!(reopened.completed_at, None, "重开必须清除终态时间戳");

    // 3) provider：显式确认阶段门后 coder 角色运行必须真的重新落地（runner 真正续跑）。
    send_json(
        &mut ws,
        &CodingWsInMessage::StageGateConfirm {
            stage: CodingExecutionStage::Coding,
        },
    )
    .await;
    let mut role_runs = Vec::new();
    for _ in 0..400 {
        role_runs = store
            .list_role_runs("project_0001", "issue_0001", "coding_attempt_0001")
            .expect("role runs");
        if !role_runs.is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    assert!(
        !role_runs.is_empty(),
        "重开后 coder 角色运行必须重新落地（runner 真正续跑）"
    );

    ws.close(None).await.expect("close ws");
    server.abort();
}

/// F-44（group 面，授权实施）：中止终态经 `restart_coding` 复位 resume target——
/// 首个非 Completed unit 复位为 active、指针对齐、已 Completed unit 不被复位，
/// 随后由既有 runner 阶段链续跑（阶段门 + provider 角色运行）。
/// 形态对齐现场 attempt e4a4（unit1 已完成 + 末 unit 中止时被归一为 Skipped）。
#[tokio::test]
async fn aborted_group_attempt_restart_coding_restores_resume_target() {
    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let app = app_with_group_full_chain_attempt(root.path());
    crate::seed_coding_attempt_running(&store, "project_0001", "issue_0001", "coding_attempt_0001");
    store
        .update_coding_unit_status(
            "project_0001",
            "issue_0001",
            "coding_attempt_0001",
            "coding_unit_0001",
            CodingExecutionUnitStatus::Completed,
            None,
        )
        .expect("complete unit 1");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            "coding_attempt_0001",
            CodingAttemptStatus::Aborted,
        )
        .expect("abort group attempt");

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    let url = format!("ws://{addr}/ws/coding-attempts/coding_attempt_0001");
    let (mut ws, _) = connect_async(url).await.expect("connect ws");
    let _initial = recv_json(&mut ws).await;
    send_json(&mut ws, &CodingWsInMessage::RestartCoding).await;

    let mut saw_reopened = false;
    let mut saw_stage_gate = false;
    for _ in 0..50 {
        match recv_json(&mut ws).await {
            CodingWsOutMessage::CodingProtocolError { code, message } => {
                panic!("group restart_coding must not be rejected: {code}: {message}");
            }
            CodingWsOutMessage::CodingSessionState { status, .. } => {
                if status == CodingAttemptStatus::Running {
                    saw_reopened = true;
                    // 立即取 durable 状态（runner 后续推进/失败不再影响这一步的断言）。
                    let reopened = store
                        .get_attempt("project_0001", "issue_0001", "coding_attempt_0001")
                        .expect("attempt after restart");
                    assert_eq!(reopened.status, CodingAttemptStatus::Running);
                    assert_eq!(reopened.completed_at, None, "重开必须清除终态时间戳");
                    let units = store
                        .list_coding_units("project_0001", "issue_0001", "coding_attempt_0001")
                        .expect("units");
                    let completed = units
                        .iter()
                        .find(|unit| unit.id == "coding_unit_0001")
                        .expect("unit 1");
                    let restored = units
                        .iter()
                        .find(|unit| unit.id == "coding_unit_0002")
                        .expect("unit 2");
                    assert_eq!(
                        completed.status,
                        CodingExecutionUnitStatus::Completed,
                        "已完成的 unit 不得被复位"
                    );
                    assert_eq!(
                        restored.status,
                        CodingExecutionUnitStatus::Running,
                        "中止归一的 Skipped unit 必须被复位为 resume target"
                    );
                    assert_eq!(restored.completed_at, None, "复位必须清除 unit 终态时间戳");
                    assert_eq!(
                        reopened.active_unit_id.as_deref(),
                        Some(restored.id.as_str()),
                        "active_unit_id 必须对齐 resume target"
                    );
                    assert_eq!(
                        reopened.current_work_item_id.as_deref(),
                        Some(restored.logical_work_item_id.as_str()),
                        "current_work_item_id 必须对齐 resume target"
                    );
                }
            }
            CodingWsOutMessage::CodingGateRequired { gate }
                if gate.kind == CodingGateKind::StageGate
                    && gate.stage == Some(CodingExecutionStage::Coding) =>
            {
                saw_stage_gate = true;
                break;
            }
            _ => {}
        }
    }
    assert!(saw_reopened, "group 重开必须回发 Running 会话快照");
    assert!(
        saw_stage_gate,
        "group 重开后 runner 既有阶段链必须推进到 Coding 阶段门"
    );

    // provider：显式确认阶段门后 coder 角色运行必须重新落地。
    send_json(
        &mut ws,
        &CodingWsInMessage::StageGateConfirm {
            stage: CodingExecutionStage::Coding,
        },
    )
    .await;
    let mut role_runs = Vec::new();
    for _ in 0..400 {
        role_runs = store
            .list_role_runs("project_0001", "issue_0001", "coding_attempt_0001")
            .expect("role runs");
        if !role_runs.is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    assert!(
        !role_runs.is_empty(),
        "group 重开后 coder 角色运行必须重新落地（runner 真正续跑）"
    );

    ws.close(None).await.expect("close ws");
    server.abort();
}

/// F-44 fix1（P1）：group 继任选择器必须把中止归一出（`group_terminal`）的
/// `Skipped` remainder 纳入候选。否则「中止在第 k<n 个 unit」的主流形态下
/// unit_{k+1..n} 被永久放弃、final confirm 永不满足、而非终态又拒 `RestartCoding`
/// ——死胡同循环。本用例直接驱动选择器：unit1 已完成 + unit2 为 Skipped 形态，
/// 断言 unit2 被接续复活为 active（依赖语义与 Pending 候选同构）。
#[tokio::test]
async fn group_remainder_selector_resumes_skipped_successor_unit() {
    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let _app = app_with_group_full_chain_attempt(root.path());
    crate::seed_coding_attempt_running(&store, "project_0001", "issue_0001", "coding_attempt_0001");
    store
        .update_coding_unit_status(
            "project_0001",
            "issue_0001",
            "coding_attempt_0001",
            "coding_unit_0001",
            CodingExecutionUnitStatus::Completed,
            None,
        )
        .expect("complete unit 1");
    store
        .update_coding_unit_status(
            "project_0001",
            "issue_0001",
            "coding_attempt_0001",
            "coding_unit_0002",
            CodingExecutionUnitStatus::Skipped,
            None,
        )
        .expect("skip unit 2 (abort normalization shape)");
    let attempt = store
        .get_attempt("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("attempt");

    let (event_tx, _event_rx) = tokio::sync::mpsc::channel(8);
    let engine = cadence_aria::product::coding_workspace_engine::CodingWorkspaceEngine::new(
        store.clone(),
        cadence_aria::product::git_workspace_service::GitWorkspaceService::new(),
        event_tx,
    );
    let advanced = engine
        .advance_to_next_group_unit(&attempt)
        .await
        .expect("advance to next group unit");
    assert_eq!(
        advanced.active_unit_id.as_deref(),
        Some("coding_unit_0002"),
        "Skipped remainder 必须被继任选择器接续复活"
    );
    let units = store
        .list_coding_units("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("units");
    assert_eq!(
        units
            .iter()
            .find(|unit| unit.id == "coding_unit_0002")
            .expect("unit 2")
            .status,
        CodingExecutionUnitStatus::Running
    );
}
