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
