use super::super::dto::*;
use super::super::support::*;
use super::super::*;
use super::{coding_provider_config_snapshot, group_work_item_execution_order};

pub async fn create_group_coding_attempt(
    State(state): State<WebAppState>,
    Path((project_id, issue_id, plan_id)): Path<(String, String, String)>,
) -> ApiResult<Json<CodingAttemptDto>> {
    let app_paths = product_app_paths(&state);
    let lifecycle = LifecycleStore::new(app_paths.clone());
    let plan = lifecycle
        .get_issue_work_item_plan(&project_id, &issue_id, &plan_id)
        .map_err(product_store_api_error)?;
    if plan.status != IssueWorkItemPlanStatus::Confirmed {
        return Err(ApiError::validation(
            "work_item_plan_not_confirmed",
            "work item plan must be confirmed before group coding",
        ));
    }

    let group_lock_key = format!("work_item_group:{project_id}:{issue_id}:{plan_id}");
    let _group_guard = state.coding_runs.lock_named(&group_lock_key).await;
    let coding_store = CodingAttemptStore::new(app_paths.clone());
    if let Some(existing) = coding_store
        .get_attempt_for_work_item_group(&project_id, &issue_id, &plan_id)
        .map_err(product_store_api_error)?
    {
        coding_store
            .validate_group_attempt_integrity(&existing)
            .map_err(coding_group_attempt_incomplete_api_error)?;
        return Ok(Json(coding_attempt_dto(&existing)));
    }

    let all_work_items = lifecycle
        .list_work_items(&project_id, &issue_id)
        .map_err(product_store_api_error)?;
    let ordered = group_work_item_execution_order(&plan, &all_work_items)?;
    if ordered.is_empty() {
        return Err(ApiError::validation(
            "work_item_group_empty",
            "work item group has no compiled work items",
        ));
    }
    if let Some(mismatched) = ordered
        .iter()
        .find(|item| item.work_item_set_id.as_deref() != Some(plan_id.as_str()))
    {
        return Err(ApiError::validation_with_details(
            "work_item_group_mismatch",
            "compiled work item does not belong to the selected group",
            json!({ "work_item_id": mismatched.id }),
        ));
    }

    let authoritative = coding_store
        .resolve_authoritative_group_plan_binding(&project_id, &issue_id, &plan_id)
        .map_err(coding_plan_revision_binding_api_error)?;
    if authoritative
        .units
        .iter()
        .map(|unit| unit.logical_work_item_id.as_str())
        .ne(ordered.iter().map(|item| item.id.as_str()))
    {
        return Err(coding_plan_revision_binding_api_error(
            ProductStoreError::IdentityMismatch {
                kind: "coding_group_order",
                id: plan_id.clone(),
            },
        ));
    }
    let bound_plan_revision_id = authoritative.plan_revision_id;
    let unit_bindings = authoritative.units;

    let current_work_item = ordered.first().expect("checked non-empty");
    let repository = find_repository(&app_paths, &project_id, &current_work_item.repository_id)?;
    if !is_git_repo(&repository.path) {
        return Err(ApiError::validation(
            "repository_path_not_git_repo",
            "repository path must point to a git work tree",
        ));
    }

    let branch_name = format!("aria/issues/{issue_id}");
    let base_branch = current_git_branch(&repository.path).unwrap_or_else(|| "HEAD".to_string());
    let shared_worktree_path = repository
        .path
        .join(".worktrees")
        .join("aria-issues")
        .join(&issue_id);
    lifecycle
        .upsert_issue_shared_worktree(UpsertIssueSharedWorktreeInput {
            project_id: project_id.clone(),
            issue_id: issue_id.clone(),
            repository_id: repository.id.clone(),
            branch_name: branch_name.clone(),
            worktree_path: shared_worktree_path,
            base_branch: base_branch.clone(),
        })
        .map_err(product_store_api_error)?;
    let already_locked_by_current = lifecycle
        .get_issue_shared_worktree(&project_id, &issue_id)
        .map_err(product_store_api_error)?
        .and_then(|record| record.current_active_work_item_id)
        .as_deref()
        == Some(current_work_item.id.as_str());
    let _lock = lifecycle
        .try_acquire_issue_worktree_lock(&project_id, &issue_id, &current_work_item.id)
        .map_err(|error| match error {
            ProductStoreError::Io(ref msg) if msg.contains("issue_worktree_active") => {
                ApiError::runtime(
                    "issue_worktree_active",
                    "another work item is already active on the issue shared worktree",
                    json!({}),
                )
            }
            _ => product_store_api_error(error),
        })?;

    let provider_config_snapshot = coding_provider_config_snapshot(
        &lifecycle,
        current_work_item,
        &repository.default_provider_mode,
        &*state.provider_availability,
    )?;
    let attempt = match coding_store.create_group_attempt(CreateGroupCodingAttemptInput {
        project_id: project_id.clone(),
        issue_id: issue_id.clone(),
        plan_id: plan_id.clone(),
        current_work_item_id: current_work_item.id.clone(),
        base_branch,
        branch_name,
        worktree_path: None,
        provider_config_snapshot,
        max_auto_rework: 2,
    }) {
        Ok(attempt) => attempt,
        Err(ProductStoreError::Io(message))
            if message.starts_with("coding_attempt_group_already_exists:") =>
        {
            if !already_locked_by_current {
                let _ = lifecycle.release_issue_worktree_lock(
                    &project_id,
                    &issue_id,
                    &current_work_item.id,
                );
            }
            let existing_id = message
                .strip_prefix("coding_attempt_group_already_exists:")
                .expect("matched prefix")
                .trim();
            let existing = coding_store
                .get_attempt(&project_id, &issue_id, existing_id)
                .map_err(product_store_api_error)?;
            coding_store
                .validate_group_attempt_integrity(&existing)
                .map_err(coding_group_attempt_incomplete_api_error)?;
            return Ok(Json(coding_attempt_dto(&existing)));
        }
        Err(error) => {
            if !already_locked_by_current {
                let _ = lifecycle.release_issue_worktree_lock(
                    &project_id,
                    &issue_id,
                    &current_work_item.id,
                );
            }
            return Err(match error {
                ProductStoreError::Io(message)
                    if message.starts_with("active_coding_attempt_exists:") =>
                {
                    ApiError::runtime(
                        "issue_worktree_active",
                        "another work item is already active on the issue shared worktree",
                        json!({}),
                    )
                }
                other => product_store_api_error(other),
            });
        }
    };

    if let Err(error) = coding_store.save_plan_binding(
        &attempt,
        &CodingAttemptPlanBinding {
            attempt_id: attempt.id.clone(),
            plan_id: plan_id.clone(),
            bound_plan_revision_id,
            applied_amendment_ids: Vec::new(),
            updated_at: chrono::Utc::now().to_rfc3339(),
        },
    ) {
        rollback_group_attempt_creation(
            &coding_store,
            &lifecycle,
            &project_id,
            &issue_id,
            &current_work_item.id,
            &attempt.id,
            already_locked_by_current,
        )
        .map_err(product_store_api_error)?;
        return Err(product_store_api_error(error));
    }

    for (index, unit_binding) in unit_bindings.into_iter().enumerate() {
        if let Err(error) = coding_store.create_coding_unit(CreateCodingExecutionUnitInput {
            attempt_id: attempt.id.clone(),
            project_id: project_id.clone(),
            issue_id: issue_id.clone(),
            plan_id: plan_id.clone(),
            logical_work_item_id: unit_binding.logical_work_item_id,
            work_item_revision_id: unit_binding.work_item_revision_id,
            dependency_logical_work_item_ids: unit_binding.dependency_logical_work_item_ids,
            order_index: index as u32,
            status: if index == 0 {
                CodingExecutionUnitStatus::Running
            } else {
                CodingExecutionUnitStatus::Pending
            },
        }) {
            rollback_group_attempt_creation(
                &coding_store,
                &lifecycle,
                &project_id,
                &issue_id,
                &current_work_item.id,
                &attempt.id,
                already_locked_by_current,
            )
            .map_err(product_store_api_error)?;
            return Err(product_store_api_error(error));
        }
    }

    let persisted_attempt = coding_store
        .get_attempt(&project_id, &issue_id, &attempt.id)
        .map_err(product_store_api_error)?;

    Ok(Json(coding_attempt_dto(&persisted_attempt)))
}

fn coding_plan_revision_binding_api_error(_error: ProductStoreError) -> ApiError {
    ApiError::validation(
        "coding_plan_revision_binding_missing",
        "group coding requires complete authoritative plan revision bindings",
    )
}

fn coding_group_attempt_incomplete_api_error(_error: ProductStoreError) -> ApiError {
    ApiError::validation(
        "coding_group_attempt_incomplete",
        "existing group coding attempt is only partially initialized or inconsistent",
    )
}

fn rollback_group_attempt_creation(
    coding_store: &CodingAttemptStore,
    lifecycle: &LifecycleStore,
    project_id: &str,
    issue_id: &str,
    lock_work_item_id: &str,
    attempt_id: &str,
    already_locked_by_current: bool,
) -> Result<(), ProductStoreError> {
    coding_store.delete_attempt(project_id, issue_id, attempt_id)?;
    if !already_locked_by_current {
        lifecycle.release_issue_worktree_lock(project_id, issue_id, lock_work_item_id)?;
    }
    Ok(())
}
