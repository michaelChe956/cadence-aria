pub(crate) async fn bootstrap_story_and_design(app: axum::Router, repo_path: &std::path::Path) {
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects",
        json!({"name":"Coding","description":null}),
    )
    .await;
    let repository = register_repository_and_wait(
        app.clone(),
        json!({"name":"Repo","path":repo_path}),
    )
    .await;
    assert_eq!(repository["repository_id"], "repository_0001");
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues",
        json!({"title":"爬楼梯","description":"实现 O(n) 算法","repository_id":"repository_0001"}),
    )
    .await;
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/story-specs:generate",
        json!({"title":"爬楼梯 Story","author_provider":"fake","reviewer_provider":"fake"}),
    )
    .await;
    request_json(
        app.clone(),
        Method::POST,
        "/api/workspace-sessions/workspace_session_0001/confirm",
        json!({"confirmed_by":"human"}),
    )
    .await;
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/design-specs:generate",
        json!({
            "title":"爬楼梯 Design",
            "story_spec_ids":["story_spec_0001"],
            "author_provider":"fake",
            "reviewer_provider":"fake"
        }),
    )
    .await;
    request_json(
        app.clone(),
        Method::POST,
        "/api/workspace-sessions/workspace_session_0002/confirm",
        json!({"confirmed_by":"human"}),
    )
    .await;
}

async fn register_repository_and_wait(app: axum::Router, request: Value) -> Value {
    let (status, accepted) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/repositories",
        request,
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{accepted}");

    let operation_id = accepted["operation_id"].as_str().expect("operation id");
    let operation_uri = format!(
        "/api/projects/project_0001/repository-initializations/{operation_id}"
    );
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        let (status, snapshot) = request_json(
            app.clone(),
            Method::GET,
            &operation_uri,
            json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{snapshot}");
        match snapshot["status"].as_str() {
            Some("completed") => return snapshot["result"]["repository"].clone(),
            Some("failed") => panic!("repository initialization failed: {snapshot}"),
            _ => {
                if tokio::time::Instant::now() >= deadline {
                    panic!("repository initialization did not finish: {snapshot}");
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        }
    }
}

pub(crate) async fn request_json(
    app: axum::Router,
    method: Method,
    uri: &str,
    body: Value,
) -> (StatusCode, Value) {
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("request");
    let response = app.oneshot(request).await.expect("response");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, value)
}

pub(crate) fn assert_global_attempt_id(value: &Value) -> String {
    let id = value["attempt_id"].as_str().expect("attempt id");
    let uuid = id.strip_prefix("coding_attempt_").expect("attempt prefix");
    assert_eq!(uuid.len(), 32);
    uuid::Uuid::parse_str(uuid).expect("valid attempt UUID");
    id.to_string()
}

pub(crate) fn scoped_attempt_uri(attempt_id: &str, suffix: &str) -> String {
    format!(
        "/api/projects/project_0001/issues/issue_0001/coding-attempts/{attempt_id}{suffix}"
    )
}

pub(crate) fn inject_invalid_group_second_work_item(
    app_paths: &ProductAppPaths,
) -> (PathBuf, Vec<u8>) {
    let projection_path = app_paths
        .issue_root("project_0001", "issue_0001")
        .join(
            "work-item-revisions/work_item_plan_0001/plan-projection-bundles/plan_projection_bundle_0001.json",
        );
    let original_projection = fs::read(&projection_path).expect("plan projection bundle");
    let mut invalid_projection: Value =
        serde_json::from_slice(&original_projection).expect("parse plan projection bundle");
    invalid_projection["coder_group_context"]["ordered_logical_work_item_ids"][1] =
        json!("../invalid_work_item");
    fs::write(
        &projection_path,
        serde_json::to_vec_pretty(&invalid_projection)
            .expect("serialize invalid plan projection bundle"),
    )
    .expect("write invalid plan projection bundle");
    (projection_path, original_projection)
}

pub(crate) fn restore_group_second_work_item(
    fixture: (PathBuf, Vec<u8>),
) {
    let (projection_path, original_projection) = fixture;
    fs::write(projection_path, original_projection).expect("restore plan projection bundle");
}

pub(crate) fn assert_group_attempt_creation_rolled_back(app_paths: &ProductAppPaths) {
    let coding_store = CodingAttemptStore::new(app_paths.clone());
    assert!(
        coding_store
            .list_attempts_for_work_item("project_0001", "issue_0001", "work_item_0001")
            .expect("list attempts after failed create")
            .is_empty()
    );
    let issue_root = app_paths.issue_lifecycle_root("project_0001", "issue_0001");
    let attempts_root = issue_root.join("coding-attempts");
    if attempts_root.exists() {
        assert_eq!(
            fs::read_dir(attempts_root)
                .expect("coding attempts root")
                .count(),
            0
        );
    }
    let lifecycle = LifecycleStore::new(app_paths.clone());
    let shared_worktree = lifecycle
        .get_issue_shared_worktree("project_0001", "issue_0001")
        .expect("shared worktree");
    if let Some(shared_worktree) = shared_worktree {
        assert_eq!(shared_worktree.current_active_work_item_id, None);
    }
}

pub(crate) fn git_repo() -> tempfile::TempDir {
    let dir = tempdir().expect("repo");
    run_git(dir.path(), &["init"]);
    run_git(dir.path(), &["config", "user.email", "aria@example.com"]);
    run_git(dir.path(), &["config", "user.name", "Aria Test"]);
    fs::write(dir.path().join("README.md"), "# repo\n").expect("seed readme");
    run_git(dir.path(), &["add", "."]);
    run_git(dir.path(), &["commit", "-m", "initial"]);
    dir
}

pub(crate) fn run_git(cwd: &std::path::Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .status()
        .expect("git");
    assert!(status.success());
}

pub(crate) fn prepare_attempt_with_worktree(
    store: &CodingAttemptStore,
    repo_path: &std::path::Path,
    project_id: &str,
    issue_id: &str,
    attempt_id: &str,
) -> CodingExecutionAttempt {
    let attempt = store
        .get_attempt(project_id, issue_id, attempt_id)
        .expect("attempt");
    if !branch_exists(repo_path, &attempt.branch_name) {
        run_git(repo_path, &["branch", &attempt.branch_name, "HEAD"]);
    }
    let worktree_path = if let Some(issue_id) = attempt.branch_name.strip_prefix("aria/issues/") {
        repo_path
            .join(".worktrees")
            .join("aria-issues")
            .join(issue_id)
    } else {
        repo_path
            .join(".worktrees")
            .join("aria-work-items")
            .join(&attempt.work_item_id)
            .join(format!("attempt-{}", attempt.attempt_no))
    };
    if !worktree_path.exists() {
        run_git(
            repo_path,
            &[
                "worktree",
                "add",
                worktree_path.to_str().expect("worktree path"),
                &attempt.branch_name,
            ],
        );
    }
    store
        .update_attempt_worktree_path(project_id, issue_id, attempt_id, worktree_path)
        .expect("update worktree path")
}

fn branch_exists(repo_path: &std::path::Path, branch_name: &str) -> bool {
    Command::new("git")
        .args([
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/heads/{branch_name}"),
        ])
        .current_dir(repo_path)
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}
