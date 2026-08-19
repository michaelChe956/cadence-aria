// ---- Task 7：prepare_work_item_plan Logical 分支接线（REQ-TGT-01）----
// 多仓 issue（repo_id=None，manifest+selection → RepositoryRouting::Logical）+ Confirmed
// 聚合 Story/Design → prepare 三态分流：
// 1. design involved ∈ selection → 200，plan draft 且 work_item_ids 空（target 校验通过，
//    无单仓 repository_id 要求）；
// 2. design involved ∉ selection（真实 selection 只含 [A]）→ 4xx target_not_in_selection
//    （REQ-TGT-01：target 必须 ∈ selection 有效成员）。
// 复用同模块 part_02 的 seed_logical_codebase（manifest + selection + index + policy）。

/// Task 7 fixture：建多仓 issue（repo_id=None，成为 issue_0001）+ seed logical codebase
/// （member 在 selection）+ Confirmed 聚合 Story + Confirmed 聚合 Design。
/// `design_effective`/`design_involved` 决定 design 聚合视野字段（involved 必须 ⊆ effective
/// 才可通过 create 校验）；resolver 的有效成员由真实 selection 决定（= [member]）。
/// 返回 (root, app, story_id, design_id)；必须保留 TempDir 直到请求完成。
async fn create_logical_prepare_fixture(
    member: LogicalRepositoryId,
    design_effective: Vec<LogicalRepositoryId>,
    design_involved: Vec<LogicalRepositoryId>,
) -> (tempfile::TempDir, axum::Router, String, String) {
    let root = tempdir().expect("root");
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects",
        json!({"name":"Lifecycle","description":null}),
    )
    .await;
    // 先建多仓 issue（repo_id=None，成为 issue_0001），再写 selection。
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    IssueStore::new(app_paths.clone())
        .create(CreateProductIssueInput {
            project_id: "project_0001".to_string(),
            repo_id: None,
            logical_codebase_id: None,
            title: "多仓聚合 WorkItemPlan".to_string(),
            description: Some("跨仓库聚合计划".to_string()),
            change_id: None,
        })
        .expect("multi-repo issue");
    seed_logical_codebase(&app_paths, member);
    let lifecycle = LifecycleStore::new(app_paths.clone());
    let story = lifecycle
        .create_story_spec(CreateStorySpecInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: String::new(),
            title: "前置聚合 Story".to_string(),
            aggregate_codebase: Some(AggregateStorySpecScope {
                logical_codebase_ref: uuid::Uuid::from_u128(0x0100),
                effective_member_ids: vec![member],
                involved_repository_ids: vec![member],
                focus_repository_id: None,
            }),
        })
        .expect("logical story");
    lifecycle
        .update_spec_confirmation_status(
            "project_0001",
            "issue_0001",
            &story.id,
            LifecycleConfirmationStatus::Confirmed,
        )
        .expect("confirm story");
    let design = lifecycle
        .create_design_spec(CreateDesignSpecInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            story_spec_ids: vec![story.id.clone()],
            title: "前置聚合 Design".to_string(),
            aggregate_codebase: Some(AggregateDesignSpecScope {
                logical_codebase_ref: uuid::Uuid::from_u128(0x0100),
                effective_member_ids: design_effective,
                involved_repository_ids: design_involved,
                change_order: Vec::new(),
            }),
        })
        .expect("logical design");
    lifecycle
        .update_spec_confirmation_status(
            "project_0001",
            "issue_0001",
            &design.id,
            LifecycleConfirmationStatus::Confirmed,
        )
        .expect("confirm design");
    (root, app, story.id, design.id)
}

#[tokio::test]
async fn prepare_work_item_plan_logical_branch_validates_target_in_selection() {
    // 多仓 issue + confirmed design（involved=[A] ∈ selection）→ prepare → 200，plan 为
    // draft 且 work_item_ids 空（Logical 分支走 resolver + target 校验通过，不要求 repo_id）。
    let member = LogicalRepositoryId(uuid::Uuid::from_u128(1));
    let (_root, app, story_id, design_id) =
        create_logical_prepare_fixture(member, vec![member], vec![member]).await;

    let (status, response) = request_json(
        app,
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-item-plans:prepare",
        json!({
            "title": "聚合 Work Item Plan",
            "story_spec_ids": [story_id],
            "design_spec_ids": [design_id],
            "author_provider": "fake",
            "reviewer_provider": "codex",
            "review_rounds": 1,
            "superpowers_enabled": false,
            "openspec_enabled": false
        }),
    )
    .await;

    assert_eq!(
        status,
        StatusCode::OK,
        "Logical prepare_work_item_plan 必须成功: {response}"
    );
    assert_eq!(response["work_item_plan"]["status"], "draft");
    assert!(
        response["work_item_plan"]["work_item_ids"]
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
}

#[tokio::test]
async fn prepare_work_item_plan_logical_branch_rejects_involved_outside_selection() {
    // REQ-TGT-01：design involved=[B] 但真实 selection 只含 [A] → prepare → 4xx
    // target_not_in_selection（design scope 声明 effective=[A,B] 仅通过 create 校验；
    // resolver 以真实 selection 为准 = [A]）。
    let member_a = LogicalRepositoryId(uuid::Uuid::from_u128(1));
    let member_b = LogicalRepositoryId(uuid::Uuid::from_u128(2));
    let (_root, app, story_id, design_id) = create_logical_prepare_fixture(
        member_a,
        vec![member_a, member_b],
        vec![member_b],
    )
    .await;

    let (status, response) = request_json(
        app,
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-item-plans:prepare",
        json!({
            "title": "聚合 Work Item Plan",
            "story_spec_ids": [story_id],
            "design_spec_ids": [design_id],
            "author_provider": "fake",
            "reviewer_provider": "codex",
            "review_rounds": 1,
            "superpowers_enabled": false,
            "openspec_enabled": false
        }),
    )
    .await;

    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "involved 越界 selection 必须 4xx 阻断: {response}"
    );
    assert_eq!(response["code"], "target_not_in_selection");
}

// ---- #5 minor 收尾：spec 加载失败的 HTTP 状态码区分 ----
// validate_confirm_aggregate_spec 经 load_existing_spec 读盘，失败分两类：
//   ① NotFound（spec 文件不存在，如 session 指向已删 spec）→ 应 404；
//   ② IO/JSON 解析错误（文件损坏）→ 应 500。
// 修复前两者都被包成 "confirm_gate_spec_load_failed: ..." → confirm_gate_failed → 500，
// NotFound 丢失 404 语义。本组测试守护区分后的稳定契约。

/// 建多仓 Story spec + session，额外返回 spec 文件路径（供调用方删/写坏以模拟加载失败）。
async fn create_logical_confirm_fixture_with_spec_path(
    involved: Vec<LogicalRepositoryId>,
) -> (
    tempfile::TempDir,
    axum::Router,
    String,
    std::path::PathBuf,
) {
    let root = tempdir().expect("root");
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle = LifecycleStore::new(app_paths.clone());
    IssueStore::new(app_paths.clone())
        .create(CreateProductIssueInput {
            project_id: "project_0001".to_string(),
            repo_id: None,
            logical_codebase_id: None,
            title: "多仓 confirm 加载失败".to_string(),
            description: None,
            change_id: None,
        })
        .expect("multi-repo issue");
    let story = lifecycle
        .create_story_spec(CreateStorySpecInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: String::new(),
            title: "多仓 Story".to_string(),
            aggregate_codebase: Some(AggregateStorySpecScope {
                logical_codebase_ref: uuid::Uuid::from_u128(0x0100),
                effective_member_ids: involved.clone(),
                involved_repository_ids: involved,
                focus_repository_id: None,
            }),
        })
        .expect("logical story");
    let session = lifecycle
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: story.id.clone(),
            workspace_type: WorkspaceType::Story,
            author_provider: ProviderName::Fake,
            reviewer_provider: ProviderName::Codex,
            review_rounds: 1,
            superpowers_enabled: false,
            openspec_enabled: false,
        })
        .expect("workspace session");
    let spec_path = app_paths
        .issue_lifecycle_root("project_0001", "issue_0001")
        .join("story-specs")
        .join(format!("{}.json", story.id));
    (root, app, session.id, spec_path)
}

#[tokio::test]
async fn confirm_logical_gate_missing_spec_file_returns_not_found() {
    // #5：spec 文件不存在（NotFound）→ confirm 应 404，而非 500。
    // 修复前：load_existing_spec 返 NotFound → 被包成 confirm_gate_spec_load_failed 字符串
    // → HTTP 映射 confirm_gate_failed → 500（错误，丢了 404 语义）。
    let member = LogicalRepositoryId(uuid::Uuid::from_u128(1));
    let (_root, app, session_id, spec_path) =
        create_logical_confirm_fixture_with_spec_path(vec![member]).await;
    fs::remove_file(&spec_path).expect("remove spec file to simulate NotFound");
    let (status, body) = request_json(
        app,
        Method::POST,
        &format!("/api/workspace-sessions/{session_id}/confirm"),
        json!({ "confirmed_by": "human" }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "spec 文件缺失应是 404 NotFound，而非 500: {body}"
    );
    assert_eq!(body["code"], "spec_not_found");
}

#[tokio::test]
async fn confirm_logical_gate_corrupted_spec_file_returns_server_error() {
    // #5：spec 文件 JSON 损坏（解析错误）→ confirm 应 500，且 code 区分于 NotFound/gate 违规。
    // 修复前：落 confirm_gate_failed（碰巧 500，但与 NotFound 混在一起无法区分）。
    let member = LogicalRepositoryId(uuid::Uuid::from_u128(1));
    let (_root, app, session_id, spec_path) =
        create_logical_confirm_fixture_with_spec_path(vec![member]).await;
    fs::write(&spec_path, "{ this is not valid json").expect("corrupt spec file");
    let (status, body) = request_json(
        app,
        Method::POST,
        &format!("/api/workspace-sessions/{session_id}/confirm"),
        json!({ "confirmed_by": "human" }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "spec 文件损坏应是 500 server error: {body}"
    );
    assert_eq!(body["code"], "confirm_gate_spec_load_failed");
    // reviewer Important 修复：SpecLoad 的 message 必须脱敏，不得拼接底层 ProductStoreError
    // 的 Display（避免绝对路径/JSON 解析诊断经公开 API message 泄露）。
    let message = body["message"].as_str().expect("message 字段应为字符串");
    assert_eq!(
        message, "spec load failed",
        "SpecLoad message 必须是脱敏固定文案，不得含底层错误细节: {message}"
    );
    assert!(
        !message.contains(".json"),
        "SpecLoad message 不得泄露文件路径: {message}"
    );
}
