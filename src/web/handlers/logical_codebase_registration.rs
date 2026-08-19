use axum::Json;
use axum::extract::{Path, State};
use chrono::Utc;
use uuid::Uuid;

use crate::product::logical_codebase::{
    AggregateRootPreflight, CanonicalAggregateRoot, ConfirmedRegistrationBatchInput,
    LogicalCodebaseRegistrationCoordinator, LogicalCodebaseStore, RegistrationBatchStatus,
    RegistrationCandidateState, RegistrationPreflightInput, RegistrationPreflightResult,
    RegistrationPreflightSnapshot, RegistrationPreflightSnapshotStore,
};
use crate::web::error::{ApiError, ApiResult};
use crate::web::handlers::support::{
    aggregate_root_api_error, default_logical_codebase_id, product_app_paths,
    product_store_api_error, require_logical_codebase,
};
use crate::web::state::WebAppState;
use crate::web::types::{
    RegistrationBatchDto, RegistrationBatchItemDto, RegistrationPreflightItemDto,
    RegistrationSubmitRequest,
};
use crate::web::types::{RegistrationPreflightRequest, RegistrationPreflightResponse};

include!("logical_codebase_registration/dto.inc.rs");
include!("logical_codebase_registration/preflight.inc.rs");
include!("logical_codebase_registration/submit.inc.rs");
include!("logical_codebase_registration/query.inc.rs");
include!("logical_codebase_registration/resume_cancel.inc.rs");
