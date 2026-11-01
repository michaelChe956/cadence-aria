/// Legacy `/logical-codebase/registrations/{batch_id}` compatibility alias for
/// the default first logical codebase (v1.2 migration artifact).
pub async fn get_logical_codebase_registration_batch(
    State(state): State<WebAppState>,
    Path((project_id, batch_id)): Path<(String, String)>,
) -> ApiResult<Json<RegistrationBatchDto>> {
    let paths = product_app_paths(&state);
    let logical_codebase_id = default_logical_codebase_id(&paths, &project_id)?;
    get_registration_batch_for_lc(state, project_id, logical_codebase_id, batch_id)
}

/// v1.3 canonical endpoint: batches are resolved per logical codebase.
pub async fn get_lc_registration_batch(
    State(state): State<WebAppState>,
    Path((project_id, logical_codebase_id, batch_id)): Path<(String, String, String)>,
) -> ApiResult<Json<RegistrationBatchDto>> {
    let paths = product_app_paths(&state);
    require_logical_codebase(&paths, &project_id, &logical_codebase_id)?;
    get_registration_batch_for_lc(state, project_id, logical_codebase_id, batch_id)
}

fn get_registration_batch_for_lc(
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
        .get_batch(&project_id, &batch_id)
        .map_err(registration_batch_api_error)?;
    Ok(Json(RegistrationBatchDto::from_record(&batch)))
}

fn validate_registration_ids(project_id: &str, batch_id: &str) -> ApiResult<()> {
    crate::product::json_store::validate_relative_id(project_id).map_err(|error| {
        ApiError::validation("invalid_project_id", format!("invalid project id: {error}"))
    })?;
    crate::product::json_store::validate_relative_id(batch_id).map_err(|error| {
        ApiError::validation(
            "invalid_registration_batch_id",
            format!("invalid batch id: {error}"),
        )
    })
}

fn registration_batch_api_error(
    error: crate::product::json_store::ProductStoreError,
) -> ApiError {
    match error {
        crate::product::json_store::ProductStoreError::NotFound {
            kind: "registration_batch",
            ..
        } => ApiError::runtime(
            "registration_batch_not_found",
            "registration batch not found",
            serde_json::json!({}),
        ),
        other => product_store_api_error(other),
    }
}
