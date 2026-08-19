/// Legacy `/logical-codebase/registrations/preflight` compatibility alias for
/// the default first logical codebase (v1.2 migration artifact).
pub async fn preflight_logical_codebase_registration(
    State(state): State<WebAppState>,
    Path(project_id): Path<String>,
    Json(request): Json<RegistrationPreflightRequest>,
) -> ApiResult<Json<crate::web::types::RegistrationPreflightResponse>> {
    let paths = product_app_paths(&state);
    let logical_codebase_id = default_logical_codebase_id(&paths, &project_id)?;
    preflight_registration_for_lc(state, project_id, logical_codebase_id, request)
}

/// v1.3 canonical endpoint: preflight is resolved per logical codebase.
pub async fn preflight_lc_registration(
    State(state): State<WebAppState>,
    Path((project_id, logical_codebase_id)): Path<(String, String)>,
    Json(request): Json<RegistrationPreflightRequest>,
) -> ApiResult<Json<crate::web::types::RegistrationPreflightResponse>> {
    let paths = product_app_paths(&state);
    require_logical_codebase(&paths, &project_id, &logical_codebase_id)?;
    preflight_registration_for_lc(state, project_id, logical_codebase_id, request)
}

fn preflight_registration_for_lc(
    state: WebAppState,
    project_id: String,
    logical_codebase_id: String,
    request: RegistrationPreflightRequest,
) -> ApiResult<Json<crate::web::types::RegistrationPreflightResponse>> {
    let paths = product_app_paths(&state);
    // Admission must precede all candidate classification and durable writes.
    let root = std::path::PathBuf::from(&request.aggregate_root);
    // Candidate-level containment is classified by the coordinator into the
    // stable seven classes. The admission validator still runs first, but is
    // intentionally given no candidates so one outside/missing candidate does
    // not reject the complete preflight request.
    let canonical_root = AggregateRootPreflight::new(paths.clone())
        .validate(&project_id, &root, &[])
        .map_err(aggregate_root_api_error)?;
    let candidate_paths = if request.auto_discover {
        discover_direct_git_children(&canonical_root.canonical_path)?
    } else {
        request
            .candidate_paths
            .iter()
            .cloned()
            .map(Into::into)
            .collect::<Vec<_>>()
    };
    let coordinator = LogicalCodebaseRegistrationCoordinator::for_lc(
        paths.clone(),
        logical_codebase_id.clone(),
    );
    let result = coordinator
        .preflight(RegistrationPreflightInput {
            project_id: project_id.clone(),
            aggregate_root: canonical_root.clone(),
            paths: candidate_paths,
        })
        .map_err(crate::web::handlers::support::product_store_api_error)?;
    let id = preflight_id();
    let created_at = Utc::now().to_rfc3339();
    RegistrationPreflightSnapshotStore::for_lc(paths, logical_codebase_id)
        .save(&RegistrationPreflightSnapshot {
            preflight_id: id.clone(),
            project_id,
            aggregate_root: canonical_root.canonical_path,
            candidates: result.candidates.clone(),
            created_at: created_at.clone(),
        })
        .map_err(crate::web::handlers::support::product_store_api_error)?;
    Ok(Json(RegistrationPreflightResponse {
        preflight_id: id,
        created_at,
        items: result
            .candidates
            .iter()
            .map(RegistrationPreflightItemDto::from_candidate)
            .collect(),
    }))
}

fn discover_direct_git_children(root: &std::path::Path) -> ApiResult<Vec<std::path::PathBuf>> {
    let discovery_error = |reason| {
        ApiError::runtime(
            "aggregate_root_missing",
            "aggregate root preflight rejected",
            serde_json::json!({ "reason": reason }),
        )
    };
    let entries = std::fs::read_dir(root).map_err(|error| {
        discovery_error(format!(
            "could not inspect aggregate root {}: {error}",
            root.display()
        ))
    })?;
    let mut candidates = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| {
            discovery_error(format!(
                "could not inspect aggregate root {}: {error}",
                root.display()
            ))
        })?;
        let path = entry.path();
        if entry
            .file_type()
            .map_err(|error| {
                discovery_error(format!(
                    "could not inspect aggregate root entry {}: {error}",
                    path.display()
                ))
            })?
            .is_dir()
            && path.join(".git").exists()
        {
            candidates.push(path);
        }
    }
    candidates.sort();
    Ok(candidates)
}
