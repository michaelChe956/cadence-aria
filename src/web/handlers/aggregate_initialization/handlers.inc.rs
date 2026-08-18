pub async fn create_aggregate_initialization(
    State(state): State<WebAppState>,
    Path(project_id): Path<String>,
    Json(request): Json<CreateAggregateInitializationRequest>,
) -> ApiResult<Response> {
    let project_paths = product_app_paths(&state);
    require_multi_repo_project(&project_paths, &project_id)?;
    validate_project_id(&project_id)?;
    validate_idempotency_key(&request.idempotency_key)?;
    let dependencies = aggregate_initialization_dependencies(&state);
    let operation_id = deterministic_operation_id(&project_id, &request.idempotency_key);
    let manifest = load_manifest_for_profile(&project_paths, &project_id)?;
    let input = crate::product::logical_codebase::AggregateInitializationOperationInput {
        idempotency_key: request.idempotency_key.clone(),
        manifest_revision: manifest.membership_revision,
        policy_digest: "sha256:aggregate-policy".to_string(),
        profile_evidence_digest: Some("sha256:profile".to_string()),
        provider_context_root: manifest.provider_context_root.clone(),
        provider: "claude_code".to_string(),
    };
    let operation = dependencies
        .coordinator
        .begin(operation_id, &project_id, input)
        .map_err(aggregate_initialization_api_error)?;
    // `begin` returns only a newly-created operation: an existing terminal
    // operation with the same key is reported as an idempotency conflict.
    let key = InitializationRunKey::aggregate(&project_id, &operation.operation_id);
    let lease = dependencies.runs.register(key).ok_or_else(|| {
        ApiError::runtime(
            "aggregate_initialization_in_progress",
            "aggregate initialization is already in progress",
            json!({}),
        )
    })?;
    let token = lease.cancellation_token();
    let coordinator = dependencies.coordinator.clone();
    let index = dependencies.index.clone();
    let manifest_revision = operation.input.manifest_revision;
    let project_id_for_worker = project_id.clone();
    let operation_id_for_worker = operation.operation_id.clone();
    tokio::spawn(async move {
        // Keep the lease alive for the entire coordinator execution. Dropping
        // it before spawning would make cancellation and recovery observability
        // racy with the first provider turn.
        let _lease = lease;
        match coordinator
            .execute(&project_id_for_worker, &operation_id_for_worker, token)
            .await
        {
            Ok(_) => {
                // Index creation is deliberately detached from initialization
                // durability. A failed index build is observable in its own
                // operation and must not roll back a completed initialization.
                let project_id = project_id_for_worker.clone();
                let operation_id = operation_id_for_worker.clone();
                // Do not extend the initialization lease over the follow-up
                // index build: this is an independent, best-effort task.
                tokio::spawn(async move {
                    let build_project_id = project_id.clone();
                    let result = tokio::task::spawn_blocking(move || {
                        index.build(&build_project_id, manifest_revision)
                    })
                    .await;
                    match result {
                        Ok(Ok(_)) => {}
                        Ok(Err(error)) => {
                            tracing::warn!(
                                project_id = %project_id,
                                operation_id = %operation_id,
                                error = %error,
                                "aggregate initialization index build stopped"
                            );
                            // `AggregateIndexOperation::build` persists a Failed
                            // generation before returning. Keep this detached
                            // failure independent from the already-Completed
                            // initialization operation, while making the
                            // missing projection actionable through GET active.
                        }
                        Err(error) => tracing::warn!(
                            project_id = %project_id,
                            operation_id = %operation_id,
                            error = %error,
                            "aggregate initialization index worker panicked"
                        ),
                    }
                });
            }
            Err(error) => {
                tracing::warn!(
                    project_id = %project_id_for_worker,
                    operation_id = %operation_id_for_worker,
                    error = %error,
                    "aggregate initialization worker stopped"
                );
            }
        }
    });
    Ok((StatusCode::ACCEPTED, Json(aggregate_initialization_dto(operation))).into_response())
}

pub async fn get_aggregate_initialization(
    State(state): State<WebAppState>,
    Path((project_id, operation_id)): Path<(String, String)>,
) -> ApiResult<Response> {
    let project_paths = product_app_paths(&state);
    require_multi_repo_project(&project_paths, &project_id)?;
    validate_project_id(&project_id)?;
    validate_operation_id(&operation_id)?;
    let dependencies = aggregate_initialization_dependencies(&state);
    let operation = dependencies
        .coordinator
        .get(&project_id, &operation_id)
        .map_err(aggregate_initialization_api_error)?;
    let operation = if matches!(
        operation.status,
        AggregateInitializationOperationStatus::Running
    ) && !dependencies
        .runs
        .is_active(&InitializationRunKey::aggregate(&project_id, &operation_id))
    {
        dependencies
            .coordinator
            .recover_interrupted(&project_id, &operation_id)
            .map_err(aggregate_initialization_api_error)?
    } else {
        operation
    };
    Ok(Json(aggregate_initialization_dto(operation)).into_response())
}

pub async fn cancel_aggregate_initialization(
    State(state): State<WebAppState>,
    Path((project_id, operation_id)): Path<(String, String)>,
    Json(request): Json<CancelAggregateInitializationRequest>,
) -> ApiResult<Response> {
    let project_paths = product_app_paths(&state);
    require_multi_repo_project(&project_paths, &project_id)?;
    validate_project_id(&project_id)?;
    validate_operation_id(&operation_id)?;
    let dependencies = aggregate_initialization_dependencies(&state);
    let operation = dependencies
        .coordinator
        .cancel(
            &project_id,
            &operation_id,
            &request.reason,
            request.detail.clone(),
        )
        .map_err(aggregate_initialization_api_error)?;
    // Persist the cancellation first, then signal the in-memory worker. The
    // worker checks this token at every step boundary and will not advance
    // after the provider turn currently in flight completes.
    dependencies
        .runs
        .cancel(&InitializationRunKey::aggregate(&project_id, &operation_id));
    Ok((StatusCode::OK, Json(aggregate_initialization_dto(operation))).into_response())
}

fn aggregate_initialization_dependencies(
    state: &WebAppState,
) -> AggregateInitializationDependencies {
    state.aggregate_initialization_dependencies()
}

fn load_manifest_for_profile(
    paths: &ProductAppPaths,
    project_id: &str,
) -> ApiResult<crate::product::logical_codebase::store::LogicalCodebaseManifest> {
    let store = crate::product::logical_codebase::LogicalCodebaseStore::new(paths.clone());
    store
        .load_manifest(project_id)
        .map_err(product_store_api_error)?
        .ok_or_else(|| {
            ApiError::runtime(
                "logical_codebase_manifest_missing",
                "logical codebase manifest is missing; register members first",
                json!({}),
            )
        })
}

fn validate_project_id(project_id: &str) -> ApiResult<()> {
    validate_relative_id(project_id).map_err(|error| {
        ApiError::validation("invalid_project_id", format!("invalid project id: {error}"))
    })?;
    Ok(())
}

fn validate_operation_id(operation_id: &str) -> ApiResult<()> {
    validate_relative_id(operation_id).map_err(|error| {
        ApiError::validation(
            "invalid_operation_id",
            format!("invalid operation id: {error}"),
        )
    })?;
    Ok(())
}

fn validate_idempotency_key(key: &str) -> ApiResult<()> {
    if key.is_empty() {
        return Err(ApiError::validation(
            "invalid_idempotency_key",
            "idempotency_key must not be empty",
        ));
    }
    Ok(())
}

fn deterministic_operation_id(project_id: &str, idempotency_key: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(project_id.as_bytes());
    hasher.update(idempotency_key.as_bytes());
    format!("aggregate_initialization_{:x}", hasher.finalize())[..40].to_string()
}

fn aggregate_initialization_api_error(error: AggregateInitializationError) -> ApiError {
    match error {
        AggregateInitializationError::NotFound { .. } => ApiError::runtime(
            "aggregate_initialization_operation_not_found",
            "aggregate initialization operation not found",
            json!({}),
        ),
        AggregateInitializationError::StateRejected { detail, .. } => {
            ApiError::runtime("aggregate_initialization_state_rejected", detail, json!({}))
        }
        AggregateInitializationError::Store(error) => product_store_api_error(error),
        other => ApiError::runtime(
            "aggregate_initialization_failed",
            other.to_string(),
            json!({}),
        ),
    }
}
