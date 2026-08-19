use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use cadence_aria::web::app::build_web_router;
use cadence_aria::web::events::EventHub;
use cadence_aria::web::runtime::WebRuntime;
use cadence_aria::web::state::WebAppState;
use serde_json::{Value, json};
use std::process::Command;
use tempfile::tempdir;
use tower::ServiceExt;

#[tokio::test]
async fn creates_project_repository_and_issue_via_product_api() {
    let root = tempdir().expect("root");
    let events = EventHub::new();
    let state = WebAppState::with_events(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
        events,
    );
    let app = build_web_router(state);

    let (status, created) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects",
        json!({"name":"Aria","description":"Workbench"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(created["project_id"], "project_0001");
    assert_eq!(created["name"], "Aria");

    let (status, aggregate) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects",
        json!({"name":"Aggregate","description":null}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(aggregate["project_id"], "project_0002");

    let (status, project) = request_json(
        app.clone(),
        Method::GET,
        "/api/projects/project_0001",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(project["project_id"], "project_0001");
    assert_eq!(project["name"], "Aria");

    let (status, opened) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/open",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(opened["project_id"], "project_0001");
    assert!(opened["last_opened_at"].is_string());

    let (status, missing) = request_json(
        app.clone(),
        Method::GET,
        "/api/projects/project_missing",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(missing["code"], "project_not_found");
    assert_eq!(missing["message"], "project not found");
    let missing_text = missing.to_string();
    assert!(!missing_text.contains(".aria"));
    assert!(!missing_text.contains("project.json"));
    assert!(!missing_text.contains(&root.path().display().to_string()));

    let projects = app
        .oneshot(
            Request::builder()
                .uri("/api/projects")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("list");
    assert_eq!(projects.status(), StatusCode::OK);
}

#[tokio::test]
async fn project_creation_has_no_repository_mode_and_legacy_repository_mutations_succeed() {
    let root = tempdir().expect("root");
    let repository_root = git_repo();
    let state = WebAppState::with_events(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
        EventHub::new(),
    );
    let app = build_web_router(state);

    let (status, project) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects",
        json!({"name":"Multi","description":null}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        project.get("multi_repo").is_none(),
        "project repository mode was removed in R1: {project}"
    );

    // R1 retires the legacy-repository-endpoint-on-multi-repo protection:
    // legacy repository creation is ordinary single-repo CRUD with no project mode.
    let (status, _body) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/repositories",
        json!({"name":"Multi Repo","path":repository_root.path()}),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);

    let (repositories_status, repositories) = request_json(
        app.clone(),
        Method::GET,
        "/api/projects/project_0001/repositories",
        json!({}),
    )
    .await;
    assert_eq!(repositories_status, StatusCode::OK);
    assert_eq!(repositories["repositories"], json!([]));

    let (missing, body) = request_json(
        app,
        Method::GET,
        "/api/projects/project_missing/repositories",
        json!({}),
    )
    .await;
    assert_eq!(missing, StatusCode::NOT_FOUND);
    assert_eq!(body["code"], "project_not_found");
}

#[tokio::test]
async fn mixed_project_single_repo_endpoints_stay_plain_crud() {
    // R6（v1.3 §4）：单仓端点是单仓代码库 CRUD，与 project 是否存在逻辑代码库
    // 无关（多仓 project 防护语义废除）。混合 project 的 GET/POST/DELETE
    // /repositories 与 GET /repository-initializations 只读写 repos.json 与
    // 初始化操作存储，绝不投影逻辑成员、不触发 identity 迁移 bootstrap。
    let root = tempdir().expect("root");
    let repository_root = git_repo();
    let state = WebAppState::with_events(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
        EventHub::new(),
    );
    let app = build_web_router(state);

    let (status, _) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects",
        json!({"name":"Mixed","description":null}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // 新式逻辑代码库（仅 record.json 子树，无 legacy manifest）。
    let aggregate_root = tempdir().expect("aggregate root");
    let (status, logical) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/logical-codebases",
        json!({"name":"LC","aggregate_root":aggregate_root.path()}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{logical}");

    // 预置一条既有单仓记录。
    let paths = cadence_aria::product::app_paths::ProductAppPaths::new(root.path().join(".aria"));
    let repos_path = paths.project_root("project_0001").join("repos.json");
    cadence_aria::product::json_store::write_json(
        &repos_path,
        &vec![serde_json::json!({
            "id": "repository_0001",
            "project_id": "project_0001",
            "name": "seeded",
            "path": root.path().join("seeded"),
            "repo_hash": "seeded-hash",
            "runtime_root": root.path().join("seeded/.aria/runtime"),
            "default_policy_preset": "manual-write",
            "default_provider_mode": "fake",
            "created_at": "2026-08-19T00:00:00Z",
            "updated_at": "2026-08-19T00:00:00Z",
        })],
    )
    .expect("seed repos.json");

    // GET：只列 repos.json 单仓条目；逻辑成员不投影进来。
    let (status, repositories) = request_json(
        app.clone(),
        Method::GET,
        "/api/projects/project_0001/repositories",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{repositories}");
    let entries = repositories["repositories"]
        .as_array()
        .expect("repositories array");
    assert_eq!(entries.len(), 1, "plain repos.json listing: {repositories}");
    assert_eq!(entries[0]["repository_id"], "repository_0001");

    // POST：普通单仓登记流程照常可用。
    let (status, accepted) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/repositories",
        json!({"name":"Second","path":repository_root.path()}),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{accepted}");
    let operation_id = accepted["operation_id"]
        .as_str()
        .expect("operation id")
        .to_string();

    // GET 初始化操作：行为不变，轮询至终态。
    let mut operation = serde_json::Value::Null;
    for _ in 0..200 {
        let (status, body) = request_json(
            app.clone(),
            Method::GET,
            &format!("/api/projects/project_0001/repository-initializations/{operation_id}"),
            json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        operation = body;
        if matches!(operation["status"].as_str(), Some("completed" | "failed")) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(
        operation["status"], "completed",
        "single-repo initialization must complete: {operation}"
    );

    let (status, repositories) = request_json(
        app.clone(),
        Method::GET,
        "/api/projects/project_0001/repositories",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let entries = repositories["repositories"]
        .as_array()
        .expect("repositories array");
    assert_eq!(
        entries.len(),
        2,
        "seeded + newly registered: {repositories}"
    );

    // 单仓登记绝不 bootstrap legacy 逻辑代码库 authority，也不改写既有记录。
    assert!(
        !paths
            .logical_codebase_root("project_0001")
            .join("manifest.json")
            .exists()
    );

    // DELETE：普通单仓删除（legacy_delete=true），不动 LC 子树。
    let (status, receipt) =
        delete_repository_with_idempotency_key(app.clone(), "project_0001", "repository_0001")
            .await;
    assert_eq!(status, StatusCode::OK, "{receipt}");
    assert_eq!(receipt["legacy_delete"], true);

    let (status, repositories) = request_json(
        app,
        Method::GET,
        "/api/projects/project_0001/repositories",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let entries = repositories["repositories"]
        .as_array()
        .expect("repositories array");
    assert_eq!(entries.len(), 1, "only the newly registered entry remains");
    assert_eq!(entries[0]["repository_id"], "repository_0002");
}

#[tokio::test]
async fn manages_workspace_repositories_and_keeps_issue_on_lifecycle_flow() {
    let root = tempdir().expect("root");
    let repo_a = git_repo();
    let repo_b = git_repo();
    let events = EventHub::new();
    let state = WebAppState::with_events(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
        events,
    );
    let app = build_web_router(state);

    let (status, workspace) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects",
        json!({"name":"Product Workspace","description":"Issue lifecycle"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(workspace["project_id"], "project_0001");

    let repository_a = crate::create_repository_and_wait(
        app.clone(),
        "project_0001",
        json!({"name":"Repo A","path":repo_a.path()}),
    )
    .await;
    assert_eq!(
        repository_a["repository"]["repository_id"],
        "repository_0001"
    );
    assert_eq!(repository_a["repository"]["project_id"], "project_0001");

    let repository_b = crate::create_repository_and_wait(
        app.clone(),
        "project_0001",
        json!({"name":"Repo B","path":repo_b.path()}),
    )
    .await;
    assert_eq!(
        repository_b["repository"]["repository_id"],
        "repository_0002"
    );

    let (status, repositories) = request_json(
        app.clone(),
        Method::GET,
        "/api/projects/project_0001/repositories",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        repositories["repositories"]
            .as_array()
            .expect("repositories")
            .len(),
        2
    );

    let (status, issue) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues",
        json!({
            "title":"优化任务管理页面",
            "description":"展示 story spec、design spec、work item 和完成状态",
            "repository_id":"repository_0002"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(issue["issue_id"], "issue_0001");
    assert_eq!(issue["project_id"], "project_0001");
    assert_eq!(issue["repo_id"], "repository_0002");
    assert_eq!(issue["phase"], "clarification");
    assert_eq!(issue["status"], "draft");

    let (status, _) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/start",
        json!({"repository_id":"repository_0002","provider_mode":"fake"}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, issues) = request_json(
        app,
        Method::GET,
        "/api/projects/project_0001/issues",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let issues = issues["issues"].as_array().expect("issues");
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0]["repo_id"], "repository_0002");
    assert_eq!(issues[0]["phase"], "clarification");
    assert_eq!(issues[0]["status"], "draft");
    assert_eq!(issues[0]["active_binding_id"], Value::Null);
    assert_eq!(
        issues[0]["artifacts"].as_array().expect("artifacts").len(),
        0
    );
    assert!(!repo_b.path().join(".aria/runtime/tasks/task_0001").exists());
}

#[tokio::test]
async fn product_issue_start_endpoint_is_removed() {
    let root = tempdir().expect("root");
    let repo_a = git_repo();
    let repo_b = git_repo();
    let events = EventHub::new();
    let state = WebAppState::with_events(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
        events,
    );
    let app = build_web_router(state);

    request_json(
        app.clone(),
        Method::POST,
        "/api/projects",
        json!({"name":"Product Project","description":"Issue lifecycle"}),
    )
    .await;
    crate::create_repository_and_wait(
        app.clone(),
        "project_0001",
        json!({"name":"Workspace A","path":repo_a.path()}),
    )
    .await;
    crate::create_repository_and_wait(
        app.clone(),
        "project_0001",
        json!({"name":"Workspace B","path":repo_b.path()}),
    )
    .await;
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues",
        json!({
            "title":"执行一次即可",
            "description":"再次点击应该跳转",
            "repository_id":"repository_0001"
        }),
    )
    .await;

    let (status, _) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/start",
        json!({"workspace_id":"repository_0001","provider_mode":"fake"}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, _) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/start",
        json!({"workspace_id":"repository_0002","provider_mode":"fake"}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, issues) = request_json(
        app,
        Method::GET,
        "/api/projects/project_0001/issues",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(issues["issues"][0]["workspace_id"], Value::Null);
    assert_eq!(issues["issues"][0]["task_id"], Value::Null);
    assert_eq!(issues["issues"][0]["session_id"], Value::Null);
}

#[tokio::test]
async fn deletes_workspace_project_repository_and_issue_records() {
    let root = tempdir().expect("root");
    let workspace_a = git_repo();
    let workspace_b = git_repo();
    let repo = git_repo();
    let events = EventHub::new();
    let state = WebAppState::with_events(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
        events,
    );
    let app = build_web_router(state);

    request_json(
        app.clone(),
        Method::POST,
        "/api/workspaces",
        json!({"name":"Workspace A","path":workspace_a.path()}),
    )
    .await;
    request_json(
        app.clone(),
        Method::POST,
        "/api/workspaces",
        json!({"name":"Workspace B","path":workspace_b.path()}),
    )
    .await;
    let (status, _) = request_json(
        app.clone(),
        Method::DELETE,
        "/api/workspaces/workspace_0001",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, workspaces) =
        request_json(app.clone(), Method::GET, "/api/workspaces", json!({})).await;
    assert_eq!(status, StatusCode::OK);
    let workspaces = workspaces["workspaces"].as_array().expect("workspaces");
    assert_eq!(workspaces.len(), 1);
    assert_eq!(workspaces[0]["workspace_id"], "workspace_0002");

    request_json(
        app.clone(),
        Method::POST,
        "/api/projects",
        json!({"name":"Product","description":null}),
    )
    .await;
    crate::create_repository_and_wait(
        app.clone(),
        "project_0001",
        json!({"name":"Code Repo","path":repo.path()}),
    )
    .await;
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues",
        json!({
            "title":"Issue to delete",
            "description":null,
            "repository_id":"repository_0001"
        }),
    )
    .await;

    let (status, _) =
        delete_repository_with_idempotency_key(app.clone(), "project_0001", "repository_0001")
            .await;
    assert_eq!(status, StatusCode::OK);
    let (status, repositories) = request_json(
        app.clone(),
        Method::GET,
        "/api/projects/project_0001/repositories",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        repositories["repositories"]
            .as_array()
            .expect("repositories")
            .len(),
        0
    );

    let (status, _) = request_json(
        app.clone(),
        Method::DELETE,
        "/api/projects/project_0001/issues/issue_0001",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, issues) = request_json(
        app.clone(),
        Method::GET,
        "/api/projects/project_0001/issues",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(issues["issues"].as_array().expect("issues").len(), 0);

    let (status, _) = request_json(
        app.clone(),
        Method::DELETE,
        "/api/projects/project_0001",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, projects) = request_json(app, Method::GET, "/api/projects", json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(projects["projects"].as_array().expect("projects").len(), 0);
}

async fn request_json(
    app: axum::Router,
    method: Method,
    uri: &str,
    body: Value,
) -> (StatusCode, Value) {
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("request");
    let response = app.oneshot(request).await.expect("response");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, value)
}

async fn delete_repository_with_idempotency_key(
    app: axum::Router,
    project_id: &str,
    repository_id: &str,
) -> (StatusCode, Value) {
    let request = Request::builder()
        .method(Method::DELETE)
        .uri(format!(
            "/api/projects/{project_id}/repositories/{repository_id}"
        ))
        .header("content-type", "application/json")
        .header("Idempotency-Key", "test-delete-repo-product-0001")
        .body(Body::from("{}".to_string()))
        .expect("delete repository request");
    let response = app
        .oneshot(request)
        .await
        .expect("delete repository response");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("delete repository body");
    let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, value)
}

fn git_repo() -> tempfile::TempDir {
    let dir = tempdir().expect("repo");
    let status = Command::new("git")
        .args(["init"])
        .current_dir(dir.path())
        .status()
        .expect("git init");
    assert!(status.success());
    dir
}
