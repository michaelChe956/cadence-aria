use super::dto::*;
use super::support::*;
use super::*;
use crate::product::coding_models::CodingAttemptScope;
use crate::web::state::CodingAttemptRunKey;

mod group;
mod scope;

pub use group::create_group_coding_attempt;
use scope::{CodingAttemptArtifactRoutePath, CodingAttemptRoutePath, resolve_coding_attempt};

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
    let active_attempts = coding_store
        .list_attempts_for_work_item(&project_id, &issue_id, &work_item.id)
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
        lifecycle
            .bind_issue_worktree_lock_to_attempt(
                &project_id,
                &issue_id,
                &work_item.id,
                &active_attempt.id,
            )
            .map_err(product_store_api_error)?;
        return Err(ApiError::runtime(
            "coding_attempt_active",
            "work item already has an active coding attempt",
            json!({ "attempt_id": active_attempt.id }),
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
    let issue_worktree_lease_id = format!("issue_worktree_lease_{}", uuid::Uuid::new_v4().simple());
    let issue_worktree_lease = lifecycle
        .try_acquire_issue_worktree_lock(
            &project_id,
            &issue_id,
            &work_item_id,
            &issue_worktree_lease_id,
        )
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
    state
        .test_controls
        .pause_coding_attempt_after_worktree_acquire_if_configured()
        .await;

    let provider_config_snapshot = match coding_provider_config_snapshot(
        &lifecycle,
        work_item,
        &repository.default_provider_mode,
        &*state.provider_availability,
    ) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            if issue_worktree_lease.acquired {
                let _ = lifecycle.release_issue_worktree_lock(
                    &project_id,
                    &issue_id,
                    &work_item_id,
                    &issue_worktree_lease.lease_id,
                );
            }
            return Err(error);
        }
    };
    let attempt = match coding_store.create_attempt(CreateCodingAttemptInput {
        project_id: project_id.clone(),
        issue_id: issue_id.clone(),
        work_item_id: work_item.id.clone(),
        base_branch,
        branch_name,
        worktree_path: None,
        provider_config_snapshot,
        max_auto_rework: 2,
    }) {
        Ok(attempt) => attempt,
        Err(
            error @ ProductStoreError::Conflict {
                kind: "active_coding_attempt",
                ..
            },
        ) => return Err(product_store_api_error(error)),
        Err(error) => {
            if issue_worktree_lease.acquired {
                let _ = lifecycle.release_issue_worktree_lock(
                    &project_id,
                    &issue_id,
                    &work_item_id,
                    &issue_worktree_lease.lease_id,
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
    if let Err(error) = lifecycle.bind_issue_worktree_lock_to_attempt(
        &project_id,
        &issue_id,
        &work_item_id,
        &attempt.id,
    ) {
        let _ = coding_store.delete_attempt(&project_id, &issue_id, &attempt.id);
        if issue_worktree_lease.acquired {
            let _ = lifecycle.release_issue_worktree_lock(
                &project_id,
                &issue_id,
                &work_item_id,
                &issue_worktree_lease.lease_id,
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

pub(crate) async fn get_coding_attempt(
    State(state): State<WebAppState>,
    Path(path): Path<CodingAttemptRoutePath>,
) -> ApiResult<Json<CodingAttemptSnapshotResponse>> {
    let app_paths = product_app_paths(&state);
    let coding_store = CodingAttemptStore::new(app_paths);
    let attempt = resolve_coding_attempt(
        &coding_store,
        path.project_id.as_deref(),
        path.issue_id.as_deref(),
        &path.attempt_id,
    )?;
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
        work_item_execution_plan,
        work_item_handoff,
    }))
}

pub(crate) async fn coding_attempt_diff(
    State(state): State<WebAppState>,
    Path(path): Path<CodingAttemptRoutePath>,
) -> ApiResult<Json<CodingAttemptDiffResponse>> {
    let app_paths = product_app_paths(&state);
    let coding_store = CodingAttemptStore::new(app_paths);
    let attempt = resolve_coding_attempt(
        &coding_store,
        path.project_id.as_deref(),
        path.issue_id.as_deref(),
        &path.attempt_id,
    )?;
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

pub(crate) async fn abort_coding_attempt(
    State(state): State<WebAppState>,
    Path(path): Path<CodingAttemptRoutePath>,
) -> ApiResult<Json<CodingAttemptDto>> {
    let app_paths = product_app_paths(&state);
    let coding_store = CodingAttemptStore::new(app_paths.clone());
    let attempt = resolve_coding_attempt(
        &coding_store,
        path.project_id.as_deref(),
        path.issue_id.as_deref(),
        &path.attempt_id,
    )?;
    state
        .coding_runs
        .abort_attempt(&CodingAttemptRunKey::from_attempt(&attempt))
        .await;
    let engine = coding_workspace_engine_with_dummy_events(coding_store);
    let aborted = engine
        .handle_abort(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .await
        .map_err(coding_workspace_api_error)?;
    Ok(Json(coding_attempt_dto(&aborted)))
}

pub(crate) async fn delete_coding_attempt(
    State(state): State<WebAppState>,
    Path(path): Path<CodingAttemptRoutePath>,
) -> ApiResult<Response> {
    let app_paths = product_app_paths(&state);
    let coding_store = CodingAttemptStore::new(app_paths.clone());
    let lifecycle = LifecycleStore::new(app_paths.clone());
    let attempt = resolve_coding_attempt(
        &coding_store,
        path.project_id.as_deref(),
        path.issue_id.as_deref(),
        &path.attempt_id,
    )?;
    state
        .coding_runs
        .abort_attempt(&CodingAttemptRunKey::from_attempt(&attempt))
        .await;
    let active_work_item_id = attempt
        .current_work_item_id
        .as_deref()
        .unwrap_or(&attempt.work_item_id);
    let work_item = lifecycle
        .list_work_items(&attempt.project_id, &attempt.issue_id)
        .map_err(product_store_api_error)?
        .into_iter()
        .find(|work_item| work_item.id == active_work_item_id)
        .ok_or_else(|| {
            product_store_api_error(ProductStoreError::NotFound {
                kind: "work_item",
                id: active_work_item_id.to_string(),
            })
        })?;
    let repository = find_repository(&app_paths, &attempt.project_id, &work_item.repository_id)?;

    if let Ok(Some(shared)) =
        lifecycle.get_issue_shared_worktree(&attempt.project_id, &attempt.issue_id)
        && shared.current_active_work_item_id.as_deref() == Some(active_work_item_id)
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

pub(crate) async fn confirm_work_item_execution_plan(
    State(state): State<WebAppState>,
    Path(path): Path<CodingAttemptRoutePath>,
) -> ApiResult<Json<WorkItemExecutionPlan>> {
    let app_paths = product_app_paths(&state);
    let coding_store = CodingAttemptStore::new(app_paths.clone());
    let lifecycle = LifecycleStore::new(app_paths);
    let attempt = resolve_coding_attempt(
        &coding_store,
        path.project_id.as_deref(),
        path.issue_id.as_deref(),
        &path.attempt_id,
    )?;

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

pub(crate) async fn request_work_item_execution_plan_change(
    State(state): State<WebAppState>,
    Path(path): Path<CodingAttemptRoutePath>,
    Json(payload): Json<RequestExecutionPlanChangeRequest>,
) -> ApiResult<Json<WorkItemExecutionPlan>> {
    let app_paths = product_app_paths(&state);
    let coding_store = CodingAttemptStore::new(app_paths.clone());
    let lifecycle = LifecycleStore::new(app_paths);
    let attempt = resolve_coding_attempt(
        &coding_store,
        path.project_id.as_deref(),
        path.issue_id.as_deref(),
        &path.attempt_id,
    )?;

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

pub(crate) async fn coding_attempt_artifact_content(
    State(state): State<WebAppState>,
    Path(path): Path<CodingAttemptArtifactRoutePath>,
) -> ApiResult<Json<ArtifactContentResponse>> {
    let artifact_id = path.artifact_id;
    validate_relative_id(&artifact_id)
        .map_err(|_| ApiError::validation("invalid_artifact_id", "invalid artifact id"))?;
    let app_paths = product_app_paths(&state);
    let coding_store = CodingAttemptStore::new(app_paths);
    let attempt = resolve_coding_attempt(
        &coding_store,
        path.project_id.as_deref(),
        path.issue_id.as_deref(),
        &path.attempt_id,
    )?;
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
