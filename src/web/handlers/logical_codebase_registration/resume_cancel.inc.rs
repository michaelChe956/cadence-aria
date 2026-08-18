pub async fn resume_logical_codebase_registration(
    State(state): State<WebAppState>,
    Path((project_id, batch_id)): Path<(String, String)>,
) -> ApiResult<Json<RegistrationBatchDto>> {
    let paths = product_app_paths(&state);
    require_multi_repo_project(&paths, &project_id)?;
    validate_registration_ids(&project_id, &batch_id)?;
    let coordinator = registration_coordinator(&paths);
    let batch = coordinator
        .resume_batch(&project_id, &batch_id)
        .map_err(registration_batch_api_error)?;
    Ok(Json(RegistrationBatchDto::from_record(&batch)))
}

pub async fn cancel_logical_codebase_registration(
    State(state): State<WebAppState>,
    Path((project_id, batch_id)): Path<(String, String)>,
) -> ApiResult<Json<RegistrationBatchDto>> {
    let paths = product_app_paths(&state);
    require_multi_repo_project(&paths, &project_id)?;
    validate_registration_ids(&project_id, &batch_id)?;
    let coordinator = registration_coordinator(&paths);
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
