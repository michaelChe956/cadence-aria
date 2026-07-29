#[tokio::test]
async fn returns_coding_attempt_snapshot_with_persisted_execution_state() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    bootstrap_confirmed_work_item(app.clone(), repo.path()).await;

    let (status, attempt) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0001/coding-attempts",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let attempt_id = assert_global_attempt_id(&attempt);

    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let persisted_attempt = store
        .get_attempt("project_0001", "issue_0001", &attempt_id)
        .expect("persisted attempt");
    let code_review = sample_code_review_report(&attempt_id);
    let review_request = sample_review_request(&attempt_id);
    let internal_review = sample_internal_review(&attempt_id, &review_request.id);
    store
        .save_code_review_report(&persisted_attempt, &code_review)
        .expect("save code review");
    store
        .save_review_request(&persisted_attempt, &review_request)
        .expect("save review request");
    store
        .save_internal_pr_review(&persisted_attempt, &internal_review)
        .expect("save internal review");
    store
        .save_timeline_node(&persisted_attempt, sample_completed_node(&attempt_id))
        .expect("save completed node");
    store
        .save_timeline_node(&persisted_attempt, sample_running_node(&attempt_id))
        .expect("save running node");
    store
        .create_choice_gate(&persisted_attempt, CreateChoiceGateInput {
            attempt_id: attempt_id.clone(),
            choice_id: "choice_0001".to_string(),
            stage: CodingExecutionStage::Coding,
            node_id: Some("coding_node_0002".to_string()),
            role: CodingProviderRole::Coder,
            provider: ProviderName::Codex,
            source: "request_user_input".to_string(),
            prompt: "请选择实现范围".to_string(),
            options: vec![CodingChoiceOption {
                id: "backend_first".to_string(),
                label: "先做后端".to_string(),
                description: None,
            }],
            allow_multiple: false,
            allow_free_text: true,
        })
        .expect("create choice gate");

    let (status, snapshot) = request_json(
        app,
        Method::GET,
        &scoped_attempt_uri(&attempt_id, ""),
        json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(snapshot["attempt"]["attempt_id"], attempt_id);
    assert_eq!(snapshot["attempt"]["stage"], "prepare_context");
    assert_eq!(snapshot["active_node_id"], "coding_node_0002");
    assert_eq!(snapshot["timeline_nodes"].as_array().unwrap().len(), 2);
    assert_eq!(
        snapshot["code_review_reports"][0]["summary"],
        code_review.summary.as_str()
    );
    assert_eq!(
        snapshot["review_request"]["commit_sha"],
        review_request.commit_sha.as_str()
    );
    assert_eq!(
        snapshot["internal_pr_review"]["summary"],
        internal_review.summary.as_str()
    );
    assert_eq!(snapshot["pending_choices"][0]["choice_id"], "choice_0001");
    assert_eq!(
        snapshot["pending_choices"][0]["source"],
        "request_user_input"
    );
}

#[tokio::test]
async fn coding_attempt_snapshot_does_not_reactivate_historical_blocked_node() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    bootstrap_confirmed_work_item(app.clone(), repo.path()).await;

    let (status, attempt) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0001/coding-attempts",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let attempt_id = assert_global_attempt_id(&attempt);

    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let persisted_attempt = store
        .get_attempt("project_0001", "issue_0001", &attempt_id)
        .expect("persisted attempt");
    let mut blocked_node = sample_running_node(&attempt_id);
    blocked_node.id = "coding_node_0001".to_string();
    blocked_node.status = CodingTimelineNodeStatus::Blocked;
    blocked_node.summary = Some("code review 被阻塞".to_string());
    blocked_node.completed_at = Some("2026-05-23T00:03:00Z".to_string());
    let mut completed_retry_node = sample_completed_node(&attempt_id);
    completed_retry_node.id = "coding_node_0002".to_string();
    completed_retry_node.stage = CodingExecutionStage::CodeReview;
    completed_retry_node.started_at = "2026-05-23T00:04:00Z".to_string();
    completed_retry_node.completed_at = Some("2026-05-23T00:05:00Z".to_string());
    store
        .save_timeline_node(&persisted_attempt, blocked_node)
        .expect("save blocked node");
    store
        .save_timeline_node(&persisted_attempt, completed_retry_node)
        .expect("save completed retry node");

    let (status, snapshot) = request_json(
        app,
        Method::GET,
        &scoped_attempt_uri(&attempt_id, ""),
        json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(snapshot["active_node_id"], Value::Null);
}

#[tokio::test]
async fn aborts_coding_attempt_and_allows_next_attempt_for_same_work_item() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    bootstrap_confirmed_work_item(app.clone(), repo.path()).await;

    let (status, first) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0001/coding-attempts",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let first_attempt_id = assert_global_attempt_id(&first);

    let (status, aborted) = request_json(
        app.clone(),
        Method::POST,
        &scoped_attempt_uri(&first_attempt_id, "/abort"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(aborted["attempt_id"], first_attempt_id);
    assert_eq!(aborted["status"], "aborted");

    let (status, second) = request_json(
        app,
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0001/coding-attempts",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let second_attempt_id = assert_global_attempt_id(&second);
    assert_ne!(second_attempt_id, first_attempt_id);
    assert_eq!(second["attempt_no"], 2);
}

#[tokio::test]
async fn deletes_coding_attempt_and_preserves_work_item() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    bootstrap_confirmed_work_item(app.clone(), repo.path()).await;
    let (status, attempt) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0001/coding-attempts",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let attempt_id = assert_global_attempt_id(&attempt);

    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = prepare_attempt_with_worktree(
        &store,
        repo.path(),
        "project_0001",
        "issue_0001",
        &attempt_id,
    );
    let artifact_dir =
        store.attempt_test_output_root("project_0001", "issue_0001", &attempt_id);
    fs::create_dir_all(&artifact_dir).expect("artifact dir");
    fs::write(artifact_dir.join("unit.stdout.log"), "unit stdout\n").expect("artifact");
    store
        .save_timeline_node(&attempt, sample_running_node(&attempt_id))
        .expect("save timeline node");
    let attempt_dir = artifact_dir
        .parent()
        .and_then(|path| path.parent())
        .expect("attempt dir")
        .to_path_buf();
    let worktree_path = attempt.worktree_path.clone().expect("worktree path");
    assert!(attempt_dir.exists());
    assert!(worktree_path.exists());
    assert!(branch_exists(repo.path(), &attempt.branch_name));

    let (status, _body) = request_json(
        app.clone(),
        Method::DELETE,
        &scoped_attempt_uri(&attempt_id, ""),
        json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(!attempt_dir.exists());
    assert!(!worktree_path.exists());
    assert!(!branch_exists(repo.path(), &attempt.branch_name));

    let (status, _) = request_json(
        app.clone(),
        Method::GET,
        &scoped_attempt_uri(&attempt_id, ""),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, lifecycle) = request_json(
        app.clone(),
        Method::GET,
        "/api/issues/issue_0001/lifecycle?project_id=project_0001",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(lifecycle["work_items"].as_array().unwrap().len(), 1);
    assert!(lifecycle["work_items"][0]["latest_attempt"].is_null());
    assert!(lifecycle["coding_attempts"].as_array().unwrap().is_empty());

    let (status, second) = request_json(
        app,
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0001/coding-attempts",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_global_attempt_id(&second);
    assert_eq!(second["attempt_no"], 1);
}

#[tokio::test]
async fn delete_work_item_rejected_preserves_coding_attempts_worktrees_and_branches() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    bootstrap_confirmed_work_item(app.clone(), repo.path()).await;
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));

    // work_item_0001 上有一个活跃 coding attempt（含 worktree + 分支）。
    let (status, created) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0001/coding-attempts",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let attempt_id = assert_global_attempt_id(&created);
    let attempt = prepare_attempt_with_worktree(
        &store,
        repo.path(),
        "project_0001",
        "issue_0001",
        &attempt_id,
    );
    let artifact_dir = store.attempt_test_output_root("project_0001", "issue_0001", &attempt_id);
    fs::create_dir_all(&artifact_dir).expect("artifact dir");
    fs::write(artifact_dir.join("unit.stdout.log"), "log\n").expect("artifact");
    let attempt_dir = artifact_dir
        .parent()
        .and_then(|path| path.parent())
        .expect("attempt dir")
        .to_path_buf();
    let worktree_path = attempt
        .worktree_path
        .as_ref()
        .expect("attempt worktree")
        .to_path_buf();

    // 新语义（spec `harden-work-item-group-deletion`）：存在 coding workspace 时拒绝删除
    // work item，要求用户先删 coding workspace。拒绝时 attempt / worktree / 分支必须原样保留。
    let (status, body) = request_json(
        app.clone(),
        Method::DELETE,
        "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0001",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "delete body: {body}");
    assert_eq!(body["code"], "coding_workspace_exists");
    assert_eq!(body["details"]["work_item_id"], "work_item_0001");
    assert_eq!(body["details"]["attempt_id"], attempt_id);

    // 拒绝时 coding workspace 全部产物不动——用户要继续用或自行清理。
    assert!(attempt_dir.exists(), "attempt dir must remain after rejection");
    assert!(
        worktree_path.exists(),
        "worktree must remain after rejection"
    );
    assert!(
        branch_exists(repo.path(), &attempt.branch_name),
        "branch must remain after rejection"
    );
    assert!(
        !store
            .list_attempts_for_work_item("project_0001", "issue_0001", "work_item_0001")
            .expect("list attempts")
            .is_empty(),
        "attempt must remain after rejection"
    );

    let (status, lifecycle) = request_json(
        app,
        Method::GET,
        "/api/issues/issue_0001/lifecycle?project_id=project_0001",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        !lifecycle["work_items"].as_array().unwrap().is_empty(),
        "work item must remain after rejection"
    );
    assert!(
        !lifecycle["coding_attempts"].as_array().unwrap().is_empty(),
        "coding attempt must remain after rejection"
    );
}
