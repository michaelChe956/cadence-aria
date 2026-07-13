use std::path::PathBuf;
use std::sync::Arc;

use crate::cross_cutting::provider_adapter::ProviderAdapter;
use crate::cross_cutting::provider_availability_gate::ProviderAvailabilityGate;
use crate::protocol::contracts::ProviderType;

mod content;
mod metadata;
mod provider;
mod tasks;
mod utils;

pub struct WebRuntime {
    workspace_root: PathBuf,
    next_projection_version: u64,
    real_provider: Option<Arc<dyn ProviderAdapter + Send + Sync>>,
    provider_gate: Option<Arc<ProviderAvailabilityGate>>,
    output_sink: Option<crate::cross_cutting::cli_adapter::ProviderOutputSink>,
    host_readiness: Arc<dyn Fn() -> Result<(), crate::task_run::types::TaskRunError> + Send + Sync>,
    provider_availability: Arc<dyn Fn(&ProviderType) -> bool + Send + Sync>,
    enforce_real_provider_availability: bool,
}
