#[tokio::test]
async fn retry_reconciles_attempt_persisted_before_lease_bind_after_restart() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let initial_state = WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    );
    initial_state
        .test_controls
        .fail_next_coding_attempt_after_persist_before_bind();
    let initial_app = build_web_router(initial_state);
    bootstrap_two_ready_confirmed_work_items(initial_app.clone(), root.path(), repo.path()).await;

    let create_path = "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0001/coding-attempts";
    let (interrupted_status, interrupted) =
        request_json(initial_app, Method::POST, create_path, json!({})).await;
    assert_eq!(interrupted_status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(interrupted["code"], "coding_attempt_bind_interrupted");

    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let orphan_store = CodingAttemptStore::new(app_paths.clone());
    let orphan = orphan_store
        .get_active_attempt("project_0001", "issue_0001", "work_item_0001")
        .expect("active orphan lookup")
        .expect("active orphan");
    let lifecycle = LifecycleStore::new(app_paths.clone());
    let orphan_lease = lifecycle
        .get_issue_shared_worktree("project_0001", "issue_0001")
        .expect("orphan lease")
        .expect("orphan lease");
    assert_eq!(
        orphan_lease.current_active_work_item_id.as_deref(),
        Some("work_item_0001")
    );
    assert!(
        orphan_lease
            .current_lock_owner_id
            .as_deref()
            .is_some_and(|owner| owner.starts_with("issue_worktree_lease_"))
    );

    let restarted_app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    let (blocked_status, blocked) = request_json(
        restarted_app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0002/coding-attempts",
        json!({}),
    )
    .await;
    assert_eq!(blocked_status, StatusCode::CONFLICT);
    assert_eq!(blocked["code"], "issue_worktree_active");

    let (retry_status, retry) =
        request_json(restarted_app.clone(), Method::POST, create_path, json!({})).await;
    assert_eq!(retry_status, StatusCode::CONFLICT);
    assert_eq!(retry["code"], "coding_attempt_active");

    let restarted_store = CodingAttemptStore::new(app_paths);
    let active_attempts = restarted_store
        .list_attempts_for_work_item("project_0001", "issue_0001", "work_item_0001")
        .expect("attempts after restart")
        .into_iter()
        .filter(|attempt| attempt.status.is_active())
        .collect::<Vec<_>>();
    assert_eq!(active_attempts, vec![orphan.clone()]);
    let reconciled = lifecycle
        .get_issue_shared_worktree("project_0001", "issue_0001")
        .expect("reconciled lease")
        .expect("reconciled lease");
    assert_eq!(
        reconciled.current_active_work_item_id.as_deref(),
        Some("work_item_0001")
    );
    assert_eq!(
        reconciled.current_lock_owner_id.as_deref(),
        Some(orphan.id.as_str())
    );

    let (abort_status, _abort) = request_json(
        restarted_app,
        Method::POST,
        &scoped_attempt_uri(&orphan.id, "/abort"),
        json!({}),
    )
    .await;
    assert_eq!(abort_status, StatusCode::OK);
    let released = lifecycle
        .get_issue_shared_worktree("project_0001", "issue_0001")
        .expect("released lease")
        .expect("released lease");
    assert_eq!(released.current_active_work_item_id, None);
    assert_eq!(released.current_lock_owner_id, None);
    assert_eq!(
        restarted_store
            .get_attempt("project_0001", "issue_0001", &orphan.id)
            .expect("aborted orphan")
            .status,
        CodingAttemptStatus::Aborted
    );
}

#[tokio::test]
async fn retry_does_not_reconcile_ambiguous_active_attempts() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let initial_state = WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    );
    initial_state
        .test_controls
        .fail_next_coding_attempt_after_persist_before_bind();
    let initial_app = build_web_router(initial_state);
    bootstrap_two_ready_confirmed_work_items(initial_app.clone(), root.path(), repo.path()).await;
    let create_path = "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0001/coding-attempts";
    let (interrupted_status, _) =
        request_json(initial_app, Method::POST, create_path, json!({})).await;
    assert_eq!(interrupted_status, StatusCode::INTERNAL_SERVER_ERROR);

    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let store = CodingAttemptStore::new(app_paths.clone());
    let first = store
        .get_active_attempt("project_0001", "issue_0001", "work_item_0001")
        .expect("first attempt lookup")
        .expect("first attempt");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &first.id,
            CodingAttemptStatus::Aborted,
        )
        .expect("temporarily abort first attempt");
    let second = store
        .create_attempt(CreateCodingAttemptInput {
            project_id: first.project_id.clone(),
            issue_id: first.issue_id.clone(),
            work_item_id: first.work_item_id.clone(),
            base_branch: first.base_branch.clone(),
            branch_name: first.branch_name.clone(),
            worktree_path: first.worktree_path.clone(),
            provider_config_snapshot: first.provider_config_snapshot.clone(),
            max_auto_rework: first.max_auto_rework,
        })
        .expect("second attempt");
    store
        .save_coding_attempt(&first)
        .expect("restore duplicate active attempt fixture");
    let lifecycle = LifecycleStore::new(app_paths);
    let lease_before = lifecycle
        .get_issue_shared_worktree("project_0001", "issue_0001")
        .expect("lease before")
        .expect("lease before");

    let restarted_app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    let (retry_status, retry) =
        request_json(restarted_app, Method::POST, create_path, json!({})).await;

    assert_eq!(retry_status, StatusCode::CONFLICT);
    assert_eq!(retry["code"], "coding_attempt_ambiguous");
    assert_eq!(
        lifecycle
            .get_issue_shared_worktree("project_0001", "issue_0001")
            .expect("lease after")
            .expect("lease after"),
        lease_before
    );
    let active_ids = store
        .list_attempts_for_work_item("project_0001", "issue_0001", "work_item_0001")
        .expect("active attempts")
        .into_iter()
        .filter(|attempt| attempt.status.is_active())
        .map(|attempt| attempt.id)
        .collect::<Vec<_>>();
    assert_eq!(active_ids, vec![first.id, second.id]);
}
