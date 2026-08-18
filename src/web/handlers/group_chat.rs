use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::cross_cutting::aria_state_paths::AriaStatePaths;
use crate::cross_cutting::streaming_provider::ProviderPermissionMode;
use crate::product::group_chat_engine::GroupChatEngine;
use crate::product::group_chat_engine::finalize::FinalizeInput;
use crate::product::group_chat_engine::settings::{
    SpecGenerationMode, load_spec_generation_mode, save_spec_generation_mode,
};
use crate::product::group_chat_engine::types::{
    ArtifactLineKind, DraftSlotKey, GroupChatRoleKey, GroupChatSessionRecord, RoomEvent,
};
use crate::product::models::ProviderName;
use crate::web::error::{ApiError, ApiResult};
use crate::web::handlers::support::product_store_api_error;
use crate::web::state::WebAppState;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct SendMessageRequest {
    pub text: String,
    #[serde(default)]
    pub mentions: Vec<String>,
    #[serde(default)]
    pub draft_slot: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct AddRoleRequest {
    pub role_key: GroupChatRoleKey,
    pub provider: ProviderName,
    pub display_name: Option<String>,
    pub permission_mode: Option<ProviderPermissionMode>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct FinalizeRequest {
    pub line_kind: ArtifactLineKind,
    pub included_slots_override: Option<Vec<DraftSlotKey>>,
    pub confirmed_by: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct TriageProviderRequest {
    pub provider: Option<ProviderName>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct TriageProviderResponse {
    pub provider: Option<ProviderName>,
}

#[derive(Debug, Deserialize)]
pub struct TimelineQuery {
    pub after_seq: Option<u64>,
    pub limit: Option<usize>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct SessionResponse {
    #[serde(flatten)]
    pub session: GroupChatSessionRecord,
    pub timeline: Vec<TimelineEventResponse>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct TimelineEventResponse {
    pub seq: u64,
    pub event: RoomEvent,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct MessageResponse {
    pub summary: crate::product::group_chat_engine::coordinator::CoordinatorRunSummary,
    pub session: GroupChatSessionRecord,
}

fn engine(state: &WebAppState) -> ApiResult<Arc<GroupChatEngine>> {
    state.group_chat_engine.clone().ok_or_else(|| {
        ApiError::runtime(
            "group_chat_unavailable",
            "group chat engine is unavailable",
            json!({}),
        )
    })
}

fn session_not_found() -> ApiError {
    ApiError::runtime(
        "group_chat_session_not_found",
        "group chat session not found",
        json!({}),
    )
}

fn load_session_for_id(
    engine: &GroupChatEngine,
    session_id: &str,
) -> ApiResult<GroupChatSessionRecord> {
    let session = engine
        .store
        .find_session_by_id(session_id)
        .map_err(product_store_api_error)?
        .ok_or_else(session_not_found)?;
    engine
        .load_session(&session.project_id, &session.issue_id, &session.id)
        .map_err(product_store_api_error)
}

pub async fn create_session(
    State(state): State<WebAppState>,
    Json(request): Json<CreateSessionRequest>,
) -> ApiResult<(StatusCode, Json<SessionResponse>)> {
    let engine = engine(&state)?;
    let (session, created) = engine
        .create_or_get_session(&request.project_id, &request.issue_id)
        .map_err(product_store_api_error)?;
    let response = session_response(&engine, &session, None)?;
    let status = if created {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    Ok((status, Json(response)))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CreateSessionRequest {
    pub project_id: String,
    pub issue_id: String,
}

pub async fn get_session(
    State(state): State<WebAppState>,
    Path(id): Path<String>,
    Query(query): Query<TimelineQuery>,
) -> ApiResult<Json<SessionResponse>> {
    let engine = engine(&state)?;
    let session = load_session_for_id(&engine, &id)?;
    Ok(Json(session_response(&engine, &session, Some(query))?))
}

pub async fn send_message(
    State(state): State<WebAppState>,
    Path(id): Path<String>,
    Json(request): Json<SendMessageRequest>,
) -> ApiResult<Json<MessageResponse>> {
    if request.text.trim().is_empty() {
        return Err(ApiError::validation(
            "invalid_group_chat_message",
            "text is required",
        ));
    }
    let engine = engine(&state)?;
    let session = load_session_for_id(&engine, &id)?;
    let draft_slot = request.draft_slot.map(DraftSlotKey);
    let summary = engine
        .on_user_message(
            &session.project_id,
            &session.issue_id,
            &session.id,
            &request.text,
            request.mentions,
            draft_slot,
        )
        .await
        .map_err(group_chat_message_api_error)?;
    let session = engine
        .load_session(&session.project_id, &session.issue_id, &session.id)
        .map_err(product_store_api_error)?;
    Ok(Json(MessageResponse { summary, session }))
}

pub async fn add_role(
    State(state): State<WebAppState>,
    Path(id): Path<String>,
    Json(request): Json<AddRoleRequest>,
) -> ApiResult<Json<GroupChatSessionRecord>> {
    let engine = engine(&state)?;
    let session = load_session_for_id(&engine, &id)?;
    let updated = engine
        .add_role(
            &session.project_id,
            &session.issue_id,
            &session.id,
            request.role_key,
            request.provider,
            request.display_name,
            request.permission_mode,
        )
        .map_err(group_chat_role_api_error)?;
    Ok(Json(updated))
}

pub async fn finalize(
    State(state): State<WebAppState>,
    Path(id): Path<String>,
    Json(request): Json<FinalizeRequest>,
) -> ApiResult<Json<FinalizeResponse>> {
    let engine = engine(&state)?;
    let session = load_session_for_id(&engine, &id)?;
    let entries = engine
        .store
        .load_event_entries(&session.project_id, &session.issue_id, &session.id)
        .map_err(product_store_api_error)?;
    let (provider_run_refs, review_refs) =
        collect_refs_for_ws(&entries, &session, request.line_kind);
    let event = engine
        .finalize_line(FinalizeInput {
            project_id: session.project_id.clone(),
            issue_id: session.issue_id.clone(),
            session_id: session.id.clone(),
            line_kind: request.line_kind,
            included_slots_override: request.included_slots_override,
            confirmed_by: request.confirmed_by,
            provider_run_refs,
            review_refs,
        })
        .map_err(finalize_api_error)?;
    let session = engine
        .load_session(&session.project_id, &session.issue_id, &session.id)
        .map_err(product_store_api_error)?;
    Ok(Json(FinalizeResponse { event, session }))
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct FinalizeResponse {
    pub event: RoomEvent,
    pub session: GroupChatSessionRecord,
}

pub async fn get_triage_provider(
    State(state): State<WebAppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<TriageProviderResponse>> {
    let engine = engine(&state)?;
    let session = load_session_for_id(&engine, &id)?;
    Ok(Json(TriageProviderResponse {
        provider: session.triage_provider,
    }))
}

pub async fn update_triage_provider(
    State(state): State<WebAppState>,
    Path(id): Path<String>,
    Json(request): Json<TriageProviderRequest>,
) -> ApiResult<Json<TriageProviderResponse>> {
    let engine = engine(&state)?;
    let mut session = load_session_for_id(&engine, &id)?;
    if let Some(provider) = request.provider.as_ref()
        && engine.providers.get(provider).is_none()
    {
        return Err(ApiError::validation(
            "invalid_provider",
            "provider adapter is unavailable",
        ));
    }
    session.triage_provider = request.provider;
    session.updated_at = chrono::Utc::now().to_rfc3339();
    engine
        .store
        .save_session_snapshot(&session)
        .map_err(product_store_api_error)?;
    Ok(Json(TriageProviderResponse {
        provider: session.triage_provider,
    }))
}

pub async fn get_spec_generation_mode(
    State(state): State<WebAppState>,
) -> ApiResult<Json<SpecGenerationMode>> {
    Ok(Json(load_spec_generation_mode(
        &AriaStatePaths::from_workspace_root(&state.workspace_root),
    )))
}

pub async fn update_spec_generation_mode(
    State(state): State<WebAppState>,
    Json(mode): Json<SpecGenerationMode>,
) -> ApiResult<Json<SpecGenerationMode>> {
    let paths = AriaStatePaths::from_workspace_root(&state.workspace_root);
    save_spec_generation_mode(&paths, &mode).map_err(product_store_api_error)?;
    Ok(Json(mode))
}

fn session_response(
    engine: &GroupChatEngine,
    session: &GroupChatSessionRecord,
    query: Option<TimelineQuery>,
) -> ApiResult<SessionResponse> {
    let entries = engine
        .store
        .load_event_entries(&session.project_id, &session.issue_id, &session.id)
        .map_err(product_store_api_error)?;
    let after_seq = query
        .as_ref()
        .and_then(|query| query.after_seq)
        .unwrap_or(0);
    let limit = query
        .as_ref()
        .and_then(|query| query.limit)
        .unwrap_or(100)
        .min(500);
    let timeline = entries
        .into_iter()
        .filter(|(seq, _)| *seq > after_seq)
        .take(limit)
        .map(|(seq, event)| TimelineEventResponse { seq, event })
        .collect();
    Ok(SessionResponse {
        session: session.clone(),
        timeline,
    })
}

pub(crate) fn collect_refs_for_ws(
    entries: &[(u64, RoomEvent)],
    session: &GroupChatSessionRecord,
    line_kind: ArtifactLineKind,
) -> (Vec<String>, Vec<String>) {
    let reviewer_ids = session
        .roles
        .iter()
        .filter(|role| role.role_key == GroupChatRoleKey::Reviewer)
        .map(|role| role.id.as_str())
        .collect::<std::collections::HashSet<_>>();
    let provider_run_refs = entries
        .iter()
        .filter_map(|(seq, event)| match event {
            RoomEvent::AgentMessage {
                artifact_ref: Some(artifact_ref),
                ..
            } if artifact_ref.line == line_kind => Some(seq.to_string()),
            _ => None,
        })
        .collect();
    let review_refs = entries
        .iter()
        .filter_map(|(seq, event)| match event {
            RoomEvent::AgentMessage {
                role_instance_id, ..
            } if reviewer_ids.contains(role_instance_id.as_str()) => Some(seq.to_string()),
            _ => None,
        })
        .collect();
    (provider_run_refs, review_refs)
}

fn group_chat_message_api_error(
    error: crate::product::group_chat_engine::coordinator::CoordinatorError,
) -> ApiError {
    match error {
        crate::product::group_chat_engine::coordinator::CoordinatorError::Store(
            crate::product::json_store::ProductStoreError::InvalidRecord {
                kind: "group_chat_draft_slot",
                reason,
            },
        ) => ApiError::validation("invalid_group_chat_draft_slot", reason),
        crate::product::group_chat_engine::coordinator::CoordinatorError::Store(
            crate::product::json_store::ProductStoreError::Conflict {
                kind: "group_chat_draft_slot",
                id,
            },
        ) => ApiError::runtime("group_chat_draft_slot_claimed", id, json!({})),
        other => ApiError::runtime("group_chat_message_failed", other.to_string(), json!({})),
    }
}

fn finalize_api_error(
    error: crate::product::group_chat_engine::finalize::FinalizeError,
) -> ApiError {
    let message = error.to_string();
    let code = if message.contains("story_spec_not_confirmed") {
        "story_spec_not_confirmed"
    } else if message.contains("缺少可定稿草稿槽") {
        "group_chat_no_draft"
    } else if message.contains("产物线不存在") {
        "group_chat_line_not_found"
    } else if message.contains("草稿槽不存在") {
        "group_chat_slot_not_found"
    } else {
        "group_chat_finalize_failed"
    };
    ApiError::runtime(code, message, json!({}))
}

fn group_chat_role_api_error(error: crate::product::json_store::ProductStoreError) -> ApiError {
    match error {
        crate::product::json_store::ProductStoreError::InvalidRecord {
            kind: "group_chat_role",
            reason,
        } => ApiError::validation("invalid_provider", reason),
        other => product_store_api_error(other),
    }
}
