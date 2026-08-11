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
