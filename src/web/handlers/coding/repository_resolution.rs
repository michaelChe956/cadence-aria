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
    // C4 Task 2：kind 由 work item target 形状显式决定，身份经唯一 authority
    // resolver 冻结（issue 归属冲突/来源重复/legacy 布局冲突一律 fail-closed，
    // 且在任何 target lookup 之前）。
    // - `target_repository_id` 存在 → LogicalCodebase 形状：resolver 确认 LC
    //   authority 后按 manifest 成员严格解析。
    // - 否则 → SingleRepo 形状：显式 SingleRepo 请求先经 resolver 拒绝
    //   “单仓请求命中 LC 归属 issue”的 kind_mismatch，再走既有 legacy 兼容解析。
    let resolver =
        crate::product::logical_codebase::RepositoryAuthorityResolver::new(app_paths.clone());
    if let Some(logical_repository_id) = work_item.target_repository_id {
        let resolution = resolver
            .resolve_for_issue(project_id, &work_item.issue_id)
            .map_err(product_store_api_error)?
            .ok_or_else(|| {
                product_store_api_error(ProductStoreError::Conflict {
                    kind: "repository_routing_kind_mismatch",
                    id: format!(
                        "work_item:{}:target_repository_present:issue_not_logical",
                        work_item.id
                    ),
                })
            })?;
        let manifest = resolution.manifest.clone().ok_or_else(|| {
            product_store_api_error(routing_error(
                RepositoryRoutingErrorCode::TargetMissing,
                format!(
                    "work item {} has no logical codebase manifest",
                    work_item.id
                ),
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
        let store = RepositoryStore::new(app_paths.clone());
        return store
            .resolve_logical_repository_for_issue_codebase(
                project_id,
                resolution.target.logical_codebase_id.as_deref(),
                logical_repository_id,
            )
            .map(|(_, _, repository)| repository)
            .map_err(product_store_api_error);
    }
    // SingleRepo 形状：物理 repository_id 显式请求。issue 已归属 LC 时 resolver
    // 在读取任何仓库记录前返回 kind_mismatch（fail-closed）。
    let attributed = crate::product::logical_codebase::resolve_issue_logical_codebase_id(
        app_paths,
        project_id,
        &work_item.issue_id,
    )
    .map_err(product_store_api_error)?;
    if attributed.is_some() {
        // resolver 在 issue 归属校验点（任何仓库记录读取之前）fail-closed。
        return resolver
            .resolve(crate::product::logical_codebase::RepositoryRoutingRequest {
                project_id: project_id.to_string(),
                issue_id: Some(work_item.issue_id.clone()),
                kind: crate::product::logical_codebase::RepositoryTargetKind::SingleRepo,
                repository_id: Some(work_item.repository_id.clone()),
                logical_codebase_id: None,
                logical_repository_id: None,
                checkout_id: None,
            })
            .map(|_| unreachable!("resolver must reject single-repo request for LC issue"))
            .map_err(product_store_api_error);
    }
    let store = RepositoryStore::for_project(app_paths.clone(), &project);
    store
        .resolve_legacy_physical_repository_if_dual(project_id, &work_item.repository_id)
        .map(|(_, _, repository)| repository)
        .or_else(|_| legacy_physical_repository(&store, project_id, &work_item.repository_id))
        .map_err(product_store_api_error)
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
