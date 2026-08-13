use std::collections::HashMap;
use std::sync::Arc;

use crate::cross_cutting::provider_availability_gate::{
    GatedStreamingProviderAdapter, ProviderAvailabilityGate,
};
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

    pub fn register_gated(
        &mut self,
        name: ProviderName,
        provider: Arc<dyn StreamingProviderAdapter>,
        gate: Arc<ProviderAvailabilityGate>,
    ) {
        if name == ProviderName::Fake {
            self.register(name, provider);
            return;
        }
        let gated = Arc::new(GatedStreamingProviderAdapter::new(
            name.clone(),
            provider,
            gate,
        ));
        self.register(name, gated);
    }

    pub fn get(&self, name: &ProviderName) -> Option<Arc<dyn StreamingProviderAdapter>> {
        self.providers.get(name).cloned()
    }

    pub fn available_names(&self) -> Vec<ProviderName> {
        [
            ProviderName::ClaudeCode,
            ProviderName::Codex,
            ProviderName::Pi,
            ProviderName::KimiCode,
            ProviderName::Fake,
        ]
        .into_iter()
        .filter(|name| self.providers.contains_key(name))
        .collect()
    }

    pub fn executable_names(&self, gate: &ProviderAvailabilityGate) -> Vec<ProviderName> {
        self.available_names()
            .into_iter()
            .filter(|name| gate.ensure_available(name).is_ok())
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

    use chrono::Utc;

    use super::*;
    use crate::cross_cutting::provider_availability_gate::{
        ProviderAvailabilityGate, ProviderHealthSource,
    };
    use crate::cross_cutting::provider_health::{
        ProviderHealthEntry, ProviderHealthReasonCode, ProviderHealthSnapshot,
    };
    use crate::cross_cutting::streaming_provider::FakeStreamingProvider;

    struct RegistryHealthSource(Arc<ProviderHealthSnapshot>);

    impl ProviderHealthSource for RegistryHealthSource {
        fn snapshot(&self) -> Arc<ProviderHealthSnapshot> {
            self.0.clone()
        }

        fn degraded(&self) -> bool {
            false
        }
    }

    fn registry_gate() -> Arc<ProviderAvailabilityGate> {
        let checked_at = Utc::now();
        Arc::new(ProviderAvailabilityGate::new(Arc::new(
            RegistryHealthSource(Arc::new(ProviderHealthSnapshot {
                schema_version: 1,
                generation: 1,
                checked_at,
                providers: vec![
                    ProviderHealthEntry {
                        provider: ProviderName::ClaudeCode,
                        command: "claude --version".to_string(),
                        available: false,
                        version: None,
                        reason_code: Some(ProviderHealthReasonCode::CommandMissing),
                        reason: Some("not found".to_string()),
                        checked_at,
                    },
                    ProviderHealthEntry {
                        provider: ProviderName::Codex,
                        command: "codex --version".to_string(),
                        available: true,
                        version: Some("1.0".to_string()),
                        reason_code: None,
                        reason: None,
                        checked_at,
                    },
                ],
            })),
        )))
    }

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
        registry.register(ProviderName::Pi, Arc::new(FakeStreamingProvider));
        registry.register(ProviderName::KimiCode, Arc::new(FakeStreamingProvider));

        assert_eq!(
            registry.available_names(),
            vec![
                ProviderName::ClaudeCode,
                ProviderName::Codex,
                ProviderName::Pi,
                ProviderName::KimiCode,
                ProviderName::Fake
            ]
        );
    }

    #[test]
    fn provider_availability_gate_registry_distinguishes_registered_and_executable_names() {
        let gate = registry_gate();
        let mut registry = ProviderRegistry::new();
        registry.register_gated(
            ProviderName::ClaudeCode,
            Arc::new(FakeStreamingProvider),
            gate.clone(),
        );
        registry.register_gated(
            ProviderName::Codex,
            Arc::new(FakeStreamingProvider),
            gate.clone(),
        );
        registry.register_gated(
            ProviderName::Pi,
            Arc::new(FakeStreamingProvider),
            gate.clone(),
        );
        registry.register_gated(
            ProviderName::Fake,
            Arc::new(FakeStreamingProvider),
            gate.clone(),
        );

        assert_eq!(
            registry.available_names(),
            vec![
                ProviderName::ClaudeCode,
                ProviderName::Codex,
                ProviderName::Pi,
                ProviderName::Fake
            ]
        );
        assert_eq!(
            registry.executable_names(&gate),
            vec![ProviderName::Codex, ProviderName::Fake]
        );
    }
}
