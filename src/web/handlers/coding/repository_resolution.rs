//! R9：coding 入口的 work item 仓库解析按 issue 所属 lc_id 寻址。
use super::*;

pub(crate) fn resolve_work_item_repository(
    app_paths: &ProductAppPaths,
    project_id: &str,
    work_item: &LifecycleWorkItemRecord,
) -> ApiResult<RepositoryRecord> {
    // project 存在性门（404）保留；绑定值不再被单仓回退路径消费。
    ProjectStore::new(app_paths.clone())
        .get(project_id)
        .map_err(product_store_api_error)?;
    // C4 Task 2：kind 由 work item target 形状显式决定，身份经唯一 authority
    // resolver 冻结（issue 归属冲突/来源重复/legacy 布局冲突一律 fail-closed，
    // 且在任何 target lookup 之前）。
    // - `target_repository_id` 存在 → LogicalCodebase 形状：resolver 确认 LC
    //   authority 后按 manifest 成员严格解析。
    // - 否则 → SingleRepo 形状：显式 SingleRepo 请求经 resolver 拒绝
    //   “单仓请求命中 LC 归属 issue”的 kind_mismatch（fail-closed，不再
    //   回退 legacy 物理仓——REQ-ROUTE-02）。
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
    // SingleRepo 形状：物理 repository_id 显式请求。REQ-ROUTE-01/02：issue
    // 归属必填（老记录缺归属由读取层显式报 issue_not_bound_to_logical_codebase
    // 而非 Ok(None)），resolver 在 issue 归属校验点（任何仓库记录读取之前）
    // fail-closed 返回 kind_mismatch——统一分流不再回退物理仓。
    resolver
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
