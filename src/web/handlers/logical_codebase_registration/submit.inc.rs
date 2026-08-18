pub async fn submit_logical_codebase_registration(
    State(state): State<WebAppState>,
    Path(project_id): Path<String>,
    Json(request): Json<RegistrationSubmitRequest>,
) -> ApiResult<Json<RegistrationBatchDto>> {
    let paths = product_app_paths(&state);
    require_multi_repo_project(&paths, &project_id)?;
    let snapshot = RegistrationPreflightSnapshotStore::new(paths.clone())
        .load_unexpired(&project_id, &request.preflight_id, Utc::now())
        .map_err(product_store_api_error)?
        .ok_or_else(|| {
            product_store_api_error(crate::product::json_store::ProductStoreError::NotFound {
                kind: "registration_preflight",
                id: request.preflight_id.clone(),
            })
        })?;
    let root = std::fs::canonicalize(&request.aggregate_root).map_err(|_| {
        ApiError::runtime(
            "aggregate_root_mismatch",
            "aggregate root does not match frozen preflight",
            serde_json::json!({}),
        )
    })?;
    if snapshot.aggregate_root != root {
        return Err(ApiError::runtime(
            "aggregate_root_mismatch",
            "aggregate root does not match frozen preflight",
            serde_json::json!({}),
        ));
    }

    // Confirmation only filters the persisted candidate list. Never rescan or
    // reconstruct candidates from caller-controlled paths: snapshot ordering,
    // identity and revision evidence stay frozen across this boundary.
    let confirmed = snapshot
        .candidates
        .iter()
        .filter(|candidate| {
            request
                .confirmed_paths
                .iter()
                .any(|path| candidate.submitted_path == std::path::Path::new(path))
        })
        .cloned()
        .collect::<Vec<_>>();
    let preflight = RegistrationPreflightResult {
        project_id: snapshot.project_id.clone(),
        aggregate_root: CanonicalAggregateRoot {
            canonical_path: snapshot.aggregate_root.clone(),
        },
        candidates: confirmed,
    };
    let include_needs_attention = preflight
        .candidates
        .iter()
        .any(|candidate| candidate.state == RegistrationCandidateState::NeedsAttention);
    let coordinator = LogicalCodebaseRegistrationCoordinator::new(
        paths.clone(),
        RepositoryStore::with_logical_codebase_feature(
            paths.clone(),
            LogicalCodebaseFeature::enabled(),
        ),
        LogicalCodebaseFeature::enabled(),
    );
    crate::product::logical_codebase::LogicalCodebaseStore::new(paths)
        .validate_registration_root(&project_id, &snapshot.aggregate_root)
        .map_err(product_store_api_error)?;
    let batch = coordinator.submit_confirmed_batch(
        ConfirmedRegistrationBatchInput::from_preflight(&preflight, include_needs_attention),
    )
    .map_err(product_store_api_error)?;
    let completed = coordinator
        .resume_batch(&project_id, &batch.id)
        .map_err(product_store_api_error)?;
    Ok(Json(RegistrationBatchDto::from_record(&completed)))
}

impl RegistrationBatchDto {
    fn from_record(batch: &crate::product::logical_codebase::RegistrationBatchRecord) -> Self {
        Self {
            batch_id: batch.id.clone(),
            status: match batch.status {
                RegistrationBatchStatus::Completed => "completed",
                _ => "partial_failed",
            }
            .to_string(),
            items: batch
                .items
                .iter()
                .map(|item| RegistrationBatchItemDto {
                    path: item.submitted_path.to_string_lossy().into_owned(),
                    status: match item.status {
                        RegistrationItemStatus::Pending => "pending",
                        RegistrationItemStatus::Skipped => "skipped",
                        RegistrationItemStatus::Completed => "completed",
                        RegistrationItemStatus::Failed => "failed",
                        RegistrationItemStatus::NeedsAttention => "needs_attention",
                    }
                    .to_string(),
                    failure_reason: item.failure_reason.clone(),
                })
                .collect(),
        }
    }
}
