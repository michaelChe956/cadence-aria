use std::path::PathBuf;
use std::sync::Arc;

use crate::cross_cutting::provider_adapter::ProviderAdapter;
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
    provider_availability: Arc<dyn Fn(&ProviderType) -> bool + Send + Sync>,
    enforce_real_provider_availability: bool,
}
