use super::*;

pub async fn delete_work_item(
    State(state): State<WebAppState>,
    Path((project_id, issue_id, work_item_id)): Path<(String, String, String)>,
) -> ApiResult<Json<serde_json::Value>> {
    let app_paths = product_app_paths(&state);
    let store = LifecycleStore::new(app_paths.clone());
    delete_work_item_with_cleanup(&app_paths, &store, &project_id, &issue_id, &work_item_id)
        .await?;
    Ok(Json(json!({"status":"deleted"})))
}

pub async fn delete_work_item_plan(
    State(state): State<WebAppState>,
    Path((project_id, issue_id, plan_id)): Path<(String, String, String)>,
) -> ApiResult<Json<serde_json::Value>> {
    let app_paths = product_app_paths(&state);
    let store = LifecycleStore::new(app_paths.clone());
    let plan = store
        .get_issue_work_item_plan(&project_id, &issue_id, &plan_id)
        .map_err(product_store_api_error)?;
    if let Some(lineage) = schema_v2_plan_lineage(&app_paths, &project_id, &issue_id, &plan_id)
        .map_err(product_store_api_error)?
    {
        delete_schema_v2_work_item_plan_with_cleanup(
            &app_paths,
            &store,
            &project_id,
            &issue_id,
            &plan_id,
            &lineage,
        )
        .await?;
        return Ok(Json(json!({"status":"deleted"})));
    }
    for work_item_id in &plan.work_item_ids {
        delete_work_item_with_cleanup(&app_paths, &store, &project_id, &issue_id, work_item_id)
            .await?;
    }
    store
        .delete_issue_work_item_plan(&project_id, &issue_id, &plan_id)
        .map_err(product_store_api_error)?;
    Ok(Json(json!({"status":"deleted"})))
}

pub(crate) async fn delete_work_item_with_cleanup(
    app_paths: &ProductAppPaths,
    store: &LifecycleStore,
    project_id: &str,
    issue_id: &str,
    work_item_id: &str,
) -> ApiResult<()> {
    if schema_v2_plan_for_work_item(app_paths, store, project_id, issue_id, work_item_id)
        .map_err(product_store_api_error)?
        .is_some()
    {
        return Err(ApiError::validation(
            "schema_v2_group_delete_required",
            "schema v2 work items must be deleted through their work item plan",
        ));
    }
    let work_item = store
        .list_work_items(project_id, issue_id)
        .map_err(product_store_api_error)?
        .into_iter()
        .find(|work_item| work_item.id == work_item_id)
        .ok_or_else(|| {
            product_store_api_error(ProductStoreError::NotFound {
                kind: "work_item",
                id: work_item_id.to_string(),
            })
        })?;
    let repository = find_repository(app_paths, project_id, &work_item.repository_id)?;
    let coding_store = CodingAttemptStore::new(app_paths.clone());
    let attempts = coding_store
        .list_attempts_for_work_item(project_id, issue_id, work_item_id)
        .map_err(product_store_api_error)?;
    for attempt in attempts {
        let attempt = abort_attempt_if_active(&coding_store, attempt)?;
        cleanup_coding_attempt_workspace(&repository, &attempt).await?;
    }
    coding_store
        .delete_attempts_for_work_item(project_id, issue_id, work_item_id)
        .map_err(product_store_api_error)?;
    store
        .delete_work_item(project_id, issue_id, work_item_id)
        .map_err(product_store_api_error)?;
    Ok(())
}

fn schema_v2_plan_lineage(
    app_paths: &ProductAppPaths,
    project_id: &str,
    issue_id: &str,
    plan_id: &str,
) -> Result<Option<crate::product::models::WorkItemPlanLineage>, ProductStoreError> {
    let revision_store = WorkItemRevisionStore::new(app_paths.clone());
    match revision_store.get_plan_lineage(project_id, issue_id, plan_id) {
        Ok(lineage) => Ok(Some(lineage)),
        Err(ProductStoreError::NotFound {
            kind: "work_item_plan_lineage",
            ..
        }) => Ok(None),
        Err(error) => Err(error),
    }
}

fn schema_v2_plan_for_work_item(
    app_paths: &ProductAppPaths,
    store: &LifecycleStore,
    project_id: &str,
    issue_id: &str,
    work_item_id: &str,
) -> Result<Option<crate::product::models::WorkItemPlanLineage>, ProductStoreError> {
    let revision_store = WorkItemRevisionStore::new(app_paths.clone());
    for plan in store.list_issue_work_item_plans(project_id, issue_id)? {
        let Some(lineage) = schema_v2_plan_lineage(app_paths, project_id, issue_id, &plan.id)?
        else {
            continue;
        };
        let active_revision_id = lineage.active_revision_id.as_deref().ok_or_else(|| {
            ProductStoreError::IdentityMismatch {
                kind: "runtime_binding_missing",
                id: plan.id.clone(),
            }
        })?;
        let revision =
            revision_store.get_plan_revision(project_id, issue_id, &plan.id, active_revision_id)?;
        if revision.work_item_bindings.contains_key(work_item_id) {
            return Ok(Some(lineage));
        }
    }
    Ok(None)
}

async fn delete_schema_v2_work_item_plan_with_cleanup(
    app_paths: &ProductAppPaths,
    store: &LifecycleStore,
    project_id: &str,
    issue_id: &str,
    plan_id: &str,
    lineage: &crate::product::models::WorkItemPlanLineage,
) -> ApiResult<()> {
    let active_revision_id = lineage.active_revision_id.as_deref().ok_or_else(|| {
        product_store_api_error(ProductStoreError::IdentityMismatch {
            kind: "runtime_binding_missing",
            id: plan_id.to_string(),
        })
    })?;
    let revision_store = WorkItemRevisionStore::new(app_paths.clone());
    let plan_revision = revision_store
        .get_plan_revision(project_id, issue_id, plan_id, active_revision_id)
        .map_err(product_store_api_error)?;
    let expected_logical_ids = plan_revision
        .work_item_bindings
        .keys()
        .cloned()
        .collect::<BTreeSet<_>>();
    let runtime_reader = WorkItemRuntimeReader::new(app_paths.clone());
    let mut sessions_by_logical_id = BTreeMap::new();
    for session in store
        .list_workspace_sessions(project_id, issue_id)
        .map_err(product_store_api_error)?
        .into_iter()
        .filter(|session| {
            session.workspace_type == WorkspaceType::WorkItem
                && expected_logical_ids.contains(&session.entity_id)
        })
    {
        let binding = session.work_item_runtime_binding.as_ref().ok_or_else(|| {
            product_store_api_error(ProductStoreError::IdentityMismatch {
                kind: "runtime_binding_missing",
                id: session.id.clone(),
            })
        })?;
        if binding.plan_id != plan_id || binding.logical_work_item_id != session.entity_id {
            return Err(product_store_api_error(
                ProductStoreError::IdentityMismatch {
                    kind: "runtime_binding_integrity_mismatch",
                    id: session.id.clone(),
                },
            ));
        }
        runtime_reader
            .resolve_workspace(&session)
            .map_err(product_store_api_error)?;
        if sessions_by_logical_id
            .insert(session.entity_id.clone(), session)
            .is_some()
        {
            return Err(product_store_api_error(
                ProductStoreError::IdentityMismatch {
                    kind: "runtime_binding_integrity_mismatch",
                    id: plan_id.to_string(),
                },
            ));
        }
    }
    if sessions_by_logical_id.len() != expected_logical_ids.len() {
        return Err(product_store_api_error(
            ProductStoreError::IdentityMismatch {
                kind: "runtime_binding_missing",
                id: plan_id.to_string(),
            },
        ));
    }

    let coding_store = CodingAttemptStore::new(app_paths.clone());
    if let Some(attempt) = coding_store
        .get_attempt_for_work_item_group(project_id, issue_id, plan_id)
        .map_err(product_store_api_error)?
    {
        let issue = IssueStore::new(app_paths.clone())
            .get(project_id, issue_id)
            .map_err(product_store_api_error)?;
        let repository_id = issue.repo_id.ok_or_else(|| {
            product_store_api_error(ProductStoreError::NotFound {
                kind: "issue_repository",
                id: issue_id.to_string(),
            })
        })?;
        let repository = find_repository(app_paths, project_id, &repository_id)?;
        let attempt = abort_attempt_if_active(&coding_store, attempt)?;
        cleanup_coding_attempt_workspace(&repository, &attempt).await?;
        coding_store
            .delete_attempt(project_id, issue_id, &attempt.id)
            .map_err(product_store_api_error)?;
    }

    for session in sessions_by_logical_id.into_values() {
        store
            .delete_workspace_sessions_for_entity(
                project_id,
                issue_id,
                &session.entity_id,
                WorkspaceType::WorkItem,
            )
            .map_err(product_store_api_error)?;
    }
    store
        .delete_schema_v2_issue_work_item_plan_metadata(project_id, issue_id, plan_id)
        .map_err(product_store_api_error)?;
    Ok(())
}
