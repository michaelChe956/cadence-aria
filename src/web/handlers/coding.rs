use super::dto::*;
use super::support::*;
use super::*;
use crate::product::coding_attempt_repository::{
    SchemaV2GroupAttemptScopePolicy, resolve_coding_attempt_repository,
};
use crate::product::coding_attempt_store::AuthoritativeCodingUnitBinding;
use crate::product::coding_attempt_store::target_snapshot::build_attempt_target_snapshot;
use crate::product::coding_models::{AttemptTargetSnapshot, CodingAttemptScope};
use crate::product::logical_codebase::{
    LegacySharedWorktreeMigration, RepositoryRouting, RepositoryRoutingErrorCode,
};
use crate::product::work_item_revision_store::WorkItemRevisionStore;
use crate::web::coding_ws_handler::{coding_pending_gates, coding_role_run_snapshots};
use crate::web::state::CodingAttemptRunKey;

mod group;
mod scope;
mod worktree_route;

pub use group::create_group_coding_attempt;
use scope::{CodingAttemptArtifactRoutePath, CodingAttemptRoutePath, resolve_coding_attempt};
use worktree_route::{
    IssueWorktreeRoute, issue_worktree_active_api_error, release_worktree_lock,
    repo_worktree_active_api_error,
};

pub(crate) struct RuntimeBindingProviderConfigInput<'a> {
    pub project_id: &'a str,
    pub issue_id: &'a str,
    pub plan_id: &'a str,
    pub plan_revision_id: &'a str,
    pub unit: &'a AuthoritativeCodingUnitBinding,
    pub repository_default_provider: &'a str,
}

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
    let base_branch = current_git_branch(&repository.path).unwrap_or_else(|| "HEAD".to_string());
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

fn attempt_target_snapshot(
    app_paths: &ProductAppPaths,
    project_id: &str,
    issue_id: &str,
    work_item: &LifecycleWorkItemRecord,
) -> ApiResult<Option<AttemptTargetSnapshot>> {
    match RepositoryRouting::load_for_issue(app_paths, project_id, issue_id)
        .map_err(product_store_api_error)?
    {
        RepositoryRouting::Legacy { .. } => Ok(None),
        RepositoryRouting::Logical { .. } => {
            let logical_repository_id = work_item.target_repository_id.ok_or_else(|| {
                product_store_api_error(routing_error(
                    RepositoryRoutingErrorCode::TargetMissing,
                    format!("work item {} has no target repository", work_item.id),
                ))
            })?;
            build_attempt_target_snapshot(app_paths, project_id, logical_repository_id)
                .map(Some)
                .map_err(target_snapshot_api_error)
        }
        RepositoryRouting::FailClosed { code, reason } => {
            Err(product_store_api_error(routing_error(code, reason)))
        }
    }
}

fn target_snapshot_api_error(
    error: crate::product::coding_attempt_store::target_snapshot::TargetSnapshotError,
) -> ApiError {
    ApiError::runtime(
        "product_store_error",
        "coding attempt target snapshot capture failed",
        json!({ "reason": error.to_string() }),
    )
}

fn resolve_work_item_repository(
    app_paths: &ProductAppPaths,
    project_id: &str,
    work_item: &LifecycleWorkItemRecord,
) -> ApiResult<RepositoryRecord> {
    let store = RepositoryStore::new(app_paths.clone());
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
                .resolve_logical_repository_strict(project_id, logical_repository_id)
                .map(|(_, _, repository)| repository)
                .map_err(product_store_api_error)
        }
        RepositoryRouting::FailClosed { code, reason } => {
            Err(product_store_api_error(routing_error(code, reason)))
        }
    }
}

fn routing_error(code: RepositoryRoutingErrorCode, reason: impl Into<String>) -> ProductStoreError {
    let stable_code = code.stable_code();
    ProductStoreError::InvalidRecord {
        kind: "repository_routing",
        reason: format!("{stable_code}: {}", reason.into()),
    }
}

fn legacy_physical_repository(
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

fn resolve_attempt_repository(
    app_paths: &ProductAppPaths,
    attempt: &CodingExecutionAttempt,
) -> ApiResult<RepositoryRecord> {
    resolve_coding_attempt_repository(
        app_paths,
        attempt,
        SchemaV2GroupAttemptScopePolicy::RequireWorkItemGroupScope,
    )
    .map_err(product_store_api_error)
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
        .depends_on
        .iter()
        .filter_map(|dep_id| {
            all_work_items
                .iter()
                .find(|item| &item.id == dep_id)
                .map(|dep| WorkItemDependencyHandoffRef {
                    work_item_id: dep.id.clone(),
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

fn is_schema_v2_group_work_item(
    app_paths: &ProductAppPaths,
    lifecycle: &LifecycleStore,
    project_id: &str,
    issue_id: &str,
    work_item_id: &str,
) -> Result<bool, ProductStoreError> {
    let revision_store = WorkItemRevisionStore::new(app_paths.clone());
    for plan in lifecycle.list_issue_work_item_plans(project_id, issue_id)? {
        if plan.status != IssueWorkItemPlanStatus::Confirmed
            || !plan.work_item_ids.iter().any(|id| id == work_item_id)
        {
            continue;
        }
        let lineage = match revision_store.get_plan_lineage(project_id, issue_id, &plan.id) {
            Ok(lineage) => lineage,
            Err(ProductStoreError::NotFound {
                kind: "work_item_plan_lineage",
                ..
            }) => {
                continue;
            }
            Err(error) => return Err(error),
        };
        let Some(active_revision_id) = lineage.active_revision_id else {
            return Ok(true);
        };
        let revision = revision_store.get_plan_revision(
            project_id,
            issue_id,
            &plan.id,
            &active_revision_id,
        )?;
        if revision.work_item_bindings.contains_key(work_item_id) {
            return Ok(true);
        }
        return Err(ProductStoreError::IdentityMismatch {
            kind: "schema_v2_group_plan_binding",
            id: plan.id,
        });
    }
    Ok(false)
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
            permission_modes: session.permission_modes.clone(),
        });
    }

    let author =
        resolve_default_coding_provider(repository_default_provider, provider_availability)?
            .provider;
    Ok(ProviderConfigSnapshot {
        author: author.clone(),
        reviewer: Some(author),
        review_rounds: 1,
        permission_modes: WorkspaceRolePermissionModes::default(),
    })
}

pub(crate) fn coding_provider_config_snapshot_for_runtime_binding(
    lifecycle: &LifecycleStore,
    input: RuntimeBindingProviderConfigInput<'_>,
    provider_availability: &dyn Fn(&ProviderName) -> bool,
) -> ApiResult<ProviderConfigSnapshot> {
    let sessions = lifecycle
        .list_workspace_sessions(input.project_id, input.issue_id)
        .map_err(product_store_api_error)?;
    if let Some(session) = sessions.iter().rev().find(|session| {
        session.entity_id == input.unit.logical_work_item_id
            && session.workspace_type == WorkspaceType::WorkItem
            && session.status == WorkspaceSessionStatus::Confirmed
            && session
                .work_item_runtime_binding
                .as_ref()
                .is_some_and(|binding| {
                    binding.plan_id == input.plan_id
                        && binding.plan_revision_id == input.plan_revision_id
                        && binding.logical_work_item_id == input.unit.logical_work_item_id
                        && binding.work_item_revision_id == input.unit.work_item_revision_id
                        && binding.projection_bundle_id == input.unit.projection_bundle_id
                        && binding.verification_plan_revision_id
                            == input.unit.verification_plan_revision_id
                })
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
            permission_modes: session.permission_modes.clone(),
        });
    }

    let author =
        resolve_default_coding_provider(input.repository_default_provider, provider_availability)?
            .provider;
    Ok(ProviderConfigSnapshot {
        author: author.clone(),
        reviewer: Some(author),
        review_rounds: 1,
        permission_modes: WorkspaceRolePermissionModes::default(),
    })
}

fn coding_group_review_artifacts(
    coding_store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
) -> Result<Option<GroupReviewArtifactProjection>, ProductStoreError> {
    let projection = GroupReviewArtifactProjection {
        shard_reports: coding_store
            .list_group_review_shard_reports_for_attempt(attempt)?
            .into_iter()
            .map(|report| GroupReviewArtifactRef {
                id: report.id,
                raw_provider_output_refs: report.raw_provider_output_refs,
            })
            .collect(),
        reduction_reports: coding_store
            .list_group_review_reduction_reports_for_attempt(attempt)?
            .into_iter()
            .map(|report| GroupReviewArtifactRef {
                id: report.id,
                raw_provider_output_refs: report.raw_provider_output_refs,
            })
            .collect(),
    };
    let has_artifacts =
        !projection.shard_reports.is_empty() || !projection.reduction_reports.is_empty();
    Ok(has_artifacts.then_some(projection))
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
    let attempt = coding_store
        .reconcile_linked_plan_repair_pause(&attempt)
        .map_err(product_store_api_error)?
        .attempt;
    let timeline_nodes = coding_store
        .get_timeline_nodes(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .map_err(product_store_api_error)?;
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
    let group_final_readiness = coding_store
        .get_group_final_readiness_snapshot(&attempt)
        .map_err(product_store_api_error)?;
    let pending_choices = coding_store
        .list_open_choice_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .map_err(product_store_api_error)?;
    let role_runs =
        coding_role_run_snapshots(&coding_store, &attempt).map_err(product_store_api_error)?;
    let pending_gates =
        coding_pending_gates(&coding_store, &attempt).map_err(coding_workspace_api_error)?;
    let active_node_id = active_coding_timeline_node_id(&timeline_nodes);
    let group_review_artifacts =
        coding_group_review_artifacts(&coding_store, &attempt).map_err(product_store_api_error)?;
    let work_item_execution_plan = coding_store
        .get_work_item_execution_plan(&attempt.project_id, &attempt.issue_id, &attempt.id)
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

    let provider_config_snapshot = attempt.provider_config_snapshot.clone();

    Ok(Json(CodingAttemptSnapshotResponse {
        attempt: coding_attempt_dto(&coding_store, &attempt)?,
        attempt_scope: coding_attempt_scope_text(&attempt.scope).to_string(),
        work_item_group_id: attempt.work_item_group_id.clone(),
        current_work_item_id: attempt.current_work_item_id.clone(),
        active_unit_id: attempt.active_unit_id.clone(),
        units,
        provider_config_snapshot,
        timeline_nodes,
        active_node_id,
        code_review_reports,
        review_request,
        internal_pr_review,
        group_review_artifacts,
        group_final_readiness,
        pending_gates,
        pending_choices,
        role_runs,
        work_item_execution_plan,
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
    let attempt_key = CodingAttemptRunKey::from_attempt(&attempt);
    state.coding_runs.abort_attempt(&attempt_key).await;
    let _mutation_lease = state.coding_runs.lock_attempt_mutation(&attempt_key).await;
    let current = coding_store
        .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .map_err(product_store_api_error)?;
    let engine = coding_workspace_engine_with_dummy_events(coding_store.clone());
    let aborted = engine
        .handle_abort(&current.project_id, &current.issue_id, &current.id)
        .await
        .map_err(coding_workspace_api_error)?;
    Ok(Json(coding_attempt_dto(&coding_store, &aborted)?))
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
    let attempt_key = CodingAttemptRunKey::from_attempt(&attempt);
    state.coding_runs.abort_attempt(&attempt_key).await;
    let _mutation_lease = state.coding_runs.lock_attempt_mutation(&attempt_key).await;
    let attempt = coding_store
        .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .map_err(product_store_api_error)?;
    let attempt = coding_workspace_engine_with_dummy_events(coding_store.clone())
        .reconcile_coding_git_operation_for_termination(&attempt)
        .await
        .map_err(coding_workspace_api_error)?;
    let _group_initialization_guard = if matches!(attempt.scope, CodingAttemptScope::WorkItemGroup)
    {
        Some(
            coding_store
                .acquire_group_initialization_arbitration_async(
                    &attempt.project_id,
                    &attempt.issue_id,
                )
                .await
                .map_err(product_store_api_error)?,
        )
    } else {
        None
    };
    let active_work_item_id = attempt
        .current_work_item_id
        .as_deref()
        .unwrap_or(&attempt.work_item_id);
    let repository = resolve_attempt_repository(&app_paths, &attempt)?;

    if let Ok(Some(shared)) =
        lifecycle.get_issue_shared_worktree(&attempt.project_id, &attempt.issue_id)
        && shared.current_active_work_item_id.as_deref() == Some(active_work_item_id)
    {
        let engine = coding_workspace_engine_with_dummy_events(coding_store.clone());
        engine
            .handle_delete_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .await
            .map_err(coding_workspace_api_error)?;
    } else if let Ok(Some(shared)) =
        lifecycle.get_issue_shared_worktree(&attempt.project_id, &attempt.issue_id)
    {
        // active work item 不匹配（failed attempt 的 current_work_item_id 可能已变
        // 或已清空），但锁仍可能由本 attempt 持有：按 owner 幂等释放，
        // 避免残留孤儿锁阻塞后续 attempt。
        if shared.current_lock_owner_id.as_deref() == Some(attempt.id.as_str()) {
            let _ = lifecycle.release_issue_worktree_lock_by_owner(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
            );
        }
    }

    cleanup_coding_attempt_workspace(&repository, &attempt).await?;
    cleanup_attempt_handoff_revisions(&app_paths, &coding_store, &attempt)
        .map_err(product_store_api_error)?;
    finalize_coding_attempt_deletion(&coding_store, &app_paths, &attempt)?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// 删除 attempt 时清理该 attempt 各 unit 已认领的 handoff revision。
/// 清理在 attempt 记录删除之前执行（依赖 unit 指针可读）。归属校验由
/// `delete_handoff_revision` 负责；删除阶段文件缺失视为已清理（幂等），
/// 其他错误上抛中断删除流程（失败关闭，不静默留不一致状态）。
///
/// attempt 未进入 plan 阶段（无 plan binding）时必然无 unit、无 handoff
/// 产出，清理视为空操作返回；这与 `delete_handoff_revision` 对 handoff
/// 档案 NotFound 的容忍语义一致，不构成静默吞错。
fn cleanup_attempt_handoff_revisions(
    app_paths: &ProductAppPaths,
    coding_store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
) -> Result<(), ProductStoreError> {
    // 无 unit 认领 handoff 时清理为空操作；先取 units，避免在无 plan
    // binding 的 attempt 上强制要求 binding 存在。
    let units =
        coding_store.list_coding_units(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
    let target_units = units
        .iter()
        .filter(|unit| unit.latest_handoff_revision_id.is_some())
        .collect::<Vec<_>>();
    if target_units.is_empty() {
        return Ok(());
    }
    let binding = coding_store.get_plan_binding(attempt)?;
    let lineage = WorkItemRevisionStore::new(app_paths.clone()).get_plan_lineage(
        &attempt.project_id,
        &attempt.issue_id,
        &binding.plan_id,
    )?;
    let revision_store = WorkItemRevisionStore::new(app_paths.clone());
    for unit in target_units {
        let handoff_id = unit
            .latest_handoff_revision_id
            .as_deref()
            .expect("filtered units have handoff revision id");
        // 归属校验 + 删除交由 delete_handoff_revision；`?` 传播错误
        // （Ok(()) 返回值无意义故不绑定）。NotFound 仅在 remove_file 阶段
        // 容忍，get 阶段 NotFound 会传播（指针指向不存在档案说明状态
        // 不一致，按失败关闭处理）。
        revision_store.delete_handoff_revision(&lineage, &unit.logical_work_item_id, handoff_id)?;
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::product::logical_codebase::{LogicalCodebaseManifest, LogicalCodebaseStore};
    use crate::product::models::{
        RepositoryRecord, WorkItemContextBudget, WorkItemExecutionPlanStatus, WorkItemKind,
        WorkItemPlanStatus, WorkItemStatus,
    };
    use uuid::Uuid;

    #[test]
    fn resolve_work_item_repository_manifest_with_bad_target_is_fail_closed() {
        // 有 manifest、work_item target 指向不存在成员 → blocker，不回退物理仓库。
        let root = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(root.path().join(".aria"));
        write_physical_repository_fixture(&paths, root.path());
        LogicalCodebaseStore::new(paths.clone())
            .save_manifest(
                "project_0001",
                &LogicalCodebaseManifest::new(
                    "project_0001",
                    root.path().join("aggregate-root"),
                    Vec::new(),
                ),
            )
            .unwrap();
        let work_item =
            lifecycle_work_item_with_target_fixture("00000000-0000-0000-0000-000000000000");

        let result = resolve_work_item_repository(&paths, "project_0001", &work_item);

        assert!(result.is_err());
    }

    fn write_physical_repository_fixture(paths: &ProductAppPaths, root: &std::path::Path) {
        crate::product::json_store::write_json(
            &paths.project_root("project_0001").join("repos.json"),
            &[RepositoryRecord {
                id: "repository_0001".to_string(),
                project_id: "project_0001".to_string(),
                name: "物理仓库".to_string(),
                path: root.join("repository_0001"),
                repo_hash: "sha256:repository".to_string(),
                runtime_root: root.join("repository_0001/.aria/runtime"),
                default_policy_preset: "manual-write".to_string(),
                default_provider_mode: "fake".to_string(),
                created_at: "2026-08-11T00:00:00Z".to_string(),
                logical_repository_id: None,
                primary_checkout_id: None,
                identity_schema_version: 1,
                updated_at: "2026-08-11T00:00:00Z".to_string(),
            }],
        )
        .unwrap();
    }

    fn lifecycle_work_item_with_target_fixture(
        target_repository_id: &str,
    ) -> LifecycleWorkItemRecord {
        LifecycleWorkItemRecord {
            id: "work_item_0001".to_string(),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            target_repository_id: Some(crate::product::logical_codebase::LogicalRepositoryId(
                Uuid::parse_str(target_repository_id).unwrap(),
            )),
            story_spec_ids: Vec::new(),
            design_spec_ids: Vec::new(),
            title: "工作项".to_string(),
            plan_status: WorkItemPlanStatus::Confirmed,
            execution_status: WorkItemStatus::Pending,
            worktree_path: None,
            work_item_set_id: None,
            source_work_item_plan_id: None,
            source_outline_id: None,
            source_draft_id: None,
            planned_implementation_context: None,
            kind: WorkItemKind::default(),
            sequence_hint: None,
            depends_on: Vec::new(),
            exclusive_write_scopes: Vec::new(),
            forbidden_write_scopes: Vec::new(),
            context_budget: WorkItemContextBudget::default(),
            verification_plan_ref: None,
            require_execution_plan_confirm: false,
            execution_plan_status: WorkItemExecutionPlanStatus::NotStarted,
            completion_commit: None,
            completion_diff_summary_ref: None,
            created_at: "2026-08-11T00:00:00Z".to_string(),
            updated_at: "2026-08-11T00:00:00Z".to_string(),
        }
    }
}
