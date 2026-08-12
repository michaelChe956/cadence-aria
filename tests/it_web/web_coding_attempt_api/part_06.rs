#[tokio::test]
async fn scoped_coding_attempt_api_loads_exact_attempt_and_legacy_route_reports_ambiguity() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    bootstrap_confirmed_work_item(app.clone(), repo.path()).await;

    let (_, created) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0001/coding-attempts",
        json!({}),
    )
    .await;
    let attempt_id = assert_global_attempt_id(&created);
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let mut duplicate = store
        .get_attempt("project_0001", "issue_0001", &attempt_id)
        .expect("original attempt");
    duplicate.issue_id = "issue_0002".to_string();
    store
        .update_attempt_non_status_fields(&duplicate)
        .expect("duplicate legacy scope");

    let (scoped_status, scoped) = request_json(
        app.clone(),
        Method::GET,
        &scoped_attempt_uri(&attempt_id, ""),
        json!({}),
    )
    .await;
    assert_eq!(scoped_status, StatusCode::OK);
    assert_eq!(scoped["attempt"]["project_id"], "project_0001");
    assert_eq!(scoped["attempt"]["issue_id"], "issue_0001");

    let (legacy_status, legacy) = request_json(
        app,
        Method::GET,
        &format!("/api/coding-attempts/{attempt_id}"),
        json!({}),
    )
    .await;
    assert_eq!(legacy_status, StatusCode::CONFLICT);
    assert_eq!(legacy["code"], "coding_attempt_ambiguous");
}

#[tokio::test]
async fn scoped_coding_attempt_api_reports_scope_mismatch() {
    let root = tempdir().expect("root");
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            work_item_id: "work_item_0001".to_string(),
            base_branch: "main".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: None,
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Fake,
                reviewer: Some(ProviderName::Fake),
                review_rounds: 1,
                permission_modes: cadence_aria::product::models::WorkspaceRolePermissionModes::default(),
            },
            target_snapshot: None,
            max_auto_rework: 2,
        })
        .expect("attempt");
    let source = root.path().join(format!(
        ".aria/projects/project_0001/issues/issue_0001/coding-attempts/{}.json",
        attempt.id
    ));
    let target = root.path().join(format!(
        ".aria/projects/project_0001/issues/issue_0002/coding-attempts/{}.json",
        attempt.id
    ));
    std::fs::create_dir_all(target.parent().expect("parent")).expect("create parent");
    std::fs::copy(source, target).expect("copy mismatched record");

    let (status, body) = request_json(
        app,
        Method::GET,
        &format!(
            "/api/projects/project_0001/issues/issue_0002/coding-attempts/{}",
            attempt.id
        ),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["code"], "coding_attempt_scope_mismatch");
}

#[tokio::test]
async fn legacy_coding_attempt_api_loads_unique_valid_attempt() {
    let root = tempdir().expect("root");
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let mut attempt = store
        .create_attempt(CreateCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            work_item_id: "work_item_0001".to_string(),
            base_branch: "main".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: None,
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Fake,
                reviewer: Some(ProviderName::Fake),
                review_rounds: 1,
                permission_modes: cadence_aria::product::models::WorkspaceRolePermissionModes::default(),
            },
            target_snapshot: None,
            max_auto_rework: 2,
        })
        .expect("attempt");
    let generated_id = attempt.id.clone();
    store
        .delete_attempt(&attempt.project_id, &attempt.issue_id, &generated_id)
        .expect("delete generated attempt");
    attempt.id = "coding_attempt_legacy".to_string();
    store
        .update_attempt_non_status_fields(&attempt)
        .expect("save legacy attempt");

    let (status, body) = request_json(
        app,
        Method::GET,
        "/api/coding-attempts/coding_attempt_legacy",
        json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["attempt"]["project_id"], "project_0001");
    assert_eq!(body["attempt"]["issue_id"], "issue_0001");
    assert_eq!(body["attempt"]["attempt_id"], "coding_attempt_legacy");
}

#[tokio::test]
async fn legacy_coding_attempt_api_reports_scope_mismatch_for_unique_corrupt_alias() {
    let root = tempdir().expect("root");
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            project_id: "project_0002".to_string(),
            issue_id: "issue_0002".to_string(),
            work_item_id: "work_item_0002".to_string(),
            base_branch: "main".to_string(),
            branch_name: "aria/issues/issue_0002".to_string(),
            worktree_path: None,
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Fake,
                reviewer: Some(ProviderName::Fake),
                review_rounds: 1,
                permission_modes: cadence_aria::product::models::WorkspaceRolePermissionModes::default(),
            },
            target_snapshot: None,
            max_auto_rework: 2,
        })
        .expect("real attempt");
    let corrupt_alias_id = "coding_attempt_corrupt_alias";
    let corrupt_alias_path = root.path().join(format!(
        ".aria/projects/project_0001/issues/issue_0001/coding-attempts/{corrupt_alias_id}.json"
    ));
    fs::create_dir_all(corrupt_alias_path.parent().expect("parent"))
        .expect("create alias parent");
    fs::write(
        corrupt_alias_path,
        serde_json::to_vec_pretty(&attempt).expect("serialize attempt"),
    )
    .expect("write corrupt alias");

    let (status, body) = request_json(
        app,
        Method::GET,
        &format!("/api/coding-attempts/{corrupt_alias_id}"),
        json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["code"], "coding_attempt_scope_mismatch");
}

#[tokio::test]
async fn legacy_delete_rejects_corrupt_alias_and_preserves_real_attempt() {
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
    assert_eq!(create_status, StatusCode::OK);
    let real_attempt_id = assert_global_attempt_id(&created);
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let real_attempt = store
        .get_attempt("project_0001", "issue_0001", &real_attempt_id)
        .expect("real attempt");
    let corrupt_alias_id = "coding_attempt_corrupt_alias";
    let corrupt_alias_path = root.path().join(format!(
        ".aria/projects/project_0001/issues/issue_0002/coding-attempts/{corrupt_alias_id}.json"
    ));
    fs::create_dir_all(corrupt_alias_path.parent().expect("parent"))
        .expect("create alias parent");
    fs::write(
        &corrupt_alias_path,
        serde_json::to_vec_pretty(&real_attempt).expect("serialize attempt"),
    )
    .expect("write corrupt alias");

    let (status, body) = request_json(
        app,
        Method::DELETE,
        &format!("/api/coding-attempts/{corrupt_alias_id}"),
        json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["code"], "coding_attempt_scope_mismatch");
    assert_eq!(
        store
            .get_attempt("project_0001", "issue_0001", &real_attempt_id)
            .expect("real attempt remains"),
        real_attempt
    );
    assert!(corrupt_alias_path.is_file());
}
