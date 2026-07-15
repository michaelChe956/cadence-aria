use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use tokio_util::sync::CancellationToken;

use super::initializer::ClaudeRepositoryInitializer;
use super::types::{
    CadenceSkillsPreparationSummary, RepositoryInitializationCommandSummary,
    RepositoryInitializationSummary, RepositoryRegistrationError, RepositoryRegistrationInput,
    RepositoryRegistrationSuccess,
};
use super::{CreateRepositoryInput, RepositoryStore, canonicalize_repo_path};
use crate::cross_cutting::bounded_command_runner::{
    BoundedCommandRequest, BoundedCommandResult, BoundedCommandRunner,
};
use crate::cross_cutting::provider_availability_gate::ProviderAvailabilityGate;
use crate::cross_cutting::provider_registry::ProviderRegistry;
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
    ) -> Result<Vec<RepositoryInitializationCommandSummary>, RepositoryRegistrationError>;
}

#[async_trait::async_trait]
impl RepositoryInitializer for ClaudeRepositoryInitializer {
    async fn initialize_repository(
        &self,
        git_root: &Path,
        command_timeout: Duration,
        cancellation: CancellationToken,
    ) -> Result<Vec<RepositoryInitializationCommandSummary>, RepositoryRegistrationError> {
        self.initialize(git_root, command_timeout, cancellation)
            .await
    }
}

type HostReadiness = dyn Fn() -> Result<(), String> + Send + Sync;
type Clock = dyn Fn() -> String + Send + Sync;

pub struct RepositoryRegistrationCoordinator {
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
        Self {
            projects,
            repositories,
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

    pub async fn register(
        &self,
        input: RepositoryRegistrationInput,
        cancellation: CancellationToken,
    ) -> Result<RepositoryRegistrationSuccess, RepositoryRegistrationError> {
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
            .resolve_git_root(&canonical_input, cancellation.clone())
            .await?;

        self.reject_duplicate(&input.project_id, &git_root)
            .map_err(|error| *error)?;
        let _initialization_guard =
            InitializationGuard::try_acquire(git_root.clone()).map_err(|error| *error)?;
        self.reject_duplicate(&input.project_id, &git_root)
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

        let prepared = self
            .cadence_skills
            .prepare_skills(cancellation.clone())
            .await
            .map_err(cadence_error)?;
        let before = self
            .git_status(&git_root, cancellation.clone(), "git_status_before")
            .await?;

        let commands = match self
            .initializer
            .initialize_repository(&git_root, self.initialization_timeout, cancellation.clone())
            .await
        {
            Ok(commands) => commands,
            Err(mut error) => {
                match self
                    .git_status(&git_root, cancellation.clone(), "git_status_after_failure")
                    .await
                {
                    Ok(after) => error.changed_paths = Some(changed_paths(&before, &after)),
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
                return Err(error);
            }
        };

        let after = self
            .git_status(&git_root, cancellation.clone(), "git_status_after")
            .await?;
        let changed_paths = changed_paths(&before, &after);
        let preparation_summary = preparation_summary(&prepared);
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
                path: git_root,
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
            cadence_skills: preparation_summary,
            initialization,
            warnings: prepared.warnings,
            changed_paths,
            completed_at: (self.clock)(),
        })
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

fn command_succeeded(result: &BoundedCommandResult) -> bool {
    result.exit_code == Some(0) && !result.timed_out && !result.cancelled
}

fn command_diagnostic(result: &BoundedCommandResult) -> String {
    let message = if result.stderr.trim().is_empty() {
        result.stdout.trim()
    } else {
        result.stderr.trim()
    };
    if result.timed_out {
        format!("git command timed out: {message}")
    } else if result.cancelled {
        format!("git command cancelled: {message}")
    } else {
        format!("git command exited {:?}: {message}", result.exit_code)
    }
}

fn parse_porcelain_status(output: &str) -> BTreeMap<String, String> {
    let records = output.split('\0').collect::<Vec<_>>();
    let mut status = BTreeMap::new();
    let mut index = 0;
    while index < records.len() {
        let record = records[index];
        if record.len() < 4 {
            index += 1;
            continue;
        }
        let code = record[..2].to_string();
        let path = record[3..].to_string();
        status.insert(path, code.clone());
        if code.contains('R') || code.contains('C') {
            index += 1;
            if let Some(original) = records.get(index)
                && !original.is_empty()
            {
                status.insert((*original).to_string(), code);
            }
        }
        index += 1;
    }
    status
}

fn changed_paths(
    before: &BTreeMap<String, String>,
    after: &BTreeMap<String, String>,
) -> Vec<String> {
    before
        .keys()
        .chain(after.keys())
        .filter(|path| before.get(*path) != after.get(*path))
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn preparation_summary(
    prepared: &CadenceSkillsPreparationResult,
) -> CadenceSkillsPreparationSummary {
    CadenceSkillsPreparationSummary {
        source_mode: prepared.source_mode.as_str().to_string(),
        source_root: prepared.source_root.clone(),
        skills_root: prepared.skills_root.clone(),
        git_updated: prepared.git_updated,
        link_sync_status: prepared.link_sync_status.as_str().to_string(),
        warnings: prepared.warnings.clone(),
    }
}

fn cadence_error(error: CadenceSkillsError) -> RepositoryRegistrationError {
    claude_error(
        "cadence_skills_prepare",
        error.code(),
        &error.to_string(),
        true,
        "Repair Cadence-skills availability or synchronization, then retry.",
    )
}

fn claude_error(
    stage: &str,
    reason_code: &str,
    reason: &str,
    retryable: bool,
    action: &str,
) -> RepositoryRegistrationError {
    let mut error = registration_error(stage, reason_code, reason, retryable, action);
    error.provider = Some("claude_code".to_string());
    error
}

fn git_state_error(stage: &str, reason: &str) -> RepositoryRegistrationError {
    registration_error(
        stage,
        "repository_git_state_failed",
        reason,
        true,
        "Git state could not be determined; inspect the repository manually before retrying.",
    )
}

fn registration_error(
    stage: &str,
    reason_code: &str,
    reason: &str,
    retryable: bool,
    action: &str,
) -> RepositoryRegistrationError {
    RepositoryRegistrationError::new(
        stage,
        reason_code,
        Some(sanitize_summary(reason, 4 * 1024)),
        retryable,
        action,
    )
}

fn sanitize_summary(value: &str, limit: usize) -> String {
    let mut summary = String::new();
    let mut in_escape = false;
    for character in value.chars() {
        if in_escape {
            if character.is_ascii_alphabetic() {
                in_escape = false;
            }
            continue;
        }
        if character == '\u{1b}' {
            in_escape = true;
            continue;
        }
        if character.is_control() && !matches!(character, '\n' | '\r' | '\t') {
            continue;
        }
        if summary.len() + character.len_utf8() > limit {
            summary.push_str("…[truncated]");
            break;
        }
        summary.push(character);
    }
    summary
        .split_whitespace()
        .map(|token| {
            let Some((key, _)) = token.split_once('=') else {
                return token.to_string();
            };
            let upper = key.to_ascii_uppercase();
            if ["KEY", "TOKEN", "SECRET", "PASSWORD"]
                .iter()
                .any(|marker| upper.contains(marker))
            {
                format!("{key}=[REDACTED]")
            } else {
                token.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

static INITIALIZING_PATHS: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();

struct InitializationGuard {
    path: PathBuf,
}

impl InitializationGuard {
    fn try_acquire(path: PathBuf) -> Result<Self, Box<RepositoryRegistrationError>> {
        let paths = INITIALIZING_PATHS.get_or_init(|| Mutex::new(HashSet::new()));
        let mut paths = paths
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !paths.insert(path.clone()) {
            return Err(Box::new(registration_error(
                "repository_initialization_lock",
                "repository_initialization_in_progress",
                "repository initialization is already in progress for this canonical Git root",
                true,
                "Wait for the active initialization to finish, then retry.",
            )));
        }
        Ok(Self { path })
    }
}

impl Drop for InitializationGuard {
    fn drop(&mut self) {
        if let Some(paths) = INITIALIZING_PATHS.get() {
            paths
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .remove(&self.path);
        }
    }
}

#[cfg(test)]
mod tests;
