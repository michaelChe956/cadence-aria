mod cases;

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tempfile::TempDir;
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

use super::{
    CadenceSkillsPreparation, ProjectLookup, RepositoryInitializer, RepositoryPersistence,
    RepositoryRegistrationCoordinator,
};
use crate::cross_cutting::bounded_command_runner::{
    BoundedCommandError, BoundedCommandRequest, BoundedCommandResult, BoundedCommandRunner,
};
use crate::cross_cutting::provider_availability_gate::{
    ProviderAvailabilityGate, ProviderHealthSource,
};
use crate::cross_cutting::provider_health::{ProviderHealthEntry, ProviderHealthSnapshot};
use crate::cross_cutting::provider_registry::ProviderRegistry;
use crate::cross_cutting::streaming_provider::FakeStreamingProvider;
use crate::product::app_paths::ProductAppPaths;
use crate::product::cadence_skills::{
    CadenceSkillsError, CadenceSkillsPreparationResult, CadenceSkillsSourceMode, LinkSyncStatus,
};
use crate::product::json_store::ProductStoreError;
use crate::product::models::{ProjectRecord, ProviderName, RepositoryRecord};
use crate::product::repository_store::{
    CreateRepositoryInput, RepositoryInitializationCommandSummary,
    RepositoryInitializationOperationStore, RepositoryInitializationProgress,
    RepositoryInitializationStepKind, RepositoryRegistrationError, RepositoryRegistrationInput,
};

struct AvailableHealth;

impl ProviderHealthSource for AvailableHealth {
    fn snapshot(&self) -> Arc<ProviderHealthSnapshot> {
        Arc::new(ProviderHealthSnapshot {
            schema_version: 1,
            generation: 1,
            checked_at: chrono::Utc::now(),
            providers: vec![ProviderHealthEntry {
                provider: ProviderName::ClaudeCode,
                command: "claude --version".to_string(),
                available: true,
                reason_code: None,
                reason: None,
                version: Some("1.0.0".to_string()),
                checked_at: chrono::Utc::now(),
            }],
        })
    }

    fn degraded(&self) -> bool {
        false
    }
}

struct UnavailableHealth;

impl ProviderHealthSource for UnavailableHealth {
    fn snapshot(&self) -> Arc<ProviderHealthSnapshot> {
        let mut snapshot = AvailableHealth.snapshot().as_ref().clone();
        snapshot.providers[0].available = false;
        snapshot.providers[0].reason = Some("claude missing".to_string());
        Arc::new(snapshot)
    }

    fn degraded(&self) -> bool {
        false
    }
}

struct RecordingProjectLookup {
    calls: Arc<Mutex<Vec<&'static str>>>,
}

impl ProjectLookup for RecordingProjectLookup {
    fn get_project(&self, project_id: &str) -> Result<ProjectRecord, ProductStoreError> {
        self.calls.lock().unwrap().push("project_get");
        Ok(ProjectRecord {
            id: project_id.to_string(),
            name: "Project".to_string(),
            description: None,
            created_at: "2026-07-13T00:00:00Z".to_string(),
            updated_at: "2026-07-13T00:00:00Z".to_string(),
            last_opened_at: None,
        })
    }
}

struct RecordingRepositoryPersistence {
    calls: Arc<Mutex<Vec<&'static str>>>,
    created: AtomicUsize,
}

impl RepositoryPersistence for RecordingRepositoryPersistence {
    fn find_by_path(
        &self,
        _project_id: &str,
        _path: &std::path::Path,
    ) -> Result<Option<RepositoryRecord>, ProductStoreError> {
        self.calls.lock().unwrap().push("repository_find");
        Ok(None)
    }

    fn create_repository(
        &self,
        input: CreateRepositoryInput,
    ) -> Result<RepositoryRecord, ProductStoreError> {
        self.calls.lock().unwrap().push("repository_create");
        self.created.fetch_add(1, Ordering::SeqCst);
        Ok(RepositoryRecord {
            id: "repository_0001".to_string(),
            project_id: input.project_id,
            name: input.name,
            repo_hash: "hash".to_string(),
            runtime_root: input.path.join(".aria/runtime"),
            path: input.path,
            default_policy_preset: input
                .default_policy_preset
                .unwrap_or_else(|| "manual-write".to_string()),
            default_provider_mode: input
                .default_provider_mode
                .unwrap_or_else(|| "fake".to_string()),
            created_at: "2026-07-13T00:00:00Z".to_string(),
            updated_at: "2026-07-13T00:00:00Z".to_string(),
        })
    }
}

struct RecordingCadence {
    calls: Arc<Mutex<Vec<&'static str>>>,
    source_root: std::path::PathBuf,
}

#[async_trait::async_trait]
impl CadenceSkillsPreparation for RecordingCadence {
    async fn prepare_skills(
        &self,
        _cancellation: CancellationToken,
    ) -> Result<CadenceSkillsPreparationResult, CadenceSkillsError> {
        self.calls.lock().unwrap().push("cadence_prepare");
        Ok(CadenceSkillsPreparationResult {
            source_mode: CadenceSkillsSourceMode::Offline,
            source_root: self.source_root.clone(),
            skills_root: self.source_root.join("skills"),
            git_updated: false,
            link_sync_status: LinkSyncStatus::Synchronized,
            warnings: vec!["offline source".to_string()],
        })
    }
}

struct RecordingInitializer {
    calls: Arc<Mutex<Vec<&'static str>>>,
}

#[async_trait::async_trait]
impl RepositoryInitializer for RecordingInitializer {
    async fn initialize_repository(
        &self,
        _git_root: &std::path::Path,
        _command_timeout: Duration,
        _cancellation: CancellationToken,
        progress: Arc<dyn RepositoryInitializationProgress>,
    ) -> Result<Vec<RepositoryInitializationCommandSummary>, RepositoryRegistrationError> {
        self.calls.lock().unwrap().push("initializer");
        let mut summaries = Vec::with_capacity(4);
        for (index, step) in RepositoryInitializationStepKind::ALL
            .into_iter()
            .filter(|step| step.command().is_some())
            .enumerate()
        {
            let command = step.command().expect("Claude initialization command");
            progress.step_started(step).map_err(|error| *error)?;
            progress.step_completed(step).map_err(|error| *error)?;
            summaries.push(RepositoryInitializationCommandSummary {
                command_index: index + 1,
                command: command.to_string(),
                status: "completed".to_string(),
                output_summary: None,
            });
        }
        Ok(summaries)
    }
}

struct RecordingRunner {
    calls: Arc<Mutex<Vec<&'static str>>>,
    root: std::path::PathBuf,
    status_calls: AtomicUsize,
}

struct ConfigProjectLookup {
    exists: bool,
}

impl ProjectLookup for ConfigProjectLookup {
    fn get_project(&self, project_id: &str) -> Result<ProjectRecord, ProductStoreError> {
        if !self.exists {
            return Err(ProductStoreError::NotFound {
                kind: "project",
                id: project_id.to_string(),
            });
        }
        Ok(ProjectRecord {
            id: project_id.to_string(),
            name: "Project".to_string(),
            description: None,
            created_at: "now".to_string(),
            updated_at: "now".to_string(),
            last_opened_at: None,
        })
    }
}

struct ConfigRepositoryPersistence {
    find_results: Mutex<VecDeque<Option<RepositoryRecord>>>,
    create_count: AtomicUsize,
    fail_create: AtomicBool,
}

impl ConfigRepositoryPersistence {
    fn new(find_results: Vec<Option<RepositoryRecord>>, fail_create: bool) -> Self {
        Self {
            find_results: Mutex::new(find_results.into()),
            create_count: AtomicUsize::new(0),
            fail_create: AtomicBool::new(fail_create),
        }
    }
}

impl RepositoryPersistence for ConfigRepositoryPersistence {
    fn find_by_path(
        &self,
        _project_id: &str,
        _path: &std::path::Path,
    ) -> Result<Option<RepositoryRecord>, ProductStoreError> {
        Ok(self.find_results.lock().unwrap().pop_front().flatten())
    }

    fn create_repository(
        &self,
        input: CreateRepositoryInput,
    ) -> Result<RepositoryRecord, ProductStoreError> {
        self.create_count.fetch_add(1, Ordering::SeqCst);
        if self.fail_create.load(Ordering::SeqCst) {
            return Err(ProductStoreError::Io("persist failed".to_string()));
        }
        Ok(repository_record(&input.project_id, input.path))
    }
}

struct ConfigRunner {
    root: std::path::PathBuf,
    rev_parse: Mutex<Option<BoundedCommandResult>>,
    statuses: Mutex<VecDeque<BoundedCommandResult>>,
    call_count: AtomicUsize,
}

#[async_trait::async_trait]
impl BoundedCommandRunner for ConfigRunner {
    async fn run(
        &self,
        request: BoundedCommandRequest,
    ) -> Result<BoundedCommandResult, BoundedCommandError> {
        self.call_count.fetch_add(1, Ordering::SeqCst);
        if request.argv == ["rev-parse", "--show-toplevel"] {
            return Ok(self.rev_parse.lock().unwrap().take().unwrap_or_else(|| {
                command_result(Some(0), &format!("{}\n", self.root.display()), "")
            }));
        }
        Ok(self
            .statuses
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| command_result(Some(0), "", "")))
    }
}

struct StaticCadence {
    failure: Option<&'static str>,
    count: AtomicUsize,
    source_root: std::path::PathBuf,
}

#[async_trait::async_trait]
impl CadenceSkillsPreparation for StaticCadence {
    async fn prepare_skills(
        &self,
        _cancellation: CancellationToken,
    ) -> Result<CadenceSkillsPreparationResult, CadenceSkillsError> {
        self.count.fetch_add(1, Ordering::SeqCst);
        match self.failure {
            Some("cadence_skills_unavailable") => Err(CadenceSkillsError::unavailable(
                "prepare",
                "unavailable",
                "repair",
            )),
            Some("cadence_skills_update_failed") => Err(CadenceSkillsError::update_failed(
                "update", "failed", "repair",
            )),
            Some("cadence_skills_sync_failed") => Err(CadenceSkillsError::sync_failed(
                "claude",
                "skill",
                &self.source_root,
                "failed",
            )),
            _ => Ok(cadence_result(&self.source_root)),
        }
    }
}

struct StaticInitializer {
    fail: bool,
    count: AtomicUsize,
}

#[async_trait::async_trait]
impl RepositoryInitializer for StaticInitializer {
    async fn initialize_repository(
        &self,
        _git_root: &std::path::Path,
        _command_timeout: Duration,
        _cancellation: CancellationToken,
        progress: Arc<dyn RepositoryInitializationProgress>,
    ) -> Result<Vec<RepositoryInitializationCommandSummary>, RepositoryRegistrationError> {
        self.count.fetch_add(1, Ordering::SeqCst);
        if self.fail {
            let step = RepositoryInitializationStepKind::PreCheck;
            progress.step_started(step).map_err(|error| *error)?;
            progress.step_completed(step).map_err(|error| *error)?;
            let step = RepositoryInitializationStepKind::RuleConfig;
            progress.step_started(step).map_err(|error| *error)?;
            return Err(RepositoryRegistrationError::for_command(
                "repository_initialization",
                "repository_init_command_failed",
                2,
                step.command().expect("Claude initialization command"),
                Some("failed".to_string()),
                true,
                "inspect",
            ));
        }
        let mut summaries = Vec::with_capacity(4);
        for (index, step) in RepositoryInitializationStepKind::ALL
            .into_iter()
            .filter(|step| step.command().is_some())
            .enumerate()
        {
            let command = step.command().expect("Claude initialization command");
            progress.step_started(step).map_err(|error| *error)?;
            progress.step_completed(step).map_err(|error| *error)?;
            summaries.push(RepositoryInitializationCommandSummary {
                command_index: index + 1,
                command: command.to_string(),
                status: "completed".to_string(),
                output_summary: None,
            });
        }
        Ok(summaries)
    }
}

struct BlockingCadence {
    entered: Arc<Semaphore>,
    release: Arc<Semaphore>,
    count: AtomicUsize,
    source_root: std::path::PathBuf,
}

struct FailOnceCadence {
    count: AtomicUsize,
    source_root: std::path::PathBuf,
}

#[async_trait::async_trait]
impl CadenceSkillsPreparation for FailOnceCadence {
    async fn prepare_skills(
        &self,
        _cancellation: CancellationToken,
    ) -> Result<CadenceSkillsPreparationResult, CadenceSkillsError> {
        if self.count.fetch_add(1, Ordering::SeqCst) == 0 {
            Err(CadenceSkillsError::unavailable(
                "prepare",
                "first attempt fails",
                "retry",
            ))
        } else {
            Ok(cadence_result(&self.source_root))
        }
    }
}

#[async_trait::async_trait]
impl CadenceSkillsPreparation for BlockingCadence {
    async fn prepare_skills(
        &self,
        _cancellation: CancellationToken,
    ) -> Result<CadenceSkillsPreparationResult, CadenceSkillsError> {
        self.count.fetch_add(1, Ordering::SeqCst);
        self.entered.add_permits(1);
        self.release.acquire().await.unwrap().forget();
        Ok(cadence_result(&self.source_root))
    }
}

struct CwdRootRunner;

#[async_trait::async_trait]
impl BoundedCommandRunner for CwdRootRunner {
    async fn run(
        &self,
        request: BoundedCommandRequest,
    ) -> Result<BoundedCommandResult, BoundedCommandError> {
        if request.argv == ["rev-parse", "--show-toplevel"] {
            Ok(command_result(
                Some(0),
                &format!("{}\n", request.working_dir.display()),
                "",
            ))
        } else {
            Ok(command_result(Some(0), "", ""))
        }
    }
}

fn command_result(exit_code: Option<i32>, stdout: &str, stderr: &str) -> BoundedCommandResult {
    BoundedCommandResult {
        exit_code,
        stdout: stdout.to_string(),
        stderr: stderr.to_string(),
        timed_out: false,
        cancelled: false,
        stdout_truncated: false,
        stderr_truncated: false,
        duration_ms: 1,
    }
}

fn repository_record(project_id: &str, path: std::path::PathBuf) -> RepositoryRecord {
    RepositoryRecord {
        id: "repository_0001".to_string(),
        project_id: project_id.to_string(),
        name: "Repository".to_string(),
        repo_hash: "hash".to_string(),
        runtime_root: path.join(".aria/runtime"),
        path,
        default_policy_preset: "manual-write".to_string(),
        default_provider_mode: "claude_code".to_string(),
        created_at: "now".to_string(),
        updated_at: "now".to_string(),
    }
}

fn cadence_result(source_root: &std::path::Path) -> CadenceSkillsPreparationResult {
    CadenceSkillsPreparationResult {
        source_mode: CadenceSkillsSourceMode::Offline,
        source_root: source_root.to_path_buf(),
        skills_root: source_root.join("skills"),
        git_updated: false,
        link_sync_status: LinkSyncStatus::Synchronized,
        warnings: Vec::new(),
    }
}

fn registry() -> Arc<ProviderRegistry> {
    let mut registry = ProviderRegistry::new();
    registry.register(ProviderName::ClaudeCode, Arc::new(FakeStreamingProvider));
    Arc::new(registry)
}

#[allow(clippy::too_many_arguments)]
fn coordinator(
    projects: Arc<dyn ProjectLookup>,
    repositories: Arc<dyn RepositoryPersistence>,
    gate: Arc<ProviderAvailabilityGate>,
    cadence: Arc<dyn CadenceSkillsPreparation>,
    host: Arc<dyn Fn() -> Result<(), String> + Send + Sync>,
    runner: Arc<dyn BoundedCommandRunner>,
    initializer: Arc<dyn RepositoryInitializer>,
) -> RepositoryRegistrationCoordinator {
    RepositoryRegistrationCoordinator::new_with_operations(
        projects,
        repositories,
        RepositoryInitializationOperationStore::new(ProductAppPaths::new(
            std::env::temp_dir().join(format!(
                "repository-registration-test-{}",
                uuid::Uuid::new_v4()
            )),
        )),
        gate,
        registry(),
        cadence,
        host,
        runner,
        Arc::new(|| "completed-at".to_string()),
        initializer,
        Duration::from_secs(1),
        Duration::from_secs(1),
    )
}

fn input(path: std::path::PathBuf) -> RepositoryRegistrationInput {
    RepositoryRegistrationInput {
        project_id: "project_0001".to_string(),
        name: "Repository".to_string(),
        path,
        default_policy_preset: None,
        default_provider_mode: None,
    }
}

struct OperationCoordinatorFixture {
    _temp: TempDir,
    coordinator: RepositoryRegistrationCoordinator,
    operations: RepositoryInitializationOperationStore,
    repositories: Arc<ConfigRepositoryPersistence>,
    input: RepositoryRegistrationInput,
}

#[allow(clippy::too_many_arguments)]
fn coordinator_with_operations(
    projects: Arc<dyn ProjectLookup>,
    repositories: Arc<dyn RepositoryPersistence>,
    operations: RepositoryInitializationOperationStore,
    gate: Arc<ProviderAvailabilityGate>,
    cadence: Arc<dyn CadenceSkillsPreparation>,
    host: Arc<dyn Fn() -> Result<(), String> + Send + Sync>,
    runner: Arc<dyn BoundedCommandRunner>,
    initializer: Arc<dyn RepositoryInitializer>,
) -> RepositoryRegistrationCoordinator {
    RepositoryRegistrationCoordinator::new_with_operations(
        projects,
        repositories,
        operations,
        gate,
        registry(),
        cadence,
        host,
        runner,
        Arc::new(|| "completed-at".to_string()),
        initializer,
        Duration::from_secs(1),
        Duration::from_secs(1),
    )
}

fn operation_coordinator_fixture(
    cadence_fails: bool,
    initializer_fails: bool,
) -> OperationCoordinatorFixture {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("repository");
    std::fs::create_dir_all(&root).unwrap();
    let operations = RepositoryInitializationOperationStore::new(ProductAppPaths::new(
        temp.path().join(".aria"),
    ));
    let repositories = Arc::new(ConfigRepositoryPersistence::new(vec![], false));
    let coordinator = RepositoryRegistrationCoordinator::new_with_operations(
        Arc::new(ConfigProjectLookup { exists: true }),
        repositories.clone(),
        operations.clone(),
        Arc::new(ProviderAvailabilityGate::new(Arc::new(AvailableHealth))),
        registry(),
        Arc::new(StaticCadence {
            failure: cadence_fails.then_some("cadence_skills_unavailable"),
            count: AtomicUsize::new(0),
            source_root: temp.path().join("cadence"),
        }),
        Arc::new(|| Ok(())),
        Arc::new(ConfigRunner {
            root: root.clone(),
            rev_parse: Mutex::new(None),
            statuses: Mutex::new(
                vec![
                    command_result(Some(0), "", ""),
                    command_result(Some(0), "?? generated.txt\0", ""),
                ]
                .into(),
            ),
            call_count: AtomicUsize::new(0),
        }),
        Arc::new(|| "completed-at".to_string()),
        Arc::new(StaticInitializer {
            fail: initializer_fails,
            count: AtomicUsize::new(0),
        }),
        Duration::from_secs(1),
        Duration::from_secs(1),
    );

    OperationCoordinatorFixture {
        _temp: temp,
        coordinator,
        operations,
        repositories,
        input: input(root),
    }
}

#[async_trait::async_trait]
impl BoundedCommandRunner for RecordingRunner {
    async fn run(
        &self,
        request: BoundedCommandRequest,
    ) -> Result<BoundedCommandResult, BoundedCommandError> {
        let (label, result) = match request.argv.as_slice() {
            [first, second] if first == "rev-parse" && second == "--show-toplevel" => (
                "git_root",
                command_result(Some(0), &format!("{}\n", self.root.display()), ""),
            ),
            [first, second, third, fourth]
                if first == "status"
                    && second == "--porcelain=v1"
                    && third == "-z"
                    && fourth == "--untracked-files=all" =>
            {
                let stdout = if self.status_calls.fetch_add(1, Ordering::SeqCst) == 0 {
                    " M existing.txt\0"
                } else {
                    " M existing.txt\0?? generated.txt\0"
                };
                ("git_status", command_result(Some(0), stdout, ""))
            }
            [first, second] if first == "add" && second == "-A" => {
                ("git_finalize_add", command_result(Some(0), "", ""))
            }
            [first, second, third]
                if first == "diff" && second == "--cached" && third == "--quiet" =>
            {
                ("git_finalize_diff", command_result(Some(1), "", ""))
            }
            [first, second, message]
                if first == "commit"
                    && second == "-m"
                    && message == "初始化cadence-aria 代码库" =>
            {
                ("git_finalize_commit", command_result(Some(0), "", ""))
            }
            [first] if first == "remote" => (
                "git_finalize_remote",
                command_result(Some(0), "origin\n", ""),
            ),
            [first, second, third, fourth]
                if first == "rev-parse"
                    && second == "--abbrev-ref"
                    && third == "--symbolic-full-name"
                    && fourth == "@{u}" =>
            {
                (
                    "git_finalize_upstream",
                    command_result(Some(0), "origin/main\n", ""),
                )
            }
            [first] if first == "push" => ("git_finalize_push", command_result(Some(0), "", "")),
            argv => panic!("unexpected Git argv: {argv:?}"),
        };
        self.calls.lock().unwrap().push(label);
        Ok(result)
    }
}

struct GitFinalizeRunner {
    calls: Arc<Mutex<Vec<&'static str>>>,
    root: std::path::PathBuf,
    status_calls: AtomicUsize,
}

#[async_trait::async_trait]
impl BoundedCommandRunner for GitFinalizeRunner {
    async fn run(
        &self,
        request: BoundedCommandRequest,
    ) -> Result<BoundedCommandResult, BoundedCommandError> {
        let (label, result) = match request.argv.as_slice() {
            [first, second] if first == "rev-parse" && second == "--show-toplevel" => (
                "git_root",
                command_result(Some(0), &format!("{}\n", self.root.display()), ""),
            ),
            [first, second, third, fourth]
                if first == "status"
                    && second == "--porcelain=v1"
                    && third == "-z"
                    && fourth == "--untracked-files=all" =>
            {
                let stdout = if self.status_calls.fetch_add(1, Ordering::SeqCst) == 0 {
                    " M existing.txt\0"
                } else {
                    " M existing.txt\0?? generated.txt\0"
                };
                ("git_status", command_result(Some(0), stdout, ""))
            }
            [first, second] if first == "add" && second == "-A" => {
                ("git_finalize_add", command_result(Some(0), "", ""))
            }
            [first, second, third]
                if first == "diff" && second == "--cached" && third == "--quiet" =>
            {
                ("git_finalize_diff", command_result(Some(1), "", ""))
            }
            [first, second, message]
                if first == "commit"
                    && second == "-m"
                    && message == "初始化cadence-aria 代码库" =>
            {
                ("git_finalize_commit", command_result(Some(0), "", ""))
            }
            [first] if first == "remote" => (
                "git_finalize_remote",
                command_result(Some(0), "origin\n", ""),
            ),
            [first, second, third, fourth]
                if first == "rev-parse"
                    && second == "--abbrev-ref"
                    && third == "--symbolic-full-name"
                    && fourth == "@{u}" =>
            {
                (
                    "git_finalize_upstream",
                    command_result(Some(0), "origin/main\n", ""),
                )
            }
            [first] if first == "push" => ("git_finalize_push", command_result(Some(0), "", "")),
            argv => panic!("unexpected Git argv: {argv:?}"),
        };
        self.calls.lock().unwrap().push(label);
        Ok(result)
    }
}

struct ScriptedGitRunner {
    calls: Mutex<Vec<Vec<String>>>,
    responses: Mutex<VecDeque<(Vec<String>, BoundedCommandResult)>>,
}

impl ScriptedGitRunner {
    fn new(responses: Vec<(Vec<String>, BoundedCommandResult)>) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            responses: Mutex::new(responses.into()),
        }
    }
}

#[async_trait::async_trait]
impl BoundedCommandRunner for ScriptedGitRunner {
    async fn run(
        &self,
        request: BoundedCommandRequest,
    ) -> Result<BoundedCommandResult, BoundedCommandError> {
        let (expected_argv, result) = self
            .responses
            .lock()
            .unwrap()
            .pop_front()
            .expect("unexpected extra Git invocation");
        assert_eq!(request.argv, expected_argv, "unexpected Git argv");
        self.calls.lock().unwrap().push(request.argv);
        Ok(result)
    }
}
