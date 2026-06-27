#[tokio::test]
async fn delete_work_item_plan_cascades_children_sessions_and_attempts() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    bootstrap_confirmed_work_item(app.clone(), repo.path()).await;
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle_store = LifecycleStore::new(app_paths.clone());
    let coding_store = CodingAttemptStore::new(app_paths);

    lifecycle_store
        .create_issue_work_item_plan(CreateIssueWorkItemPlanInput {
            id: Some("issue_work_item_plan_0001".to_string()),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            source_story_spec_ids: vec!["story_spec_0001".to_string()],
            source_design_spec_ids: vec!["design_spec_0001".to_string()],
            options: IssueWorkItemPlanOptions {
                include_integration_tests: true,
                include_e2e_tests: false,
                force_frontend_backend_split: true,
                require_execution_plan_confirm: false,
            },
            status: IssueWorkItemPlanStatus::Confirmed,
            work_item_ids: vec!["work_item_0001".to_string()],
            repository_profile_ref: None,
            verification_plan_ids: vec!["verification_plan_0001".to_string()],
            dependency_graph: Vec::new(),
            created_from_provider_run: None,
            validator_findings: Vec::new(),
        })
        .expect("create work item plan");
    lifecycle_store
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: "issue_work_item_plan_0001".to_string(),
            workspace_type: WorkspaceType::WorkItemPlan,
            author_provider: ProviderName::Fake,
            reviewer_provider: ProviderName::Fake,
            review_rounds: 1,
            superpowers_enabled: false,
            openspec_enabled: false,
        })
        .expect("create work item plan session");

    let (status, _) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0001/coding-attempts",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let attempt = prepare_attempt_with_worktree(
        &coding_store,
        repo.path(),
        "project_0001",
        "issue_0001",
        "coding_attempt_0001",
    );
    let artifact_dir =
        coding_store.attempt_test_output_root("project_0001", "issue_0001", "coding_attempt_0001");
    fs::create_dir_all(&artifact_dir).expect("artifact dir");
    fs::write(artifact_dir.join("unit.stdout.log"), "unit\n").expect("artifact");
    let attempt_dir = artifact_dir
        .parent()
        .and_then(|path| path.parent())
        .expect("attempt dir")
        .to_path_buf();

    let (status, body) = request_json(
        app.clone(),
        Method::DELETE,
        "/api/projects/project_0001/issues/issue_0001/work-item-plans/issue_work_item_plan_0001",
        json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "deleted");
    assert!(!attempt_dir.exists());
    assert!(
        !attempt
            .worktree_path
            .as_ref()
            .expect("attempt worktree")
            .exists()
    );
    assert!(!branch_exists(repo.path(), &attempt.branch_name));
    assert!(
        lifecycle_store
            .get_issue_work_item_plan("project_0001", "issue_0001", "issue_work_item_plan_0001")
            .is_err()
    );
    assert!(
        lifecycle_store
            .get_verification_plan("project_0001", "issue_0001", "verification_plan_0001")
            .is_err()
    );
    assert!(
        coding_store
            .list_attempts_for_work_item("project_0001", "issue_0001", "work_item_0001")
            .expect("list attempts")
            .is_empty()
    );

    let (status, lifecycle) = request_json(
        app,
        Method::GET,
        "/api/issues/issue_0001/lifecycle?project_id=project_0001",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(lifecycle["work_item_plans"].as_array().unwrap().is_empty());
    assert!(lifecycle["work_items"].as_array().unwrap().is_empty());
    assert!(lifecycle["coding_attempts"].as_array().unwrap().is_empty());
    assert!(
        lifecycle["workspace_sessions"]
            .as_array()
            .unwrap()
            .iter()
            .all(
                |session| session["entity_id"] != "issue_work_item_plan_0001"
                    && session["entity_id"] != "work_item_0001"
            )
    );
}

#[tokio::test]
async fn reads_test_output_artifact_from_attempt_store() {
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
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let artifact_dir = store.attempt_test_output_root(
        "project_0001",
        "issue_0001",
        attempt["attempt_id"].as_str().expect("attempt id"),
    );
    fs::create_dir_all(&artifact_dir).expect("artifact dir");
    fs::write(artifact_dir.join("unit.stdout.log"), "unit stdout\n").expect("artifact");

    let (status, artifact) = request_json(
        app,
        Method::GET,
        "/api/coding-attempts/coding_attempt_0001/artifacts/unit.stdout.log",
        json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(artifact["artifact_ref"], "unit.stdout.log");
    assert_eq!(artifact["artifact_kind"], "coding_attempt_artifact");
    assert_eq!(artifact["content_type"], "text/plain");
    assert_eq!(artifact["content"], "unit stdout\n");
}

#[tokio::test]
async fn reads_coding_attempt_diff_from_worktree_against_base_branch() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    bootstrap_confirmed_work_item(app.clone(), repo.path()).await;
    let (status, created) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0001/coding-attempts",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = prepare_attempt_with_worktree(
        &store,
        repo.path(),
        "project_0001",
        "issue_0001",
        "coding_attempt_0001",
    );
    let worktree_path = attempt.worktree_path.as_ref().expect("worktree path");
    fs::write(
        worktree_path.join("climbing_stairs.py"),
        "def climb_stairs(n):\n    return n\n",
    )
    .expect("write changed file");

    let (status, diff) = request_json(
        app,
        Method::GET,
        "/api/coding-attempts/coding_attempt_0001/diff",
        json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(diff["attempt_id"], "coding_attempt_0001");
    assert_eq!(diff["base_branch"], created["base_branch"]);
    assert_eq!(
        diff["worktree_path"],
        worktree_path.to_string_lossy().to_string()
    );
    let content = diff["diff"].as_str().expect("diff content");
    assert!(content.contains("diff --git"));
    assert!(content.contains("climbing_stairs.py"));
    assert!(content.contains("+def climb_stairs(n):"));
}

async fn workspace_root_from_app(app: axum::Router) -> std::path::PathBuf {
    let (status, info) = request_json(app, Method::GET, "/api/runtime-info", json!({})).await;
    assert_eq!(status, StatusCode::OK);
    info["workspace_root"]
        .as_str()
        .expect("workspace_root")
        .into()
}

fn provider_name_from_str(name: &str) -> ProviderName {
    match name {
        "fake" => ProviderName::Fake,
        "codex" => ProviderName::Codex,
        "claude_code" => ProviderName::ClaudeCode,
        _ => ProviderName::Fake,
    }
}

async fn bootstrap_confirmed_work_item(app: axum::Router, repo_path: &std::path::Path) {
    bootstrap_confirmed_work_item_with_providers(app, repo_path, "fake", "fake").await;
}

async fn bootstrap_confirmed_work_item_with_providers(
    app: axum::Router,
    repo_path: &std::path::Path,
    work_item_author_provider: &str,
    work_item_reviewer_provider: &str,
) {
    bootstrap_story_and_design(app.clone(), repo_path).await;
    let app_paths = ProductAppPaths::new(workspace_root_from_app(app.clone()).await.join(".aria"));
    let lifecycle = LifecycleStore::new(app_paths);
    lifecycle
        .create_verification_plan(CreateVerificationPlanInput {
            id: Some("verification_plan_0001".to_string()),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            work_item_id: "work_item_0001".to_string(),
            repository_profile_ref: None,
            provider_run_ref: None,
            scope: VerificationScope::Unit,
            commands: vec![VerificationCommand {
                id: "cmd_001".to_string(),
                label: "cargo test".to_string(),
                command: "cargo test --lib".to_string(),
                cwd: String::new(),
                purpose: "unit tests".to_string(),
                required: true,
                timeout_seconds: 120,
                source: VerificationCommandSource::Provider,
                safety: VerificationCommandSafety::Approved,
            }],
            manual_checks: Vec::new(),
            required_gates: vec!["unit_tests".to_string()],
            risk_notes: Vec::new(),
            confidence: RepositoryProfileConfidence::High,
            fallback_policy: VerificationFallbackPolicy::ManualGate,
        })
        .expect("create verification plan");

    lifecycle
        .create_work_item(CreateWorkItemInput {
            id: Some("work_item_0001".to_string()),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            story_spec_ids: vec!["story_spec_0001".to_string()],
            design_spec_ids: vec!["design_spec_0001".to_string()],
            title: "实现爬楼梯".to_string(),
            verification_plan_ref: Some("verification_plan_0001".to_string()),
            plan_status: WorkItemPlanStatus::Confirmed,
            ..Default::default()
        })
        .expect("create work item");

    let author_provider = provider_name_from_str(work_item_author_provider);
    let reviewer_provider = provider_name_from_str(work_item_reviewer_provider);
    let session = lifecycle
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: "work_item_0001".to_string(),
            workspace_type: WorkspaceType::WorkItem,
            author_provider,
            reviewer_provider,
            review_rounds: 1,
            superpowers_enabled: false,
            openspec_enabled: false,
        })
        .expect("create work item session");
    lifecycle
        .update_workspace_session_status(&session.id, WorkspaceSessionStatus::Confirmed)
        .expect("confirm work item session");
}

async fn bootstrap_unconfirmed_work_item(app: axum::Router, repo_path: &std::path::Path) {
    bootstrap_story_and_design(app.clone(), repo_path).await;
    let app_paths = ProductAppPaths::new(workspace_root_from_app(app.clone()).await.join(".aria"));
    let lifecycle = LifecycleStore::new(app_paths);
    lifecycle
        .create_work_item(CreateWorkItemInput {
            id: Some("work_item_0001".to_string()),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            story_spec_ids: vec!["story_spec_0001".to_string()],
            design_spec_ids: vec!["design_spec_0001".to_string()],
            title: "实现爬楼梯".to_string(),
            plan_status: WorkItemPlanStatus::Draft,
            ..Default::default()
        })
        .expect("create work item");
}

async fn bootstrap_confirmed_work_item_without_workspace_session(
    app: axum::Router,
    root_path: &std::path::Path,
    repo_path: &std::path::Path,
    default_provider_mode: &str,
) {
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects",
        json!({"name":"Coding","description":null}),
    )
    .await;
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/repositories",
        json!({
            "name":"Repo",
            "path":repo_path,
            "default_provider_mode": default_provider_mode
        }),
    )
    .await;
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues",
        json!({"title":"爬楼梯","description":"实现 O(n) 算法","repository_id":"repository_0001"}),
    )
    .await;

    let app_paths = ProductAppPaths::new(root_path.join(".aria"));
    let lifecycle = LifecycleStore::new(app_paths);
    lifecycle
        .create_work_item(CreateWorkItemInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            story_spec_ids: Vec::new(),
            design_spec_ids: Vec::new(),
            title: "实现爬楼梯".to_string(),
            ..Default::default()
        })
        .expect("create work item");
    lifecycle
        .update_work_item_plan_status(
            "project_0001",
            "issue_0001",
            "work_item_0001",
            WorkItemPlanStatus::Confirmed,
        )
        .expect("confirm work item");
}

async fn bootstrap_confirmed_split_work_items(
    app: axum::Router,
    root_path: &std::path::Path,
    repo_path: &std::path::Path,
) {
    bootstrap_story_and_design(app.clone(), repo_path).await;
    let app_paths = ProductAppPaths::new(root_path.join(".aria"));
    let lifecycle = LifecycleStore::new(app_paths);
    lifecycle
        .create_work_item(CreateWorkItemInput {
            id: Some("work_item_0001".to_string()),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            story_spec_ids: vec!["story_spec_0001".to_string()],
            design_spec_ids: vec!["design_spec_0001".to_string()],
            title: "实现爬楼梯".to_string(),
            plan_status: WorkItemPlanStatus::Confirmed,
            ..Default::default()
        })
        .expect("create item1");
    lifecycle
        .create_work_item(CreateWorkItemInput {
            id: Some("work_item_0002".to_string()),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            story_spec_ids: vec!["story_spec_0001".to_string()],
            design_spec_ids: vec!["design_spec_0001".to_string()],
            title: "实现爬楼梯 part 2".to_string(),
            kind: WorkItemKind::Backend,
            sequence_hint: Some(20),
            depends_on: vec!["work_item_0001".to_string()],
            exclusive_write_scopes: vec!["src/".to_string()],
            plan_status: WorkItemPlanStatus::Confirmed,
            ..Default::default()
        })
        .expect("create item2 with dependency");
}

pub(crate) async fn bootstrap_two_ready_confirmed_work_items(
    app: axum::Router,
    root_path: &std::path::Path,
    repo_path: &std::path::Path,
) {
    bootstrap_story_and_design(app.clone(), repo_path).await;
    let app_paths = ProductAppPaths::new(root_path.join(".aria"));
    let lifecycle = LifecycleStore::new(app_paths);
    lifecycle
        .create_work_item(CreateWorkItemInput {
            id: Some("work_item_0001".to_string()),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            story_spec_ids: vec!["story_spec_0001".to_string()],
            design_spec_ids: vec!["design_spec_0001".to_string()],
            title: "实现爬楼梯".to_string(),
            plan_status: WorkItemPlanStatus::Confirmed,
            ..Default::default()
        })
        .expect("create item1");
    lifecycle
        .create_work_item(CreateWorkItemInput {
            id: Some("work_item_0002".to_string()),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            story_spec_ids: vec!["story_spec_0001".to_string()],
            design_spec_ids: vec!["design_spec_0001".to_string()],
            title: "实现爬楼梯 part 2".to_string(),
            kind: WorkItemKind::Backend,
            sequence_hint: Some(20),
            exclusive_write_scopes: vec!["src/".to_string()],
            plan_status: WorkItemPlanStatus::Confirmed,
            ..Default::default()
        })
        .expect("create item2");
}

async fn bootstrap_completed_dependency_without_handoff(
    app: axum::Router,
    root_path: &std::path::Path,
    repo_path: &std::path::Path,
) {
    bootstrap_story_and_design(app.clone(), repo_path).await;
    let app_paths = ProductAppPaths::new(root_path.join(".aria"));
    let lifecycle = LifecycleStore::new(app_paths);
    lifecycle
        .create_work_item(CreateWorkItemInput {
            id: Some("work_item_0001".to_string()),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            story_spec_ids: vec!["story_spec_0001".to_string()],
            design_spec_ids: vec!["design_spec_0001".to_string()],
            title: "实现爬楼梯".to_string(),
            plan_status: WorkItemPlanStatus::Confirmed,
            ..Default::default()
        })
        .expect("create item1");
    lifecycle
        .update_work_item_execution_status(
            "project_0001",
            "issue_0001",
            "work_item_0001",
            WorkItemStatus::Completed,
        )
        .expect("complete item1");

    lifecycle
        .create_work_item(CreateWorkItemInput {
            id: Some("work_item_0002".to_string()),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            story_spec_ids: vec!["story_spec_0001".to_string()],
            design_spec_ids: vec!["design_spec_0001".to_string()],
            title: "实现爬楼梯 part 2".to_string(),
            kind: WorkItemKind::Backend,
            sequence_hint: Some(20),
            depends_on: vec!["work_item_0001".to_string()],
            exclusive_write_scopes: vec!["src/".to_string()],
            required_handoff_from: vec!["work_item_0001".to_string()],
            plan_status: WorkItemPlanStatus::Confirmed,
            ..Default::default()
        })
        .expect("create item2 with handoff dependency");
}

pub(crate) async fn bootstrap_story_and_design(app: axum::Router, repo_path: &std::path::Path) {
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects",
        json!({"name":"Coding","description":null}),
    )
    .await;
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/repositories",
        json!({"name":"Repo","path":repo_path}),
    )
    .await;
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

