// REQ-COD-03: repository-keyed create must fail closed on a legacy record, then
// explicitly migrated state must permit the new route and use its repository-keyed record.

#[tokio::test]
async fn legacy_shared_worktree_http_preflight_migrates_to_repository_keyed_record() {
    use cadence_aria::product::lifecycle_store::UpsertIssueSharedWorktreeInput;

    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    bootstrap_confirmed_work_item(app.clone(), repo.path()).await;
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    IdentityMigrationExecutor::new(paths.clone())
        .ensure_identity_schema("project_0001")
        .expect("migrate identity schema");
    let repository_id = LogicalCodebaseStore::new(paths.clone())
        .load_manifest("project_0001")
        .expect("load logical manifest")
        .expect("logical manifest")
        .member_ids[0];
    IssueCodebaseSelectionStore::new(paths.clone())
        .save(&IssueCodebaseSelection::explicit(
            "project_0001",
            "issue_0001",
            vec![repository_id],
            Vec::new(),
            vec![repository_id],
            None,
        ))
        .expect("logical issue routing");

    let work_item_path = paths
        .issue_root("project_0001", "issue_0001")
        .join("work-items/work_item_0001.json");
    let mut work_item: cadence_aria::product::models::LifecycleWorkItemRecord =
        cadence_aria::product::json_store::read_json(&work_item_path).expect("work item");
    work_item.target_repository_id = Some(repository_id);
    cadence_aria::product::json_store::write_json(&work_item_path, &work_item)
        .expect("logical work item target");

    let lifecycle = LifecycleStore::new(paths.clone());
    lifecycle
        .upsert_issue_shared_worktree(UpsertIssueSharedWorktreeInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: repo.path().join("legacy-worktree"),
            base_branch: "master".to_string(),
        })
        .expect("legacy worktree");

    let create_path = "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0001/coding-attempts";
    let (status, body) = request_json(app.clone(), Method::POST, create_path, json!({})).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "legacy_shared_worktree_present");
    assert!(
        lifecycle
            .get_repo_shared_worktree("project_0001", "issue_0001", repository_id)
            .expect("new path remains readable")
            .is_none(),
        "preflight failure must not write the repository-keyed record"
    );

    let legacy = LegacySharedWorktreeMigration::load_legacy_shared_worktree(
        &paths,
        "project_0001",
        "issue_0001",
    )
    .expect("read legacy")
    .expect("legacy exists");
    LegacySharedWorktreeMigration::migrate_to_repository_keyed(&paths, legacy)
        .expect("explicit migration");

    let (status, body) = request_json(app, Method::POST, create_path, json!({})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let created = lifecycle
        .get_repo_shared_worktree("project_0001", "issue_0001", repository_id)
        .expect("repository-keyed worktree")
        .expect("new route writes repository-keyed record");
    assert_eq!(created.target_repository_id, Some(repository_id));
}
