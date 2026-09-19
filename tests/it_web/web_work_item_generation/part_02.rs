// 退役留档（T5/REQ-RET-02）：valid_canonical_draft_output 夹具为 staged draft 族测试（serial/batch/compile/
// staged_flow/恢复矩阵）专用，随消息族一并退役。


pub(crate) async fn app_with_confirmed_story_and_design_and_streaming_outputs(
    outputs: Vec<Value>,
) -> (axum::Router, tempfile::TempDir, Arc<Mutex<Vec<String>>>) {
    let root = tempdir().expect("root");
    let repo = root.path().join("repo");
    std::fs::create_dir_all(&repo).expect("create repo dir");
    let status = Command::new("git")
        .args(["init"])
        .current_dir(&repo)
        .status()
        .expect("git init");
    assert!(status.success());

    let runtime = WebRuntime::new_fake(root.path().to_path_buf());
    let mut state = WebAppState::new(root.path().to_path_buf(), runtime).with_provider_adapter(
        Arc::new(MockSplitProviderAdapter {
            output: outputs.first().cloned().unwrap_or_else(valid_split_output),
            revision_output: None,
        }),
    );

    let captured_prompts = Arc::new(Mutex::new(Vec::new()));
    let test_controls = cadence_aria::web::test_controls::TestControls::default();
    let mut registry = ProviderRegistry::new();
    registry.register(
        ProviderName::Fake,
        Arc::new(QueuedSplitStreamingProvider::new_recording(
            outputs,
            captured_prompts.clone(),
        )),
    );
    registry.register(
        ProviderName::Codex,
        Arc::new(TestControlledFakeStreamingProvider::new(
            test_controls.clone(),
        )),
    );
    registry.register(
        ProviderName::ClaudeCode,
        Arc::new(TestControlledFakeStreamingProvider::new(
            test_controls.clone(),
        )),
    );
    state.test_controls = test_controls;
    state.provider_registry = Arc::new(registry);

    let app = build_web_router(state);
    let app = bootstrap_project_repo_issue_and_specs(app, &repo).await;

    (app, root, captured_prompts)
}

// 退役留档（T5/REQ-RET-02）：app_with_confirmed_story_and_design_and_streaming_raw_outputs 夹具
// （raw stdout/挂起注入，驱动 outline structured 解析重试与 pending 场景）随消息族退役。

// 退役留档（T5/REQ-RET-02）：app_with_confirmed_story_and_design_and_test_providers 夹具
// （review verdict 注入）为 legacy review/decision 流测试专用，随消息族退役。

// 退役留档（T5/REQ-RET-02）：app_with_confirmed_story_and_design_revision_and_test_providers 夹具
// （revision+review verdict 注入）为 legacy full-candidate review/revision 流测试专用，随消息族退役。

#[tokio::test]
async fn prepare_work_item_plan_creates_draft_plan_and_session_without_generating() {
    let (app, _repo) = app_with_confirmed_story_and_design(valid_split_output()).await;

    let (status, response) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-item-plans:prepare",
        json!({
            "title": "爬楼梯问题 Work Item Plan",
            "story_spec_ids": ["story_spec_0001"],
            "design_spec_ids": ["design_spec_0001"],
            "author_provider": "fake",
            "reviewer_provider": "codex",
            "review_rounds": 1,
            "superpowers_enabled": true,
            "openspec_enabled": true,
            "include_integration_tests": true,
            "include_e2e_tests": false,
            "force_frontend_backend_split": true,
            "require_execution_plan_confirm": false
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(response["work_item_plan"]["status"], "draft");
    assert_eq!(
        response["work_item_plan"]["options"]["include_integration_tests"],
        true
    );
    assert_eq!(
        response["work_item_plan"]["options"]["include_e2e_tests"],
        false
    );
    assert_eq!(
        response["work_item_plan"]["options"]["force_frontend_backend_split"],
        true
    );
    assert_eq!(
        response["work_item_plan"]["options"]["require_execution_plan_confirm"],
        false
    );
    assert!(
        response["work_item_plan"]["work_item_ids"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        response["work_item_plan"]["verification_plan_ids"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        response["work_item_plan"]["dependency_graph"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        response["workspace_session"]["workspace_type"],
        "work_item_plan"
    );
    assert_eq!(
        response["workspace_session"]["entity_id"],
        response["work_item_plan"]["id"]
    );

    let lifecycle = cadence_aria::product::lifecycle_store::LifecycleStore::new(
        cadence_aria::product::app_paths::ProductAppPaths::new(_repo.path().join(".aria")),
    );
    assert!(
        lifecycle
            .list_work_items("project_0001", "issue_0001")
            .unwrap()
            .is_empty()
    );

    let first_message = &response["workspace_session"]["messages"][0]["content"];
    assert!(
        first_message
            .as_str()
            .unwrap()
            .contains("候选 work item plan 生成器")
    );
}
