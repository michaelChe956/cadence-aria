use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use cadence_aria::cross_cutting::provider_adapter::{ProviderAdapter, ProviderAdapterError};
use cadence_aria::protocol::contracts::{AdapterInput, AdapterOutput};
use cadence_aria::web::app::build_web_router;
use cadence_aria::web::events::{EventHub, WebEventType};
use cadence_aria::web::runtime::WebRuntime;
use cadence_aria::web::state::WebAppState;
use cadence_aria::web::types::{CreateTaskRequest, ProviderOutputChunk};
use serde_json::{Value, json};
use std::fs;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Duration;
use tempfile::tempdir;
use tower::ServiceExt;

#[test]
fn provider_output_event_carries_stdout_stderr_structured_output_gate_and_retry() {
    let hub = EventHub::new();
    let event = hub.publish_provider_output(
        Some("task_0001"),
        ProviderOutputChunk {
            node_id: "N16".to_string(),
            provider_run_id: "run_n16_0001".to_string(),
            stream: "stdout".to_string(),
            text: "running tests".to_string(),
            structured_output: Some(json!({"artifact_kind":"coding_report"})),
            manual_gate: Some("approval_required".to_string()),
            retry_attempt: Some(1),
        },
    );

    assert_eq!(event.event_type, WebEventType::ProviderOutput.as_str());
    assert_eq!(event.payload["stream"], "stdout");
    assert_eq!(
        event.payload["structured_output"]["artifact_kind"],
        "coding_report"
    );
    assert_eq!(event.payload["manual_gate"], "approval_required");
    assert_eq!(event.payload["retry_attempt"], 1);
}

#[test]
fn provider_auth_failure_is_classified_for_diagnostics_panel() {
    let workspace = tempdir().expect("workspace");
    let runtime = WebRuntime::new_fake(workspace.path().to_path_buf());
    let diagnostic =
        runtime.provider_command_diagnostic("codex", "command not found or not authenticated");

    assert_eq!(diagnostic["category"], "provider_error");
    assert_eq!(
        diagnostic["code"],
        "provider_authorization_or_command_unavailable"
    );
    assert!(
        diagnostic["message"]
            .as_str()
            .expect("message")
            .contains("codex")
    );
}

#[tokio::test]
async fn stop_task_handler_returns_stop_requested_and_publishes_projection_update() {
    let workspace = tempdir().expect("workspace");
    let state = WebAppState::new(
        workspace.path().to_path_buf(),
        WebRuntime::new_fake(workspace.path().to_path_buf()),
    );
    let app = build_web_router(state.clone());

    let response = request_json(app, Method::POST, "/api/tasks/task_0001/stop", json!({})).await;

    assert_eq!(response["status"], "stop_requested");
    assert_eq!(response["task_id"], "task_0001");
    let events = state.events.replay_after(0);
    assert!(events.iter().any(|event| {
        event.event_type == WebEventType::ProjectionUpdated.as_str()
            && event.task_id.as_deref() == Some("task_0001")
            && event.payload["reason"] == "stop_requested"
    }));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn projection_handler_responds_while_real_confirm_is_running() {
    let workspace = tempdir().expect("workspace");
    fs::create_dir_all(workspace.path().join("openspec")).expect("openspec dir");
    fs::write(
        workspace.path().join("openspec/config.yaml"),
        "project: naruto\n",
    )
    .expect("openspec config");
    git(workspace.path(), &["init", "-b", "main"]);
    let (started_tx, started_rx) = mpsc::channel();
    let release = Arc::new(AtomicBool::new(false));
    let provider = BlockingProvider {
        started: started_tx,
        release: release.clone(),
    };
    let state = WebAppState::new(
        workspace.path().to_path_buf(),
        WebRuntime::new_with_provider(workspace.path().to_path_buf(), Box::new(provider)),
    );
    let app = build_web_router(state);

    let created = request_json(
        app.clone(),
        Method::POST,
        "/api/tasks",
        serde_json::to_value(CreateTaskRequest {
            request_text: "实现 climbStairs(n)".to_string(),
            change_id: "aria-climb-stairs".to_string(),
            policy_preset: "manual-write".to_string(),
            provider_mode: "real".to_string(),
            timeout_secs: 2400,
        })
        .expect("request json"),
    )
    .await;
    let task_id = created["task_id"].as_str().expect("task id").to_string();
    let advanced = request_json(
        app.clone(),
        Method::POST,
        &format!("/api/tasks/{task_id}/advance"),
        json!({}),
    )
    .await;
    let checkpoint_id = advanced["pending_step"]["checkpoint_id"]
        .as_str()
        .expect("checkpoint")
        .to_string();
    let prompt = advanced["pending_step"]["prompt"]
        .as_str()
        .expect("prompt")
        .to_string();

    let confirm_app = app.clone();
    let confirm_task_id = task_id.clone();
    let confirm_handle = tokio::spawn(async move {
        request_status(
            confirm_app,
            Method::POST,
            &format!("/api/tasks/{confirm_task_id}/confirm"),
            json!({
                "checkpoint_id": checkpoint_id,
                "prompt": prompt,
                "policy_override": null,
            }),
        )
        .await
    });
    started_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("provider started");

    let projection_result = tokio::time::timeout(
        Duration::from_millis(250),
        request_status(
            app.clone(),
            Method::GET,
            &format!("/api/projection?task_id={task_id}"),
            json!({}),
        ),
    )
    .await;
    release.store(true, Ordering::SeqCst);
    let _ = tokio::time::timeout(Duration::from_secs(2), confirm_handle).await;
    let projection = projection_result.expect("projection should not wait for provider completion");
    assert_eq!(projection, StatusCode::OK);
}

async fn request_json(app: axum::Router, method: Method, uri: &str, body: Value) -> Value {
    let response = request_response(app, method, uri, body).await;
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    serde_json::from_slice(&bytes).expect("json")
}

async fn request_status(app: axum::Router, method: Method, uri: &str, body: Value) -> StatusCode {
    request_response(app, method, uri, body).await.status()
}

async fn request_response(
    app: axum::Router,
    method: Method,
    uri: &str,
    body: Value,
) -> axum::response::Response {
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("request");
    app.oneshot(request).await.expect("response")
}

struct BlockingProvider {
    started: mpsc::Sender<()>,
    release: Arc<AtomicBool>,
}

impl ProviderAdapter for BlockingProvider {
    fn run(&self, _input: &AdapterInput) -> Result<AdapterOutput, ProviderAdapterError> {
        let _ = self.started.send(());
        while !self.release.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(10));
        }
        Err(ProviderAdapterError::execution_failed(
            Some(1),
            "blocked provider released",
            "",
            1,
        ))
    }
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
