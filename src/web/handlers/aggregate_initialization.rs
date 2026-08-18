//! HTTP facade for the aggregate initialization operation.
//!
//! The implementation is split into bounded include units so the public
//! handler module remains easy to navigate while preserving its API.

use super::dto::aggregate_initialization_dto;
use super::support::{product_app_paths, product_store_api_error, require_multi_repo_project};
use super::*;

use std::sync::Arc;

use crate::cross_cutting::bounded_command_runner::TokioBoundedCommandRunner;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;
use sha2::{Digest, Sha256};
use tokio_util::sync::CancellationToken;

use crate::product::app_paths::ProductAppPaths;
use crate::product::cadence_skills::{CadenceSkillsManager, CadenceSkillsPaths};
use crate::product::json_store::validate_relative_id;
use crate::product::logical_codebase::aggregate_index::{
    AggregateIndexOperation, CodeGraphCli, CodeGraphExcludeGenerator,
};
use crate::product::logical_codebase::aggregate_initialization_coordinator::{
    AggregateInitializationCoordinator, AggregateInitializationError, AggregatePreflightService,
    AggregatePreflightSnapshot, AggregateProviderTurnDriver, AggregateSkillsPreparation,
    DeterministicAggregatePreflightService, GatewayBackedAggregateProviderTurnDriver,
    MachineSkillsPreparation,
};
use crate::product::logical_codebase::aggregate_initialization_store::AggregateInitializationOperationStore;
use crate::product::logical_codebase::{
    AggregateInitializationOperationStatus, AggregateInitializationStepKind,
};
use crate::web::error::ApiError;
use crate::web::gateway_factory::LogicalCodebaseGatewayFactory;
use crate::web::state::{InitializationRunKey, InitializationRunRegistry, WebAppState};
use crate::web::types::{
    CancelAggregateInitializationRequest, CreateAggregateInitializationRequest,
};

include!("aggregate_initialization/dependencies.inc.rs");
include!("aggregate_initialization/production_dependencies.inc.rs");
include!("aggregate_initialization/handlers.inc.rs");

#[cfg(test)]
mod tests {
    use super::*;
    include!("aggregate_initialization/tests.inc.rs");
}
