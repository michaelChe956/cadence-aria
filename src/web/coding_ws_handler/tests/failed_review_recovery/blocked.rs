use super::*;

#[tokio::test]
async fn blocked_provider_interrupted_review_retry_enters_the_same_recovery_journal() {
    let fixture = provider_interrupted_review_fixture(CodingAttemptScope::WorkItemGroup).await;
    let gate = fixture
        .dirty_gate
        .as_ref()
        .expect("provider interrupted gate");
    let recovery = recoverable_failed_code_review(&fixture.store, &fixture.attempt)
        .expect("inspect blocked recovery")
        .expect("blocked review must be recoverable");

    assert_eq!(recovery.gate_id, gate.gate_id);
    assert_eq!(recovery.failed_node_id, support::FAILED_NODE_ID);
    assert_eq!(recovery.stale_role_run_id, fixture.stale_role_run_id);

    let (event_tx, _event_rx) = mpsc::channel(8);
    let engine =
        CodingWorkspaceEngine::new(fixture.store.clone(), GitWorkspaceService::new(), event_tx);
    let updated = engine
        .recover_failed_code_review_for_attempt(&fixture.attempt, &gate.gate_id)
        .await
        .expect("recover blocked provider interruption");

    assert_eq!(updated.status, CodingAttemptStatus::Running);
    assert_eq!(updated.stage, CodingExecutionStage::CodeReview);
    assert_eq!(updated.completed_at, None);
    let journal = fixture
        .store
        .get_failed_code_review_recovery_journal(
            &updated.project_id,
            &updated.issue_id,
            &updated.id,
        )
        .expect("load blocked recovery journal")
        .expect("blocked recovery journal");
    assert_eq!(journal.expected_gate_id, gate.gate_id);
    assert_eq!(journal.expected_failed_node_id, support::FAILED_NODE_ID);
    assert_eq!(
        journal.expected_stale_role_run_id,
        fixture.stale_role_run_id
    );
    assert_eq!(journal.phase, FailedCodeReviewRecoveryPhase::GateResolved);

    let runs = fixture
        .store
        .list_role_runs(&updated.project_id, &updated.issue_id, &updated.id)
        .expect("role runs after blocked recovery");
    assert_eq!(runs.len(), 2);
    let stale = runs
        .iter()
        .find(|run| run.id == fixture.stale_role_run_id)
        .expect("failed reviewer run");
    assert_eq!(stale.status, CodingRoleRunStatus::Failed);
    let retry = runs
        .iter()
        .find(|run| run.trigger == CodingRoleRunTrigger::ManualRetry)
        .expect("single retry reviewer run");
    assert_eq!(
        retry.supersedes_run_id.as_deref(),
        Some(fixture.stale_role_run_id.as_str())
    );
    // Bug2-A(Task 3 Step 1):专用恢复链关门必须落完整 resolved 事实——
    // 归档含 action=retry_review 且 resolved_at 非空(现状 action=null 应红);
    // retry run 首写即携带 gate_origin{gate_id,generation,action_id}。
    let archived: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(
            fixture
                .store
                .blocked_gates_root(&updated.project_id, &updated.issue_id, &updated.id)
                .join("resolved")
                .join(format!("{}.json", gate.gate_id)),
        )
        .expect("archived blocked gate record"),
    )
    .expect("archived blocked gate json");
    assert_eq!(
        archived["resolved_action"],
        serde_json::json!("retry_review")
    );
    assert!(
        archived["resolved_at"]
            .as_str()
            .is_some_and(|at| !at.is_empty()),
        "resolved_at must be persisted on first close: {archived}"
    );
    let retry_run_json: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(
            fixture
                .store
                .role_runs_root(&updated.project_id, &updated.issue_id, &updated.id)
                .join(format!("{}.json", retry.id)),
        )
        .expect("retry role run record"),
    )
    .expect("retry role run json");
    assert_eq!(
        retry_run_json["gate_origin"],
        serde_json::json!({
            "gate_id": gate.gate_id,
            "generation": 0,
            "action_id": "retry_review",
        }),
        "dedicated retry run must first-write its gate origin"
    );
}

#[tokio::test]
async fn blocked_gate_closure_replay_keeps_first_close_and_conflicts_on_other_action() {
    let fixture = provider_interrupted_review_fixture(CodingAttemptScope::WorkItemGroup).await;
    let gate = fixture
        .dirty_gate
        .as_ref()
        .expect("provider interrupted gate");
    let (event_tx, _event_rx) = mpsc::channel(8);
    let engine =
        CodingWorkspaceEngine::new(fixture.store.clone(), GitWorkspaceService::new(), event_tx);
    engine
        .recover_failed_code_review_for_attempt(&fixture.attempt, &gate.gate_id)
        .await
        .expect("first recovery closes the gate");
    engine
        .recover_failed_code_review_for_attempt(&fixture.attempt, &gate.gate_id)
        .await
        .expect("reconnect replay converges");

    let runs = fixture
        .store
        .list_role_runs(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .expect("role runs after replay");
    assert_eq!(runs.len(), 2, "single retry run after replay: {runs:?}");
    assert_eq!(
        runs.iter()
            .filter(|run| run.trigger == CodingRoleRunTrigger::ManualRetry)
            .count(),
        1
    );

    let archive_path = fixture
        .store
        .blocked_gates_root(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .join("resolved")
        .join(format!("{}.json", gate.gate_id));
    let first_close: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&archive_path).expect("archived gate"))
            .expect("archived json");
    let first_resolved_at = first_close["resolved_at"]
        .as_str()
        .expect("first-close resolved_at")
        .to_string();

    // 同代次同动作重放:返回首关结果,首关时间不变。
    fixture
        .store
        .resolve_blocked_gate_with_action(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
            &gate.gate_id,
            Some("retry_review"),
        )
        .expect("same-action replay is idempotent");
    let replayed: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(&archive_path).expect("archived gate after replay"),
    )
    .expect("archived json after replay");
    assert_eq!(
        replayed["resolved_at"],
        serde_json::json!(first_resolved_at)
    );

    // 异动作冲突:不覆盖首关审计。
    let conflict = fixture
        .store
        .resolve_blocked_gate_with_action(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
            &gate.gate_id,
            Some("abort"),
        )
        .expect_err("different action on a closed gate must conflict");
    assert!(
        conflict.to_string().contains("product_store_conflict"),
        "{conflict}"
    );
    let after_conflict: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(&archive_path).expect("archived gate after conflict"),
    )
    .expect("archived json after conflict");
    assert_eq!(
        after_conflict["resolved_action"],
        serde_json::json!("retry_review")
    );
    assert_eq!(
        after_conflict["resolved_at"],
        serde_json::json!(first_resolved_at)
    );
}

#[tokio::test]
async fn blocked_gate_reopen_invalidates_stale_origin_generation() {
    let fixture = provider_interrupted_review_fixture(CodingAttemptScope::WorkItemGroup).await;
    let gate = fixture
        .dirty_gate
        .as_ref()
        .expect("provider interrupted gate");
    let (event_tx, _event_rx) = mpsc::channel(8);
    let engine =
        CodingWorkspaceEngine::new(fixture.store.clone(), GitWorkspaceService::new(), event_tx);
    engine
        .recover_failed_code_review_for_attempt(&fixture.attempt, &gate.gate_id)
        .await
        .expect("close gate via dedicated chain");

    let gates_root = fixture.store.blocked_gates_root(
        &fixture.attempt.project_id,
        &fixture.attempt.issue_id,
        &fixture.attempt.id,
    );
    let root_path = gates_root.join(format!("{}.json", gate.gate_id));
    let archive_path = gates_root
        .join("resolved")
        .join(format!("{}.json", gate.gate_id));
    assert!(
        !root_path.exists() && archive_path.exists(),
        "gate closed and archived before reopen"
    );

    // 重开:递增代次+清空关闭字段(旧 origin 因代次失配全部失效)。
    fixture
        .store
        .reopen_failed_code_review_gate_for_plan_repair(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
            &gate.gate_id,
        )
        .expect("reopen for plan repair");
    let reopened: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&root_path).expect("reopened root record"))
            .expect("reopened json");
    assert_eq!(reopened["status"], serde_json::json!("open"));
    assert_eq!(
        reopened["resolution_generation"],
        serde_json::json!(1),
        "reopen must bump the resolution generation: {reopened}"
    );
    assert_eq!(reopened["resolved_action"], serde_json::Value::Null);
    assert_eq!(reopened["resolved_at"], serde_json::Value::Null);
    assert!(
        fixture
            .store
            .list_open_blocked_gates(
                &fixture.attempt.project_id,
                &fixture.attempt.issue_id,
                &fixture.attempt.id
            )
            .expect("open gates after reopen")
            .iter()
            .any(|open| open.gate_id == gate.gate_id)
    );
    assert!(!archive_path.exists(), "archive removed on reopen");

    // 旧 origin(代次 0)迟到终态:代次不匹配,不关重开后的新门。
    let stale_origin = crate::product::coding_models::GateOrigin {
        gate_id: gate.gate_id.clone(),
        generation: 0,
        action_id: "retry_review".to_string(),
    };
    assert_eq!(
        fixture
            .store
            .resolve_blocked_gate_for_origin(
                &fixture.attempt.project_id,
                &fixture.attempt.issue_id,
                &fixture.attempt.id,
                &stale_origin,
            )
            .expect("stale-origin close must be a safe no-op"),
        None,
        "stale generation must not close the reopened gate"
    );
    assert!(
        root_path.exists(),
        "reopened gate survives stale origin close"
    );

    // 当前代次 origin 才能关;首关事实完整。
    let current_origin = crate::product::coding_models::GateOrigin {
        generation: 1,
        ..stale_origin
    };
    let outcome = fixture
        .store
        .resolve_blocked_gate_for_origin(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
            &current_origin,
        )
        .expect("current-origin close")
        .expect("current origin must close the reopened gate");
    assert_eq!(outcome.resolved_action.as_deref(), Some("retry_review"));
    assert!(
        outcome
            .resolved_at
            .as_deref()
            .is_some_and(|at| !at.is_empty()),
        "first-close facts must be complete"
    );
}

#[tokio::test]
async fn ordinary_review_retry_run_first_writes_gate_origin() {
    let fixture = provider_interrupted_review_fixture(CodingAttemptScope::WorkItemGroup).await;
    let gate = fixture
        .store
        .create_blocked_gate(
            &fixture.attempt,
            CreateBlockedGateInput {
                attempt_id: fixture.attempt.id.clone(),
                stage: CodingExecutionStage::CodeReview,
                node_id: None,
                role: Some(CodingProviderRole::CodeReviewer),
                title: "代码审查被阻塞".to_string(),
                description: "需人工重试评审".to_string(),
                reason_code: Some("code_review_blocked".to_string()),
                evidence_refs: Vec::new(),
                raw_provider_output_ref: None,
                available_actions: vec![crate::product::coding_models::CodingGateAction {
                    action_id: "retry_review".to_string(),
                    label: "重试评审".to_string(),
                    action_type: crate::product::coding_models::CodingGateActionType::RetryReview,
                }],
            },
        )
        .expect("ordinary blocked gate");
    let (event_tx, _event_rx) = mpsc::channel(8);
    let engine =
        CodingWorkspaceEngine::new(fixture.store.clone(), GitWorkspaceService::new(), event_tx);
    let updated = engine
        .handle_blocked_gate_response(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
            &gate.gate_id,
            "retry_review",
            None,
        )
        .await
        .expect("ordinary retry");
    assert_eq!(updated.status, CodingAttemptStatus::Running);

    let runs = fixture
        .store
        .list_role_runs(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .expect("role runs after ordinary retry");
    let retry = runs
        .iter()
        .find(|run| run.trigger == CodingRoleRunTrigger::RetryReview)
        .expect("ordinary retry run");
    let retry_run_json: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(
            fixture
                .store
                .role_runs_root(
                    &fixture.attempt.project_id,
                    &fixture.attempt.issue_id,
                    &fixture.attempt.id,
                )
                .join(format!("{}.json", retry.id)),
        )
        .expect("retry role run record"),
    )
    .expect("retry role run json");
    assert_eq!(
        retry_run_json["gate_origin"],
        serde_json::json!({
            "gate_id": gate.gate_id,
            "generation": 0,
            "action_id": "retry_review",
        }),
        "ordinary gate-retry run must first-write its gate origin"
    );

    // 门按动作收口:归档含 action 与 resolved_at。
    let archived: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(
            fixture
                .store
                .blocked_gates_root(
                    &fixture.attempt.project_id,
                    &fixture.attempt.issue_id,
                    &fixture.attempt.id,
                )
                .join("resolved")
                .join(format!("{}.json", gate.gate_id)),
        )
        .expect("archived ordinary gate"),
    )
    .expect("archived ordinary gate json");
    assert_eq!(
        archived["resolved_action"],
        serde_json::json!("retry_review")
    );
    assert!(
        archived["resolved_at"]
            .as_str()
            .is_some_and(|at| !at.is_empty())
    );
}

#[tokio::test]
async fn interrupted_gate_rejects_non_retry_actions_without_closing() {
    let fixture = provider_interrupted_review_fixture(CodingAttemptScope::WorkItemGroup).await;
    let gate = fixture
        .dirty_gate
        .as_ref()
        .expect("provider interrupted gate");
    let runs_before = fixture
        .store
        .list_role_runs(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .expect("role runs before rejected action");
    let (event_tx, _event_rx) = mpsc::channel(8);
    let engine =
        CodingWorkspaceEngine::new(fixture.store.clone(), GitWorkspaceService::new(), event_tx);

    let error = engine
        .handle_blocked_gate_response(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
            &gate.gate_id,
            "provide_context",
            None,
        )
        .await
        .expect_err("non-retry actions are not allowed on interrupted gates");
    assert!(
        error
            .to_string()
            .contains("coding_failed_review_recovery_action_not_allowed"),
        "{error}"
    );

    // 非法动作零关闭零新增:门仍 open、run 集合不变、无归档。
    assert!(
        fixture
            .store
            .list_open_blocked_gates(
                &fixture.attempt.project_id,
                &fixture.attempt.issue_id,
                &fixture.attempt.id
            )
            .expect("open gates after rejected action")
            .iter()
            .any(|open| open.gate_id == gate.gate_id)
    );
    assert_eq!(
        fixture
            .store
            .list_role_runs(
                &fixture.attempt.project_id,
                &fixture.attempt.issue_id,
                &fixture.attempt.id
            )
            .expect("role runs after rejected action"),
        runs_before
    );
    assert!(
        !fixture
            .store
            .blocked_gates_root(
                &fixture.attempt.project_id,
                &fixture.attempt.issue_id,
                &fixture.attempt.id
            )
            .join("resolved")
            .join(format!("{}.json", gate.gate_id))
            .exists()
    );
}

#[tokio::test]
async fn blocked_provider_interrupted_retry_cannot_use_the_ordinary_gate_path() {
    let fixture = provider_interrupted_review_fixture(CodingAttemptScope::WorkItemGroup).await;
    let gate = fixture
        .dirty_gate
        .as_ref()
        .expect("provider interrupted gate");
    let attempt_before = fixture.attempt.clone();
    let runs_before = fixture
        .store
        .list_role_runs(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .expect("role runs before ordinary retry");
    let (event_tx, _event_rx) = mpsc::channel(8);
    let engine =
        CodingWorkspaceEngine::new(fixture.store.clone(), GitWorkspaceService::new(), event_tx);

    let error = engine
        .handle_blocked_gate_response(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
            &gate.gate_id,
            "retry_review",
            None,
        )
        .await
        .expect_err("ordinary retry path must require a reservation");

    assert!(
        error
            .to_string()
            .contains("coding_failed_review_recovery_requires_reservation"),
        "{error}"
    );
    assert_eq!(
        fixture
            .store
            .get_attempt(
                &fixture.attempt.project_id,
                &fixture.attempt.issue_id,
                &fixture.attempt.id,
            )
            .expect("attempt after ordinary retry rejection"),
        attempt_before
    );
    assert_eq!(
        fixture
            .store
            .list_role_runs(
                &fixture.attempt.project_id,
                &fixture.attempt.issue_id,
                &fixture.attempt.id,
            )
            .expect("role runs after ordinary retry rejection"),
        runs_before
    );
    assert!(
        fixture
            .store
            .get_failed_code_review_recovery_journal(
                &fixture.attempt.project_id,
                &fixture.attempt.issue_id,
                &fixture.attempt.id,
            )
            .expect("journal after ordinary retry rejection")
            .is_none()
    );
}

#[tokio::test]
async fn blocked_provider_interrupted_recovery_prefixes_converge_idempotently() {
    for prefix in [
        RecoveryPrefix::Prepared,
        RecoveryPrefix::AttemptReopened,
        RecoveryPrefix::RetryRunCreated,
        RecoveryPrefix::AttemptRunning,
        RecoveryPrefix::GateResolved,
    ] {
        let fixture = provider_interrupted_review_fixture(CodingAttemptScope::WorkItemGroup).await;
        let recovery = recoverable_failed_code_review(&fixture.store, &fixture.attempt)
            .expect("inspect blocked recovery")
            .expect("blocked recovery identity");
        let mut journal = fixture
            .store
            .prepare_failed_code_review_recovery_journal(
                &fixture.attempt,
                &recovery.gate_id,
                &recovery.failed_node_id,
                &recovery.stale_role_run_id,
            )
            .expect("prepare blocked recovery journal");

        if matches!(
            prefix,
            RecoveryPrefix::AttemptReopened
                | RecoveryPrefix::RetryRunCreated
                | RecoveryPrefix::AttemptRunning
                | RecoveryPrefix::GateResolved
        ) {
            journal = fixture
                .store
                .advance_failed_code_review_recovery_journal(
                    &journal,
                    FailedCodeReviewRecoveryPhase::AttemptReopened,
                    None,
                )
                .expect("persist blocked attempt-reopened prefix");
        }
        if matches!(
            prefix,
            RecoveryPrefix::RetryRunCreated
                | RecoveryPrefix::AttemptRunning
                | RecoveryPrefix::GateResolved
        ) {
            let retry = fixture
                .store
                .ensure_failed_code_review_retry_role_run(&fixture.attempt, &journal)
                .expect("persist blocked retry run prefix");
            journal = fixture
                .store
                .advance_failed_code_review_recovery_journal(
                    &journal,
                    FailedCodeReviewRecoveryPhase::RetryRunCreated,
                    Some(&retry.id),
                )
                .expect("record blocked retry run prefix");
        }
        if matches!(
            prefix,
            RecoveryPrefix::AttemptRunning | RecoveryPrefix::GateResolved
        ) {
            fixture
                .store
                .admit_and_transition_attempt_to_executable(
                    &fixture.attempt.project_id,
                    &fixture.attempt.issue_id,
                    &fixture.attempt.id,
                )
                .expect("persist blocked attempt-running prefix");
            journal = fixture
                .store
                .advance_failed_code_review_recovery_journal(
                    &journal,
                    FailedCodeReviewRecoveryPhase::AttemptRunning,
                    journal.retry_role_run_id.as_deref(),
                )
                .expect("record blocked attempt-running prefix");
        }
        if matches!(prefix, RecoveryPrefix::GateResolved) {
            fixture
                .store
                .resolve_blocked_gate(
                    &fixture.attempt.project_id,
                    &fixture.attempt.issue_id,
                    &fixture.attempt.id,
                    &recovery.gate_id,
                )
                .expect("persist blocked gate-resolved prefix");
            let retry_role_run_id = journal.retry_role_run_id.clone();
            fixture
                .store
                .advance_failed_code_review_recovery_journal(
                    &journal,
                    FailedCodeReviewRecoveryPhase::GateResolved,
                    retry_role_run_id.as_deref(),
                )
                .expect("record blocked gate-resolved prefix");
        }

        let prefixed_attempt = fixture
            .store
            .get_attempt(
                &fixture.attempt.project_id,
                &fixture.attempt.issue_id,
                &fixture.attempt.id,
            )
            .expect("attempt at blocked recovery prefix");
        assert_eq!(
            crate::web::coding_ws_handler::socket::unfinished_failed_code_review_recovery_message_allowed(
                &fixture.store,
                &prefixed_attempt,
                &CodingWsInMessage::GateResponse {
                    gate_id: recovery.gate_id.clone(),
                    action_id: "retry_review".to_string(),
                    extra_context: None,
                },
            ),
            Some(true),
            "{prefix:?}: exact retry must remain allowed"
        );
        assert_eq!(
            crate::web::coding_ws_handler::socket::unfinished_failed_code_review_recovery_message_allowed(
                &fixture.store,
                &prefixed_attempt,
                &CodingWsInMessage::AbortAttempt,
            ),
            Some(false),
            "{prefix:?}: abort must not bypass unfinished recovery"
        );

        let (event_tx, _event_rx) = mpsc::channel(8);
        let engine =
            CodingWorkspaceEngine::new(fixture.store.clone(), GitWorkspaceService::new(), event_tx);
        let first = engine
            .recover_failed_code_review_for_attempt(&fixture.attempt, &recovery.gate_id)
            .await
            .unwrap_or_else(|error| panic!("{prefix:?}: first recovery: {error}"));
        let second = engine
            .recover_failed_code_review_for_attempt(&fixture.attempt, &recovery.gate_id)
            .await
            .unwrap_or_else(|error| panic!("{prefix:?}: second recovery: {error}"));

        assert_eq!(first.id, fixture.attempt.id, "{prefix:?}");
        assert_eq!(second.status, CodingAttemptStatus::Running, "{prefix:?}");
        assert_eq!(second.stage, CodingExecutionStage::CodeReview, "{prefix:?}");
        assert_eq!(second.completed_at, None, "{prefix:?}");
        let runs = fixture
            .store
            .list_role_runs(
                &fixture.attempt.project_id,
                &fixture.attempt.issue_id,
                &fixture.attempt.id,
            )
            .expect("role runs after blocked prefix convergence");
        assert_eq!(runs.len(), 2, "{prefix:?}: {runs:?}");
        assert_eq!(
            runs.iter()
                .filter(|run| run.trigger == CodingRoleRunTrigger::ManualRetry)
                .count(),
            1,
            "{prefix:?}"
        );
        let stale = runs
            .iter()
            .find(|run| run.id == fixture.stale_role_run_id)
            .expect("failed reviewer run after convergence");
        assert_eq!(stale.status, CodingRoleRunStatus::Failed, "{prefix:?}");
    }
}

// ---------------------------------------------------------------------------
// T19:人工重试与 reviewer 窗口互斥(锁序:窗口租约先于恢复仲裁)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn manual_retry_fail_closed_while_reviewer_window_holds_the_tree() {
    let fixture = provider_interrupted_review_fixture(CodingAttemptScope::WorkItem).await;
    let gate = fixture
        .dirty_gate
        .as_ref()
        .expect("provider interrupted gate");
    // T16 baseline 捕获需要至少一个 commit(fixture 只 init 未提交)。
    for args in [
        vec!["add", "."],
        vec![
            "-c",
            "user.email=t19@aria.invalid",
            "-c",
            "user.name=t19",
            "commit",
            "-qm",
            "t19 base",
        ],
    ] {
        let output = std::process::Command::new("git")
            .args(&args)
            .current_dir(
                fixture
                    .attempt
                    .worktree_path
                    .as_ref()
                    .expect("worktree path"),
            )
            .output()
            .expect("git fixture commit");
        assert!(output.status.success(), "git {args:?} failed: {output:?}");
    }
    let worktree = fixture
        .attempt
        .worktree_path
        .clone()
        .expect("worktree path");
    let aria_root = fixture._tmp.path().join(".aria");

    // T17 API 直连开窗(崩溃/停等残留窗口形态:Running 持锁)。
    let window_store = crate::product::reviewer_window_store::ReviewerWindowStore::new(
        aria_root.join("reviewer-windows"),
    );
    let snapshot_store = crate::product::reviewer_snapshot::ReviewerSnapshotStore::new(
        aria_root.join("reviewer-snapshots"),
    );
    let scope_private = fixture._tmp.path().join("t19-manual-private");
    std::fs::create_dir_all(&scope_private).expect("scope private");
    let scope_key = crate::cross_cutting::provider_resources::ResourceScopeKey {
        scope_id: "t19-manual-retry".to_string(),
        run_id: "run-t19-manual".to_string(),
        boot_identity: crate::cross_cutting::provider_resources::new_boot_identity(),
    };
    let scope =
        crate::cross_cutting::provider_resources::ProviderRunResourceScope::open_for_recovery(
            &scope_key,
            Some("t19-manual-window"),
            &scope_private,
            &fixture._tmp.path().join("t19-scope-store"),
            None,
        )
        .expect("open scope");
    let targets: [&crate::cross_cutting::provider_resources::ProviderRunResourceScope; 1] =
        [&scope];
    let handle = crate::product::reviewer_window::begin_review_window(
        &window_store,
        &snapshot_store,
        &crate::product::reviewer_window::ReviewerWindowBeginRequest {
            window_id: "t19-manual-window",
            attempt_id: &fixture.attempt.id,
            run_id: "run-t19-manual",
            provider: "claude_code",
            worktree: &worktree,
            quiesce_targets: &targets,
            platform_write_paths: Vec::new(),
            budget: crate::product::reviewer_snapshot::SnapshotBudget::generous(),
            drain_timeout: std::time::Duration::from_secs(5),
        },
    )
    .await
    .expect("open stale reviewer window");

    let (event_tx, _event_rx) = mpsc::channel(8);
    let engine =
        CodingWorkspaceEngine::new(fixture.store.clone(), GitWorkspaceService::new(), event_tx);
    let error = engine
        .recover_failed_code_review_for_attempt(&fixture.attempt, &gate.gate_id)
        .await
        .expect_err("manual retry must fail closed while window holds the tree");
    assert!(
        error.to_string().contains("reviewer_window"),
        "unexpected error: {error}"
    );
    // 恢复 journal 不得被推进(窗口先收口)。
    assert!(
        fixture
            .store
            .get_failed_code_review_recovery_journal(
                &fixture.attempt.project_id,
                &fixture.attempt.issue_id,
                &fixture.attempt.id
            )
            .expect("journal read")
            .is_none(),
        "recovery arbitration must not start behind an open window"
    );

    // 干净回收释放后人工重试照常可用。
    engine
        .recycle_reviewer_window(&handle, "t19 teardown")
        .await
        .expect("recycle releases the tree");
    let updated = engine
        .recover_failed_code_review_for_attempt(&fixture.attempt, &gate.gate_id)
        .await
        .expect("manual retry resumes after window resolution");
    assert_eq!(updated.status, CodingAttemptStatus::Running);
    assert_eq!(updated.stage, CodingExecutionStage::CodeReview);
}
