#[tokio::test]
async fn coding_plan_repair_terminal_group_post_returns_original_aborted_attempt() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    bootstrap_confirmed_work_item_plan_group(app.clone(), repo.path()).await;
    let path = "/api/projects/project_0001/issues/issue_0001/work-item-plans/work_item_plan_0001/coding-attempts";
    let (first_status, first) = request_json(app.clone(), Method::POST, path, json!({})).await;
    assert_eq!(first_status, StatusCode::OK);
    let attempt_id = assert_global_attempt_id(&first);
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt_id,
            CodingAttemptStatus::Running,
        )
        .expect("run group attempt");
    let aborted = store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt_id,
            CodingAttemptStatus::Aborted,
        )
        .expect("abort group attempt");
    assert_eq!(aborted.active_unit_id, None);
    assert_eq!(aborted.current_work_item_id, None);

    let (second_status, second) = request_json(app, Method::POST, path, json!({})).await;

    assert_eq!(second_status, StatusCode::OK);
    assert_eq!(second["attempt_id"], first["attempt_id"]);
    assert_eq!(second["status"], "aborted");
    assert_eq!(second["active_unit_id"], Value::Null);
    assert_eq!(second["current_work_item_id"], Value::Null);
}
