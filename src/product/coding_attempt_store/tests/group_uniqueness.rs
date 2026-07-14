use super::*;

#[test]
fn rejects_second_group_attempt_after_original_completed() {
    let (_tmp, store) = setup_store();
    let first = store
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
    store
        .update_attempt_status(
            PROJECT_ID,
            ISSUE_ID,
            &first.id,
            CodingAttemptStatus::Running,
        )
        .expect("start group attempt");
    store
        .update_attempt_status(
            PROJECT_ID,
            ISSUE_ID,
            &first.id,
            CodingAttemptStatus::Completed,
        )
        .expect("complete group attempt");

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
    let first = store
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
    store
        .update_attempt_status(
            PROJECT_ID,
            ISSUE_ID,
            &first.id,
            CodingAttemptStatus::Running,
        )
        .expect("start first attempt");
    store
        .update_attempt_status(
            PROJECT_ID,
            ISSUE_ID,
            &first.id,
            CodingAttemptStatus::Completed,
        )
        .expect("complete first attempt");

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
