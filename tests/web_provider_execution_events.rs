use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use cadence_aria::web::app::build_web_router;
use cadence_aria::web::events::EventHub;
use cadence_aria::web::issue_registry::{CreateIssueInput, IssueRegistry};
use cadence_aria::web::runtime::WebRuntime;
use cadence_aria::web::state::WebAppState;
use cadence_aria::web::workspace_registry::{CreateWorkspaceInput, WorkspaceRegistry};
use serde_json::{Value, json};
use std::fs;
use std::process::Command;
use tempfile::tempdir;
use tower::ServiceExt;

#[test]
fn replay_after_cursor_returns_only_later_events() {
    let hub = EventHub::new();
    let first = hub.publish(
        "provider.input_prepared",
        None,
        json!({"input_summary":"summary"}),
    );
    let second = hub.publish("provider.output_stream", None, json!({"text":"chunk"}));

    let replay = hub.replay_after(first.cursor);

    assert_eq!(replay.len(), 1);
    assert_eq!(replay[0].cursor, second.cursor);
    assert_eq!(replay[0].event_type, "provider.output_stream");
}

#[test]
fn input_prepared_event_uses_reference_not_full_prompt() {
    let hub = EventHub::new();
    let event = hub.publish(
        "provider.input_prepared",
        Some("task_0001"),
        json!({
            "node_id": "N05",
            "input_summary": "story spec generation",
            "input_full_ref": "provider-inputs/run_n05_0001.json",
            "redaction_applied": true
        }),
    );

    assert!(event.payload.get("input_full").is_none());
    assert_eq!(
        event.payload["input_full_ref"],
        "provider-inputs/run_n05_0001.json"
    );
}

#[tokio::test]
async fn provider_input_content_reads_issue_task_input_and_redacts_secret_lines() {
    let app_root = tempdir().expect("app root");
    let workspace = tempdir().expect("workspace");
    git(workspace.path(), &["init", "-b", "main"]);

    let workspace_record = WorkspaceRegistry::new(app_root.path().to_path_buf())
        .create(CreateWorkspaceInput {
            name: "Issue workspace".to_string(),
            path: workspace.path().to_path_buf(),
            default_policy_preset: None,
            default_provider_mode: None,
        })
        .expect("workspace");
    let issue_registry = IssueRegistry::new(app_root.path().to_path_buf());
    let issue = issue_registry
        .create(CreateIssueInput {
            title: "Prepare provider input".to_string(),
            description: None,
            change_id: None,
        })
        .expect("issue");
    issue_registry
        .mark_started(
            &issue.issue_id,
            &workspace_record.workspace_id,
            "task_0001",
            "sess_task_0001",
        )
        .expect("mark started");
    let input_dir = workspace
        .path()
        .join(".aria/runtime/tasks/task_0001/provider-inputs");
    fs::create_dir_all(&input_dir).expect("provider inputs dir");
    fs::write(
        input_dir.join("run_n05_0001.json"),
        "{\n  \"prompt\": \"safe line\",\n  \"Authorization: Bearer secret\": true,\n  \"api_key\": \"secret\"\n}\n",
    )
    .expect("write provider input");

    let state = WebAppState::new(
        app_root.path().to_path_buf(),
        WebRuntime::new_fake(app_root.path().to_path_buf()),
    );
    let app = build_web_router(state);

    let (status, body) = request_json(
        app,
        Method::GET,
        &format!(
            "/api/issues/{}/provider-inputs/run_n05_0001",
            issue.issue_id
        ),
        json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["input_ref"], "run_n05_0001");
    let content = body["content"].as_str().expect("content");
    assert!(content.contains("\"prompt\": \"safe line\""));
    assert!(content.contains("[REDACTED]"));
    assert!(!content.contains("Bearer secret"));
    assert!(!content.contains("\"api_key\": \"secret\""));
}

#[tokio::test]
async fn provider_input_content_rejects_issue_task_id_path_escape() {
    let (app_root, workspace, issue_id) = started_issue("../../outside");
    fs::create_dir_all(workspace.path().join(".aria/runtime/tasks")).expect("runtime tasks dir");
    let escaped_dir = workspace.path().join(".aria/outside/provider-inputs");
    fs::create_dir_all(&escaped_dir).expect("escaped provider inputs dir");
    fs::write(
        escaped_dir.join("run_n05_0001.json"),
        "{\"prompt\":\"escaped content\"}\n",
    )
    .expect("write escaped provider input");

    let state = WebAppState::new(
        app_root.path().to_path_buf(),
        WebRuntime::new_fake(app_root.path().to_path_buf()),
    );
    let app = build_web_router(state);

    let (status, body) = request_json(
        app,
        Method::GET,
        &format!("/api/issues/{issue_id}/provider-inputs/run_n05_0001"),
        json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "invalid_task_id");
    assert!(!body.to_string().contains("escaped content"));
}

#[cfg(unix)]
#[tokio::test]
async fn provider_input_content_rejects_provider_input_symlink_escape() {
    let (app_root, workspace, issue_id) = started_issue("task_0001");
    let input_dir = workspace
        .path()
        .join(".aria/runtime/tasks/task_0001/provider-inputs");
    fs::create_dir_all(&input_dir).expect("provider inputs dir");
    let escaped_file = workspace.path().join("outside-provider-input.json");
    fs::write(&escaped_file, "{\"prompt\":\"symlink escaped content\"}\n")
        .expect("write escaped provider input");
    std::os::unix::fs::symlink(&escaped_file, input_dir.join("run_n05_0001.json"))
        .expect("provider input symlink");

    let state = WebAppState::new(
        app_root.path().to_path_buf(),
        WebRuntime::new_fake(app_root.path().to_path_buf()),
    );
    let app = build_web_router(state);

    let (status, body) = request_json(
        app,
        Method::GET,
        &format!("/api/issues/{issue_id}/provider-inputs/run_n05_0001"),
        json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "provider_input_path_escape");
    assert!(!body.to_string().contains("symlink escaped content"));
}

#[cfg(unix)]
#[tokio::test]
async fn provider_input_content_rejects_task_root_symlink_escape() {
    let (app_root, workspace, issue_id) = started_issue("task_0001");
    let runtime_tasks_root = workspace.path().join(".aria/runtime/tasks");
    fs::create_dir_all(&runtime_tasks_root).expect("runtime tasks dir");
    let escaped_input_dir = workspace.path().join("outside-task/provider-inputs");
    fs::create_dir_all(&escaped_input_dir).expect("escaped task provider inputs dir");
    fs::write(
        escaped_input_dir.join("run_n05_0001.json"),
        "{\"prompt\":\"task root escaped content\"}\n",
    )
    .expect("write escaped provider input");
    std::os::unix::fs::symlink(
        workspace.path().join("outside-task"),
        runtime_tasks_root.join("task_0001"),
    )
    .expect("task root symlink");

    let state = WebAppState::new(
        app_root.path().to_path_buf(),
        WebRuntime::new_fake(app_root.path().to_path_buf()),
    );
    let app = build_web_router(state);

    let (status, body) = request_json(
        app,
        Method::GET,
        &format!("/api/issues/{issue_id}/provider-inputs/run_n05_0001"),
        json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "provider_input_path_escape");
    assert!(!body.to_string().contains("task root escaped content"));
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
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).expect("json")
    };
    (status, value)
}

fn started_issue(task_id: &str) -> (tempfile::TempDir, tempfile::TempDir, String) {
    let app_root = tempdir().expect("app root");
    let workspace = tempdir().expect("workspace");
    git(workspace.path(), &["init", "-b", "main"]);

    let workspace_record = WorkspaceRegistry::new(app_root.path().to_path_buf())
        .create(CreateWorkspaceInput {
            name: "Issue workspace".to_string(),
            path: workspace.path().to_path_buf(),
            default_policy_preset: None,
            default_provider_mode: None,
        })
        .expect("workspace");
    let issue_registry = IssueRegistry::new(app_root.path().to_path_buf());
    let issue = issue_registry
        .create(CreateIssueInput {
            title: "Prepare provider input".to_string(),
            description: None,
            change_id: None,
        })
        .expect("issue");
    issue_registry
        .mark_started(
            &issue.issue_id,
            &workspace_record.workspace_id,
            task_id,
            "sess_task_0001",
        )
        .expect("mark started");

    (app_root, workspace, issue.issue_id)
}

fn git(cwd: &std::path::Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("git");
    assert!(
        output.status.success(),
        "git {:?} failed stdout={} stderr={}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
