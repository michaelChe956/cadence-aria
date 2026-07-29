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

    // 播种 plan store 产物：真实流程里 outline 生成会写下 context index，
    // 若删除路径不清理它，plan 删除后会留下指向不存在 plan 的孤儿记录。
    let plan_store = cadence_aria::product::work_item_plan_store::WorkItemPlanStore::new(
        ProductAppPaths::new(root.path().join(".aria")),
    );
    plan_store
        .save_outline_context_index(&cadence_aria::product::models::OutlineContextIndex {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            plan_id: "issue_work_item_plan_0001".to_string(),
            generation_round_id: "outline_stage".to_string(),
            blocker_resolutions: Vec::new(),
            design_context_gaps: vec!["missing_architecture".to_string()],
            design_context_capabilities:
                cadence_aria::product::models::DesignContextCapabilities {
                    has_architecture: false,
                    has_module_breakdown: false,
                    has_tech_stack: false,
                    has_test_strategy: false,
                    has_key_paths: false,
                },
            updated_at: "2026-07-29T00:00:00Z".to_string(),
        })
        .expect("seed outline context index");

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

    // plan store 的产物（outline context index、draft、compile transaction）必须一并清理。
    // 实测缺陷：删除 plan 后 work_item_plan_outlines/<plan_id>/outline_context_index.json
    // 残留，指向已不存在的 plan；若新 plan 复用同一 ID，会读到上一轮的 design context。
    let issue_root = root
        .path()
        .join(".aria/projects/project_0001/issues/issue_0001");
    for orphan in [
        "work_item_plan_outlines/issue_work_item_plan_0001",
        "work_item_plan_drafts/issue_work_item_plan_0001",
        "work_item_plan_compiles/issue_work_item_plan_0001",
    ] {
        assert!(
            !issue_root.join(orphan).exists(),
            "deleting a work item plan must purge its plan-store artifacts; {orphan} still exists"
        );
    }
}

