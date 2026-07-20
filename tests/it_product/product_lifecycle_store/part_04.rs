#[test]
fn stale_work_item_release_is_rejected_even_for_current_owner() {
    let root = tempdir().expect("tempdir");
    let store = LifecycleStore::new(ProductAppPaths::new(root.path().join(".aria")));
    store
        .upsert_issue_shared_worktree(UpsertIssueSharedWorktreeInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: PathBuf::from("/tmp/repo/.worktrees/aria-issues/issue_0001"),
            base_branch: "main".to_string(),
        })
        .expect("shared worktree");
    store
        .try_acquire_issue_worktree_lock(
            "project_0001",
            "issue_0001",
            "work_item_0001",
            "coding_attempt_owner",
        )
        .expect("first lock");
    store
        .transfer_issue_worktree_lock(
            "project_0001",
            "issue_0001",
            "work_item_0001",
            "work_item_0002",
            "coding_attempt_owner",
        )
        .expect("transfer lock");

    let error = store
        .release_issue_worktree_lock(
            "project_0001",
            "issue_0001",
            "work_item_0001",
            "coding_attempt_owner",
        )
        .expect_err("stale work item must not release the transferred lock");
    assert!(matches!(
        error,
        ProductStoreError::Conflict {
            kind: "issue_worktree_lock_owner",
            ..
        }
    ));
    assert_eq!(
        store
            .get_issue_shared_worktree("project_0001", "issue_0001")
            .expect("reload")
            .expect("shared worktree")
            .current_active_work_item_id
            .as_deref(),
        Some("work_item_0002")
    );
}

#[test]
fn wrong_owner_completion_does_not_mutate_worktree_history() {
    let root = tempdir().expect("tempdir");
    let store = LifecycleStore::new(ProductAppPaths::new(root.path().join(".aria")));
    store
        .upsert_issue_shared_worktree(UpsertIssueSharedWorktreeInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: PathBuf::from("/tmp/repo/.worktrees/aria-issues/issue_0001"),
            base_branch: "main".to_string(),
        })
        .expect("shared worktree");
    store
        .try_acquire_issue_worktree_lock(
            "project_0001",
            "issue_0001",
            "work_item_0001",
            "coding_attempt_winner",
        )
        .expect("winner lock");
    store
        .transfer_issue_worktree_lock(
            "project_0001",
            "issue_0001",
            "work_item_0001",
            "work_item_0002",
            "coding_attempt_winner",
        )
        .expect("transfer winner lock");

    let error = store
        .mark_issue_worktree_completed_item(
            "project_0001",
            "issue_0001",
            "work_item_0001",
            "coding_attempt_loser",
        )
        .expect_err("wrong owner must not record completion");
    assert!(matches!(
        error,
        ProductStoreError::Conflict {
            kind: "issue_worktree_lock_owner",
            ..
        }
    ));
    let shared = store
        .get_issue_shared_worktree("project_0001", "issue_0001")
        .expect("reload")
        .expect("shared worktree");
    assert_eq!(shared.last_completed_work_item_id, None);
    assert_eq!(
        shared.current_lock_owner_id.as_deref(),
        Some("coding_attempt_winner")
    );
}
