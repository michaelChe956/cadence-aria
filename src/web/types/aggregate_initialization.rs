use serde::{Deserialize, Serialize};

use crate::web::error::ApiError;

/// One of the five stable aggregate initialization step projections. Never
/// reuses the single-repository `git_finalize` step id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct AggregateInitializationStepDto {
    pub step_id: String,
    pub status: String,
}

/// A frozen member projection captured by the deterministic preflight step.
/// Distinct from the single-repository registration DTO: it carries the
/// profile evidence digest, not a GitFinalize warning.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct AggregateMemberProjectionDto {
    pub logical_repository_id: String,
    pub checkout_id: String,
    pub revision: String,
    pub dirty: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_digest: Option<String>,
}

/// Aggregate initialization operation DTO. Shows the five stable steps, the
/// resolved profile, per-member evidence projections and the cancellation
/// record. It never carries the single-repository GitFinalize warning.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct AggregateInitializationOperationDto {
    pub operation_id: String,
    pub project_id: String,
    pub status: String,
    pub profile: Option<String>,
    pub steps: Vec<AggregateInitializationStepDto>,
    pub current_step: Option<String>,
    pub failed_step: Option<String>,
    pub member_projections: Vec<AggregateMemberProjectionDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cancellation: Option<AggregateCancellationDto>,
    pub error: Option<ApiError>,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct AggregateCancellationDto {
    pub reason_code: String,
    pub cancelled_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CreateAggregateInitializationRequest {
    pub idempotency_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CancelAggregateInitializationRequest {
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}
