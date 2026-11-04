use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use cadence_aria::product::app_paths::ProductAppPaths;
use cadence_aria::product::lifecycle_store::{
    CreateWorkItemInput, CreateWorkspaceSessionInput, LifecycleStore,
};
use cadence_aria::product::logical_codebase::store::legacy_logical_codebase_id;
use cadence_aria::product::models::{
    ProviderName, WorkItemPlanStatus, WorkspaceSessionStatus, WorkspaceType,
};
use cadence_aria::web::app::build_web_router;
use cadence_aria::web::runtime::WebRuntime;
use cadence_aria::web::state::WebAppState;
use cadence_aria::web::workspace_ws_types::{ArtifactPayload, ArtifactVersion};
use serde_json::{Value, json};
use std::{fs, process::Command};
use tempfile::tempdir;
use tower::ServiceExt;

#[tokio::test]
async fn issue_creation_requires_repository_and_lifecycle_lists_cards() {
    let root = tempdir().expect("root");
    let repo = git_repo();
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
    crate::create_repository_and_wait(
        app.clone(),
        "project_0001",
        json!({"name":"Repo","path":repo.path()}),
    )
    .await;

    // 块3：issue 必须挂 logical_codebase_id 归属（LC 权威由迁移建立，非假 id）；
    // 负向用例保持「缺 repository_id」维度——请求携带 LC 归属但缺
    // repository_id，仍应命中稳定码 repository_required（serde 层不再先拒）。
    let lc_id = migrate_repository_to_lc_and_rebuild(&app, root.path()).await;
    let (status, missing_repo) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues",
        json!({"title":"Missing repo","description":null,"logical_codebase_id":lc_id}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(missing_repo["code"], "repository_required");

    let (status, issue) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues",
        json!({
            "title":"登录会话过期",
            "description":"需要结合前端代码提示用户重新登录",
            "repository_id":"repository_0001",
            "logical_codebase_id":lc_id
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(issue["repo_id"], "repository_0001");

    let (status, lifecycle) = request_json(
        app,
        Method::GET,
        "/api/issues/issue_0001/lifecycle?project_id=project_0001",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(lifecycle["issue"]["issue_id"], "issue_0001");
    assert_eq!(lifecycle["story_specs"].as_array().unwrap().len(), 0);
    assert_eq!(lifecycle["design_specs"].as_array().unwrap().len(), 0);
    assert_eq!(lifecycle["work_items"].as_array().unwrap().len(), 0);
    assert_eq!(lifecycle["workspace_sessions"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn generate_endpoints_create_workspace_sessions_and_first_cards() {
    let root = tempdir().expect("root");
    let repo = git_repo();
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
    crate::create_repository_and_wait(
        app.clone(),
        "project_0001",
        json!({"name":"Repo","path":repo.path()}),
    )
    .await;
    // 块3：issue 必须挂 logical_codebase_id 归属（LC 权威由迁移建立，非假 id）。
    let lc_id = migrate_repository_to_lc_and_rebuild(&app, root.path()).await;
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues",
        json!({"title":"登录会话过期","description":"描述","repository_id":"repository_0001","logical_codebase_id":lc_id}),
    )
    .await;

    let (status, story_response) = lc_generate_json(
        app.clone(),
        "/api/projects/project_0001/issues/issue_0001/story-specs:generate",
        json!({
            "title":"登录会话过期提示",
            "author_provider":"fake",
            "reviewer_provider":"codex",
            "review_rounds":3,
            "superpowers_enabled":false,
            "openspec_enabled":true
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        story_response["story_specs"][0]["story_spec_id"],
        "story_spec_0001"
    );
    assert_eq!(
        story_response["workspace_session"]["workspace_type"],
        "story"
    );
    assert_eq!(
        story_response["workspace_session"]["author_provider"],
        "fake"
    );
    assert_eq!(
        story_response["workspace_session"]["reviewer_provider"],
        "codex"
    );
    assert_eq!(story_response["workspace_session"]["review_rounds"], 3);
    assert_eq!(
        story_response["workspace_session"]["superpowers_enabled"],
        false
    );
    let context_messages = story_response["workspace_session"]["messages"]
        .as_array()
        .expect("workspace context messages");
    assert_eq!(context_messages.len(), 1);
    assert_eq!(context_messages[0]["role"], "system");
    let context = context_messages[0]["content"]
        .as_str()
        .expect("context content");
    assert!(context.contains("登录会话过期"));
    assert!(context.contains("描述"));
    assert!(context.contains("Repo"));
    // 聚合上下文的成员路径按【聚合根相对】投影（REQ-PLN-01 成员 inventory 的
    // id/alias/path/role/profile；path 为 checkout 相对
    // manifest.provider_context_root 的后缀，见 planning_context_set.rs
    // `member_root_relative_paths`）⇒ 绝对物理仓路径不再是契约面。
    let app_paths =
        cadence_aria::product::app_paths::ProductAppPaths::new(root.path().join(".aria"));
    let logical = cadence_aria::product::logical_codebase::LogicalCodebaseStore::new(app_paths);
    let manifest = logical
        .load_manifest("project_0001")
        .expect("load logical manifest")
        .expect("logical manifest");
    let member = logical
        .list_members("project_0001")
        .expect("list logical members")
        .into_iter()
        .next()
        .expect("logical member");
    let checkout = logical
        .load_checkout("project_0001", member.checkout_ids[0])
        .expect("load logical checkout")
        .expect("logical checkout");
    let member_relative_path = checkout
        .canonical_path
        .strip_prefix(&manifest.provider_context_root)
        .expect("member checkout under aggregate root")
        .to_string_lossy()
        .to_string();
    assert!(
        context.contains(&member_relative_path),
        "aggregate context must carry the member inventory path ({member_relative_path}): {context}"
    );
    assert!(context.contains("登录会话过期提示"));
    assert!(context.contains("[system]"));
    assert!(context.contains("候选 spec 生成器"));
    assert!(context.contains("[constraint_summary]"));
    assert!(context.contains("OpenSpec"));
    assert!(context.contains("不要直接修改 OpenSpec"));
    assert!(context.contains("## 范围"));
    assert!(context.contains("## 用户故事"));
    assert!(context.contains("## 功能需求"));
    assert!(context.contains("## 成功标准"));
    assert!(context.contains("[REQ-001]"));
    assert!(context.contains("[AC-001]"));
    assert!(
        !context.contains("必须遵守 using-superpowers"),
        "explicitly disabled superpowers should not be advertised as enabled"
    );

    let (status, lifecycle) = request_json(
        app,
        Method::GET,
        "/api/issues/issue_0001/lifecycle?project_id=project_0001",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(lifecycle["story_specs"].as_array().unwrap().len(), 1);
    assert_eq!(lifecycle["workspace_sessions"].as_array().unwrap().len(), 1);
    assert_eq!(
        lifecycle["workspace_sessions"][0]["workspace_session_id"],
        "workspace_session_0001"
    );
    assert_eq!(
        lifecycle["workspace_sessions"][0]["entity_id"],
        "story_spec_0001"
    );
    assert!(
        lifecycle["workspace_sessions"][0]
            .as_object()
            .expect("workspace session summary")
            .get("messages")
            .is_none(),
        "lifecycle list should return workspace session summaries without full messages"
    );
}

#[tokio::test]
async fn generate_story_specs_falls_back_from_default_codex_to_available_claude_code() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::with_provider_availability(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
        |provider| matches!(provider, ProviderName::ClaudeCode),
    ));

    request_json(
        app.clone(),
        Method::POST,
        "/api/projects",
        json!({"name":"Lifecycle","description":null}),
    )
    .await;
    crate::create_repository_and_wait(
        app.clone(),
        "project_0001",
        json!({"name":"Repo","path":repo.path()}),
    )
    .await;
    // 块3：issue 必须挂 logical_codebase_id 归属（LC 权威由迁移建立，非假 id）。
    let lc_id = migrate_repository_to_lc_and_rebuild(&app, root.path()).await;
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues",
        json!({"title":"登录会话过期","description":"描述","repository_id":"repository_0001","logical_codebase_id":lc_id}),
    )
    .await;

    let (status, story_response) = request_json(
        app,
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/story-specs:generate",
        json!({"title":"登录会话过期提示"}),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        story_response["workspace_session"]["author_provider"],
        "claude_code"
    );
    assert_eq!(
        story_response["workspace_session"]["reviewer_provider"],
        "claude_code"
    );
}

#[tokio::test]
async fn confirmed_story_and_design_can_generate_design_and_work_item_workspaces() {
    let root = tempdir().expect("root");
    let repo = git_repo();
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
    crate::create_repository_and_wait(
        app.clone(),
        "project_0001",
        json!({"name":"Repo","path":repo.path()}),
    )
    .await;
    // 块3：issue 必须挂 logical_codebase_id 归属（LC 权威由迁移建立，非假 id）。
    let lc_id = migrate_repository_to_lc_and_rebuild(&app, root.path()).await;
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues",
        json!({"title":"登录会话过期","description":"描述","repository_id":"repository_0001","logical_codebase_id":lc_id}),
    )
    .await;
    lc_generate_json(
        app.clone(),
        "/api/projects/project_0001/issues/issue_0001/story-specs:generate",
        json!({"title":"登录会话过期提示","involved_repository_ids":legacy_lc_involved_repository_ids(root.path())}),
    )
    .await;
    request_json(
        app.clone(),
        Method::POST,
        "/api/workspace-sessions/workspace_session_0001/confirm",
        json!({"confirmed_by":"human"}),
    )
    .await;

    let (status, design_response) = lc_generate_json(
        app.clone(),
        "/api/projects/project_0001/issues/issue_0001/design-specs:generate",
        json!({
            "title":"会话过期后端设计",
            "story_spec_ids":["story_spec_0001"],
            "involved_repository_ids":legacy_lc_involved_repository_ids(root.path()),
            "author_provider":"codex",
            "reviewer_provider":"claude_code",
            "review_rounds":2,
            "superpowers_enabled":true,
            "openspec_enabled":true
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        design_response["design_specs"][0]["design_spec_id"],
        "design_spec_0001"
    );
    assert_eq!(
        design_response["design_specs"][0]["story_spec_ids"],
        json!(["story_spec_0001"])
    );
    assert_eq!(
        design_response["workspace_session"]["workspace_type"],
        "design"
    );

    request_json(
        app.clone(),
        Method::POST,
        "/api/workspace-sessions/workspace_session_0002/confirm",
        json!({"confirmed_by":"human"}),
    )
    .await;

    bootstrap_confirmed_work_item_session(root.path(), ProviderName::Codex, ProviderName::Fake)
        .await;

    let (status, lifecycle) = request_json(
        app,
        Method::GET,
        "/api/issues/issue_0001/lifecycle?project_id=project_0001",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        lifecycle["story_specs"][0]["confirmation_status"],
        "confirmed"
    );
    assert_eq!(
        lifecycle["design_specs"][0]["confirmation_status"],
        "confirmed"
    );
    assert_eq!(lifecycle["work_items"].as_array().unwrap().len(), 1);
    assert_eq!(lifecycle["workspace_sessions"].as_array().unwrap().len(), 3);
}

#[tokio::test]
async fn delete_lifecycle_entities_removes_cards_and_workspace_sessions() {
    let root = tempdir().expect("root");
    let repo = git_repo();
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
    crate::create_repository_and_wait(
        app.clone(),
        "project_0001",
        json!({"name":"Repo","path":repo.path()}),
    )
    .await;
    // 块3：issue 必须挂 logical_codebase_id 归属（LC 权威由迁移建立，非假 id）。
    let lc_id = migrate_repository_to_lc_and_rebuild(&app, root.path()).await;
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues",
        json!({"title":"登录会话过期","description":"描述","repository_id":"repository_0001","logical_codebase_id":lc_id}),
    )
    .await;
    lc_generate_json(
        app.clone(),
        "/api/projects/project_0001/issues/issue_0001/story-specs:generate",
        json!({"title":"登录会话过期提示","involved_repository_ids":legacy_lc_involved_repository_ids(root.path())}),
    )
    .await;
    request_json(
        app.clone(),
        Method::POST,
        "/api/workspace-sessions/workspace_session_0001/confirm",
        json!({"confirmed_by":"human"}),
    )
    .await;
    lc_generate_json(
        app.clone(),
        "/api/projects/project_0001/issues/issue_0001/design-specs:generate",
        json!({
            "title":"会话过期前端设计",
            "story_spec_ids":["story_spec_0001"],
            "involved_repository_ids":legacy_lc_involved_repository_ids(root.path())
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
    bootstrap_confirmed_work_item_session(root.path(), ProviderName::Fake, ProviderName::Fake)
        .await;

    // D2.2 迁移：按引用序「先删后置」——work item 引用 design、design 引用
    // story（REQ-DEL-03 下倒序会被 entity_referenced 拒）；design 已 confirm，
    // 须显式确认一次（REQ-DEL-04）。
    let (status, response) = request_json(
        app.clone(),
        Method::DELETE,
        "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0001",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(response["status"], "deleted");
    let (_, lifecycle) = request_json(
        app.clone(),
        Method::GET,
        "/api/issues/issue_0001/lifecycle?project_id=project_0001",
        json!({}),
    )
    .await;
    assert_eq!(lifecycle["story_specs"].as_array().unwrap().len(), 1);
    assert_eq!(lifecycle["design_specs"].as_array().unwrap().len(), 1);
    assert_eq!(lifecycle["work_items"].as_array().unwrap().len(), 0);
    assert_eq!(lifecycle["workspace_sessions"].as_array().unwrap().len(), 2);

    let (status, response) = request_json(
        app.clone(),
        Method::DELETE,
        "/api/projects/project_0001/issues/issue_0001/design-specs/design_spec_0001?confirmed=true",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(response["status"], "deleted");

    let (status, response) = request_json(
        app.clone(),
        Method::DELETE,
        // D2.2 迁移：该 story 亦已 confirm（fixture 会话确认），同须确认一次。
        "/api/projects/project_0001/issues/issue_0001/story-specs/story_spec_0001?confirmed=true",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "story delete: {response}");
    assert_eq!(response["status"], "deleted");
    let (_, lifecycle) = request_json(
        app,
        Method::GET,
        "/api/issues/issue_0001/lifecycle?project_id=project_0001",
        json!({}),
    )
    .await;
    assert_eq!(lifecycle["story_specs"].as_array().unwrap().len(), 0);
    assert_eq!(lifecycle["design_specs"].as_array().unwrap().len(), 0);
    assert_eq!(lifecycle["work_items"].as_array().unwrap().len(), 0);
    assert_eq!(lifecycle["workspace_sessions"].as_array().unwrap().len(), 0);
}

/// 场景（REQ-ROUTE-01 场景「不一致即失败关闭」+ REQ-ROUTE-06 稳定码表）：
///
/// ① **删遗留物理仓记录不影响 LC 通道**：`delete_repository`
///    （product_resources.rs:113）删的是 repos.json 里的物理仓记录，LC 权威
///    （manifest/member/checkout/selection）完好 ⇒ 规划上下文仍可生成（200）。
///    旧断言钉的「删物理仓 ⇒ 404 repository_not_found」其保护对象已不成立：
///    `repository_not_found` 是【物理仓记录缺失】的码，而 REQ-ROUTE-01 的
///    fail-closed 清单（(Some,None)/(None,Some)、selection 失效、member
///    删除/停用、target 不在有效 selection、snapshot 不一致）里没有「删物理仓
///    记录」这一项 ⇒ 迁就 404 会丢掉保护意图（对 LC 归属 issue 硬造一个物理仓
///    缺失的码），MUST NOT。
/// ② **删 LC member 必须 fail-closed**（REQ-ROUTE-01 明文）：成员停用
///    （Removed）后同一入口必须非 200 且带稳定码，MUST NOT 回退物理仓库、
///    MUST NOT 猜测成员。
#[tokio::test]
async fn generating_story_specs_survives_legacy_repository_deletion_but_fails_closed_without_lc_member() {
    let root = tempdir().expect("root");
    let repo = git_repo();
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
    crate::create_repository_and_wait(
        app.clone(),
        "project_0001",
        json!({"name":"Repo","path":repo.path()}),
    )
    .await;
    // 块3：issue 必须挂 logical_codebase_id 归属（LC 权威由迁移建立，非假 id）。
    let lc_id = migrate_repository_to_lc_and_rebuild(&app, root.path()).await;
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues",
        json!({
            "title":"登录会话过期",
            "description":"描述",
            "repository_id":"repository_0001",
            "logical_codebase_id":lc_id
        }),
    )
    .await;
    delete_repository_with_idempotency_key(app.clone(), "project_0001", "repository_0001").await;

    let (status, response) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/story-specs:generate",
        json!({"title":"登录会话过期提示"}),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "deleting the legacy physical repository record must not break the LC channel: {response}"
    );

    // ② LC member 停用（Removed）⇒ REQ-ROUTE-01 fail-closed。
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let logical = cadence_aria::product::logical_codebase::LogicalCodebaseStore::new(app_paths);
    let mut member = logical
        .list_members("project_0001")
        .expect("list logical members")
        .into_iter()
        .next()
        .expect("logical member");
    member.status = cadence_aria::product::logical_codebase::MemberStatus::Removed;
    logical
        .save_member("project_0001", &member)
        .expect("remove logical member");

    let (status, error) = request_json(
        app,
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/story-specs:generate",
        json!({"title":"登录会话过期提示二"}),
    )
    .await;
    assert_ne!(
        status,
        StatusCode::OK,
        "a removed LC member must fail closed (REQ-ROUTE-01): {error}"
    );
    assert!(
        error["code"].is_string(),
        "fail-closed must carry a stable error code: {error}"
    );
}

#[tokio::test]
async fn workspace_session_message_run_and_confirm_update_session_state() {
    let root = tempdir().expect("root");
    let repo = git_repo();
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
    crate::create_repository_and_wait(
        app.clone(),
        "project_0001",
        json!({"name":"Repo","path":repo.path()}),
    )
    .await;
    // 块3：issue 必须挂 logical_codebase_id 归属（LC 权威由迁移建立，非假 id）。
    let lc_id = migrate_repository_to_lc_and_rebuild(&app, root.path()).await;
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues",
        json!({"title":"登录会话过期","description":"描述","repository_id":"repository_0001","logical_codebase_id":lc_id}),
    )
    .await;
    lc_generate_json(
        app.clone(),
        "/api/projects/project_0001/issues/issue_0001/story-specs:generate",
        json!({"title":"登录会话过期提示","involved_repository_ids":legacy_lc_involved_repository_ids(root.path())}),
    )
    .await;

    let (status, message) = request_json(
        app.clone(),
        Method::POST,
        "/api/workspace-sessions/workspace_session_0001/message",
        json!({"role":"user","content":"请强调重新登录按钮"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        message["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|message| {
                message["role"] == "user" && message["content"] == "请强调重新登录按钮"
            })
    );

    let (status, running) = request_json(
        app.clone(),
        Method::POST,
        "/api/workspace-sessions/workspace_session_0001/run-next",
        json!({"user_prompt":"请生成带验收标准的 Story Spec"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(running["status"], "waiting_for_human");
    assert!(
        running["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|message| message["role"] == "user"
                && message["content"] == "请生成带验收标准的 Story Spec")
    );
    assert!(
        running["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|message| message["role"] == "provider"
                && message["content"]
                    .as_str()
                    .unwrap()
                    .contains("Provider Workspace"))
    );
    assert!(
        running["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|message| message["role"] == "reviewer")
    );
    let version: Value = serde_json::from_str(
        &fs::read_to_string(root.path().join(
            ".aria/projects/project_0001/issues/issue_0001/versions/story_spec_0001/version_0001.json",
        ))
        .expect("version file"),
    )
    .expect("version json");
    assert!(
        version["markdown"]
            .as_str()
            .unwrap()
            .contains("Provider Workspace")
    );
    let review_round: Value = serde_json::from_str(
        &fs::read_to_string(root.path().join(
            ".aria/projects/project_0001/issues/issue_0001/provider-review-rounds/review_round_0001.json",
        ))
        .expect("review round file"),
    )
    .expect("review round json");
    assert_eq!(review_round["round_index"], 1);

    let (status, confirmed) = request_json(
        app,
        Method::POST,
        "/api/workspace-sessions/workspace_session_0001/confirm",
        json!({"confirmed_by":"human"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(confirmed["status"], "confirmed");
    assert!(
        confirmed["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|message| message["role"] == "system"
                && message["content"]
                    .as_str()
                    .unwrap()
                    .contains("已由 human 确认"))
    );
}
