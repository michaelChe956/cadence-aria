use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex};

use crate::cross_cutting::claude_code_provider::ClaudeCodeProvider;
use crate::cross_cutting::codex_provider::CodexProvider;
use crate::cross_cutting::provider_registry::ProviderRegistry;
use crate::cross_cutting::streaming_provider::ProviderCommand;
use crate::product::models::ProviderName;
use crate::web::events::EventHub;
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
    pub test_controls: TestControls,
    pub workspace_runs: WorkspaceRunRegistry,
}

impl WebAppState {
    pub fn new(workspace_root: PathBuf, runtime: WebRuntime) -> Self {
        Self::with_events(workspace_root, runtime, EventHub::new())
    }

    pub fn with_events(workspace_root: PathBuf, runtime: WebRuntime, events: EventHub) -> Self {
        let test_controls = TestControls::default();
        Self {
            workspace_root,
            runtime: Arc::new(StdMutex::new(runtime)),
            events,
            provider_registry: default_provider_registry(test_controls.clone()),
            test_controls,
            workspace_runs: WorkspaceRunRegistry::default(),
        }
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
        Self {
            workspace_root,
            runtime: Arc::new(StdMutex::new(runtime)),
            events,
            provider_registry,
            test_controls: TestControls::default(),
            workspace_runs: WorkspaceRunRegistry::default(),
        }
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
