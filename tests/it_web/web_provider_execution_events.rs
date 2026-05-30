use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use cadence_aria::web::app::build_web_router;
use cadence_aria::web::events::EventHub;
use cadence_aria::web::issue_registry::{CreateIssueInput, IssueRegistry};
use cadence_aria::web::redaction::redact_sensitive_lines;
use cadence_aria::web::runtime::WebRuntime;
use cadence_aria::web::state::WebAppState;
use cadence_aria::web::types::CreateTaskRequest;
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
            "input_ref": "run_n05_0001",
            "redaction_applied": true
        }),
    );

    assert!(event.payload.get("input_full").is_none());
    assert!(event.payload.get("prompt").is_none());
    assert_eq!(event.payload["input_ref"], "run_n05_0001");
}

#[test]
fn redaction_matches_real_secret_keys_case_insensitively_and_preserves_clean_input() {
    let clean = "{\r\n  \"prompt\": \"safe line\"\r\n}\r\n";
    assert_eq!(redact_sensitive_lines(clean), clean);

    let redacted = redact_sensitive_lines(
        "{\r\n  \"Authorization\": \"Bearer secret\",\r\n  \"authorization\": \"Bearer lower\",\r\n  \"api-key\": \"secret\",\r\n  \"apikey\": \"secret\",\r\n  \"private-key\": \"secret\",\r\n  \"token\": \"secret\"\r\n}\r\n",
    );

    assert!(redacted.contains("[REDACTED]\r\n"));
    assert!(!redacted.contains("Bearer secret"));
    assert!(!redacted.contains("Bearer lower"));
    assert!(!redacted.contains("api-key"));
    assert!(!redacted.contains("apikey"));
    assert!(!redacted.contains("private-key"));
    assert!(!redacted.contains("\"token\": \"secret\""));
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
        "{\r\n  \"prompt\": \"safe line\",\r\n  \"Authorization\": \"Bearer secret\",\r\n  \"authorization\": \"Bearer lower\",\r\n  \"api-key\": \"secret\",\r\n  \"apikey\": \"secret\",\r\n  \"private-key\": \"secret\",\r\n  \"token\": \"secret\"\r\n}\r\n",
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
    assert!(!content.contains("Bearer lower"));
    assert!(!content.contains("api-key"));
    assert!(!content.contains("apikey"));
    assert!(!content.contains("private-key"));
    assert!(!content.contains("\"token\": \"secret\""));
}

#[tokio::test]
async fn confirm_task_prepares_route_safe_provider_input_event_and_readable_ref() {
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

    let mut setup_runtime = WebRuntime::new_fake(workspace.path().to_path_buf());
    let task = setup_runtime
        .create_task(CreateTaskRequest {
            request_text: "Implement login".to_string(),
            change_id: "login".to_string(),
            policy_preset: "manual-write".to_string(),
            provider_mode: "fake".to_string(),
            timeout_secs: 2400,
        })
        .expect("create task");

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
            &task.task_id,
            &task.session_id,
        )
        .expect("mark started");

    let state = WebAppState::new(
        app_root.path().to_path_buf(),
        WebRuntime::new_fake(app_root.path().to_path_buf()),
    );
    let app = build_web_router(state.clone());
    let prompt = "Implement login";

    let (status, _) = request_json(
        app.clone(),
        Method::POST,
        &format!(
            "/api/tasks/{}/confirm?workspace_id={}",
            task.task_id, workspace_record.workspace_id
        ),
        json!({
            "checkpoint_id": "ckpt_0001",
            "prompt": prompt,
            "policy_override": null,
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    let prepared = state
        .events
        .replay_after(0)
        .into_iter()
        .find(|event| event.event_type == "provider.input_prepared")
        .expect("provider input event");
    assert_eq!(prepared.task_id.as_deref(), Some(task.task_id.as_str()));
    assert_eq!(prepared.payload["node_id"], "N16");
    assert_eq!(prepared.payload["input_ref"], "run_n16_0001");
    assert!(prepared.payload.get("input_full").is_none());
    assert!(prepared.payload.get("prompt").is_none());
    assert!(!prepared.payload.to_string().contains("Implement login"));

    let input_ref = prepared.payload["input_ref"].as_str().expect("input ref");
    assert!(!input_ref.contains('/'));
    let (input_status, input_body) = request_json(
        app,
        Method::GET,
        &format!("/api/issues/{}/provider-inputs/{input_ref}", issue.issue_id),
        json!({}),
    )
    .await;

    assert_eq!(input_status, StatusCode::OK);
    let content = input_body["content"].as_str().expect("content");
    assert!(content.contains("Implement login"));
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

#[cfg(unix)]
#[tokio::test]
async fn provider_input_content_rejects_runtime_tasks_root_symlink_escape() {
    let (app_root, workspace, issue_id) = started_issue("task_0001");
    let runtime_root = workspace.path().join(".aria/runtime");
    fs::create_dir_all(&runtime_root).expect("runtime dir");
    let outside = tempdir().expect("outside runtime tasks");
    let outside_tasks_root = outside.path().join("tasks");
    let escaped_input_dir = outside_tasks_root.join("task_0001/provider-inputs");
    fs::create_dir_all(&escaped_input_dir).expect("escaped provider inputs dir");
    fs::write(
        escaped_input_dir.join("run_n05_0001.json"),
        "{\"prompt\":\"runtime tasks root escaped content\"}\n",
    )
    .expect("write escaped provider input");
    std::os::unix::fs::symlink(&outside_tasks_root, runtime_root.join("tasks"))
        .expect("runtime tasks root symlink");

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
    assert!(
        !body
            .to_string()
            .contains("runtime tasks root escaped content")
    );
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
