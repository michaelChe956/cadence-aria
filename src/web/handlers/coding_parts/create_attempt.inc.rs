pub async fn create_coding_attempt(
    State(state): State<WebAppState>,
    Path((project_id, issue_id, work_item_id)): Path<(String, String, String)>,
) -> ApiResult<Json<CodingAttemptDto>> {
    let app_paths = product_app_paths(&state);
    let lifecycle = LifecycleStore::new(app_paths.clone());
    let coding_store = CodingAttemptStore::new(app_paths.clone());
    let creation_guard = coding_store
        .acquire_work_item_attempt_creation_async(&project_id, &issue_id, &work_item_id)
        .await
        .map_err(product_store_api_error)?;
    let active_attempts = coding_store
        .list_attempts_for_work_item(&project_id, &issue_id, &work_item_id)
        .map_err(product_store_api_error)?
        .into_iter()
        .filter(|attempt| attempt.status.is_active())
        .collect::<Vec<_>>();
    if active_attempts.len() > 1 {
        return Err(ApiError::runtime(
            "coding_attempt_ambiguous",
            "multiple active coding attempts exist for this work item",
            json!({
                "attempt_ids": active_attempts
                    .iter()
                    .map(|attempt| attempt.id.as_str())
                    .collect::<Vec<_>>()
            }),
        ));
    }
    if let Some(active_attempt) = active_attempts.into_iter().next() {
        if active_attempt.scope == CodingAttemptScope::WorkItem {
            lifecycle
                .bind_issue_worktree_lock_to_attempt(
                    &project_id,
                    &issue_id,
                    &work_item_id,
                    &active_attempt.id,
                )
                .map_err(product_store_api_error)?;
        }
        return Err(ApiError::runtime(
            "coding_attempt_active",
            "work item already has an active coding attempt",
            json!({ "attempt_id": active_attempt.id }),
        ));
    }
    if is_schema_v2_group_work_item(
        &app_paths,
        &lifecycle,
        &project_id,
        &issue_id,
        &work_item_id,
    )
    .map_err(product_store_api_error)?
    {
        return Err(ApiError::validation(
            "schema_v2_group_coding_required",
            "schema v2 work items must start coding through their work item group",
        ));
    }
    let work_items = lifecycle
        .list_work_items(&project_id, &issue_id)
        .map_err(product_store_api_error)?;
    let work_item = work_item_by_id(&work_items, &work_item_id).ok_or_else(|| {
        ApiError::runtime("work_item_not_found", "work item not found", json!({}))
    })?;
    if work_item.plan_status != WorkItemPlanStatus::Confirmed {
        return Err(ApiError::validation(
            "work_item_plan_not_confirmed",
            "work item plan must be confirmed before coding",
        ));
    }

    let missing_dependencies: Vec<String> = work_item
        .depends_on
        .iter()
        .filter(|dep_id| {
            work_items
                .iter()
                .find(|item| &item.id == *dep_id)
                .map(|item| item.execution_status != WorkItemStatus::Completed)
                .unwrap_or(true)
        })
        .cloned()
        .collect();
    if !missing_dependencies.is_empty() {
        return Err(ApiError::validation_with_details(
            "work_item_dependency_not_completed",
            "one or more dependency work items are not completed",
            json!({ "missing_dependencies": missing_dependencies }),
        ));
    }

    if work_item.require_execution_plan_confirm
        && work_item.execution_plan_status != WorkItemExecutionPlanStatus::Confirmed
    {
        return Err(ApiError::validation(
            "work_item_execution_plan_not_confirmed",
            "work item execution plan must be confirmed before coding",
        ));
    }

    let repository = resolve_work_item_repository(&app_paths, &project_id, work_item)?;
    if !is_git_repo(&repository.path) {
        return Err(ApiError::validation(
            "repository_path_not_git_repo",
            "repository path must point to a git work tree",
        ));
    }

    let target_snapshot = attempt_target_snapshot(&app_paths, &project_id, &issue_id, work_item)?;
    let worktree_route = IssueWorktreeRoute::from_target_snapshot(&target_snapshot);

    let branch_name = format!("aria/issues/{issue_id}");
    // REQ-PIB-03（T3.1）：fork 基线读 issue.base_branch 经三面同源解析链
    //（`fork_base_branch_from_issue`）；不可解析 fail-closed 422，不回退当前检出。
    let base_branch =
        fork_base_branch_from_issue(&app_paths, &repository.path, &project_id, &issue_id)?;
    match &worktree_route {
        IssueWorktreeRoute::Legacy => {
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
        }
        IssueWorktreeRoute::Repository { repository_id } => {
            let legacy_error = match LegacySharedWorktreeMigration::load_legacy_shared_worktree(
                &app_paths,
                &project_id,
                &issue_id,
            ) {
                Ok(None) => None,
                Ok(Some(_)) => Some("legacy_shared_worktree_present"),
                Err(ProductStoreError::InvalidRecord { reason, .. })
                    if reason.starts_with("legacy_shared_worktree_inconsistent:") =>
                {
                    Some("legacy_shared_worktree_inconsistent")
                }
                Err(error) => return Err(product_store_api_error(error)),
            };
            if let Some(code) = legacy_error {
                return Err(ApiError::validation(code, "legacy"));
            }
            let shared_worktree_path = target_snapshot
                .as_ref()
                .expect("repository route has target snapshot")
                .canonical_path
                .join(".worktrees")
                .join("aria-issues")
                .join(&issue_id);
            lifecycle
                .upsert_repo_shared_worktree(UpsertRepoSharedWorktreeInput {
                    project_id: project_id.clone(),
                    issue_id: issue_id.clone(),
                    repository_id: *repository_id,
                    branch_name: branch_name.clone(),
                    worktree_path: shared_worktree_path,
                    base_branch: base_branch.clone(),
                })
                .map_err(product_store_api_error)?;
        }
    }
    let worktree_lease_id = match &worktree_route {
        IssueWorktreeRoute::Legacy => {
            format!("issue_worktree_lease_{}", uuid::Uuid::new_v4().simple())
        }
        IssueWorktreeRoute::Repository { .. } => {
            format!("repo_worktree_lease_{}", uuid::Uuid::new_v4().simple())
        }
    };
    let (worktree_lease_acquired, worktree_lease_id) = match &worktree_route {
        IssueWorktreeRoute::Legacy => {
            let lease = lifecycle
                .try_acquire_issue_worktree_lock(
                    &project_id,
                    &issue_id,
                    &work_item_id,
                    &worktree_lease_id,
                )
                .map_err(issue_worktree_active_api_error)?;
            (lease.acquired, lease.lease_id)
        }
        IssueWorktreeRoute::Repository { repository_id } => {
            let lease = lifecycle
                .try_acquire_repo_worktree_lock(
                    &project_id,
                    &issue_id,
                    *repository_id,
                    &work_item_id,
                    &worktree_lease_id,
                )
                .map_err(repo_worktree_active_api_error)?;
            (lease.acquired, lease.lease_id)
        }
    };
    state
        .test_controls
        .pause_coding_attempt_after_worktree_acquire_if_configured()
        .await;

    let provider_config_snapshot = match coding_provider_config_snapshot(
        &lifecycle,
        work_item,
        &repository.default_provider_mode,
        state.test_provider_enabled,
        &*state.provider_availability,
    ) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            if worktree_lease_acquired {
                release_worktree_lock(
                    &lifecycle,
                    &worktree_route,
                    &project_id,
                    &issue_id,
                    &work_item_id,
                    &worktree_lease_id,
                );
            }
            return Err(error);
        }
    };
    let attempt = match coding_store.create_attempt_with_guard(
        CreateCodingAttemptInput {
            project_id: project_id.clone(),
            issue_id: issue_id.clone(),
            work_item_id: work_item.id.clone(),
            base_branch,
            branch_name,
            worktree_path: None,
            provider_config_snapshot,
            target_snapshot,
            max_auto_rework: 2,
        },
        &creation_guard,
    ) {
        Ok(attempt) => attempt,
        Err(
            error @ ProductStoreError::Conflict {
                kind: "active_coding_attempt",
                ..
            },
        ) => return Err(product_store_api_error(error)),
        Err(error) => {
            if worktree_lease_acquired {
                release_worktree_lock(
                    &lifecycle,
                    &worktree_route,
                    &project_id,
                    &issue_id,
                    &work_item_id,
                    &worktree_lease_id,
                );
            }
            return Err(product_store_api_error(error));
        }
    };
    if state
        .test_controls
        .consume_coding_attempt_after_persist_before_bind_failure()
    {
        return Err(ApiError::runtime(
            "coding_attempt_bind_interrupted",
            "coding attempt creation interrupted before worktree lease binding",
            json!({}),
        ));
    }
    let bind_result = match &worktree_route {
        IssueWorktreeRoute::Legacy => lifecycle.bind_issue_worktree_lock_to_attempt(
            &project_id,
            &issue_id,
            &work_item_id,
            &attempt.id,
        ),
        IssueWorktreeRoute::Repository { repository_id } => lifecycle
            .bind_repo_worktree_lock_to_attempt(
                &project_id,
                &issue_id,
                *repository_id,
                &work_item_id,
                &attempt.id,
            ),
    };
    if let Err(error) = bind_result {
        let _ = coding_store.delete_attempt(&project_id, &issue_id, &attempt.id);
        if worktree_lease_acquired {
            release_worktree_lock(
                &lifecycle,
                &worktree_route,
                &project_id,
                &issue_id,
                &work_item_id,
                &worktree_lease_id,
            );
        }
        return Err(product_store_api_error(error));
    }

    let _ = save_work_item_execution_plan_for_attempt(
        &coding_store,
        &lifecycle,
        &attempt,
        work_item,
        &work_items,
    );

    Ok(Json(coding_attempt_dto(&coding_store, &attempt)?))
}
