fn cleared_active_recovery_fixture() -> (
    GroupCompletionFixture,
    CodingExecutionAttempt,
    WorkItemPlanLineage,
    HandoffRevision,
) {
    let fixture = group_completion_fixture(true, false);
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
    let (fixture, partial, lineage, handoff) = cleared_active_recovery_fixture();
    let revision_store = WorkItemRevisionStore::new(fixture.store.paths());

    let updated = fixture
        .engine
        .complete_group_unit_after_code_review(&partial)
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
async fn coding_plan_repair_group_completion_rejects_stale_attempt_after_review_request() {
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
    let units_before = fixture
        .store
        .list_coding_units(
            &stale_attempt.project_id,
            &stale_attempt.issue_id,
            &stale_attempt.id,
        )
        .expect("units before");

    fixture
        .engine
        .complete_group_unit_after_code_review(&stale_attempt)
        .await
        .expect_err("stale attempt must not bypass review request guard");

    assert_eq!(
        fixture
            .store
            .list_coding_units(
                &stale_attempt.project_id,
                &stale_attempt.issue_id,
                &stale_attempt.id,
            )
            .expect("units after"),
        units_before
    );
    assert_eq!(
        fixture
            .store
            .get_attempt(
                &stale_attempt.project_id,
                &stale_attempt.issue_id,
                &stale_attempt.id,
            )
            .expect("attempt after")
            .stage,
        CodingExecutionStage::ReviewRequest
    );
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
