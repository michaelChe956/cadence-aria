pub async fn get_logical_codebase_registration_batch(
    State(state): State<WebAppState>,
    Path((project_id, batch_id)): Path<(String, String)>,
) -> ApiResult<Json<RegistrationBatchDto>> {
    let paths = product_app_paths(&state);
    require_multi_repo_project(&paths, &project_id)?;
    validate_registration_ids(&project_id, &batch_id)?;
    let coordinator = registration_coordinator(&paths);
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
        ApiError::validation("invalid_registration_batch_id", format!("invalid batch id: {error}"))
    })
}

fn registration_coordinator(
    paths: &crate::product::app_paths::ProductAppPaths,
) -> LogicalCodebaseRegistrationCoordinator {
    LogicalCodebaseRegistrationCoordinator::new(
        paths.clone(),
        RepositoryStore::with_logical_codebase_feature(
            paths.clone(),
            LogicalCodebaseFeature::enabled(),
        ),
        LogicalCodebaseFeature::enabled(),
    )
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
