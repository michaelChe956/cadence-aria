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

// ---- N5（REQ-CS-05）：plan-run first_ready 首项完成安全暂停短路 ----

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

/// 有剩余 Pending units：完成事实落定后、SC selector/推进之前安全暂停；
/// completion commit/权威 run/handoff/unit Completed 均不回滚。
#[tokio::test]
async fn plan_coding_n5_completion_pauses_before_selector() {
    let fixture = group_completion_fixture(true, false);
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

    let paused = fixture
        .engine
        .complete_group_unit_after_code_review(&sc_attempt)
        .await
        .expect("first unit completion pauses before the selector");

    assert_eq!(paused.status, CodingAttemptStatus::Blocked);
    assert_eq!(paused.stage, CodingExecutionStage::CodeReview);
    assert!(paused.active_unit_id.is_none());
    assert!(paused.current_work_item_id.is_none());

    let units = fixture
        .store
        .list_coding_units(&paused.project_id, &paused.issue_id, &paused.id)
        .expect("units after pause");
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
    for unit in units.iter().filter(|unit| unit.id != selected.id) {
        assert_eq!(
            unit.status,
            CodingExecutionUnitStatus::Pending,
            "remaining units keep their factual state"
        );
        assert!(
            fixture
                .store
                .list_coding_unit_runs(&paused, &unit.id)
                .expect("other unit runs")
                .is_empty(),
            "no other unit may be started"
        );
    }
    let completed_run = fixture
        .store
        .list_coding_unit_runs(&paused, &selected.id)
        .expect("selected unit runs")
        .into_iter()
        .find(|run| run.id == "coding_unit_run_0001")
        .expect("authoritative run preserved");
    assert_eq!(completed_run.status, CodingUnitRunStatus::Completed);
    assert_eq!(
        completed_run.completion_commit.as_deref(),
        Some(fixture.original_head.as_str())
    );

    assert!(
        fixture
            .store
            .get_group_dependency_gate_snapshot(&paused)
            .expect("dependency gate snapshot lookup")
            .is_none(),
        "the SC selector must not run before the pause"
    );
    assert!(
        fixture
            .store
            .list_review_requests(&paused.project_id, &paused.issue_id, &paused.id)
            .expect("review requests")
            .is_empty(),
        "no review request/push may be issued"
    );

    let gates = fixture
        .store
        .list_open_blocked_gates(&paused.project_id, &paused.issue_id, &paused.id)
        .expect("pause gates");
    assert_eq!(gates.len(), 1);
    assert_eq!(
        gates[0].reason_code.as_deref(),
        Some(crate::product::coding_plan_run::PLAN_CODING_CONTINUATION_NOT_ENABLED)
    );
    assert!(
        gates[0].available_actions.is_empty(),
        "pause gate exposes no actions"
    );

    assert_eq!(
        n5_plan_run_phase(&fixture),
        crate::product::coding_plan_run::CodingPlanRunPhase::Paused
    );
    fixture
        .store
        .validate_group_attempt_integrity(&paused)
        .expect("legal plan-coding pause passes integrity validation");
    let error = fixture
        .store
        .ensure_provider_run_allowed(&paused)
        .expect_err("paused run must reject provider admission");
    assert!(
        error.to_string().contains("plan_coding_paused"),
        "unexpected admission result: {error}"
    );
}

/// 无剩余 Pending units：不因本仓无下一 unit 转入 ReviewRequest/push，保持
/// Blocked+CodeReview 暂停。
#[tokio::test]
async fn plan_coding_n5_last_target_unit_stays_code_review() {
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
                Some("already completed before the pause".to_string()),
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

    let paused = fixture
        .engine
        .complete_group_unit_after_code_review(&fixture.attempt)
        .await
        .expect("last target unit pauses instead of ReviewRequest");

    assert_eq!(paused.status, CodingAttemptStatus::Blocked);
    assert_eq!(paused.stage, CodingExecutionStage::CodeReview);
    assert!(paused.active_unit_id.is_none());
    assert!(paused.current_work_item_id.is_none());
    assert!(
        fixture
            .store
            .list_review_requests(&paused.project_id, &paused.issue_id, &paused.id)
            .expect("review requests")
            .is_empty(),
        "no review request/push may be issued"
    );
    let units = fixture
        .store
        .list_coding_units(&paused.project_id, &paused.issue_id, &paused.id)
        .expect("units after pause");
    assert!(
        units
            .iter()
            .all(|unit| unit.status == CodingExecutionUnitStatus::Completed),
        "completion facts are preserved"
    );
    let gates = fixture
        .store
        .list_open_blocked_gates(&paused.project_id, &paused.issue_id, &paused.id)
        .expect("pause gates");
    assert_eq!(gates.len(), 1);
    assert!(gates[0].available_actions.is_empty());
    assert_eq!(
        n5_plan_run_phase(&fixture),
        crate::product::coding_plan_run::CodingPlanRunPhase::Paused
    );
}

/// CompletedRetry 同身份恢复：清指针后 pause 写回前的中断窗口先补安全暂停；
/// 暂停后的重入幂等返回，run Paused 补记中断可恢复；均不推进下一 unit。
#[tokio::test]
async fn plan_coding_n5_completed_retry_reconciles_pause_window() {
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

    // ① 清指针后、pause 写回前的中断窗口：CompletedRetry 先落安全暂停，
    // 不进 SC FailedClosed/next-unit。
    let paused = fixture
        .engine
        .complete_group_unit_after_code_review(&partial)
        .await
        .expect("completed retry reconciles the pause window");
    assert_eq!(paused.status, CodingAttemptStatus::Blocked);
    assert_eq!(paused.stage, CodingExecutionStage::CodeReview);
    assert!(paused.active_unit_id.is_none());
    assert!(paused.current_work_item_id.is_none());
    let units = fixture
        .store
        .list_coding_units(&paused.project_id, &paused.issue_id, &paused.id)
        .expect("units after pause");
    let next = units
        .iter()
        .find(|unit| unit.logical_work_item_id == "work_item_0002")
        .expect("next unit");
    assert_eq!(next.status, CodingExecutionUnitStatus::Pending);
    assert!(
        fixture
            .store
            .list_coding_unit_runs(&paused, &next.id)
            .expect("next unit runs")
            .is_empty()
    );
    assert_eq!(
        WorkItemRevisionStore::new(fixture.store.paths())
            .get_handoff_revision(&lineage, &handoff.logical_work_item_id, &handoff.id)
            .expect("handoff preserved"),
        handoff
    );
    assert_eq!(
        n5_plan_run_phase(&fixture),
        crate::product::coding_plan_run::CodingPlanRunPhase::Paused
    );

    // ② 暂停后的 CompletedRetry 重入：不因 Blocked+清指针被 preflight 误拒，
    // 幂等返回暂停形状，不新增 gate、不推进。
    let again = fixture
        .engine
        .complete_group_unit_after_code_review(&paused)
        .await
        .expect("paused re-entry returns the paused attempt");
    assert_eq!(again.status, CodingAttemptStatus::Blocked);
    assert_eq!(again.stage, CodingExecutionStage::CodeReview);
    assert_eq!(
        fixture
            .store
            .list_open_blocked_gates(&again.project_id, &again.issue_id, &again.id)
            .expect("gates after re-entry")
            .len(),
        1
    );
    assert_eq!(
        fixture
            .store
            .list_coding_units(&again.project_id, &again.issue_id, &again.id)
            .expect("units after re-entry")
            .iter()
            .find(|unit| unit.logical_work_item_id == "work_item_0002")
            .expect("next unit")
            .status,
        CodingExecutionUnitStatus::Pending
    );

    // ③ run Paused 补记中断窗口：attempt 已暂停而 run 相位仍是 Started，
    // 重入按 matching completed selection 恢复补记。
    let runs_dir = fixture
        .store
        .paths()
        .issue_root(&fixture.attempt.project_id, &fixture.attempt.issue_id)
        .join("coding-plan-runs");
    let run_path = runs_dir.join("work_item_plan_0001.json");
    let mut run_value: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&run_path).expect("read plan run"))
            .expect("parse plan run");
    run_value["phase"] = serde_json::json!("started");
    run_value
        .as_object_mut()
        .expect("run object")
        .remove("pause_reason");
    fs::write(
        &run_path,
        serde_json::to_string(&run_value).expect("serialize plan run"),
    )
    .expect("reset run phase");

    let recovered = fixture
        .engine
        .complete_group_unit_after_code_review(&again)
        .await
        .expect("re-entry re-marks the run paused");
    assert_eq!(recovered.status, CodingAttemptStatus::Blocked);
    assert_eq!(recovered.stage, CodingExecutionStage::CodeReview);
    assert_eq!(
        n5_plan_run_phase(&fixture),
        crate::product::coding_plan_run::CodingPlanRunPhase::Paused
    );
    assert_eq!(
        fixture
            .store
            .list_coding_units(&recovered.project_id, &recovered.issue_id, &recovered.id)
            .expect("units after recovery")
            .iter()
            .find(|unit| unit.logical_work_item_id == "work_item_0002")
            .expect("next unit")
            .status,
        CodingExecutionUnitStatus::Pending
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
