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
            },
            max_auto_rework: 2,
        })
        .expect("group attempt");
    seed_group_attempt_fixture(&store, &attempt, true, with_dependency);
    let attempt = store
        .update_attempt_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running attempt");
    let attempt = store
        .update_attempt_stage(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            stage,
        )
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
    legacy_handoffs: Vec<Option<WorkItemHandoff>>,
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
    let legacy_handoffs = units
        .iter()
        .map(|unit| {
            fixture.store.get_coding_unit_handoff(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                &unit.id,
            )
        })
        .collect::<Result<Vec<_>, _>>()
        .expect("legacy handoff snapshot");
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
        legacy_handoffs,
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
    save_active_legacy_handoff(
        &fixture,
        vec![
            "cargo test --locked".to_string(),
            "cargo check --locked".to_string(),
            "cargo test --locked".to_string(),
        ],
        vec![
            "src/z.rs".to_string(),
            "src/a.rs".to_string(),
            "src/z.rs".to_string(),
        ],
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
    let handoff = expected_handoff_revision(
        &source_run,
        &fixture.original_head,
        "2026-07-19T00:00:00Z",
    );
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
        CodingExecutionStage::Testing,
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
            error.to_string().contains("group_completion_stage_not_ready"),
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
        CodingExecutionStage::Testing,
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
            error.to_string().contains("group_completion_stage_not_ready"),
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
    fs::write(fixture.worktree.join("untracked.txt"), "untracked\n")
        .expect("untracked file");
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
    assert_eq!(
        first_gate.stage,
        Some(CodingExecutionStage::ReviewRequest)
    );
    assert_eq!(
        first_gate.reason_code.as_deref(),
        Some("shared_worktree_dirty_manual_gate")
    );
    assert_eq!(first_gate.available_actions.len(), 2);
    assert!(first_gate.available_actions.iter().any(|action| {
        action.action_type == CodingGateActionType::ManualContinue
    }));
    assert!(
        first_gate
            .available_actions
            .iter()
            .any(|action| action.action_type == CodingGateActionType::Abort)
    );
    assert!(!first_gate.available_actions.iter().any(|action| {
        action.action_type == CodingGateActionType::SendToCoder
    }));
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

    assert_eq!(updated.head_commit.as_deref(), Some(committed_head.as_str()));
    assert_eq!(
        completed.completion_commit.as_deref(),
        Some(committed_head.as_str())
    );
}
