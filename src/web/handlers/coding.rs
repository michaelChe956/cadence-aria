use super::dto::*;
use super::support::*;
use super::*;
use crate::product::coding_models::CodingAttemptScope;

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
    let coding_store = CodingAttemptStore::new(app_paths.clone());
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

    for (index, item) in ordered.iter().enumerate() {
        if let Err(error) = coding_store.create_coding_unit(CreateCodingExecutionUnitInput {
            attempt_id: attempt.id.clone(),
            project_id: project_id.clone(),
            issue_id: issue_id.clone(),
            plan_id: plan_id.clone(),
            work_item_id: item.id.clone(),
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

pub(crate) fn rollback_group_attempt_creation(
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

pub async fn create_coding_attempt(
    State(state): State<WebAppState>,
    Path((project_id, issue_id, work_item_id)): Path<(String, String, String)>,
) -> ApiResult<Json<CodingAttemptDto>> {
    let app_paths = product_app_paths(&state);
    let lifecycle = LifecycleStore::new(app_paths.clone());
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

    let missing_handoffs: Vec<String> = work_item
        .required_handoff_from
        .iter()
        .filter(|handoff_id| {
            work_items
                .iter()
                .find(|item| &item.id == *handoff_id)
                .map(|item| item.handoff_summary_ref.is_none())
                .unwrap_or(true)
        })
        .cloned()
        .collect();
    if !missing_handoffs.is_empty() {
        return Err(ApiError::validation_with_details(
            "work_item_handoff_missing",
            "required dependency handoff summary is missing",
            json!({ "missing_handoffs": missing_handoffs }),
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

    let repository = find_repository(&app_paths, &project_id, &work_item.repository_id)?;
    if !is_git_repo(&repository.path) {
        return Err(ApiError::validation(
            "repository_path_not_git_repo",
            "repository path must point to a git work tree",
        ));
    }

    let coding_store = CodingAttemptStore::new(app_paths.clone());
    if coding_store
        .get_active_attempt(&project_id, &issue_id, &work_item.id)
        .map_err(product_store_api_error)?
        .is_some()
    {
        return Err(ApiError::runtime(
            "coding_attempt_active",
            "work item already has an active coding attempt",
            json!({}),
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
    let _ = lifecycle
        .try_acquire_issue_worktree_lock(&project_id, &issue_id, &work_item_id)
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
        work_item,
        &repository.default_provider_mode,
        &*state.provider_availability,
    )?;
    let attempt_result = coding_store.create_attempt(CreateCodingAttemptInput {
        project_id: project_id.clone(),
        issue_id: issue_id.clone(),
        work_item_id: work_item.id.clone(),
        base_branch,
        branch_name,
        worktree_path: None,
        provider_config_snapshot,
        max_auto_rework: 2,
    });

    if attempt_result.is_err() {
        let _ = lifecycle.release_issue_worktree_lock(&project_id, &issue_id, &work_item_id);
    }
    let attempt = attempt_result.map_err(product_store_api_error)?;

    let _ = save_work_item_execution_plan_for_attempt(
        &coding_store,
        &lifecycle,
        &attempt,
        work_item,
        &work_items,
    );

    Ok(Json(coding_attempt_dto(&attempt)))
}

pub(crate) fn save_work_item_execution_plan_for_attempt(
    coding_store: &CodingAttemptStore,
    lifecycle: &LifecycleStore,
    attempt: &CodingExecutionAttempt,
    work_item: &LifecycleWorkItemRecord,
    all_work_items: &[LifecycleWorkItemRecord],
) -> Result<(), ApiError> {
    let verification_summary = work_item
        .verification_plan_ref
        .as_ref()
        .and_then(|plan_id| {
            lifecycle
                .get_verification_plan(&attempt.project_id, &attempt.issue_id, plan_id)
                .ok()
                .map(|plan| {
                    let gates = plan.required_gates.join(", ");
                    format!("provider supplied required gate {}", gates)
                })
        });

    let dependency_handoffs: Vec<WorkItemDependencyHandoffRef> = work_item
        .required_handoff_from
        .iter()
        .filter_map(|dep_id| {
            all_work_items
                .iter()
                .find(|item| &item.id == dep_id)
                .map(|dep| WorkItemDependencyHandoffRef {
                    work_item_id: dep.id.clone(),
                    summary_ref: dep.handoff_summary_ref.clone(),
                    summary: dep
                        .handoff_summary_ref
                        .clone()
                        .map(|r| format!("handoff summary available at {}", r)),
                    commit_sha: dep.completion_commit.clone(),
                })
        })
        .collect();

    let plan = WorkItemExecutionPlan {
        id: next_execution_plan_id(
            coding_store,
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
        ),
        project_id: attempt.project_id.clone(),
        issue_id: attempt.issue_id.clone(),
        work_item_id: attempt.work_item_id.clone(),
        attempt_id: attempt.id.clone(),
        status: WorkItemExecutionPlanStatus::Draft,
        goal: work_item.title.clone(),
        allowed_write_scopes: work_item.exclusive_write_scopes.clone(),
        forbidden_write_scopes: work_item.forbidden_write_scopes.clone(),
        dependency_handoffs,
        story_refs: work_item.story_spec_ids.clone(),
        design_refs: work_item.design_spec_ids.clone(),
        openspec_refs: Vec::new(),
        superpowers_contract: String::new(),
        tdd_contract: String::new(),
        verification_plan_ref: work_item.verification_plan_ref.clone(),
        verification_summary,
        risk_notes: Vec::new(),
        created_at: attempt.created_at.clone(),
        updated_at: attempt.updated_at.clone(),
    };

    coding_store
        .save_work_item_execution_plan(&plan)
        .map_err(product_store_api_error)
}

pub(crate) fn group_work_item_execution_order(
    plan: &IssueWorkItemPlanRecord,
    work_items: &[LifecycleWorkItemRecord],
) -> Result<Vec<LifecycleWorkItemRecord>, ApiError> {
    let mut selected = plan
        .work_item_ids
        .iter()
        .enumerate()
        .map(|(index, id)| {
            work_items
                .iter()
                .find(|item| &item.id == id)
                .cloned()
                .map(|item| (index, item))
                .ok_or_else(|| {
                    ApiError::runtime(
                        "work_item_not_found",
                        "plan work item not found",
                        json!({ "work_item_id": id }),
                    )
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    selected.sort_by(|(left_index, left_item), (right_index, right_item)| {
        left_item
            .sequence_hint
            .unwrap_or(u32::MAX)
            .cmp(&right_item.sequence_hint.unwrap_or(u32::MAX))
            .then_with(|| left_index.cmp(right_index))
    });
    Ok(selected.into_iter().map(|(_, item)| item).collect())
}

pub(crate) fn next_execution_plan_id(
    _coding_store: &CodingAttemptStore,
    project_id: &str,
    issue_id: &str,
    attempt_id: &str,
) -> String {
    format!(
        "work_item_execution_plan_{}_{}_{}",
        project_id, issue_id, attempt_id
    )
}

pub(crate) fn work_item_by_id<'a>(
    work_items: &'a [LifecycleWorkItemRecord],
    work_item_id: &str,
) -> Option<&'a LifecycleWorkItemRecord> {
    work_items.iter().find(|item| item.id == work_item_id)
}

pub(crate) fn coding_provider_config_snapshot(
    lifecycle: &LifecycleStore,
    work_item: &LifecycleWorkItemRecord,
    repository_default_provider: &str,
    provider_availability: &dyn Fn(&ProviderName) -> bool,
) -> ApiResult<ProviderConfigSnapshot> {
    let sessions = lifecycle
        .list_workspace_sessions(&work_item.project_id, &work_item.issue_id)
        .map_err(product_store_api_error)?;
    if let Some(session) = sessions.iter().rev().find(|session| {
        session.entity_id == work_item.id
            && session.workspace_type == WorkspaceType::WorkItem
            && session.status == WorkspaceSessionStatus::Confirmed
    }) {
        let author = resolve_explicit_provider_name(
            provider_name_key(&session.author_provider),
            provider_availability,
        )?
        .provider;
        let reviewer = resolve_explicit_provider_name(
            provider_name_key(&session.reviewer_provider),
            provider_availability,
        )?
        .provider;
        return Ok(ProviderConfigSnapshot {
            author,
            reviewer: Some(reviewer),
            review_rounds: session.review_rounds,
        });
    }

    let author =
        resolve_default_coding_provider(repository_default_provider, provider_availability)?
            .provider;
    Ok(ProviderConfigSnapshot {
        author: author.clone(),
        reviewer: Some(author),
        review_rounds: 1,
    })
}

pub async fn get_coding_attempt(
    State(state): State<WebAppState>,
    Path(attempt_id): Path<String>,
) -> ApiResult<Json<CodingAttemptSnapshotResponse>> {
    let app_paths = product_app_paths(&state);
    let coding_store = CodingAttemptStore::new(app_paths);
    let attempt = coding_store
        .get_attempt_by_id(&attempt_id)
        .map_err(product_store_api_error)?;
    let timeline_nodes = coding_store
        .get_timeline_nodes(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .map_err(product_store_api_error)?;
    let testing_report = coding_store
        .list_testing_reports(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .map_err(product_store_api_error)?
        .into_iter()
        .last();
    let code_review_reports = coding_store
        .list_code_review_reports(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .map_err(product_store_api_error)?;
    let review_request = coding_store
        .list_review_requests(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .map_err(product_store_api_error)?
        .into_iter()
        .last();
    let internal_pr_review = coding_store
        .list_internal_pr_reviews(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .map_err(product_store_api_error)?
        .into_iter()
        .last();
    let latest_analyst_decision = coding_store
        .latest_analyst_decision(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .map_err(product_store_api_error)?;
    let pending_choices = coding_store
        .list_open_choice_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .map_err(product_store_api_error)?;
    let active_node_id = active_coding_timeline_node_id(&timeline_nodes);
    let work_item_execution_plan = coding_store
        .get_work_item_execution_plan(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .map_err(product_store_api_error)?;
    let work_item_handoff = coding_store
        .get_visible_work_item_handoff(&attempt)
        .map_err(product_store_api_error)?;
    let units = if matches!(attempt.scope, CodingAttemptScope::WorkItemGroup) {
        coding_store
            .list_coding_units(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .map_err(product_store_api_error)?
            .into_iter()
            .map(|unit| coding_execution_unit_dto(&unit))
            .collect()
    } else {
        Vec::new()
    };

    Ok(Json(CodingAttemptSnapshotResponse {
        attempt: coding_attempt_dto(&attempt),
        attempt_scope: coding_attempt_scope_text(&attempt.scope).to_string(),
        work_item_group_id: attempt.work_item_group_id.clone(),
        current_work_item_id: attempt.current_work_item_id.clone(),
        active_unit_id: attempt.active_unit_id.clone(),
        units,
        provider_config_snapshot: attempt.provider_config_snapshot,
        timeline_nodes,
        active_node_id,
        testing_report,
        code_review_reports,
        review_request,
        internal_pr_review,
        pending_gates: Vec::new(),
        pending_choices,
        latest_analyst_decision,
        work_item_execution_plan,
        work_item_handoff,
    }))
}

pub async fn coding_attempt_diff(
    State(state): State<WebAppState>,
    Path(attempt_id): Path<String>,
) -> ApiResult<Json<CodingAttemptDiffResponse>> {
    let app_paths = product_app_paths(&state);
    let coding_store = CodingAttemptStore::new(app_paths);
    let attempt = coding_store
        .get_attempt_by_id(&attempt_id)
        .map_err(product_store_api_error)?;
    let worktree_path = attempt.worktree_path.clone().ok_or_else(|| {
        ApiError::runtime(
            "coding_attempt_worktree_not_ready",
            "coding attempt worktree is not ready",
            json!({}),
        )
    })?;
    let diff = GitWorkspaceService::new()
        .git_diff(&worktree_path, &attempt.base_branch)
        .await
        .map_err(git_workspace_diff_api_error)?;

    Ok(Json(CodingAttemptDiffResponse {
        attempt_id: attempt.id,
        base_branch: attempt.base_branch,
        worktree_path,
        diff,
    }))
}

pub async fn abort_coding_attempt(
    State(state): State<WebAppState>,
    Path(attempt_id): Path<String>,
) -> ApiResult<Json<CodingAttemptDto>> {
    let app_paths = product_app_paths(&state);
    let coding_store = CodingAttemptStore::new(app_paths.clone());
    let attempt = coding_store
        .get_attempt_by_id(&attempt_id)
        .map_err(product_store_api_error)?;
    let engine = coding_workspace_engine_with_dummy_events(coding_store);
    let aborted = engine
        .handle_abort(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .await
        .map_err(coding_workspace_api_error)?;
    Ok(Json(coding_attempt_dto(&aborted)))
}

pub async fn delete_coding_attempt(
    State(state): State<WebAppState>,
    Path(attempt_id): Path<String>,
) -> ApiResult<Response> {
    let app_paths = product_app_paths(&state);
    let coding_store = CodingAttemptStore::new(app_paths.clone());
    let lifecycle = LifecycleStore::new(app_paths.clone());
    let attempt = coding_store
        .get_attempt_by_id(&attempt_id)
        .map_err(product_store_api_error)?;
    let work_item = lifecycle
        .list_work_items(&attempt.project_id, &attempt.issue_id)
        .map_err(product_store_api_error)?
        .into_iter()
        .find(|work_item| work_item.id == attempt.work_item_id)
        .ok_or_else(|| {
            product_store_api_error(ProductStoreError::NotFound {
                kind: "work_item",
                id: attempt.work_item_id.clone(),
            })
        })?;
    let repository = find_repository(&app_paths, &attempt.project_id, &work_item.repository_id)?;

    if let Ok(Some(shared)) =
        lifecycle.get_issue_shared_worktree(&attempt.project_id, &attempt.issue_id)
        && shared.current_active_work_item_id.as_deref() == Some(&attempt.work_item_id)
    {
        let engine = coding_workspace_engine_with_dummy_events(coding_store.clone());
        engine
            .handle_delete_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .await
            .map_err(coding_workspace_api_error)?;
    }

    cleanup_coding_attempt_workspace(&repository, &attempt).await?;
    coding_store
        .delete_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .map_err(product_store_api_error)?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

pub async fn confirm_work_item_execution_plan(
    State(state): State<WebAppState>,
    Path(attempt_id): Path<String>,
) -> ApiResult<Json<WorkItemExecutionPlan>> {
    let app_paths = product_app_paths(&state);
    let coding_store = CodingAttemptStore::new(app_paths.clone());
    let lifecycle = LifecycleStore::new(app_paths);
    let attempt = coding_store
        .get_attempt_by_id(&attempt_id)
        .map_err(product_store_api_error)?;

    let plan = coding_store
        .update_work_item_execution_plan_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            WorkItemExecutionPlanStatus::Confirmed,
        )
        .map_err(product_store_api_error)?;

    let _ = lifecycle.update_work_item_execution_plan_status(
        &attempt.project_id,
        &attempt.issue_id,
        &attempt.work_item_id,
        WorkItemExecutionPlanStatus::Confirmed,
    );

    Ok(Json(plan))
}

pub async fn request_work_item_execution_plan_change(
    State(state): State<WebAppState>,
    Path(attempt_id): Path<String>,
    Json(payload): Json<RequestExecutionPlanChangeRequest>,
) -> ApiResult<Json<WorkItemExecutionPlan>> {
    let app_paths = product_app_paths(&state);
    let coding_store = CodingAttemptStore::new(app_paths.clone());
    let lifecycle = LifecycleStore::new(app_paths);
    let attempt = coding_store
        .get_attempt_by_id(&attempt_id)
        .map_err(product_store_api_error)?;

    let mut plan = coding_store
        .get_work_item_execution_plan(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .map_err(product_store_api_error)?
        .ok_or_else(|| {
            ApiError::runtime(
                "work_item_execution_plan_not_found",
                "execution plan not found",
                json!({}),
            )
        })?;

    plan.status = WorkItemExecutionPlanStatus::ChangeRequested;
    if !payload.note.is_empty() {
        plan.risk_notes.push(payload.note);
    }
    plan.updated_at = chrono::Utc::now().to_rfc3339();

    coding_store
        .save_work_item_execution_plan(&plan)
        .map_err(product_store_api_error)?;

    let _ = lifecycle.update_work_item_execution_plan_status(
        &attempt.project_id,
        &attempt.issue_id,
        &attempt.work_item_id,
        WorkItemExecutionPlanStatus::ChangeRequested,
    );

    Ok(Json(plan))
}

pub async fn coding_attempt_artifact_content(
    State(state): State<WebAppState>,
    Path((attempt_id, artifact_id)): Path<(String, String)>,
) -> ApiResult<Json<ArtifactContentResponse>> {
    validate_relative_id(&artifact_id)
        .map_err(|_| ApiError::validation("invalid_artifact_id", "invalid artifact id"))?;
    let app_paths = product_app_paths(&state);
    let coding_store = CodingAttemptStore::new(app_paths);
    let attempt = coding_store
        .get_attempt_by_id(&attempt_id)
        .map_err(product_store_api_error)?;
    let artifact_path = coding_store
        .attempt_test_output_path(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &artifact_id,
        )
        .map_err(product_store_api_error)?;
    if !artifact_path.is_file() {
        return Err(ApiError::runtime(
            "artifact_not_found",
            "coding attempt artifact not found",
            json!({}),
        ));
    }
    let content = fs::read_to_string(&artifact_path).map_err(|error| {
        ApiError::runtime(
            "artifact_read_failed",
            "coding attempt artifact could not be read",
            json!({"error": error.to_string()}),
        )
    })?;

    Ok(Json(ArtifactContentResponse {
        artifact_ref: artifact_id,
        artifact_kind: "coding_attempt_artifact".to_string(),
        producer_node: None,
        path: artifact_path.to_string_lossy().to_string(),
        content_type: "text/plain".to_string(),
        content,
    }))
}

pub(crate) fn abort_attempt_if_active(
    coding_store: &CodingAttemptStore,
    attempt: CodingExecutionAttempt,
) -> ApiResult<CodingExecutionAttempt> {
    if !attempt.status.is_active() {
        return Ok(attempt);
    }
    coding_store
        .update_attempt_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingAttemptStatus::Aborted,
        )
        .map_err(product_store_api_error)
}

pub(crate) async fn cleanup_coding_attempt_workspace(
    repository: &RepositoryRecord,
    attempt: &CodingExecutionAttempt,
) -> ApiResult<()> {
    let git = GitWorkspaceService::new();
    if let Some(worktree_path) = attempt.worktree_path.as_ref() {
        git.remove_worktree(&repository.path, worktree_path)
            .await
            .map_err(git_workspace_api_error)?;
    }
    git.prune_worktrees(&repository.path)
        .await
        .map_err(git_workspace_api_error)?;
    git.delete_local_branch(&repository.path, &attempt.branch_name)
        .await
        .map_err(git_workspace_api_error)?;
    Ok(())
}

pub(crate) fn git_workspace_api_error(error: GitWorkspaceError) -> ApiError {
    ApiError::runtime(
        "git_workspace_cleanup_failed",
        "git workspace cleanup failed",
        json!({"details": error.to_string()}),
    )
}

pub(crate) fn coding_workspace_engine_with_dummy_events(
    store: CodingAttemptStore,
) -> CodingWorkspaceEngine {
    let (event_tx, _event_rx) = tokio::sync::mpsc::channel(8);
    CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), event_tx)
}

pub(crate) fn coding_workspace_api_error(error: CodingWorkspaceEngineError) -> ApiError {
    let error_message = error.to_string();
    if error_message.contains("shared_worktree_dirty_manual_gate") {
        return ApiError::runtime(
            "shared_worktree_dirty_manual_gate",
            "shared worktree has uncommitted changes; manual cleanup required",
            json!({"details": error_message}),
        );
    }
    ApiError::runtime(
        "coding_workspace_engine_failed",
        "coding workspace engine operation failed",
        json!({"details": error_message}),
    )
}

pub(crate) fn git_workspace_diff_api_error(error: GitWorkspaceError) -> ApiError {
    ApiError::runtime(
        "git_workspace_diff_failed",
        "git workspace diff failed",
        json!({"details": error.to_string()}),
    )
}

pub(crate) fn is_git_repo(path: &StdPath) -> bool {
    Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .current_dir(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

pub(crate) fn current_git_branch(path: &StdPath) -> Option<String> {
    let output = Command::new("git")
        .args(["branch", "--show-current"])
        .current_dir(path)
        .stdin(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!branch.is_empty()).then_some(branch)
}
