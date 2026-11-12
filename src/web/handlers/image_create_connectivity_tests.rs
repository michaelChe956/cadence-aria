//! 连通性探测端点（POST /api/image-create/settings/connectivity）的 handler 层用例：
//! 入参校验 400、草稿 key 优先、空 key 回落已保存 key、两者皆无 400 且零出站。

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use serde_json::{Value, json};
use tempfile::tempdir;
use tower::ServiceExt;

use crate::cross_cutting::aria_state_paths::AriaStatePaths;
use crate::product::image_create::SettingsStore;
use crate::product::image_create::models::SettingsStoreApi;
use crate::web::app::build_web_router;
use crate::web::runtime::WebRuntime;
use crate::web::state::WebAppState;

fn state(root: &std::path::Path) -> WebAppState {
    WebAppState::new(root.to_path_buf(), WebRuntime::new_fake(root.to_path_buf()))
}

fn json_request(method: &str, uri: &str, body: Value) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .expect("request")
}

async fn response_json(response: axum::response::Response) -> Value {
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("response body");
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

/// 发起连通性探测请求，返回状态码与响应体。
async fn connectivity(app: Router, body: Value) -> (StatusCode, Value) {
    let response = app
        .oneshot(json_request(
            "POST",
            "/api/image-create/settings/connectivity",
            body,
        ))
        .await
        .expect("connectivity response");
    let status = response.status();
    (status, response_json(response).await)
}

/// PUT 保存 gpt 家配置（base_url 随意合法即可，key 为 given）。
async fn save_gpt_key(root: &std::path::Path, base_url: &str, api_key: &str) {
    let paths = AriaStatePaths::from_workspace_root(root);
    let store = SettingsStore::new(paths);
    let mut settings = store.load().await;
    settings.vendors.gpt.base_url = base_url.to_string();
    settings.vendors.gpt.api_key = api_key.to_string();
    store.save(&settings).await.expect("save settings");
}

/// 全通网关 mock：/v1/models 200 + generations 400，均校验 Bearer key，各期望 hits 次。
async fn passing_gateway(
    server: &mut mockito::Server,
    key: &str,
    model: &str,
    hits: usize,
) -> (mockito::Mock, mockito::Mock) {
    let bearer = format!("Bearer {key}");
    let models = server
        .mock("GET", "/v1/models")
        .match_header("authorization", bearer.as_str())
        .with_status(200)
        .with_body(format!(r#"{{"data":[{{"id":"{model}"}}]}}"#))
        .expect(hits)
        .create_async()
        .await;
    let generations = server
        .mock("POST", "/v1/images/generations")
        .match_header("authorization", bearer.as_str())
        .match_body(mockito::Matcher::Json(json!({ "model": model })))
        .with_status(400)
        .with_body("missing prompt")
        .expect(hits)
        .create_async()
        .await;
    (models, generations)
}

#[tokio::test]
async fn connectivity_rejects_malformed_requests_with_400() {
    let root = tempdir().expect("root");
    let app = build_web_router(state(root.path()));

    for body in [
        json!({"base_url": "https://api.example.com", "api_key": "sk-x"}),
        json!({"vendor": "gpt", "api_key": "sk-x"}),
        json!({"vendor": "unknown-vendor", "base_url": "https://api.example.com"}),
    ] {
        let (status, payload) = connectivity(app.clone(), body).await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "缺 vendor/base_url 或非法 vendor 必须 400"
        );
        assert!(
            payload["error"].is_string(),
            "错误体必须带 error 字段：{payload}"
        );
    }
}

#[tokio::test]
async fn connectivity_probes_with_draft_key_and_hides_it_in_response() {
    let root = tempdir().expect("root");
    let app = build_web_router(state(root.path()));
    let mut server = mockito::Server::new_async().await;
    let (models, generations) = passing_gateway(&mut server, "sk-draft", "m1", 1).await;

    let (status, payload) = connectivity(
        app,
        json!({
            "vendor": "gpt",
            "base_url": server.url(),
            "api_key": "sk-draft",
            "models": ["m1", "m2"],
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "探测结果一律 200：{payload}");
    assert_eq!(payload["ok"], json!(true));
    assert_eq!(payload["message"], json!("连通性正常"));
    assert_eq!(payload["missing_models"], json!(["m2"]));
    assert_eq!(payload["steps"][0]["key"], json!("models"));
    assert_eq!(payload["steps"][1]["key"], json!("generations"));
    assert!(
        payload["duration_ms"].is_u64(),
        "duration_ms 必须是非负整数"
    );
    assert!(
        !payload.to_string().contains("sk-draft"),
        "响应体不得含 key 明文：{payload}"
    );
    models.assert_async().await;
    generations.assert_async().await;
}

#[tokio::test]
async fn connectivity_with_empty_key_falls_back_to_saved_key() {
    let root = tempdir().expect("root");
    let app = build_web_router(state(root.path()));
    save_gpt_key(root.path(), "https://gateway.example.com", "sk-saved1234").await;
    let mut server = mockito::Server::new_async().await;
    let (models, generations) = passing_gateway(&mut server, "sk-saved1234", "m1", 1).await;

    let (status, payload) = connectivity(
        app,
        json!({
            "vendor": "gpt",
            "base_url": server.url(),
            "api_key": "",
            "models": ["m1"],
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "空 key 回落已保存 key：{payload}");
    assert_eq!(
        payload["ok"],
        json!(true),
        "已保存 key 有效则探测通过：{payload}"
    );
    models.assert_async().await;
    generations.assert_async().await;
}

#[tokio::test]
async fn connectivity_without_any_key_is_400_and_sends_nothing() {
    let root = tempdir().expect("root");
    let app = build_web_router(state(root.path()));
    let mut server = mockito::Server::new_async().await;
    let (models, generations) = passing_gateway(&mut server, "whatever", "m1", 0).await;

    let (status, payload) = connectivity(
        app,
        json!({
            "vendor": "gpt",
            "base_url": server.url(),
            "api_key": "",
            "models": ["m1"],
        }),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        payload["error"]
            .as_str()
            .is_some_and(|error| error.contains("未配置 API Key")),
        "错误文案须指明未配置 key：{payload}"
    );
    models.assert_async().await;
    generations.assert_async().await;
}

#[tokio::test]
async fn connectivity_with_invalid_base_url_is_400_and_sends_nothing() {
    let root = tempdir().expect("root");
    let app = build_web_router(state(root.path()));
    let mut server = mockito::Server::new_async().await;
    let (models, generations) = passing_gateway(&mut server, "sk-draft", "m1", 0).await;

    let (status, payload) = connectivity(
        app,
        json!({
            "vendor": "gpt",
            "base_url": server.url().replace("http://", "ftp://"),
            "api_key": "sk-draft",
            "models": ["m1"],
        }),
    )
    .await;

    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "非 https/loopback 必须 400：{payload}"
    );
    models.assert_async().await;
    generations.assert_async().await;
}
