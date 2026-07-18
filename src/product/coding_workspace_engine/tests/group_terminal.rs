use super::*;

fn running_group_attempt() -> (
    tempfile::TempDir,
    CodingAttemptStore,
    CodingExecutionAttempt,
) {
    let root = tempdir().expect("tempdir");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_group_attempt(CreateGroupCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            current_work_item_id: "work_item_0001".to_string(),
            base_branch: "HEAD".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: None,
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Codex,
                reviewer: Some(ProviderName::ClaudeCode),
                review_rounds: 1,
            },
            max_auto_rework: 2,
        })
        .expect("group attempt");
    seed_group_attempt_fixture(&store, &attempt, true);
    let attempt = store
        .update_attempt_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running group attempt");
    (root, store, attempt)
}

fn unit_statuses(
    store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
) -> Vec<CodingExecutionUnitStatus> {
    store
        .list_coding_units(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("coding units")
        .into_iter()
        .map(|unit| unit.status)
        .collect()
}

#[tokio::test]
async fn coding_plan_repair_group_terminal_abort_converges_units_and_clears_resume_pointers() {
    let (_root, store, attempt) = running_group_attempt();
    let units = store
        .list_coding_units(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("coding units");
    store
        .update_coding_unit_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &units[2].id,
            CodingExecutionUnitStatus::Completed,
            None,
        )
        .expect("completed unit");
    let (tx, mut rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

    let aborted = engine
        .handle_abort(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .await
        .expect("abort group attempt");

    assert_eq!(aborted.status, CodingAttemptStatus::Aborted);
    assert_eq!(aborted.active_unit_id, None);
    assert_eq!(aborted.current_work_item_id, None);
    assert_eq!(
        unit_statuses(&store, &aborted),
        vec![
            CodingExecutionUnitStatus::Skipped,
            CodingExecutionUnitStatus::Skipped,
            CodingExecutionUnitStatus::Completed,
        ]
    );
    store
        .validate_group_attempt_integrity(&aborted)
        .expect("aborted group integrity");
    let error = engine
        .start_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .await
        .expect_err("terminal group must not restart");
    assert!(
        error
            .to_string()
            .contains("invalid_coding_attempt_status_transition")
    );
    assert!(
        rx.try_recv().is_err(),
        "terminal group emitted a coding event"
    );
}

#[tokio::test]
async fn coding_plan_repair_group_terminal_failure_fails_active_unit_and_skips_pending_units() {
    let (_root, store, attempt) = running_group_attempt();
    let attempt = store
        .update_attempt_stage(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingExecutionStage::CodeReview,
        )
        .expect("code review group attempt");
    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

    let failed = engine
        .handle_attempt_failed(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .await
        .expect("fail group attempt");

    assert_eq!(failed.status, CodingAttemptStatus::Failed);
    assert_eq!(failed.active_unit_id, None);
    assert_eq!(failed.current_work_item_id, None);
    assert_eq!(
        unit_statuses(&store, &failed),
        vec![
            CodingExecutionUnitStatus::Failed,
            CodingExecutionUnitStatus::Skipped,
            CodingExecutionUnitStatus::Skipped,
        ]
    );
    store
        .validate_group_attempt_integrity(&failed)
        .expect("failed group integrity");
    assert!(
        recoverable_failed_code_review(&store, &failed)
            .expect("inspect fatal review failure")
            .is_none()
    );
}

#[test]
fn coding_plan_repair_group_terminal_validator_accepts_final_review_and_completed_states() {
    let (_root, store, attempt) = running_group_attempt();
    for unit in store
        .list_coding_units(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("coding units")
    {
        store
            .update_coding_unit_status(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                &unit.id,
                CodingExecutionUnitStatus::Completed,
                None,
            )
            .expect("complete unit");
    }
    let final_review = store
        .update_attempt_stage(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingExecutionStage::ReviewRequest,
        )
        .expect("final review stage");
    store
        .validate_group_attempt_integrity(&final_review)
        .expect("final review integrity");

    let completed = store
        .update_attempt_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingAttemptStatus::Completed,
        )
        .expect("completed group attempt");
    store
        .validate_group_attempt_integrity(&completed)
        .expect("completed group integrity");
}

#[test]
fn coding_plan_repair_group_terminal_completion_fails_closed_until_all_units_completed() {
    let (_root, store, attempt) = running_group_attempt();
    let original_attempt = store
        .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("original attempt");
    let original_statuses = unit_statuses(&store, &attempt);

    let error = store
        .update_attempt_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingAttemptStatus::Completed,
        )
        .expect_err("incomplete group cannot complete");

    assert!(
        error
            .to_string()
            .contains("coding_group_attempt_incomplete")
    );
    assert_eq!(
        store
            .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("unchanged attempt"),
        original_attempt
    );
    assert_eq!(unit_statuses(&store, &attempt), original_statuses);
}

#[test]
fn coding_plan_repair_group_terminal_write_failure_does_not_expose_terminal_attempt() {
    let (_root, store, attempt) = running_group_attempt();
    let active_unit_id = attempt.active_unit_id.as_deref().expect("active unit");
    let active_unit_path = store
        .paths()
        .issue_lifecycle_root(&attempt.project_id, &attempt.issue_id)
        .join("coding-attempts")
        .join(&attempt.id)
        .join("units")
        .join(format!("{active_unit_id}.json"));
    fs::write(active_unit_path, b"{ invalid json").expect("corrupt active unit");

    store
        .update_attempt_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingAttemptStatus::Aborted,
        )
        .expect_err("unit read failure must abort terminal transition");

    let unchanged = store
        .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("attempt remains readable");
    assert_eq!(unchanged.status, CodingAttemptStatus::Running);
    assert_eq!(unchanged.active_unit_id, attempt.active_unit_id);
    assert_eq!(unchanged.current_work_item_id, attempt.current_work_item_id);
}

#[test]
fn coding_plan_repair_group_terminal_validator_rejects_created_attempt_disguised_as_final_review() {
    let (_root, store, attempt) = running_group_attempt();
    for unit in store
        .list_coding_units(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("coding units")
    {
        store
            .update_coding_unit_status(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                &unit.id,
                CodingExecutionUnitStatus::Completed,
                None,
            )
            .expect("complete unit");
    }
    let mut corrupted = store
        .update_attempt_stage(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingExecutionStage::ReviewRequest,
        )
        .expect("review request stage");
    corrupted.status = CodingAttemptStatus::Created;
    store
        .save_coding_attempt(&corrupted)
        .expect("corrupt attempt status");

    let error = store
        .validate_group_attempt_integrity(&corrupted)
        .expect_err("created attempt cannot use final review no-target state");

    assert!(
        error
            .to_string()
            .contains("coding_group_attempt_incomplete")
    );
}

#[test]
fn coding_plan_repair_single_work_item_terminal_status_update_is_unchanged() {
    let root = tempdir().expect("tempdir");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            work_item_id: "work_item_0001".to_string(),
            base_branch: "HEAD".to_string(),
            branch_name: "aria/work-items/work_item_0001".to_string(),
            worktree_path: None,
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Codex,
                reviewer: Some(ProviderName::ClaudeCode),
                review_rounds: 1,
            },
            max_auto_rework: 2,
        })
        .expect("single attempt");
    let running = store
        .update_attempt_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running single attempt");

    let aborted = store
        .update_attempt_status(
            &running.project_id,
            &running.issue_id,
            &running.id,
            CodingAttemptStatus::Aborted,
        )
        .expect("abort single attempt");

    assert_eq!(aborted.status, CodingAttemptStatus::Aborted);
    assert_eq!(aborted.scope, CodingAttemptScope::WorkItem);
    assert_eq!(aborted.work_item_id, running.work_item_id);
    assert_eq!(aborted.active_unit_id, running.active_unit_id);
    assert_eq!(aborted.current_work_item_id, running.current_work_item_id);
}
