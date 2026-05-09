use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use cadence_aria::web::app::build_web_router;
use cadence_aria::web::runtime::WebRuntime;
use cadence_aria::web::state::WebAppState;
use serde_json::{Value, json};
use std::fs;
use tempfile::tempdir;
use tower::ServiceExt;

#[tokio::test]
async fn resource_handlers_cover_tasks_artifact_file_and_diff_contracts() {
    let workspace = tempdir().expect("workspace");
    let task_root = workspace.path().join(".aria/runtime/tasks/task_0001");
    fs::create_dir_all(task_root.join("artifacts/execution")).expect("artifacts");
    fs::create_dir_all(workspace.path().join("src")).expect("src");
    fs::write(
        task_root.join("state.json"),
        r#"{"task_id":"task_0001","phase":"execution","change_id":"aria-fibonacci-square"}"#,
    )
    .expect("state");
    fs::write(
        task_root.join("artifacts/execution/0000.json"),
        r#"{"artifact_ref":"coding_report_work_wt_001_0001","artifact_kind":"coding_report","producer_node":"N16"}"#,
    )
    .expect("artifact");
    fs::write(
        workspace.path().join("src/fibonacciSquareSum.js"),
        "export const ok = true;\n",
    )
    .expect("source");

    let state = WebAppState::new(
        workspace.path().to_path_buf(),
        WebRuntime::new_fake(workspace.path().to_path_buf()),
    );
    let app = build_web_router(state);

    let tasks = request_json(app.clone(), Method::GET, "/api/tasks", json!({})).await;
    assert_eq!(tasks["tasks"][0]["task_id"], "task_0001");

    let artifact = request_json(
        app.clone(),
        Method::GET,
        "/api/artifacts/coding_report_work_wt_001_0001",
        json!({}),
    )
    .await;
    assert_eq!(artifact["artifact_ref"], "coding_report_work_wt_001_0001");
    assert_eq!(artifact["content_type"], "json");

    let file = request_json(
        app.clone(),
        Method::GET,
        "/api/files/content?path=src/fibonacciSquareSum.js",
        json!({}),
    )
    .await;
    assert_eq!(file["path"], "src/fibonacciSquareSum.js");
    assert!(
        file["content"]
            .as_str()
            .expect("content")
            .contains("export const ok")
    );

    let diff = request_json(
        app,
        Method::GET,
        "/api/files/diff?base_checkpoint=ckpt_0001&path=src/fibonacciSquareSum.js",
        json!({}),
    )
    .await;
    assert_eq!(diff["path"], "src/fibonacciSquareSum.js");
    assert!(diff["diff"].is_string());
}

async fn request_json(app: axum::Router, method: Method, uri: &str, body: Value) -> Value {
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("request");
    let response = app.oneshot(request).await.expect("response");
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    serde_json::from_slice(&bytes).expect("json")
}
