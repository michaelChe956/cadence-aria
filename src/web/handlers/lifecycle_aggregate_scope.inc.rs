/// Validates optional aggregate scope fields accepted by the HTTP generation
/// endpoints. The product store repeats these invariants when persisting; doing
/// it here keeps malformed provider/gateway output in the request validation
/// family rather than exposing a generic store failure.
fn validate_requested_aggregate_scope(
    resolved_upper_bound: &[LogicalRepositoryId],
    involved_repository_ids: &[LogicalRepositoryId],
    change_order: &[LogicalRepositoryId],
) -> ApiResult<()> {
    for involved in involved_repository_ids {
        if !resolved_upper_bound.contains(involved) {
            return Err(ApiError::validation(
                "involved_repository_not_effective",
                "involved_repository_ids must be contained in the issue repository upper bound (selection focus or effective members)",
            ));
        }
    }
    for ordered in change_order {
        if !involved_repository_ids.contains(ordered) {
            return Err(ApiError::validation(
                "change_order_repository_not_involved",
                "change_order must be contained in involved_repository_ids",
            ));
        }
    }
    let unique_order = change_order.iter().collect::<BTreeSet<_>>();
    if unique_order.len() != change_order.len() {
        return Err(ApiError::validation(
            "change_order_duplicate_repository",
            "change_order must not contain a repository more than once",
        ));
    }
    Ok(())
}

fn reject_aggregate_scope_on_legacy_project(
    has_involved_repository_ids: bool,
    has_change_order: bool,
) -> ApiResult<()> {
    if has_involved_repository_ids || has_change_order {
        return Err(ApiError::validation(
            "aggregate_scope_requires_logical_codebase",
            "aggregate scope fields are only supported for multi-repository logical codebases",
        ));
    }
    Ok(())
}

pub(crate) fn validate_confirmed_story_specs(
    lifecycle: &LifecycleStore,
    project_id: &str,
    issue_id: &str,
    story_spec_ids: &[String],
) -> ApiResult<()> {
    if story_spec_ids.is_empty() {
        return Err(ApiError::validation(
            "story_spec_required",
            "story_spec_ids is required",
        ));
    }

    let stories = lifecycle
        .list_story_specs(project_id, issue_id)
        .map_err(product_store_api_error)?;
    for story_id in story_spec_ids {
        let Some(story) = stories.iter().find(|story| story.id == *story_id) else {
            return Err(ApiError::runtime(
                "story_spec_not_found",
                "story spec not found",
                json!({}),
            ));
        };
        if story.confirmation_status != LifecycleConfirmationStatus::Confirmed {
            return Err(ApiError::validation(
                "story_spec_not_confirmed",
                "story spec must be confirmed before generating downstream artifacts",
            ));
        }
    }
    Ok(())
}

pub(crate) fn validate_confirmed_design_specs(
    lifecycle: &LifecycleStore,
    project_id: &str,
    issue_id: &str,
    design_spec_ids: &[String],
) -> ApiResult<()> {
    if design_spec_ids.is_empty() {
        return Err(ApiError::validation(
            "design_spec_required",
            "design_spec_ids is required",
        ));
    }

    let designs = lifecycle
        .list_design_specs(project_id, issue_id)
        .map_err(product_store_api_error)?;
    for design_id in design_spec_ids {
        let Some(design) = designs.iter().find(|design| design.id == *design_id) else {
            return Err(ApiError::runtime(
                "design_spec_not_found",
                "design spec not found",
                json!({}),
            ));
        };
        if design.confirmation_status != LifecycleConfirmationStatus::Confirmed {
            return Err(ApiError::validation(
                "design_spec_not_confirmed",
                "design spec must be confirmed before generating work items",
            ));
        }
    }
    Ok(())
}

/// confirm gate 失败映射（#5 收尾）：按 [`ConfirmAggregateGateError`] 变体区分 HTTP 状态。
/// - Violation（业务违规）→ 4xx + 稳定码（error.rs 显式映射 400）；
/// - SpecNotFound → 404 `spec_not_found`；
/// - SpecLoad → 500 `confirm_gate_spec_load_failed`。
fn confirm_gate_api_error(error: ConfirmAggregateGateError) -> ApiError {
    let code = error.stable_code();
    let message = error.message();
    match error {
        ConfirmAggregateGateError::Violation { .. } => ApiError::validation(code, message),
        ConfirmAggregateGateError::SpecNotFound(_) => ApiError::validation(code, message),
        ConfirmAggregateGateError::SpecLoad(_) => ApiError::runtime(code, message, json!({})),
    }
}

pub(crate) fn confirm_workspace_entity(
    lifecycle: &LifecycleStore,
    session: &WorkspaceSessionRecord,
) -> ApiResult<()> {
    match session.workspace_type {
        WorkspaceType::Story | WorkspaceType::Design => {
            // confirm gate（方案 X 3b 收紧，Blocker 2 下沉到 product 层）：多仓 Spec
            // （logical_codebase_ref Some）在 Confirmed 前校验 ① involved 非空（REQ-PLN-04）；
            // ② Design involved>1 必须 change_order（REQ-PLN-05 收紧）。单仓不校验（红线）。
            // #5 收尾：load_existing_spec 失败按 NotFound→404 / IO→500 区分（修复前统一 500）。
            if let Err(error) = lifecycle.validate_confirm_aggregate_spec(
                &session.project_id,
                &session.issue_id,
                &session.entity_id,
                &session.workspace_type,
            ) {
                return Err(confirm_gate_api_error(error));
            }
            lifecycle
                .update_spec_confirmation_status(
                    &session.project_id,
                    &session.issue_id,
                    &session.entity_id,
                    LifecycleConfirmationStatus::Confirmed,
                )
                .map_err(product_store_api_error)
        }
        WorkspaceType::WorkItem => lifecycle
            .update_work_item_plan_status(
                &session.project_id,
                &session.issue_id,
                &session.entity_id,
                WorkItemPlanStatus::Confirmed,
            )
            .map(|_| ())
            .map_err(product_store_api_error),
        WorkspaceType::WorkItemPlan => Err(ApiError::runtime(
            "work_item_plan_confirm_not_supported",
            "confirm is not yet supported for work item plan sessions",
            json!({}),
        )),
    }
}

