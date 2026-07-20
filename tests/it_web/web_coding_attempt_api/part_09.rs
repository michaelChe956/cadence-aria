#[tokio::test]
async fn concurrent_same_work_item_loser_preserves_winner_issue_worktree_lease() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let state = WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    );
    let acquire_pause = state
        .test_controls
        .pause_next_coding_attempt_after_worktree_acquire();
    let app = build_web_router(state);
    bootstrap_two_ready_confirmed_work_items(app.clone(), root.path(), repo.path()).await;

    let paused_app = app.clone();
    let paused_request = tokio::spawn(async move {
        request_json(
            paused_app,
            Method::POST,
            "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0001/coding-attempts",
            json!({}),
        )
        .await
    });
    tokio::time::timeout(
        std::time::Duration::from_secs(3),
        acquire_pause.wait_until_paused(),
    )
    .await
    .expect("first request did not pause after worktree acquire");

    let (winner_status, winner) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0001/coding-attempts",
        json!({}),
    )
    .await;
    assert_eq!(winner_status, StatusCode::OK);
    let winner_attempt_id = assert_global_attempt_id(&winner);

    acquire_pause.resume();
    let (loser_status, loser) = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        paused_request,
    )
    .await
    .expect("paused request did not finish")
    .expect("paused request task");
    assert_eq!(loser_status, StatusCode::CONFLICT);
    assert_eq!(loser["code"], "coding_attempt_active");

    let lifecycle = LifecycleStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let shared = lifecycle
        .get_issue_shared_worktree("project_0001", "issue_0001")
        .expect("load issue shared worktree")
        .expect("issue shared worktree");
    assert_eq!(
        shared.current_active_work_item_id.as_deref(),
        Some("work_item_0001")
    );
    assert_eq!(
        shared.current_lock_owner_id.as_deref(),
        Some(winner_attempt_id.as_str())
    );

    let (blocked_status, blocked) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0002/coding-attempts",
        json!({}),
    )
    .await;
    assert_eq!(blocked_status, StatusCode::CONFLICT);
    assert_eq!(blocked["code"], "issue_worktree_active");

    let (abort_status, _abort) = request_json(
        app.clone(),
        Method::POST,
        &scoped_attempt_uri(&winner_attempt_id, "/abort"),
        json!({}),
    )
    .await;
    assert_eq!(abort_status, StatusCode::OK);

    let (next_status, next_attempt) = request_json(
        app,
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0002/coding-attempts",
        json!({}),
    )
    .await;
    assert_eq!(next_status, StatusCode::OK);
    assert_global_attempt_id(&next_attempt);
}
