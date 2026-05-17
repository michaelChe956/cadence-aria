use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::cross_cutting::claude_code_provider::ClaudeCodeProvider;
use crate::cross_cutting::codex_provider::CodexProvider;
use crate::cross_cutting::provider_registry::ProviderRegistry;
use crate::cross_cutting::streaming_provider::FakeStreamingProvider;
use crate::product::models::ProviderName;
use crate::web::events::EventHub;
use crate::web::runtime::WebRuntime;

#[derive(Clone)]
pub struct WebAppState {
    pub workspace_root: PathBuf,
    pub runtime: Arc<Mutex<WebRuntime>>,
    pub events: EventHub,
    pub provider_registry: Arc<ProviderRegistry>,
}

impl WebAppState {
    pub fn new(workspace_root: PathBuf, runtime: WebRuntime) -> Self {
        Self::with_events(workspace_root, runtime, EventHub::new())
    }

    pub fn with_events(workspace_root: PathBuf, runtime: WebRuntime, events: EventHub) -> Self {
        Self::with_events_and_provider_registry(
            workspace_root,
            runtime,
            events,
            default_provider_registry(),
        )
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
        }
    }
}

fn default_provider_registry() -> Arc<ProviderRegistry> {
    let mut registry = ProviderRegistry::new();
    registry.register(ProviderName::Fake, Arc::new(FakeStreamingProvider));
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
