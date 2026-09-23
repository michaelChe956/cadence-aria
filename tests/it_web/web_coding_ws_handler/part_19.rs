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

/// F-44（fail-closed 面）：group attempt 中止后 unit 全为 Skipped（无 active
/// unit、指针为空、非全 Completed），重开后不存在合法 active/resume target——
/// 必须**拒绝重开**且不改写 durable 状态，而不是「先重开、再让 runner 死于
/// coding_group_attempt_incomplete 并被推进为人工恢复态」。
#[tokio::test]
async fn aborted_group_attempt_restart_coding_is_refused_without_state_write() {
    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let app = app_with_group_full_chain_attempt(root.path());
    crate::seed_coding_attempt_running(&store, "project_0001", "issue_0001", "coding_attempt_0001");
    let aborted = store
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

    match recv_json(&mut ws).await {
        CodingWsOutMessage::CodingProtocolError { code, message } => {
            assert_eq!(code, "coding_restart_failed");
            assert!(
                message.contains("attempt_restart_no_resume_target"),
                "拒绝必须携带稳定原因码，got: {message}"
            );
        }
        other => panic!("expected fail-closed restart rejection, got {other:?}"),
    }
    let after = store
        .get_attempt("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("attempt after refused restart");
    assert_eq!(
        after.status,
        CodingAttemptStatus::Aborted,
        "被拒的重开必须保持原终态（不得先重开再崩成人工恢复态）"
    );
    assert_eq!(after.version, aborted.version, "被拒的重开不得推进版本");
    assert!(
        store
            .list_role_runs("project_0001", "issue_0001", "coding_attempt_0001")
            .expect("role runs")
            .is_empty(),
        "被拒的重开不得触发 provider 角色运行"
    );

    ws.close(None).await.expect("close ws");
    server.abort();
}
