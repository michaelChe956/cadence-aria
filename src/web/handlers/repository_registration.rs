use super::dto::repository_registration_response;
use super::support::product_app_paths;
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
use crate::product::app_paths::ProductAppPaths;
use crate::product::cadence_skills::{
    CadenceSkillsManager, CadenceSkillsPreparationResult, CadenceSkillsSourceMode, LinkSyncStatus,
};
use crate::product::project_store::ProjectStore;
use crate::product::repository_store::{
    CadenceSkillsPreparation, ClaudeRepositoryInitializer, ProjectLookup,
    RepositoryInitializationCommandSummary, RepositoryInitializer, RepositoryPersistence,
    RepositoryRegistrationCoordinator, RepositoryRegistrationError, RepositoryRegistrationInput,
    RepositoryRegistrationSuccess, RepositoryStore,
};

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

/// Repository POST 路由可注入、可克隆的协调器依赖。
///
/// 外部集成测试通过 [`RepositoryRegistrationDependenciesBuilder`] 安装临时用户根、
/// 受控 runner、共享健康 gate/registry、Cadence preparation 与 initializer；路由本身
/// 仍只调用 `RepositoryRegistrationCoordinator`。
#[derive(Clone)]
pub struct RepositoryRegistrationDependencies {
    registrar: Arc<dyn RepositoryRegistrar>,
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
            initialization_timeout: Duration::from_secs(300),
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
        let projects = self
            .projects
            .unwrap_or_else(|| Arc::new(ProjectStore::new(self.app_paths.clone())));
        let repositories = self
            .repositories
            .unwrap_or_else(|| Arc::new(RepositoryStore::new(self.app_paths)));
        let cadence_skills = self.cadence_skills.unwrap_or_else(|| {
            Arc::new(CadenceSkillsManager::with_dependencies(
                home,
                self.runner.clone(),
                self.command_environment,
            ))
        });
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
        Ok(RepositoryRegistrationDependencies {
            registrar: Arc::new(RepositoryRegistrationCoordinator::new(
                projects,
                repositories,
                self.gate,
                self.registry,
                cadence_skills,
                host_readiness,
                self.runner,
                self.clock,
                initializer,
                self.git_command_timeout,
                self.initialization_timeout,
            )),
        })
    }
}

pub async fn create_repository(
    State(state): State<WebAppState>,
    Path(project_id): Path<String>,
    Json(request): Json<CreateRepositoryRequest>,
) -> ApiResult<Response> {
    let dependencies = match state.repository_registration_dependencies() {
        Some(dependencies) => dependencies,
        None => default_dependencies(&state).map_err(|error| ApiError::from(*error))?,
    };
    let success = dependencies
        .registrar
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
}
