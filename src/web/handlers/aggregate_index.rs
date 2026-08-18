//! HTTP projection and synchronous rebuild endpoints for aggregate indexes.

use super::support::{product_app_paths, require_multi_repo_project};
use super::*;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

use crate::product::json_store::validate_relative_id;
use crate::product::logical_codebase::aggregate_index::{
    AggregateIndexError, AggregateIndexRecord, AggregateIndexStatus,
};
use crate::web::error::ApiError;
use crate::web::state::WebAppState;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct AggregateIndexActiveResponse {
    pub state: &'static str,
    pub revision: Option<u64>,
    pub indexed_at: Option<String>,
    pub warning: Option<String>,
}

pub async fn get_active_aggregate_index(
    State(state): State<WebAppState>,
    Path(project_id): Path<String>,
) -> ApiResult<Response> {
    let paths = product_app_paths(&state);
    require_multi_repo_project(&paths, &project_id)?;
    validate_project_id(&project_id)?;
    let response = read_active_projection(&paths, &project_id)?;
    Ok((StatusCode::OK, Json(response)).into_response())
}

pub async fn rebuild_aggregate_index(
    State(state): State<WebAppState>,
    Path(project_id): Path<String>,
) -> ApiResult<Response> {
    let paths = product_app_paths(&state);
    require_multi_repo_project(&paths, &project_id)?;
    validate_project_id(&project_id)?;
    let _lease = state
        .aggregate_index_rebuilds
        .try_register(&project_id)
        .ok_or_else(|| {
            ApiError::runtime(
                "aggregate_index_rebuild_in_progress",
                "aggregate index rebuild is already in progress",
                serde_json::json!({}),
            )
        })?;
    let dependencies = state.aggregate_initialization_dependencies();
    let operation = dependencies.index.clone();
    let project_id_for_worker = project_id.clone();
    let result = tokio::task::spawn_blocking(move || operation.rebuild(&project_id_for_worker))
        .await
        .map_err(|error| {
            ApiError::runtime(
                "aggregate_index_unavailable",
                format!("aggregate index rebuild worker failed: {error}"),
                serde_json::json!({}),
            )
        })?;
    if let Err(error) = result {
        return Err(aggregate_index_api_error(error));
    }
    // Keep the lease until after the durable active projection is read. This
    // makes a same-project request observe either rebuilding or the new state,
    // never a transient gap between operation completion and response creation.
    let response = read_active_projection(&paths, &project_id)?;
    Ok((StatusCode::OK, Json(response)).into_response())
}

fn read_active_projection(
    paths: &crate::product::app_paths::ProductAppPaths,
    project_id: &str,
) -> ApiResult<AggregateIndexActiveResponse> {
    let store =
        crate::product::logical_codebase::aggregate_index::AggregateIndexStore::new(paths.clone());
    let mut records = store
        .records(project_id)
        .map_err(aggregate_index_api_error)?;
    records.retain(|record| record.status != AggregateIndexStatus::Superseded);
    records.sort_by(|left, right| {
        right
            .updated_at
            .cmp(&left.updated_at)
            .then_with(|| right.aggregate_index_id.cmp(&left.aggregate_index_id))
    });

    let latest = records.first().cloned();
    let response = match latest {
        None => missing_response(None),
        Some(record) if record.status == AggregateIndexStatus::Failed => {
            let good = records
                .iter()
                .find(|candidate| {
                    matches!(
                        candidate.status,
                        AggregateIndexStatus::Active
                            | AggregateIndexStatus::Stale
                            | AggregateIndexStatus::Degraded
                    )
                })
                .cloned();
            match good {
                None => missing_response(record.warning),
                Some(good) => projection("degraded", &good, record.warning),
            }
        }
        Some(record) => projection(
            match record.status {
                AggregateIndexStatus::Active => "active",
                AggregateIndexStatus::Stale => "stale",
                AggregateIndexStatus::Degraded => "degraded",
                AggregateIndexStatus::Building => "rebuilding",
                AggregateIndexStatus::Superseded | AggregateIndexStatus::Failed => "missing",
            },
            &record,
            None,
        ),
    };
    Ok(response)
}

fn projection(
    state: &'static str,
    record: &AggregateIndexRecord,
    warning: Option<String>,
) -> AggregateIndexActiveResponse {
    AggregateIndexActiveResponse {
        state,
        revision: Some(record.membership_revision),
        indexed_at: Some(record.updated_at.clone()),
        warning: warning.or_else(|| record.warning.clone()),
    }
}

fn missing_response(warning: Option<String>) -> AggregateIndexActiveResponse {
    AggregateIndexActiveResponse {
        state: "missing",
        revision: None,
        indexed_at: None,
        warning,
    }
}

fn validate_project_id(project_id: &str) -> ApiResult<()> {
    validate_relative_id(project_id).map_err(|error| {
        ApiError::validation("invalid_project_id", format!("invalid project id: {error}"))
    })?;
    Ok(())
}

fn aggregate_index_api_error(error: AggregateIndexError) -> ApiError {
    let code = match error {
        AggregateIndexError::Failed { code, .. } | AggregateIndexError::Degraded { code, .. } => {
            code
        }
    };
    ApiError::runtime(
        "aggregate_index_unavailable",
        error.to_string(),
        serde_json::json!({
            "reason_code": code,
        }),
    )
}
