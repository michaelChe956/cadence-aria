//! Task R2：统一 codebases 列表 + 逻辑代码库 CRUD 集成测试。
//!
//! 契约来源：v1.3 §4——
//! - GET  /api/projects/{pid}/codebases 混合列表（single_repo 来自 repos.json 呈现层，
//!   logical 来自 LogicalCodebaseStore，member_count 从 manifest active 成员计）
//! - POST /api/projects/{pid}/logical-codebases（重复 name → 409 logical_codebase_name_conflict）
//! - GET  /api/projects/{pid}/logical-codebases/{lc_id}（record + 成员 + 状态汇总）
//! - DELETE 同路径（软删/tombstone；不存在 → 404 logical_codebase_not_found）
use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use cadence_aria::web::app::build_web_router;
use cadence_aria::web::events::EventHub;
use cadence_aria::web::runtime::WebRuntime;
use cadence_aria::web::state::WebAppState;
use serde_json::{Value, json};
use tempfile::tempdir;
use tower::ServiceExt;

#[tokio::test]
async fn codebases_crud_roundtrip_merges_single_repo_and_logical_entries() {
    let root = tempdir().expect("root");
    let repository_root = git_repo();
    let aggregate_root = tempdir().expect("aggregate root");
    let state = WebAppState::with_events(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
        EventHub::new(),
    );
    let app = build_web_router(state);
    let repos_json = root
        .path()
        .join(".aria")
        .join("projects")
        .join("project_0001")
        .join("repos.json");

    let (status, _) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects",
        json!({"name":"Mixed","description":null}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // 空项目：混合列表为空。
    let (status, body) = request_json(
        app.clone(),
        Method::GET,
        "/api/projects/project_0001/codebases",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["codebases"], json!([]));

    // 单仓条目：既有 repositories 流程零变化（repos.json 数据不动）。
    let repository = crate::create_repository_and_wait(
        app.clone(),
        "project_0001",
        json!({"name":"Solo","path":repository_root.path()}),
    )
    .await;
    assert_eq!(repository["repository"]["repository_id"], "repository_0001");
    let repos_json_before = std::fs::read_to_string(&repos_json).expect("repos.json readable");

    // 单仓零变化：混合列表只呈现 repos.json 单仓条目，不触发任何迁移。
    let (status, body) = request_json(
        app.clone(),
        Method::GET,
        "/api/projects/project_0001/codebases",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let codebases = body["codebases"].as_array().expect("codebases");
    assert_eq!(codebases.len(), 1, "{codebases:?}");
    assert_eq!(codebases[0]["kind"], "single_repo");

    // 创建逻辑代码库：record + 空子树，manifest 待首批登记创建。
    let (status, logical) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/logical-codebases",
        json!({"name":"Platform","aggregate_root":aggregate_root.path()}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{logical}");
    let logical_id = logical["id"].as_str().expect("logical id").to_string();
    assert!(logical_id.starts_with("logical_codebase_"));
    assert_eq!(logical["name"], "Platform");
    assert_eq!(
        logical["aggregate_root"],
        aggregate_root.path().display().to_string()
    );

    // 混合列表：单仓 + 逻辑并存。
    let (status, body) = request_json(
        app.clone(),
        Method::GET,
        "/api/projects/project_0001/codebases",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let codebases = body["codebases"].as_array().expect("codebases");
    assert_eq!(codebases.len(), 2, "{codebases:?}");
    let single = codebases
        .iter()
        .find(|entry| entry["kind"] == "single_repo")
        .expect("single repo entry");
    assert_eq!(single["id"], "repository_0001");
    assert_eq!(single["name"], "Solo");
    assert_eq!(single["repository_id"], "repository_0001");
    assert_eq!(single["logical_codebase_id"], Value::Null);
    let logical_entry = codebases
        .iter()
        .find(|entry| entry["kind"] == "logical")
        .expect("logical entry");
    assert_eq!(logical_entry["id"], logical_id);
    assert_eq!(logical_entry["name"], "Platform");
    assert_eq!(logical_entry["repository_id"], Value::Null);
    assert_eq!(logical_entry["logical_codebase_id"], logical_id);
    assert_eq!(logical_entry["member_count"], 0);

    // 详情：record + 成员 + 初始化/索引状态汇总。
    let (status, detail) = request_json(
        app.clone(),
        Method::GET,
        &format!("/api/projects/project_0001/logical-codebases/{logical_id}"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{detail}");
    assert_eq!(detail["id"], logical_id);
    assert_eq!(detail["name"], "Platform");
    assert_eq!(detail["member_count"], 0);
    assert_eq!(detail["members"], json!([]));
    assert_eq!(detail["manifest_present"], false);
    assert_eq!(detail["membership_revision"], Value::Null);
    assert_eq!(detail["active_aggregate_index_id"], Value::Null);

    // 重复 name → 409 logical_codebase_name_conflict。
    let (status, conflict) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/logical-codebases",
        json!({"name":"Platform","aggregate_root":aggregate_root.path()}),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{conflict}");
    assert_eq!(conflict["code"], "logical_codebase_name_conflict");

    // 必填字段缺失 → 400。
    let (status, invalid) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/logical-codebases",
        json!({"name":"  ","aggregate_root":aggregate_root.path()}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{invalid}");
    assert_eq!(invalid["code"], "logical_codebase_name_required");
    let (status, invalid) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/logical-codebases",
        json!({"name":"Other","aggregate_root":""}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{invalid}");
    assert_eq!(invalid["code"], "aggregate_root_required");

    // 软删 → tombstone；成员仓（单仓 repos.json）零变化。
    let (status, deleted) = request_json(
        app.clone(),
        Method::DELETE,
        &format!("/api/projects/project_0001/logical-codebases/{logical_id}"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{deleted}");
    assert_eq!(deleted["status"], "deleted");
    assert_eq!(
        std::fs::read_to_string(&repos_json).expect("repos.json readable"),
        repos_json_before,
        "single-repo repos.json must not change across logical codebase CRUD"
    );

    // 删除后：列表只剩单仓；详情/重复删除 → 404 logical_codebase_not_found。
    let (status, body) = request_json(
        app.clone(),
        Method::GET,
        "/api/projects/project_0001/codebases",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let codebases = body["codebases"].as_array().expect("codebases");
    assert_eq!(codebases.len(), 1, "{codebases:?}");
    assert_eq!(codebases[0]["kind"], "single_repo");

    let (status, missing) = request_json(
        app.clone(),
        Method::GET,
        &format!("/api/projects/project_0001/logical-codebases/{logical_id}"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{missing}");
    assert_eq!(missing["code"], "logical_codebase_not_found");

    let (status, missing) = request_json(
        app.clone(),
        Method::DELETE,
        &format!("/api/projects/project_0001/logical-codebases/{logical_id}"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{missing}");
    assert_eq!(missing["code"], "logical_codebase_not_found");

    // 删除后同名可重建（tombstone 不占名）。
    let (status, recreated) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/logical-codebases",
        json!({"name":"Platform","aggregate_root":aggregate_root.path()}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{recreated}");
    assert_ne!(recreated["id"], logical_id);
}

#[tokio::test]
async fn codebases_endpoints_reject_unknown_project_and_lc() {
    let root = tempdir().expect("root");
    let state = WebAppState::with_events(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
        EventHub::new(),
    );
    let app = build_web_router(state);

    let (status, body) = request_json(
        app.clone(),
        Method::GET,
        "/api/projects/project_missing/codebases",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["code"], "project_not_found");

    let (status, body) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_missing/logical-codebases",
        json!({"name":"Ghost","aggregate_root":"/tmp/ghost"}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["code"], "project_not_found");

    let (status, _) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects",
        json!({"name":"Empty","description":null}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, body) = request_json(
        app.clone(),
        Method::GET,
        "/api/projects/project_0001/logical-codebases/logical_codebase_nope",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["code"], "logical_codebase_not_found");

    let (status, body) = request_json(
        app,
        Method::DELETE,
        "/api/projects/project_0001/logical-codebases/logical_codebase_nope",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["code"], "logical_codebase_not_found");
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

fn git_repo() -> tempfile::TempDir {
    let dir = tempdir().expect("repo");
    let status = std::process::Command::new("git")
        .args(["init"])
        .current_dir(dir.path())
        .status()
        .expect("git init");
    assert!(status.success());
    dir
}
