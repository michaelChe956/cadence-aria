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
    // B3:base_checkpoint 必须能解析为真实 commit——夹具改真仓真 SHA,
    // 不再是 `ckpt_0001` 伪引用(解析失败会走真实 git 错误,不再假空 diff)。
    git(workspace.path(), &["init", "--quiet", "-b", "main"]);
    git(
        workspace.path(),
        &["config", "user.email", "aria-b3@example.invalid"],
    );
    git(workspace.path(), &["config", "user.name", "Aria B3"]);
    git(workspace.path(), &["add", "."]);
    git(workspace.path(), &["commit", "--quiet", "-m", "init"]);
    let head = git(workspace.path(), &["rev-parse", "HEAD"]);
    fs::write(
        workspace.path().join("src/fibonacciSquareSum.js"),
        "export const ok = false;\n",
    )
    .expect("source change");

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
        &format!("/api/files/diff?base_checkpoint={head}&path=src/fibonacciSquareSum.js"),
        json!({}),
    )
    .await;
    assert_eq!(diff["path"], "src/fibonacciSquareSum.js");
    assert_eq!(diff["base_checkpoint"], head.as_str());
    let diff_text = diff["diff"].as_str().expect("diff text");
    assert!(
        !diff_text.trim().is_empty() && diff_text.contains("fibonacciSquareSum.js"),
        "合法真 SHA 必须产生真实 diff:{diff_text}"
    );
    assert!(
        diff_text.contains("ok = false"),
        "diff 必须反映 worktree 变更:{diff_text}"
    );
}

/// Bug45/F4:base_checkpoint 是 REST 直通 query——选项注入形态必须在 git
/// 执行前返回 400,且不产生任何文件副作用。
#[tokio::test]
async fn file_diff_rejects_option_injection_at_rest_boundary() {
    let workspace = tempdir().expect("workspace");
    fs::create_dir_all(workspace.path().join("src")).expect("src");
    fs::write(
        workspace.path().join("src/app.js"),
        "export const ok = true;\n",
    )
    .expect("source");

    let state = WebAppState::new(
        workspace.path().to_path_buf(),
        WebRuntime::new_fake(workspace.path().to_path_buf()),
    );
    let app = build_web_router(state);

    let probe = workspace.path().join("aria-pwned.txt");
    let injected = format!("--output={}", probe.display());
    let encoded: String = injected
        .bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
                (byte as char).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect();
    let (status, body) = request_status_json(
        app.clone(),
        Method::GET,
        &format!("/api/files/diff?base_checkpoint={encoded}&path=src/app.js"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body: {body}");
    assert_eq!(body["code"], "invalid_base_checkpoint");
    assert!(!probe.exists(), "拒绝的 base_checkpoint 不得产生文件");

    let (status, body) = request_status_json(
        app,
        Method::GET,
        "/api/files/diff?base_checkpoint=--ext-diff&path=src/app.js",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body: {body}");
    assert_eq!(body["code"], "invalid_base_checkpoint");
}

fn git(workspace: &std::path::Path, argv: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .args(argv)
        .current_dir(workspace)
        .output()
        .expect("fixture git");
    assert!(
        output.status.success(),
        "fixture git {argv:?} 失败: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

async fn request_json(app: axum::Router, method: Method, uri: &str, body: Value) -> Value {
    let (status, body) = request_status_json(app, method, uri, body).await;
    assert_eq!(status, StatusCode::OK);
    body
}

async fn request_status_json(
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
    (status, serde_json::from_slice(&bytes).expect("json"))
}
