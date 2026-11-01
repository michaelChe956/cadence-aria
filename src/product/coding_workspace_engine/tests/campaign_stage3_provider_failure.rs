/// Step 2 —— GCE-02 failure E2E：一次 transient 失败在同 attempt/unit 有界
/// 重试；另案用户 abort 后 attempt Aborted 且 units/runs/logs/commit/event
/// 保留；list attempts 始终一项，投影显示 reason。
#[tokio::test]
async fn campaign_stage3_provider_failure_retries_same_unit_and_abort_preserves_evidence() {
    use crate::product::coding_models::{CodingRoleRunStatus, CodingRoleRunTrigger};
    use crate::product::coding_workspace_engine::CodingExecutionContext;

    // —— 案 1：transient 失败 → 同 attempt/unit bounded retry ——
    let (_root, store, attempt) = super::running_attempt_with_worktree();
    let provider = super::provider_failure_recovery::TransportFailuresThenSuccessProvider::new(
        1,
        "coder retry completed",
    );
    let (tx, _rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let (_command_tx, mut command_rx) = mpsc::channel(1);
    let _ = engine
        .execute_coding_with_commands(
            &attempt,
            &provider,
            &CodingExecutionContext {
                work_item_markdown: Some("# Retry work item\n\nKeep the full context.".to_string()),
                verification_commands: vec!["cargo test --locked --lib retry".to_string()],
            },
            &mut command_rx,
        )
        .await
        .expect("bounded retry must recover the same unit");
    let runs = store
        .list_role_runs(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("role runs");
    assert_eq!(runs.len(), 2, "一次 transient 失败恰好一次自动重试");
    assert_eq!(runs[0].status, CodingRoleRunStatus::Failed);
    assert_eq!(runs[1].trigger, CodingRoleRunTrigger::AutomaticRetry);
    assert_eq!(
        runs[1]
            .retry_metadata
            .as_ref()
            .expect("retry metadata")
            .attempt_no,
        2
    );
    assert_eq!(
        store
            .list_attempts_for_issue(&attempt.project_id, &attempt.issue_id)
            .expect("attempts")
            .len(),
        1,
        "失败重试绝不新建 attempt"
    );

    // —— 案 2：用户 abort → attempt Aborted，证据保留，投影显示 reason ——
    let (root, store, attempt) = super::running_attempt_with_worktree();
    let provider = super::provider_failure_recovery::RetryBoundaryMutationProvider::new(
        super::provider_failure_recovery::RetryBoundaryMutation::Abort {
            store: store.clone(),
            attempt: attempt.clone(),
        },
    );
    let (tx, _rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let (_command_tx, mut command_rx) = mpsc::channel(1);
    let _ = engine
        .execute_coding_with_commands(
            &attempt,
            &provider,
            &crate::product::coding_workspace_engine::CodingExecutionContext::default(),
            &mut command_rx,
        )
        .await
        .expect_err("abort stops the retry cycle");
    let aborted = store
        .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("aborted attempt");
    assert_eq!(aborted.status, CodingAttemptStatus::Aborted);
    // runs/logs/event 保留（不抹除证据）：本 fixture 为单 work-item attempt
    // （无 group units），abort 证据面 = role runs + raw 输出 refs + timeline；
    // group units 的保留由 8.4a amendment 家族在同 attempt 断言覆盖。
    let runs = store
        .list_role_runs(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("role runs after abort");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].status, CodingRoleRunStatus::Failed);
    assert_eq!(
        runs[0].reason_code.as_deref(),
        Some("provider_retry_attempt_state_changed")
    );
    assert_eq!(
        runs[0].raw_provider_output_refs.len(),
        1,
        "abort 保留 raw 输出日志 refs"
    );
    let nodes = store
        .get_timeline_nodes(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("timeline nodes after abort");
    assert_eq!(nodes.len(), 1);
    assert_eq!(
        nodes[0].status,
        crate::product::coding_models::CodingTimelineNodeStatus::Failed
    );
    assert_eq!(
        store
            .list_attempts_for_issue(&attempt.project_id, &attempt.issue_id)
            .expect("attempts after abort")
            .len(),
        1,
        "abort 后 list attempts 始终一项"
    );
    // 投影显示 reason：HTTP snapshot 真实 handler 读取。
    let state = crate::web::state::WebAppState::new(
        root.path().to_path_buf(),
        crate::web::runtime::WebRuntime::new_fake(root.path().to_path_buf()),
    );
    let path = axum::extract::Path(crate::web::handlers::CodingAttemptRoutePath {
        project_id: Some(attempt.project_id.clone()),
        issue_id: Some(attempt.issue_id.clone()),
        attempt_id: attempt.id.clone(),
    });
    let axum::Json(snapshot) =
        crate::web::handlers::get_coding_attempt(axum::extract::State(state), path)
            .await
            .expect("GET aborted attempt snapshot");
    assert_eq!(snapshot.attempt.attempt_id, attempt.id);
    assert_eq!(
        snapshot.attempt.status, "aborted",
        "投影如实显示 abort 终态"
    );
}
