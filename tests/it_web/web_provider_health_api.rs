//! 验收追踪：
//! AC-001/002/003 -> `provider_health_http_matrix_reports_stable_states`
//! AC-004/005 -> `provider_health_degraded_state_fails_closed_and_recheck_recovers`
//! AC-006/011 -> `provider_health_shared_gate_controls_lifecycle_coding_and_routing_entries`

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use cadence_aria::cross_cutting::aria_state_paths::AriaStatePaths;
use cadence_aria::cross_cutting::bounded_command_runner::{
    BoundedCommandError, BoundedCommandRequest, BoundedCommandResult, BoundedCommandRunner,
    TokioBoundedCommandRunner,
};
use cadence_aria::cross_cutting::provider_adapter::{ProviderAdapter, ProviderAdapterError};
use cadence_aria::cross_cutting::provider_availability_gate::ProviderAvailabilityGate;
use cadence_aria::cross_cutting::provider_health::{ProviderHealthClock, ProviderHealthService};
use cadence_aria::product::app_paths::ProductAppPaths;
use cadence_aria::product::lifecycle_store::{
    CreateWorkItemInput, CreateWorkspaceSessionInput, LifecycleStore,
};
use cadence_aria::product::models::{
    ProviderName, WorkItemPlanStatus, WorkspaceSessionStatus, WorkspaceType,
};
use cadence_aria::protocol::contracts::{
    AdapterInput, AdapterOutput, AdapterRole, ProviderType, TimeoutStatus,
};
use cadence_aria::task_run::provider_factory::RoutingProviderAdapter;
use cadence_aria::web::app::build_web_router;
use cadence_aria::web::provider_availability::{
    ProviderSelection, resolve_default_coding_provider,
};
use cadence_aria::web::runtime::WebRuntime;
use cadence_aria::web::state::WebAppState;
use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use tempfile::{TempDir, tempdir};
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;

const PROVIDER_FIXTURE: &str = "tests/fixtures/provider-health/provider-cli.sh";

struct FixedClock(DateTime<Utc>);

impl ProviderHealthClock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        self.0
    }
}

struct FixtureRunner {
    executables: Mutex<HashMap<String, PathBuf>>,
    calls: Mutex<Vec<String>>,
}

impl FixtureRunner {
    fn new() -> Self {
        Self {
            executables: Mutex::new(HashMap::new()),
            calls: Mutex::new(Vec::new()),
        }
    }

    fn map(&self, provider: &str, executable: PathBuf) {
        self.executables
            .lock()
            .expect("executables")
            .insert(provider.to_string(), executable);
    }
}

#[async_trait::async_trait]
impl BoundedCommandRunner for FixtureRunner {
    async fn run(
        &self,
        mut request: BoundedCommandRequest,
    ) -> Result<BoundedCommandResult, BoundedCommandError> {
        self.calls
            .lock()
            .expect("calls")
            .push(request.executable.clone());
        if let Some(executable) = self
            .executables
            .lock()
            .expect("executables")
            .get(&request.executable)
            .cloned()
        {
            request.executable = executable.to_string_lossy().into_owned();
        }
        TokioBoundedCommandRunner.run(request).await
    }
}

fn install_cli(root: &Path, name: &str, scenario: &str) -> PathBuf {
    let bin = root.join("bin");
    fs::create_dir_all(&bin).expect("bin");
    let executable = bin.join(name);
    fs::copy(PROVIDER_FIXTURE, &executable).expect("copy fixture");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).expect("chmod");
    }
    fs::write(bin.join(format!("{name}.scenario")), scenario).expect("scenario");
    executable
}

fn configured_runner(root: &Path, claude: &str, codex: &str) -> Arc<FixtureRunner> {
    let runner = Arc::new(FixtureRunner::new());
    runner.map(
        "claude",
        if claude == "command_missing" {
            root.join("missing/claude")
        } else {
            install_cli(root, "claude", claude)
        },
    );
    runner.map(
        "codex",
        if codex == "command_missing" {
            root.join("missing/codex")
        } else {
            install_cli(root, "codex", codex)
        },
    );
    runner.map("pi", root.join("missing/pi"));
    runner.map("kimi", root.join("missing/kimi"));
    runner
}

fn provider_state(
    root: &Path,
    runner: Arc<FixtureRunner>,
    timeout: Duration,
) -> (WebAppState, Arc<ProviderHealthService>) {
    let checked_at = "2026-07-14T02:00:00Z".parse().expect("timestamp");
    let command_runner: Arc<dyn BoundedCommandRunner> = runner;
    let health = Arc::new(ProviderHealthService::with_dependencies(
        AriaStatePaths::from_workspace_root(root),
        command_runner.clone(),
        Arc::new(FixedClock(checked_at)),
        timeout,
        4096,
    ));
    let gate = Arc::new(ProviderAvailabilityGate::new(health.clone()));
    let state = WebAppState::new(root.to_path_buf(), WebRuntime::new_fake(root.to_path_buf()))
        .with_provider_health(health.clone(), gate, command_runner);
    (state, health)
}

async fn request_json(
    app: axum::Router,
    method: Method,
    uri: &str,
    body: Value,
) -> (StatusCode, Value) {
    let response = app
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
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

async fn status_for(claude: &str, codex: &str, timeout: Duration) -> (TempDir, Value) {
    let root = tempdir().expect("root");
    let runner = configured_runner(root.path(), claude, codex);
    let (state, health) = provider_state(root.path(), runner, timeout);
    let _ = health.refresh(CancellationToken::new()).await;
    let (status, body) = request_json(
        build_web_router(state),
        Method::GET,
        "/api/providers/status",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    (root, body)
}

#[tokio::test]
async fn provider_health_http_matrix_reports_stable_states() {
    let (_root, ready) = status_for("claude_ok", "codex_ok", Duration::from_millis(200)).await;
    assert_eq!(ready["generation"], 1);
    assert_eq!(ready["state_status"], "ready");
    assert_eq!(ready["real_workflow_blocked"], false);
    assert_eq!(ready["providers"][0]["version"], "1.2.3");
    assert_eq!(ready["providers"][1]["version"], "4.5.6");
    assert_eq!(ready["providers"][0]["checked_at"], ready["checked_at"]);
    assert_eq!(ready["providers"][1]["checked_at"], ready["checked_at"]);
    assert!(
        ready["providers"]
            .as_array()
            .unwrap()
            .iter()
            .all(|entry| entry["provider"] != "fake")
    );

    let (_root, partial) =
        status_for("non_zero_exit", "codex_ok", Duration::from_millis(200)).await;
    assert_eq!(partial["providers"][0]["reason_code"], "non_zero_exit");
    assert_eq!(partial["providers"][1]["available"], true);
    assert_eq!(partial["real_workflow_blocked"], false);

    let (_root, missing_and_non_zero) = status_for(
        "command_missing",
        "non_zero_exit",
        Duration::from_millis(200),
    )
    .await;
    assert_eq!(
        missing_and_non_zero["providers"][0]["reason_code"],
        "command_missing"
    );
    assert_eq!(
        missing_and_non_zero["providers"][1]["reason_code"],
        "non_zero_exit"
    );
    assert_eq!(missing_and_non_zero["real_workflow_blocked"], true);

    let (_root, timeout_and_unparseable) =
        status_for("timeout", "version_unparseable", Duration::from_millis(30)).await;
    assert_eq!(
        timeout_and_unparseable["providers"][0]["reason_code"],
        "timeout"
    );
    assert_eq!(
        timeout_and_unparseable["providers"][1]["reason_code"],
        "version_unparseable"
    );
    assert_eq!(timeout_and_unparseable["real_workflow_blocked"], true);
}

#[tokio::test]
async fn provider_health_degraded_state_fails_closed_and_recheck_recovers() {
    let root = tempdir().expect("root");
    let runner = configured_runner(root.path(), "claude_ok", "codex_ok");
    let (state, health) = provider_state(root.path(), runner, Duration::from_millis(200));
    let health_path = health.paths().provider_health_file();
    fs::create_dir_all(&health_path).expect("blocking directory");
    let gate = state.provider_gate.clone();
    let app = build_web_router(state);

    let (status, degraded) = request_json(
        app.clone(),
        Method::POST,
        "/api/providers/recheck",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(degraded["state_status"], "degraded");
    assert!(degraded["state_error"].is_string());
    assert_eq!(degraded["real_workflow_blocked"], true);
    assert!(gate.ensure_available(&ProviderName::ClaudeCode).is_err());
    let (health_status, _) = request_json(app.clone(), Method::GET, "/api/health", json!({})).await;
    assert_eq!(health_status, StatusCode::OK);

    fs::remove_dir(&health_path).expect("remove blocking directory");
    let (_, recovered) = request_json(app, Method::POST, "/api/providers/recheck", json!({})).await;
    assert_eq!(recovered["generation"], 2);
    assert_eq!(recovered["state_status"], "ready");
    assert_eq!(recovered["real_workflow_blocked"], false);
}

struct RecordingAdapter(Arc<AtomicUsize>);

impl ProviderAdapter for RecordingAdapter {
    fn run(&self, _input: &AdapterInput) -> Result<AdapterOutput, ProviderAdapterError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(AdapterOutput {
            stdout: "ok".to_string(),
            stderr: String::new(),
            exit_code: Some(0),
            structured_output: None,
            files_modified: Vec::new(),
            duration_ms: 1,
            timeout_status: TimeoutStatus::NotTimedOut,
        })
    }
}

#[tokio::test]
async fn provider_health_shared_gate_controls_lifecycle_coding_and_routing_entries() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let runner = configured_runner(root.path(), "claude_ok", "non_zero_exit");
    let (state, health) = provider_state(root.path(), runner, Duration::from_millis(200));
    health
        .refresh(CancellationToken::new())
        .await
        .expect("persist health");
    let gate = state.provider_gate.clone();
    let fallback = resolve_default_coding_provider("codex", |provider| {
        gate.ensure_available(provider).is_ok()
    })
    .expect("healthy fallback");
    assert_eq!(fallback.provider, ProviderName::ClaudeCode);
    assert_eq!(
        fallback.selection,
        ProviderSelection::Fallback {
            requested: ProviderName::Codex,
            fallback: ProviderName::ClaudeCode,
        }
    );
    let calls = Arc::new(AtomicUsize::new(0));
    let routing = RoutingProviderAdapter::new_with_gate(
        Box::new(RecordingAdapter(calls.clone())),
        Box::new(RecordingAdapter(calls.clone())),
        gate,
    );
    let error = routing
        .run(&AdapterInput {
            provider_type: ProviderType::Codex,
            role: AdapterRole::Executor,
            worktree_path: Some(root.path().to_string_lossy().into_owned()),
            provider_stream_log_dir: None,
            prompt: "run".to_string(),
            context_files: Vec::new(),
            output_schema: String::new(),
            timeout: 1,
            max_retries: 0,
        })
        .expect_err("codex blocked");
    assert!(error.to_string().contains("provider_unavailable"));
    assert_eq!(calls.load(Ordering::SeqCst), 0);

    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let app = build_web_router(state);
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects",
        json!({"name":"Gate","description":null}),
    )
    .await;
    crate::create_repository_and_wait(
        app.clone(),
        "project_0001",
        json!({"name":"Repo","path":repo.path()}),
    )
    .await;
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues",
        json!({"title":"Gate","description":null,"repository_id":"repository_0001"}),
    )
    .await;
    let (status, lifecycle_error) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/story-specs:generate",
        json!({"title":"Blocked","author_provider":"codex","reviewer_provider":"claude_code"}),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(lifecycle_error["code"], "provider_unavailable");

    let lifecycle = LifecycleStore::new(app_paths);
    lifecycle
        .create_work_item(CreateWorkItemInput {
            id: Some("work_item_0001".to_string()),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            title: "Blocked coding".to_string(),
            plan_status: WorkItemPlanStatus::Confirmed,
            ..Default::default()
        })
        .expect("work item");
    let session = lifecycle
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: "work_item_0001".to_string(),
            workspace_type: WorkspaceType::WorkItem,
            author_provider: ProviderName::Codex,
            reviewer_provider: ProviderName::ClaudeCode,
            review_rounds: 1,
            superpowers_enabled: false,
            openspec_enabled: false,
        })
        .expect("session");
    lifecycle
        .update_workspace_session_status(&session.id, WorkspaceSessionStatus::Confirmed)
        .expect("confirm session");
    let (status, coding_error) = request_json(
        app,
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0001/coding-attempts",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(coding_error["code"], "provider_unavailable");
}

fn git_repo() -> TempDir {
    let repo = tempdir().expect("repo");
    let status = Command::new("git")
        .args(["init", "--quiet"])
        .current_dir(repo.path())
        .status()
        .expect("git init");
    assert!(status.success());
    repo
}
