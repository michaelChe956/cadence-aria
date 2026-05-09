use axum::Json;
use axum::extract::{Path, Query, State};
use axum::response::sse::{Event, KeepAlive, Sse};
use futures_util::stream::{self, Stream, StreamExt};
use serde::Deserialize;
use serde_json::json;
use std::convert::Infallible;
use tokio_stream::wrappers::BroadcastStream;

use crate::interactive::models::WebWorkspaceProjection;
use crate::web::error::ApiResult;
use crate::web::events::WebEventType;
use crate::web::state::WebAppState;
use crate::web::types::{
    AdvanceTaskResponse, ConfirmTaskRequest, ConfirmTaskResponse, CreateTaskRequest,
    CreateTaskResponse, WebEvent,
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
    let response = runtime.create_task(request)?;
    state.events.publish(
        WebEventType::ProjectionUpdated.as_str(),
        Some(&response.task_id),
        json!({"phase": response.phase}),
    );
    Ok(Json(response))
}

pub async fn advance_task(
    State(state): State<WebAppState>,
    Path(task_id): Path<String>,
) -> ApiResult<Json<AdvanceTaskResponse>> {
    let mut runtime = state.runtime.lock().expect("runtime lock");
    let response = runtime.advance_task(&task_id)?;
    if let AdvanceTaskResponse::PausedForApproval { pending_step } = &response {
        state.events.publish(
            WebEventType::CheckpointCreated.as_str(),
            Some(&task_id),
            json!({"checkpoint_id": pending_step.checkpoint_id}),
        );
        state.events.publish(
            WebEventType::PausedForApproval.as_str(),
            Some(&task_id),
            json!({"node_id": pending_step.node_id}),
        );
    }
    state.events.publish(
        WebEventType::ProjectionUpdated.as_str(),
        Some(&task_id),
        json!({}),
    );
    Ok(Json(response))
}

pub async fn confirm_task(
    State(state): State<WebAppState>,
    Path(task_id): Path<String>,
    Json(request): Json<ConfirmTaskRequest>,
) -> ApiResult<Json<ConfirmTaskResponse>> {
    let mut runtime = state.runtime.lock().expect("runtime lock");
    let response = runtime.confirm_task(&task_id, request)?;
    state.events.publish(
        WebEventType::NodeStarted.as_str(),
        Some(&task_id),
        json!({"node_id": response.node_id}),
    );
    state.events.publish(
        WebEventType::ProviderOutput.as_str(),
        Some(&task_id),
        json!({"node_id": response.node_id, "stream": "stdout"}),
    );
    state.events.publish(
        WebEventType::ArtifactWritten.as_str(),
        Some(&task_id),
        json!({"node_id": response.node_id, "artifact_ref": "coding_report_work_wt_001_0001"}),
    );
    state.events.publish(
        WebEventType::NodeCompleted.as_str(),
        Some(&task_id),
        json!({"node_id": response.node_id}),
    );
    state.events.publish(
        WebEventType::ProjectionUpdated.as_str(),
        Some(&task_id),
        json!({}),
    );
    Ok(Json(response))
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

pub async fn events(
    State(state): State<WebAppState>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let replay_stream = stream::iter(state.events.replay_after(0));
    let live_stream = BroadcastStream::new(state.events.subscribe())
        .filter_map(|event| async move { event.ok() });
    let sse_stream = replay_stream
        .chain(live_stream)
        .map(|event| Ok::<Event, Infallible>(sse_event(event)));
    Sse::new(sse_stream).keep_alive(KeepAlive::default())
}

fn sse_event(event: WebEvent) -> Event {
    Event::default()
        .id(event.cursor.to_string())
        .event(event.event_type.clone())
        .json_data(event)
        .expect("serialize web event")
}
