use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::product::models::{
    PlanAmendmentManifest, PlanProjectionBundle, PlanRepairCandidatePackageArtifact,
    PlanRepairRequest, PlanRepairRequestStatus, PlanValidationReportArtifact, WorkItemPlanLineage,
    WorkItemProjectionBundle,
};
use crate::product::work_item_contract::{DependencyContractEdge, RequiredDependencyContract};
use crate::product::work_item_projection::{
    CompiledPlanProjections, CompiledWorkItemProjections, plan_projection_hashes, projection_hashes,
};
use crate::product::work_item_revision_store::WorkItemRevisionStore;

use super::{ContractDelta, ContractImpactReport, ImpactExplanationPath, PlanRepairError};

#[derive(Serialize)]
struct CandidatePackageFingerprintInput {
    request: CandidateRequestBinding,
    manifest: PlanAmendmentManifest,
    plan_projection: PlanProjectionBundle,
    work_item_projections: Vec<WorkItemProjectionBundle>,
    validation: PlanValidationReportArtifact,
    impact: ContractImpactReport,
}

#[derive(Serialize)]
struct CandidateRequestBinding {
    id: String,
    plan_id: String,
    base_plan_revision_id: String,
    amendment_id: Option<String>,
    trigger_attempt_id: String,
    trigger_unit_run_id: String,
    trigger_review_id: Option<String>,
    trigger_finding_id: String,
    defect_class: crate::product::models::PlanDefectClass,
    reason_code: String,
    repair_target: crate::product::models::RepairTarget,
    contract_refs: Vec<String>,
    capability_refs: Vec<String>,
    evidence: Vec<crate::product::models::PlanDefectEvidence>,
    request_fingerprint: String,
}

pub fn candidate_package_fingerprint(
    request: &PlanRepairRequest,
    manifest: &PlanAmendmentManifest,
    plan_projection: &PlanProjectionBundle,
    work_item_projections: &[WorkItemProjectionBundle],
    validation: &PlanValidationReportArtifact,
    impact: &ContractImpactReport,
) -> Result<String, PlanRepairError> {
    validate_plan_projection_payload_hashes(plan_projection)?;
    let work_item_projections =
        canonical_work_item_projection_bundles(plan_projection, work_item_projections)?;
    let input = CandidatePackageFingerprintInput {
        request: normalized_request_binding(request)?,
        manifest: normalized_manifest(manifest)?,
        plan_projection: plan_projection.clone(),
        work_item_projections,
        validation: normalized_validation(validation)?,
        impact: normalized_impact(impact)?,
    };
    let bytes = serde_json::to_vec(&input).map_err(|error| {
        PlanRepairError::InvalidRepairTarget(format!(
            "serialize Plan Repair candidate package fingerprint: {error}"
        ))
    })?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn normalized_request_binding(
    request: &PlanRepairRequest,
) -> Result<CandidateRequestBinding, PlanRepairError> {
    let mut repair_target = request.repair_target.clone();
    repair_target.logical_work_item_ids = sorted_unique(&repair_target.logical_work_item_ids);
    repair_target.work_item_revision_ids = sorted_unique(&repair_target.work_item_revision_ids);
    let mut evidence = request.evidence.clone();
    sort_serializable(&mut evidence)?;
    evidence.dedup();
    Ok(CandidateRequestBinding {
        id: request.id.clone(),
        plan_id: request.plan_id.clone(),
        base_plan_revision_id: request.base_plan_revision_id.clone(),
        amendment_id: request.amendment_id.clone(),
        trigger_attempt_id: request.trigger_attempt_id.clone(),
        trigger_unit_run_id: request.trigger_unit_run_id.clone(),
        trigger_review_id: request.trigger_review_id.clone(),
        trigger_finding_id: request.trigger_finding_id.clone(),
        defect_class: request.defect_class.clone(),
        reason_code: request.reason_code.clone(),
        repair_target,
        contract_refs: sorted_unique(&request.contract_refs),
        capability_refs: sorted_unique(&request.capability_refs),
        evidence,
        request_fingerprint: request.fingerprint.clone(),
    })
}

fn normalized_manifest(
    manifest: &PlanAmendmentManifest,
) -> Result<PlanAmendmentManifest, PlanRepairError> {
    let mut normalized = manifest.clone();
    normalized.superseded_revisions = sorted_unique(&normalized.superseded_revisions);
    normalized.unaffected_units = sorted_unique(&normalized.unaffected_units);
    normalized.revalidation_required_units = sorted_unique(&normalized.revalidation_required_units);
    normalized.stale_units = sorted_unique(&normalized.stale_units);
    for replacements in normalized.replacement_units.values_mut() {
        *replacements = sorted_unique(replacements);
    }
    for change in &mut normalized.dependency_graph_changes {
        change.previous = change.previous.take().map(normalized_edge);
        change.next = change.next.take().map(normalized_edge);
    }
    sort_serializable(&mut normalized.dependency_graph_changes)?;
    for delta in &mut normalized.contract_deltas {
        normalize_delta(delta);
    }
    sort_serializable(&mut normalized.contract_deltas)?;
    Ok(normalized)
}

fn normalized_edge(mut edge: DependencyContractEdge) -> DependencyContractEdge {
    for contract in &mut edge.required_contracts {
        normalize_required_contract(contract);
    }
    edge.required_contracts.sort_by(|left, right| {
        (
            &left.contract_id,
            &left.required_capabilities,
            compatibility_policy_rank(&left.compatibility_policy),
        )
            .cmp(&(
                &right.contract_id,
                &right.required_capabilities,
                compatibility_policy_rank(&right.compatibility_policy),
            ))
    });
    edge.required_contracts.dedup();
    edge
}

fn compatibility_policy_rank(
    policy: &crate::product::work_item_contract::ContractCompatibilityPolicy,
) -> u8 {
    match policy {
        crate::product::work_item_contract::ContractCompatibilityPolicy::RequireAll => 0,
        crate::product::work_item_contract::ContractCompatibilityPolicy::RequireAny => 1,
    }
}

fn normalize_required_contract(contract: &mut RequiredDependencyContract) {
    contract.required_capabilities = sorted_unique(&contract.required_capabilities);
}

fn normalize_delta(delta: &mut ContractDelta) {
    delta.added_contracts = sorted_unique(&delta.added_contracts);
    delta.removed_contracts = sorted_unique(&delta.removed_contracts);
    delta.added_capabilities = sorted_unique(&delta.added_capabilities);
    delta.removed_capabilities = sorted_unique(&delta.removed_capabilities);
    delta.changed_capabilities = sorted_unique(&delta.changed_capabilities);
    delta.added_capability_associations.sort();
    delta.added_capability_associations.dedup();
    delta.removed_capability_associations.sort();
    delta.removed_capability_associations.dedup();
}

pub fn canonical_work_item_projection_bundles(
    plan_projection: &PlanProjectionBundle,
    bundles: &[WorkItemProjectionBundle],
) -> Result<Vec<WorkItemProjectionBundle>, PlanRepairError> {
    let refs = &plan_projection.work_item_projection_bundle_refs;
    let unique_refs = refs.iter().collect::<BTreeSet<_>>();
    if unique_refs.len() != refs.len() {
        return Err(invalid_package(
            "Plan projection contains duplicate WorkItem projection refs",
        ));
    }
    let mut by_id = BTreeMap::new();
    for bundle in bundles {
        if by_id.insert(bundle.id.clone(), bundle.clone()).is_some() {
            return Err(invalid_package(
                "candidate package contains duplicate WorkItem projection bundles",
            ));
        }
    }
    if by_id.len() != refs.len() {
        return Err(invalid_package(
            "candidate package WorkItem projection bundle count mismatch",
        ));
    }
    let ordered = refs
        .iter()
        .map(|bundle_id| {
            by_id.remove(bundle_id).ok_or_else(|| {
                invalid_package("candidate package is missing a referenced WorkItem projection")
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    if !by_id.is_empty() {
        return Err(invalid_package(
            "candidate package contains an unreferenced WorkItem projection",
        ));
    }
    for bundle in &ordered {
        validate_work_item_projection_payload_hashes(bundle)?;
    }
    Ok(ordered)
}

pub fn build_plan_repair_candidate_package(
    plan: &WorkItemPlanLineage,
    request: &PlanRepairRequest,
    manifest: &PlanAmendmentManifest,
    plan_projection: &PlanProjectionBundle,
    work_item_projections: &[WorkItemProjectionBundle],
    validation: &PlanValidationReportArtifact,
    impact: &ContractImpactReport,
) -> Result<PlanRepairCandidatePackageArtifact, PlanRepairError> {
    if request.plan_id != plan.id
        || request.status != PlanRepairRequestStatus::InProgress
        || request.amendment_id.as_deref() != Some(manifest.id.as_str())
        || request.base_plan_revision_id != manifest.previous_plan_revision_id
        || manifest.repair_request_id != request.id
        || plan_projection.plan_revision_id != manifest.new_plan_revision_id
        || validation.plan_id != plan.id
        || validation.plan_revision_id != manifest.new_plan_revision_id
        || validation.plan_projection_bundle_id != plan_projection.id
    {
        return Err(invalid_package(
            "candidate package identity does not match request and prepared artifacts",
        ));
    }
    let bundles = canonical_work_item_projection_bundles(plan_projection, work_item_projections)?;
    let fingerprint = candidate_package_fingerprint(
        request,
        manifest,
        plan_projection,
        &bundles,
        validation,
        impact,
    )?;
    Ok(PlanRepairCandidatePackageArtifact {
        id: format!("plan_repair_candidate_package_{}", manifest.id),
        project_id: plan.project_id.clone(),
        issue_id: plan.issue_id.clone(),
        plan_id: plan.id.clone(),
        request: request.clone(),
        request_id: request.id.clone(),
        amendment_id: manifest.id.clone(),
        base_plan_revision_id: request.base_plan_revision_id.clone(),
        new_plan_revision_id: manifest.new_plan_revision_id.clone(),
        minimum_manifest: manifest.clone(),
        plan_projection_bundle: plan_projection.clone(),
        work_item_projection_bundles: bundles,
        validation_report: validation.clone(),
        impact_report: impact.clone(),
        candidate_package_fingerprint: fingerprint,
        created_at: manifest.created_at.clone(),
    })
}

pub fn load_plan_repair_candidate_package(
    store: &WorkItemRevisionStore,
    plan: &WorkItemPlanLineage,
    package_id: &str,
) -> Result<PlanRepairCandidatePackageArtifact, PlanRepairError> {
    let artifact = store
        .get_plan_repair_candidate_package(plan, package_id)
        .map_err(PlanRepairError::Store)?;
    let request = store
        .get_repair_request(plan, &artifact.request_id)
        .map_err(PlanRepairError::Store)?;
    let stored_projection = store
        .get_plan_projection_bundle(plan, &artifact.plan_projection_bundle.id)
        .map_err(PlanRepairError::Store)?;
    let stored_validation = store
        .get_plan_validation_report(plan, &artifact.validation_report.id)
        .map_err(PlanRepairError::Store)?;
    let stored_bundles = artifact
        .plan_projection_bundle
        .work_item_projection_bundle_refs
        .iter()
        .map(|bundle_id| {
            store
                .get_work_item_projection_bundle(plan, bundle_id)
                .map_err(PlanRepairError::Store)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let canonical = build_plan_repair_candidate_package(
        plan,
        &artifact.request,
        &artifact.minimum_manifest,
        &stored_projection,
        &stored_bundles,
        &stored_validation,
        &artifact.impact_report,
    )?;
    if !candidate_request_binding_matches(&artifact.request, &request)
        || artifact.plan_projection_bundle != stored_projection
        || artifact.work_item_projection_bundles != stored_bundles
        || artifact.validation_report != stored_validation
        || artifact != canonical
    {
        return Err(invalid_package(
            "persisted candidate package differs from canonical authoritative artifacts",
        ));
    }
    Ok(artifact)
}

pub fn candidate_request_binding_matches(
    candidate: &PlanRepairRequest,
    authoritative: &PlanRepairRequest,
) -> bool {
    let mut normalized = authoritative.clone();
    normalized.status = candidate.status.clone();
    normalized.updated_at = candidate.updated_at.clone();
    normalized == *candidate
}

pub fn candidate_request_matches_review_status(
    candidate: &PlanRepairRequest,
    authoritative: &PlanRepairRequest,
    expected_authoritative_status: PlanRepairRequestStatus,
) -> bool {
    if !matches!(
        expected_authoritative_status,
        PlanRepairRequestStatus::InProgress | PlanRepairRequestStatus::AwaitingConfirmation
    ) || candidate.status != PlanRepairRequestStatus::InProgress
        || authoritative.status != expected_authoritative_status
        || (expected_authoritative_status == PlanRepairRequestStatus::InProgress
            && candidate.updated_at != authoritative.updated_at)
    {
        return false;
    }
    candidate_request_binding_matches(candidate, authoritative)
}

fn validate_plan_projection_payload_hashes(
    projection: &PlanProjectionBundle,
) -> Result<(), PlanRepairError> {
    let hashes = plan_projection_hashes(&CompiledPlanProjections {
        human: projection.human_group_projection.clone(),
        coder: projection.coder_group_context.clone(),
        reviewer: projection.reviewer_group_matrix.clone(),
    })
    .map_err(|error| invalid_package(&format!("hash Plan projection payload: {error}")))?;
    if projection.human_group_projection_hash != hashes.human
        || projection.coder_group_context_hash != hashes.coder
        || projection.reviewer_group_matrix_hash != hashes.reviewer
    {
        return Err(invalid_package("Plan projection payload hash mismatch"));
    }
    Ok(())
}

fn validate_work_item_projection_payload_hashes(
    projection: &WorkItemProjectionBundle,
) -> Result<(), PlanRepairError> {
    let hashes = projection_hashes(&CompiledWorkItemProjections {
        human: projection.human_projection.clone(),
        coder: projection.coder_projection.clone(),
        reviewer: projection.reviewer_projection.clone(),
    })
    .map_err(|error| invalid_package(&format!("hash WorkItem projection payload: {error}")))?;
    if projection.human_projection_hash != hashes.human
        || projection.coder_projection_hash != hashes.coder
        || projection.reviewer_projection_hash != hashes.reviewer
    {
        return Err(invalid_package("WorkItem projection payload hash mismatch"));
    }
    Ok(())
}

fn normalized_validation(
    validation: &PlanValidationReportArtifact,
) -> Result<PlanValidationReportArtifact, PlanRepairError> {
    let mut normalized = validation.clone();
    sort_serializable(&mut normalized.contract_validation.findings)?;
    sort_serializable(&mut normalized.projection_validation.findings)?;
    Ok(normalized)
}

fn normalized_impact(
    impact: &ContractImpactReport,
) -> Result<ContractImpactReport, PlanRepairError> {
    let mut normalized = impact.clone();
    normalized.unaffected = sorted_unique(&normalized.unaffected);
    normalized.direct_revalidation = sorted_unique(&normalized.direct_revalidation);
    normalized.direct_stale = sorted_unique(&normalized.direct_stale);
    normalized.conditional_downstream = sorted_unique(&normalized.conditional_downstream);
    for path in &mut normalized.explanation_paths {
        normalize_explanation_path(path);
    }
    sort_serializable(&mut normalized.explanation_paths)?;
    Ok(normalized)
}

fn normalize_explanation_path(path: &mut ImpactExplanationPath) {
    path.capability_refs = sorted_unique(&path.capability_refs);
}

fn sort_serializable<T: Serialize>(values: &mut Vec<T>) -> Result<(), PlanRepairError> {
    let mut keyed = values
        .drain(..)
        .map(|value| {
            serde_json::to_string(&value)
                .map(|key| (key, value))
                .map_err(|error| {
                    PlanRepairError::InvalidRepairTarget(format!(
                        "serialize candidate package collection: {error}"
                    ))
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    keyed.sort_by(|left, right| left.0.cmp(&right.0));
    keyed.dedup_by(|left, right| left.0 == right.0);
    values.extend(keyed.into_iter().map(|(_, value)| value));
    Ok(())
}

fn sorted_unique(values: &[String]) -> Vec<String> {
    values
        .iter()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn invalid_package(message: &str) -> PlanRepairError {
    PlanRepairError::InvalidRepairTarget(format!(
        "invalid Plan Repair candidate package: {message}"
    ))
}
