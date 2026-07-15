//! 验收追踪：
//! AC-007/008 -> `repository_initialization_cadence_offline_syncs_three_link_layers_safely`
//! AC-009/010 -> `repository_initialization_http_success_runs_four_independent_claude_turns`
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
    ChoiceOptionData, ChoiceRequestData, ChoiceRequestSource, ProviderCommand, ProviderEvent,
    ProviderPermissionMode, ProviderSession, StreamingProviderAdapter, StreamingProviderInput,
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
    RepositoryInitializer, RepositoryPersistence, RepositoryRegistrationError,
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
                .try_send(ProviderEvent::Completed {
                    full_output: "completed".to_string(),
                    provider_session_id: None,
                })
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

#[tokio::test]
async fn repository_initialization_http_success_runs_four_independent_claude_turns() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let provider = Arc::new(ScriptedClaude::new(vec![TurnScript::Complete; 4]));
    let app = build_web_router(integration_state(root.path(), provider.clone(), None, None));
    create_project(app.clone()).await;
    let (status, body) = request_json(
        app,
        Method::POST,
        "/api/projects/project_0001/repositories",
        json!({"name":"Repo","path":repo.path(),"default_provider_mode":"claude_code"}),
    )
    .await;

    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["repository"]["repository_id"], "repository_0001");
    assert_eq!(body["initialization"]["source"], "offline");
    assert_eq!(
        body["initialization"]["commands"].as_array().unwrap().len(),
        4
    );
    assert_eq!(
        body["initialization"]["warnings"],
        json!(["fixture_warning"])
    );
    assert_eq!(
        body["initialization"]["completed_at"],
        "2026-07-14T03:00:00Z"
    );
    assert_eq!(
        body["initialization"]["changed_paths"]
            .as_array()
            .unwrap()
            .len(),
        4
    );
    let inputs = provider.inputs.lock().expect("inputs");
    assert_eq!(
        inputs
            .iter()
            .map(|input| input.prompt.as_str())
            .collect::<Vec<_>>(),
        vec![
            "/pre-check",
            "/rule-config",
            "/mcp-configuration",
            "/project-rules-examples"
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
async fn repository_initialization_failures_stop_and_leave_no_repository_record() {
    for fail_at in 1..=4 {
        let root = tempdir().expect("root");
        let repo = git_repo();
        let mut scripts = vec![TurnScript::Complete; fail_at - 1];
        scripts.push(TurnScript::Fail);
        let provider = Arc::new(ScriptedClaude::new(scripts));
        let app = build_web_router(integration_state(root.path(), provider.clone(), None, None));
        create_project(app.clone()).await;
        let (status, error) = request_json(
            app.clone(),
            Method::POST,
            "/api/projects/project_0001/repositories",
            json!({"name":"Repo","path":repo.path()}),
        )
        .await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{error}");
        assert_eq!(error["code"], "repository_init_command_failed");
        assert_eq!(
            error["details"]["command"],
            command_summaries()[fail_at - 1].command
        );
        assert_eq!(provider.inputs.lock().expect("inputs").len(), fail_at);
        assert_eq!(
            error["details"]["changed_paths"].as_array().unwrap().len(),
            fail_at
        );
        assert!(error["details"]["retryable"].as_bool().unwrap());
        assert!(error["details"]["action"].is_string());
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
    let (status, error) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/repositories",
        json!({"name":"Repo","path":repo.path()}),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(error["code"], "repository_init_interaction_required");
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
    ) -> Result<Vec<RepositoryInitializationCommandSummary>, RepositoryRegistrationError> {
        self.started.add_permits(1);
        let permit = self.release.acquire().await.expect("release");
        permit.forget();
        Ok(command_summaries())
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
    let (status, error) = request_json(
        app,
        Method::POST,
        "/api/projects/project_0001/repositories",
        json!({"name":"Repo","path":repo.path()}),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{error}");
    assert_eq!(error["code"], "repository_persist_failed");
    assert_eq!(
        error["details"]["changed_paths"].as_array().unwrap().len(),
        4
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
    let first_app = app.clone();
    let repo_path = repo.path().to_path_buf();
    let first = tokio::spawn(async move {
        request_json(
            first_app,
            Method::POST,
            "/api/projects/project_0001/repositories",
            json!({"name":"Repo","path":repo_path}),
        )
        .await
    });
    let permit = started.acquire().await.expect("started");
    permit.forget();
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
    let (status, _) = first.await.expect("first request");
    assert_eq!(status, StatusCode::CREATED);
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
    [
        "/pre-check",
        "/rule-config",
        "/mcp-configuration",
        "/project-rules-examples",
    ]
    .into_iter()
    .enumerate()
    .map(|(offset, command)| RepositoryInitializationCommandSummary {
        command_index: offset + 1,
        command: command.to_string(),
        status: "completed".to_string(),
        output_summary: None,
    })
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
