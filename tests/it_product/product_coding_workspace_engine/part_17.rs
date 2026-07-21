#[tokio::test]
async fn group_unit_owner_conflict_does_not_advance_unit_or_attempt() {
    let (_root, paths, store, engine, attempt) = group_engine_with_two_units();
    let lifecycle = LifecycleStore::new(paths.clone());
    lifecycle
        .upsert_issue_shared_worktree(UpsertIssueSharedWorktreeInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: paths.root().join("shared-worktree"),
            base_branch: "HEAD".to_string(),
        })
        .expect("upsert shared worktree");
    lifecycle
        .try_acquire_issue_worktree_lock(
            "project_0001",
            "issue_0001",
            "work_item_0001",
            "coding_attempt_other",
        )
        .expect("acquire conflicting owner lock");

    let error = engine
        .complete_current_group_unit(&attempt, Some("must not persist".to_string()))
        .await
        .expect_err("owner mismatch must fail before state changes");
    assert!(matches!(
        error,
        cadence_aria::product::coding_workspace_engine::CodingWorkspaceEngineError::Store(
            cadence_aria::product::json_store::ProductStoreError::Conflict {
            kind: "issue_worktree_lock_owner",
            ..
            }
        )
    ));

    let current = store
        .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("reload attempt");
    let units = store
        .list_coding_units(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("reload units");
    assert_eq!(current.current_work_item_id.as_deref(), Some("work_item_0001"));
    assert_eq!(current.active_unit_id.as_deref(), Some("coding_unit_0001"));
    assert_eq!(units[0].status, CodingExecutionUnitStatus::Running);
    assert_eq!(units[1].status, CodingExecutionUnitStatus::Pending);
}

#[tokio::test]
async fn final_confirm_owner_conflict_does_not_complete_attempt() {
    let root = tempdir().expect("root");
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle = LifecycleStore::new(paths.clone());
    lifecycle
        .upsert_issue_shared_worktree(UpsertIssueSharedWorktreeInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: root.path().join("shared-worktree"),
            base_branch: "main".to_string(),
        })
        .expect("shared worktree");
    lifecycle
        .try_acquire_issue_worktree_lock(
            "project_0001",
            "issue_0001",
            "work_item_0001",
            "coding_attempt_other",
        )
        .expect("conflicting owner lock");
    let (store, attempt) = final_confirm_attempt(paths, "work_item_0001");
    let (tx, _rx) = tokio::sync::mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

    let error = engine
        .handle_final_confirm("project_0001", "issue_0001", &attempt.id)
        .await
        .expect_err("owner mismatch must fail before terminal status");
    assert!(matches!(
        error,
        cadence_aria::product::coding_workspace_engine::CodingWorkspaceEngineError::Store(
            cadence_aria::product::json_store::ProductStoreError::Conflict {
                kind: "issue_worktree_lock_owner",
                ..
            }
        )
    ));
    assert_eq!(
        store
            .get_attempt("project_0001", "issue_0001", &attempt.id)
            .expect("reload attempt")
            .status,
        CodingAttemptStatus::WaitingForHuman
    );
}

#[tokio::test]
async fn abort_owner_conflict_does_not_abort_attempt() {
    let root = tempdir().expect("root");
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle = LifecycleStore::new(paths.clone());
    lifecycle
        .upsert_issue_shared_worktree(UpsertIssueSharedWorktreeInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: root.path().join("shared-worktree"),
            base_branch: "main".to_string(),
        })
        .expect("shared worktree");
    lifecycle
        .try_acquire_issue_worktree_lock(
            "project_0001",
            "issue_0001",
            "work_item_0001",
            "coding_attempt_other",
        )
        .expect("conflicting owner lock");
    let (store, attempt) = coding_store_with_attempt(root.path(), "work_item_0001", "branch");
    let (tx, _rx) = tokio::sync::mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

    engine
        .handle_abort("project_0001", "issue_0001", &attempt.id)
        .await
        .expect_err("owner mismatch must fail before abort");
    assert_eq!(
        store
            .get_attempt("project_0001", "issue_0001", &attempt.id)
            .expect("reload attempt")
            .status,
        CodingAttemptStatus::Created
    );
}

#[tokio::test]
async fn failure_owner_conflict_does_not_fail_attempt() {
    let root = tempdir().expect("root");
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle = LifecycleStore::new(paths.clone());
    lifecycle
        .upsert_issue_shared_worktree(UpsertIssueSharedWorktreeInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: root.path().join("shared-worktree"),
            base_branch: "main".to_string(),
        })
        .expect("shared worktree");
    lifecycle
        .try_acquire_issue_worktree_lock(
            "project_0001",
            "issue_0001",
            "work_item_0001",
            "coding_attempt_other",
        )
        .expect("conflicting owner lock");
    let (store, attempt) = coding_store_with_attempt(root.path(), "work_item_0001", "branch");
    let (tx, _rx) = tokio::sync::mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

    engine
        .handle_attempt_failed("project_0001", "issue_0001", &attempt.id)
        .await
        .expect_err("owner mismatch must fail before terminal failure");
    assert_eq!(
        store
            .get_attempt("project_0001", "issue_0001", &attempt.id)
            .expect("reload attempt")
            .status,
        CodingAttemptStatus::Created
    );
}
