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

pub async fn preflight_logical_codebase_registration(
    State(state): State<WebAppState>,
    Path(project_id): Path<String>,
    Json(request): Json<RegistrationPreflightRequest>,
) -> ApiResult<Json<RegistrationPreflightResponse>> {
    let paths = product_app_paths(&state);
    // Admission must precede all candidate classification and durable writes.
    require_multi_repo_project(&paths, &project_id)?;
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
    let coordinator = LogicalCodebaseRegistrationCoordinator::new(
        paths.clone(),
        RepositoryStore::with_logical_codebase_feature(paths.clone(), LogicalCodebaseFeature::enabled()),
        LogicalCodebaseFeature::enabled(),
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
    RegistrationPreflightSnapshotStore::new(paths)
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
