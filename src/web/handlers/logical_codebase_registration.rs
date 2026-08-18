use axum::Json;
use axum::extract::{Path, State};
use chrono::Utc;
use uuid::Uuid;

use crate::product::logical_codebase::{
    AggregateRootPreflight, LogicalCodebaseFeature, LogicalCodebaseRegistrationCoordinator,
    RegistrationPreflightInput, RegistrationPreflightSnapshot, RegistrationPreflightSnapshotStore,
};
use crate::product::repository_store::RepositoryStore;
use crate::web::error::ApiResult;
use crate::web::handlers::support::{
    aggregate_root_api_error, product_app_paths, require_multi_repo_project,
};
use crate::web::state::WebAppState;
use crate::web::types::RegistrationPreflightItemDto;
use crate::web::types::{RegistrationPreflightRequest, RegistrationPreflightResponse};

include!("logical_codebase_registration/dto.inc.rs");
include!("logical_codebase_registration/preflight.inc.rs");
