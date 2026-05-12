use crate::cross_cutting::adapter_compatibility::default_compatibility_matrix;
use crate::cross_cutting::cli_adapter::{CliAdapterConfig, CliProviderAdapter};
use crate::cross_cutting::provider_adapter::{ProviderAdapter, ProviderAdapterError};
use crate::protocol::contracts::{AdapterInput, AdapterOutput, ProviderType};
use crate::task_run::types::TaskRunError;

pub struct RoutingProviderAdapter {
    claude: Box<dyn ProviderAdapter + Send + Sync>,
    codex: Box<dyn ProviderAdapter + Send + Sync>,
}

impl RoutingProviderAdapter {
    pub fn new(
        claude: Box<dyn ProviderAdapter + Send + Sync>,
        codex: Box<dyn ProviderAdapter + Send + Sync>,
    ) -> Self {
        Self { claude, codex }
    }
}

impl ProviderAdapter for RoutingProviderAdapter {
    fn run(&self, input: &AdapterInput) -> Result<AdapterOutput, ProviderAdapterError> {
        match &input.provider_type {
            ProviderType::ClaudeCode => self.claude.run(input),
            ProviderType::Codex => self.codex.run(input),
            ProviderType::Fake => Err(ProviderAdapterError::incompatible_output(
                "task run routing provider does not execute fake provider inputs",
                String::new(),
                String::new(),
            )),
        }
    }
}

pub fn real_routing_provider() -> Result<RoutingProviderAdapter, TaskRunError> {
    let matrix = default_compatibility_matrix();
    let claude = matrix
        .entry_for(ProviderType::ClaudeCode)
        .cloned()
        .ok_or_else(|| TaskRunError::new("provider_matrix_missing", "missing claude entry"))?;
    let codex = matrix
        .entry_for(ProviderType::Codex)
        .cloned()
        .ok_or_else(|| TaskRunError::new("provider_matrix_missing", "missing codex entry"))?;

    Ok(RoutingProviderAdapter::new(
        Box::new(CliProviderAdapter::new(CliAdapterConfig {
            compatibility: claude,
            expected_artifact_kind: None,
        })),
        Box::new(CliProviderAdapter::new(CliAdapterConfig {
            compatibility: codex,
            expected_artifact_kind: None,
        })),
    ))
}
