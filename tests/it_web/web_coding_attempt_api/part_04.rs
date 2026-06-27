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
    let attempt_id = attempt["attempt_id"].as_str().expect("attempt id");

    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let testing_report = sample_testing_report(attempt_id);
    let code_review = sample_code_review_report(attempt_id);
    let review_request = sample_review_request(attempt_id);
    let internal_review = sample_internal_review(attempt_id, &review_request.id);
    store
        .save_testing_report(&testing_report)
        .expect("save testing report");
    store
        .save_code_review_report(&code_review)
        .expect("save code review");
    store
        .save_review_request(&review_request)
        .expect("save review request");
    store
        .save_internal_pr_review(&internal_review)
        .expect("save internal review");
    store
        .save_timeline_node(sample_completed_node(attempt_id))
        .expect("save completed node");
    store
        .save_timeline_node(sample_running_node(attempt_id))
        .expect("save running node");
    store
        .create_choice_gate(CreateChoiceGateInput {
            attempt_id: attempt_id.to_string(),
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
        "/api/coding-attempts/coding_attempt_0001",
        json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(snapshot["attempt"]["attempt_id"], "coding_attempt_0001");
    assert_eq!(snapshot["attempt"]["stage"], "prepare_context");
    assert_eq!(snapshot["active_node_id"], "coding_node_0002");
    assert_eq!(snapshot["timeline_nodes"].as_array().unwrap().len(), 2);
    assert_eq!(snapshot["testing_report"]["id"], testing_report.id.as_str());
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
    assert_eq!(first["attempt_id"], "coding_attempt_0001");

    let (status, aborted) = request_json(
        app.clone(),
        Method::POST,
        "/api/coding-attempts/coding_attempt_0001/abort",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(aborted["attempt_id"], "coding_attempt_0001");
    assert_eq!(aborted["status"], "aborted");

    let (status, second) = request_json(
        app,
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0001/coding-attempts",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(second["attempt_id"], "coding_attempt_0002");
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
    assert_eq!(attempt["attempt_id"], "coding_attempt_0001");

    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = prepare_attempt_with_worktree(
        &store,
        repo.path(),
        "project_0001",
        "issue_0001",
        "coding_attempt_0001",
    );
    let artifact_dir =
        store.attempt_test_output_root("project_0001", "issue_0001", "coding_attempt_0001");
    fs::create_dir_all(&artifact_dir).expect("artifact dir");
    fs::write(artifact_dir.join("unit.stdout.log"), "unit stdout\n").expect("artifact");
    store
        .save_testing_report(&sample_testing_report("coding_attempt_0001"))
        .expect("save testing report");
    store
        .save_timeline_node(sample_running_node("coding_attempt_0001"))
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
        "/api/coding-attempts/coding_attempt_0001",
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
        "/api/coding-attempts/coding_attempt_0001",
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
    assert_eq!(second["attempt_id"], "coding_attempt_0001");
    assert_eq!(second["attempt_no"], 1);
}

#[tokio::test]
async fn delete_work_item_cascades_coding_attempts_worktrees_and_branches() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    bootstrap_confirmed_work_item(app.clone(), repo.path()).await;
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));

    let (status, _) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0001/coding-attempts",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let first = prepare_attempt_with_worktree(
        &store,
        repo.path(),
        "project_0001",
        "issue_0001",
        "coding_attempt_0001",
    );
    let first_artifact_dir =
        store.attempt_test_output_root("project_0001", "issue_0001", "coding_attempt_0001");
    fs::create_dir_all(&first_artifact_dir).expect("first artifact dir");
    fs::write(first_artifact_dir.join("unit.stdout.log"), "first\n").expect("first artifact");
    let (status, _) = request_json(
        app.clone(),
        Method::POST,
        "/api/coding-attempts/coding_attempt_0001/abort",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, _) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0001/coding-attempts",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let second = prepare_attempt_with_worktree(
        &store,
        repo.path(),
        "project_0001",
        "issue_0001",
        "coding_attempt_0002",
    );
    let second_artifact_dir =
        store.attempt_test_output_root("project_0001", "issue_0001", "coding_attempt_0002");
    fs::create_dir_all(&second_artifact_dir).expect("second artifact dir");
    fs::write(second_artifact_dir.join("unit.stdout.log"), "second\n").expect("second artifact");
    let first_attempt_dir = first_artifact_dir
        .parent()
        .and_then(|path| path.parent())
        .expect("first attempt dir")
        .to_path_buf();
    let second_attempt_dir = second_artifact_dir
        .parent()
        .and_then(|path| path.parent())
        .expect("second attempt dir")
        .to_path_buf();

    let (status, body) = request_json(
        app.clone(),
        Method::DELETE,
        "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0001",
        json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "deleted");
    assert!(!first_attempt_dir.exists());
    assert!(!second_attempt_dir.exists());
    assert!(
        !first
            .worktree_path
            .as_ref()
            .expect("first worktree")
            .exists()
    );
    assert!(
        !second
            .worktree_path
            .as_ref()
            .expect("second worktree")
            .exists()
    );
    assert!(!branch_exists(repo.path(), &first.branch_name));
    assert!(!branch_exists(repo.path(), &second.branch_name));

    let (status, lifecycle) = request_json(
        app,
        Method::GET,
        "/api/issues/issue_0001/lifecycle?project_id=project_0001",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(lifecycle["work_items"].as_array().unwrap().is_empty());
    assert!(lifecycle["coding_attempts"].as_array().unwrap().is_empty());
}
