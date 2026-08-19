//! R9：coding 入口的 work item 仓库解析按 issue 所属 lc_id 寻址。
use super::*;

pub(crate) fn resolve_work_item_repository(
    app_paths: &ProductAppPaths,
    project_id: &str,
    work_item: &LifecycleWorkItemRecord,
) -> ApiResult<RepositoryRecord> {
    let project = ProjectStore::new(app_paths.clone())
        .get(project_id)
        .map_err(product_store_api_error)?;
    // v1.3：按 issue 所属 lc_id 寻址（R9）；单仓/无 LC 回退 legacy project 级路径。
    let lc_id = crate::product::logical_codebase::resolve_issue_logical_codebase_id(
        app_paths,
        project_id,
        &work_item.issue_id,
    )
    .map_err(product_store_api_error)?;
    let store = match lc_id.as_deref() {
        Some(_) => RepositoryStore::new(app_paths.clone()),
        None => RepositoryStore::for_project(app_paths.clone(), &project),
    };
    match RepositoryRouting::load_for_issue(app_paths, project_id, &work_item.issue_id)
        .map_err(product_store_api_error)?
    {
        RepositoryRouting::Legacy { .. } => store
            .resolve_legacy_physical_repository_if_dual(project_id, &work_item.repository_id)
            .map(|(_, _, repository)| repository)
            .or_else(|_| legacy_physical_repository(&store, project_id, &work_item.repository_id))
            .map_err(product_store_api_error),
        RepositoryRouting::Logical { manifest, .. } => {
            let logical_repository_id = work_item.target_repository_id.ok_or_else(|| {
                product_store_api_error(routing_error(
                    RepositoryRoutingErrorCode::TargetMissing,
                    format!("work item {} has no target repository", work_item.id),
                ))
            })?;
            if !manifest.member_ids.contains(&logical_repository_id) {
                return Err(product_store_api_error(routing_error(
                    RepositoryRoutingErrorCode::TargetUnknown,
                    format!(
                        "work item {} target repository is absent from the manifest",
                        work_item.id
                    ),
                )));
            }
            store
                .resolve_logical_repository_for_issue_codebase(
                    project_id,
                    lc_id.as_deref(),
                    logical_repository_id,
                )
                .map(|(_, _, repository)| repository)
                .map_err(product_store_api_error)
        }
        RepositoryRouting::FailClosed { code, reason } => {
            Err(product_store_api_error(routing_error(code, reason)))
        }
    }
}

pub(crate) fn routing_error(
    code: RepositoryRoutingErrorCode,
    reason: impl Into<String>,
) -> ProductStoreError {
    let stable_code = code.stable_code();
    ProductStoreError::InvalidRecord {
        kind: "repository_routing",
        reason: format!("{stable_code}: {}", reason.into()),
    }
}

pub(crate) fn legacy_physical_repository(
    store: &RepositoryStore,
    project_id: &str,
    physical_repository_id: &str,
) -> Result<RepositoryRecord, ProductStoreError> {
    store
        .list(project_id)?
        .into_iter()
        .find(|repository| repository.id == physical_repository_id)
        .ok_or_else(|| ProductStoreError::NotFound {
            kind: "repository",
            id: physical_repository_id.to_string(),
        })
}
