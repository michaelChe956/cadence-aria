// Task 4 legacy 删除路径门禁 + 清理覆盖。
//
// 旧语义：legacy plan 删除会级联中止并清理下属 coding attempt。
// 新语义（spec `harden-work-item-group-deletion`）：存在 coding workspace 时拒绝删除，
// 要求用户先删 coding workspace。下面四个测试覆盖 legacy 路径的：
//   - 工作项级门禁（plan 遍历删除下属 work item 时命中 attempt → 拒绝）
//   - plan 级 group attempt 门禁（plan 上挂了 group attempt → 拒绝）
//   - 独立 work item 删除入口的 attempt 门禁
//   - 无 attempt 时仍级联清理 plan store 产物（保留原覆盖，不编码旧行为）

/// 构造一个 legacy work item plan（无 schema v2 lineage）并返回 issue_root，
/// 供需要直接播种文件系统产物的测试使用。
async fn bootstrap_legacy_plan_with_session(app: axum::Router, repo_path: &std::path::Path) {
    bootstrap_confirmed_work_item(app.clone(), repo_path).await;
    let app_paths = ProductAppPaths::new(workspace_root_from_app(app).await.join(".aria"));
    let lifecycle_store = LifecycleStore::new(app_paths);
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
        .expect("create legacy work item plan");
    lifecycle_store
        .create_workspace_session(CreateWorkspaceSessionInput { project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
        entity_id: "issue_work_item_plan_0001".to_string(),
        workspace_type: WorkspaceType::WorkItemPlan,
        author_provider: ProviderName::Fake,
        reviewer_provider: ProviderName::Fake,
        review_rounds: 1,
        superpowers_enabled: false, openspec_enabled: false, work_item_plan_options: None, })
        .expect("create work item plan session");
}

#[tokio::test]
async fn delete_work_item_plan_legacy_rejected_when_work_item_has_attempt() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    bootstrap_legacy_plan_with_session(app.clone(), repo.path()).await;
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle_store = LifecycleStore::new(app_paths.clone());
    let coding_store = CodingAttemptStore::new(app_paths.clone());

    // 给 plan 下属的 work_item_0001 创建 single coding attempt。
    let (create_status, created) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0001/coding-attempts",
        json!({}),
    )
    .await;
    assert_eq!(create_status, StatusCode::OK, "attempt create: {created}");
    let attempt_id = assert_global_attempt_id(&created);

    // 新语义：有 coding workspace 必须拒绝删除 plan。
    let (status, body) = request_json(
        app.clone(),
        Method::DELETE,
        "/api/projects/project_0001/issues/issue_0001/work-item-plans/issue_work_item_plan_0001",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "delete body: {body}");
    assert_eq!(body["code"], "coding_workspace_exists");
    assert_eq!(body["details"]["work_item_id"], "work_item_0001");
    assert_eq!(body["details"]["attempt_id"], attempt_id);

    // 拒绝时 plan、work item 与 attempt 都必须原样保留。
    assert!(
        lifecycle_store
            .get_issue_work_item_plan("project_0001", "issue_0001", "issue_work_item_plan_0001")
            .is_ok(),
        "plan must remain after rejection"
    );
    assert!(
        !coding_store
            .list_attempts_for_work_item("project_0001", "issue_0001", "work_item_0001")
            .expect("list attempts")
            .is_empty(),
        "attempt must remain after rejection"
    );
}

#[tokio::test]
async fn delete_work_item_plan_legacy_rejected_when_group_attempt_bound() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    bootstrap_legacy_plan_with_session(app.clone(), repo.path()).await;
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle_store = LifecycleStore::new(app_paths.clone());
    let coding_store = CodingAttemptStore::new(app_paths.clone());

    // 直接在 store 层为本 legacy plan 绑定一个 group coding attempt
    // （HTTP group attempt 入口要求 schema v2 lineage，legacy plan 走不通，故用 store 注入）。
    let attempt = coding_store
        .create_group_attempt(CreateGroupCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            plan_id: "issue_work_item_plan_0001".to_string(),
            current_work_item_id: "work_item_0001".to_string(),
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
        .expect("create group attempt");

    // plan 级 group 门禁必须先于遍历拒绝，details 带 plan_id。
    let (status, body) = request_json(
        app.clone(),
        Method::DELETE,
        "/api/projects/project_0001/issues/issue_0001/work-item-plans/issue_work_item_plan_0001",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "delete body: {body}");
    assert_eq!(body["code"], "coding_workspace_exists");
    assert_eq!(body["details"]["plan_id"], "issue_work_item_plan_0001");
    assert_eq!(body["details"]["attempt_id"], attempt.id);

    assert!(
        lifecycle_store
            .get_issue_work_item_plan("project_0001", "issue_0001", "issue_work_item_plan_0001")
            .is_ok(),
        "plan must remain after rejection"
    );
    assert!(
        coding_store
            .get_attempt("project_0001", "issue_0001", &attempt.id)
            .is_ok(),
        "group attempt must remain after rejection"
    );
}

#[tokio::test]
async fn delete_work_item_rejected_when_work_item_has_attempt() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    bootstrap_confirmed_work_item(app.clone(), repo.path()).await;
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let coding_store = CodingAttemptStore::new(app_paths);

    let (create_status, created) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0001/coding-attempts",
        json!({}),
    )
    .await;
    assert_eq!(create_status, StatusCode::OK, "attempt create: {created}");
    let attempt_id = assert_global_attempt_id(&created);

    // 独立删除 work item 入口同样受门禁约束。
    let (status, body) = request_json(
        app,
        Method::DELETE,
        "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0001",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "delete body: {body}");
    assert_eq!(body["code"], "coding_workspace_exists");
    assert_eq!(body["details"]["work_item_id"], "work_item_0001");
    assert_eq!(body["details"]["attempt_id"], attempt_id);
    assert!(
        !coding_store
            .list_attempts_for_work_item("project_0001", "issue_0001", "work_item_0001")
            .expect("list attempts")
            .is_empty(),
        "attempt must remain after rejection"
    );
}

#[tokio::test]
async fn delete_work_item_plan_legacy_cascades_plan_store_artifacts_without_attempt() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    bootstrap_legacy_plan_with_session(app.clone(), repo.path()).await;
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle_store = LifecycleStore::new(app_paths.clone());

    // 播种 plan store 产物：真实流程里 outline 生成会写下 context index，
    // 若删除路径不清理它，plan 删除后会留下指向不存在 plan 的孤儿记录。
    let plan_store = cadence_aria::product::work_item_plan_store::WorkItemPlanStore::new(
        app_paths.clone(),
    );
    plan_store
        .save_outline_context_index(&cadence_aria::product::models::OutlineContextIndex {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            plan_id: "issue_work_item_plan_0001".to_string(),
            generation_round_id: "outline_stage".to_string(),
            blocker_resolutions: Vec::new(),
            design_context_gaps: vec!["missing_architecture".to_string()],
            design_context_capabilities: cadence_aria::product::models::DesignContextCapabilities {
                has_architecture: false,
                has_module_breakdown: false,
                has_tech_stack: false,
                has_test_strategy: false,
                has_key_paths: false,
            },
            updated_at: "2026-07-29T00:00:00Z".to_string(),
        })
        .expect("seed outline context index");

    // 无 attempt → 门禁放行 → 级联清理。
    let (status, body) = request_json(
        app,
        Method::DELETE,
        "/api/projects/project_0001/issues/issue_0001/work-item-plans/issue_work_item_plan_0001",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "delete body: {body}");
    assert_eq!(body["status"], "deleted");

    assert!(
        lifecycle_store
            .get_issue_work_item_plan("project_0001", "issue_0001", "issue_work_item_plan_0001")
            .is_err(),
        "plan record must be deleted"
    );

    // plan store 的产物（outline context index、draft、compile transaction）必须一并清理。
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
            "deleting a legacy plan must purge its plan-store artifacts; {orphan} still exists"
        );
    }
}
