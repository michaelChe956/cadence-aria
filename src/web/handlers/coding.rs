use super::dto::*;
use super::product_resources::issue_baseline_api_error;
use super::support::*;
use super::*;
use crate::product::coding_attempt_repository::{
    SchemaV2GroupAttemptScopePolicy, resolve_coding_attempt_repository,
};
use crate::product::coding_attempt_store::AuthoritativeCodingUnitBinding;
use crate::product::coding_attempt_store::target_snapshot::build_attempt_target_snapshot;
use crate::product::coding_models::{AttemptTargetSnapshot, CodingAttemptScope};
use crate::product::issue_store::IssueStore;
use crate::product::logical_codebase::{
    LegacySharedWorktreeMigration, RepositoryRouting, RepositoryRoutingErrorCode,
};
use crate::product::work_item_revision_store::WorkItemRevisionStore;
use crate::web::coding_ws_handler::{coding_pending_gates, coding_role_run_snapshots};
use crate::web::state::CodingAttemptRunKey;
mod advance;
mod group;
mod progress;
pub(crate) mod repository_resolution;
pub(crate) mod scope;
mod verification_surface;

pub(crate) use verification_surface::{
    get_coding_policy_text, get_verification_command_evidence,
    get_verification_triage_records, post_coding_policy_reauthorization,
    post_rerun_planned_command, post_verification_triage_decision,
    post_verification_triage_enter,
};
mod worktree_route;
#[allow(unused_imports)]
pub(crate) use advance::map_advance_outcome;
pub use group::create_group_coding_attempt;
#[cfg(test)]
#[path = "coding/advance_tests.rs"]
mod advance_tests;
pub(crate) use progress::build_group_work_item_progress;
use repository_resolution::{resolve_work_item_repository, routing_error};
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

/// REQ-PIB-03（T3.1）：coding fork 入口的共享基线解析——读 issue.base_branch 并经
/// `resolve_effective_base_branch`（三面同源唯一解析链，与创建校验、author 上下文
/// 基线同一解析点）：显式锁定分支须本地存在，存量 None 走默认链 main→master；
/// 不可解析（分支被删/皆无/仓库不可用）→ 422 fail-closed，不回退当前检出或
/// HEAD，防止 coder worktree 分叉点与 author 所见/C1 核对树错位。单件
/// （create_coding_attempt）与组（create_group_coding_attempt）两入口共用。
pub(crate) fn fork_base_branch_from_issue(
    app_paths: &ProductAppPaths,
    repository_path: &StdPath,
    project_id: &str,
    issue_id: &str,
) -> ApiResult<String> {
    let issue = IssueStore::new(app_paths.clone())
        .get(project_id, issue_id)
        .map_err(product_store_api_error)?;
    crate::product::issue_baseline::resolve_effective_base_branch(
        repository_path,
        issue.base_branch.as_deref(),
    )
    .map_err(issue_baseline_api_error)
}

fn attempt_target_snapshot(
    app_paths: &ProductAppPaths,
    project_id: &str,
    issue_id: &str,
    work_item: &LifecycleWorkItemRecord,
) -> ApiResult<Option<AttemptTargetSnapshot>> {
    // v1.3：按 issue 所属 lc_id 寻址（R9）；单仓/无 LC 回退 legacy project 级路径。
    let lc_id = crate::product::logical_codebase::resolve_issue_logical_codebase_id(
        app_paths, project_id, issue_id,
    )
    .map_err(product_store_api_error)?;
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
            build_attempt_target_snapshot(
                app_paths,
                project_id,
                logical_repository_id,
                lc_id.as_deref(),
            )
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
        // C2 Task 5（REQ-CRO-05）：reviewer 三值透传——Confirmed WorkItem 会话缺
        // reviewer 时保持空 effective（coding 侧 review 阶段落缺配置门）。
        let reviewer = session
            .reviewer_provider
            .as_ref()
            .map(|reviewer| resolve_explicit_provider_name(provider_name_key(reviewer), provider_availability))
            .transpose()?
            .map(|resolved| resolved.provider);
        return Ok(ProviderConfigSnapshot {
            author,
            reviewer,
            review_rounds: session.review_rounds,
            permission_modes: session.permission_modes.clone(),
        });
    }

    let author =
        resolve_default_coding_provider(repository_default_provider, provider_availability)?
            .provider;
    // C2 Task 5（REQ-CRO-05）：无会话可引用时不再以 author 顶替 reviewer——
    // 空 effective，coding 侧 review 阶段落缺配置门等待用户配置。
    Ok(ProviderConfigSnapshot {
        author: author.clone(),
        reviewer: None,
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
        // C2 Task 5（REQ-CRO-05）：reviewer 三值透传——Confirmed WorkItem 会话缺
        // reviewer 时保持空 effective（coding 侧 review 阶段落缺配置门）。
        let reviewer = session
            .reviewer_provider
            .as_ref()
            .map(|reviewer| resolve_explicit_provider_name(provider_name_key(reviewer), provider_availability))
            .transpose()?
            .map(|resolved| resolved.provider);
        return Ok(ProviderConfigSnapshot {
            author,
            reviewer,
            review_rounds: session.review_rounds,
            permission_modes: session.permission_modes.clone(),
        });
    }

    let author =
        resolve_default_coding_provider(input.repository_default_provider, provider_availability)?
            .provider;
    // C2 Task 5（REQ-CRO-05）：同上——不回填 author。
    Ok(ProviderConfigSnapshot {
        author: author.clone(),
        reviewer: None,
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
    let mut pending_choices = coding_store
        .list_open_choice_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .map_err(product_store_api_error)?;
    // P0 1.3（REQ-WIGA-05）Task 11：冷驾驶舱 REST 作答依赖快照携带应答绑定
    // 的 run 化身（claim 要求精确匹配 active_run_incarnation）；无唯一活跃
    // run 时不注入——消费者不得猜 run。durable 记录恒不带该字段。
    if let Some(incarnation) = state
        .coding_runs
        .active_run_incarnation(&CodingAttemptRunKey::from_attempt(&attempt))
    {
        for gate in &mut pending_choices {
            gate.expected_run_id = Some(incarnation.clone());
        }
    }
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
    let (group_coding_progress, group_progress) =
        if matches!(attempt.scope, CodingAttemptScope::WorkItemGroup) {
            let (progress, aggregate) = build_group_work_item_progress(&coding_store, &attempt)
                .map_err(product_store_api_error)?;
            (Some(progress), Some(aggregate))
        } else {
            (None, None)
        };

    let provider_config_snapshot = attempt.provider_config_snapshot.clone();

    Ok(Json(CodingAttemptSnapshotResponse {
        attempt: coding_attempt_dto(&coding_store, &attempt)?,
        attempt_scope: coding_attempt_scope_text(&attempt.scope).to_string(),
        work_item_group_id: attempt.work_item_group_id.clone(),
        current_work_item_id: attempt.current_work_item_id.clone(),
        active_unit_id: attempt.active_unit_id.clone(),
        units,
        group_coding_progress,
        group_progress,
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

/// C2 Task 3（REQ-CRO-03）：abort 后显式 restart 的 REST 薄入口——与 WS
/// `RestartCoding` 同一应用服务语义（命令账本→版本/终态门→租约非活跃→
/// 清退役标记→重开 admission→spawn 新 runner）。状态映射：
/// Accepted／Replayed→200，NeedsHuman（停等）→202，Rejected（版本／终态
/// 不符）→409；错对象（body attempt_id 与路径不符）→400。
pub(crate) async fn restart_coding_attempt(
    State(state): State<WebAppState>,
    Path(path): Path<CodingAttemptRoutePath>,
    Json(request): Json<crate::web::coding_start::RestartCodingAttemptRequest>,
) -> ApiResult<(axum::http::StatusCode, Json<crate::web::coding_start::RestartCodingAttemptResult>)> {
    use axum::http::StatusCode;
    use crate::product::models::automation::OperationState;

    let app_paths = product_app_paths(&state);
    let coding_store = CodingAttemptStore::new(app_paths.clone());
    let attempt = resolve_coding_attempt(
        &coding_store,
        path.project_id.as_deref(),
        path.issue_id.as_deref(),
        &path.attempt_id,
    )?;
    if request.attempt_id != attempt.id {
        return Err(ApiError::validation(
            "coding_restart_attempt_mismatch",
            "request attempt_id does not match the routed coding attempt",
        ));
    }
    let result = crate::web::coding_start::restart_coding_attempt(
        &state,
        &attempt.project_id,
        &attempt.issue_id,
        request,
    )
    .await
    .map_err(|error| {
        ApiError::runtime(
            "coding_restart_failed",
            "coding attempt restart failed",
            serde_json::json!({ "code": error.code(), "message": error.message() }),
        )
    })?;
    let code = match result.state {
        OperationState::NeedsHuman => StatusCode::ACCEPTED,
        OperationState::Rejected => StatusCode::CONFLICT,
        _ => StatusCode::OK,
    };
    Ok((code, Json(result)))
}

/// C2 Task 12（REQ-CRO-06）：驾驶舱 gate response REST 请求——与 WS
/// `GateResponse` 同一应用服务（`handle_blocked_gate_response`），外加
/// REST 契约的稳定 `command_id`＋expected 版本（同 command 同 payload
/// 重放首次 durable 结果，异 payload fail-closed；旧版本 Rejected
/// "请刷新"，不启动 provider）。
#[derive(Debug, serde::Deserialize)]
pub struct CodingGateResponseRestRequest {
    pub command_id: String,
    pub gate_id: String,
    pub action_id: String,
    #[serde(default)]
    pub extra_context: Option<String>,
    pub expected_version: u64,
}

/// C2 Task 12：gate response REST 结果（状态语义同 restart：Accepted／
/// Replayed→200，NeedsHuman→202，Rejected→409）。
#[derive(Debug, serde::Serialize)]
pub struct CodingGateResponseRestResult {
    pub command_id: String,
    pub state: crate::product::models::automation::OperationState,
    pub attempt_id: String,
    pub gate_id: String,
    pub action_id: String,
    pub reason: Option<String>,
}

/// C2 Task 12：POST /coding-attempts/{attempt_id}/gate-responses——无
/// coding socket 也能作答（复用 C1 "coding choice 从驾驶舱按 attempt
/// 地址经 REST 作答"模式）；`manual_continue` 等白名单动作在 durable
/// 落门后按 WS 同款续跑判定唤回 runner。
pub(crate) async fn post_coding_gate_response(
    State(state): State<WebAppState>,
    Path(path): Path<CodingAttemptRoutePath>,
    Json(request): Json<CodingGateResponseRestRequest>,
) -> ApiResult<(
    axum::http::StatusCode,
    Json<CodingGateResponseRestResult>,
)> {
    use crate::product::coding_attempt_store::CodingAttemptCommandRecord;
    use crate::product::json_store::validate_relative_id;
    use crate::product::models::automation::OperationState;
    use crate::web::coding_ws_handler::{
        should_resume_runner_after_gate_response, spawn_coding_runner,
    };
    use axum::http::StatusCode;

    let app_paths = product_app_paths(&state);
    let coding_store = CodingAttemptStore::new(app_paths.clone());
    let attempt = resolve_coding_attempt(
        &coding_store,
        path.project_id.as_deref(),
        path.issue_id.as_deref(),
        &path.attempt_id,
    )?;
    validate_relative_id(&request.command_id).map_err(|error| {
        ApiError::validation(
            "coding_gate_response_invalid_command_id",
            format!("invalid gate response command id: {error}"),
        )
    })?;
    if request.gate_id.trim().is_empty() || request.action_id.trim().is_empty() {
        return Err(ApiError::validation(
            "coding_gate_response_invalid_identity",
            "gate_id and action_id must not be blank",
        ));
    }

    // 命令账本（Task 2）：payload digest 覆盖对象身份＋gate／action／版本
    //＋extra_context（同 command 异 payload fail-closed"请刷新"）。
    let extra_digest = {
        use sha2::Digest;
        let mut hasher = sha2::Sha256::new();
        hasher.update(request.extra_context.as_deref().unwrap_or("").as_bytes());
        hex::encode(hasher.finalize())
    };
    let payload_digest = format!(
        "gate-response|{}|{}|{}|{}|{}|{}|{extra_digest}",
        attempt.project_id,
        attempt.issue_id,
        attempt.id,
        request.gate_id,
        request.action_id,
        request.expected_version
    );
    let record = |cmd_state: OperationState| CodingAttemptCommandRecord {
        command_id: request.command_id.clone(),
        payload_digest: payload_digest.clone(),
        state: cmd_state,
        recorded_at: chrono::Utc::now().to_rfc3339(),
    };
    let result = |cmd_state: OperationState, reason: Option<String>| {
        CodingGateResponseRestResult {
            command_id: request.command_id.clone(),
            state: cmd_state,
            attempt_id: attempt.id.clone(),
            gate_id: request.gate_id.clone(),
            action_id: request.action_id.clone(),
            reason,
        }
    };
    let store_error = |error: crate::product::json_store::ProductStoreError| {
        ApiError::runtime(
            "coding_gate_response_ledger_failed",
            "coding gate response command ledger failed",
            serde_json::json!({ "details": error.to_string() }),
        )
    };

    if let Some(existing) = coding_store
        .find_attempt_command_result(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &request.command_id,
        )
        .map_err(store_error)?
    {
        if existing.payload_digest != payload_digest {
            return Err(ApiError::runtime(
                "coding_gate_response_command_conflict",
                "同 command 异 payload，请刷新后重试",
                serde_json::json!({}),
            ));
        }
        // 同 command 同 payload：重放首次 durable 结果，不重复副作用。
        let cmd_state = if existing.state == OperationState::Accepted {
            OperationState::Replayed
        } else {
            existing.state
        };
        return Ok((StatusCode::OK, Json(result(cmd_state, None))));
    }

    // 版本门（旧页面"请刷新"，不启动 provider、不改 attempt）。
    if attempt.version != request.expected_version {
        let _ = coding_store
            .append_attempt_command_result(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                &record(OperationState::Rejected),
            )
            .map_err(store_error)?;
        return Ok((
            StatusCode::CONFLICT,
            Json(result(
                OperationState::Rejected,
                Some(format!(
                    "expected version {} but durable version is {}; 请刷新",
                    request.expected_version, attempt.version
                )),
            )),
        ));
    }

    // 与 WS 同一应用服务（mutation lease 下 durable 落门）。
    let attempt_key = CodingAttemptRunKey::from_attempt(&attempt);
    let mutation_lease = state.coding_runs.lock_attempt_mutation(&attempt_key).await;
    let engine = coding_workspace_engine_with_dummy_events(coding_store.clone());
    let updated = match engine
        .handle_blocked_gate_response(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &request.gate_id,
            &request.action_id,
            request.extra_context.clone(),
        )
        .await
    {
        Ok(updated) => updated,
        Err(error) => {
            drop(mutation_lease);
            return Err(ApiError::runtime(
                "coding_gate_response_failed",
                "coding gate response failed",
                serde_json::json!({ "details": error.to_string() }),
            ));
        }
    };
    // WS 同款续跑判定：白名单动作且门后 attempt 回到 Running 时唤回
    // runner（观察通道缺失不阻塞业务事实——C2 Task 1 语义）。
    if should_resume_runner_after_gate_response(&request.action_id, &attempt)
        && updated.status == crate::product::coding_models::CodingAttemptStatus::Running
    {
        let (event_tx, _event_rx) = tokio::sync::mpsc::channel(64);
        drop(_event_rx);
        let _ = spawn_coding_runner(
            state.clone(),
            coding_store.clone(),
            event_tx,
            updated.clone(),
        );
    }
    drop(mutation_lease);
    coding_store
        .append_attempt_command_result(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &record(OperationState::Accepted),
        )
        .map_err(store_error)?;
    Ok((StatusCode::OK, Json(result(OperationState::Accepted, None))))
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

mod artifact_content;
pub(crate) use artifact_content::coding_attempt_artifact_content;

/// C2 Task 12（REQ-CRO-06）：gate-responses REST 契约——同 command 同
/// payload 重放首次 durable 结果（不重复副作用）、同 command 异 payload
/// fail-closed"请刷新"、旧版本 Rejected 不改 attempt 不启动 provider；
/// 与 WS `GateResponse` 同一应用服务（`handle_blocked_gate_response`）。
#[cfg(test)]
mod c2_gate_response_rest_tests {
    use axum::extract::{Path, State};
    use axum::Json;

    use crate::product::app_paths::ProductAppPaths;
    use crate::product::coding_attempt_store::CodingAttemptStore;
    use crate::product::coding_attempt_store::CreateCodingAttemptInput;
    use crate::product::issue_store::{CreateProductIssueInput, IssueStore};
    use crate::product::models::automation::OperationState;
    use crate::web::handlers::coding::scope::CodingAttemptRoutePath;
    use crate::web::handlers::coding::{
        CodingGateResponseRestRequest, post_coding_gate_response,
    };
    use crate::web::state::WebAppState;
    use crate::web::workspace_ws_types::ProviderConfigSnapshot;

    async fn fixture() -> (tempfile::TempDir, WebAppState, CodingAttemptStore, crate::product::coding_models::CodingExecutionAttempt) {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path().to_path_buf();
        let state = WebAppState::new(
            root.clone(),
            crate::web::runtime::WebRuntime::new_fake(root.clone()),
        );
        let paths = ProductAppPaths::new(root.join(".aria"));
        IssueStore::new(paths.clone())
            .create(CreateProductIssueInput {
                project_id: "project_0001".to_string(),
                repo_id: Some("repository_0001".to_string()),
                logical_codebase_id: None,
                title: "gate response issue".to_string(),
                description: None,
                change_id: None,
                base_branch: None,
            })
            .expect("issue");
        let store = CodingAttemptStore::new(paths.clone());
        let attempt = store
            .create_attempt(CreateCodingAttemptInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                work_item_id: "work_item_0001".to_string(),
                base_branch: "main".to_string(),
                branch_name: "aria/c2-gate-response".to_string(),
                worktree_path: None,
                provider_config_snapshot: ProviderConfigSnapshot {
                    author: crate::product::models::ProviderName::Fake,
                    reviewer: None,
                    review_rounds: 0,
                    permission_modes: Default::default(),
                },
                target_snapshot: None,
                max_auto_rework: 0,
            })
            .expect("attempt");
        (temp, state, store, attempt)
    }

    fn request(
        command_id: &str,
        gate_id: &str,
        action_id: &str,
        expected_version: u64,
        extra_context: Option<&str>,
    ) -> Json<CodingGateResponseRestRequest> {
        Json(CodingGateResponseRestRequest {
            command_id: command_id.to_string(),
            gate_id: gate_id.to_string(),
            action_id: action_id.to_string(),
            extra_context: extra_context.map(str::to_string),
            expected_version,
        })
    }

    fn route(attempt: &crate::product::coding_models::CodingExecutionAttempt) -> Path<CodingAttemptRoutePath> {
        Path(CodingAttemptRoutePath {
            project_id: Some(attempt.project_id.clone()),
            issue_id: Some(attempt.issue_id.clone()),
            attempt_id: attempt.id.clone(),
        })
    }

    #[tokio::test]
    async fn c2_gate_response_rest_replays_same_command_and_rejects_stale_version() {
        let (_tmp, state, store, attempt) = fixture().await;

        // 旧页面过期版本：Rejected"请刷新"，不改 attempt、不启动 provider。
        let (status, body) = post_coding_gate_response(
            State(state.clone()),
            route(&attempt),
            request("cmd-gate-0001", "gate_missing", "manual_continue", 99, None),
        )
        .await
        .expect("stale version handled as conflict body");
        assert_eq!(status, axum::http::StatusCode::CONFLICT);
        assert_eq!(body.state, OperationState::Rejected);
        assert!(body.reason.as_deref().unwrap_or("").contains("请刷新"));
        let after = store
            .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .unwrap();
        assert_eq!(after.version, attempt.version, "stale version must not mutate");
        assert_eq!(after.status, attempt.status, "stale version must not advance status");
        let ledger = store
            .find_attempt_command_result(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                "cmd-gate-0001",
            )
            .unwrap()
            .expect("rejected result recorded");
        assert_eq!(ledger.state, OperationState::Rejected);

        // 正确版本（门不存在 → 引擎按 WS 语义 no-op 返回 attempt）：
        // Accepted 落账，200。
        let (status, body) = post_coding_gate_response(
            State(state.clone()),
            route(&attempt),
            request("cmd-gate-0002", "gate_missing", "manual_continue", attempt.version, None),
        )
        .await
        .expect("accepted");
        assert_eq!(status, axum::http::StatusCode::OK);
        assert_eq!(body.state, OperationState::Accepted);

        // 同 command 同 payload：重放首次 durable 结果（Replayed），零副作用。
        let (status, replay) = post_coding_gate_response(
            State(state.clone()),
            route(&attempt),
            request("cmd-gate-0002", "gate_missing", "manual_continue", attempt.version, None),
        )
        .await
        .expect("replayed");
        assert_eq!(status, axum::http::StatusCode::OK);
        assert_eq!(replay.state, OperationState::Replayed);

        // 同 command 异 payload（extra_context 变更）：fail-closed"请刷新"。
        let conflict = post_coding_gate_response(
            State(state.clone()),
            route(&attempt),
            request(
                "cmd-gate-0002",
                "gate_missing",
                "manual_continue",
                attempt.version,
                Some("different payload"),
            ),
        )
        .await
        .expect_err("same command with different payload must fail closed");
        assert_eq!(
            conflict.code, "coding_gate_response_command_conflict",
            "conflict surfaces a refresh prompt, not a provider start"
        );
    }
}
