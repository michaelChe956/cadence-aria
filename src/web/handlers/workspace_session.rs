use super::dto::*;
use super::lifecycle::confirm_workspace_entity;
use super::support::*;
use super::*;

pub async fn workspace_session_message(
    State(state): State<WebAppState>,
    Path(session_id): Path<String>,
    Json(request): Json<WorkspaceSessionMessageRequest>,
) -> ApiResult<Json<WorkspaceSessionDto>> {
    validate_workspace_message(&request)?;
    let session = LifecycleStore::new(product_app_paths(&state))
        .append_workspace_message(&session_id, request.role, request.content)
        .map_err(product_store_api_error)?;
    Ok(Json(workspace_session_dto(session)))
}

pub async fn workspace_session_run_next(
    State(state): State<WebAppState>,
    Path(session_id): Path<String>,
    Json(request): Json<WorkspaceSessionRunNextRequest>,
) -> ApiResult<Json<WorkspaceSessionDto>> {
    let paths = product_app_paths(&state);
    let lifecycle = LifecycleStore::new(paths.clone());
    lifecycle
        .get_workspace_session(&session_id)
        .map_err(product_store_api_error)?;
    let user_prompt = workspace_user_prompt(request.user_prompt);
    lifecycle
        .append_workspace_message(&session_id, "user".to_string(), user_prompt.clone())
        .map_err(product_store_api_error)?;

    let runner = ProviderWorkspaceRunner::new(paths);
    let output = runner
        .run_next(
            WorkspaceProviderRunInput {
                session_id,
                user_prompt: provider_workspace_prompt(user_prompt),
            },
            &FakeProviderAdapter,
        )
        .map_err(|error| {
            ApiError::runtime(
                "provider_workspace_run_failed",
                "provider workspace run failed",
                json!({"details": error.details}),
            )
        })?;
    Ok(Json(workspace_session_dto(output.session)))
}

pub async fn workspace_session_confirm(
    State(state): State<WebAppState>,
    Path(session_id): Path<String>,
    Json(request): Json<WorkspaceSessionConfirmRequest>,
) -> ApiResult<Json<WorkspaceSessionDto>> {
    let lifecycle = LifecycleStore::new(product_app_paths(&state));
    let session = lifecycle
        .update_workspace_session_status(&session_id, WorkspaceSessionStatus::Confirmed)
        .map_err(product_store_api_error)?;
    confirm_workspace_entity(&lifecycle, &session)?;
    let session = lifecycle
        .append_workspace_message(
            &session_id,
            "system".to_string(),
            format!("已由 {} 确认当前 Workspace 产物。", request.confirmed_by),
        )
        .map_err(product_store_api_error)?;
    Ok(Json(workspace_session_dto(session)))
}

pub async fn workspace_session_timeline_node_detail(
    State(state): State<WebAppState>,
    Path((session_id, node_id)): Path<(String, String)>,
) -> ApiResult<Json<NodeDetail>> {
    let detail = LifecycleStore::new(product_app_paths(&state))
        .load_node_detail(&session_id, &node_id)
        .map_err(node_detail_store_api_error)?;
    Ok(Json(detail))
}

pub async fn workspace_session_timeline_node_prompt(
    State(state): State<WebAppState>,
    Path((session_id, node_id)): Path<(String, String)>,
) -> ApiResult<Json<serde_json::Value>> {
    let detail = LifecycleStore::new(product_app_paths(&state))
        .load_node_detail(&session_id, &node_id)
        .map_err(node_detail_store_api_error)?;
    let prompt = detail.prompt.ok_or_else(|| {
        ApiError::runtime(
            "node_detail_prompt_not_found",
            "node detail prompt not found",
            json!({}),
        )
    })?;
    Ok(Json(json!({"node_id": node_id, "prompt": prompt})))
}

pub async fn workspace_session_timeline_event_output(
    State(state): State<WebAppState>,
    Path((session_id, node_id, event_id)): Path<(String, String, String)>,
) -> ApiResult<Json<serde_json::Value>> {
    let detail = LifecycleStore::new(product_app_paths(&state))
        .load_node_detail(&session_id, &node_id)
        .map_err(node_detail_store_api_error)?;
    let output = detail
        .execution_events
        .iter()
        .find(|event| event.get("event_id").and_then(|value| value.as_str()) == Some(&event_id))
        .and_then(|event| event.get("output").and_then(|value| value.as_str()))
        .ok_or_else(|| {
            ApiError::runtime(
                "event_output_not_found",
                "timeline event output not found",
                json!({}),
            )
        })?;
    Ok(Json(
        json!({"node_id": node_id, "event_id": event_id, "output": output}),
    ))
}

pub async fn workspace_session_artifact_version(
    State(state): State<WebAppState>,
    Path((session_id, version)): Path<(String, u32)>,
) -> ApiResult<Json<serde_json::Value>> {
    let version = LifecycleStore::new(product_app_paths(&state))
        .list_artifact_versions(&session_id)
        .map_err(product_store_api_error)?
        .into_iter()
        .find(|artifact| artifact.version == version)
        .ok_or_else(|| {
            ApiError::runtime(
                "artifact_version_not_found",
                "artifact version not found",
                json!({}),
            )
        })?;
    Ok(Json(
        json!({"version": version.version, "markdown": version.markdown()}),
    ))
}

pub(crate) fn validate_workspace_message(
    request: &WorkspaceSessionMessageRequest,
) -> ApiResult<()> {
    if !matches!(
        request.role.as_str(),
        "user" | "assistant" | "system" | "provider" | "reviewer"
    ) || request.content.trim().is_empty()
    {
        return Err(ApiError::validation(
            "invalid_workspace_message",
            "workspace message role/content is invalid",
        ));
    }
    Ok(())
}

pub(crate) fn workspace_user_prompt(user_prompt: Option<String>) -> String {
    user_prompt
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "请基于当前 Issue 上下文生成或修订 Workspace 产物。".to_string())
}

pub(crate) fn provider_workspace_prompt(prompt: String) -> String {
    let structured = json!({
        "markdown": format!("# Provider Workspace\n\n{prompt}"),
        "review_result": "review completed",
        "revision_result": "revision completed"
    });
    format!("{prompt}\n\n{STRUCTURED_OUTPUT_START}\n{structured}\n{STRUCTURED_OUTPUT_END}")
}
