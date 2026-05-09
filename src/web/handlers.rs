use axum::Json;
use axum::extract::{Path, Query, State};
use serde::Deserialize;
use serde_json::json;

use crate::interactive::models::WebWorkspaceProjection;
use crate::web::error::ApiResult;
use crate::web::state::WebAppState;
use crate::web::types::{
    AdvanceTaskResponse, ConfirmTaskRequest, ConfirmTaskResponse, CreateTaskRequest,
    CreateTaskResponse,
};

#[derive(Debug, Deserialize)]
pub struct ProjectionQuery {
    pub task_id: Option<String>,
    pub node_id: Option<String>,
}

pub async fn health() -> Json<serde_json::Value> {
    Json(json!({"status":"ok"}))
}

pub async fn create_task(
    State(state): State<WebAppState>,
    Json(request): Json<CreateTaskRequest>,
) -> ApiResult<Json<CreateTaskResponse>> {
    let mut runtime = state.runtime.lock().expect("runtime lock");
    Ok(Json(runtime.create_task(request)?))
}

pub async fn advance_task(
    State(state): State<WebAppState>,
    Path(task_id): Path<String>,
) -> ApiResult<Json<AdvanceTaskResponse>> {
    let mut runtime = state.runtime.lock().expect("runtime lock");
    Ok(Json(runtime.advance_task(&task_id)?))
}

pub async fn confirm_task(
    State(state): State<WebAppState>,
    Path(task_id): Path<String>,
    Json(request): Json<ConfirmTaskRequest>,
) -> ApiResult<Json<ConfirmTaskResponse>> {
    let mut runtime = state.runtime.lock().expect("runtime lock");
    Ok(Json(runtime.confirm_task(&task_id, request)?))
}

pub async fn projection(
    State(state): State<WebAppState>,
    Query(query): Query<ProjectionQuery>,
) -> ApiResult<Json<WebWorkspaceProjection>> {
    let runtime = state.runtime.lock().expect("runtime lock");
    Ok(Json(runtime.projection(
        query.task_id.as_deref(),
        query.node_id.as_deref(),
    )?))
}
