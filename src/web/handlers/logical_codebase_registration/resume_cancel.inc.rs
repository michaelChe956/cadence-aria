/// Legacy `/logical-codebase/registrations/{batch_id}/resume` compatibility
/// alias for the default first logical codebase (v1.2 migration artifact).
pub async fn resume_logical_codebase_registration(
    State(state): State<WebAppState>,
    Path((project_id, batch_id)): Path<(String, String)>,
) -> ApiResult<Json<RegistrationBatchDto>> {
    let paths = product_app_paths(&state);
    let logical_codebase_id = default_logical_codebase_id(&paths, &project_id)?;
    resume_registration_batch_for_lc(state, project_id, logical_codebase_id, batch_id)
}

/// v1.3 canonical endpoint: resume is resolved per logical codebase.
pub async fn resume_lc_registration(
    State(state): State<WebAppState>,
    Path((project_id, logical_codebase_id, batch_id)): Path<(String, String, String)>,
) -> ApiResult<Json<RegistrationBatchDto>> {
    let paths = product_app_paths(&state);
    require_logical_codebase(&paths, &project_id, &logical_codebase_id)?;
    resume_registration_batch_for_lc(state, project_id, logical_codebase_id, batch_id)
}

fn resume_registration_batch_for_lc(
    state: WebAppState,
    project_id: String,
    logical_codebase_id: String,
    batch_id: String,
) -> ApiResult<Json<RegistrationBatchDto>> {
    let paths = product_app_paths(&state);
    validate_registration_ids(&project_id, &batch_id)?;
    let coordinator =
        LogicalCodebaseRegistrationCoordinator::for_lc(paths, logical_codebase_id);
    let batch = coordinator
        .resume_batch(&project_id, &batch_id)
        .map_err(registration_batch_api_error)?;
    Ok(Json(RegistrationBatchDto::from_record(&batch)))
}

/// Legacy `/logical-codebase/registrations/{batch_id}/cancel` compatibility
/// alias for the default first logical codebase (v1.2 migration artifact).
pub async fn cancel_logical_codebase_registration(
    State(state): State<WebAppState>,
    Path((project_id, batch_id)): Path<(String, String)>,
) -> ApiResult<Json<RegistrationBatchDto>> {
    let paths = product_app_paths(&state);
    let logical_codebase_id = default_logical_codebase_id(&paths, &project_id)?;
    cancel_registration_batch_for_lc(state, project_id, logical_codebase_id, batch_id)
}

/// v1.3 canonical endpoint: cancel is resolved per logical codebase.
pub async fn cancel_lc_registration(
    State(state): State<WebAppState>,
    Path((project_id, logical_codebase_id, batch_id)): Path<(String, String, String)>,
) -> ApiResult<Json<RegistrationBatchDto>> {
    let paths = product_app_paths(&state);
    require_logical_codebase(&paths, &project_id, &logical_codebase_id)?;
    cancel_registration_batch_for_lc(state, project_id, logical_codebase_id, batch_id)
}

fn cancel_registration_batch_for_lc(
    state: WebAppState,
    project_id: String,
    logical_codebase_id: String,
    batch_id: String,
) -> ApiResult<Json<RegistrationBatchDto>> {
    let paths = product_app_paths(&state);
    validate_registration_ids(&project_id, &batch_id)?;
    let coordinator =
        LogicalCodebaseRegistrationCoordinator::for_lc(paths, logical_codebase_id);
    let current = coordinator
        .get_batch(&project_id, &batch_id)
        .map_err(registration_batch_api_error)?;
    if !matches!(
        current.status,
        RegistrationBatchStatus::Queued | RegistrationBatchStatus::PartialFailed
    ) {
        return Err(ApiError::runtime(
            "registration_batch_not_cancelable",
            "registration batch is not cancelable in its current state",
            serde_json::json!({ "status": current.status }),
        ));
    }
    let batch = coordinator
        .cancel_batch(&project_id, &batch_id)
        .map_err(registration_batch_api_error)?;
    if batch.status != RegistrationBatchStatus::Cancelled {
        return Err(ApiError::runtime(
            "registration_batch_not_cancelable",
            "registration batch is not cancelable in its current state",
            serde_json::json!({ "status": batch.status }),
        ));
    }
    Ok(Json(RegistrationBatchDto::from_record(&batch)))
}
