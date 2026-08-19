//! Task R3：登记端点换形 `/logical-codebases/{lc_id}/registrations…` 集成测试。
//!
//! 契约来源：v1.3 §4——
//! - 五端点新路径（preflight / submit / get / resume / cancel）按 lc_id 解析子树与 manifest 键
//! - guard：lc_id 不存在 → 404 logical_codebase_not_found；单仓 project 同 404
//! - 登记零 repos.json 改写（R2 concern③：不经 feature-enabled RepositoryStore，
//!   不触发懒惰 identity 迁移）
//! - 旧路径 `/logical-codebase/registrations*` 保留为"默认第一个逻辑代码库"兼容别名；
//!   无任何 LC 时 404
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
async fn lc_registration_full_chain_on_new_path_never_rewrites_repos_json() {
    let root = tempdir().expect("root");
    let aggregate_root = tempdir().expect("aggregate root");
    let member_a = aggregate_root.path().join("alpha");
    let member_b = aggregate_root.path().join("beta");
    git_repo_at(&member_a);
    git_repo_at(&member_b);
    commit(&member_a, "alpha");
    commit(&member_b, "beta");
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
        json!({"name":"Chain","description":null}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // 先落地一个单仓（repos.json 存在，制造懒惰迁移风险面）。
    let repository = crate::create_repository_and_wait(
        app.clone(),
        "project_0001",
        json!({"name":"Solo","path":member_a}),
    )
    .await;
    assert_eq!(repository["repository"]["repository_id"], "repository_0001");
    let repos_json_before = std::fs::read_to_string(&repos_json).expect("repos.json readable");

    // 创建逻辑代码库（manifest 待首批登记原子创建）。
    let (status, logical) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/logical-codebases",
        json!({"name":"Platform","aggregate_root":aggregate_root.path()}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{logical}");
    let lc_id = logical["id"].as_str().expect("logical id").to_string();

    // 新路径 preflight：候选分类与 D8 快照冻结语义不变。
    let (status, preflight) = request_json(
        app.clone(),
        Method::POST,
        &format!("/api/projects/project_0001/logical-codebases/{lc_id}/registrations/preflight"),
        json!({"aggregate_root": aggregate_root.path(), "candidate_paths": [], "auto_discover": true}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{preflight:?}");
    let preflight_id = preflight["preflight_id"]
        .as_str()
        .expect("preflight id")
        .to_string();
    let items = preflight["items"].as_array().expect("items");
    assert_eq!(items.len(), 2, "{items:?}");
    assert!(items.iter().all(|item| item["class"] == "eligible"));

    // 新路径 submit：D7 首批登记原子创建 manifest。
    let (status, batch) = request_json(
        app.clone(),
        Method::POST,
        &format!("/api/projects/project_0001/logical-codebases/{lc_id}/registrations"),
        json!({
            "preflight_id": preflight_id,
            "aggregate_root": aggregate_root.path(),
            "confirmed_paths": [
                aggregate_root.path().join("alpha").display().to_string(),
                aggregate_root.path().join("beta").display().to_string(),
            ],
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{batch}");
    let batch_id = batch["batch_id"].as_str().expect("batch id").to_string();
    assert_eq!(batch["status"], "completed");
    assert_eq!(batch["items"].as_array().expect("items").len(), 2);
    assert!(
        batch["items"]
            .as_array()
            .expect("items")
            .iter()
            .all(|item| item["status"] == "completed")
    );

    // 新路径 get：批次可查。
    let (status, fetched) = request_json(
        app.clone(),
        Method::GET,
        &format!("/api/projects/project_0001/logical-codebases/{lc_id}/registrations/{batch_id}"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{fetched}");
    assert_eq!(fetched["batch_id"], batch_id);

    // LC 详情：成员落位 + manifest 创建（per-LC 子树）。
    let (status, detail) = request_json(
        app.clone(),
        Method::GET,
        &format!("/api/projects/project_0001/logical-codebases/{lc_id}"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{detail}");
    assert_eq!(detail["manifest_present"], true);
    assert_eq!(detail["member_count"], 2);
    assert_eq!(detail["members"].as_array().expect("members").len(), 2);

    // ⚠️ R2 concern③ 断言：登记全链后 repos.json 字节不变（零懒惰迁移/投影改写）。
    assert_eq!(
        std::fs::read_to_string(&repos_json).expect("repos.json readable"),
        repos_json_before,
        "registration chain must not rewrite repos.json"
    );

    // 新路径 404 语义：lc_id 不存在（含单仓 project 上调用）。
    for (method, uri, body) in [
        (
            Method::POST,
            "/api/projects/project_0001/logical-codebases/logical_codebase_missing/registrations/preflight".to_string(),
            json!({"aggregate_root": aggregate_root.path(), "candidate_paths": [], "auto_discover": true}),
        ),
        (
            Method::POST,
            "/api/projects/project_0001/logical-codebases/logical_codebase_missing/registrations".to_string(),
            json!({"preflight_id": preflight_id, "aggregate_root": aggregate_root.path(), "confirmed_paths": []}),
        ),
        (
            Method::GET,
            format!(
                "/api/projects/project_0001/logical-codebases/logical_codebase_missing/registrations/{batch_id}"
            ),
            json!({}),
        ),
        (
            Method::POST,
            format!(
                "/api/projects/project_0001/logical-codebases/logical_codebase_missing/registrations/{batch_id}/resume"
            ),
            json!({}),
        ),
        (
            Method::POST,
            format!(
                "/api/projects/project_0001/logical-codebases/logical_codebase_missing/registrations/{batch_id}/cancel"
            ),
            json!({}),
        ),
    ] {
        let (status, body_response) = request_json(app.clone(), method, &uri, body).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body_response["code"], "logical_codebase_not_found");
    }
}

#[tokio::test]
async fn lc_registration_legacy_alias_routes_to_default_first_codebase() {
    let root = tempdir().expect("root");
    let aggregate_root = tempdir().expect("aggregate root");
    git_repo_at(&aggregate_root.path().join("gamma"));

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
        json!({"name":"Alias","description":null}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // 无任何 LC 时旧路径 404（不再产出 logical_codebase_feature_disabled）。
    let (status, missing) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/logical-codebase/registrations/preflight",
        json!({"aggregate_root": aggregate_root.path(), "candidate_paths": []}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{missing}");
    assert_eq!(missing["code"], "logical_codebase_not_found");

    // 创建第一个 LC 后，旧路径成为其兼容别名。
    let (status, logical) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/logical-codebases",
        json!({"name":"Default","aggregate_root":aggregate_root.path()}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{logical}");
    let lc_id = logical["id"].as_str().expect("logical id").to_string();

    let (status, preflight) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/logical-codebase/registrations/preflight",
        json!({"aggregate_root": aggregate_root.path(), "candidate_paths": [], "auto_discover": true}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{preflight:?}");
    let preflight_id = preflight["preflight_id"]
        .as_str()
        .expect("preflight id")
        .to_string();

    // 旧路径提交 → 新路径可见（同一默认 LC 子树）。
    let (status, batch) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/logical-codebase/registrations",
        json!({
            "preflight_id": preflight_id,
            "aggregate_root": aggregate_root.path(),
            "confirmed_paths": [
                aggregate_root.path().join("gamma").display().to_string()
            ],
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{batch}");
    let batch_id = batch["batch_id"].as_str().expect("batch id").to_string();

    let (status, fetched) = request_json(
        app.clone(),
        Method::GET,
        &format!("/api/projects/project_0001/logical-codebases/{lc_id}/registrations/{batch_id}"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{fetched}");
    assert_eq!(fetched["batch_id"], batch_id);
    assert_eq!(fetched["status"], "completed");

    // 旧路径批次查询同样可用。
    let (status, legacy_fetched) = request_json(
        app.clone(),
        Method::GET,
        &format!("/api/projects/project_0001/logical-codebase/registrations/{batch_id}"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{legacy_fetched}");
    assert_eq!(legacy_fetched["batch_id"], batch_id);
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

fn git_repo_at(path: &std::path::Path) {
    std::fs::create_dir_all(path).expect("create repo dir");
    run_git(path, &["init", "-q"]);
}

fn run_git(path: &std::path::Path, arguments: &[&str]) {
    let status = std::process::Command::new("git")
        .args(arguments)
        .current_dir(path)
        .status()
        .expect("git");
    assert!(status.success(), "git {arguments:?} failed");
}

fn commit(path: &std::path::Path, message: &str) {
    std::fs::write(path.join("README.md"), message).expect("write");
    run_git(path, &["add", "."]);
    run_git(
        path,
        &[
            "-c",
            "user.email=t@t",
            "-c",
            "user.name=t",
            "commit",
            "-q",
            "-m",
            message,
        ],
    );
}
