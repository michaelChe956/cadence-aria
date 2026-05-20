use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::cross_cutting::claude_code_provider::ClaudeCodeProvider;
use crate::cross_cutting::codex_provider::CodexProvider;
use crate::cross_cutting::provider_registry::ProviderRegistry;
use crate::product::models::ProviderName;
use crate::web::events::EventHub;
use crate::web::runtime::WebRuntime;
use crate::web::test_controls::{TestControlledFakeStreamingProvider, TestControls};

#[derive(Clone)]
pub struct WebAppState {
    pub workspace_root: PathBuf,
    pub runtime: Arc<Mutex<WebRuntime>>,
    pub events: EventHub,
    pub provider_registry: Arc<ProviderRegistry>,
    pub test_controls: TestControls,
}

impl WebAppState {
    pub fn new(workspace_root: PathBuf, runtime: WebRuntime) -> Self {
        Self::with_events(workspace_root, runtime, EventHub::new())
    }

    pub fn with_events(workspace_root: PathBuf, runtime: WebRuntime, events: EventHub) -> Self {
        let test_controls = TestControls::default();
        Self {
            workspace_root,
            runtime: Arc::new(Mutex::new(runtime)),
            events,
            provider_registry: default_provider_registry(test_controls.clone()),
            test_controls,
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
            runtime: Arc::new(Mutex::new(runtime)),
            events,
            provider_registry,
            test_controls: TestControls::default(),
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
        let _guard = ENV_LOCK.lock().expect("env lock");
        let _provider_mode = ProviderModeGuard::fake();
        let root = tempdir().expect("root");
        let state = WebAppState::new(
            root.path().to_path_buf(),
            WebRuntime::new_fake(root.path().to_path_buf()),
        );
        let provider = state
            .provider_registry
            .get(&ProviderName::Codex)
            .expect("codex provider");

        let mut session = provider
            .start(
                StreamingProviderInput {
                    provider_type: ProviderType::Codex,
                    role: AdapterRole::Reviewer,
                    prompt: "Workspace 类型: Story Spec\nIssue: E2E\n[user]: 开始生成".to_string(),
                    working_dir: root.path().to_path_buf(),
                    session_id: Some("workspace_session_1".to_string()),
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
