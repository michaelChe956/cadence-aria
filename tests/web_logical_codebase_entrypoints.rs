//! 独立的 logical codebase 生产入口 HTTP 契约测试二进制。
use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use serde_json::Value;
use tower::ServiceExt;

pub(crate) async fn request(
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
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

pub(crate) fn assert_error(
    actual: (StatusCode, Value),
    expected_status: StatusCode,
    expected_code: &str,
) -> Value {
    let (status, body) = actual;
    assert_eq!(status, expected_status, "unexpected error response: {body}");
    assert_eq!(
        body["code"], expected_code,
        "unexpected error response: {body}"
    );
    body
}

#[path = "web_logical_codebase_entrypoints/guards.rs"]
mod guards;
#[path = "web_logical_codebase_entrypoints/initialization.rs"]
mod initialization;
#[path = "web_logical_codebase_entrypoints/registration.rs"]
mod registration;
