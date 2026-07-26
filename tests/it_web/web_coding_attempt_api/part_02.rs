#[tokio::test]
async fn repeated_group_coding_attempt_create_returns_original_attempt() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    bootstrap_confirmed_work_item_plan_group(app.clone(), repo.path()).await;
    let path = "/api/projects/project_0001/issues/issue_0001/work-item-plans/work_item_plan_0001/coding-attempts";

    let (first_status, first) =
        request_json(app.clone(), Method::POST, path, json!({})).await;
    let (second_status, second) =
        request_json(app.clone(), Method::POST, path, json!({})).await;

    assert_eq!(first_status, StatusCode::OK);
    let attempt_id = assert_global_attempt_id(&first);
    assert_eq!(second_status, StatusCode::OK);
    assert_eq!(second["attempt_id"], first["attempt_id"]);
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    assert_eq!(
        store
            .list_coding_units(
                "project_0001",
                "issue_0001",
                &attempt_id,
            )
            .expect("units")
            .len(),
        2,
    );
}

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
        &coding_store,
        repo.path(),
        "project_0001",
        "issue_0001",
        &attempt_id,
    );
    let artifact_dir = coding_store.attempt_test_output_root(
        "project_0001",
        "issue_0001",
        &attempt_id,
    );
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
    let attempt_id = assert_global_attempt_id(&attempt);
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let artifact_dir = store.attempt_test_output_root(
        "project_0001",
        "issue_0001",
        &attempt_id,
    );
    fs::create_dir_all(&artifact_dir).expect("artifact dir");
    fs::write(artifact_dir.join("unit.stdout.log"), "unit stdout\n").expect("artifact");

    let (status, artifact) = request_json(
        app,
        Method::GET,
        &scoped_attempt_uri(&attempt_id, "/artifacts/unit.stdout.log"),
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
    let attempt_id = assert_global_attempt_id(&created);
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = prepare_attempt_with_worktree(
        &store,
        repo.path(),
        "project_0001",
        "issue_0001",
        &attempt_id,
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
        &scoped_attempt_uri(&attempt_id, "/diff"),
        json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(diff["attempt_id"], attempt_id);
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
    let lifecycle = LifecycleStore::new(app_paths.clone());
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
    let lifecycle = LifecycleStore::new(app_paths.clone());
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
    let repository = register_repository_and_wait(
        app.clone(),
        json!({
            "name":"Repo",
            "path":repo_path,
            "default_provider_mode": default_provider_mode
        }),
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

async fn bootstrap_confirmed_work_item_plan_group(app: axum::Router, repo_path: &std::path::Path) {
    bootstrap_work_item_plan_group(app, repo_path, IssueWorkItemPlanStatus::Confirmed).await;
}

async fn bootstrap_draft_work_item_plan_group(app: axum::Router, repo_path: &std::path::Path) {
    bootstrap_work_item_plan_group(app, repo_path, IssueWorkItemPlanStatus::Draft).await;
}

async fn bootstrap_work_item_plan_group(
    app: axum::Router,
    repo_path: &std::path::Path,
    plan_status: IssueWorkItemPlanStatus,
) {
    bootstrap_story_and_design(app.clone(), repo_path).await;
    let app_paths = ProductAppPaths::new(workspace_root_from_app(app).await.join(".aria"));
    let lifecycle = LifecycleStore::new(app_paths.clone());

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
        .expect("create verification plan 1");
    lifecycle
        .create_verification_plan(CreateVerificationPlanInput {
            id: Some("verification_plan_0002".to_string()),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            work_item_id: "work_item_0002".to_string(),
            repository_profile_ref: None,
            provider_run_ref: None,
            scope: VerificationScope::Unit,
            commands: vec![VerificationCommand {
                id: "cmd_002".to_string(),
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
        .expect("create verification plan 2");

    lifecycle
        .create_work_item(CreateWorkItemInput {
            id: Some("work_item_0001".to_string()),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            story_spec_ids: vec!["story_spec_0001".to_string()],
            design_spec_ids: vec!["design_spec_0001".to_string()],
            title: "实现爬楼梯".to_string(),
            work_item_set_id: Some("work_item_plan_0001".to_string()),
            sequence_hint: Some(10),
            verification_plan_ref: Some("verification_plan_0001".to_string()),
            plan_status: WorkItemPlanStatus::Confirmed,
            ..Default::default()
        })
        .expect("create group item1");
    lifecycle
        .create_work_item(CreateWorkItemInput {
            id: Some("work_item_0002".to_string()),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            story_spec_ids: vec!["story_spec_0001".to_string()],
            design_spec_ids: vec!["design_spec_0001".to_string()],
            title: "实现爬楼梯 part 2".to_string(),
            work_item_set_id: Some("work_item_plan_0001".to_string()),
            kind: WorkItemKind::Backend,
            sequence_hint: Some(20),
            depends_on: vec!["work_item_0001".to_string()],
            exclusive_write_scopes: vec!["src/".to_string()],
            verification_plan_ref: Some("verification_plan_0002".to_string()),
            plan_status: WorkItemPlanStatus::Confirmed,
            ..Default::default()
        })
        .expect("create group item2");

    lifecycle
        .create_issue_work_item_plan(CreateIssueWorkItemPlanInput {
            id: Some("work_item_plan_0001".to_string()),
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
            status: plan_status,
            work_item_ids: vec!["work_item_0001".to_string(), "work_item_0002".to_string()],
            repository_profile_ref: None,
            verification_plan_ids: vec![
                "verification_plan_0001".to_string(),
                "verification_plan_0002".to_string(),
            ],
            dependency_graph: vec![cadence_aria::product::models::IssueWorkItemDependencyEdge {
                from_work_item_id: "work_item_0001".to_string(),
                to_work_item_id: "work_item_0002".to_string(),
            }],
            created_from_provider_run: None,
            validator_findings: Vec::new(),
        })
        .expect("create work item plan group");

    seed_group_plan_revision(&app_paths);
}

fn seed_group_plan_revision(app_paths: &ProductAppPaths) {
    let store = WorkItemRevisionStore::new(app_paths.clone());
    let lineage = WorkItemPlanLineage {
        id: "work_item_plan_0001".to_string(),
        project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
        story_spec_refs: vec!["story_spec_0001".to_string()],
        design_spec_refs: vec!["design_spec_0001".to_string()],
        active_revision_id: None,
        active_amendment_id: None,
        created_at: "2026-07-18T00:00:00Z".to_string(),
        updated_at: "2026-07-18T00:00:00Z".to_string(),
    };
    store.put_plan_lineage(&lineage).expect("plan lineage");
    seed_group_work_item_revisions(&store, &lineage);
    let dependency = DependencyGraphRevision {
        id: "dependency_graph_revision_0001".to_string(),
        plan_id: lineage.id.clone(),
        edges: vec![DependencyContractEdge {
            from: "work_item_0001".to_string(),
            to: "work_item_0002".to_string(),
            required_contracts: Vec::new(),
        }],
        created_at: "2026-07-18T00:00:00Z".to_string(),
    };
    store
        .put_dependency_graph_revision(&lineage, &dependency)
        .expect("dependency graph");
    let revision = WorkItemPlanRevision {
        id: "plan_revision_0001".to_string(),
        plan_id: lineage.id.clone(),
        revision_no: 1,
        supersedes: None,
        reason: PlanRevisionReason::InitialCompile,
        work_item_bindings: BTreeMap::from([
            (
                "work_item_0001".to_string(),
                "work_item_revision_0001".to_string(),
            ),
            (
                "work_item_0002".to_string(),
                "work_item_revision_0002".to_string(),
            ),
        ]),
        dependency_graph_revision_id: dependency.id,
        validation_report_ref: "plan-validation-report.json".to_string(),
        plan_projection_bundle_id: "plan_projection_bundle_0001".to_string(),
        created_at: "2026-07-18T00:00:00Z".to_string(),
    };
    store.put_plan_revision(&lineage, &revision).expect("plan revision");
    store
        .set_active_plan_revision(&lineage, &revision.id)
        .expect("active plan revision");
}
