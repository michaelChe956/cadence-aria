use super::*;

#[test]
fn rejects_second_group_attempt_after_original_completed() {
    let (_tmp, store) = setup_store();
    let mut first = store
        .create_group_attempt(CreateGroupCodingAttemptInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            current_work_item_id: WORK_ITEM_ID.to_string(),
            base_branch: "main".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: None,
            provider_config_snapshot: provider_snapshot(),
            max_auto_rework: 2,
        })
        .expect("first group attempt");
    persist_completed_group_uniqueness_fixture(&store, &mut first);

    let error = store
        .create_group_attempt(CreateGroupCodingAttemptInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            current_work_item_id: WORK_ITEM_ID.to_string(),
            base_branch: "main".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: None,
            provider_config_snapshot: provider_snapshot(),
            max_auto_rework: 2,
        })
        .expect_err("same group must not create a second attempt");

    assert_eq!(
        error.to_string(),
        format!(
            "product_store_io: coding_attempt_group_already_exists: {}",
            first.id
        )
    );
}

#[test]
fn allows_group_attempt_for_different_plan_after_original_completed() {
    let (_tmp, store) = setup_store();
    let mut first = store
        .create_group_attempt(CreateGroupCodingAttemptInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            current_work_item_id: WORK_ITEM_ID.to_string(),
            base_branch: "main".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: None,
            provider_config_snapshot: provider_snapshot(),
            max_auto_rework: 2,
        })
        .expect("first group attempt");
    persist_completed_group_uniqueness_fixture(&store, &mut first);

    let second = store
        .create_group_attempt(CreateGroupCodingAttemptInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            plan_id: "work_item_plan_0002".to_string(),
            current_work_item_id: "work_item_0002".to_string(),
            base_branch: "main".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: None,
            provider_config_snapshot: provider_snapshot(),
            max_auto_rework: 2,
        })
        .expect("different group attempt");

    assert_eq!(
        second.work_item_group_id.as_deref(),
        Some("work_item_plan_0002")
    );
}

fn persist_completed_group_uniqueness_fixture(
    store: &CodingAttemptStore,
    attempt: &mut CodingExecutionAttempt,
) {
    let completed_at = chrono::Utc::now().to_rfc3339();
    attempt.status = CodingAttemptStatus::Completed;
    attempt.completed_at = Some(completed_at.clone());
    attempt.updated_at = completed_at;
    store
        .save_coding_attempt(attempt)
        .expect("persist terminal group uniqueness fixture");
}
