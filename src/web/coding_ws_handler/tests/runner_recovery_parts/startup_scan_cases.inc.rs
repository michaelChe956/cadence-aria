/// 主线：Running + 已持久 ProviderMayHaveStarted claim 的半启动 attempt，
/// 进程「重启」后无任何 WS attach，启动扫描即恢复 runner；幂等不双启、
/// 不新建 attempt。
#[tokio::test]
async fn startup_reconciles_claimed_running_coding_without_attach() {
    let fixture = seed_startup_attempt(
        Some(crate::product::coding_models::CodingStartPhase::ProviderMayHaveStarted),
        true,
    );
    let fresh = fixture.restart_state();
    assert_eq!(fresh.coding_runs.runner_count(&fixture.attempt_key()), 0);
    fixture.reconcile(&fresh).await;
    assert_eq!(
        fresh.coding_runs.runner_count(&fixture.attempt_key()),
        1,
        "startup reconcile must resume the claimed running attempt without attach"
    );
    assert_eq!(
        fixture
            .store()
            .list_attempts_for_issue("project_0001", "issue_0001")
            .expect("list attempts")
            .len(),
        1,
        "recovery must resume the original attempt, not create a new one"
    );
    assert_eq!(fixture.reload().status, CodingAttemptStatus::Running);
    // 幂等：重复扫描不得双启（registry/claim 去重）。
    fixture.reconcile(&fresh).await;
    assert_eq!(fresh.coding_runs.runner_count(&fixture.attempt_key()), 1);
}

/// Claimed 未跨 barrier 的安全窗：同 command/origin 幂等续启（Task 4 单一
/// 恢复方法），不隐式新 command。
#[tokio::test]
async fn startup_reconcile_resumes_claimed_first_start_with_same_command() {
    let fixture = seed_startup_attempt(
        Some(crate::product::coding_models::CodingStartPhase::Claimed),
        false,
    );
    let fresh = fixture.restart_state();
    fixture.reconcile(&fresh).await;
    assert_eq!(
        fresh.coding_runs.runner_count(&fixture.attempt_key()),
        1,
        "claimed-not-crossed-barrier attempt must resume via the same command"
    );
    let reloaded = fixture.reload();
    assert_eq!(
        reloaded
            .start_claim
            .as_ref()
            .expect("claim kept")
            .command_id,
        format!("startup-reconcile-{}", reloaded.id),
        "recovery must replay the durable claim command identity"
    );
    assert_eq!(
        fixture
            .store()
            .list_attempts_for_issue("project_0001", "issue_0001")
            .unwrap()
            .len(),
        1
    );
}

/// Created + ProviderMayHaveStarted + 无可信 ledger：外部副作用不可证明 →
/// durable AwaitingManualRecovery（UI 可诊断），零 runner。
#[tokio::test]
async fn startup_unproven_provider_window_moves_created_attempt_to_manual_recovery() {
    let fixture = seed_startup_attempt(
        Some(crate::product::coding_models::CodingStartPhase::ProviderMayHaveStarted),
        false,
    );
    let fresh = fixture.restart_state();
    fixture.reconcile(&fresh).await;
    assert_eq!(fresh.coding_runs.runner_count(&fixture.attempt_key()), 0);
    let reloaded = fixture.reload();
    assert_eq!(
        reloaded.status,
        CodingAttemptStatus::AwaitingManualRecovery,
        "unproven provider window must fail closed for manual triage"
    );
    assert!(reloaded.manual_recovery_reason.is_some());
    // 人工恢复态二次扫描零动作。
    fixture.reconcile(&fresh).await;
    assert_eq!(fresh.coding_runs.runner_count(&fixture.attempt_key()), 0);
}

/// Created 无 claim 不隐式启动；legacy Running（无 claim、有既有准入证据）
/// 按原规则恢复；Failed/Aborted/Completed 一律不恢复。
#[tokio::test]
async fn startup_reconcile_skips_terminal_and_unclaimed_but_recovers_legacy_running() {
    let idle = seed_startup_attempt(None, false);
    let fresh = idle.restart_state();
    idle.reconcile(&fresh).await;
    assert_eq!(
        fresh.coding_runs.runner_count(&idle.attempt_key()),
        0,
        "created attempt without claim must not auto-start"
    );
    assert_eq!(idle.reload().status, CodingAttemptStatus::Created);

    let legacy = seed_startup_attempt(None, true);
    let legacy_state = legacy.restart_state();
    legacy.reconcile(&legacy_state).await;
    assert_eq!(
        legacy_state.coding_runs.runner_count(&legacy.attempt_key()),
        1,
        "legacy running attempt with durable admission must recover by the original rules"
    );

    for status in [
        CodingAttemptStatus::Failed,
        CodingAttemptStatus::Aborted,
        CodingAttemptStatus::Completed,
    ] {
        let terminal = seed_startup_attempt(None, false);
        let mut attempt = terminal.reload();
        attempt.status = status.clone();
        terminal
            .store()
            .write_coding_attempt_for_test(&attempt)
            .expect("seed terminal attempt");
        let terminal_state = terminal.restart_state();
        terminal.reconcile(&terminal_state).await;
        assert_eq!(
            terminal_state
                .coding_runs
                .runner_count(&terminal.attempt_key()),
            0,
            "{status:?} must not recover"
        );
        assert_eq!(terminal.reload().status, status);
    }
}

/// P2 Task 5 迁移回归修复：Running+ReviewRequest 的死 runner 原由 WS
/// StartCoding 直启路径隐式复活，迁移后唯一人工通道是 attach/启动扫描的
/// 半启动恢复。启动扫描必须覆盖该阶段（review 已完成的 durable 事实，
/// 恢复只续推进不重跑 review）；Running+PrepareContext 为过渡/异常态，
/// 保持 fail-safe 不自动复活。
#[tokio::test]
async fn startup_reconcile_recovers_legacy_running_review_request_stage() {
    let fixture = seed_startup_attempt(None, true);
    fixture
        .store()
        .update_attempt_stage(
            "project_0001",
            "issue_0001",
            &fixture.reload().id,
            CodingExecutionStage::ReviewRequest,
        )
        .expect("seed review request stage");
    let fresh = fixture.restart_state();
    fixture.reconcile(&fresh).await;
    assert_eq!(
        fresh.coding_runs.runner_count(&fixture.attempt_key()),
        1,
        "startup reconcile must resume a running review-request attempt"
    );
    assert_eq!(fixture.reload().stage, CodingExecutionStage::ReviewRequest);

    // 对照：Running+PrepareContext 是过渡/异常态，不自动复活。
    let transitional = seed_startup_attempt(None, false);
    let mut transitional_attempt = transitional.reload();
    transitional_attempt.status = CodingAttemptStatus::Running;
    transitional
        .store()
        .write_coding_attempt_for_test(&transitional_attempt)
        .expect("seed running prepare-context attempt");
    let transitional_state = transitional.restart_state();
    transitional.reconcile(&transitional_state).await;
    assert_eq!(
        transitional_state
            .coding_runs
            .runner_count(&transitional.attempt_key()),
        0,
        "running prepare-context attempt must stay fail-safe for manual triage"
    );
}
