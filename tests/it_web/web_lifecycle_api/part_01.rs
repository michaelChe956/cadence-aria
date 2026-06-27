use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use cadence_aria::product::app_paths::ProductAppPaths;
use cadence_aria::product::lifecycle_store::{
    CreateWorkItemInput, CreateWorkspaceSessionInput, LifecycleStore,
};
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
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/repositories",
        json!({"name":"Repo","path":repo.path()}),
    )
    .await;

    let (status, missing_repo) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues",
        json!({"title":"Missing repo","description":null}),
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
            "repository_id":"repository_0001"
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
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/repositories",
        json!({"name":"Repo","path":repo.path()}),
    )
    .await;
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues",
        json!({"title":"登录会话过期","description":"描述","repository_id":"repository_0001"}),
    )
    .await;

    let (status, story_response) = request_json(
        app.clone(),
        Method::POST,
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
    assert!(context.contains(&repo.path().display().to_string()));
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
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/repositories",
        json!({"name":"Repo","path":repo.path()}),
    )
    .await;
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues",
        json!({"title":"登录会话过期","description":"描述","repository_id":"repository_0001"}),
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
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/repositories",
        json!({"name":"Repo","path":repo.path()}),
    )
    .await;
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues",
        json!({"title":"登录会话过期","description":"描述","repository_id":"repository_0001"}),
    )
    .await;
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/story-specs:generate",
        json!({"title":"登录会话过期提示"}),
    )
    .await;
    request_json(
        app.clone(),
        Method::POST,
        "/api/workspace-sessions/workspace_session_0001/confirm",
        json!({"confirmed_by":"human"}),
    )
    .await;

    let (status, design_response) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/design-specs:generate",
        json!({
            "title":"会话过期后端设计",
            "story_spec_ids":["story_spec_0001"],
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
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/repositories",
        json!({"name":"Repo","path":repo.path()}),
    )
    .await;
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues",
        json!({"title":"登录会话过期","description":"描述","repository_id":"repository_0001"}),
    )
    .await;
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/story-specs:generate",
        json!({"title":"登录会话过期提示"}),
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
            "title":"会话过期前端设计",
            "story_spec_ids":["story_spec_0001"]
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

    let (status, response) = request_json(
        app.clone(),
        Method::DELETE,
        "/api/projects/project_0001/issues/issue_0001/story-specs/story_spec_0001",
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
    assert_eq!(lifecycle["story_specs"].as_array().unwrap().len(), 0);
    assert_eq!(lifecycle["design_specs"].as_array().unwrap().len(), 1);
    assert_eq!(lifecycle["work_items"].as_array().unwrap().len(), 1);
    assert_eq!(lifecycle["workspace_sessions"].as_array().unwrap().len(), 2);

    let (status, response) = request_json(
        app.clone(),
        Method::DELETE,
        "/api/projects/project_0001/issues/issue_0001/design-specs/design_spec_0001",
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
    assert_eq!(lifecycle["design_specs"].as_array().unwrap().len(), 0);
    assert_eq!(lifecycle["work_items"].as_array().unwrap().len(), 1);
    assert_eq!(lifecycle["workspace_sessions"].as_array().unwrap().len(), 1);

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
        app,
        Method::GET,
        "/api/issues/issue_0001/lifecycle?project_id=project_0001",
        json!({}),
    )
    .await;
    assert_eq!(lifecycle["work_items"].as_array().unwrap().len(), 0);
    assert_eq!(lifecycle["workspace_sessions"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn generating_story_specs_returns_404_when_bound_repository_was_deleted() {
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
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/repositories",
        json!({"name":"Repo","path":repo.path()}),
    )
    .await;
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues",
        json!({
            "title":"登录会话过期",
            "description":"描述",
            "repository_id":"repository_0001"
        }),
    )
    .await;
    request_json(
        app.clone(),
        Method::DELETE,
        "/api/projects/project_0001/repositories/repository_0001",
        json!({}),
    )
    .await;

    let (status, error) = request_json(
        app,
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/story-specs:generate",
        json!({"title":"登录会话过期提示"}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(error["code"], "repository_not_found");
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
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/repositories",
        json!({"name":"Repo","path":repo.path()}),
    )
    .await;
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues",
        json!({"title":"登录会话过期","description":"描述","repository_id":"repository_0001"}),
    )
    .await;
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/story-specs:generate",
        json!({"title":"登录会话过期提示"}),
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

