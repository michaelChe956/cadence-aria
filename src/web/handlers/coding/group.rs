use std::collections::BTreeSet;

use super::super::dto::*;
use super::super::support::*;
use super::super::*;
use super::{
    RuntimeBindingProviderConfigInput, coding_provider_config_snapshot_for_runtime_binding,
};
use crate::product::coding_attempt_store::{
    AuthoritativeGroupPlanBinding, CodingGroupInitializationPhase,
};
use crate::product::issue_store::IssueStore;
use crate::product::logical_codebase::{
    LogicalRepositoryId, RepositoryRouting, RepositoryRoutingErrorCode, SelectionPolicy,
};
use crate::product::work_item_revision_store::WorkItemRevisionStore;

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
    let pending_journal =
        match coding_store.get_group_initialization(&project_id, &issue_id, &plan_id) {
            Ok(journal) => Some(journal),
            Err(ProductStoreError::NotFound {
                kind: "coding_group_initialization_journal",
                ..
            }) => None,
            Err(error) => return Err(coding_group_attempt_incomplete_api_error(error)),
        };
    if let Some(journal) = pending_journal
        && journal.phase != CodingGroupInitializationPhase::Completed
    {
        let active_revision_id = WorkItemRevisionStore::new(app_paths.clone())
            .get_plan_lineage(&project_id, &issue_id, &plan_id)
            .map_err(product_store_api_error)?
            .active_revision_id;
        if active_revision_id.as_deref()
            != Some(journal.plan_binding.bound_plan_revision_id.as_str())
        {
            return Err(coding_group_attempt_incomplete_api_error(
                ProductStoreError::IdentityMismatch {
                    kind: "coding_group_initialization_plan_revision",
                    id: journal.attempt.id,
                },
            ));
        }
    }
    let authoritative = coding_store
        .resolve_authoritative_group_plan_binding(&project_id, &issue_id, &plan_id)
        .map_err(coding_plan_revision_binding_api_error)?;
    let current_unit = authoritative.units.first().ok_or_else(|| {
        coding_plan_revision_binding_api_error(ProductStoreError::IdentityMismatch {
            kind: "coding_group_order",
            id: plan_id.clone(),
        })
    })?;
    let repository = resolve_group_repository(&app_paths, &project_id, &issue_id, &authoritative)?;
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
    let provider_config_snapshot = coding_provider_config_snapshot_for_runtime_binding(
        &lifecycle,
        RuntimeBindingProviderConfigInput {
            project_id: &project_id,
            issue_id: &issue_id,
            plan_id: &plan_id,
            plan_revision_id: &authoritative.plan_revision_id,
            unit: current_unit,
            repository_default_provider: &repository.default_provider_mode,
        },
        &*state.provider_availability,
    )?;
    let initialization_input = CreateGroupCodingAttemptInput {
        project_id: project_id.clone(),
        issue_id: issue_id.clone(),
        plan_id: plan_id.clone(),
        current_work_item_id: current_unit.logical_work_item_id.clone(),
        base_branch: base_branch.clone(),
        branch_name: branch_name.clone(),
        worktree_path: None,
        provider_config_snapshot,
        max_auto_rework: 2,
    };

    let _initialization_guard = coding_store
        .acquire_group_initialization_arbitration_async(&project_id, &issue_id)
        .await
        .map_err(product_store_api_error)?;
    let creation_guard = coding_store
        .acquire_work_item_attempt_creation_async(
            &project_id,
            &issue_id,
            &current_unit.logical_work_item_id,
        )
        .await
        .map_err(product_store_api_error)?;
    let mut journal = coding_store
        .prepare_group_initialization(
            &initialization_input,
            &authoritative.plan_revision_id,
            &authoritative.units,
        )
        .map_err(group_initialization_api_error)?;
    if journal.phase == CodingGroupInitializationPhase::Completed {
        let existing = coding_store
            .get_attempt(&project_id, &issue_id, &journal.attempt.id)
            .map_err(coding_group_attempt_incomplete_api_error)?;
        coding_store
            .validate_group_attempt_integrity(&existing)
            .map_err(coding_group_attempt_incomplete_api_error)?;
        return Ok(Json(coding_attempt_dto(&existing)));
    }

    lifecycle
        .upsert_issue_shared_worktree(UpsertIssueSharedWorktreeInput {
            project_id: project_id.clone(),
            issue_id: issue_id.clone(),
            repository_id: repository.id.clone(),
            branch_name,
            worktree_path: shared_worktree_path,
            base_branch,
        })
        .map_err(product_store_api_error)?;
    let worktree_lease = lifecycle
        .try_acquire_issue_worktree_lock(
            &project_id,
            &issue_id,
            &journal.lock_work_item_id,
            &journal.worktree_lease_id,
        )
        .map_err(issue_worktree_active_api_error)?;
    let replay_already_bound = journal
        .phase
        .has_reached(CodingGroupInitializationPhase::AttemptPersisted)
        && worktree_lease.worktree.current_lock_owner_id.as_deref()
            == Some(journal.attempt.id.as_str());
    if !worktree_lease.acquired && !replay_already_bound {
        return Err(coding_group_attempt_incomplete_api_error(
            ProductStoreError::IdentityMismatch {
                kind: "coding_group_worktree_lease",
                id: journal.attempt.id.clone(),
            },
        ));
    }
    if !worktree_lease.acquired {
        coding_store
            .validate_materialized_group_initialization_attempt(&journal, &creation_guard)
            .map_err(coding_group_attempt_incomplete_api_error)?;
    }
    state
        .test_controls
        .pause_group_attempt_after_worktree_acquire_if_configured()
        .await;

    let attempt = coding_store
        .ensure_group_initialization_attempt(&journal, &creation_guard)
        .map_err(coding_group_attempt_incomplete_api_error)?;
    journal = coding_store
        .advance_group_initialization_phase(
            &journal,
            CodingGroupInitializationPhase::AttemptPersisted,
        )
        .map_err(coding_group_attempt_incomplete_api_error)?;
    maybe_interrupt_group_initialization(
        &state,
        crate::web::test_controls::GroupAttemptInitializationCheckpoint::PersistedBeforeBind,
    )?;

    lifecycle
        .bind_issue_worktree_lock_to_attempt(
            &project_id,
            &issue_id,
            &journal.lock_work_item_id,
            &attempt.id,
        )
        .map_err(product_store_api_error)?;
    maybe_interrupt_group_initialization(
        &state,
        crate::web::test_controls::GroupAttemptInitializationCheckpoint::BoundBeforePhaseAdvance,
    )?;
    journal = coding_store
        .advance_group_initialization_phase(&journal, CodingGroupInitializationPhase::WorktreeBound)
        .map_err(coding_group_attempt_incomplete_api_error)?;
    maybe_interrupt_group_initialization(
        &state,
        crate::web::test_controls::GroupAttemptInitializationCheckpoint::BoundBeforePlanBinding,
    )?;

    coding_store
        .ensure_group_initialization_plan_binding(&journal)
        .map_err(coding_group_attempt_incomplete_api_error)?;
    journal = coding_store
        .advance_group_initialization_phase(
            &journal,
            CodingGroupInitializationPhase::PlanBindingSaved,
        )
        .map_err(coding_group_attempt_incomplete_api_error)?;
    for index in 0..journal.units.len() {
        coding_store
            .ensure_group_initialization_unit(&journal, index)
            .map_err(coding_group_attempt_incomplete_api_error)?;
        if index == 0 {
            maybe_interrupt_group_initialization(
                &state,
                crate::web::test_controls::GroupAttemptInitializationCheckpoint::FirstUnitPersisted,
            )?;
        }
    }
    journal = coding_store
        .advance_group_initialization_phase(
            &journal,
            CodingGroupInitializationPhase::UnitsMaterialized,
        )
        .map_err(coding_group_attempt_incomplete_api_error)?;

    let persisted_attempt = coding_store
        .get_attempt(&project_id, &issue_id, &attempt.id)
        .map_err(product_store_api_error)?;
    coding_store
        .validate_group_attempt_integrity(&persisted_attempt)
        .map_err(coding_group_attempt_incomplete_api_error)?;
    coding_store
        .advance_group_initialization_phase(&journal, CodingGroupInitializationPhase::Completed)
        .map_err(coding_group_attempt_incomplete_api_error)?;
    Ok(Json(coding_attempt_dto(&persisted_attempt)))
}

fn resolve_group_repository(
    app_paths: &ProductAppPaths,
    project_id: &str,
    issue_id: &str,
    authoritative: &AuthoritativeGroupPlanBinding,
) -> ApiResult<RepositoryRecord> {
    match RepositoryRouting::load_for_issue(app_paths, project_id, issue_id)
        .map_err(product_store_api_error)?
    {
        RepositoryRouting::Legacy { .. } => {
            let repository_id = IssueStore::new(app_paths.clone())
                .get(project_id, issue_id)
                .map_err(product_store_api_error)?
                .repo_id
                .ok_or_else(|| {
                    product_store_api_error(ProductStoreError::NotFound {
                        kind: "repository",
                        id: format!("issue:{issue_id}:repo_id"),
                    })
                })?;
            resolve_legacy_group_repository(app_paths, project_id, &repository_id)
        }
        RepositoryRouting::Logical {
            manifest,
            selection,
        } => {
            if let Some(reason) = authoritative
                .units
                .iter()
                .find_map(|unit| unit.source_draft_error.as_deref())
            {
                return Err(routing_api_error(
                    RepositoryRoutingErrorCode::Inconsistent,
                    reason,
                ));
            }
            let selected_ids =
                validate_logical_group_selection(app_paths, project_id, &manifest, &selection)?;
            let target_ids: BTreeSet<LogicalRepositoryId> = authoritative
                .units
                .iter()
                .filter_map(|unit| unit.target_repository_id)
                .collect();
            let logical_repository_id = match target_ids.len() {
                1 => *target_ids.first().expect("one group target exists"),
                0 => {
                    let [focus_repository_id] = selection.focus_repository_ids.as_slice() else {
                        return Err(routing_api_error(
                            RepositoryRoutingErrorCode::TargetMissing,
                            "group has no unique target repository and selection focus is not unique",
                        ));
                    };
                    *focus_repository_id
                }
                _ => {
                    return Err(routing_api_error(
                        RepositoryRoutingErrorCode::TargetAmbiguous,
                        "group has multiple target repositories",
                    ));
                }
            };
            if !selected_ids.contains(&logical_repository_id) {
                return Err(routing_api_error(
                    RepositoryRoutingErrorCode::TargetUnknown,
                    "group target repository is not in the effective selection",
                ));
            }
            RepositoryStore::new(app_paths.clone())
                .resolve_logical_repository_strict(project_id, logical_repository_id)
                .map(|(_, _, repository)| repository)
                .map_err(product_store_api_error)
        }
        RepositoryRouting::FailClosed { code, reason } => Err(routing_api_error(code, &reason)),
    }
}

fn resolve_legacy_group_repository(
    app_paths: &ProductAppPaths,
    project_id: &str,
    physical_repository_id: &str,
) -> ApiResult<RepositoryRecord> {
    let store = RepositoryStore::new(app_paths.clone());
    match store.resolve_legacy_physical_repository_if_dual(project_id, physical_repository_id) {
        Ok((_, _, repository)) => return Ok(repository),
        Err(ProductStoreError::NotFound {
            kind: "logical_repository",
            ..
        }) => {}
        Err(error) => return Err(product_store_api_error(error)),
    }
    store
        .list(project_id)
        .map_err(product_store_api_error)?
        .into_iter()
        .find(|repository| repository.id == physical_repository_id)
        .ok_or_else(|| {
            product_store_api_error(ProductStoreError::NotFound {
                kind: "repository",
                id: physical_repository_id.to_string(),
            })
        })
}

fn validate_logical_group_selection(
    app_paths: &ProductAppPaths,
    project_id: &str,
    manifest: &crate::product::logical_codebase::LogicalCodebaseManifest,
    selection: &crate::product::logical_codebase::IssueCodebaseSelection,
) -> ApiResult<BTreeSet<LogicalRepositoryId>> {
    if selection.invalidation.is_some() {
        return Err(routing_api_error(
            RepositoryRoutingErrorCode::SelectionInvalidated,
            "issue codebase selection has been invalidated",
        ));
    }
    let active_members: BTreeSet<LogicalRepositoryId> =
        crate::product::logical_codebase::LogicalCodebaseStore::new(app_paths.clone())
            .list_members(project_id)
            .map_err(product_store_api_error)?
            .into_iter()
            .filter(|member| {
                member.status == crate::product::logical_codebase::MemberStatus::Active
            })
            .map(|member| member.logical_repository_id)
            .collect();
    if manifest
        .member_ids
        .iter()
        .any(|id| !active_members.contains(id))
    {
        return Err(routing_api_error(
            RepositoryRoutingErrorCode::MemberRemoved,
            "logical codebase manifest references a missing or inactive member",
        ));
    }
    match selection.selection_policy {
        SelectionPolicy::AllMembers => {
            if selection
                .focus_repository_ids
                .iter()
                .any(|id| !manifest.member_ids.contains(id))
            {
                return Err(routing_api_error(
                    RepositoryRoutingErrorCode::Inconsistent,
                    "issue codebase selection focus is outside the manifest",
                ));
            }
            Ok(manifest.member_ids.iter().copied().collect())
        }
        SelectionPolicy::Explicit => {
            selection.validate_focus_subset().map_err(|error| {
                routing_api_error(
                    RepositoryRoutingErrorCode::Inconsistent,
                    &format!("invalid issue codebase selection: {error}"),
                )
            })?;
            let selected_ids: BTreeSet<LogicalRepositoryId> =
                selection.resolve_effective_members().into_iter().collect();
            if selected_ids
                .iter()
                .any(|id| !manifest.member_ids.contains(id))
            {
                return Err(routing_api_error(
                    RepositoryRoutingErrorCode::Inconsistent,
                    "issue codebase selection references a member absent from the manifest",
                ));
            }
            Ok(selected_ids)
        }
    }
}

fn routing_api_error(code: RepositoryRoutingErrorCode, reason: &str) -> ApiError {
    let stable_code = match code {
        RepositoryRoutingErrorCode::TargetMissing => "repository_routing_target_missing",
        RepositoryRoutingErrorCode::OrphanedSelection
        | RepositoryRoutingErrorCode::Inconsistent
        | RepositoryRoutingErrorCode::MemberRemoved
        | RepositoryRoutingErrorCode::SelectionInvalidated => "repository_routing_inconsistent",
        RepositoryRoutingErrorCode::TargetUnknown => "repository_routing_target_unknown",
        RepositoryRoutingErrorCode::TargetAmbiguous => "repository_routing_ambiguous",
    };
    ApiError::runtime(
        stable_code,
        "repository routing failed closed",
        json!({ "reason": reason }),
    )
}

fn issue_worktree_active_api_error(error: ProductStoreError) -> ApiError {
    match error {
        ProductStoreError::Io(message) if message.contains("issue_worktree_active") => {
            ApiError::runtime(
                "issue_worktree_active",
                "another work item is already active on the issue shared worktree",
                json!({}),
            )
        }
        other => product_store_api_error(other),
    }
}

fn group_initialization_api_error(error: ProductStoreError) -> ApiError {
    match error {
        ProductStoreError::Io(message) if message.starts_with("active_coding_attempt_exists:") => {
            ApiError::runtime(
                "issue_worktree_active",
                "another work item is already active on the issue shared worktree",
                json!({}),
            )
        }
        other => coding_group_attempt_incomplete_api_error(other),
    }
}

fn maybe_interrupt_group_initialization(
    state: &WebAppState,
    checkpoint: crate::web::test_controls::GroupAttemptInitializationCheckpoint,
) -> ApiResult<()> {
    if !state
        .test_controls
        .consume_group_attempt_initialization_failure(checkpoint)
    {
        return Ok(());
    }
    Err(ApiError::runtime(
        "coding_group_initialization_interrupted",
        "group coding attempt initialization interrupted",
        json!({ "checkpoint": format!("{checkpoint:?}") }),
    ))
}

fn coding_plan_revision_binding_api_error(error: ProductStoreError) -> ApiError {
    if is_group_business_validation_error(&error) {
        ApiError::validation(
            "coding_plan_revision_binding_missing",
            "group coding requires complete authoritative plan revision bindings",
        )
    } else {
        product_store_api_error(error)
    }
}

fn coding_group_attempt_incomplete_api_error(error: ProductStoreError) -> ApiError {
    if is_group_business_validation_error(&error) {
        ApiError::validation(
            "coding_group_attempt_incomplete",
            "existing group coding attempt is only partially initialized or inconsistent",
        )
    } else {
        product_store_api_error(error)
    }
}
