use std::collections::BTreeSet;

use crate::product::models::{
    PlanRepairAwaitingConfirmationPackage, PlanRepairSessionSnapshotDto, WorkItemPlanLineage,
};
use crate::product::plan_repair::PlanRepairError;
use crate::web::workspace_ws_types::{WorkItemPlanReviewAction, WorkItemPlanReviewVerdict};

pub(crate) fn validate_awaiting_confirmation_package(
    snapshot: &PlanRepairSessionSnapshotDto,
    plan: &WorkItemPlanLineage,
    package: &PlanRepairAwaitingConfirmationPackage,
) -> Result<(), PlanRepairError> {
    if !package.validation.contract_validation.is_valid() {
        return Err(PlanRepairError::ContractValidation(
            package.validation.contract_validation.clone(),
        ));
    }
    if !package.validation.projection_validation.is_valid() {
        return Err(PlanRepairError::ProjectionValidation(
            package.validation.projection_validation.clone(),
        ));
    }
    if package.plan_review.verdict != WorkItemPlanReviewVerdict::Pass
        || package.plan_review.review_action != WorkItemPlanReviewAction::Continue
        || !package.plan_review.gates.is_empty()
    {
        return Err(invalid_package(
            "plan review must pass with continue and no gates",
        ));
    }
    let request = &snapshot.request;
    let amendment = &package.amendment;
    if plan.id != request.plan_id || package.validation.plan_id != request.plan_id {
        return Err(invalid_package("plan identity mismatch"));
    }
    if plan.active_revision_id.as_deref() != Some(request.base_plan_revision_id.as_str()) {
        return Err(PlanRepairError::AmendmentConflict {
            expected: request.base_plan_revision_id.clone(),
            actual: plan.active_revision_id.clone().unwrap_or_default(),
        });
    }
    if request.amendment_id.as_deref() != Some(amendment.id.as_str()) {
        return Err(PlanRepairError::AmendmentConflict {
            expected: request.amendment_id.clone().unwrap_or_default(),
            actual: amendment.id.clone(),
        });
    }
    if amendment.repair_request_id != request.id
        || amendment.previous_plan_revision_id != request.base_plan_revision_id
        || amendment.new_plan_revision_id == request.base_plan_revision_id
        || package.projection.plan_revision_id != amendment.new_plan_revision_id
    {
        return Err(invalid_package(
            "request, amendment, and revision identity mismatch",
        ));
    }
    if package.projection.human_group_projection.plan_id != request.plan_id
        || package.projection.coder_group_context.plan_id != request.plan_id
        || package.projection.reviewer_group_matrix.plan_id != request.plan_id
    {
        return Err(invalid_package("projection plan identity mismatch"));
    }
    if string_set(&package.impact.unaffected) != string_set(&amendment.unaffected_units)
        || string_set(&package.impact.direct_revalidation)
            != string_set(&amendment.revalidation_required_units)
        || string_set(&package.impact.direct_stale) != string_set(&amendment.stale_units)
    {
        return Err(invalid_package(
            "impact classifications do not match amendment manifest",
        ));
    }
    Ok(())
}

fn string_set(values: &[String]) -> BTreeSet<&str> {
    values.iter().map(String::as_str).collect()
}

fn invalid_package(message: &str) -> PlanRepairError {
    PlanRepairError::InvalidRepairTarget(format!(
        "invalid plan repair awaiting-confirmation package: {message}"
    ))
}
