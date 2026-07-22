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
        1 => "pre_check",
        2 => "rule_config",
        3 => "mcp_configuration",
        4 => "project_rules_examples",
        _ => panic!("unexpected command index: {command_index}"),
    }
}

#[tokio::test]
async fn repository_initialization_post_returns_202_then_get_returns_completed_five_step_result() {
    let root = tempdir().unwrap();
    let repo = git_repo();
    let provider = Arc::new(ScriptedClaude::new(vec![TurnScript::Complete; 4]));
    let app = build_web_router(integration_state(root.path(), provider.clone(), None, None));
    create_project(app.clone()).await;

    let (status, accepted) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/repositories",
        json!({"name":"Repo","path":repo.path()}),
    )
    .await;

    assert_eq!(status, StatusCode::ACCEPTED, "{accepted}");
    let operation_id = accepted["operation_id"].as_str().unwrap();
    assert_eq!(accepted["status"], "created");
    assert_eq!(accepted["steps"].as_array().unwrap().len(), 5);
    assert!(
        accepted["steps"]
            .as_array()
            .unwrap()
            .iter()
            .all(|step| step["status"] == "pending")
    );

    let completed = get_operation_until_terminal(&app, "project_0001", operation_id).await;
    let canonical_repository_path = repo
        .path()
        .canonicalize()
        .expect("canonical repository path");
    let canonical_repository_path = canonical_repository_path.to_string_lossy();
    assert_eq!(completed["status"], "completed");
    assert!(
        completed["steps"]
            .as_array()
            .unwrap()
            .iter()
            .all(|step| step["status"] == "completed")
    );
    assert_eq!(
        completed["result"]["repository"]["repository_id"],
        "repository_0001"
    );
    assert_eq!(completed["result"]["repository"]["path"], "<path>");
    assert_eq!(completed["result"]["repository"]["runtime_root"], "<path>");
    let serialized_completed =
        serde_json::to_string(&completed).expect("serialize completed operation response");
    assert!(
        !serialized_completed.contains(canonical_repository_path.as_ref()),
        "completed operation response leaked repository path: {serialized_completed}"
    );
    assert_eq!(
        completed["result"]["initialization"]["commands"][0]["command"],
        "/pre-check --no-interrupt",
    );

    let inputs = provider.inputs.lock().expect("inputs");
    assert_eq!(
        inputs
            .iter()
            .map(|input| input.prompt.as_str())
            .collect::<Vec<_>>(),
        vec![
            "/pre-check --no-interrupt",
            "/rule-config --no-interrupt",
            "/mcp-configuration --no-interrupt",
            "/project-rules-examples --no-interrupt"
        ]
    );
    for input in inputs.iter() {
        assert_eq!(input.working_dir, repo.path().canonicalize().unwrap());
        assert_eq!(input.permission_mode, ProviderPermissionMode::Auto);
        assert!(input.workspace_session_id.is_none());
        assert!(input.resume_provider_session_id.is_none());
    }
}

#[tokio::test]
async fn repository_initialization_completed_operation_get_sanitizes_persisted_result_paths() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let repository_path = repo
        .path()
        .canonicalize()
        .expect("canonical repository path");
    let runtime_root = repository_path.join(".aria/runtime");
    let app = build_web_router(integration_state(
        root.path(),
        Arc::new(ScriptedClaude::new(Vec::new())),
        None,
        None,
    ));
    create_project(app.clone()).await;

    let operation_id = "repository_initialization_completed_sanitization";
    let operation_store =
        cadence_aria::product::repository_store::RepositoryInitializationOperationStore::new(
            ProductAppPaths::new(root.path().join(".aria")),
        );
    operation_store
        .create(RepositoryInitializationOperation::new(
            operation_id.to_string(),
            "project_0001".to_string(),
            RepositoryInitializationOperationInput {
                name: "Repo".to_string(),
                git_root: repository_path.clone(),
                default_policy_preset: Some("manual-write".to_string()),
                default_provider_mode: Some("claude_code".to_string()),
            },
            "2026-07-14T03:00:00Z".to_string(),
        ))
        .expect("create completed operation");
    operation_store
        .mark_running(
            "project_0001",
            operation_id,
            "2026-07-14T03:00:01Z".to_string(),
        )
        .expect("mark operation running");
    for (index, step) in RepositoryInitializationStepKind::ALL
        .into_iter()
        .enumerate()
    {
        operation_store
            .mark_step_running(
                "project_0001",
                operation_id,
                step,
                format!("2026-07-14T03:00:{:02}Z", index * 2 + 2),
            )
            .expect("mark operation step running");
        operation_store
            .mark_step_completed(
                "project_0001",
                operation_id,
                step,
                format!("2026-07-14T03:00:{:02}Z", index * 2 + 3),
            )
            .expect("mark operation step completed");
    }
    operation_store
        .finish_completed(
            "project_0001",
            operation_id,
            cadence_aria::product::repository_store::RepositoryRegistrationSuccess {
                repository: RepositoryRecord {
                    id: "repository_0001".to_string(),
                    project_id: "project_0001".to_string(),
                    name: "Repo".to_string(),
                    path: repository_path.clone(),
                    repo_hash: "repo_hash".to_string(),
                    runtime_root: runtime_root.clone(),
                    default_policy_preset: "manual-write".to_string(),
                    default_provider_mode: "claude_code".to_string(),
                    created_at: "2026-07-14T03:00:12Z".to_string(),
                    updated_at: "2026-07-14T03:00:12Z".to_string(),
                },
                cadence_skills:
                    cadence_aria::product::repository_store::CadenceSkillsPreparationSummary {
                        source_mode: "offline".to_string(),
                        source_root: PathBuf::from("/private/cadence-skills"),
                        skills_root: PathBuf::from("/private/repo/.claude/skills"),
                        git_updated: false,
                        link_sync_status: "synchronized".to_string(),
                        warnings: Vec::new(),
                    },
                initialization:
                    cadence_aria::product::repository_store::RepositoryInitializationSummary {
                        provider: "claude_code".to_string(),
                        source: PathBuf::from("/private/cadence-skills"),
                        source_mode: "offline".to_string(),
                        skills_root: PathBuf::from("/private/repo/.claude/skills"),
                        git_updated: false,
                        link_sync_status: "synchronized".to_string(),
                        commands: vec![RepositoryInitializationCommandSummary {
                            command_index: 1,
                            command: "/pre-check --no-interrupt".to_string(),
                            status: "completed".to_string(),
                            output_summary: None,
                        }],
                    },
                warnings: Vec::new(),
                changed_paths: vec![
                    "/private/repo/generated".to_string(),
                    ".claude/rules/project.md".to_string(),
                    "src/monkey.rs".to_string(),
                ],
                completed_at: "2026-07-14T03:00:12Z".to_string(),
            },
            "2026-07-14T03:00:12Z".to_string(),
        )
        .expect("finish completed operation");

    let (status, completed) = request_json(
        app,
        Method::GET,
        &format!("/api/projects/project_0001/repository-initializations/{operation_id}"),
        json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "{completed}");
    assert_eq!(completed["status"], "completed");
    assert_eq!(
        completed["result"]["initialization"]["changed_paths"],
        json!(["<path>", ".claude/rules/project.md", "src/monkey.rs"])
    );
    assert_eq!(completed["result"]["repository"]["path"], "<path>");
    assert_eq!(completed["result"]["repository"]["runtime_root"], "<path>");

    let serialized = serde_json::to_string(&completed).expect("serialize completed operation");
    assert!(
        !serialized.contains("/private/repo/generated"),
        "completed operation response leaked changed path: {serialized}"
    );
    assert!(
        !serialized.contains(repository_path.to_string_lossy().as_ref()),
        "completed operation response leaked repository path: {serialized}"
    );
    assert!(
        !serialized.contains(runtime_root.to_string_lossy().as_ref()),
        "completed operation response leaked runtime root: {serialized}"
    );
}

#[tokio::test]
async fn repository_initialization_failures_stop_and_leave_no_repository_record() {
    for fail_at in 1..=4 {
        let root = tempdir().expect("root");
        let repo = git_repo();
        let mut scripts = vec![TurnScript::Complete; fail_at - 1];
        scripts.push(TurnScript::Fail);
        let provider = Arc::new(ScriptedClaude::new(scripts));
        let app = build_web_router(integration_state(root.path(), provider.clone(), None, None));
        create_project(app.clone()).await;
        let (status, accepted) = request_json(
            app.clone(),
            Method::POST,
            "/api/projects/project_0001/repositories",
            json!({"name":"Repo","path":repo.path()}),
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED, "{accepted}");
        let operation_id = accepted["operation_id"].as_str().expect("operation id");
        let failed = get_operation_until_terminal(&app, "project_0001", operation_id).await;
        assert_eq!(failed["status"], "failed");
        assert_eq!(failed["failed_step"], command_step_id(fail_at));
        assert_eq!(failed["error"]["code"], "repository_init_command_failed");
        assert_eq!(
            failed["error"]["details"]["command"],
            command_summaries()[fail_at - 1].command
        );
        assert_eq!(provider.inputs.lock().expect("inputs").len(), fail_at);
        assert!(failed["error"]["details"]["retryable"].as_bool().unwrap());
        assert!(failed["error"]["details"]["action"].is_string());
        let (_, repositories) = request_json(
            app,
            Method::GET,
            "/api/projects/project_0001/repositories",
            json!({}),
        )
        .await;
        assert!(repositories["repositories"].as_array().unwrap().is_empty());
    }
}

#[tokio::test]
async fn repository_initialization_interaction_aborts_and_does_not_persist() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let provider = Arc::new(ScriptedClaude::new(vec![TurnScript::Interaction]));
    let app = build_web_router(integration_state(root.path(), provider.clone(), None, None));
    create_project(app.clone()).await;
    let (status, accepted) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/repositories",
        json!({"name":"Repo","path":repo.path()}),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{accepted}");
    let operation_id = accepted["operation_id"].as_str().expect("operation id");
    let failed = get_operation_until_terminal(&app, "project_0001", operation_id).await;
    assert_eq!(failed["status"], "failed");
    assert_eq!(failed["failed_step"], "pre_check");
    assert_eq!(
        failed["error"]["code"],
        "repository_init_interaction_required"
    );
    for _ in 0..10 {
        if provider
            .commands
            .lock()
            .expect("commands")
            .contains(&ProviderCommand::Abort)
        {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert!(
        provider
            .commands
            .lock()
            .expect("commands")
            .contains(&ProviderCommand::Abort)
    );
    assert_eq!(provider.inputs.lock().expect("inputs").len(), 1);
    let (_, repositories) = request_json(
        app,
        Method::GET,
        "/api/projects/project_0001/repositories",
        json!({}),
    )
    .await;
    assert!(repositories["repositories"].as_array().unwrap().is_empty());
}

struct FailingPersistence;

impl RepositoryPersistence for FailingPersistence {
    fn find_by_path(
        &self,
        _project_id: &str,
        _path: &Path,
    ) -> Result<Option<RepositoryRecord>, ProductStoreError> {
        Ok(None)
    }

    fn create_repository(
        &self,
        _input: CreateRepositoryInput,
    ) -> Result<RepositoryRecord, ProductStoreError> {
        Err(ProductStoreError::Io(
            "scripted persistence failure".to_string(),
        ))
    }
}

struct BlockingInitializer {
    started: Arc<Semaphore>,
    release: Arc<Semaphore>,
}

#[async_trait::async_trait]
impl RepositoryInitializer for BlockingInitializer {
    async fn initialize_repository(
        &self,
        _git_root: &Path,
        _command_timeout: Duration,
        _cancellation: CancellationToken,
        progress: Arc<dyn RepositoryInitializationProgress>,
    ) -> Result<Vec<RepositoryInitializationCommandSummary>, RepositoryRegistrationError> {
        self.started.add_permits(1);
        let permit = self.release.acquire().await.expect("release");
        permit.forget();
        let summaries: Result<_, Box<RepositoryRegistrationError>> =
            RepositoryInitializationStepKind::ALL
                .into_iter()
                .filter_map(|step| step.command().map(|command| (step, command)))
                .enumerate()
                .map(|(offset, (step, command))| {
                    progress.step_started(step)?;
                    progress.step_completed(step)?;
                    Ok(RepositoryInitializationCommandSummary {
                        command_index: offset + 1,
                        command: command.to_string(),
                        status: "completed".to_string(),
                        output_summary: None,
                    })
                })
                .collect();
        summaries.map_err(|error| *error)
    }
}

#[tokio::test]
async fn repository_initialization_persist_failure_and_same_path_lock_are_transactional() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let provider = Arc::new(ScriptedClaude::new(vec![TurnScript::Complete; 4]));
    let app = build_web_router(integration_state(
        root.path(),
        provider,
        Some(Arc::new(FailingPersistence)),
        None,
    ));
    create_project(app.clone()).await;
    let (status, accepted) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/repositories",
        json!({"name":"Repo","path":repo.path()}),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{accepted}");
    let operation_id = accepted["operation_id"].as_str().expect("operation id");
    let failed = get_operation_until_terminal(&app, "project_0001", operation_id).await;
    assert_eq!(failed["status"], "failed");
    assert!(
        failed["steps"]
            .as_array()
            .unwrap()
            .iter()
            .all(|step| step["status"] == "completed")
    );
    assert_eq!(failed["failed_step"], Value::Null);
    assert_eq!(
        failed["error"]["details"]["reason_code"],
        "repository_persist_failed"
    );

    let root = tempdir().expect("root");
    let repo = git_repo();
    let started = Arc::new(Semaphore::new(0));
    let release = Arc::new(Semaphore::new(0));
    let initializer = Arc::new(BlockingInitializer {
        started: started.clone(),
        release: release.clone(),
    });
    let provider = Arc::new(ScriptedClaude::new(Vec::new()));
    let app = build_web_router(integration_state(
        root.path(),
        provider,
        None,
        Some(initializer),
    ));
    create_project(app.clone()).await;
    let (status, accepted) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/repositories",
        json!({"name":"Repo","path":repo.path()}),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{accepted}");
    let operation_id = accepted["operation_id"].as_str().expect("operation id");
    let permit = started.acquire().await.expect("started");
    permit.forget();
    let (status, running) = get_operation(app.clone(), "project_0001", operation_id).await;
    assert_eq!(status, StatusCode::OK, "{running}");
    assert_eq!(running["status"], "running");
    let (status, in_progress) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/repositories",
        json!({"name":"Repo","path":repo.path()}),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(in_progress["code"], "repository_initialization_in_progress");
    release.add_permits(1);
    let completed = get_operation_until_terminal(&app, "project_0001", operation_id).await;
    assert_eq!(completed["status"], "completed");
    let (status, already_registered) = request_json(
        app,
        Method::POST,
        "/api/projects/project_0001/repositories",
        json!({"name":"Repo","path":repo.path()}),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(already_registered["code"], "repository_already_registered");
}

#[tokio::test]
async fn repository_initialization_operation_unknown_and_cross_project_are_not_found() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let provider = Arc::new(ScriptedClaude::new(vec![TurnScript::Complete; 4]));
    let app = build_web_router(integration_state(root.path(), provider, None, None));
    create_project(app.clone()).await;
    let (status, unknown) = get_operation(
        app.clone(),
        "project_0001",
        "repository_initialization_unknown",
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{unknown}");
    assert_eq!(
        unknown["code"],
        "repository_initialization_operation_not_found"
    );

    let (status, accepted) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/repositories",
        json!({"name":"Repo","path":repo.path()}),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{accepted}");
    let operation_id = accepted["operation_id"].as_str().expect("operation id");
    let (status, cross_project) = get_operation(app, "project_0002", operation_id).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{cross_project}");
    assert_eq!(
        cross_project["code"],
        "repository_initialization_operation_not_found"
    );
}

#[tokio::test]
async fn repository_initialization_stale_records_are_recovered_before_new_launch() {
    for status in [
        RepositoryInitializationOperationStatus::Created,
        RepositoryInitializationOperationStatus::Running,
    ] {
        let root = tempdir().expect("root");
        let repo = git_repo();
        let initial_provider = Arc::new(ScriptedClaude::new(Vec::new()));
        let initial_app =
            build_web_router(integration_state(root.path(), initial_provider, None, None));
        create_project(initial_app).await;
        let paths = ProductAppPaths::new(root.path().join(".aria"));
        let store =
            cadence_aria::product::repository_store::RepositoryInitializationOperationStore::new(
                paths,
            );
        let operation_id = format!("repository_initialization_stale_{status:?}").to_lowercase();
        let stale = RepositoryInitializationOperation::new(
            operation_id.clone(),
            "project_0001".to_string(),
            RepositoryInitializationOperationInput {
                name: "Stale".to_string(),
                git_root: repo.path().canonicalize().expect("canonical git root"),
                default_policy_preset: None,
                default_provider_mode: None,
            },
            "2026-07-14T03:00:00Z".to_string(),
        );
        store.create(stale).expect("stale operation");
        if status == RepositoryInitializationOperationStatus::Running {
            store
                .mark_running(
                    "project_0001",
                    &operation_id,
                    "2026-07-14T03:01:00Z".to_string(),
                )
                .expect("mark stale operation running");
            store
                .mark_step_running(
                    "project_0001",
                    &operation_id,
                    RepositoryInitializationStepKind::CadenceSkills,
                    "2026-07-14T03:01:00Z".to_string(),
                )
                .expect("mark stale operation step running");
        }

        let provider = Arc::new(ScriptedClaude::new(vec![TurnScript::Complete; 4]));
        let app = build_web_router(integration_state(root.path(), provider, None, None));
        let (get_status, recovered) =
            get_operation(app.clone(), "project_0001", &operation_id).await;
        assert_eq!(get_status, StatusCode::OK, "{recovered}");
        assert_eq!(recovered["status"], "failed");
        assert_eq!(
            recovered["error"]["details"]["reason_code"],
            "repository_initialization_interrupted"
        );

        let (post_status, accepted) = request_json(
            app,
            Method::POST,
            "/api/projects/project_0001/repositories",
            json!({"name":"Repo","path":repo.path()}),
        )
        .await;
        assert_eq!(post_status, StatusCode::ACCEPTED, "{accepted}");
    }
}

struct NoGitRunner;

struct LocalOriginRunner {
    origin: PathBuf,
    calls: Mutex<Vec<Vec<String>>>,
}

#[async_trait::async_trait]
impl BoundedCommandRunner for LocalOriginRunner {
    async fn run(
        &self,
        mut request: BoundedCommandRequest,
    ) -> Result<BoundedCommandResult, BoundedCommandError> {
        self.calls.lock().expect("calls").push(request.argv.clone());
        if request.argv.first().map(String::as_str) == Some("clone") {
            request.argv[1] = self.origin.to_string_lossy().into_owned();
        }
        TokioBoundedCommandRunner.run(request).await
    }
}

#[tokio::test]
async fn repository_initialization_cadence_online_clone_update_and_no_upstream_are_local() {
    let source_repo = git_repo();
    copy_fixture_tree(
        Path::new(CADENCE_FIXTURE),
        &source_repo.path().join("cadence-init/skills"),
    );
    run_git(source_repo.path(), &["add", "cadence-init/skills"]);
    run_git(source_repo.path(), &["commit", "--quiet", "-m", "skills"]);
    let origin_parent = tempdir().expect("origin parent");
    let origin = origin_parent.path().join("cadence-skills.git");
    let status = Command::new("git")
        .args(["init", "--bare", "--quiet"])
        .arg(&origin)
        .status()
        .expect("bare origin");
    assert!(status.success());
    let origin_text = origin.to_string_lossy().into_owned();
    run_git(
        source_repo.path(),
        &["remote", "add", "origin", &origin_text],
    );
    run_git(
        source_repo.path(),
        &["push", "--quiet", "-u", "origin", "master"],
    );

    let root = tempdir().expect("root");
    let home = root.path().join("home");
    let runner = Arc::new(LocalOriginRunner {
        origin,
        calls: Mutex::new(Vec::new()),
    });
    let environment = std::env::var("PATH")
        .ok()
        .map(|path| BTreeMap::from([("PATH".to_string(), path)]))
        .unwrap_or_default();
    let manager = CadenceSkillsManager::with_dependencies(&home, runner.clone(), environment);
    let cloned = manager
        .prepare(CancellationToken::new())
        .await
        .expect("online clone");
    assert_eq!(cloned.source_mode, CadenceSkillsSourceMode::OnlineClone);
    assert!(cloned.git_updated);

    fs::write(
        source_repo
            .path()
            .join("cadence-init/skills/alpha/SKILL.md"),
        "updated alpha\n",
    )
    .expect("updated fixture");
    run_git(
        source_repo.path(),
        &["add", "cadence-init/skills/alpha/SKILL.md"],
    );
    run_git(
        source_repo.path(),
        &["commit", "--quiet", "-m", "update alpha"],
    );
    run_git(source_repo.path(), &["push", "--quiet"]);
    let updated = manager
        .prepare(CancellationToken::new())
        .await
        .expect("online update");
    assert_eq!(updated.source_mode, CadenceSkillsSourceMode::OnlineUpdate);
    assert!(updated.git_updated);
    assert_eq!(
        fs::read_to_string(home.join(".agents/Cadence-skills/cadence-init/skills/alpha/SKILL.md"))
            .expect("updated alpha"),
        "updated alpha\n"
    );

    run_git(
        &home.join(".agents/Cadence-skills"),
        &["branch", "--unset-upstream"],
    );
    let no_upstream = manager
        .prepare(CancellationToken::new())
        .await
        .expect("no-upstream update");
    assert_eq!(
        no_upstream.source_mode,
        CadenceSkillsSourceMode::OnlineUpdate
    );
    assert!(!no_upstream.git_updated);
    assert_eq!(no_upstream.warnings, vec!["cadence_skills_no_upstream"]);
    let calls = runner.calls.lock().expect("calls");
    assert!(
        calls
            .iter()
            .any(|argv| argv.starts_with(&["fetch".to_string(), "--all".to_string()]))
    );
    assert!(
        calls
            .iter()
            .any(|argv| argv.starts_with(&["pull".to_string(), "--ff-only".to_string()]))
    );
}

#[async_trait::async_trait]
impl BoundedCommandRunner for NoGitRunner {
    async fn run(
        &self,
        request: BoundedCommandRequest,
    ) -> Result<BoundedCommandResult, BoundedCommandError> {
        panic!(
            "offline Cadence branch must not execute {}",
            request.executable
        )
    }
}

#[tokio::test]
async fn repository_initialization_cadence_offline_syncs_three_link_layers_safely() {
    let root = tempdir().expect("root");
    let home = root.path().join("home");
    let source = home.join(".agents/Cadence-skills/cadence-init/skills");
    copy_fixture_tree(Path::new(CADENCE_FIXTURE), &source);
    let manager =
        CadenceSkillsManager::with_dependencies(&home, Arc::new(NoGitRunner), BTreeMap::new());
    let result = manager
        .prepare(CancellationToken::new())
        .await
        .expect("offline prepare");
    assert_eq!(result.source_mode, CadenceSkillsSourceMode::Offline);
    assert!(!result.git_updated);
    for skill in ["alpha", "beta"] {
        let source_skill = source.join(skill);
        let shared = home.join(".agents/skills").join(skill);
        let codex = home.join(".codex/skills/skills").join(skill);
        let claude = home.join(".claude/skills").join(skill);
        assert_eq!(fs::read_link(&shared).expect("shared link"), source_skill);
        assert_eq!(fs::read_link(&codex).expect("codex link"), shared);
        assert_eq!(fs::read_link(&claude).expect("claude link"), shared);
    }
    let second = manager
        .prepare(CancellationToken::new())
        .await
        .expect("idempotent prepare");
    assert!(second.warnings.is_empty());
}

#[cfg(unix)]
#[tokio::test]
async fn repository_initialization_cadence_link_conflicts_preserve_user_content() {
    use std::os::unix::fs::symlink;

    let root = tempdir().expect("root");
    let home = root.path().join("home");
    let source = home.join(".agents/Cadence-skills/cadence-init/skills");
    copy_fixture_tree(Path::new(CADENCE_FIXTURE), &source);
    let shared_root = home.join(".agents/skills");
    let codex_root = home.join(".codex/skills/skills");
    fs::create_dir_all(&shared_root).expect("shared root");
    fs::create_dir_all(&codex_root).expect("codex root");
    let old_managed = home.join(".agents/Cadence-skills/old/skills/alpha");
    symlink(&old_managed, shared_root.join("alpha")).expect("old managed link");
    fs::write(shared_root.join("beta"), "user file\n").expect("user content");
    let unrelated = root.path().join("unrelated-alpha");
    symlink(&unrelated, codex_root.join("alpha")).expect("unrelated link");

    let manager =
        CadenceSkillsManager::with_dependencies(&home, Arc::new(NoGitRunner), BTreeMap::new());
    let result = manager
        .prepare(CancellationToken::new())
        .await
        .expect("conflict prepare");

    assert_eq!(
        fs::read_link(shared_root.join("alpha")).expect("replaced managed link"),
        source.join("alpha")
    );
    assert_eq!(
        fs::read_link(codex_root.join("alpha")).expect("preserved unrelated link"),
        unrelated
    );
    assert_eq!(
        fs::read_to_string(shared_root.join("beta")).expect("preserved user file"),
        "user file\n"
    );
    assert!(
        result
            .warnings
            .iter()
            .any(|warning| warning.contains("unrelated_symlink"))
    );
    assert!(
        result
            .warnings
            .iter()
            .any(|warning| warning.contains("user_content"))
    );
}

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
