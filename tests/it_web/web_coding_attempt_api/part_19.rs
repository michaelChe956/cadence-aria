// Task 16（§4.6 稳定码契约收口）：legacy_shared_worktree_present 经
// CodingWorkspaceEngineError::LegacySharedWorktreePresent 承载，在 abort 端点的
// coding_workspace_api_error 转换点映射为 409 CONFLICT（Task 10 preflight 引入）。

#[tokio::test]
async fn abort_logical_group_attempt_with_legacy_shared_worktree_is_409() {
    use cadence_aria::product::lifecycle_store::UpsertIssueSharedWorktreeInput;

    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    bootstrap_confirmed_work_item_plan_group(app.clone(), repo.path()).await;
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    seed_group_logical_target_fixture(&app_paths, repo.path());

    let (create_status, created) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-item-plans/work_item_plan_0001/coding-attempts",
        json!({}),
    )
    .await;
    assert_eq!(create_status, StatusCode::OK, "create: {created}");
    let attempt_id = assert_global_attempt_id(&created);

    // 迁移 preflight（§4.2.6）：逻辑 attempt 存在 target_snapshot，但同 issue 下出现旧
    // issue-shared-worktree.json → fail-closed legacy_shared_worktree_present。
    LifecycleStore::new(app_paths.clone())
        .upsert_issue_shared_worktree(UpsertIssueSharedWorktreeInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            branch_name: "legacy-branch".to_string(),
            worktree_path: repo.path().join("legacy-worktree"),
            base_branch: "main".to_string(),
        })
        .expect("legacy issue shared worktree");

    let (abort_status, abort_body) = request_json(
        app,
        Method::POST,
        &scoped_attempt_uri(&attempt_id, "/abort"),
        json!({}),
    )
    .await;

    assert_eq!(
        abort_status,
        StatusCode::CONFLICT,
        "abort must fail closed: {abort_body}"
    );
    assert_eq!(abort_body["code"], "legacy_shared_worktree_present");
}
