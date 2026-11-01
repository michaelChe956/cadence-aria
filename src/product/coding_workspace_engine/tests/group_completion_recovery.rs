fn group_completion_fixture_at_stage(
    with_dependency: bool,
    dirty: bool,
    stage: CodingExecutionStage,
) -> GroupCompletionFixture {
    let root = tempdir().expect("tempdir");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree");
    init_test_git_repo(&worktree);
    let original_head = git_stdout(&worktree, &["rev-parse", "HEAD"])
        .trim()
        .to_string();
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_group_attempt(CreateGroupCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            current_work_item_id: "work_item_0001".to_string(),
            base_branch: original_head.clone(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: Some(worktree.clone()),
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Codex,
                reviewer: Some(ProviderName::ClaudeCode),
                review_rounds: 1,
                permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
            },
            target_snapshot: None,
            max_auto_rework: 2,
            start_run_policy: crate::product::coding_models::CodingStartRunPolicy::Manual,
        })
        .expect("group attempt");
    seed_group_attempt_fixture(&store, &attempt, true, with_dependency);
    let attempt = store
        .seed_running_attempt_for_test(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("running attempt");
    let attempt = store
        .update_attempt_stage(&attempt.project_id, &attempt.issue_id, &attempt.id, stage)
        .expect("completion stage");
    if dirty {
        fs::write(worktree.join("unit1.txt"), "unit 1 change\n").expect("unit change");
    }
    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    GroupCompletionFixture {
        _root: root,
        worktree,
        store,
        engine,
        attempt,
        original_head,
    }
}

#[derive(Debug, PartialEq, Eq)]
struct GroupCompletionStateSnapshot {
    attempt: CodingExecutionAttempt,
    units: Vec<crate::product::coding_models::CodingExecutionUnit>,
    runs: Vec<Vec<CodingUnitRun>>,
    canonical_handoffs: Vec<HandoffRevision>,
    head: String,
    status: String,
}

fn snapshot_group_completion_state(
    fixture: &GroupCompletionFixture,
) -> GroupCompletionStateSnapshot {
    let attempt = fixture
        .store
        .get_attempt(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .expect("attempt snapshot");
    let units = fixture
        .store
        .list_coding_units(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("unit snapshot");
    let runs = units
        .iter()
        .map(|unit| {
            fixture
                .store
                .list_coding_unit_runs(&attempt, &unit.id)
                .expect("run snapshot")
        })
        .collect();
    let revision_store = WorkItemRevisionStore::new(fixture.store.paths());
    let lineage = revision_store
        .get_plan_lineage(
            &attempt.project_id,
            &attempt.issue_id,
            attempt
                .work_item_group_id
                .as_deref()
                .expect("group plan binding"),
        )
        .expect("lineage snapshot");
    let canonical_handoffs = units
        .iter()
        .filter_map(|unit| {
            unit.latest_handoff_revision_id.as_ref().map(|handoff_id| {
                revision_store
                    .get_handoff_revision(&lineage, &unit.logical_work_item_id, handoff_id)
                    .expect("canonical handoff snapshot")
            })
        })
        .collect();
    GroupCompletionStateSnapshot {
        attempt,
        units,
        runs,
        canonical_handoffs,
        head: git_stdout(&fixture.worktree, &["rev-parse", "HEAD"])
            .trim()
            .to_string(),
        status: git_stdout(&fixture.worktree, &["status", "--porcelain"]),
    }
}

fn lock_group_fixture_with_other_owner(fixture: &GroupCompletionFixture) -> LifecycleStore {
    let lifecycle = LifecycleStore::new(fixture.store.paths());
    lifecycle
        .upsert_issue_shared_worktree(
            crate::product::lifecycle_store::UpsertIssueSharedWorktreeInput {
                project_id: fixture.attempt.project_id.clone(),
                issue_id: fixture.attempt.issue_id.clone(),
                repository_id: "repository_0001".to_string(),
                branch_name: fixture.attempt.branch_name.clone(),
                worktree_path: fixture.worktree.clone(),
                base_branch: fixture.original_head.clone(),
            },
        )
        .expect("shared worktree");
    lifecycle
        .try_acquire_issue_worktree_lock(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            "work_item_0001",
            "coding_attempt_other",
        )
        .expect("conflicting shared worktree owner");
    lifecycle
}

fn cleared_active_recovery_fixture() -> (
    GroupCompletionFixture,
    CodingExecutionAttempt,
    WorkItemPlanLineage,
    HandoffRevision,
) {
    cleared_active_recovery_fixture_at_stage(CodingExecutionStage::ReviewRequest)
}

#[tokio::test]
async fn group_completion_running_owner_conflict_is_zero_write_at_production_entry() {
    let fixture = group_completion_fixture(false, true);
    create_authoritative_active_run(
        &fixture,
        "coding_unit_run_0001",
        1,
        CodingUnitRunStatus::Running,
        None,
        None,
    );
    let lifecycle = lock_group_fixture_with_other_owner(&fixture);
    let before = snapshot_group_completion_state(&fixture);
    let lease_before = lifecycle
        .get_issue_shared_worktree(&fixture.attempt.project_id, &fixture.attempt.issue_id)
        .expect("shared worktree before")
        .expect("shared worktree before");

    let error = fixture
        .engine
        .complete_group_unit_after_code_review(&fixture.attempt)
        .await
        .expect_err("owner conflict must reject the production completion entry");

    assert!(matches!(
        error,
        CodingWorkspaceEngineError::Store(ProductStoreError::Conflict {
            kind: "issue_worktree_lock_owner",
            ..
        })
    ));
    assert_eq!(snapshot_group_completion_state(&fixture), before);
    assert_eq!(
        lifecycle
            .get_issue_shared_worktree(&fixture.attempt.project_id, &fixture.attempt.issue_id)
            .expect("shared worktree after")
            .expect("shared worktree after"),
        lease_before
    );
}

#[tokio::test]
async fn group_completion_retry_owner_conflict_is_zero_write_at_production_entry() {
    let (fixture, partial, _, _) = cleared_active_recovery_fixture();
    let lifecycle = lock_group_fixture_with_other_owner(&fixture);
    let before = snapshot_group_completion_state(&fixture);
    let lease_before = lifecycle
        .get_issue_shared_worktree(&partial.project_id, &partial.issue_id)
        .expect("shared worktree before")
        .expect("shared worktree before");

    let error = fixture
        .engine
        .complete_group_unit_after_code_review(&partial)
        .await
        .expect_err("owner conflict must reject completed retry before next unit starts");

    assert!(matches!(
        error,
        CodingWorkspaceEngineError::Store(ProductStoreError::Conflict {
            kind: "issue_worktree_lock_owner",
            ..
        })
    ));
    assert_eq!(snapshot_group_completion_state(&fixture), before);
    assert_eq!(
        lifecycle
            .get_issue_shared_worktree(&partial.project_id, &partial.issue_id)
            .expect("shared worktree after")
            .expect("shared worktree after"),
        lease_before
    );
}

#[tokio::test]
async fn abort_during_running_group_completion_prewrite_pause_is_stable() {
    let fixture = group_completion_fixture(false, true);
    create_authoritative_active_run(
        &fixture,
        "coding_unit_run_0001",
        1,
        CodingUnitRunStatus::Running,
        None,
        None,
    );
    let (_pause, reached, resume) = register_coding_mutation_test_pause(
        fixture.store.paths().root(),
        CodingMutationTestPoint::GroupCompletionRunning,
    );
    let store = fixture.store.clone();
    let attempt = fixture.attempt.clone();
    let completion = tokio::spawn(async move {
        let (tx, _rx) = mpsc::channel(8);
        CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), tx)
            .complete_group_unit_after_code_review(&attempt)
            .await
    });
    reached.await.expect("running completion prewrite pause");

    fixture
        .engine
        .handle_abort(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .await
        .expect("abort paused running completion");
    let after_abort = snapshot_group_completion_state(&fixture);
    resume.send(()).expect("resume running completion");
    let error = completion
        .await
        .expect("running completion task")
        .expect_err("aborted running completion must fail closed");

    assert!(
        error
            .to_string()
            .contains("group_completion_status_not_ready"),
        "{error}"
    );
    assert_eq!(snapshot_group_completion_state(&fixture), after_abort);
}

#[tokio::test]
async fn abort_during_completed_retry_prewrite_pause_is_stable() {
    let (fixture, partial, _, _) = cleared_active_recovery_fixture();
    let (_pause, reached, resume) = register_coding_mutation_test_pause(
        fixture.store.paths().root(),
        CodingMutationTestPoint::GroupCompletionCompletedRetry,
    );
    let store = fixture.store.clone();
    let retry_attempt = partial.clone();
    let completion = tokio::spawn(async move {
        let (tx, _rx) = mpsc::channel(8);
        CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), tx)
            .complete_group_unit_after_code_review(&retry_attempt)
            .await
    });
    reached.await.expect("completed retry prewrite pause");

    fixture
        .engine
        .handle_abort(&partial.project_id, &partial.issue_id, &partial.id)
        .await
        .expect("abort paused completed retry");
    let after_abort = snapshot_group_completion_state(&fixture);
    resume.send(()).expect("resume completed retry");
    let error = completion
        .await
        .expect("completed retry task")
        .expect_err("aborted completed retry must fail closed");

    assert!(
        error
            .to_string()
            .contains("group_completion_status_not_ready"),
        "{error}"
    );
    assert_eq!(snapshot_group_completion_state(&fixture), after_abort);
}

fn cleared_active_recovery_fixture_at_stage(
    stage: CodingExecutionStage,
) -> (
    GroupCompletionFixture,
    CodingExecutionAttempt,
    WorkItemPlanLineage,
    HandoffRevision,
) {
    let fixture = group_completion_fixture_at_stage(true, false, stage);
    let source_run = create_authoritative_active_run(
        &fixture,
        "coding_unit_run_0001",
        1,
        CodingUnitRunStatus::Completed,
        Some(fixture.original_head.clone()),
        None,
    );
    let active = fixture
        .store
        .get_active_coding_unit(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .expect("active unit lookup")
        .expect("active unit");
    let handoff =
        expected_handoff_revision(&source_run, &fixture.original_head, "2026-07-19T00:00:00Z");
    let revision_store = WorkItemRevisionStore::new(fixture.store.paths());
    let lineage = revision_store
        .get_plan_lineage(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            "work_item_plan_0001",
        )
        .expect("lineage");
    revision_store
        .put_handoff_revision(&lineage, &handoff)
        .expect("canonical handoff");
    fixture
        .store
        .update_coding_unit_completion_commit(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
            &active.id,
            Some(fixture.original_head.clone()),
        )
        .expect("unit completion commit");
    fixture
        .store
        .update_coding_unit_latest_handoff_revision_id(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
            &active.id,
            Some(handoff.id.clone()),
        )
        .expect("canonical pointer");
    fixture
        .store
        .update_coding_unit_status(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
            &active.id,
            CodingExecutionUnitStatus::Completed,
            Some("current unit completed".to_string()),
        )
        .expect("completed unit before next start");
    let partial = fixture
        .store
        .get_attempt(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .expect("partial attempt");
    assert!(partial.active_unit_id.is_none());

    (fixture, partial, lineage, handoff)
}

#[tokio::test]
async fn coding_plan_repair_group_completion_recovers_after_active_pointer_was_cleared() {
    let (fixture, authoritative, lineage, handoff) = cleared_active_recovery_fixture();
    let revision_store = WorkItemRevisionStore::new(fixture.store.paths());
    let mut stale_attempt = authoritative.clone();
    stale_attempt.stage = CodingExecutionStage::PrepareContext;
    assert_eq!(authoritative.stage, CodingExecutionStage::ReviewRequest);

    let updated = fixture
        .engine
        .complete_group_unit_after_code_review(&stale_attempt)
        .await
        .expect("recover cleared active pointer boundary");
    let units = fixture
        .store
        .list_coding_units(&updated.project_id, &updated.issue_id, &updated.id)
        .expect("units");
    let next = units
        .iter()
        .find(|unit| unit.logical_work_item_id == "work_item_0002")
        .expect("next unit");

    assert_eq!(next.status, CodingExecutionUnitStatus::Running);
    assert_eq!(updated.active_unit_id.as_deref(), Some(next.id.as_str()));
    assert_eq!(
        revision_store
            .get_handoff_revision(&lineage, &handoff.logical_work_item_id, &handoff.id)
            .expect("canonical handoff after recovery"),
        handoff
    );
}

#[tokio::test]
async fn coding_plan_repair_group_completion_running_wrong_stages_are_zero_write() {
    for stage in [
        CodingExecutionStage::PrepareContext,
        CodingExecutionStage::WorktreePrepare,
        CodingExecutionStage::Coding,
        CodingExecutionStage::CodeReview,
        CodingExecutionStage::InternalPrReview,
        CodingExecutionStage::FinalConfirm,
    ] {
        let fixture = group_completion_fixture_at_stage(true, true, stage);
        create_authoritative_active_run(
            &fixture,
            "coding_unit_run_0001",
            1,
            CodingUnitRunStatus::Running,
            None,
            None,
        );
        let before = snapshot_group_completion_state(&fixture);

        let error = fixture
            .engine
            .complete_group_unit_after_code_review(&fixture.attempt)
            .await
            .expect_err("wrong running stage must fail closed");

        assert!(
            error
                .to_string()
                .contains("group_completion_stage_not_ready"),
            "{error}"
        );
        assert_eq!(snapshot_group_completion_state(&fixture), before);
        assert!(
            fixture
                .store
                .list_open_blocked_gates(
                    &fixture.attempt.project_id,
                    &fixture.attempt.issue_id,
                    &fixture.attempt.id,
                )
                .expect("blocked gates")
                .is_empty()
        );
    }
}

#[tokio::test]
async fn coding_plan_repair_group_completion_completed_retry_wrong_stages_are_zero_write() {
    for stage in [
        CodingExecutionStage::PrepareContext,
        CodingExecutionStage::WorktreePrepare,
        CodingExecutionStage::Coding,
        CodingExecutionStage::CodeReview,
        CodingExecutionStage::InternalPrReview,
        CodingExecutionStage::FinalConfirm,
    ] {
        let (fixture, stale_attempt, lineage, handoff) =
            cleared_active_recovery_fixture_at_stage(stage);
        let before = snapshot_group_completion_state(&fixture);

        let error = fixture
            .engine
            .complete_group_unit_after_code_review(&stale_attempt)
            .await
            .expect_err("wrong completed retry stage must fail closed");

        assert!(
            error
                .to_string()
                .contains("group_completion_stage_not_ready"),
            "{error}"
        );
        assert_eq!(snapshot_group_completion_state(&fixture), before);
        assert_eq!(
            WorkItemRevisionStore::new(fixture.store.paths())
                .get_handoff_revision(&lineage, &handoff.logical_work_item_id, &handoff.id)
                .expect("canonical handoff after rejection"),
            handoff
        );
        assert!(
            fixture
                .store
                .list_open_blocked_gates(
                    &stale_attempt.project_id,
                    &stale_attempt.issue_id,
                    &stale_attempt.id,
                )
                .expect("blocked gates")
                .is_empty()
        );
    }
}

#[tokio::test]
async fn coding_plan_repair_group_completion_completed_retry_dirty_worktree_reuses_manual_gate() {
    let (fixture, stale_attempt, _, _) = cleared_active_recovery_fixture();
    fixture
        .store
        .update_attempt_stage(
            &stale_attempt.project_id,
            &stale_attempt.issue_id,
            &stale_attempt.id,
            CodingExecutionStage::ReviewRequest,
        )
        .expect("review request stage");
    fs::write(fixture.worktree.join("staged.txt"), "staged\n").expect("staged file");
    run_test_git(&fixture.worktree, &["add", "staged.txt"]);
    fs::write(fixture.worktree.join("README.md"), "unstaged\n").expect("unstaged file");
    fs::write(fixture.worktree.join("untracked.txt"), "untracked\n").expect("untracked file");
    let before = snapshot_group_completion_state(&fixture);

    let first_error = fixture
        .engine
        .complete_group_unit_after_code_review(&stale_attempt)
        .await
        .expect_err("dirty completed retry must create manual gate");
    assert!(
        first_error
            .to_string()
            .contains("shared_worktree_dirty_manual_gate"),
        "{first_error}"
    );
    let first_gates = fixture
        .store
        .list_open_blocked_gates(
            &stale_attempt.project_id,
            &stale_attempt.issue_id,
            &stale_attempt.id,
        )
        .expect("first blocked gates");
    assert_eq!(first_gates.len(), 1);
    let first_gate = &first_gates[0];
    assert_eq!(first_gate.stage, Some(CodingExecutionStage::ReviewRequest));
    assert_eq!(
        first_gate.reason_code.as_deref(),
        Some("shared_worktree_dirty_manual_gate")
    );
    assert_eq!(first_gate.available_actions.len(), 2);
    assert!(
        first_gate
            .available_actions
            .iter()
            .any(|action| { action.action_type == CodingGateActionType::ManualContinue })
    );
    assert!(
        first_gate
            .available_actions
            .iter()
            .any(|action| action.action_type == CodingGateActionType::Abort)
    );
    assert!(
        !first_gate
            .available_actions
            .iter()
            .any(|action| { action.action_type == CodingGateActionType::SendToCoder })
    );
    assert_eq!(snapshot_group_completion_state(&fixture), before);

    let second_error = fixture
        .engine
        .complete_group_unit_after_code_review(&stale_attempt)
        .await
        .expect_err("dirty completed retry must reuse manual gate");
    assert!(
        second_error
            .to_string()
            .contains("shared_worktree_dirty_manual_gate"),
        "{second_error}"
    );
    let second_gates = fixture
        .store
        .list_open_blocked_gates(
            &stale_attempt.project_id,
            &stale_attempt.issue_id,
            &stale_attempt.id,
        )
        .expect("second blocked gates");
    assert_eq!(second_gates.len(), 1);
    assert_eq!(second_gates[0].gate_id, first_gate.gate_id);
    assert_eq!(snapshot_group_completion_state(&fixture), before);
}

#[tokio::test]
async fn coding_plan_repair_group_completion_rejects_noncontiguous_unit_statuses() {
    let (fixture, _, _, _) = cleared_active_recovery_fixture();
    let units = fixture
        .store
        .list_coding_units(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .expect("units");
    let last = units.last().expect("last unit");
    fixture
        .store
        .update_coding_unit_status(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
            &last.id,
            CodingExecutionUnitStatus::Completed,
            Some("invalid noncontiguous completion".to_string()),
        )
        .expect("complete last unit out of order");
    let partial = fixture
        .store
        .get_attempt(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .expect("partial attempt");
    let units_before = fixture
        .store
        .list_coding_units(&partial.project_id, &partial.issue_id, &partial.id)
        .expect("units before");

    fixture
        .engine
        .complete_group_unit_after_code_review(&partial)
        .await
        .expect_err("noncontiguous statuses must fail closed");

    assert_eq!(
        fixture
            .store
            .list_coding_units(&partial.project_id, &partial.issue_id, &partial.id)
            .expect("units after"),
        units_before
    );
}

#[tokio::test]
async fn coding_plan_repair_group_completion_recovers_git_commit_before_store_identity() {
    let fixture = group_completion_fixture(false, true);
    let attempt = fixture
        .store
        .update_attempt_head_commit(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
            Some(fixture.original_head.clone()),
        )
        .expect("persist pre-commit head");
    create_authoritative_active_run(
        &fixture,
        "coding_unit_run_0001",
        1,
        CodingUnitRunStatus::Running,
        None,
        None,
    );
    run_test_git(&fixture.worktree, &["add", "."]);
    run_test_git(
        &fixture.worktree,
        &["commit", "-m", "feat: partial group unit commit"],
    );
    let committed_head = git_stdout(&fixture.worktree, &["rev-parse", "HEAD"])
        .trim()
        .to_string();
    assert_ne!(committed_head, fixture.original_head);

    let updated = fixture
        .engine
        .complete_group_unit_after_code_review(&attempt)
        .await
        .expect("recover git commit before store identity");
    let completed = fixture
        .store
        .list_coding_unit_runs(&updated, "coding_unit_0001")
        .expect("unit runs")
        .into_iter()
        .find(|run| run.id == "coding_unit_run_0001")
        .expect("completed run");

    assert_eq!(
        updated.head_commit.as_deref(),
        Some(committed_head.as_str())
    );
    assert_eq!(
        completed.completion_commit.as_deref(),
        Some(committed_head.as_str())
    );
}

// ---- S2b（N5 步暂停移除）：plan-run first_ready 完成链自动推进 ----

fn seed_plan_coding_first_ready_run(
    fixture: &GroupCompletionFixture,
    unit: &crate::product::coding_models::CodingExecutionUnit,
    phase: crate::product::coding_plan_run::CodingPlanRunPhase,
) {
    let selection = crate::product::coding_plan_run::CodingPlanSelection {
        unit_id: unit.id.clone(),
        logical_work_item_id: unit.logical_work_item_id.clone(),
        attempt_id: fixture.attempt.id.clone(),
        target_repository_id: "repository_0001".to_string(),
        start_command_id:
            crate::product::coding_plan_run::CodingPlanSelection::deterministic_start_command_id(
                &fixture.attempt.project_id,
                &fixture.attempt.issue_id,
                "work_item_plan_0001",
                "cmd_n5_plan_run",
                "repository_0001",
            ),
    };
    let run = crate::product::coding_plan_run::CodingPlanRun {
        project_id: fixture.attempt.project_id.clone(),
        issue_id: fixture.attempt.issue_id.clone(),
        plan_id: "work_item_plan_0001".to_string(),
        command_id: "cmd_n5_plan_run".to_string(),
        bound_plan_revision_id: "plan_revision_n5_fixture".to_string(),
        dependency_graph_revision_id: "dependency_graph_revision_n5_fixture".to_string(),
        target_attempts: Vec::new(),
        mode: crate::product::coding_plan_run::PLAN_CODING_MODE_FIRST_READY_UNIT.to_string(),
        selection: Some(selection),
        phase,
        start_outcome: None,
        pause_reason: None,
        created_at: "2026-10-13T00:00:00Z".to_string(),
        updated_at: "2026-10-13T00:00:00Z".to_string(),
    };
    let runs_dir = fixture
        .store
        .paths()
        .issue_root(&fixture.attempt.project_id, &fixture.attempt.issue_id)
        .join("coding-plan-runs");
    fs::create_dir_all(&runs_dir).expect("coding-plan-runs directory");
    fs::write(
        runs_dir.join("work_item_plan_0001.json"),
        serde_json::to_string(&run).expect("serialize plan run"),
    )
    .expect("persist plan run");
}

fn n5_first_selected_unit(
    fixture: &GroupCompletionFixture,
) -> crate::product::coding_models::CodingExecutionUnit {
    fixture
        .store
        .get_active_coding_unit(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .expect("active unit lookup")
        .expect("selected first unit")
}

fn n5_plan_run_phase(
    fixture: &GroupCompletionFixture,
) -> crate::product::coding_plan_run::CodingPlanRunPhase {
    crate::product::coding_plan_run::CodingPlanRunStore::new(fixture.store.paths())
        .get(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            "work_item_plan_0001",
        )
        .expect("plan run lookup")
        .expect("plan run present")
        .phase
}

/// S2b：同步改写权威 dependency graph edges（unit 声明与 graph 必须一致，
/// 否则选择器先以 SC_GROUP_DEPENDENCY_UNKNOWN 拒绝）。graph revision 经
/// store 不可变；此处作为前置态 seed 直接改写 durable JSON（与 campaign
/// 测试同一 seed 通道）。
fn set_recovery_graph_edges(fixture: &GroupCompletionFixture, edges: Vec<(String, String)>) {
    let revision_store = WorkItemRevisionStore::new(fixture.store.paths());
    let lineage = revision_store
        .get_plan_lineage(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            "work_item_plan_0001",
        )
        .expect("lineage");
    let plan_revision = revision_store
        .get_plan_revision(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            "work_item_plan_0001",
            lineage
                .active_revision_id
                .as_deref()
                .expect("active revision"),
        )
        .expect("plan revision");
    let graph = revision_store
        .get_dependency_graph_revision(&lineage, &plan_revision.dependency_graph_revision_id)
        .expect("dependency graph");
    let graph_path = fixture
        .store
        .paths()
        .issue_root(&fixture.attempt.project_id, &fixture.attempt.issue_id)
        .join("work-item-revisions")
        .join("work_item_plan_0001")
        .join("dependency-graph-revisions")
        .join(format!("{}.json", graph.id));
    let mut graph = graph;
    graph.edges = edges
        .into_iter()
        .map(
            |(from, to)| crate::product::work_item_contract::DependencyContractEdge {
                from,
                to,
                required_contracts: Vec::new(),
            },
        )
        .collect();
    fs::write(
        &graph_path,
        serde_json::to_vec_pretty(&graph).expect("dependency graph JSON encoding"),
    )
    .expect("dependency graph JSON rewrite");
}

/// S2b：直接改写 unit 的声明依赖（与 `set_recovery_graph_edges` 同一
/// seed 通道，保持声明与权威图全等）。
fn set_recovery_unit_dependencies(
    fixture: &GroupCompletionFixture,
    logical_work_item_id: &str,
    dependencies: &[&str],
) {
    let target = fixture
        .store
        .list_coding_units(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .expect("units")
        .into_iter()
        .find(|unit| unit.logical_work_item_id == logical_work_item_id)
        .expect("target unit");
    let path = fixture.store.coding_unit_path(
        &fixture.attempt.project_id,
        &fixture.attempt.issue_id,
        &fixture.attempt.id,
        &target.id,
    );
    let mut unit: crate::product::coding_models::CodingExecutionUnit =
        serde_json::from_slice(&fs::read(&path).expect("coding unit JSON"))
            .expect("coding unit JSON value");
    unit.dependency_logical_work_item_ids = dependencies
        .iter()
        .map(|dependency| (*dependency).to_string())
        .collect();
    fs::write(
        &path,
        serde_json::to_vec_pretty(&unit).expect("coding unit JSON encoding"),
    )
    .expect("coding unit JSON rewrite");
}

/// S2b 红测①：plan-run 首项（WI-001）完成后引擎自动推进就绪的下一项——
/// 完成事实落定后直接进入 SC selector：WI-002 被 start 成 Running（run/
/// active 指针/PrepareContext）；不再写默认步暂停：无 Blocked、无暂停
/// gate、run 相位保持 Started，完成窗口后的 provider 准入不被短路。
#[tokio::test]
async fn plan_coding_s2_completion_auto_advances_next_ready_unit() {
    let fixture = group_completion_fixture(true, false);
    let mut sc_attempt = fixture.attempt.clone();
    sc_attempt.admission_kind = crate::product::coding_models::CodingAdmissionKind::ScAdvance;
    fixture
        .store
        .write_coding_attempt_for_test(&sc_attempt)
        .expect("sc admission kind");
    // 串行链 0001→0002→0003：0001 完成后唯一就绪项是 0002。
    set_recovery_graph_edges(
        &fixture,
        vec![
            ("work_item_0001".to_string(), "work_item_0002".to_string()),
            ("work_item_0002".to_string(), "work_item_0003".to_string()),
        ],
    );
    set_recovery_unit_dependencies(&fixture, "work_item_0003", &["work_item_0002"]);
    let selected = n5_first_selected_unit(&fixture);
    create_authoritative_active_run(
        &fixture,
        "coding_unit_run_0001",
        1,
        CodingUnitRunStatus::Running,
        None,
        None,
    );
    seed_plan_coding_first_ready_run(
        &fixture,
        &selected,
        crate::product::coding_plan_run::CodingPlanRunPhase::Started,
    );

    let advanced = fixture
        .engine
        .complete_group_unit_after_code_review(&sc_attempt)
        .await
        .expect("first unit completion auto-advances to the next ready unit");

    assert_eq!(advanced.status, CodingAttemptStatus::Running);
    assert_eq!(advanced.stage, CodingExecutionStage::PrepareContext);

    let units = fixture
        .store
        .list_coding_units(&advanced.project_id, &advanced.issue_id, &advanced.id)
        .expect("units after advance");
    let completed = units
        .iter()
        .find(|unit| unit.id == selected.id)
        .expect("selected unit");
    assert_eq!(completed.status, CodingExecutionUnitStatus::Completed);
    assert_eq!(
        completed.completion_commit.as_deref(),
        Some(fixture.original_head.as_str())
    );
    assert!(completed.latest_handoff_revision_id.is_some());
    let next = units
        .iter()
        .find(|unit| unit.logical_work_item_id == "work_item_0002")
        .expect("next unit");
    assert_eq!(next.status, CodingExecutionUnitStatus::Running);
    assert_eq!(advanced.active_unit_id.as_deref(), Some(next.id.as_str()));
    assert_eq!(
        advanced.current_work_item_id.as_deref(),
        Some("work_item_0002")
    );
    // 引擎级 start 证据=unit Running+权威指针+PrepareContext；unit run 记录
    // 由 runner 管道的 provider 声明物化（start_pending_coding_unit_run 对
    // 无记录的新 unit 不预写 run）。
    let completed_run = fixture
        .store
        .list_coding_unit_runs(&advanced, &selected.id)
        .expect("selected unit runs")
        .into_iter()
        .find(|run| run.id == "coding_unit_run_0001")
        .expect("authoritative run preserved");
    assert_eq!(completed_run.status, CodingUnitRunStatus::Completed);
    assert_eq!(
        completed_run.completion_commit.as_deref(),
        Some(fixture.original_head.as_str())
    );

    // SC selector 真实运行并落 Ready 快照（不再被暂停先行短路）。
    let snapshot = fixture
        .store
        .get_group_dependency_gate_snapshot(&advanced)
        .expect("dependency gate snapshot lookup")
        .expect("selector ran after completion");
    assert_eq!(
        snapshot.status,
        crate::product::coding_models::GroupDependencyGateStatus::Ready
    );

    // 不写默认暂停：run 保持 Started、零暂停 gate。
    assert_eq!(
        n5_plan_run_phase(&fixture),
        crate::product::coding_plan_run::CodingPlanRunPhase::Started
    );
    assert!(
        fixture
            .store
            .list_open_blocked_gates(&advanced.project_id, &advanced.issue_id, &advanced.id)
            .expect("blocked gates")
            .is_empty(),
        "no default pause gate may be created"
    );

    // 注：夹具直接改写依赖图 JSON 未重编译 plan projection，故此处不做
    // integrity 全量断言（真实链路图/投影原子一致）。
    // P5 撤除：完成窗口后的 provider 准入不再被「冻结首项已完成」短路拒绝。
    fixture
        .store
        .ensure_provider_run_allowed(&advanced)
        .expect("auto-advance window admits the next unit's provider run");
}

/// S2b：无剩余 Pending units——完成即 plan_complete（全 Completed）：转入
/// ReviewRequest 进入人工终审链，不再保持 Blocked+CodeReview 暂停；run
/// 相位保持 Started、零暂停 gate。
#[tokio::test]
async fn plan_coding_s2_last_unit_completion_reaches_review_request() {
    let fixture = group_completion_fixture(true, false);
    let selected = n5_first_selected_unit(&fixture);
    for unit in fixture
        .store
        .list_coding_units(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .expect("units")
        .iter()
        .filter(|unit| unit.id != selected.id)
    {
        fixture
            .store
            .update_coding_unit_status(
                &fixture.attempt.project_id,
                &fixture.attempt.issue_id,
                &fixture.attempt.id,
                &unit.id,
                CodingExecutionUnitStatus::Completed,
                Some("already completed before the advance".to_string()),
            )
            .expect("pre-complete remaining unit");
    }
    create_authoritative_active_run(
        &fixture,
        "coding_unit_run_0001",
        1,
        CodingUnitRunStatus::Running,
        None,
        None,
    );
    seed_plan_coding_first_ready_run(
        &fixture,
        &selected,
        crate::product::coding_plan_run::CodingPlanRunPhase::Started,
    );

    let completed = fixture
        .engine
        .complete_group_unit_after_code_review(&fixture.attempt)
        .await
        .expect("last target unit completion reaches ReviewRequest");

    assert_eq!(completed.status, CodingAttemptStatus::Running);
    assert_eq!(completed.stage, CodingExecutionStage::ReviewRequest);
    assert!(completed.active_unit_id.is_none());
    let units = fixture
        .store
        .list_coding_units(&completed.project_id, &completed.issue_id, &completed.id)
        .expect("units after completion");
    assert!(
        units
            .iter()
            .all(|unit| unit.status == CodingExecutionUnitStatus::Completed),
        "completion facts are preserved"
    );
    assert!(
        fixture
            .store
            .list_open_blocked_gates(&completed.project_id, &completed.issue_id, &completed.id)
            .expect("blocked gates")
            .is_empty(),
        "plan completion must not create a pause gate"
    );
    assert_eq!(
        n5_plan_run_phase(&fixture),
        crate::product::coding_plan_run::CodingPlanRunPhase::Started
    );
}

/// S2b：清指针中断窗口的 CompletedRetry 重入不再补安全暂停——按无 run
/// 的既有恢复语义继续完成链：恢复完成事实后自动推进下一 unit。legacy
/// 暂停形状（Blocked+CodeReview+清指针，run Paused）重入失败关闭，交由
/// 入口面处置，完成链不再识别/补写暂停身份。
#[tokio::test]
async fn plan_coding_s2_completed_retry_recovers_and_advances() {
    let (fixture, partial, lineage, handoff) = cleared_active_recovery_fixture();
    let selected = fixture
        .store
        .list_coding_units(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .expect("units")
        .into_iter()
        .find(|unit| unit.logical_work_item_id == handoff.logical_work_item_id)
        .expect("completed selected unit");
    seed_plan_coding_first_ready_run(
        &fixture,
        &selected,
        crate::product::coding_plan_run::CodingPlanRunPhase::Started,
    );

    // ① 清指针后、旧 pause 写回前的中断窗口：CompletedRetry 恢复完成事实
    // 后自动推进 work_item_0002（与无 run 的恢复路径同语义）。
    let advanced = fixture
        .engine
        .complete_group_unit_after_code_review(&partial)
        .await
        .expect("completed retry recovers and auto-advances");
    assert_eq!(advanced.status, CodingAttemptStatus::Running);
    assert_eq!(advanced.stage, CodingExecutionStage::PrepareContext);
    let units = fixture
        .store
        .list_coding_units(&advanced.project_id, &advanced.issue_id, &advanced.id)
        .expect("units after advance");
    let next = units
        .iter()
        .find(|unit| unit.logical_work_item_id == "work_item_0002")
        .expect("next unit");
    assert_eq!(next.status, CodingExecutionUnitStatus::Running);
    assert_eq!(advanced.active_unit_id.as_deref(), Some(next.id.as_str()));
    // unit run 记录由 runner 管道物化（见红测①说明）。
    assert_eq!(
        WorkItemRevisionStore::new(fixture.store.paths())
            .get_handoff_revision(&lineage, &handoff.logical_work_item_id, &handoff.id)
            .expect("handoff preserved"),
        handoff
    );
    assert_eq!(
        n5_plan_run_phase(&fixture),
        crate::product::coding_plan_run::CodingPlanRunPhase::Started
    );
    assert!(
        fixture
            .store
            .list_open_blocked_gates(&advanced.project_id, &advanced.issue_id, &advanced.id)
            .expect("blocked gates after recovery")
            .is_empty()
    );

    // ② legacy 暂停形状重入（Blocked+CodeReview+清指针 + run Paused）：
    // 完成链失败关闭（preflight 拒绝非 Running），不再幂等返回暂停。
    let run_store = crate::product::coding_plan_run::CodingPlanRunStore::new(fixture.store.paths());
    let run = run_store
        .get(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            "work_item_plan_0001",
        )
        .expect("plan run lookup")
        .expect("plan run present");
    run_store
        .mark_paused(&run)
        .expect("seed legacy paused run phase");
    let mut legacy_paused = advanced.clone();
    legacy_paused.status = CodingAttemptStatus::Blocked;
    legacy_paused.stage = CodingExecutionStage::CodeReview;
    legacy_paused.active_unit_id = None;
    legacy_paused.current_work_item_id = None;
    legacy_paused.updated_at = "2026-10-14T00:00:00Z".to_string();
    fixture
        .store
        .write_coding_attempt_for_test(&legacy_paused)
        .expect("legacy paused attempt shape");
    let error = fixture
        .engine
        .complete_group_unit_after_code_review(&legacy_paused)
        .await
        .expect_err("legacy paused re-entry must fail closed");
    assert!(
        error
            .to_string()
            .contains("group_completion_stage_not_ready"),
        "unexpected re-entry result: {error}"
    );
}

/// S2b 红测②：依赖未就绪不 start——完成 WI-0001 后，若 WI-0002 还有
/// 未解除的依赖（已 Completed 但未发布 handoff 的 work_item_0003），
/// SC selector 给出 Waiting：不启动任何下一 unit、清 active 指针、落
/// Waiting 快照；plan run 保持 Started、零暂停 gate。
#[tokio::test]
async fn plan_coding_s2_unmet_dependency_waits_without_start() {
    let fixture = group_completion_fixture(true, false);
    let mut sc_attempt = fixture.attempt.clone();
    sc_attempt.admission_kind = crate::product::coding_models::CodingAdmissionKind::ScAdvance;
    fixture
        .store
        .write_coding_attempt_for_test(&sc_attempt)
        .expect("sc admission kind");
    // 0002 同时依赖 0001（本次完成）与 0003（已完成但未发布 handoff）。
    set_recovery_graph_edges(
        &fixture,
        vec![
            ("work_item_0001".to_string(), "work_item_0002".to_string()),
            ("work_item_0003".to_string(), "work_item_0002".to_string()),
        ],
    );
    set_recovery_unit_dependencies(
        &fixture,
        "work_item_0002",
        &["work_item_0001", "work_item_0003"],
    );
    let selected = n5_first_selected_unit(&fixture);
    let blocker = fixture
        .store
        .list_coding_units(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .expect("units")
        .into_iter()
        .find(|unit| unit.logical_work_item_id == "work_item_0003")
        .expect("blocker unit");
    fixture
        .store
        .update_coding_unit_status(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
            &blocker.id,
            CodingExecutionUnitStatus::Completed,
            Some("completed without publishing handoff".to_string()),
        )
        .expect("pre-complete blocker without handoff");
    create_authoritative_active_run(
        &fixture,
        "coding_unit_run_0001",
        1,
        CodingUnitRunStatus::Running,
        None,
        None,
    );
    seed_plan_coding_first_ready_run(
        &fixture,
        &selected,
        crate::product::coding_plan_run::CodingPlanRunPhase::Started,
    );

    let waiting = fixture
        .engine
        .complete_group_unit_after_code_review(&sc_attempt)
        .await
        .expect("unmet dependency advances into Waiting");

    let units = fixture
        .store
        .list_coding_units(&waiting.project_id, &waiting.issue_id, &waiting.id)
        .expect("units after waiting");
    let consumer = units
        .iter()
        .find(|unit| unit.logical_work_item_id == "work_item_0002")
        .expect("consumer unit");
    assert_eq!(
        consumer.status,
        CodingExecutionUnitStatus::Pending,
        "dependency 未就绪的 unit 不得被 start"
    );
    assert!(
        fixture
            .store
            .list_coding_unit_runs(&waiting, &consumer.id)
            .expect("consumer runs")
            .is_empty(),
        "consumer 未 Ready 前不得有任何 run"
    );
    assert!(
        waiting.active_unit_id.is_none(),
        "Waiting 不得保留执行权威指针"
    );
    let snapshot = fixture
        .store
        .get_group_dependency_gate_snapshot(&waiting)
        .expect("gate snapshot lookup")
        .expect("waiting snapshot");
    assert_eq!(
        snapshot.status,
        crate::product::coding_models::GroupDependencyGateStatus::Waiting
    );
    assert_eq!(snapshot.pending_unit_ids, vec![consumer.id.clone()]);
    assert_eq!(
        n5_plan_run_phase(&fixture),
        crate::product::coding_plan_run::CodingPlanRunPhase::Started
    );
    assert!(
        fixture
            .store
            .list_open_blocked_gates(&waiting.project_id, &waiting.issue_id, &waiting.id)
            .expect("blocked gates")
            .is_empty()
    );
}

/// S2b 红测③：失败链停——provider 非重试失败后 attempt Blocked、失败
/// unit Blocked、依赖 unit 保持 Pending 零 run、不推进；失败不是暂停：
/// run 相位保持 Started、不产生 CONTINUATION 暂停 gate。
#[tokio::test]
async fn plan_coding_s2_failure_stops_dependent_chain_without_pause() {
    let fixture = group_completion_fixture_at_stage(true, false, CodingExecutionStage::Coding);
    let mut sc_attempt = fixture.attempt.clone();
    sc_attempt.admission_kind = crate::product::coding_models::CodingAdmissionKind::ScAdvance;
    fixture
        .store
        .write_coding_attempt_for_test(&sc_attempt)
        .expect("sc admission kind");
    let selected = n5_first_selected_unit(&fixture);
    create_authoritative_active_run(
        &fixture,
        "coding_unit_run_0001",
        1,
        CodingUnitRunStatus::Running,
        None,
        None,
    );
    seed_plan_coding_first_ready_run(
        &fixture,
        &selected,
        crate::product::coding_plan_run::CodingPlanRunPhase::Started,
    );

    let outcome = fixture
        .engine
        .handle_group_unit_failure(
            &sc_attempt,
            &selected.id,
            crate::product::coding_workspace_engine::provider_retry::ProviderFailureClassification::NonRetryable {
                reason_code: "provider_nonretryable".to_string(),
                interaction_wait: false,
            },
        )
        .await
        .expect("failure is classified");
    assert!(
        matches!(
            outcome,
            crate::product::coding_workspace_engine::group::GroupUnitFailureOutcome::AwaitingManualRecovery {
                ref reason_code,
                ..
            } if reason_code == "provider_nonretryable"
        ),
        "unexpected failure outcome for the non-retryable provider failure"
    );

    let blocked = fixture
        .store
        .get_attempt(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .expect("blocked attempt");
    assert_eq!(blocked.status, CodingAttemptStatus::Blocked);
    let units = fixture
        .store
        .list_coding_units(&blocked.project_id, &blocked.issue_id, &blocked.id)
        .expect("units after failure");
    let failed = units
        .iter()
        .find(|unit| unit.id == selected.id)
        .expect("failed unit");
    assert_eq!(failed.status, CodingExecutionUnitStatus::Blocked);
    for unit in units
        .iter()
        .filter(|unit| unit.logical_work_item_id == "work_item_0002")
    {
        assert_eq!(unit.status, CodingExecutionUnitStatus::Pending);
        assert!(
            fixture
                .store
                .list_coding_unit_runs(&blocked, &unit.id)
                .expect("dependent runs")
                .is_empty(),
            "失败链停：依赖 unit 不得被 start"
        );
    }
    assert_eq!(
        n5_plan_run_phase(&fixture),
        crate::product::coding_plan_run::CodingPlanRunPhase::Started,
        "失败不写暂停相位"
    );
    let gates = fixture
        .store
        .list_open_blocked_gates(&blocked.project_id, &blocked.issue_id, &blocked.id)
        .expect("failure gates");
    assert_eq!(gates.len(), 1);
    assert_eq!(
        gates[0].reason_code.as_deref(),
        Some("provider_nonretryable"),
        "失败门携带失败 reason，而非默认暂停门"
    );
}

/// 无 plan-run 的普通旧 group：完成链维持既有推进（下一 unit 正常启动）。
#[tokio::test]
async fn plan_coding_n5_no_run_uses_existing_advance() {
    let fixture = group_completion_fixture(true, false);
    create_authoritative_active_run(
        &fixture,
        "coding_unit_run_0001",
        1,
        CodingUnitRunStatus::Running,
        None,
        None,
    );

    let updated = fixture
        .engine
        .complete_group_unit_after_code_review(&fixture.attempt)
        .await
        .expect("legacy advance path is unchanged");

    let next = fixture
        .store
        .list_coding_units(&updated.project_id, &updated.issue_id, &updated.id)
        .expect("units")
        .into_iter()
        .find(|unit| unit.logical_work_item_id == "work_item_0002")
        .expect("next unit");
    assert_eq!(next.status, CodingExecutionUnitStatus::Running);
    assert_eq!(updated.active_unit_id.as_deref(), Some(next.id.as_str()));
    assert_eq!(updated.stage, CodingExecutionStage::PrepareContext);
    assert!(
        fixture
            .store
            .list_open_blocked_gates(&updated.project_id, &updated.issue_id, &updated.id)
            .expect("blocked gates")
            .is_empty(),
        "no pause gate without a matching plan run"
    );
}

/// P2 修复轮：非首 unit 完成 + issue 共享 worktree 锁——advance 的锁转移
/// from 身份必须取完成前的 current 指针（重读后指针已清会回退创建期
/// work_item_id，使 from≠锁 owner 而 `issue_worktree_active` 失败）。
#[tokio::test]
async fn plan_coding_n5_second_unit_advance_keeps_worktree_lock_transfer() {
    let fixture = group_completion_fixture(false, false);
    create_authoritative_active_run(
        &fixture,
        "coding_unit_run_0001",
        1,
        CodingUnitRunStatus::Running,
        None,
        None,
    );
    // 第 1 个 unit 正常完成并推进第 2 个 unit（无共享锁记录，跳过转移）。
    let second = fixture
        .engine
        .complete_group_unit_after_code_review(&fixture.attempt)
        .await
        .expect("first unit completes and advances");
    let second_unit = fixture
        .store
        .list_coding_units(&second.project_id, &second.issue_id, &second.id)
        .expect("units")
        .into_iter()
        .find(|unit| unit.logical_work_item_id == "work_item_0002")
        .expect("second unit running");
    assert_eq!(second_unit.status, CodingExecutionUnitStatus::Running);
    let second = fixture
        .store
        .update_attempt_stage(
            &second.project_id,
            &second.issue_id,
            &second.id,
            CodingExecutionStage::ReviewRequest,
        )
        .expect("second unit review request stage");

    // issue 共享 worktree 锁由本 attempt 持有、active 指向第 2 个 unit。
    let lifecycle = LifecycleStore::new(fixture.store.paths());
    lifecycle
        .upsert_issue_shared_worktree(
            crate::product::lifecycle_store::UpsertIssueSharedWorktreeInput {
                project_id: fixture.attempt.project_id.clone(),
                issue_id: fixture.attempt.issue_id.clone(),
                repository_id: "repository_0001".to_string(),
                branch_name: fixture.attempt.branch_name.clone(),
                worktree_path: fixture.worktree.clone(),
                base_branch: fixture.original_head.clone(),
            },
        )
        .expect("shared worktree record");
    lifecycle
        .try_acquire_issue_worktree_lock(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            "work_item_0002",
            &second.id,
        )
        .expect("second unit owns the shared worktree lock");
    create_authoritative_active_run(
        &fixture,
        "coding_unit_run_0002",
        1,
        CodingUnitRunStatus::Running,
        None,
        None,
    );

    // 第 2 个 unit 完成：锁必须成功转移到第 3 个 unit（from=work_item_0002）。
    let third = fixture
        .engine
        .complete_group_unit_after_code_review(&second)
        .await
        .expect("second unit completion transfers the worktree lock");
    let third_unit = fixture
        .store
        .list_coding_units(&third.project_id, &third.issue_id, &third.id)
        .expect("units")
        .into_iter()
        .find(|unit| unit.logical_work_item_id == "work_item_0003")
        .expect("third unit");
    assert_eq!(third_unit.status, CodingExecutionUnitStatus::Running);
    assert_eq!(
        third.active_unit_id.as_deref(),
        Some(third_unit.id.as_str())
    );
    let record = lifecycle
        .get_issue_shared_worktree(&fixture.attempt.project_id, &fixture.attempt.issue_id)
        .expect("shared worktree after transfer")
        .expect("shared worktree after transfer");
    assert_eq!(
        record.current_active_work_item_id.as_deref(),
        Some("work_item_0003")
    );
    assert_eq!(
        record.current_lock_owner_id.as_deref(),
        Some(third.id.as_str())
    );
}

/// S2 切片C(CompletedRetry 恢复):`complete_coding_unit_run` 幂等——已完成
/// run 同 completion_commit 重放返回权威事实,异 commit fail-closed;清指针
/// CompletedRetry 重入后 handoff 重建逐字段相等(确定性重建,不产生第二
/// 事实)。
#[tokio::test]
async fn completed_retry_completion_is_idempotent_and_rebuilds_handoff_field_equal() {
    let (fixture, partial, lineage, canonical) = cleared_active_recovery_fixture();

    // ① store 级幂等:同 commit 重放 → 原样返回权威 run。
    let replayed = fixture
        .store
        .complete_coding_unit_run(
            &partial,
            &canonical.coding_unit_run_id,
            &canonical.commit_sha,
        )
        .expect("idempotent replay returns the authoritative completed run");
    assert_eq!(replayed.status, CodingUnitRunStatus::Completed);
    assert_eq!(
        replayed.completion_commit.as_deref(),
        Some(canonical.commit_sha.as_str())
    );
    // 异 commit 重放 → fail-closed(identity mismatch),不得改写完成事实。
    let divergent = fixture
        .store
        .complete_coding_unit_run(&partial, &canonical.coding_unit_run_id, "deadbeef0000")
        .expect_err("divergent completion commit must fail closed");
    assert!(
        divergent
            .to_string()
            .contains("product_store_identity_mismatch"),
        "unexpected divergent replay result: {divergent}"
    );

    // ② CompletedRetry 重入(清指针恢复形状):完成链重建 handoff。
    let advanced = fixture
        .engine
        .complete_group_unit_after_code_review(&partial)
        .await
        .expect("completed retry re-entry rebuilds the completion facts");
    assert_eq!(advanced.status, CodingAttemptStatus::Running);
    let unit = fixture
        .store
        .list_coding_units(&advanced.project_id, &advanced.issue_id, &advanced.id)
        .expect("units after retry")
        .into_iter()
        .find(|unit| unit.logical_work_item_id == canonical.logical_work_item_id)
        .expect("completed unit after retry");
    let rebuilt = WorkItemRevisionStore::new(fixture.store.paths())
        .get_handoff_revision(
            &lineage,
            &canonical.logical_work_item_id,
            unit.latest_handoff_revision_id
                .as_deref()
                .expect("handoff pointer after retry"),
        )
        .expect("rebuilt handoff");
    // 逐字段相等:整结构相等 + 关键字段显式对拍。
    assert_eq!(rebuilt, canonical);
    assert_eq!(rebuilt.id, canonical.id);
    assert_eq!(rebuilt.logical_work_item_id, canonical.logical_work_item_id);
    assert_eq!(rebuilt.work_item_revision_id, canonical.work_item_revision_id);
    assert_eq!(rebuilt.coding_unit_run_id, canonical.coding_unit_run_id);
    assert_eq!(rebuilt.commit_sha, canonical.commit_sha);
    assert_eq!(rebuilt.contract_hash, canonical.contract_hash);
    assert_eq!(rebuilt.created_at, canonical.created_at);
    assert_eq!(
        rebuilt.provided_contracts, canonical.provided_contracts,
        "rebuilt handoff must keep the provided contract set stable"
    );
    assert_eq!(
        rebuilt.provided_capabilities, canonical.provided_capabilities,
        "rebuilt handoff must keep the provided capability map stable"
    );
}
