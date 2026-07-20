use super::super::dto::*;
use super::super::support::*;
use super::super::*;
use super::{coding_provider_config_snapshot, group_work_item_execution_order};
use crate::product::coding_attempt_store::CodingGroupInitializationPhase;

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
    let provider_config_snapshot = coding_provider_config_snapshot(
        &lifecycle,
        current_work_item,
        &repository.default_provider_mode,
        &*state.provider_availability,
    )?;
    let initialization_input = CreateGroupCodingAttemptInput {
        project_id: project_id.clone(),
        issue_id: issue_id.clone(),
        plan_id: plan_id.clone(),
        current_work_item_id: current_work_item.id.clone(),
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
        .acquire_work_item_attempt_creation_async(&project_id, &issue_id, &current_work_item.id)
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
