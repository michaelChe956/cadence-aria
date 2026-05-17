use std::collections::HashMap;
use std::sync::Arc;

use crate::cross_cutting::streaming_provider::StreamingProviderAdapter;
use crate::product::models::ProviderName;

pub struct ProviderRegistry {
    providers: HashMap<ProviderName, Arc<dyn StreamingProviderAdapter>>,
}

impl ProviderRegistry {
    pub fn new() -> Self {
        Self {
            providers: HashMap::new(),
        }
    }

    pub fn register(&mut self, name: ProviderName, provider: Arc<dyn StreamingProviderAdapter>) {
        self.providers.insert(name, provider);
    }

    pub fn get(&self, name: &ProviderName) -> Option<Arc<dyn StreamingProviderAdapter>> {
        self.providers.get(name).cloned()
    }

    pub fn available_names(&self) -> Vec<ProviderName> {
        [
            ProviderName::ClaudeCode,
            ProviderName::Codex,
            ProviderName::Fake,
        ]
        .into_iter()
        .filter(|name| self.providers.contains_key(name))
        .collect()
    }
}

impl Default for ProviderRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::cross_cutting::streaming_provider::FakeStreamingProvider;

    #[test]
    fn provider_registry_returns_registered_fake_provider() {
        let mut registry = ProviderRegistry::new();
        registry.register(ProviderName::Fake, Arc::new(FakeStreamingProvider));

        assert!(registry.get(&ProviderName::Fake).is_some());
        assert!(registry.get(&ProviderName::ClaudeCode).is_none());
    }

    #[test]
    fn provider_registry_available_names_use_stable_provider_order() {
        let mut registry = ProviderRegistry::new();
        registry.register(ProviderName::Fake, Arc::new(FakeStreamingProvider));
        registry.register(ProviderName::ClaudeCode, Arc::new(FakeStreamingProvider));
        registry.register(ProviderName::Codex, Arc::new(FakeStreamingProvider));

        assert_eq!(
            registry.available_names(),
            vec![
                ProviderName::ClaudeCode,
                ProviderName::Codex,
                ProviderName::Fake
            ]
        );
    }
}
