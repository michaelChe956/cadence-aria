use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex};

use crate::cross_cutting::claude_code_provider::ClaudeCodeProvider;
use crate::cross_cutting::codex_provider::CodexProvider;
use crate::cross_cutting::provider_adapter::ProviderAdapter;
use crate::cross_cutting::provider_registry::ProviderRegistry;
use crate::cross_cutting::streaming_provider::ProviderCommand;
use crate::product::coding_workspace_runner::CodingRunnerCommand;
use crate::product::models::ProviderName;
use crate::web::events::EventHub;
use crate::web::provider_availability::provider_name_available;
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

#[derive(Clone, Default)]
pub struct CodingRunRegistry {
    inner: Arc<StdMutex<CodingRunRegistryInner>>,
}

#[derive(Default)]
struct CodingRunRegistryInner {
    next_run_id: u64,
    runs: HashMap<String, HashMap<u64, mpsc::Sender<CodingRunnerCommand>>>,
}

impl CodingRunRegistry {
    pub fn insert(&self, attempt_id: String, command_tx: mpsc::Sender<CodingRunnerCommand>) -> u64 {
        let mut inner = self.inner.lock().expect("coding run registry lock");
        inner.next_run_id += 1;
        let run_id = inner.next_run_id;
        inner
            .runs
            .entry(attempt_id)
            .or_default()
            .insert(run_id, command_tx);
        run_id
    }

    pub fn remove(&self, attempt_id: &str, run_id: u64) {
        let mut inner = self.inner.lock().expect("coding run registry lock");
        if let Some(runs) = inner.runs.get_mut(attempt_id) {
            runs.remove(&run_id);
            if runs.is_empty() {
                inner.runs.remove(attempt_id);
            }
        }
    }

    pub async fn abort_attempt(&self, attempt_id: &str) -> usize {
        let senders = {
            let mut inner = self.inner.lock().expect("coding run registry lock");
            inner
                .runs
                .remove(attempt_id)
                .map(|runs| runs.into_values().collect::<Vec<_>>())
                .unwrap_or_default()
        };
        let mut sent = 0;
        for sender in senders {
            if sender.send(CodingRunnerCommand::AbortAttempt).await.is_ok() {
                sent += 1;
            }
        }
        sent
    }

    pub fn runner_count(&self, attempt_id: &str) -> usize {
        self.inner
            .lock()
            .expect("coding run registry lock")
            .runs
            .get(attempt_id)
            .map(HashMap::len)
            .unwrap_or(0)
    }
}

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
    pub test_controls: TestControls,
    pub workspace_runs: WorkspaceRunRegistry,
    pub coding_runs: CodingRunRegistry,
}

impl WebAppState {
    pub fn new(workspace_root: PathBuf, runtime: WebRuntime) -> Self {
        Self::with_events(workspace_root, runtime, EventHub::new())
    }

    pub fn with_events(workspace_root: PathBuf, runtime: WebRuntime, events: EventHub) -> Self {
        let test_controls = TestControls::default();
        let provider_availability: Arc<dyn Fn(&ProviderName) -> bool + Send + Sync> =
            if runtime.enforces_real_provider_availability() {
                Arc::new(provider_name_available)
            } else {
                Arc::new(|_| true)
            };
        let provider_adapter = runtime.provider_adapter();
        Self {
            workspace_root,
            runtime: Arc::new(StdMutex::new(runtime)),
            events,
            provider_registry: default_provider_registry(test_controls.clone()),
            provider_availability,
            provider_adapter,
            test_controls,
            workspace_runs: WorkspaceRunRegistry::default(),
            coding_runs: CodingRunRegistry::default(),
        }
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
        let provider_availability: Arc<dyn Fn(&ProviderName) -> bool + Send + Sync> =
            if runtime.enforces_real_provider_availability() {
                Arc::new(provider_name_available)
            } else {
                Arc::new(|_| true)
            };
        let provider_adapter = runtime.provider_adapter();
        Self {
            workspace_root,
            runtime: Arc::new(StdMutex::new(runtime)),
            events,
            provider_registry,
            provider_availability,
            provider_adapter,
            test_controls: TestControls::default(),
            workspace_runs: WorkspaceRunRegistry::default(),
            coding_runs: CodingRunRegistry::default(),
        }
    }

    pub fn with_provider_adapter(
        mut self,
        provider_adapter: Arc<dyn ProviderAdapter + Send + Sync>,
    ) -> Self {
        self.provider_adapter = provider_adapter;
        self
    }
}

fn default_provider_registry(test_controls: TestControls) -> Arc<ProviderRegistry> {
    let mut registry = ProviderRegistry::new();
    if std::env::var("ARIA_PROVIDER_MODE").as_deref() == Ok("fake") {
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
            Arc::new(TestControlledFakeStreamingProvider::new(test_controls)),
        );
        return Arc::new(registry);
    }

    registry.register(
        ProviderName::Fake,
        Arc::new(TestControlledFakeStreamingProvider::new(test_controls)),
    );
    registry.register(
        ProviderName::ClaudeCode,
        Arc::new(ClaudeCodeProvider::new(PathBuf::from("claude"))),
    );
    registry.register(
        ProviderName::Codex,
        Arc::new(CodexProvider::new(PathBuf::from("codex"))),
    );
    Arc::new(registry)
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::time::Duration;

    use tempfile::tempdir;
    use tokio_util::sync::CancellationToken;

    use crate::cross_cutting::streaming_provider::{
        ProviderEvent, ProviderPermissionMode, StreamingProviderInput,
    };
    use crate::protocol::contracts::{AdapterRole, ProviderType};

    use super::*;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    struct ProviderModeGuard;

    impl ProviderModeGuard {
        fn fake() -> Self {
            unsafe {
                std::env::set_var("ARIA_PROVIDER_MODE", "fake");
            }
            Self
        }
    }

    impl Drop for ProviderModeGuard {
        fn drop(&mut self) {
            unsafe {
                std::env::remove_var("ARIA_PROVIDER_MODE");
            }
        }
    }

    #[tokio::test]
    async fn coding_run_registry_aborts_all_runs_for_attempt_and_removes_them() {
        let registry = CodingRunRegistry::default();
        let (first_tx, mut first_rx) = mpsc::channel(1);
        let (second_tx, mut second_rx) = mpsc::channel(1);
        let (other_tx, mut other_rx) = mpsc::channel(1);

        registry.insert("coding_attempt_0001".to_string(), first_tx);
        registry.insert("coding_attempt_0001".to_string(), second_tx);
        registry.insert("coding_attempt_0002".to_string(), other_tx);

        assert_eq!(registry.runner_count("coding_attempt_0001"), 2);
        assert_eq!(registry.abort_attempt("coding_attempt_0001").await, 2);
        assert_eq!(registry.runner_count("coding_attempt_0001"), 0);
        assert_eq!(registry.runner_count("coding_attempt_0002"), 1);
        assert_eq!(
            first_rx.recv().await.expect("first abort"),
            CodingRunnerCommand::AbortAttempt
        );
        assert_eq!(
            second_rx.recv().await.expect("second abort"),
            CodingRunnerCommand::AbortAttempt
        );
        assert!(other_rx.try_recv().is_err());
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
}
