//! 验收追踪：
//! AC-007/008 -> `repository_initialization_cadence_offline_syncs_three_link_layers_safely`
//! AC-009/010 -> `repository_initialization_post_returns_202_then_get_returns_completed_five_step_result`
//! AC-012 -> `repository_initialization_failures_stop_and_leave_no_repository_record`
//! AC-013 -> `repository_initialization_persist_failure_and_same_path_lock_are_transactional`

use std::collections::{BTreeMap, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use cadence_aria::cross_cutting::bounded_command_runner::{
    BoundedCommandError, BoundedCommandRequest, BoundedCommandResult, BoundedCommandRunner,
    TokioBoundedCommandRunner,
};
use cadence_aria::cross_cutting::provider_adapter::ProviderAdapterError;
use cadence_aria::cross_cutting::provider_availability_gate::{
    ProviderAvailabilityGate, ProviderHealthSource,
};
use cadence_aria::cross_cutting::provider_health::{ProviderHealthEntry, ProviderHealthSnapshot};
use cadence_aria::cross_cutting::provider_registry::ProviderRegistry;
use cadence_aria::cross_cutting::streaming_provider::{
    ChoiceOptionData, ChoiceRequestData, ChoiceRequestSource, ProviderCommand, ProviderCompletion,
    ProviderEvent, ProviderPermissionMode, ProviderSession, StreamingProviderAdapter,
    StreamingProviderInput,
};
use cadence_aria::product::app_paths::ProductAppPaths;
use cadence_aria::product::cadence_skills::{
    CadenceSkillsError, CadenceSkillsManager, CadenceSkillsPreparationResult,
    CadenceSkillsSourceMode, LinkSyncStatus,
};
use cadence_aria::product::json_store::ProductStoreError;
use cadence_aria::product::models::{ProviderName, RepositoryRecord};
use cadence_aria::product::repository_store::{
    CadenceSkillsPreparation, CreateRepositoryInput, RepositoryInitializationCommandSummary,
    RepositoryInitializationOperation, RepositoryInitializationOperationInput,
    RepositoryInitializationOperationStatus, RepositoryInitializationProgress,
    RepositoryInitializationStepKind, RepositoryInitializer, RepositoryPersistence,
    RepositoryRegistrationError,
};
use cadence_aria::web::app::build_web_router;
use cadence_aria::web::handlers::RepositoryRegistrationDependencies;
use cadence_aria::web::runtime::WebRuntime;
use cadence_aria::web::state::WebAppState;
use chrono::Utc;
use serde_json::{Value, json};
use tempfile::{TempDir, tempdir};
use tokio::sync::{Semaphore, mpsc};
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;

const CADENCE_FIXTURE: &str = "tests/fixtures/cadence-skills";

struct HealthySource(Arc<ProviderHealthSnapshot>);

impl HealthySource {
    fn new() -> Self {
        let checked_at = Utc::now();
        Self(Arc::new(ProviderHealthSnapshot {
            schema_version: 1,
            generation: 1,
            checked_at,
            providers: vec![ProviderHealthEntry {
                provider: ProviderName::ClaudeCode,
                command: "claude --version".to_string(),
                available: true,
                version: Some("test".to_string()),
                reason_code: None,
                reason: None,
                checked_at,
            }],
        }))
    }
}

impl ProviderHealthSource for HealthySource {
    fn snapshot(&self) -> Arc<ProviderHealthSnapshot> {
        self.0.clone()
    }

    fn degraded(&self) -> bool {
        false
    }
}

struct FixtureCadence {
    root: PathBuf,
}

#[async_trait::async_trait]
impl CadenceSkillsPreparation for FixtureCadence {
    async fn prepare_skills(
        &self,
        _cancellation: CancellationToken,
    ) -> Result<CadenceSkillsPreparationResult, CadenceSkillsError> {
        Ok(CadenceSkillsPreparationResult {
            source_mode: CadenceSkillsSourceMode::Offline,
            source_root: self.root.join("source"),
            skills_root: self.root.join("skills"),
            git_updated: false,
            link_sync_status: LinkSyncStatus::Synchronized,
            warnings: vec!["fixture_warning".to_string()],
        })
    }
}

#[derive(Clone)]
enum TurnScript {
    Complete,
    Fail,
    Interaction,
}

struct ScriptedClaude {
    scripts: Mutex<VecDeque<TurnScript>>,
    inputs: Mutex<Vec<StreamingProviderInput>>,
    commands: Arc<Mutex<Vec<ProviderCommand>>>,
}

impl ScriptedClaude {
    fn new(scripts: Vec<TurnScript>) -> Self {
        Self {
            scripts: Mutex::new(scripts.into()),
            inputs: Mutex::new(Vec::new()),
            commands: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for ScriptedClaude {
    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let script = self
            .scripts
            .lock()
            .expect("scripts")
            .pop_front()
            .expect("turn");
        let ordinal = self.inputs.lock().expect("inputs").len() + 1;
        fs::create_dir_all(input.working_dir.join(".aria-init")).expect("init dir");
        fs::write(
            input
                .working_dir
                .join(format!(".aria-init/turn-{ordinal}.txt")),
            &input.prompt,
        )
        .expect("turn output");
        self.inputs.lock().expect("inputs").push(input);
        let (event_tx, event_rx) = mpsc::channel(8);
        let (command_tx, mut command_rx) = mpsc::channel(8);
        let commands = self.commands.clone();
        tokio::spawn(async move {
            while let Some(command) = command_rx.recv().await {
                commands.lock().expect("commands").push(command);
            }
        });
        match script {
            TurnScript::Complete => event_tx
                .try_send(ProviderEvent::Completed(ProviderCompletion::plain(
                    "completed",
                    None,
                )))
                .expect("completed event"),
            TurnScript::Fail => event_tx
                .try_send(ProviderEvent::Failed {
                    message: "scripted failure".to_string(),
                })
                .expect("failed event"),
            TurnScript::Interaction => event_tx
                .try_send(ProviderEvent::ChoiceRequest(ChoiceRequestData {
                    id: "choice_0001".to_string(),
                    prompt: "choose".to_string(),
                    options: vec![ChoiceOptionData {
                        id: "option_0001".to_string(),
                        label: "Continue".to_string(),
                        description: None,
                    }],
                    allow_multiple: false,
                    allow_free_text: false,
                    questions: Vec::new(),
                    source: ChoiceRequestSource::ProviderChoice,
                }))
                .expect("interaction event"),
        }
        Ok(ProviderSession {
            events: event_rx,
            commands: command_tx,
        })
    }
}

fn integration_state(
    root: &Path,
    provider: Arc<ScriptedClaude>,
    repositories: Option<Arc<dyn RepositoryPersistence>>,
    initializer: Option<Arc<dyn RepositoryInitializer>>,
) -> WebAppState {
    let gate = Arc::new(ProviderAvailabilityGate::new(
        Arc::new(HealthySource::new()),
    ));
    let mut registry = ProviderRegistry::new();
    registry.register(ProviderName::ClaudeCode, provider);
    let registry = Arc::new(registry);
    let runner: Arc<dyn BoundedCommandRunner> = Arc::new(TokioBoundedCommandRunner);
    let mut builder = RepositoryRegistrationDependencies::builder(
        ProductAppPaths::new(root.join(".aria")),
        root.join("home"),
        runner,
        gate,
        registry,
    )
    .with_cadence_skills(Arc::new(FixtureCadence {
        root: root.join("home"),
    }))
    .with_host_readiness(Arc::new(|| Ok(())))
    .with_clock(Arc::new(|| "2026-07-14T03:00:00Z".to_string()))
    .with_timeouts(Duration::from_secs(2), Duration::from_secs(5));
    if let Some(repositories) = repositories {
        builder = builder.with_repository_persistence(repositories);
    }
    if let Some(initializer) = initializer {
        builder = builder.with_initializer(initializer);
    }
    let dependencies = builder.build().expect("integration dependencies");
    WebAppState::new(root.to_path_buf(), WebRuntime::new_fake(root.to_path_buf()))
        .with_repository_registration_dependencies(dependencies)
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

async fn create_project(app: axum::Router) {
    let (status, _) = request_json(
        app,
        Method::POST,
        "/api/projects",
        json!({"name":"Repository integration","description":null}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}

async fn get_operation_until_terminal(
    app: &axum::Router,
    project_id: &str,
    operation_id: &str,
) -> Value {
    let uri = format!("/api/projects/{project_id}/repository-initializations/{operation_id}");
    let mut last_snapshot = Value::Null;
    for _ in 0..100 {
        let (status, snapshot) = request_json(app.clone(), Method::GET, &uri, json!({})).await;
        assert_eq!(status, StatusCode::OK, "{snapshot}");
        if matches!(snapshot["status"].as_str(), Some("completed" | "failed")) {
            return snapshot;
        }
        last_snapshot = snapshot;
        tokio::task::yield_now().await;
    }
    panic!("operation did not reach a terminal state: {last_snapshot}");
}

async fn get_operation(
    app: axum::Router,
    project_id: &str,
    operation_id: &str,
) -> (StatusCode, Value) {
    request_json(
        app,
        Method::GET,
        &format!("/api/projects/{project_id}/repository-initializations/{operation_id}"),
        json!({}),
    )
    .await
}

fn command_step_id(command_index: usize) -> &'static str {
    match command_index {
        1 => "rule_config",
        2 => "pre_check",
        3 => "mcp_configuration",
        4 => "project_rules_examples",
        _ => panic!("unexpected command index: {command_index}"),
    }
}

include!("web_repository_initialization/operation_http.rs");
include!("web_repository_initialization/cadence.rs");

fn command_summaries() -> Vec<RepositoryInitializationCommandSummary> {
    RepositoryInitializationStepKind::ALL
        .into_iter()
        .filter_map(|step| step.command().map(|command| (step, command)))
        .enumerate()
        .map(
            |(offset, (_, command))| RepositoryInitializationCommandSummary {
                command_index: offset + 1,
                command: command.to_string(),
                status: "completed".to_string(),
                output_summary: None,
            },
        )
        .collect()
}

fn copy_fixture_tree(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).expect("fixture destination");
    for entry in fs::read_dir(source).expect("fixture root") {
        let entry = entry.expect("fixture entry");
        let target = destination.join(entry.file_name());
        if entry.file_type().expect("fixture type").is_dir() {
            copy_fixture_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).expect("copy fixture file");
        }
    }
}

fn git_repo() -> TempDir {
    let repo = tempdir().expect("repo");
    run_git(repo.path(), &["init", "--quiet"]);
    run_git(repo.path(), &["config", "user.name", "Aria Test"]);
    run_git(repo.path(), &["config", "user.email", "aria@example.test"]);
    fs::write(repo.path().join("README.md"), "fixture\n").expect("readme");
    run_git(repo.path(), &["add", "README.md"]);
    run_git(repo.path(), &["commit", "--quiet", "-m", "fixture"]);
    repo
}

fn run_git(cwd: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .status()
        .expect("git command");
    assert!(status.success(), "git {args:?}");
}
