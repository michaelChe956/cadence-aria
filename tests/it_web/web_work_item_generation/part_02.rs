pub(crate) fn valid_canonical_draft_output(outline_id: &str, title: &str) -> Value {
    let (
        logical_work_item_id,
        kind,
        goal,
        exclusive_scopes,
        forbidden_scopes,
        provider_logical_work_item_id,
        command,
    ) = match outline_id {
        "outline_frontend_expiry" => (
            "wi_frontend_expiry",
            "frontend",
            "在前端展示会话过期提示并触发重新登录入口。",
            vec!["web/src/session/expiry.ts"],
            vec!["src/product/**"],
            Some("wi_backend_session"),
            "pnpm -C web test",
        ),
        "outline_integration_session" => (
            "wi_integration_session",
            "integration",
            "覆盖会话过期到前端提示的贯通路径。",
            vec!["tests/session/expiry.rs"],
            Vec::new(),
            Some("wi_frontend_expiry"),
            "cargo test --locked --test it_web session",
        ),
        _ => (
            "wi_backend_session",
            "backend",
            "提供登录会话过期检测与刷新相关 API。",
            vec!["src/product/session.rs", "src/web/session_handlers.rs"],
            vec!["web/**"],
            None,
            "cargo test --locked --lib session",
        ),
    };
    let input_contracts = provider_logical_work_item_id.map_or_else(
        || json!([]),
        |provider| {
            json!([{
                "contract_id": format!("contract.{provider}.output"),
                "provider_logical_work_item_id": provider,
                "required_capabilities": ["ready"],
                "compatibility_policy": "require_all"
            }])
        },
    );
    let output_contract_id = format!("contract.{logical_work_item_id}.output");
    let task_id = format!("task.{logical_work_item_id}.1");
    let requirement_id = format!("REQ-{logical_work_item_id}-001");
    let acceptance_id = format!("AC-{logical_work_item_id}-001");
    let check_id = format!("check.{logical_work_item_id}.focused");
    let verification_check = json!({
        "check_id": check_id,
        "command": command,
        "manual_instruction": null,
        "required": true,
        "non_zero_test_execution_required": true
    });

    json!({
        "draft": {
            "outline_id": outline_id,
            "logical_work_item_id": logical_work_item_id,
            "canonical_contract": {
                "schema_version": 1,
                "identity": {
                    "logical_work_item_id": logical_work_item_id,
                    "title": title,
                    "kind": kind
                },
                "goal": { "summary": goal },
                "non_goals": [],
                "input_contracts": input_contracts,
                "output_contracts": [{
                    "contract_id": output_contract_id,
                    "capabilities": ["ready"]
                }],
                "tasks": [{
                    "task_id": task_id,
                    "statement": goal,
                    "requirement_refs": [requirement_id],
                    "done_when_refs": [acceptance_id]
                }],
                "write_policy": {
                    "exclusive_scopes": exclusive_scopes,
                    "forbidden_scopes": forbidden_scopes
                },
                "acceptance_criteria": [{
                    "criterion_id": acceptance_id,
                    "statement": "当前 work item 通过验证",
                    "required_evidence": ["source_diff", "non_zero_test_execution"]
                }],
                "verification_checks": [verification_check],
                "handoff_contract": {
                    "required_fields": ["commit_sha", "tests"],
                    "provided_contract_refs": [output_contract_id],
                    "reviewer_check_refs": [acceptance_id]
                },
                "blocker_rules": [],
                "design_traceability": [{
                    "source_type": "design_spec",
                    "source_id": "design_spec_0001",
                    "requirement_id": requirement_id
                }]
            },
            "verification_plan": {
                "checks": [verification_check]
            }
        }
    })
}

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

pub(crate) async fn app_with_confirmed_story_and_design_and_streaming_raw_outputs(
    outputs: Vec<QueuedSplitOutput>,
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
            output: valid_split_output(),
            revision_output: None,
        }),
    );

    let captured_prompts = Arc::new(Mutex::new(Vec::new()));
    let test_controls = cadence_aria::web::test_controls::TestControls::default();
    let mut registry = ProviderRegistry::new();
    registry.register(
        ProviderName::Fake,
        Arc::new(QueuedSplitStreamingProvider::new_raw_recording(
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

pub(crate) fn app_for_existing_root_with_streaming_raw_outputs(
    root: &std::path::Path,
    outputs: Vec<QueuedSplitOutput>,
) -> (axum::Router, Arc<Mutex<Vec<String>>>) {
    let runtime = WebRuntime::new_fake(root.to_path_buf());
    let mut state = WebAppState::new(root.to_path_buf(), runtime).with_provider_adapter(Arc::new(
        MockSplitProviderAdapter {
            output: valid_split_output(),
            revision_output: None,
        },
    ));
    let captured_prompts = Arc::new(Mutex::new(Vec::new()));
    let test_controls = cadence_aria::web::test_controls::TestControls::default();
    let mut registry = ProviderRegistry::new();
    registry.register(
        ProviderName::Fake,
        Arc::new(QueuedSplitStreamingProvider::new_raw_recording(
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

    (build_web_router(state), captured_prompts)
}

/// 与 `app_with_confirmed_story_and_design` 相同，但额外把 codex/claude_code 也注册为
/// TestControlledFakeStreamingProvider，以便在 review 阶段通过 review fixture 注入固定 verdict。
pub(crate) async fn app_with_confirmed_story_and_design_and_test_providers(
    output: Value,
) -> (axum::Router, tempfile::TempDir) {
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
            output,
            revision_output: None,
        }),
    );

    let mut registry = ProviderRegistry::new();
    let test_controls = cadence_aria::web::test_controls::TestControls::default();
    registry.register(
        ProviderName::Fake,
        Arc::new(TestControlledFakeStreamingProvider::new(
            test_controls.clone(),
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

    (app, root)
}

/// 与 `app_with_confirmed_story_and_design_and_test_providers` 相同，但额外提供 revision 输出，
/// 用于验证包含 review / revision 的完整 WorkItemPlan 流程。
pub(crate) async fn app_with_confirmed_story_and_design_revision_and_test_providers(
    output: Value,
    revision_output: Value,
) -> (axum::Router, tempfile::TempDir) {
    let _test_controls_env = crate::enable_test_controls().await;
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
            output,
            revision_output: Some(revision_output),
        }),
    );

    let mut registry = ProviderRegistry::new();
    let test_controls = cadence_aria::web::test_controls::TestControls::default();
    registry.register(
        ProviderName::Fake,
        Arc::new(TestControlledFakeStreamingProvider::new(
            test_controls.clone(),
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

    (app, root)
}

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
