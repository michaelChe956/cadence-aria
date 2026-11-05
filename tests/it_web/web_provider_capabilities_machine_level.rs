//! REQ-PCR-08：provider 能力面改机器级路由（HTTP 面不再按项目 / 逻辑代码库
//! 定位）。
//!
//! - `GET /api/providers/capabilities`：**一个项目 / 逻辑代码库都没有**时
//!   仍返回 200 与四家 provider 投影（provider 是平台公用能力，与项目无关）。
//! - 旧的四条按项目 / 逻辑代码库定位的路由**整条删除**（无兼容别名 / 转发
//!   shim / 双写）：请求旧路由 ⇒ 404，且**不是**转发到新的机器级路由
//!   （响应体不得是能力投影）。

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use cadence_aria::web::app::build_web_router;
use cadence_aria::web::events::EventHub;
use cadence_aria::web::runtime::WebRuntime;
use cadence_aria::web::state::WebAppState;
use serde_json::{Value, json};
use tempfile::{TempDir, tempdir};
use tower::ServiceExt;

/// 机器级能力面路由（provider 是平台公用能力，与项目、逻辑代码库无关）。
const CAPABILITIES_URI: &str = "/api/providers/capabilities";
const REVALIDATE_URI: &str = "/api/providers/capabilities/revalidate";
const REVALIDATE_ALL_URI: &str = "/api/providers/capabilities/revalidate-all";

async fn request_json(
    app: &axum::Router,
    method: Method,
    uri: &str,
    body: Value,
) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .expect("request"),
        )
        .await
        .expect("response");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let text = String::from_utf8_lossy(&bytes);
    let value = if text.trim().is_empty() {
        Value::Null
    } else {
        // 非 JSON 体（如 axum 的提取器拒绝文本）也如实带回，不 panic（断言
        // 消息要看的是状态码）。
        serde_json::from_str(&text).unwrap_or_else(|_| Value::String(text.to_string()))
    };
    (status, value)
}

/// 机器级夹具：**不建任何项目 / 逻辑代码库**（能力是本机 provider CLI 的
/// 事实，与项目无关 ⇒ 一个项目都没有也必须可用）。
fn app_fixture() -> (axum::Router, TempDir) {
    let root = tempdir().expect("root");
    let state = WebAppState::with_events(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
        EventHub::new(),
    );
    let app = build_web_router(state);
    (app, root)
}

/// 旧的按项目 / 逻辑代码库定位的四条路由（已整条删除 ⇒ 必须 404）。
fn legacy_uris(project_id: &str, lc_id: &str) -> Vec<(Method, String)> {
    let base = format!("/api/projects/{project_id}/logical-codebases/{lc_id}");
    vec![
        (Method::GET, format!("{base}/capabilities")),
        (Method::POST, format!("{base}/capability-revalidate")),
        (Method::POST, format!("{base}/capabilities/revalidate-all")),
        (
            Method::GET,
            format!("{base}/capabilities/revalidate-all/capability_batch_missing"),
        ),
    ]
}

/// 机器级能力面：无任何项目 / 逻辑代码库时 GET 200 且返回四家投影。
#[tokio::test]
async fn machine_level_capability_status_requires_no_project_or_lc() {
    let (app, _root) = app_fixture();
    let (status, body) = request_json(&app, Method::GET, CAPABILITIES_URI, json!({})).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "无项目也必须可读机器级能力面: {body}"
    );
    let providers = body["providers"].as_array().expect("providers");
    assert_eq!(providers.len(), 4, "四家真实 provider: {body}");
    let types: Vec<&str> = providers
        .iter()
        .map(|provider| provider["provider_type"].as_str().expect("type"))
        .collect();
    for expected in ["claude_code", "codex", "pi", "kimi_code"] {
        assert!(types.contains(&expected), "缺 {expected}: {types:?}");
    }
}

/// 旧的四条按项目 / 逻辑代码库定位的路由整条删除：404，且不是转发到新路由。
#[tokio::test]
async fn legacy_project_scoped_capability_routes_are_gone() {
    let (app, _root) = app_fixture();
    for (method, uri) in legacy_uris("project_0001", "logical_codebase_0001") {
        let (status, body) = request_json(&app, method.clone(), &uri, json!({})).await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "{method} {uri} 必须 404（旧路由整条删除，不得留兼容别名）: {body}"
        );
        assert!(
            body.get("providers").is_none(),
            "{method} {uri} 不得转发到机器级能力面: {body}"
        );
    }
}

/// 机器级写面同样存在（路由已迁到机器级，不再按项目 / 逻辑代码库定位）：
/// 未知 provider_type ⇒ 422 稳定码（证明路由命中处理器而非 404）。
#[tokio::test]
async fn machine_level_revalidate_routes_exist_without_project_or_lc() {
    let (app, _root) = app_fixture();
    let (status, body) = request_json(
        &app,
        Method::POST,
        REVALIDATE_URI,
        json!({ "provider_type": "not-a-provider" }),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "body: {body}");
    assert_eq!(body["code"], "provider_capability_probe_unsupported");

    // 批量发起：立即返回 job_id（202），不等待探针。
    let (status, body) = request_json(&app, Method::POST, REVALIDATE_ALL_URI, json!({})).await;
    assert_eq!(status, StatusCode::ACCEPTED, "body: {body}");
    let job_id = body["job_id"].as_str().expect("job_id").to_string();
    assert!(job_id.starts_with("capability_batch_"), "job_id: {job_id}");

    // 只凭 job_id 就能读回作业（无项目 / 逻辑代码库标识）。
    let (status, body) = request_json(
        &app,
        Method::GET,
        &format!("{REVALIDATE_ALL_URI}/{job_id}"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(body["job_id"], json!(job_id), "body: {body}");
    assert!(
        body.get("project_id").is_none() && body.get("logical_codebase_id").is_none(),
        "批量作业不得绑定项目 / 逻辑代码库: {body}"
    );
}
