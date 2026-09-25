// ---- Logical 分支族（issue/story/design specs Logical 注入）+ Confirm 逻辑族 + review_status 投影 ----
// 自 part_02.rs 纯移动拆分（无行为变化）；helpers（seed_logical_codebase 等）仍留 part_02（include! 平铺同作用域）。

#[tokio::test]
async fn logical_issue_lifecycle_does_not_require_repo_id() {
    // 多仓 issue（manifest+selection，无 repo_id）→ GET issue_lifecycle → 200，
    // 不报 repository_required / repository 相关 4xx。
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

    // 先建多仓 issue（repo_id=None，成为 issue_0001），再写 selection —— 避免 selection 的
    // issue_0001 目录被 count_entries 计入导致 issue 变成 issue_0002。
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    IssueStore::new(app_paths.clone())
        .create(CreateProductIssueInput {
            project_id: "project_0001".to_string(),
            repo_id: None,
            logical_codebase_id: None,
            title: "多仓聚合 Issue".to_string(),
            description: Some("跨 api 仓库的聚合变更".to_string()),
            change_id: None,
                   base_branch: None,
 })
        .expect("multi-repo issue");
    let member_id = LogicalRepositoryId(uuid::Uuid::from_u128(1));
    seed_logical_codebase(&app_paths, member_id);

    let (status, lifecycle) = request_json(
        app,
        Method::GET,
        "/api/issues/issue_0001/lifecycle?project_id=project_0001",
        json!({}),
    )
    .await;

    assert_eq!(
        status,
        StatusCode::OK,
        "Logical issue_lifecycle must not require repo_id: {lifecycle}"
    );
    assert_ne!(lifecycle["code"], "repository_required");
    assert_eq!(lifecycle["issue"]["repo_id"], Value::Null);
    assert_eq!(lifecycle["story_specs"].as_array().unwrap().len(), 0);
    assert_eq!(lifecycle["work_items"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn generate_story_specs_logical_branch_injects_aggregate_prompt() {
    // 多仓 issue → POST story-specs:generate → 200，story 为草稿态聚合视野
    // （aggregate_codebase=Some），session context message 含 inventory + 聚合指令。
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
            title: "多仓聚合 Story".to_string(),
            description: Some("跨 api 仓库的聚合变更".to_string()),
            change_id: None,
                   base_branch: None,
 })
        .expect("multi-repo issue");
    let member_id = LogicalRepositoryId(uuid::Uuid::from_u128(1));
    seed_logical_codebase(&app_paths, member_id);

    let (status, story_response) = request_json(
        app,
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/story-specs:generate",
        json!({
            "title":"聚合 Story Spec",
            "author_provider":"fake",
            "reviewer_provider":"codex",
            "review_rounds":3,
            "superpowers_enabled":false,
            "openspec_enabled":false
        }),
    )
    .await;

    assert_eq!(
        status,
        StatusCode::OK,
        "Logical story-specs:generate must succeed: {story_response}"
    );
    // 草稿态聚合 story（involved 空、Draft），repository_id 为空串（Logical 无单仓 repo_id）。
    let story = &story_response["story_specs"][0];
    assert_eq!(story["repository_id"], "");

    // session context message 注入 inventory + 聚合视野指令。
    let messages = story_response["workspace_session"]["messages"]
        .as_array()
        .unwrap();
    let context = messages
        .iter()
        .find(|message| {
            message["role"] == "system"
                && message["content"]
                    .as_str()
                    .is_some_and(|content| content.contains("Workspace 生成任务已准备"))
        })
        .expect("generation context message");
    let content = context["content"].as_str().unwrap();
    assert!(
        content.contains("聚合代码库成员清单"),
        "缺 inventory：{content}"
    );
    assert!(
        content.contains("00000000-0000-0000-0000-000000000001"),
        "缺成员行：{content}"
    );
    assert!(
        content.contains("involved_repository_ids"),
        "缺聚合指令：{content}"
    );
    assert!(
        content.contains("禁止回落到任意单一 primary 仓库"),
        "缺禁止 primary 回落指令：{content}"
    );
}

#[tokio::test]
async fn generate_design_specs_logical_branch_injects_aggregate_prompt() {
    // 多仓 issue → POST design-specs:generate → 200，design 为草稿态聚合视野
    // （aggregate_codebase=Some），session context message 注入 aggregate_design prompt
    // （inventory + involved/change_order/depends_on 指令）。C1 回归保护：Design Logical
    // 分支不得因 issue.repo_id=None 在 workspace_entity_context（issue_repo_id）处失败。
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
            title: "多仓聚合 Design".to_string(),
            description: Some("跨 api 仓库的聚合设计".to_string()),
            change_id: None,
                   base_branch: None,
 })
        .expect("multi-repo issue");
    let member_id = LogicalRepositoryId(uuid::Uuid::from_u128(1));
    seed_logical_codebase(&app_paths, member_id);

    // Design 生成要求至少一个 Confirmed story（validate_confirmed_story_specs）。
    let lifecycle = LifecycleStore::new(app_paths.clone());
    let story = lifecycle
        .create_story_spec(CreateStorySpecInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            title: "前置 Story Spec".to_string(),
            aggregate_codebase: None,
        })
        .expect("confirmed story");
    lifecycle
        .update_spec_confirmation_status(
            "project_0001",
            "issue_0001",
            &story.id,
            LifecycleConfirmationStatus::Confirmed,
        )
        .expect("confirm story");

    let (status, design_response) = request_json(
        app,
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/design-specs:generate",
        json!({
            "title":"聚合 Design Spec",
            "story_spec_ids":[story.id],
            "author_provider":"fake",
            "reviewer_provider":"codex",
            "review_rounds":3,
            "superpowers_enabled":false,
            "openspec_enabled":false
        }),
    )
    .await;

    assert_eq!(
        status,
        StatusCode::OK,
        "Logical design-specs:generate must succeed: {design_response}"
    );
    // 草稿态聚合 design（involved 空、Draft）。
    assert_eq!(
        design_response["design_specs"][0]["confirmation_status"],
        "draft"
    );

    // session context message 注入 inventory + 聚合视野指令（含 change_order/depends_on）。
    let messages = design_response["workspace_session"]["messages"]
        .as_array()
        .unwrap();
    let context = messages
        .iter()
        .find(|message| {
            message["role"] == "system"
                && message["content"]
                    .as_str()
                    .is_some_and(|content| content.contains("Workspace 生成任务已准备"))
        })
        .expect("generation context message");
    let content = context["content"].as_str().unwrap();
    assert!(
        content.contains("聚合代码库成员清单"),
        "缺 inventory：{content}"
    );
    assert!(
        content.contains("00000000-0000-0000-0000-000000000001"),
        "缺成员行：{content}"
    );
    assert!(
        content.contains("involved_repository_ids"),
        "缺聚合指令：{content}"
    );
    assert!(
        content.contains("禁止回落到任意单一 primary 仓库"),
        "缺禁止 primary 回落指令：{content}"
    );
    assert!(
        content.contains("change_order"),
        "缺 change_order 指令：{content}"
    );
    assert!(
        content.contains("depends_on"),
        "缺 depends_on 依据：{content}"
    );
}

// ---- Task 6：confirm gate（多仓 involved + change_order 校验，3b 收紧）----
// confirm_workspace_entity 在 Confirmed 前对多仓（logical_codebase_ref Some）Spec 校验：
// ① involved 非空（REQ-PLN-04「AI 不确定即 blocker」）；② Design involved>1 必须 change_order
// （决策 3b，REQ-PLN-05 收紧）。单仓（logical_codebase_ref None）不校验，保持 Legacy 行为不变。
use cadence_aria::product::lifecycle_store::{
    AggregateDesignSpecScope, AggregateStorySpecScope, CreateDesignSpecInput,
};

/// 建多仓 issue + 聚合视野 Story/Design spec + 其 workspace session，返回 (root, router, session_id)。
/// 直接经 LifecycleStore 构造聚合视野（involved/change_order 由调用方指定），确认门只读 spec。
/// 必须保留返回的 TempDir（root）直到请求完成，否则目录被清理导致 404。
async fn create_logical_confirm_fixture(
    kind: WorkspaceType,
    involved: Vec<LogicalRepositoryId>,
    change_order: Vec<LogicalRepositoryId>,
) -> (tempfile::TempDir, axum::Router, String) {
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
            title: "多仓聚合 confirm".to_string(),
            description: None,
            change_id: None,
                   base_branch: None,
 })
        .expect("multi-repo issue");
    let effective = involved.clone();
    let entity_id = match kind {
        WorkspaceType::Story => {
            let story = lifecycle
                .create_story_spec(CreateStorySpecInput {
                    project_id: "project_0001".to_string(),
                    issue_id: "issue_0001".to_string(),
                    repository_id: String::new(),
                    title: "多仓 Story".to_string(),
                    aggregate_codebase: Some(AggregateStorySpecScope {
                        logical_codebase_ref: uuid::Uuid::from_u128(0x0100),
                        effective_member_ids: effective,
                        involved_repository_ids: involved,
                        focus_repository_id: None,
                    }),
                })
                .expect("logical story");
            story.id
        }
        WorkspaceType::Design => {
            let design = lifecycle
                .create_design_spec(CreateDesignSpecInput {
                    project_id: "project_0001".to_string(),
                    issue_id: "issue_0001".to_string(),
                    story_spec_ids: Vec::new(),
                    title: "多仓 Design".to_string(),
                    aggregate_codebase: Some(AggregateDesignSpecScope {
                        logical_codebase_ref: uuid::Uuid::from_u128(0x0100),
                        effective_member_ids: effective,
                        involved_repository_ids: involved,
                        change_order,
                    }),
                })
                .expect("logical design");
            design.id
        }
        _ => panic!("only Story/Design supported"),
    };
    let session = lifecycle
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id,
            workspace_type: kind,
            author_provider: ProviderName::Fake,
            reviewer_provider: ProviderName::Codex,
            review_rounds: 1,
            superpowers_enabled: false,
            openspec_enabled: false,
            work_item_plan_options: None,
        })
        .expect("workspace session");
    (root, app, session.id)
}

#[tokio::test]
async fn confirm_logical_design_without_change_order_is_blocked() {
    // 多仓 Design（involved>1，无 change_order）→ confirm → 4xx blocker（3b）。
    let m1 = LogicalRepositoryId(uuid::Uuid::from_u128(1));
    let m2 = LogicalRepositoryId(uuid::Uuid::from_u128(2));
    let (_root, app, session_id) =
        create_logical_confirm_fixture(WorkspaceType::Design, vec![m1, m2], Vec::new()).await;
    let (status, body) = request_json(
        app,
        Method::POST,
        &format!("/api/workspace-sessions/{session_id}/confirm"),
        json!({ "confirmed_by": "human" }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "多仓 Design 缺 change_order 必须 4xx 阻断: {body}"
    );
    assert_eq!(body["code"], "change_order_required_for_logical_codebase");
}

#[tokio::test]
async fn confirm_logical_story_with_involved_succeeds() {
    // 多仓 Story（involved 非空）→ confirm → 200（3b：Story 不要求 change_order）。
    let member = LogicalRepositoryId(uuid::Uuid::from_u128(1));
    let (_root, app, session_id) =
        create_logical_confirm_fixture(WorkspaceType::Story, vec![member], Vec::new()).await;
    let (status, body) = request_json(
        app,
        Method::POST,
        &format!("/api/workspace-sessions/{session_id}/confirm"),
        json!({ "confirmed_by": "human" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "多仓 Story confirm 应成功: {body}");
    assert_eq!(body["status"], "confirmed");
}

#[tokio::test]
async fn confirm_logical_story_without_involved_is_blocked() {
    // 多仓 Story（involved 空）→ confirm → 4xx blocker（REQ-PLN-04「AI 不确定即 blocker」）。
    let (_root, app, session_id) =
        create_logical_confirm_fixture(WorkspaceType::Story, Vec::new(), Vec::new()).await;
    let (status, body) = request_json(
        app,
        Method::POST,
        &format!("/api/workspace-sessions/{session_id}/confirm"),
        json!({ "confirmed_by": "human" }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "多仓 Story 缺 involved 必须 4xx 阻断: {body}"
    );
    assert_eq!(body["code"], "involved_repositories_undetermined");
}

#[tokio::test]
async fn confirm_logical_design_with_change_order_succeeds() {
    // 多仓 Design（involved>1，有 change_order）→ confirm → 200。
    let m1 = LogicalRepositoryId(uuid::Uuid::from_u128(1));
    let m2 = LogicalRepositoryId(uuid::Uuid::from_u128(2));
    let (_root, app, session_id) =
        create_logical_confirm_fixture(WorkspaceType::Design, vec![m1, m2], vec![m1, m2]).await;
    let (status, body) = request_json(
        app,
        Method::POST,
        &format!("/api/workspace-sessions/{session_id}/confirm"),
        json!({ "confirmed_by": "human" }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "多仓 Design 带 change_order confirm 应成功: {body}"
    );
    assert_eq!(body["status"], "confirmed");
}

#[tokio::test]
async fn confirm_legacy_single_repo_story_without_involved_succeeds() {
    // 红线：单仓（logical_codebase_ref None）不校验 involved，保持 Legacy 行为不变。
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
            repo_id: Some("repository_0001".to_string()),
            logical_codebase_id: None,
            title: "单仓 Story confirm".to_string(),
            description: None,
            change_id: None,
                   base_branch: None,
 })
        .expect("legacy issue");
    let story = lifecycle
        .create_story_spec(CreateStorySpecInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            title: "单仓 Story".to_string(),
            aggregate_codebase: None,
        })
        .expect("legacy story");
    let session = lifecycle
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: story.id,
            workspace_type: WorkspaceType::Story,
            author_provider: ProviderName::Fake,
            reviewer_provider: ProviderName::Codex,
            review_rounds: 1,
            superpowers_enabled: false,
            openspec_enabled: false,
            work_item_plan_options: None,
        })
        .expect("legacy session");
    let (status, body) = request_json(
        app,
        Method::POST,
        &format!("/api/workspace-sessions/{}/confirm", session.id),
        json!({ "confirmed_by": "human" }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "单仓 Story confirm 不得被新门拦截: {body}"
    );
    assert_eq!(body["status"], "confirmed");
}

// F-26b（0497 现场锚）：design 审核与 plan Review Round 在 workspace timeline 有
// reviewer 节点证据，但生命周期工作台（issue 面）零展示——用户无从确认
// 「review 已做」。修复后：spec DTO 附带 review_status 投影（active reviewer 节点
// → running；否则 completed reviewer 节点 → completed；无证据 → null）。
#[tokio::test]
async fn lifecycle_projects_review_status_from_workspace_timeline_reviewer_runs() {
    use cadence_aria::web::workspace_ws_types::{
        ProviderConfigSnapshot, TimelineNode, TimelineNodeStatus, TimelineNodeType, WorkspaceStage,
    };

    fn reviewer_run_node(node_id: &str, status: TimelineNodeStatus) -> TimelineNode {
        TimelineNode {
            node_id: node_id.to_string(),
            node_type: TimelineNodeType::ReviewerRun,
            agent: Some(ProviderName::Codex),
            stage: WorkspaceStage::CrossReview,
            round: Some(1),
            status,
            title: "Review Round 1".to_string(),
            summary: None,
            started_at: "2026-09-21T00:00:00Z".to_string(),
            completed_at: Some("2026-09-21T00:01:00Z".to_string()),
            duration_ms: Some(60_000),
            artifact_ref: None,
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Fake,
                reviewer: Some(ProviderName::Codex),
                review_rounds: 1,
                permission_modes: Default::default(),
            },
            retry: None,
        }
    }

    let root = tempdir().expect("root");
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    cadence_aria::product::project_store::ProjectStore::new(app_paths.clone())
        .create(cadence_aria::product::project_store::CreateProjectInput {
            name: "Lifecycle".to_string(),
            description: None,
        })
        .expect("project");
    let lifecycle = LifecycleStore::new(app_paths.clone());
    IssueStore::new(app_paths.clone())
        .create(CreateProductIssueInput {
            project_id: "project_0001".to_string(),
            repo_id: Some("repository_0001".to_string()),
            logical_codebase_id: None,
            title: "review evidence issue".to_string(),
            description: None,
            change_id: None,
                   base_branch: None,
 })
        .expect("issue");
    let story = lifecycle
        .create_story_spec(CreateStorySpecInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            title: "review evidence story".to_string(),
            aggregate_codebase: None,
        })
        .expect("story");
    let session = lifecycle
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: story.id,
            workspace_type: WorkspaceType::Story,
            author_provider: ProviderName::Fake,
            reviewer_provider: ProviderName::Codex,
            review_rounds: 1,
            superpowers_enabled: false,
            openspec_enabled: false,
            work_item_plan_options: None,
        })
        .expect("session");

    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    let lifecycle_uri = "/api/issues/issue_0001/lifecycle?project_id=project_0001";

    // 1) 无 reviewer 节点 → review_status 缺省（null）。
    lifecycle
        .save_timeline_nodes(&session.id, &[])
        .expect("empty timeline nodes");
    let (status, body) = request_json(app.clone(), Method::GET, lifecycle_uri, json!({})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        body["story_specs"][0]["review_status"].is_null(),
        "no reviewer evidence must project null review_status: {body}"
    );

    // 2) 仅有 completed reviewer_run → completed。
    lifecycle
        .save_timeline_nodes(
            &session.id,
            &[reviewer_run_node(
                "timeline_node_002",
                TimelineNodeStatus::Completed,
            )],
        )
        .expect("completed reviewer node");
    let (status, body) = request_json(app.clone(), Method::GET, lifecycle_uri, json!({})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["story_specs"][0]["review_status"], "completed",
        "completed reviewer_run must project review_status=completed: {body}"
    );

    // 3) completed + active 并存 → running（进行中优先）。
    lifecycle
        .save_timeline_nodes(
            &session.id,
            &[
                reviewer_run_node("timeline_node_002", TimelineNodeStatus::Completed),
                reviewer_run_node("timeline_node_003", TimelineNodeStatus::Active),
            ],
        )
        .expect("active reviewer node");
    let (status, body) = request_json(app, Method::GET, lifecycle_uri, json!({})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["story_specs"][0]["review_status"], "running",
        "an active reviewer_run must project review_status=running: {body}"
    );
}
