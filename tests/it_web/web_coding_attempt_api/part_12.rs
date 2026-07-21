#[tokio::test]
async fn group_initialization_recovers_bind_before_phase_advance_after_restart() {
    use cadence_aria::product::coding_attempt_store::CodingGroupInitializationPhase;
    use cadence_aria::web::test_controls::GroupAttemptInitializationCheckpoint;

    let root = tempdir().expect("root");
    let repo = git_repo();
    let initial_state = WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    );
    initial_state
        .test_controls
        .fail_next_group_attempt_initialization_at(
            GroupAttemptInitializationCheckpoint::BoundBeforePhaseAdvance,
        );
    let initial_app = build_web_router(initial_state);
    bootstrap_confirmed_work_item_plan_group(initial_app.clone(), repo.path()).await;
    let create_path = "/api/projects/project_0001/issues/issue_0001/work-item-plans/work_item_plan_0001/coding-attempts";

    let (interrupted_status, interrupted) =
        request_json(initial_app, Method::POST, create_path, json!({})).await;
    assert_eq!(interrupted_status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        interrupted["code"],
        "coding_group_initialization_interrupted"
    );

    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let store = CodingAttemptStore::new(app_paths.clone());
    let journal = store
        .get_group_initialization("project_0001", "issue_0001", "work_item_plan_0001")
        .expect("bind-before-phase journal");
    assert_eq!(
        journal.phase,
        CodingGroupInitializationPhase::AttemptPersisted
    );
    let lifecycle = LifecycleStore::new(app_paths.clone());
    let bound = lifecycle
        .get_issue_shared_worktree("project_0001", "issue_0001")
        .expect("bound shared worktree")
        .expect("bound shared worktree");
    assert_eq!(
        bound.current_lock_owner_id.as_deref(),
        Some(journal.attempt.id.as_str())
    );

    let restarted_app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    let (retry_status, retry) =
        request_json(restarted_app, Method::POST, create_path, json!({})).await;
    assert_eq!(retry_status, StatusCode::OK, "{retry}");
    assert_eq!(retry["attempt_id"], journal.attempt.id);

    let completed = store
        .get_group_initialization("project_0001", "issue_0001", "work_item_plan_0001")
        .expect("completed journal");
    assert_eq!(completed.phase, CodingGroupInitializationPhase::Completed);
    let recovered = store
        .get_attempt("project_0001", "issue_0001", &completed.attempt.id)
        .expect("recovered attempt");
    store
        .validate_group_attempt_integrity(&recovered)
        .expect("recovered group integrity");
}

#[tokio::test]
async fn single_does_not_bind_unfinished_group_attempt_before_group_retry() {
    use cadence_aria::product::coding_attempt_store::CodingGroupInitializationPhase;
    use cadence_aria::web::test_controls::GroupAttemptInitializationCheckpoint;

    let root = tempdir().expect("root");
    let repo = git_repo();
    let initial_state = WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    );
    initial_state
        .test_controls
        .fail_next_group_attempt_initialization_at(
            GroupAttemptInitializationCheckpoint::PersistedBeforeBind,
        );
    let initial_app = build_web_router(initial_state);
    bootstrap_confirmed_work_item_plan_group(initial_app.clone(), repo.path()).await;
    let group_path = "/api/projects/project_0001/issues/issue_0001/work-item-plans/work_item_plan_0001/coding-attempts";

    let (interrupted_status, interrupted) =
        request_json(initial_app, Method::POST, group_path, json!({})).await;
    assert_eq!(interrupted_status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        interrupted["code"],
        "coding_group_initialization_interrupted"
    );

    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let store = CodingAttemptStore::new(app_paths.clone());
    let journal = store
        .get_group_initialization("project_0001", "issue_0001", "work_item_plan_0001")
        .expect("persisted-before-bind journal");
    assert_eq!(
        journal.phase,
        CodingGroupInitializationPhase::AttemptPersisted
    );
    let lifecycle = LifecycleStore::new(app_paths);
    let pending = lifecycle
        .get_issue_shared_worktree("project_0001", "issue_0001")
        .expect("pending shared worktree")
        .expect("pending shared worktree");
    assert_eq!(
        pending.current_lock_owner_id.as_deref(),
        Some(journal.worktree_lease_id.as_str())
    );

    let restarted_app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    let single_path = "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0001/coding-attempts";
    let (single_status, single) = request_json(
        restarted_app.clone(),
        Method::POST,
        single_path,
        json!({}),
    )
    .await;
    assert_eq!(single_status, StatusCode::CONFLICT, "{single}");
    assert_eq!(single["code"], "coding_attempt_active");
    let after_single = lifecycle
        .get_issue_shared_worktree("project_0001", "issue_0001")
        .expect("shared worktree after single conflict")
        .expect("shared worktree after single conflict");
    assert_eq!(
        after_single.current_lock_owner_id.as_deref(),
        Some(journal.worktree_lease_id.as_str()),
        "Single must not bind a Group initialization lease"
    );

    let (retry_status, retry) =
        request_json(restarted_app, Method::POST, group_path, json!({})).await;
    assert_eq!(retry_status, StatusCode::OK, "{retry}");
    assert_eq!(retry["attempt_id"], journal.attempt.id);
    let recovered = lifecycle
        .get_issue_shared_worktree("project_0001", "issue_0001")
        .expect("recovered shared worktree")
        .expect("recovered shared worktree");
    assert_eq!(
        recovered.current_lock_owner_id.as_deref(),
        Some(journal.attempt.id.as_str())
    );
}

#[tokio::test]
async fn delete_reconciles_worktree_created_before_journal_phase_advance() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    bootstrap_confirmed_work_item(app.clone(), repo.path()).await;
    let (create_status, created) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0001/coding-attempts",
        json!({}),
    )
    .await;
    assert_eq!(create_status, StatusCode::OK, "{created}");
    let attempt_id = assert_global_attempt_id(&created);
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .get_attempt("project_0001", "issue_0001", &attempt_id)
        .expect("attempt");
    let before_head = String::from_utf8_lossy(
        &Command::new("git")
            .args(["rev-parse", &attempt.base_branch])
            .current_dir(repo.path())
            .output()
            .expect("base head")
            .stdout,
    )
    .trim()
    .to_string();
    let worktree = repo
        .path()
        .join(".worktrees/aria-work-items")
        .join(&attempt.work_item_id)
        .join(format!("attempt-{}", attempt.attempt_no));
    let journal = store
        .prepare_coding_git_operation(
            &attempt,
            PrepareCodingGitOperationInput {
                kind: CodingGitOperationKind::WorktreePrepare,
                repo_path: repo.path().to_path_buf(),
                worktree_path: worktree.clone(),
                branch_name: attempt.branch_name.clone(),
                base_branch: attempt.base_branch.clone(),
                before_head,
                remote: None,
                commit_message: None,
            },
        )
        .expect("prepare journal");
    let git = GitWorkspaceService::new();
    git.create_branch(repo.path(), &attempt.branch_name, &attempt.base_branch)
        .await
        .expect("branch");
    store
        .advance_coding_git_operation(
            &attempt,
            &journal,
            CodingGitOperationPhase::BranchCreated,
            None,
        )
        .expect("branch phase");
    git.create_worktree(repo.path(), &attempt.branch_name, &worktree)
        .await
        .expect("worktree side effect");

    let (delete_status, delete) = request_json(
        app,
        Method::DELETE,
        &scoped_attempt_uri(&attempt_id, ""),
        json!({}),
    )
    .await;

    assert_eq!(delete_status, StatusCode::NO_CONTENT, "{delete}");
    assert!(!worktree.exists());
    assert!(!branch_exists(repo.path(), &attempt.branch_name));
    assert!(
        store
            .get_attempt("project_0001", "issue_0001", &attempt_id)
            .is_err()
    );
}
