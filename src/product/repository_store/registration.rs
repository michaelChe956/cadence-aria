use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio_util::sync::CancellationToken;

use uuid::Uuid;

use super::initializer::ClaudeRepositoryInitializer;
use super::types::{
    RepositoryInitializationCommandSummary, RepositoryInitializationOperation,
    RepositoryInitializationOperationInput, RepositoryInitializationProgress,
    RepositoryInitializationStepKind, RepositoryInitializationSummary, RepositoryRegistrationError,
    RepositoryRegistrationInput, RepositoryRegistrationSuccess,
};
use super::{
    CreateRepositoryInput, RepositoryInitializationOperationStore, RepositoryStore,
    canonicalize_repo_path,
};
use crate::cross_cutting::bounded_command_runner::{
    BoundedCommandRequest, BoundedCommandResult, BoundedCommandRunner,
};
use crate::cross_cutting::provider_availability_gate::ProviderAvailabilityGate;
use crate::cross_cutting::provider_registry::ProviderRegistry;
use crate::product::app_paths::ProductAppPaths;
use crate::product::cadence_skills::{
    CadenceSkillsError, CadenceSkillsManager, CadenceSkillsPreparationResult,
};
use crate::product::json_store::ProductStoreError;
use crate::product::models::{ProjectRecord, ProviderName, RepositoryRecord};
use crate::product::project_store::ProjectStore;

const GIT_OUTPUT_LIMIT: usize = 64 * 1024;

pub trait ProjectLookup: Send + Sync {
    fn get_project(&self, project_id: &str) -> Result<ProjectRecord, ProductStoreError>;
}

impl ProjectLookup for ProjectStore {
    fn get_project(&self, project_id: &str) -> Result<ProjectRecord, ProductStoreError> {
        self.get(project_id)
    }
}

pub trait RepositoryPersistence: Send + Sync {
    fn find_by_path(
        &self,
        project_id: &str,
        path: &Path,
    ) -> Result<Option<RepositoryRecord>, ProductStoreError>;

    fn initialization_operation_store(&self) -> Option<RepositoryInitializationOperationStore> {
        None
    }

    fn create_repository(
        &self,
        input: CreateRepositoryInput,
    ) -> Result<RepositoryRecord, ProductStoreError>;
}

impl RepositoryPersistence for RepositoryStore {
    fn find_by_path(
        &self,
        project_id: &str,
        path: &Path,
    ) -> Result<Option<RepositoryRecord>, ProductStoreError> {
        RepositoryStore::find_by_path(self, project_id, path)
    }

    fn initialization_operation_store(&self) -> Option<RepositoryInitializationOperationStore> {
        Some(RepositoryStore::initialization_operation_store(self))
    }

    fn create_repository(
        &self,
        input: CreateRepositoryInput,
    ) -> Result<RepositoryRecord, ProductStoreError> {
        self.create(input)
    }
}

#[async_trait::async_trait]
pub trait CadenceSkillsPreparation: Send + Sync {
    async fn prepare_skills(
        &self,
        cancellation: CancellationToken,
    ) -> Result<CadenceSkillsPreparationResult, CadenceSkillsError>;
}

#[async_trait::async_trait]
impl CadenceSkillsPreparation for CadenceSkillsManager {
    async fn prepare_skills(
        &self,
        cancellation: CancellationToken,
    ) -> Result<CadenceSkillsPreparationResult, CadenceSkillsError> {
        self.prepare(cancellation).await
    }
}

#[async_trait::async_trait]
pub trait RepositoryInitializer: Send + Sync {
    async fn initialize_repository(
        &self,
        git_root: &Path,
        command_timeout: Duration,
        cancellation: CancellationToken,
        progress: Arc<dyn RepositoryInitializationProgress>,
    ) -> Result<Vec<RepositoryInitializationCommandSummary>, RepositoryRegistrationError>;
}

#[async_trait::async_trait]
impl RepositoryInitializer for ClaudeRepositoryInitializer {
    async fn initialize_repository(
        &self,
        git_root: &Path,
        command_timeout: Duration,
        cancellation: CancellationToken,
        progress: Arc<dyn RepositoryInitializationProgress>,
    ) -> Result<Vec<RepositoryInitializationCommandSummary>, RepositoryRegistrationError> {
        self.initialize(git_root, command_timeout, cancellation, progress)
            .await
    }
}

type HostReadiness = dyn Fn() -> Result<(), String> + Send + Sync;
type Clock = dyn Fn() -> String + Send + Sync;

#[allow(dead_code)]
pub(crate) struct RepositoryInitializationLaunch {
    operation_id: String,
    project_id: String,
    input: RepositoryRegistrationInput,
    git_root: PathBuf,
    snapshot: RepositoryInitializationOperation,
    _guard: InitializationGuard,
}

impl RepositoryInitializationLaunch {
    #[allow(dead_code)]
    pub(crate) fn operation_id(&self) -> &str {
        &self.operation_id
    }

    #[allow(dead_code)]
    pub(crate) fn snapshot(&self) -> &RepositoryInitializationOperation {
        &self.snapshot
    }
}

struct OperationProgressReporter {
    operations: RepositoryInitializationOperationStore,
    project_id: String,
    operation_id: String,
    clock: Arc<Clock>,
    current_step: Arc<Mutex<Option<RepositoryInitializationStepKind>>>,
}

impl OperationProgressReporter {
    fn new(
        operations: RepositoryInitializationOperationStore,
        project_id: String,
        operation_id: String,
        clock: Arc<Clock>,
    ) -> Self {
        Self {
            operations,
            project_id,
            operation_id,
            clock,
            current_step: Arc::new(Mutex::new(None)),
        }
    }

    fn current_step(&self) -> Option<RepositoryInitializationStepKind> {
        *self
            .current_step
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn operation_store_error(error: ProductStoreError) -> RepositoryRegistrationError {
        registration_error(
            "repository_initialization_operation",
            "repository_initialization_operation_store_failed",
            &error.to_string(),
            true,
            "Repository initialization state could not be persisted; query the operation after recovery.",
        )
    }
}

impl RepositoryInitializationProgress for OperationProgressReporter {
    fn step_started(
        &self,
        step: RepositoryInitializationStepKind,
    ) -> Result<(), Box<RepositoryRegistrationError>> {
        self.operations
            .mark_step_running(&self.project_id, &self.operation_id, step, (self.clock)())
            .map_err(Self::operation_store_error)
            .map_err(Box::new)?;
        *self
            .current_step
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(step);
        Ok(())
    }

    fn step_completed(
        &self,
        step: RepositoryInitializationStepKind,
    ) -> Result<(), Box<RepositoryRegistrationError>> {
        self.operations
            .mark_step_completed(&self.project_id, &self.operation_id, step, (self.clock)())
            .map_err(Self::operation_store_error)
            .map_err(Box::new)?;
        let mut current_step = self
            .current_step
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if *current_step == Some(step) {
            *current_step = None;
        }
        Ok(())
    }
}

pub struct RepositoryRegistrationCoordinator {
    projects: Arc<dyn ProjectLookup>,
    repositories: Arc<dyn RepositoryPersistence>,
    operations: RepositoryInitializationOperationStore,
    provider_gate: Arc<ProviderAvailabilityGate>,
    provider_registry: Arc<ProviderRegistry>,
    cadence_skills: Arc<dyn CadenceSkillsPreparation>,
    host_readiness: Arc<HostReadiness>,
    runner: Arc<dyn BoundedCommandRunner>,
    clock: Arc<Clock>,
    initializer: Arc<dyn RepositoryInitializer>,
    git_command_timeout: Duration,
    initialization_timeout: Duration,
}

impl RepositoryRegistrationCoordinator {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        projects: Arc<dyn ProjectLookup>,
        repositories: Arc<dyn RepositoryPersistence>,
        provider_gate: Arc<ProviderAvailabilityGate>,
        provider_registry: Arc<ProviderRegistry>,
        cadence_skills: Arc<dyn CadenceSkillsPreparation>,
        host_readiness: Arc<HostReadiness>,
        runner: Arc<dyn BoundedCommandRunner>,
        clock: Arc<Clock>,
        initializer: Arc<dyn RepositoryInitializer>,
        git_command_timeout: Duration,
        initialization_timeout: Duration,
    ) -> Self {
        let operations = repositories
            .initialization_operation_store()
            .unwrap_or_else(|| {
                RepositoryInitializationOperationStore::new(ProductAppPaths::new(
                    std::env::temp_dir().join(format!(
                        "repository-registration-operations-{}",
                        Uuid::new_v4()
                    )),
                ))
            });
        Self::new_with_operations(
            projects,
            repositories,
            operations,
            provider_gate,
            provider_registry,
            cadence_skills,
            host_readiness,
            runner,
            clock,
            initializer,
            git_command_timeout,
            initialization_timeout,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new_with_operations(
        projects: Arc<dyn ProjectLookup>,
        repositories: Arc<dyn RepositoryPersistence>,
        operations: RepositoryInitializationOperationStore,
        provider_gate: Arc<ProviderAvailabilityGate>,
        provider_registry: Arc<ProviderRegistry>,
        cadence_skills: Arc<dyn CadenceSkillsPreparation>,
        host_readiness: Arc<HostReadiness>,
        runner: Arc<dyn BoundedCommandRunner>,
        clock: Arc<Clock>,
        initializer: Arc<dyn RepositoryInitializer>,
        git_command_timeout: Duration,
        initialization_timeout: Duration,
    ) -> Self {
        Self {
            projects,
            repositories,
            operations,
            provider_gate,
            provider_registry,
            cadence_skills,
            host_readiness,
            runner,
            clock,
            initializer,
            git_command_timeout,
            initialization_timeout,
        }
    }

    pub(crate) async fn begin_initialization(
        &self,
        input: RepositoryRegistrationInput,
        cancellation: CancellationToken,
    ) -> Result<RepositoryInitializationLaunch, RepositoryRegistrationError> {
        self.projects
            .get_project(&input.project_id)
            .map_err(|error| {
                registration_error(
                    "project_lookup",
                    "repository_project_not_found",
                    &error.to_string(),
                    false,
                    "Select an existing project before registering the repository.",
                )
            })?;

        let canonical_input = canonicalize_repo_path(&input.path).map_err(|error| {
            registration_error(
                "repository_path",
                "repository_path_invalid",
                &error.to_string(),
                false,
                "Choose an existing repository path that can be canonicalized.",
            )
        })?;
        let git_root = self
            .resolve_git_root(&canonical_input, cancellation)
            .await?;

        self.reject_duplicate(&input.project_id, &git_root)
            .map_err(|error| *error)?;
        let guard = InitializationGuard::try_acquire(git_root.clone()).map_err(|error| *error)?;
        self.reject_duplicate(&input.project_id, &git_root)
            .map_err(|error| *error)?;

        let operation_id = format!("repository_initialization_{}", Uuid::new_v4().simple());
        let snapshot = self
            .operations
            .create(RepositoryInitializationOperation::new(
                operation_id.clone(),
                input.project_id.clone(),
                RepositoryInitializationOperationInput {
                    name: input.name.clone(),
                    git_root: git_root.clone(),
                    default_policy_preset: input.default_policy_preset.clone(),
                    default_provider_mode: input.default_provider_mode.clone(),
                },
                (self.clock)(),
            ))
            .map_err(OperationProgressReporter::operation_store_error)?;

        Ok(RepositoryInitializationLaunch {
            operation_id,
            project_id: input.project_id.clone(),
            input,
            git_root,
            snapshot,
            _guard: guard,
        })
    }

    pub(crate) async fn execute_initialization(
        &self,
        launch: RepositoryInitializationLaunch,
        cancellation: CancellationToken,
    ) -> Result<RepositoryInitializationOperation, RepositoryRegistrationError> {
        let RepositoryInitializationLaunch {
            operation_id,
            project_id,
            input,
            git_root,
            snapshot: _,
            _guard,
        } = launch;
        let reporter = Arc::new(OperationProgressReporter::new(
            self.operations.clone(),
            project_id.clone(),
            operation_id.clone(),
            self.clock.clone(),
        ));

        self.operations
            .mark_running(&project_id, &operation_id, (self.clock)())
            .map_err(OperationProgressReporter::operation_store_error)?;

        let mut before = None;
        let execution = async {
            reporter
                .step_started(RepositoryInitializationStepKind::CadenceSkills)
                .map_err(|error| *error)?;
            let prepared = self
                .cadence_skills
                .prepare_skills(cancellation.clone())
                .await
                .map_err(cadence_error)?;
            reporter
                .step_completed(RepositoryInitializationStepKind::CadenceSkills)
                .map_err(|error| *error)?;

            reporter
                .step_started(RepositoryInitializationStepKind::RuleConfig)
                .map_err(|error| *error)?;
            self.provider_gate
                .ensure_available(&ProviderName::ClaudeCode)
                .map_err(|error| {
                    claude_error(
                        "provider_gate",
                        error.code(),
                        error.reason(),
                        true,
                        "Restore Claude Code availability, then retry repository registration.",
                    )
                })?;
            if self
                .provider_registry
                .get(&ProviderName::ClaudeCode)
                .is_none()
            {
                return Err(claude_error(
                    "provider_registry",
                    "provider_unavailable",
                    "Claude Code provider is not registered",
                    true,
                    "Register the gated Claude Code provider, then retry.",
                ));
            }
            (self.host_readiness)().map_err(|reason| {
                claude_error(
                    "host_readiness",
                    "host_real_workflow_blocked",
                    &reason,
                    true,
                    "Restore host readiness for real workflows, then retry.",
                )
            })?;
            before = Some(
                self.git_status(&git_root, cancellation.clone(), "git_status_before")
                    .await?,
            );

            let commands = self
                .initializer
                .initialize_repository(
                    &git_root,
                    self.initialization_timeout,
                    cancellation.clone(),
                    reporter.clone(),
                )
                .await?;
            let after = self
                .git_status(&git_root, cancellation.clone(), "git_status_after")
                .await?;
            let changed_paths = changed_paths(
                before
                    .as_ref()
                    .expect("git status before exists before initialization"),
                &after,
            );
            let initialization = RepositoryInitializationSummary {
                provider: "claude_code".to_string(),
                source: prepared.source_root.clone(),
                source_mode: prepared.source_mode.as_str().to_string(),
                skills_root: prepared.skills_root.clone(),
                git_updated: prepared.git_updated,
                link_sync_status: prepared.link_sync_status.as_str().to_string(),
                commands,
            };
            let repository = self
                .repositories
                .create_repository(CreateRepositoryInput {
                    project_id: input.project_id,
                    name: input.name,
                    path: git_root.clone(),
                    default_policy_preset: input.default_policy_preset,
                    default_provider_mode: input.default_provider_mode,
                })
                .map_err(|error| {
                    let mut mapped = registration_error(
                        "repository_persist",
                        "repository_persist_failed",
                        &error.to_string(),
                        true,
                        "Initialization files may exist, but no product Repository record was created; inspect the repository and retry persistence.",
                    );
                    mapped.changed_paths = Some(changed_paths.clone());
                    mapped
                })?;

            Ok(RepositoryRegistrationSuccess {
                repository,
                cadence_skills: preparation_summary(&prepared),
                initialization,
                warnings: prepared.warnings,
                changed_paths,
                completed_at: (self.clock)(),
            })
        }
        .await;

        match execution {
            Ok(success) => self
                .operations
                .finish_completed(&project_id, &operation_id, success, (self.clock)())
                .map_err(OperationProgressReporter::operation_store_error),
            Err(mut error) => {
                if let Some(before) = before.as_ref()
                    && error.changed_paths.is_none()
                    && error.stage != "git_status_after"
                {
                    match self
                        .git_status(&git_root, cancellation, "git_status_after_failure")
                        .await
                    {
                        Ok(after) => error.changed_paths = Some(changed_paths(before, &after)),
                        Err(status_error) => {
                            error.changed_paths = None;
                            let final_state = status_error
                                .stderr_summary
                                .as_deref()
                                .unwrap_or("unknown final Git state error");
                            let combined = match error.stderr_summary.as_deref() {
                                Some(initializer) => {
                                    format!("{initializer}; final_git_state: {final_state}")
                                }
                                None => format!("final_git_state: {final_state}"),
                            };
                            error.stderr_summary = Some(sanitize_summary(&combined, 4 * 1024));
                            error.action.push_str(
                                " Final Git state could not be collected; inspect the repository manually.",
                            );
                        }
                    }
                }

                let failed_step = reporter.current_step();
                self.operations
                    .finish_failed(
                        &project_id,
                        &operation_id,
                        failed_step,
                        error.clone(),
                        (self.clock)(),
                    )
                    .map_err(OperationProgressReporter::operation_store_error)?;
                Err(error)
            }
        }
    }

    pub fn get_initialization_operation(
        &self,
        project_id: &str,
        operation_id: &str,
    ) -> Result<RepositoryInitializationOperation, ProductStoreError> {
        self.operations.get(project_id, operation_id)
    }

    pub fn recover_interrupted_operation(
        &self,
        project_id: &str,
        operation_id: &str,
    ) -> Result<RepositoryInitializationOperation, ProductStoreError> {
        self.operations
            .recover_interrupted(project_id, operation_id, (self.clock)())
    }

    pub async fn register(
        &self,
        input: RepositoryRegistrationInput,
        cancellation: CancellationToken,
    ) -> Result<RepositoryRegistrationSuccess, RepositoryRegistrationError> {
        let launch = self
            .begin_initialization(input, cancellation.clone())
            .await?;
        let project_id = launch.project_id.clone();
        let operation_id = launch.operation_id.clone();
        match self.execute_initialization(launch, cancellation).await {
            Ok(operation) => operation.result.ok_or_else(|| {
                registration_error(
                    "repository_initialization_operation",
                    "repository_initialization_operation_result_missing",
                    "completed repository initialization operation did not contain a result",
                    true,
                    "Query the operation again after recovery.",
                )
            }),
            Err(execution_error) => {
                let error = match self.operations.get(&project_id, &operation_id) {
                    Ok(operation) => operation.error.unwrap_or(execution_error),
                    Err(_) => execution_error,
                };
                Err(error)
            }
        }
    }

    fn reject_duplicate(
        &self,
        project_id: &str,
        git_root: &Path,
    ) -> Result<(), Box<RepositoryRegistrationError>> {
        match self.repositories.find_by_path(project_id, git_root) {
            Ok(Some(_)) => Err(Box::new(registration_error(
                "repository_duplicate_check",
                "repository_already_registered",
                "repository path is already registered for this project",
                false,
                "Use the existing Repository record.",
            ))),
            Ok(None) => Ok(()),
            Err(error) => Err(Box::new(registration_error(
                "repository_duplicate_check",
                "repository_path_invalid",
                &error.to_string(),
                false,
                "Repair the repository path or product repository index, then retry.",
            ))),
        }
    }

    async fn resolve_git_root(
        &self,
        working_dir: &Path,
        cancellation: CancellationToken,
    ) -> Result<PathBuf, RepositoryRegistrationError> {
        let result = self
            .run_git(
                working_dir,
                vec!["rev-parse".to_string(), "--show-toplevel".to_string()],
                cancellation,
            )
            .await
            .map_err(|reason| {
                registration_error(
                    "git_root",
                    "repository_not_git",
                    &reason,
                    false,
                    "Choose a path inside a valid Git repository.",
                )
            })?;
        if !command_succeeded(&result) {
            return Err(registration_error(
                "git_root",
                "repository_not_git",
                &command_diagnostic(&result),
                false,
                "Choose a path inside a valid Git repository.",
            ));
        }
        let root = result.stdout.trim();
        if root.is_empty() {
            return Err(registration_error(
                "git_root",
                "repository_not_git",
                "git rev-parse returned an empty repository root",
                false,
                "Choose a path inside a valid Git repository.",
            ));
        }
        canonicalize_repo_path(Path::new(root)).map_err(|error| {
            registration_error(
                "git_root",
                "repository_not_git",
                &error.to_string(),
                false,
                "Choose a path inside a valid Git repository.",
            )
        })
    }

    async fn git_status(
        &self,
        git_root: &Path,
        cancellation: CancellationToken,
        stage: &str,
    ) -> Result<BTreeMap<String, String>, RepositoryRegistrationError> {
        let result = self
            .run_git(
                git_root,
                vec![
                    "status".to_string(),
                    "--porcelain=v1".to_string(),
                    "-z".to_string(),
                    "--untracked-files=all".to_string(),
                ],
                cancellation,
            )
            .await
            .map_err(|reason| git_state_error(stage, &reason))?;
        if !command_succeeded(&result) || result.stdout_truncated {
            return Err(git_state_error(stage, &command_diagnostic(&result)));
        }
        Ok(parse_porcelain_status(&result.stdout))
    }

    async fn run_git(
        &self,
        working_dir: &Path,
        argv: Vec<String>,
        cancellation: CancellationToken,
    ) -> Result<BoundedCommandResult, String> {
        self.runner
            .run(BoundedCommandRequest {
                executable: "git".to_string(),
                argv,
                working_dir: working_dir.to_path_buf(),
                timeout: self.git_command_timeout,
                cancellation,
                environment: BTreeMap::from([("LC_ALL".to_string(), "C".to_string())]),
                stdout_limit: GIT_OUTPUT_LIMIT,
                stderr_limit: GIT_OUTPUT_LIMIT,
            })
            .await
            .map_err(|error| error.to_string())
    }
}

mod helpers;
use helpers::*;

#[cfg(test)]
mod tests;
