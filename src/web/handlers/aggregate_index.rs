//! HTTP projection and synchronous rebuild endpoints for aggregate indexes.

use super::support::{default_logical_codebase_id, product_app_paths, require_logical_codebase};
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
    let logical_codebase_id = default_logical_codebase_id(&paths, &project_id)?;
    get_active_aggregate_index_for_lc(&state, &project_id, &logical_codebase_id)
}

/// v1.3 canonical endpoint: the active projection is resolved per logical
/// codebase.
pub async fn get_lc_active_aggregate_index(
    State(state): State<WebAppState>,
    Path((project_id, logical_codebase_id)): Path<(String, String)>,
) -> ApiResult<Response> {
    let paths = product_app_paths(&state);
    require_logical_codebase(&paths, &project_id, &logical_codebase_id)?;
    get_active_aggregate_index_for_lc(&state, &project_id, &logical_codebase_id)
}

fn get_active_aggregate_index_for_lc(
    state: &WebAppState,
    project_id: &str,
    logical_codebase_id: &str,
) -> ApiResult<Response> {
    let paths = product_app_paths(state);
    validate_project_id(project_id)?;
    let response = read_active_projection(&paths, project_id, logical_codebase_id)?;
    Ok((StatusCode::OK, Json(response)).into_response())
}

pub async fn rebuild_aggregate_index(
    State(state): State<WebAppState>,
    Path(project_id): Path<String>,
) -> ApiResult<Response> {
    let paths = product_app_paths(&state);
    let logical_codebase_id = default_logical_codebase_id(&paths, &project_id)?;
    rebuild_aggregate_index_for_lc(state, project_id, logical_codebase_id).await
}

/// v1.3 canonical endpoint: the rebuild is resolved per logical codebase.
pub async fn rebuild_lc_aggregate_index(
    State(state): State<WebAppState>,
    Path((project_id, logical_codebase_id)): Path<(String, String)>,
) -> ApiResult<Response> {
    let paths = product_app_paths(&state);
    require_logical_codebase(&paths, &project_id, &logical_codebase_id)?;
    rebuild_aggregate_index_for_lc(state, project_id, logical_codebase_id).await
}

async fn rebuild_aggregate_index_for_lc(
    state: WebAppState,
    project_id: String,
    logical_codebase_id: String,
) -> ApiResult<Response> {
    let paths = product_app_paths(&state);
    validate_project_id(&project_id)?;
    let rebuild_key = format!("{project_id}/{logical_codebase_id}");
    let _lease = state
        .aggregate_index_rebuilds
        .try_register(&rebuild_key)
        .ok_or_else(|| {
            ApiError::runtime(
                "aggregate_index_rebuild_in_progress",
                "aggregate index rebuild is already in progress",
                serde_json::json!({}),
            )
        })?;
    let dependencies = state
        .aggregate_initialization_dependencies()
        .for_lc(logical_codebase_id.clone());
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
    // makes a same-codebase request observe either rebuilding or the new state,
    // never a transient gap between operation completion and response creation.
    let response = read_active_projection(&paths, &project_id, &logical_codebase_id)?;
    Ok((StatusCode::OK, Json(response)).into_response())
}

fn read_active_projection(
    paths: &crate::product::app_paths::ProductAppPaths,
    project_id: &str,
    logical_codebase_id: &str,
) -> ApiResult<AggregateIndexActiveResponse> {
    let store = crate::product::logical_codebase::aggregate_index::AggregateIndexStore::for_lc(
        paths.clone(),
        logical_codebase_id,
    );
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
    // degraded LKG 优先于 active/stale 投影：只要存在一个 degraded generation，
    // 最近一次刷新已失败（成功 rebuild 会把 degraded 前代翻成 superseded）。
    let degraded_record = records
        .iter()
        .find(|candidate| candidate.status == AggregateIndexStatus::Degraded)
        .cloned();
    let response = match latest {
        None => missing_response(None),
        Some(record) if record.status == AggregateIndexStatus::Building => {
            projection("rebuilding", &record, None)
        }
        _ if degraded_record.is_some() => {
            projection("degraded", &degraded_record.expect("checked above"), None)
        }
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

#[cfg(test)]
mod projection_tests {
    use super::*;
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::logical_codebase::aggregate_index::AggregateIndexStore;

    fn record(
        aggregate_index_id: &str,
        status: AggregateIndexStatus,
        updated_at: &str,
    ) -> AggregateIndexRecord {
        let mut record = AggregateIndexRecord::building(
            aggregate_index_id.to_string(),
            "project_0001".to_string(),
            3,
            Vec::new(),
            updated_at.to_string(),
        );
        record.status = status;
        record
    }

    #[test]
    fn read_active_projection_reports_missing_when_no_generation_exists() {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let response = read_active_projection(&paths, "project_0001", "logical_codebase_0001")
            .unwrap();
        assert_eq!(response.state, "missing");
        assert_eq!(response.revision, None);
    }

    #[test]
    fn read_active_projection_maps_first_build_failure_to_missing_with_warning() {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let store = AggregateIndexStore::for_lc(paths.clone(), "logical_codebase_0001");
        store
            .create(
                "project_0001",
                record(
                    "aggregate_index_failed_gen",
                    AggregateIndexStatus::Failed,
                    "2026-09-28T01:00:00Z",
                ),
            )
            .unwrap();
        store
            .mark_status(
                "project_0001",
                "aggregate_index_failed_gen",
                AggregateIndexStatus::Failed,
                Some("aggregate_index_failed:codegraph_init_failed: cli exploded".to_string()),
            )
            .unwrap();

        let response = read_active_projection(&paths, "project_0001", "logical_codebase_0001")
            .unwrap();
        // 无 LKG：首建失败投影 missing，但保留可操作 warning。
        assert_eq!(response.state, "missing");
        assert!(response
            .warning
            .as_deref()
            .unwrap_or_default()
            .contains("codegraph_init_failed"));
    }

    #[test]
    fn read_active_projection_maps_rebuild_failure_to_degraded_last_known_good() {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let store = AggregateIndexStore::for_lc(paths.clone(), "logical_codebase_0001");
        store
            .create(
                "project_0001",
                record(
                    "aggregate_index_lkg",
                    AggregateIndexStatus::Active,
                    "2026-09-28T01:00:00Z",
                ),
            )
            .unwrap();
        // rebuild 失败现场：LKG 被 degrade，新 generation 以 Stale + warning 落盘。
        store
            .mark_status(
                "project_0001",
                "aggregate_index_lkg",
                AggregateIndexStatus::Degraded,
                Some("aggregate_index_failed:codegraph_init_failed: cli exploded".to_string()),
            )
            .unwrap();
        store
            .create(
                "project_0001",
                record(
                    "aggregate_index_refresh_gen",
                    AggregateIndexStatus::Stale,
                    "2026-09-28T02:00:00Z",
                ),
            )
            .unwrap();
        store
            .mark_status(
                "project_0001",
                "aggregate_index_refresh_gen",
                AggregateIndexStatus::Stale,
                Some("aggregate_index_failed:codegraph_init_failed: cli exploded".to_string()),
            )
            .unwrap();

        let response = read_active_projection(&paths, "project_0001", "logical_codebase_0001")
            .unwrap();
        assert_eq!(response.state, "degraded");
        assert_eq!(response.revision, Some(3));
        assert!(response
            .warning
            .as_deref()
            .unwrap_or_default()
            .contains("codegraph_init_failed"));
    }

    #[test]
    fn read_active_projection_reports_rebuilding_while_generation_is_building() {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let store = AggregateIndexStore::for_lc(paths.clone(), "logical_codebase_0001");
        store
            .create(
                "project_0001",
                record(
                    "aggregate_index_building_gen",
                    AggregateIndexStatus::Building,
                    "2026-09-28T01:00:00Z",
                ),
            )
            .unwrap();

        let response = read_active_projection(&paths, "project_0001", "logical_codebase_0001")
            .unwrap();
        assert_eq!(response.state, "rebuilding");
    }
}
