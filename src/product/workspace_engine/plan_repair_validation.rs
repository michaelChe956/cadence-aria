use std::collections::BTreeSet;

use crate::product::models::{
    PlanRepairAwaitingConfirmationPackage, PlanRepairRequestStatus, PlanRepairSessionSnapshotDto,
    PlanRepairSessionStage, WorkItemPlanLineage,
};
use crate::product::plan_repair::{
    PlanRepairError, candidate_request_matches_review_status, load_plan_repair_candidate_package,
};
use crate::product::work_item_revision_store::WorkItemRevisionStore;
use crate::web::workspace_ws_types::{
    WorkItemPlanReviewAction, WorkItemPlanReviewScope, WorkItemPlanReviewVerdict,
};

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
        || package.plan_review.review_scope != WorkItemPlanReviewScope::Outline
        || package.plan_review.review_action != WorkItemPlanReviewAction::Continue
        || !package.plan_review.gates.is_empty()
        || package.plan_review.draft_id.is_some()
        || package.plan_review.batch_id.is_some()
    {
        return Err(invalid_package(
            "outline plan review must pass with continue, no gates, and no draft or batch binding",
        ));
    }
    let request = &snapshot.request;
    let amendment = &package.amendment;
    let identity = &package.package_identity;
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
    if plan.active_amendment_id.as_deref() != Some(amendment.id.as_str()) {
        return Err(PlanRepairError::AmendmentConflict {
            expected: amendment.id.clone(),
            actual: plan.active_amendment_id.clone().unwrap_or_default(),
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
    if identity.request_id != request.id
        || identity.amendment_id != amendment.id
        || identity.plan_id != request.plan_id
        || identity.base_plan_revision_id != request.base_plan_revision_id
        || identity.next_plan_revision_id != amendment.new_plan_revision_id
        || identity.projection_bundle_id != package.projection.id
        || identity.validation_report_id != package.validation.id
        || identity.reviewed_plan_revision_id != amendment.new_plan_revision_id
        || identity.review_generation_round_id != package.plan_review.generation_round_id
        || snapshot.candidate_package_artifact_id.as_deref()
            != Some(identity.candidate_package_artifact_id.as_str())
    {
        return Err(invalid_package("package identity binding mismatch"));
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

pub(crate) fn validate_persisted_awaiting_confirmation_package(
    revision_store: &WorkItemRevisionStore,
    snapshot: &PlanRepairSessionSnapshotDto,
    plan: &WorkItemPlanLineage,
    package: &PlanRepairAwaitingConfirmationPackage,
) -> Result<(), PlanRepairError> {
    validate_awaiting_confirmation_package(snapshot, plan, package)?;
    let candidate = load_plan_repair_candidate_package(
        revision_store,
        plan,
        &package.package_identity.candidate_package_artifact_id,
    )?;
    let persisted_review = revision_store
        .get_plan_repair_review_attestation(plan, &package.package_identity.review_attestation_id)
        .map_err(PlanRepairError::Store)?;
    let (expected_review_scope, expected_risk) = match &snapshot.impact_scope_review {
        Some(proposal) => (
            proposal.proposed_accepted_impact_scope.clone(),
            Some(proposal.risk_acceptance_reason.as_str()),
        ),
        None => (
            package
                .amendment
                .revalidation_required_units
                .iter()
                .chain(package.amendment.stale_units.iter())
                .cloned()
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>(),
            None,
        ),
    };
    let expected_request_status = if snapshot.stage == PlanRepairSessionStage::AwaitingConfirmation
        || snapshot.impact_scope_review.is_some()
    {
        PlanRepairRequestStatus::AwaitingConfirmation
    } else {
        PlanRepairRequestStatus::InProgress
    };
    if !candidate_request_matches_review_status(
        &candidate.request,
        &snapshot.request,
        expected_request_status,
    ) {
        return Err(invalid_package("candidate request provenance mismatch"));
    }
    if snapshot.candidate_package_artifact_id.as_deref() != Some(candidate.id.as_str())
        || candidate.minimum_manifest != package.amendment
        || candidate.plan_projection_bundle != package.projection
        || candidate.validation_report != package.validation
        || candidate.impact_report != package.impact
    {
        return Err(invalid_package("candidate artifact payload mismatch"));
    }
    if package.package_identity.candidate_package_artifact_id != candidate.id
        || package.package_identity.candidate_package_fingerprint
            != candidate.candidate_package_fingerprint
    {
        return Err(invalid_package("candidate package identity mismatch"));
    }
    if persisted_review.request_id != snapshot.request.id
        || persisted_review.amendment_id != package.amendment.id
        || persisted_review.plan_id != snapshot.request.plan_id
        || persisted_review.base_plan_revision_id != snapshot.request.base_plan_revision_id
        || persisted_review.reviewed_plan_revision_id != package.amendment.new_plan_revision_id
        || persisted_review.plan_projection_bundle_id != package.projection.id
        || persisted_review.generation_round_id != package.plan_review.generation_round_id
        || persisted_review.accepted_impact_scope != expected_review_scope
        || persisted_review.risk_acceptance_reason.as_deref() != expected_risk
        || persisted_review.candidate_package_artifact_id != candidate.id
        || persisted_review.candidate_package_fingerprint != candidate.candidate_package_fingerprint
    {
        return Err(invalid_package("persisted review provenance mismatch"));
    }
    if persisted_review.review != package.plan_review {
        return Err(invalid_package("persisted review payload mismatch"));
    }
    Ok(())
}

pub(crate) fn awaiting_confirmation_package_from_snapshot(
    snapshot: &PlanRepairSessionSnapshotDto,
) -> Result<PlanRepairAwaitingConfirmationPackage, PlanRepairError> {
    Ok(PlanRepairAwaitingConfirmationPackage {
        package_identity: snapshot
            .package_identity
            .clone()
            .ok_or_else(|| invalid_package("awaiting snapshot package identity is missing"))?,
        projection: snapshot
            .projection
            .clone()
            .ok_or_else(|| invalid_package("awaiting snapshot projection is missing"))?,
        amendment: snapshot
            .amendment
            .clone()
            .ok_or_else(|| invalid_package("awaiting snapshot amendment is missing"))?,
        validation: snapshot
            .validation
            .clone()
            .ok_or_else(|| invalid_package("awaiting snapshot validation is missing"))?,
        impact: snapshot
            .impact
            .clone()
            .ok_or_else(|| invalid_package("awaiting snapshot impact is missing"))?,
        plan_review: snapshot
            .plan_review
            .clone()
            .ok_or_else(|| invalid_package("awaiting snapshot plan review is missing"))?,
    })
}

fn string_set(values: &[String]) -> BTreeSet<&str> {
    values.iter().map(String::as_str).collect()
}

fn invalid_package(message: &str) -> PlanRepairError {
    PlanRepairError::InvalidRepairTarget(format!(
        "invalid plan repair awaiting-confirmation package: {message}"
    ))
}
