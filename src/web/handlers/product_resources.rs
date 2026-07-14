use super::dto::*;
use super::support::*;
use super::*;

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use tokio_util::sync::CancellationToken;

use crate::cross_cutting::bounded_command_runner::BoundedCommandRunner;
use crate::cross_cutting::provider_availability_gate::{
    ProviderAvailabilityGate, ProviderHealthSource,
};
use crate::cross_cutting::provider_health::{ProviderHealthEntry, ProviderHealthSnapshot};
use crate::cross_cutting::provider_registry::ProviderRegistry;
use crate::product::cadence_skills::{
    CadenceSkillsManager, CadenceSkillsPreparationResult, CadenceSkillsSourceMode, LinkSyncStatus,
};
use crate::product::repository_store::{
    CadenceSkillsPreparation, ClaudeRepositoryInitializer, RepositoryInitializationCommandSummary,
    RepositoryInitializer, RepositoryRegistrationCoordinator, RepositoryRegistrationError,
    RepositoryRegistrationInput, RepositoryRegistrationSuccess,
};

pub async fn list_workspaces(
    State(state): State<WebAppState>,
) -> ApiResult<Json<WorkspaceListResponse>> {
    let registry = WorkspaceRegistry::new(state.workspace_root.clone());
    let workspaces = registry.ensure_default_workspace()?;
    Ok(Json(WorkspaceListResponse {
        workspaces: workspaces.into_iter().map(workspace_dto).collect(),
    }))
}

pub async fn create_workspace(
    State(state): State<WebAppState>,
    Json(request): Json<CreateWorkspaceRequest>,
) -> ApiResult<Json<WorkspaceDto>> {
    let registry = WorkspaceRegistry::new(state.workspace_root.clone());
    let workspace = registry.create(CreateWorkspaceInput {
        name: request.name,
        path: request.path.into(),
        default_policy_preset: request.default_policy_preset,
        default_provider_mode: request.default_provider_mode,
    })?;
    Ok(Json(workspace_dto(workspace)))
}

pub async fn delete_workspace(
    State(state): State<WebAppState>,
    Path(workspace_id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    let registry = WorkspaceRegistry::new(state.workspace_root.clone());
    registry.delete(&workspace_id)?;
    Ok(Json(json!({"status":"deleted"})))
}

pub async fn list_projects(
    State(state): State<WebAppState>,
) -> ApiResult<Json<ProjectListResponse>> {
    let store = ProjectStore::new(product_app_paths(&state));
    let projects = store.list().map_err(product_store_api_error)?;
    Ok(Json(ProjectListResponse {
        projects: projects.into_iter().map(project_dto).collect(),
    }))
}

pub async fn create_project(
    State(state): State<WebAppState>,
    Json(request): Json<CreateProjectRequest>,
) -> ApiResult<Json<ProjectDto>> {
    let store = ProjectStore::new(product_app_paths(&state));
    let project = store
        .create(CreateProjectInput {
            name: request.name,
            description: request.description,
        })
        .map_err(product_store_api_error)?;
    Ok(Json(project_dto(project)))
}

pub async fn get_project(
    State(state): State<WebAppState>,
    Path(project_id): Path<String>,
) -> ApiResult<Json<ProjectDto>> {
    let store = ProjectStore::new(product_app_paths(&state));
    let project = store.get(&project_id).map_err(product_store_api_error)?;
    Ok(Json(project_dto(project)))
}

pub async fn open_project(
    State(state): State<WebAppState>,
    Path(project_id): Path<String>,
) -> ApiResult<Json<ProjectDto>> {
    let store = ProjectStore::new(product_app_paths(&state));
    let project = store.open(&project_id).map_err(product_store_api_error)?;
    Ok(Json(project_dto(project)))
}

pub async fn delete_project(
    State(state): State<WebAppState>,
    Path(project_id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    let store = ProjectStore::new(product_app_paths(&state));
    store.delete(&project_id).map_err(product_store_api_error)?;
    Ok(Json(json!({"status":"deleted"})))
}

pub async fn list_repositories(
    State(state): State<WebAppState>,
    Path(project_id): Path<String>,
) -> ApiResult<Json<RepositoryListResponse>> {
    let store = RepositoryStore::new(product_app_paths(&state));
    let repositories = store.list(&project_id).map_err(product_store_api_error)?;
    Ok(Json(RepositoryListResponse {
        repositories: repositories.into_iter().map(repository_dto).collect(),
    }))
}

#[async_trait::async_trait]
trait RepositoryRegistrar: Send + Sync {
    async fn register(
        &self,
        input: RepositoryRegistrationInput,
    ) -> Result<RepositoryRegistrationSuccess, RepositoryRegistrationError>;
}

#[async_trait::async_trait]
impl RepositoryRegistrar for RepositoryRegistrationCoordinator {
    async fn register(
        &self,
        input: RepositoryRegistrationInput,
    ) -> Result<RepositoryRegistrationSuccess, RepositoryRegistrationError> {
        RepositoryRegistrationCoordinator::register(self, input, CancellationToken::new()).await
    }
}

#[derive(Clone)]
struct RepositoryRegistrationRequestDependencies {
    app_paths: ProductAppPaths,
    runner: Arc<dyn BoundedCommandRunner>,
    gate: Arc<ProviderAvailabilityGate>,
    registry: Arc<ProviderRegistry>,
    home: PathBuf,
    fake_runtime: bool,
}

type RepositoryRegistrationBuildResult<T> = Result<T, Box<RepositoryRegistrationError>>;

struct RepositoryRegistrationRuntimeDependencies {
    gate: Arc<ProviderAvailabilityGate>,
    cadence_skills: Arc<dyn CadenceSkillsPreparation>,
    host_readiness: Arc<dyn Fn() -> Result<(), String> + Send + Sync>,
    initializer: Arc<dyn RepositoryInitializer>,
}

fn request_repository_registration_dependencies_with_home<F>(
    state: &WebAppState,
    home_resolver: F,
) -> RepositoryRegistrationBuildResult<RepositoryRegistrationRequestDependencies>
where
    F: FnOnce() -> RepositoryRegistrationBuildResult<PathBuf>,
{
    let app_paths = product_app_paths(state);
    let runner = state.command_runner.clone();
    let gate = state.provider_gate.clone();
    let registry = state.provider_registry.clone();
    let fake_runtime = !state
        .runtime
        .lock()
        .expect("web runtime lock")
        .enforces_real_provider_availability();
    let home = validate_user_home(home_resolver()?)?;
    Ok(RepositoryRegistrationRequestDependencies {
        app_paths,
        runner,
        gate,
        registry,
        home,
        fake_runtime,
    })
}

fn request_repository_registration_dependencies(
    state: &WebAppState,
) -> RepositoryRegistrationBuildResult<RepositoryRegistrationRequestDependencies> {
    let fake_runtime = !state
        .runtime
        .lock()
        .expect("web runtime lock")
        .enforces_real_provider_availability();
    let test_home = state.workspace_root.clone();
    request_repository_registration_dependencies_with_home(state, move || {
        if fake_runtime {
            Ok(test_home)
        } else {
            resolve_user_home_with(|name| std::env::var_os(name))
        }
    })
}

fn resolve_user_home_with<F>(mut read_environment: F) -> RepositoryRegistrationBuildResult<PathBuf>
where
    F: FnMut(&str) -> Option<OsString>,
{
    let home = read_environment("HOME").filter(|value| !value.is_empty());
    let selected = home
        .or_else(|| read_environment("USERPROFILE").filter(|value| !value.is_empty()))
        .ok_or_else(|| home_resolution_error("HOME and USERPROFILE are missing or empty"))?;
    validate_user_home(PathBuf::from(selected))
}

fn validate_user_home(home: PathBuf) -> RepositoryRegistrationBuildResult<PathBuf> {
    if home.as_os_str().is_empty() || !home.is_absolute() {
        return Err(home_resolution_error(
            "the user home directory must be a non-empty absolute path",
        ));
    }
    Ok(home)
}

fn home_resolution_error(reason: &str) -> Box<RepositoryRegistrationError> {
    Box::new(RepositoryRegistrationError {
        stage: "cadence_skills_home".to_string(),
        provider: None,
        command_index: None,
        command: None,
        reason_code: "cadence_skills_unavailable".to_string(),
        stderr_summary: Some(reason.to_string()),
        changed_paths: Some(Vec::new()),
        retryable: false,
        action: "Configure HOME or USERPROFILE with an absolute user directory, then add the repository again."
            .to_string(),
    })
}

fn build_repository_registrar(
    dependencies: RepositoryRegistrationRequestDependencies,
) -> Arc<dyn RepositoryRegistrar> {
    // 持久化输入仍由协调器独占构造，handler 不创建该类型。
    let _coordinator_owned_persistence_input = std::marker::PhantomData::<CreateRepositoryInput>;
    let projects = Arc::new(ProjectStore::new(dependencies.app_paths.clone()));
    let repositories = Arc::new(RepositoryStore::new(dependencies.app_paths));
    let runtime_dependencies = if dependencies.fake_runtime {
        RepositoryRegistrationRuntimeDependencies {
            gate: fake_repository_registration_gate(),
            cadence_skills: Arc::new(FakeCadenceSkillsPreparation {
                home: dependencies.home,
            }),
            host_readiness: Arc::new(|| Ok(())),
            initializer: Arc::new(CompletedRepositoryInitializer),
        }
    } else {
        let gate = dependencies.gate.clone();
        let manager = CadenceSkillsManager::with_dependencies(
            dependencies.home,
            dependencies.runner.clone(),
            command_environment(),
        );
        RepositoryRegistrationRuntimeDependencies {
            gate,
            cadence_skills: Arc::new(manager),
            host_readiness: Arc::new(|| {
                crate::web::provider_availability::host_real_workflow_ready()
                    .map_err(|error| error.message)
            }),
            initializer: Arc::new(ClaudeRepositoryInitializer::new(
                dependencies.gate.clone(),
                dependencies.registry.clone(),
                4 * 1024,
            )),
        }
    };

    Arc::new(RepositoryRegistrationCoordinator::new(
        projects,
        repositories,
        runtime_dependencies.gate,
        dependencies.registry,
        runtime_dependencies.cadence_skills,
        runtime_dependencies.host_readiness,
        dependencies.runner,
        Arc::new(|| Utc::now().to_rfc3339()),
        runtime_dependencies.initializer,
        Duration::from_secs(180),
        Duration::from_secs(300),
    ))
}

fn command_environment() -> BTreeMap<String, String> {
    std::env::var("PATH")
        .ok()
        .map(|path| BTreeMap::from([("PATH".to_string(), path)]))
        .unwrap_or_default()
}

struct FixedAvailableProviderHealth(Arc<ProviderHealthSnapshot>);

impl ProviderHealthSource for FixedAvailableProviderHealth {
    fn snapshot(&self) -> Arc<ProviderHealthSnapshot> {
        self.0.clone()
    }

    fn degraded(&self) -> bool {
        false
    }
}

fn fake_repository_registration_gate() -> Arc<ProviderAvailabilityGate> {
    let checked_at = Utc::now();
    let snapshot = ProviderHealthSnapshot {
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
    };
    Arc::new(ProviderAvailabilityGate::new(Arc::new(
        FixedAvailableProviderHealth(Arc::new(snapshot)),
    )))
}

struct FakeCadenceSkillsPreparation {
    home: PathBuf,
}

#[async_trait::async_trait]
impl CadenceSkillsPreparation for FakeCadenceSkillsPreparation {
    async fn prepare_skills(
        &self,
        _cancellation: CancellationToken,
    ) -> Result<CadenceSkillsPreparationResult, crate::product::cadence_skills::CadenceSkillsError>
    {
        Ok(CadenceSkillsPreparationResult {
            source_mode: CadenceSkillsSourceMode::Offline,
            source_root: self.home.join("cadence-skills-source"),
            skills_root: self.home.join("cadence-skills"),
            git_updated: false,
            link_sync_status: LinkSyncStatus::Synchronized,
            warnings: Vec::new(),
        })
    }
}

struct CompletedRepositoryInitializer;

#[async_trait::async_trait]
impl RepositoryInitializer for CompletedRepositoryInitializer {
    async fn initialize_repository(
        &self,
        _git_root: &StdPath,
        _command_timeout: Duration,
        _cancellation: CancellationToken,
    ) -> Result<Vec<RepositoryInitializationCommandSummary>, RepositoryRegistrationError> {
        Ok([
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
        .collect())
    }
}

pub async fn create_repository(
    State(state): State<WebAppState>,
    Path(project_id): Path<String>,
    Json(request): Json<CreateRepositoryRequest>,
) -> ApiResult<Response> {
    let dependencies = request_repository_registration_dependencies(&state)
        .map_err(|error| ApiError::from(*error))?;
    let registrar = build_repository_registrar(dependencies);
    create_repository_with_registrar(registrar.as_ref(), project_id, request).await
}

async fn create_repository_with_registrar(
    registrar: &dyn RepositoryRegistrar,
    project_id: String,
    request: CreateRepositoryRequest,
) -> ApiResult<Response> {
    let success = registrar
        .register(RepositoryRegistrationInput {
            project_id,
            name: request.name,
            path: request.path.into(),
            default_policy_preset: request.default_policy_preset,
            default_provider_mode: request.default_provider_mode,
        })
        .await
        .map_err(ApiError::from)?;
    Ok((
        StatusCode::CREATED,
        Json(repository_registration_response(success)),
    )
        .into_response())
}

pub async fn delete_repository(
    State(state): State<WebAppState>,
    Path((project_id, repository_id)): Path<(String, String)>,
) -> ApiResult<Json<serde_json::Value>> {
    let store = RepositoryStore::new(product_app_paths(&state));
    store
        .delete(&project_id, &repository_id)
        .map_err(product_store_api_error)?;
    Ok(Json(json!({"status":"deleted"})))
}

pub async fn list_product_issues(
    State(state): State<WebAppState>,
    Path(project_id): Path<String>,
) -> ApiResult<Json<ProductIssueListResponse>> {
    let store = IssueStore::new(product_app_paths(&state));
    let issues = store.list(&project_id).map_err(product_store_api_error)?;
    Ok(Json(ProductIssueListResponse {
        issues: issues
            .into_iter()
            .map(|issue| product_issue_dto_with_binding(&product_app_paths(&state), issue))
            .collect::<ApiResult<Vec<_>>>()?,
    }))
}

pub async fn create_product_issue(
    State(state): State<WebAppState>,
    Path(project_id): Path<String>,
    Json(request): Json<CreateProductIssueRequest>,
) -> ApiResult<Json<ProductIssueDto>> {
    let repository_id = request
        .repository_id
        .ok_or_else(|| ApiError::validation("repository_required", "repository_id is required"))?;
    let app_paths = product_app_paths(&state);
    let _repository = find_repository(&app_paths, &project_id, &repository_id)?;
    let store = IssueStore::new(app_paths);
    let issue = store
        .create_with_repository(CreateProductIssueWithRepositoryInput {
            project_id,
            repo_id: repository_id,
            title: request.title,
            description: request.description,
            change_id: request.change_id,
        })
        .map_err(product_store_api_error)?;
    Ok(Json(product_issue_dto(issue, None)))
}

pub async fn delete_product_issue(
    State(state): State<WebAppState>,
    Path((project_id, issue_id)): Path<(String, String)>,
) -> ApiResult<Json<serde_json::Value>> {
    let store = IssueStore::new(product_app_paths(&state));
    store
        .delete(&project_id, &issue_id)
        .map_err(product_store_api_error)?;
    Ok(Json(json!({"status":"deleted"})))
}

pub async fn list_issues(State(state): State<WebAppState>) -> ApiResult<Json<IssueListResponse>> {
    let registry = IssueRegistry::new(state.workspace_root.clone());
    let issues = registry.list()?;
    Ok(Json(IssueListResponse {
        issues: issues.into_iter().map(issue_dto).collect(),
    }))
}

pub async fn create_issue(
    State(state): State<WebAppState>,
    Json(request): Json<CreateIssueRequest>,
) -> ApiResult<Json<IssueDto>> {
    let registry = IssueRegistry::new(state.workspace_root.clone());
    let issue = registry.create(CreateIssueInput {
        title: request.title,
        description: request.description,
        change_id: request.change_id,
    })?;
    Ok(Json(issue_dto(issue)))
}

pub async fn delete_issue(
    State(state): State<WebAppState>,
    Path(issue_id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    let registry = IssueRegistry::new(state.workspace_root.clone());
    registry.delete(&issue_id)?;
    Ok(Json(json!({"status":"deleted"})))
}

#[cfg(test)]
pub(super) mod create_repository_tests {
    use std::ffi::OsString;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use axum::body::to_bytes;
    use tempfile::tempdir;

    use super::*;
    use crate::product::repository_store::{
        CadenceSkillsPreparationSummary, RepositoryInitializationCommandSummary,
        RepositoryInitializationSummary, RepositoryRegistrationError, RepositoryRegistrationInput,
        RepositoryRegistrationSuccess,
    };
    use crate::web::events::EventHub;

    struct RecordingRegistrar {
        calls: AtomicUsize,
        inputs: Mutex<Vec<RepositoryRegistrationInput>>,
        result: Result<RepositoryRegistrationSuccess, RepositoryRegistrationError>,
    }

    #[async_trait::async_trait]
    impl RepositoryRegistrar for RecordingRegistrar {
        async fn register(
            &self,
            input: RepositoryRegistrationInput,
        ) -> Result<RepositoryRegistrationSuccess, RepositoryRegistrationError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.inputs.lock().expect("inputs").push(input);
            self.result.clone()
        }
    }

    pub(crate) fn registration_success() -> RepositoryRegistrationSuccess {
        RepositoryRegistrationSuccess {
            repository: RepositoryRecord {
                id: "repository_0001".to_string(),
                project_id: "project_0001".to_string(),
                name: "Aria".to_string(),
                path: PathBuf::from("/work/aria"),
                repo_hash: "repo-hash".to_string(),
                runtime_root: PathBuf::from("/work/aria/.aria"),
                default_policy_preset: "balanced".to_string(),
                default_provider_mode: "claude_code".to_string(),
                created_at: "2026-07-14T00:00:00Z".to_string(),
                updated_at: "2026-07-14T00:00:00Z".to_string(),
            },
            cadence_skills: CadenceSkillsPreparationSummary {
                source_mode: "offline".to_string(),
                source_root: PathBuf::from("/skills/source"),
                skills_root: PathBuf::from("/skills"),
                git_updated: false,
                link_sync_status: "synchronized".to_string(),
                warnings: vec![
                    "cadence_skills_conflict:/home/alice/.codex/skills/demo".to_string(),
                ],
            },
            initialization: RepositoryInitializationSummary {
                provider: "claude_code".to_string(),
                source: PathBuf::from("/skills/source"),
                source_mode: "offline".to_string(),
                skills_root: PathBuf::from("/skills"),
                git_updated: false,
                link_sync_status: "synchronized".to_string(),
                commands: [
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
                .collect(),
            },
            warnings: vec!["cadence_skills_conflict:/home/alice/.codex/skills/demo".to_string()],
            changed_paths: vec![".claude/rules/project.md".to_string()],
            completed_at: "2026-07-14T00:01:00Z".to_string(),
        }
    }

    #[tokio::test]
    async fn create_repository_calls_registrar_once_and_returns_created_envelope() {
        let registrar = RecordingRegistrar {
            calls: AtomicUsize::new(0),
            inputs: Mutex::new(Vec::new()),
            result: Ok(registration_success()),
        };

        let response = create_repository_with_registrar(
            &registrar,
            "project_0001".to_string(),
            CreateRepositoryRequest {
                name: "Aria".to_string(),
                path: "/work/aria".to_string(),
                default_policy_preset: Some("balanced".to_string()),
                default_provider_mode: Some("claude_code".to_string()),
            },
        )
        .await
        .expect("created response");

        assert_eq!(response.status(), StatusCode::CREATED);
        assert_eq!(registrar.calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            registrar.inputs.lock().expect("inputs").as_slice(),
            &[RepositoryRegistrationInput {
                project_id: "project_0001".to_string(),
                name: "Aria".to_string(),
                path: PathBuf::from("/work/aria"),
                default_policy_preset: Some("balanced".to_string()),
                default_provider_mode: Some("claude_code".to_string()),
            }]
        );
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body");
        let value: serde_json::Value = serde_json::from_slice(&body).expect("response json");
        assert_eq!(value["repository"]["repository_id"], "repository_0001");
        assert_eq!(value["initialization"]["source"], "offline");
        assert_eq!(
            value["initialization"]["commands"],
            json!([
                {"index": 1, "command": "/pre-check", "status": "completed"},
                {"index": 2, "command": "/rule-config", "status": "completed"},
                {"index": 3, "command": "/mcp-configuration", "status": "completed"},
                {"index": 4, "command": "/project-rules-examples", "status": "completed"}
            ])
        );
        assert_eq!(
            value["initialization"]["warnings"],
            json!(["cadence_skills_conflict:<path>"])
        );
        assert_eq!(
            value["initialization"]["changed_paths"],
            json!([".claude/rules/project.md"])
        );
        assert_eq!(
            value["initialization"]["completed_at"],
            "2026-07-14T00:01:00Z"
        );
    }

    #[tokio::test]
    async fn create_repository_returns_structured_error_without_success_dto() {
        let registrar = RecordingRegistrar {
            calls: AtomicUsize::new(0),
            inputs: Mutex::new(Vec::new()),
            result: Err(RepositoryRegistrationError {
                stage: "repository_duplicate_check".to_string(),
                provider: None,
                command_index: None,
                command: None,
                reason_code: "repository_already_registered".to_string(),
                stderr_summary: Some("already registered".to_string()),
                changed_paths: Some(Vec::new()),
                retryable: false,
                action: "Choose another repository, then add it again.".to_string(),
            }),
        };

        let error = create_repository_with_registrar(
            &registrar,
            "project_0001".to_string(),
            CreateRepositoryRequest {
                name: "Aria".to_string(),
                path: "/work/aria".to_string(),
                default_policy_preset: None,
                default_provider_mode: None,
            },
        )
        .await
        .expect_err("registration should fail");

        assert_eq!(registrar.calls.load(Ordering::SeqCst), 1);
        let response = error.into_response();
        assert_eq!(response.status(), StatusCode::CONFLICT);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body");
        let value: serde_json::Value = serde_json::from_slice(&body).expect("response json");
        assert_eq!(value["code"], "repository_already_registered");
        assert!(value.get("repository").is_none());
    }

    #[test]
    fn create_repository_home_resolver_prefers_home_and_falls_back_to_userprofile() {
        let home = resolve_user_home_with(|name| match name {
            "HOME" => Some(OsString::from("/home/alice")),
            "USERPROFILE" => Some(OsString::from("/users/fallback")),
            _ => None,
        })
        .expect("HOME");
        assert_eq!(home, PathBuf::from("/home/alice"));

        let fallback = resolve_user_home_with(|name| match name {
            "HOME" => Some(OsString::new()),
            "USERPROFILE" => Some(OsString::from("/users/fallback")),
            _ => None,
        })
        .expect("USERPROFILE fallback");
        assert_eq!(fallback, PathBuf::from("/users/fallback"));
    }

    #[test]
    fn create_repository_home_resolver_rejects_missing_empty_and_relative_values() {
        for values in [
            (None, None),
            (Some(OsString::new()), Some(OsString::new())),
            (Some(OsString::from("relative/home")), None),
        ] {
            let error = resolve_user_home_with(|name| match name {
                "HOME" => values.0.clone(),
                "USERPROFILE" => values.1.clone(),
                _ => None,
            })
            .expect_err("invalid home");
            assert_eq!(error.reason_code, "cadence_skills_unavailable");
            assert_eq!(error.stage, "cadence_skills_home");
        }
    }

    #[test]
    fn create_repository_request_factory_reuses_state_dependencies_and_is_request_scoped() {
        let root = tempdir().expect("root");
        let state = WebAppState::with_events(
            root.path().to_path_buf(),
            WebRuntime::new_real(root.path().to_path_buf()).expect("real runtime"),
            EventHub::new(),
        );
        let home = root.path().join("home");
        let calls = Arc::new(AtomicUsize::new(0));
        let dependencies = (0..2)
            .map(|_| {
                let calls = calls.clone();
                let home = home.clone();
                request_repository_registration_dependencies_with_home(&state, move || {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Ok(home)
                })
                .expect("request dependencies")
            })
            .collect::<Vec<_>>();

        assert_eq!(calls.load(Ordering::SeqCst), 2);
        for dependency in &dependencies {
            assert_eq!(dependency.app_paths.root(), root.path().join(".aria"));
            assert_eq!(dependency.home, home);
            assert!(Arc::ptr_eq(&dependency.runner, &state.command_runner));
            assert!(Arc::ptr_eq(&dependency.gate, &state.provider_gate));
            assert!(Arc::ptr_eq(&dependency.registry, &state.provider_registry));
        }

        let first = build_repository_registrar(dependencies[0].clone());
        let second = build_repository_registrar(dependencies[1].clone());
        assert!(!Arc::ptr_eq(&first, &second));
    }
}
