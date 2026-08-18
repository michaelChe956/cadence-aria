use super::dto::repository_initialization_operation_dto;
use super::support::{
    product_app_paths, product_store_api_error, reject_legacy_repository_endpoint_on_multi_repo,
};
use super::*;

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::rejection::JsonRejection;
use chrono::Utc;
use tokio_util::sync::CancellationToken;

use crate::cross_cutting::bounded_command_runner::BoundedCommandRunner;
use crate::cross_cutting::provider_availability_gate::{
    ProviderAvailabilityGate, ProviderHealthSource,
};
use crate::cross_cutting::provider_health::{ProviderHealthEntry, ProviderHealthSnapshot};
use crate::cross_cutting::provider_registry::ProviderRegistry;
use crate::product::app_paths::ProductAppPaths;
use crate::product::cadence_skills::{
    CadenceSkillsManager, CadenceSkillsPreparationResult, CadenceSkillsSourceMode, LinkSyncStatus,
};
use crate::product::project_store::ProjectStore;
use crate::product::repository_store::{
    CadenceSkillsPreparation, ClaudeRepositoryInitializer, ProjectLookup,
    RepositoryInitializationCommandSummary, RepositoryInitializationOperation,
    RepositoryInitializationOperationStatus, RepositoryInitializationOperationStore,
    RepositoryInitializationProgress, RepositoryInitializationStepKind, RepositoryInitializer,
    RepositoryPersistence, RepositoryRegistrationCoordinator, RepositoryRegistrationError,
    RepositoryRegistrationInput, RepositoryStore,
};

struct ProjectAwareRepositoryPersistence {
    app_paths: ProductAppPaths,
}

impl ProjectAwareRepositoryPersistence {
    fn new(app_paths: ProductAppPaths) -> Self {
        Self { app_paths }
    }

    fn store_for_project(&self, project_id: &str) -> Result<RepositoryStore, ProductStoreError> {
        let project = ProjectStore::new(self.app_paths.clone()).get(project_id)?;
        Ok(RepositoryStore::for_project(
            self.app_paths.clone(),
            &project,
        ))
    }
}

impl RepositoryPersistence for ProjectAwareRepositoryPersistence {
    fn find_by_path(
        &self,
        project_id: &str,
        path: &std::path::Path,
    ) -> Result<Option<RepositoryRecord>, ProductStoreError> {
        self.store_for_project(project_id)?
            .find_by_path(project_id, path)
    }

    fn initialization_operation_store(&self) -> Option<RepositoryInitializationOperationStore> {
        Some(RepositoryInitializationOperationStore::new(
            self.app_paths.clone(),
        ))
    }

    fn create_repository(
        &self,
        input: crate::product::repository_store::CreateRepositoryInput,
    ) -> Result<RepositoryRecord, ProductStoreError> {
        self.store_for_project(&input.project_id)?.create(input)
    }
}

/// Repository POST 路由可注入、可克隆的协调器依赖。
///
/// 外部集成测试通过 [`RepositoryRegistrationDependenciesBuilder`] 安装临时用户根、
/// 受控 runner、共享健康 gate/registry、Cadence preparation 与 initializer；路由本身
/// 仍只调用 `RepositoryRegistrationCoordinator`。
#[derive(Clone)]
pub struct RepositoryRegistrationDependencies {
    coordinator: Arc<RepositoryRegistrationCoordinator>,
}

pub type RepositoryRegistrationBuildResult<T> = Result<T, Box<RepositoryRegistrationError>>;

impl RepositoryRegistrationDependencies {
    pub fn builder(
        app_paths: ProductAppPaths,
        home: impl Into<PathBuf>,
        runner: Arc<dyn BoundedCommandRunner>,
        gate: Arc<ProviderAvailabilityGate>,
        registry: Arc<ProviderRegistry>,
    ) -> RepositoryRegistrationDependenciesBuilder {
        RepositoryRegistrationDependenciesBuilder::new(app_paths, home, runner, gate, registry)
    }

    async fn begin_initialization(
        &self,
        input: RepositoryRegistrationInput,
        cancellation: CancellationToken,
    ) -> Result<
        crate::product::repository_store::RepositoryInitializationLaunch,
        RepositoryRegistrationError,
    > {
        self.coordinator
            .begin_initialization(input, cancellation)
            .await
    }

    async fn execute_initialization(
        &self,
        launch: crate::product::repository_store::RepositoryInitializationLaunch,
        cancellation: CancellationToken,
    ) -> Result<RepositoryInitializationOperation, RepositoryRegistrationError> {
        self.coordinator
            .execute_initialization(launch, cancellation)
            .await
    }

    fn get_initialization_operation(
        &self,
        project_id: &str,
        operation_id: &str,
    ) -> Result<RepositoryInitializationOperation, ProductStoreError> {
        self.coordinator
            .get_initialization_operation(project_id, operation_id)
    }

    fn recover_interrupted_operation(
        &self,
        project_id: &str,
        operation_id: &str,
    ) -> Result<RepositoryInitializationOperation, ProductStoreError> {
        self.coordinator
            .recover_interrupted_operation(project_id, operation_id)
    }
}

pub struct RepositoryRegistrationDependenciesBuilder {
    app_paths: ProductAppPaths,
    home: PathBuf,
    command_environment: BTreeMap<String, String>,
    runner: Arc<dyn BoundedCommandRunner>,
    gate: Arc<ProviderAvailabilityGate>,
    registry: Arc<ProviderRegistry>,
    projects: Option<Arc<dyn ProjectLookup>>,
    repositories: Option<Arc<dyn RepositoryPersistence>>,
    cadence_skills: Option<Arc<dyn CadenceSkillsPreparation>>,
    host_readiness: Option<Arc<dyn Fn() -> Result<(), String> + Send + Sync>>,
    initializer: Option<Arc<dyn RepositoryInitializer>>,
    clock: Arc<dyn Fn() -> String + Send + Sync>,
    git_command_timeout: Duration,
    initialization_timeout: Duration,
}

impl RepositoryRegistrationDependenciesBuilder {
    fn new(
        app_paths: ProductAppPaths,
        home: impl Into<PathBuf>,
        runner: Arc<dyn BoundedCommandRunner>,
        gate: Arc<ProviderAvailabilityGate>,
        registry: Arc<ProviderRegistry>,
    ) -> Self {
        Self {
            app_paths,
            home: home.into(),
            command_environment: BTreeMap::new(),
            runner,
            gate,
            registry,
            projects: None,
            repositories: None,
            cadence_skills: None,
            host_readiness: None,
            initializer: None,
            clock: Arc::new(|| Utc::now().to_rfc3339()),
            git_command_timeout: Duration::from_secs(180),
            initialization_timeout: Duration::from_secs(1800),
        }
    }

    pub fn with_command_environment(mut self, environment: BTreeMap<String, String>) -> Self {
        self.command_environment = environment;
        self
    }

    pub fn with_project_lookup(mut self, projects: Arc<dyn ProjectLookup>) -> Self {
        self.projects = Some(projects);
        self
    }

    pub fn with_repository_persistence(
        mut self,
        repositories: Arc<dyn RepositoryPersistence>,
    ) -> Self {
        self.repositories = Some(repositories);
        self
    }

    pub fn with_cadence_skills(
        mut self,
        cadence_skills: Arc<dyn CadenceSkillsPreparation>,
    ) -> Self {
        self.cadence_skills = Some(cadence_skills);
        self
    }

    pub fn with_host_readiness(
        mut self,
        host_readiness: Arc<dyn Fn() -> Result<(), String> + Send + Sync>,
    ) -> Self {
        self.host_readiness = Some(host_readiness);
        self
    }

    pub fn with_initializer(mut self, initializer: Arc<dyn RepositoryInitializer>) -> Self {
        self.initializer = Some(initializer);
        self
    }

    pub fn with_clock(mut self, clock: Arc<dyn Fn() -> String + Send + Sync>) -> Self {
        self.clock = clock;
        self
    }

    pub fn with_timeouts(
        mut self,
        git_command_timeout: Duration,
        initialization_timeout: Duration,
    ) -> Self {
        self.git_command_timeout = git_command_timeout;
        self.initialization_timeout = initialization_timeout;
        self
    }

    pub fn build(self) -> RepositoryRegistrationBuildResult<RepositoryRegistrationDependencies> {
        let home = validate_user_home(self.home)?;
        let app_paths = self.app_paths.clone();
        let projects = self
            .projects
            .unwrap_or_else(|| Arc::new(ProjectStore::new(app_paths.clone())));
        let repositories = self
            .repositories
            .unwrap_or_else(|| Arc::new(ProjectAwareRepositoryPersistence::new(app_paths.clone())));
        let operations = RepositoryInitializationOperationStore::new(app_paths);
        let cadence_skills = self.cadence_skills.unwrap_or_else(|| {
            Arc::new(CadenceSkillsManager::with_dependencies(
                home.clone(),
                self.runner.clone(),
                self.command_environment,
            ))
        });
        let git_environment = git_finalize_environment(&home);
        let host_readiness = self.host_readiness.unwrap_or_else(|| {
            Arc::new(|| {
                crate::web::provider_availability::host_real_workflow_ready()
                    .map_err(|error| error.message)
            })
        });
        let initializer = self.initializer.unwrap_or_else(|| {
            Arc::new(ClaudeRepositoryInitializer::new(
                self.gate.clone(),
                self.registry.clone(),
                4 * 1024,
            ))
        });
        let coordinator = RepositoryRegistrationCoordinator::new_with_operations(
            projects,
            repositories,
            operations,
            self.gate,
            self.registry,
            cadence_skills,
            host_readiness,
            self.runner,
            self.clock,
            initializer,
            self.git_command_timeout,
            self.initialization_timeout,
        )
        .with_git_environment(git_environment);
        Ok(RepositoryRegistrationDependencies {
            coordinator: Arc::new(coordinator),
        })
    }
}

fn git_finalize_environment(home: &std::path::Path) -> std::collections::BTreeMap<String, String> {
    let mut environment = std::collections::BTreeMap::from([
        ("LC_ALL".to_string(), "C".to_string()),
        ("HOME".to_string(), home.to_string_lossy().into_owned()),
    ]);
    if let Some(value) = std::env::var_os("SSH_AUTH_SOCK").filter(|value| !value.is_empty()) {
        environment.insert(
            "SSH_AUTH_SOCK".to_string(),
            value.to_string_lossy().into_owned(),
        );
    }
    environment
}

pub async fn create_repository(
    State(state): State<WebAppState>,
    Path(project_id): Path<String>,
    request: Result<Json<CreateRepositoryRequest>, JsonRejection>,
) -> ApiResult<Response> {
    let project = ProjectStore::new(product_app_paths(&state))
        .get(&project_id)
        .map_err(product_store_api_error)?;
    reject_legacy_repository_endpoint_on_multi_repo(&project)?;
    let Json(request) = match request {
        Ok(request) => request,
        Err(error) => return Ok(error.into_response()),
    };
    let dependencies = match state.repository_registration_dependencies() {
        Some(dependencies) => dependencies,
        None => default_dependencies(&state).map_err(|error| ApiError::from(*error))?,
    };
    let launch = dependencies
        .begin_initialization(
            RepositoryRegistrationInput {
                project_id,
                name: request.name,
                path: request.path.into(),
                default_policy_preset: request.default_policy_preset,
                default_provider_mode: request.default_provider_mode,
            },
            CancellationToken::new(),
        )
        .await
        .map_err(ApiError::from)?;
    let snapshot = launch.snapshot().clone();
    let lease = state
        .repository_initialization_runs
        .register(snapshot.operation_id.clone())
        .ok_or_else(|| {
            ApiError::runtime(
                "repository_initialization_in_progress",
                "repository initialization is already in progress",
                serde_json::json!({}),
            )
        })?;
    let worker_dependencies = dependencies.clone();
    tokio::spawn(async move {
        let _lease = lease;
        if let Err(error) = worker_dependencies
            .execute_initialization(launch, CancellationToken::new())
            .await
        {
            tracing::error!(reason_code = %error.reason_code, "repository initialization worker failed");
        }
    });
    Ok((
        StatusCode::ACCEPTED,
        Json(repository_initialization_operation_dto(snapshot)),
    )
        .into_response())
}

pub async fn get_repository_initialization(
    State(state): State<WebAppState>,
    Path((project_id, operation_id)): Path<(String, String)>,
) -> ApiResult<Response> {
    match ProjectStore::new(product_app_paths(&state)).get(&project_id) {
        Ok(project) => reject_legacy_repository_endpoint_on_multi_repo(&project)?,
        // Existing GET semantics resolve the operation first: an unknown or
        // cross-project operation remains `repository_initialization_operation_not_found`
        // even when its project record is absent. Only an existing multi-repo
        // project is newly guarded.
        Err(ProductStoreError::NotFound {
            kind: "project", ..
        }) => {}
        Err(error) => return Err(product_store_api_error(error)),
    }
    let dependencies = match state.repository_registration_dependencies() {
        Some(dependencies) => dependencies,
        None => default_dependencies(&state).map_err(|error| ApiError::from(*error))?,
    };
    let operation = dependencies
        .get_initialization_operation(&project_id, &operation_id)
        .map_err(product_store_api_error)?;
    let operation = if matches!(
        operation.status,
        RepositoryInitializationOperationStatus::Created
            | RepositoryInitializationOperationStatus::Running
    ) && !state
        .repository_initialization_runs
        .is_active(&operation_id)
    {
        dependencies
            .recover_interrupted_operation(&project_id, &operation_id)
            .map_err(product_store_api_error)?
    } else {
        operation
    };
    Ok(Json(repository_initialization_operation_dto(operation)).into_response())
}

fn default_dependencies(
    state: &WebAppState,
) -> RepositoryRegistrationBuildResult<RepositoryRegistrationDependencies> {
    let fake_runtime = !state
        .runtime
        .lock()
        .expect("web runtime lock")
        .enforces_real_provider_availability();
    let home = if fake_runtime {
        state.workspace_root.clone()
    } else {
        resolve_user_home_with(|name| std::env::var_os(name))?
    };
    let builder = RepositoryRegistrationDependencies::builder(
        product_app_paths(state),
        home.clone(),
        state.command_runner.clone(),
        state.provider_gate.clone(),
        state.provider_registry.clone(),
    )
    .with_command_environment(command_environment());
    if fake_runtime {
        builder
            .with_cadence_skills(Arc::new(FakeCadenceSkillsPreparation { home }))
            .with_host_readiness(Arc::new(|| Ok(())))
            .with_initializer(Arc::new(CompletedRepositoryInitializer))
            .build_with_gate(fake_repository_registration_gate())
    } else {
        builder.build()
    }
}

impl RepositoryRegistrationDependenciesBuilder {
    fn build_with_gate(
        mut self,
        gate: Arc<ProviderAvailabilityGate>,
    ) -> RepositoryRegistrationBuildResult<RepositoryRegistrationDependencies> {
        self.gate = gate;
        self.build()
    }
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
    Arc::new(ProviderAvailabilityGate::new(Arc::new(
        FixedAvailableProviderHealth(Arc::new(ProviderHealthSnapshot {
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
        })),
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
        progress: Arc<dyn RepositoryInitializationProgress>,
    ) -> Result<Vec<RepositoryInitializationCommandSummary>, RepositoryRegistrationError> {
        let mut summaries = Vec::with_capacity(4);
        for (offset, step) in RepositoryInitializationStepKind::ALL
            .into_iter()
            .filter(|step| step.command().is_some())
            .enumerate()
        {
            let command = step.command().expect("Claude initialization command");
            progress.step_started(step).map_err(|error| *error)?;
            progress.step_completed(step).map_err(|error| *error)?;
            summaries.push(RepositoryInitializationCommandSummary {
                command_index: offset + 1,
                command: command.to_string(),
                status: "completed".to_string(),
                output_summary: None,
            });
        }
        Ok(summaries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn home_resolution_prefers_home_and_rejects_relative_paths() {
        let home =
            resolve_user_home_with(|name| (name == "HOME").then(|| OsString::from("/home/alice")))
                .expect("HOME");
        assert_eq!(home, PathBuf::from("/home/alice"));

        let error = resolve_user_home_with(|name| {
            (name == "HOME").then(|| OsString::from("relative/home"))
        })
        .expect_err("relative HOME");
        assert_eq!(error.stage, "cadence_skills_home");
        assert_eq!(error.reason_code, "cadence_skills_unavailable");
    }

    #[test]
    fn built_coordinator_carries_validated_home_in_git_environment() {
        let dependencies = RepositoryRegistrationDependencies::builder(
            ProductAppPaths::new(std::env::temp_dir().join("git-env-test")),
            "/home/tester",
            Arc::new(crate::cross_cutting::bounded_command_runner::TokioBoundedCommandRunner),
            fake_repository_registration_gate(),
            Arc::new(ProviderRegistry::default()),
        )
        .build()
        .expect("build");
        let environment = dependencies.coordinator.git_environment();
        assert_eq!(
            environment.get("HOME").map(String::as_str),
            Some("/home/tester")
        );
        assert_eq!(environment.get("LC_ALL").map(String::as_str), Some("C"));
        for key in environment.keys() {
            assert!(
                matches!(key.as_str(), "LC_ALL" | "HOME" | "SSH_AUTH_SOCK"),
                "unexpected key {key}"
            );
        }
    }
}
