use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use crate::cross_cutting::aria_state_paths::AriaStatePaths;
use crate::cross_cutting::bounded_command_runner::{
    BoundedCommandRunner, TokioBoundedCommandRunner,
};
use crate::cross_cutting::claude_code_provider::ClaudeCodeProvider;
use crate::cross_cutting::codex_provider::CodexProvider;
use crate::cross_cutting::image_client::ImageClient;
use crate::cross_cutting::provider_adapter::ProviderAdapter;
use crate::cross_cutting::provider_availability_gate::ProviderAvailabilityGate;
use crate::cross_cutting::provider_health::{
    ProviderHealthService, ProviderHealthSnapshot, SystemProviderHealthClock,
};
use crate::cross_cutting::provider_registry::ProviderRegistry;
use crate::cross_cutting::streaming_provider::ProviderCommand;
use crate::product::app_paths::ProductAppPaths;
use crate::product::image_create::{
    ImageCreateEngine, ImageCreateRunRegistry, SessionStore, SettingsStore,
};
use crate::product::models::ProviderName;
use crate::web::events::EventHub;
use crate::web::gateway_factory::LogicalCodebaseGatewayFactory;
use crate::web::handlers::RepositoryRegistrationDependencies;
use crate::web::runtime::WebRuntime;
use crate::web::test_controls::{TestControlledFakeStreamingProvider, TestControls};
use tokio::sync::{Mutex as AsyncMutex, mpsc};
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone)]
pub struct WorkspaceActiveRun {
    pub id: u64,
    pub token: u64,
    pub cancel: CancellationToken,
    pub command_tx: mpsc::Sender<ProviderCommand>,
    pub pending_choice_ids: Arc<AsyncMutex<HashSet<String>>>,
}

mod coding_run_registry;
pub(crate) use coding_run_registry::CodingAttemptMutationLease;
pub use coding_run_registry::{CodingAttemptRunKey, CodingRunRegistry, CodingRunReservation};
mod coding_socket_registry;
pub use coding_socket_registry::CodingSocketRegistry;
mod repository_initialization_run_registry;
pub use repository_initialization_run_registry::{
    InitializationOperationKind, InitializationRunKey, InitializationRunRegistry,
    RepositoryInitializationRunRegistry,
};

#[derive(Clone, Default)]
pub struct WorkspaceRunRegistry {
    runs: Arc<AsyncMutex<HashMap<String, WorkspaceActiveRun>>>,
}

impl WorkspaceRunRegistry {
    pub async fn insert(&self, session_id: String, run: WorkspaceActiveRun) {
        self.runs.lock().await.insert(session_id, run);
    }

    pub async fn take(&self, session_id: &str) -> Option<WorkspaceActiveRun> {
        self.runs.lock().await.remove(session_id)
    }

    pub async fn command_tx(&self, session_id: &str) -> Option<mpsc::Sender<ProviderCommand>> {
        self.runs
            .lock()
            .await
            .get(session_id)
            .map(|run| run.command_tx.clone())
    }

    pub async fn run(&self, session_id: &str) -> Option<WorkspaceActiveRun> {
        self.runs.lock().await.get(session_id).cloned()
    }

    pub async fn register_choice(&self, session_id: &str, choice_id: String) -> bool {
        let Some(run) = self.runs.lock().await.get(session_id).cloned() else {
            return false;
        };
        run.pending_choice_ids.lock().await.insert(choice_id);
        true
    }

    pub async fn remove_if_token(&self, session_id: &str, token: u64) -> bool {
        let mut runs = self.runs.lock().await;
        if runs.get(session_id).is_some_and(|run| run.token == token) {
            runs.remove(session_id);
            return true;
        }
        false
    }

    pub async fn replace_command_tx_if_token(
        &self,
        session_id: &str,
        token: u64,
        command_tx: mpsc::Sender<ProviderCommand>,
    ) {
        if let Some(run) = self.runs.lock().await.get_mut(session_id)
            && run.token == token
        {
            run.command_tx = command_tx;
        }
    }
}

#[derive(Clone)]
pub struct WebAppState {
    pub workspace_root: PathBuf,
    pub runtime: Arc<StdMutex<WebRuntime>>,
    pub events: EventHub,
    pub provider_registry: Arc<ProviderRegistry>,
    pub provider_availability: Arc<dyn Fn(&ProviderName) -> bool + Send + Sync>,
    pub provider_adapter: Arc<dyn ProviderAdapter + Send + Sync>,
    pub provider_health: Arc<ProviderHealthService>,
    pub provider_gate: Arc<ProviderAvailabilityGate>,
    pub command_runner: Arc<dyn BoundedCommandRunner>,
    repository_registration_dependencies: Option<RepositoryRegistrationDependencies>,
    aggregate_initialization_dependencies:
        Option<crate::web::handlers::AggregateInitializationDependencies>,
    pub test_provider_enabled: bool,
    provider_health_error: Arc<StdMutex<Option<String>>>,
    pub test_controls: TestControls,
    pub workspace_runs: WorkspaceRunRegistry,
    pub coding_runs: CodingRunRegistry,
    pub coding_sockets: CodingSocketRegistry,
    pub repository_initialization_runs: RepositoryInitializationRunRegistry,
    pub image_create_run_registry: Arc<ImageCreateRunRegistry>,
    pub image_create_engine: Option<Arc<ImageCreateEngine>>,
    pub logical_gateway_factory: Option<Arc<LogicalCodebaseGatewayFactory>>,
}

impl WebAppState {
    pub fn new(workspace_root: PathBuf, runtime: WebRuntime) -> Self {
        Self::with_events(workspace_root, runtime, EventHub::new())
    }

    pub fn with_events(workspace_root: PathBuf, mut runtime: WebRuntime, events: EventHub) -> Self {
        let test_controls = TestControls::default();
        let test_provider_enabled =
            provider_mode_is_fake() || !runtime.enforces_real_provider_availability();
        let command_runner: Arc<dyn BoundedCommandRunner> = Arc::new(TokioBoundedCommandRunner);
        let provider_health = Arc::new(ProviderHealthService::with_dependencies(
            AriaStatePaths::from_workspace_root(&workspace_root),
            command_runner.clone(),
            Arc::new(SystemProviderHealthClock),
            Duration::from_secs(5),
            4096,
        ));
        let provider_gate = Arc::new(ProviderAvailabilityGate::new(provider_health.clone()));
        runtime
            .install_provider_gate(provider_gate.clone())
            .expect("install shared provider gate");
        let provider_availability: Arc<dyn Fn(&ProviderName) -> bool + Send + Sync> =
            if runtime.enforces_real_provider_availability() && !test_provider_enabled {
                availability_from_gate(provider_gate.clone())
            } else {
                Arc::new(|_| true)
            };
        let provider_adapter = runtime.provider_adapter();
        let provider_registry = default_provider_registry(
            test_controls.clone(),
            provider_gate.clone(),
            test_provider_enabled,
        );
        let image_create_run_registry = Arc::new(ImageCreateRunRegistry::default());
        let image_create_engine = Some(build_image_create_engine(
            &workspace_root,
            provider_registry.clone(),
            image_create_run_registry.clone(),
        ));
        let logical_gateway_factory = Arc::new(LogicalCodebaseGatewayFactory::new(
            ProductAppPaths::new(workspace_root.join(".aria")),
            provider_registry.clone(),
            provider_adapter.clone(),
            provider_gate.clone(),
        ));
        let mut state = Self {
            workspace_root,
            runtime: Arc::new(StdMutex::new(runtime)),
            events,
            provider_registry,
            provider_availability,
            provider_adapter,
            provider_health,
            provider_gate,
            command_runner,
            repository_registration_dependencies: None,
            aggregate_initialization_dependencies: None,
            test_provider_enabled,
            provider_health_error: Arc::new(StdMutex::new(None)),
            test_controls,
            workspace_runs: WorkspaceRunRegistry::default(),
            coding_runs: CodingRunRegistry::default(),
            coding_sockets: CodingSocketRegistry::default(),
            repository_initialization_runs: RepositoryInitializationRunRegistry::default(),
            image_create_run_registry,
            image_create_engine,
            logical_gateway_factory: Some(logical_gateway_factory),
        };
        state.aggregate_initialization_dependencies = Some(
            crate::web::handlers::AggregateInitializationDependencies::production(&state)
                .expect("build aggregate initialization dependencies"),
        );
        state
    }

    pub fn with_provider_availability<F>(
        workspace_root: PathBuf,
        runtime: WebRuntime,
        provider_availability: F,
    ) -> Self
    where
        F: Fn(&ProviderName) -> bool + Send + Sync + 'static,
    {
        let mut state = Self::new(workspace_root, runtime);
        state.provider_availability = Arc::new(provider_availability);
        state
    }

    pub fn with_provider_registry(
        workspace_root: PathBuf,
        runtime: WebRuntime,
        provider_registry: ProviderRegistry,
    ) -> Self {
        Self::with_events_and_provider_registry(
            workspace_root,
            runtime,
            EventHub::new(),
            Arc::new(provider_registry),
        )
    }

    pub fn with_events_and_provider_registry(
        workspace_root: PathBuf,
        runtime: WebRuntime,
        events: EventHub,
        provider_registry: Arc<ProviderRegistry>,
    ) -> Self {
        let mut state = Self::with_events(workspace_root, runtime, events);
        state.provider_registry = provider_registry;
        state.image_create_engine = Some(build_image_create_engine(
            &state.workspace_root,
            state.provider_registry.clone(),
            state.image_create_run_registry.clone(),
        ));
        // T4 deferred minor:注入 registry 后必须用注入的 registry 同步重建
        // `logical_gateway_factory`,否则 factory 持有旧 registry,后续注入 fake
        // registry 时 gateway 解析不到 fake。
        state.logical_gateway_factory = Some(Arc::new(LogicalCodebaseGatewayFactory::new(
            ProductAppPaths::new(state.workspace_root.join(".aria")),
            state.provider_registry.clone(),
            state.provider_adapter.clone(),
            state.provider_gate.clone(),
        )));
        state.aggregate_initialization_dependencies = Some(
            crate::web::handlers::AggregateInitializationDependencies::production(&state)
                .expect("build aggregate initialization dependencies"),
        );
        state
    }

    pub fn with_provider_adapter(
        mut self,
        provider_adapter: Arc<dyn ProviderAdapter + Send + Sync>,
    ) -> Self {
        self.provider_adapter = provider_adapter;
        self
    }

    pub fn with_gateway_factory(mut self, factory: Arc<LogicalCodebaseGatewayFactory>) -> Self {
        self.logical_gateway_factory = Some(factory);
        self.aggregate_initialization_dependencies = Some(
            crate::web::handlers::AggregateInitializationDependencies::production(&self)
                .expect("build aggregate initialization dependencies"),
        );
        self
    }

    pub fn gateway_factory(&self) -> Option<&Arc<LogicalCodebaseGatewayFactory>> {
        self.logical_gateway_factory.as_ref()
    }

    pub fn with_provider_health(
        mut self,
        provider_health: Arc<ProviderHealthService>,
        provider_gate: Arc<ProviderAvailabilityGate>,
        command_runner: Arc<dyn BoundedCommandRunner>,
    ) -> Self {
        self.provider_availability = if self.test_provider_enabled {
            Arc::new(|_| true)
        } else {
            availability_from_gate(provider_gate.clone())
        };
        self.provider_health = provider_health;
        self.provider_gate = provider_gate;
        self.command_runner = command_runner;
        {
            let mut runtime = self.runtime.lock().expect("web runtime lock");
            runtime
                .install_provider_gate(self.provider_gate.clone())
                .expect("install shared provider gate");
            self.provider_adapter = runtime.provider_adapter();
        }
        *self
            .provider_health_error
            .lock()
            .expect("provider health error lock") = None;
        self
    }

    pub fn with_repository_registration_dependencies(
        mut self,
        dependencies: RepositoryRegistrationDependencies,
    ) -> Self {
        self.repository_registration_dependencies = Some(dependencies);
        self
    }

    pub(crate) fn repository_registration_dependencies(
        &self,
    ) -> Option<RepositoryRegistrationDependencies> {
        self.repository_registration_dependencies.clone()
    }

    pub fn with_aggregate_initialization_dependencies(
        mut self,
        dependencies: crate::web::handlers::AggregateInitializationDependencies,
    ) -> Self {
        self.aggregate_initialization_dependencies = Some(dependencies);
        self
    }

    /// Returns a clone of dependencies constructed once for this state.
    /// Every clone keeps the same coordinator, index operation and run registry.
    pub(crate) fn aggregate_initialization_dependencies(
        &self,
    ) -> crate::web::handlers::AggregateInitializationDependencies {
        self.aggregate_initialization_dependencies
            .clone()
            .expect("aggregate initialization dependencies are initialized")
    }

    pub async fn refresh_provider_health(&self) -> Arc<ProviderHealthSnapshot> {
        match self.provider_health.refresh(CancellationToken::new()).await {
            Ok(snapshot) => {
                *self
                    .provider_health_error
                    .lock()
                    .expect("provider health error lock") = None;
                snapshot
            }
            Err(error) => {
                *self
                    .provider_health_error
                    .lock()
                    .expect("provider health error lock") = Some(error.to_string());
                self.provider_health.latest_diagnostic()
            }
        }
    }

    pub fn provider_health_error(&self) -> Option<String> {
        self.provider_health_error
            .lock()
            .expect("provider health error lock")
            .clone()
    }
}

fn build_image_create_engine(
    workspace_root: &std::path::Path,
    provider_registry: Arc<ProviderRegistry>,
    run_registry: Arc<ImageCreateRunRegistry>,
) -> Arc<ImageCreateEngine> {
    let paths = AriaStatePaths::from_workspace_root(workspace_root);
    Arc::new(ImageCreateEngine::new(
        paths.clone(),
        Arc::new(SessionStore::new(paths.clone())),
        Arc::new(SettingsStore::new(paths)),
        Arc::new(ImageClient::new()),
        provider_registry,
        run_registry,
    ))
}

fn availability_from_gate(
    gate: Arc<ProviderAvailabilityGate>,
) -> Arc<dyn Fn(&ProviderName) -> bool + Send + Sync> {
    Arc::new(move |provider| gate.ensure_available(provider).is_ok())
}

fn provider_mode_is_fake() -> bool {
    std::env::var("ARIA_PROVIDER_MODE").as_deref() == Ok("fake")
}

fn default_provider_registry(
    test_controls: TestControls,
    provider_gate: Arc<ProviderAvailabilityGate>,
    test_provider_enabled: bool,
) -> Arc<ProviderRegistry> {
    let registry = if test_provider_enabled {
        fake_mode_provider_registry(test_controls)
    } else {
        real_provider_registry(provider_gate)
    };
    Arc::new(registry)
}

/// 生产模式 registry:只注册真实 ClaudeCode/Codex 实现,不含 `ProviderName::Fake`,
/// 也不注册 Pi。
fn real_provider_registry(provider_gate: Arc<ProviderAvailabilityGate>) -> ProviderRegistry {
    let mut registry = ProviderRegistry::new();
    registry.register_gated(
        ProviderName::ClaudeCode,
        Arc::new(ClaudeCodeProvider::new(PathBuf::from("claude"))),
        provider_gate.clone(),
    );
    registry.register_gated(
        ProviderName::Codex,
        Arc::new(CodexProvider::new(PathBuf::from("codex"))),
        provider_gate,
    );
    registry
}

/// fake 模式 registry:保持既有 fake 分支全部内容(Fake + 冒充 ClaudeCode/Codex/Pi
/// 的 `TestControlledFakeStreamingProvider`)。
fn fake_mode_provider_registry(test_controls: TestControls) -> ProviderRegistry {
    let mut registry = ProviderRegistry::new();
    registry.register(
        ProviderName::Fake,
        Arc::new(TestControlledFakeStreamingProvider::new(
            test_controls.clone(),
        )),
    );
    registry.register(
        ProviderName::ClaudeCode,
        Arc::new(TestControlledFakeStreamingProvider::new(
            test_controls.clone(),
        )),
    );
    registry.register(
        ProviderName::Codex,
        Arc::new(TestControlledFakeStreamingProvider::new(
            test_controls.clone(),
        )),
    );
    registry.register(
        ProviderName::Pi,
        Arc::new(TestControlledFakeStreamingProvider::new(test_controls)),
    );
    registry
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::ffi::OsString;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use chrono::{DateTime, Utc};
    use tempfile::tempdir;
    use tokio_util::sync::CancellationToken;

    use crate::cross_cutting::aria_state_paths::AriaStatePaths;
    use crate::cross_cutting::bounded_command_runner::{
        BoundedCommandError, BoundedCommandRequest, BoundedCommandResult, BoundedCommandRunner,
    };
    use crate::cross_cutting::provider_availability_gate::ProviderAvailabilityGate;
    use crate::cross_cutting::provider_health::{ProviderHealthClock, ProviderHealthService};
    use crate::cross_cutting::streaming_provider::{
        ProviderEvent, ProviderPermissionMode, StreamingProviderInput,
    };
    use crate::protocol::contracts::{AdapterRole, ProviderType};

    use super::*;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    struct FixedClock(DateTime<Utc>);

    impl ProviderHealthClock for FixedClock {
        fn now(&self) -> DateTime<Utc> {
            self.0
        }
    }

    struct ScriptedRunner {
        calls: AtomicUsize,
        results: Mutex<VecDeque<Result<BoundedCommandResult, BoundedCommandError>>>,
    }

    impl ScriptedRunner {
        fn new(results: Vec<Result<BoundedCommandResult, BoundedCommandError>>) -> Self {
            Self {
                calls: AtomicUsize::new(0),
                results: Mutex::new(results.into()),
            }
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait::async_trait]
    impl BoundedCommandRunner for ScriptedRunner {
        async fn run(
            &self,
            _request: BoundedCommandRequest,
        ) -> Result<BoundedCommandResult, BoundedCommandError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.results
                .lock()
                .expect("scripted results")
                .pop_front()
                .expect("scripted result")
        }
    }

    fn success(version: &str) -> Result<BoundedCommandResult, BoundedCommandError> {
        Ok(BoundedCommandResult {
            exit_code: Some(0),
            stdout: format!("provider {version}"),
            stderr: String::new(),
            timed_out: false,
            cancelled: false,
            stdout_truncated: false,
            stderr_truncated: false,
            duration_ms: 1,
        })
    }

    fn provider_health(
        root: &std::path::Path,
        runner: Arc<dyn BoundedCommandRunner>,
    ) -> Arc<ProviderHealthService> {
        Arc::new(ProviderHealthService::with_dependencies(
            AriaStatePaths::from_workspace_root(root),
            runner,
            Arc::new(FixedClock(Utc::now())),
            Duration::from_secs(1),
            4096,
        ))
    }

    struct ProviderModeGuard {
        previous: Option<OsString>,
    }

    impl ProviderModeGuard {
        fn fake() -> Self {
            let previous = std::env::var_os("ARIA_PROVIDER_MODE");
            unsafe {
                std::env::set_var("ARIA_PROVIDER_MODE", "fake");
            }
            Self { previous }
        }

        fn disabled() -> Self {
            let previous = std::env::var_os("ARIA_PROVIDER_MODE");
            unsafe {
                std::env::remove_var("ARIA_PROVIDER_MODE");
            }
            Self { previous }
        }
    }

    impl Drop for ProviderModeGuard {
        fn drop(&mut self) {
            unsafe {
                if let Some(previous) = &self.previous {
                    std::env::set_var("ARIA_PROVIDER_MODE", previous);
                } else {
                    std::env::remove_var("ARIA_PROVIDER_MODE");
                }
            }
        }
    }

    #[test]
    fn default_provider_registry_real_mode_excludes_fake_and_pi() {
        let root = tempdir().expect("root");
        let runner = Arc::new(ScriptedRunner::new(Vec::new()));
        let health = provider_health(root.path(), runner);
        let gate = Arc::new(ProviderAvailabilityGate::new(health));
        let registry = default_provider_registry(TestControls::default(), gate, false);

        assert!(registry.get(&ProviderName::ClaudeCode).is_some());
        assert!(registry.get(&ProviderName::Codex).is_some());
        assert!(registry.get(&ProviderName::Fake).is_none());
        assert!(registry.get(&ProviderName::Pi).is_none());
    }

    #[test]
    fn default_provider_registry_real_registry_only_holds_gated_claude_code_and_codex() {
        let root = tempdir().expect("root");
        let runner = Arc::new(ScriptedRunner::new(Vec::new()));
        let health = provider_health(root.path(), runner);
        let gate = Arc::new(ProviderAvailabilityGate::new(health));
        let registry = real_provider_registry(gate);

        assert!(registry.get(&ProviderName::ClaudeCode).is_some());
        assert!(registry.get(&ProviderName::Codex).is_some());
        assert!(registry.get(&ProviderName::Fake).is_none());
        assert!(registry.get(&ProviderName::Pi).is_none());
    }

    #[test]
    fn default_provider_registry_fake_mode_registry_keeps_legacy_fake_branch_contents() {
        let registry = fake_mode_provider_registry(TestControls::default());

        assert!(registry.get(&ProviderName::Fake).is_some());
        assert!(registry.get(&ProviderName::ClaudeCode).is_some());
        assert!(registry.get(&ProviderName::Codex).is_some());
        assert!(registry.get(&ProviderName::Pi).is_some());
    }

    #[test]
    fn default_provider_registry_dispatches_on_test_provider_enabled() {
        let root = tempdir().expect("root");
        let runner = Arc::new(ScriptedRunner::new(Vec::new()));
        let health = provider_health(root.path(), runner);
        let gate = Arc::new(ProviderAvailabilityGate::new(health));

        let real = default_provider_registry(TestControls::default(), gate.clone(), false);
        assert!(real.get(&ProviderName::Fake).is_none());
        assert!(real.get(&ProviderName::Pi).is_none());

        let fake = default_provider_registry(TestControls::default(), gate, true);
        assert!(fake.get(&ProviderName::Fake).is_some());
        assert!(fake.get(&ProviderName::Pi).is_some());
    }

    #[test]
    fn web_app_state_fake_runtime_registers_fake_provider_without_env_flag() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        let _provider_mode = ProviderModeGuard::disabled();
        let root = tempdir().expect("root");
        let state = WebAppState::new(
            root.path().to_path_buf(),
            WebRuntime::new_fake(root.path().to_path_buf()),
        );

        assert!(state.provider_registry.get(&ProviderName::Fake).is_some());
    }

    #[test]
    fn web_app_state_real_runtime_excludes_fake_provider() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        let _provider_mode = ProviderModeGuard::disabled();
        let root = tempdir().expect("root");
        let state = WebAppState::new(
            root.path().to_path_buf(),
            WebRuntime::new_real(root.path().to_path_buf()).expect("real runtime"),
        );

        assert!(state.provider_registry.get(&ProviderName::Fake).is_none());
    }

    #[test]
    fn image_create_engine_is_constructed_without_changing_new_signature() {
        let root = tempdir().expect("root");
        let state = WebAppState::new(
            root.path().to_path_buf(),
            WebRuntime::new_fake(root.path().to_path_buf()),
        );

        assert!(state.image_create_engine.is_some());
    }

    #[test]
    fn injected_provider_registry_rebuilds_image_create_engine() {
        let root = tempdir().expect("root");
        let provider_registry = Arc::new(ProviderRegistry::new());
        let state = WebAppState::with_events_and_provider_registry(
            root.path().to_path_buf(),
            WebRuntime::new_fake(root.path().to_path_buf()),
            EventHub::new(),
            provider_registry.clone(),
        );

        assert!(Arc::ptr_eq(&state.provider_registry, &provider_registry));
        assert!(Arc::ptr_eq(
            state
                .image_create_engine
                .as_ref()
                .expect("image create engine")
                .iteration_registry(),
            &provider_registry
        ));
    }

    #[tokio::test]
    async fn provider_mode_fake_routes_codex_workspace_provider_to_fake_adapter() {
        let root = tempdir().expect("root");
        let provider = {
            let _guard = ENV_LOCK.lock().expect("env lock");
            let _provider_mode = ProviderModeGuard::fake();
            let state = WebAppState::new(
                root.path().to_path_buf(),
                WebRuntime::new_fake(root.path().to_path_buf()),
            );
            state
                .provider_registry
                .get(&ProviderName::Codex)
                .expect("codex provider")
        };

        let mut session = provider
            .start(
                StreamingProviderInput {
                    provider_type: ProviderType::Codex,
                    role: AdapterRole::Reviewer,
                    prompt: "Workspace 类型: Story Spec\nIssue: E2E\n[user]: 开始生成".to_string(),
                    working_dir: root.path().to_path_buf(),
                    workspace_session_id: Some("workspace_session_1".to_string()),
                    resume_provider_session_id: None,
                    permission_mode: ProviderPermissionMode::Auto,
                    structured_output_contract: None,
                    env_vars: Default::default(),
                    timeout_secs: 60,
                },
                CancellationToken::new(),
            )
            .await
            .expect("fake codex provider session");

        match tokio::time::timeout(Duration::from_secs(1), session.events.recv())
            .await
            .expect("provider event")
            .expect("text delta")
        {
            ProviderEvent::TextDelta { content } => assert!(content.contains("Story Spec")),
            other => panic!("unexpected provider event: {other:?}"),
        }
    }

    #[tokio::test]
    async fn provider_availability_reads_latest_shared_health_snapshot() {
        let (_root, health, state) = {
            let _guard = ENV_LOCK.lock().expect("env lock");
            let _provider_mode = ProviderModeGuard::disabled();
            let root = tempdir().expect("root");
            let runner = Arc::new(ScriptedRunner::new(vec![
                success("1.0"),
                success("2.0"),
                success("0.83.0"),
            ]));
            let health = provider_health(root.path(), runner.clone());
            let gate = Arc::new(ProviderAvailabilityGate::new(health.clone()));
            let state = WebAppState::new(
                root.path().to_path_buf(),
                WebRuntime::new_real(root.path().to_path_buf()).expect("real runtime"),
            )
            .with_provider_health(health.clone(), gate, runner);
            assert!(!(state.provider_availability)(&ProviderName::Codex));
            (root, health, state)
        };
        health
            .refresh(CancellationToken::new())
            .await
            .expect("refresh");
        assert!((state.provider_availability)(&ProviderName::Codex));
        assert_eq!(state.provider_health.snapshot().generation, 1);
    }

    #[test]
    fn provider_availability_fake_mode_does_not_execute_real_probe() {
        let root = tempdir().expect("root");
        let runner = Arc::new(ScriptedRunner::new(Vec::new()));
        let health = provider_health(root.path(), runner.clone());
        let gate = Arc::new(ProviderAvailabilityGate::new(health.clone()));
        let state = {
            let _guard = ENV_LOCK.lock().expect("env lock");
            let _provider_mode = ProviderModeGuard::fake();
            WebAppState::new(
                root.path().to_path_buf(),
                WebRuntime::new_fake(root.path().to_path_buf()),
            )
            .with_provider_health(health, gate, runner.clone())
        };

        assert_eq!(runner.calls(), 0);
        assert!(state.test_provider_enabled);
        assert!((state.provider_availability)(&ProviderName::Fake));
        assert!((state.provider_availability)(&ProviderName::Codex));
    }
}
