use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::product::models::{
    PlanAmendmentManifest, PlanProjectionBundle, PlanRepairRequest, PlanValidationReportArtifact,
    WorkItemProjectionBundle,
};
use crate::product::work_item_contract::{DependencyContractEdge, RequiredDependencyContract};

use super::{ContractDelta, ContractImpactReport, ImpactExplanationPath, PlanRepairError};

#[derive(Serialize)]
struct CandidatePackageFingerprintInput {
    request: CandidateRequestBinding,
    manifest: PlanAmendmentManifest,
    plan_projection: PlanProjectionBinding,
    work_item_projections: Vec<WorkItemProjectionBinding>,
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
    request_fingerprint: String,
}

#[derive(Serialize)]
struct PlanProjectionBinding {
    id: String,
    plan_revision_id: String,
    dependency_graph_revision_id: String,
    work_item_projection_bundle_refs: Vec<String>,
    human_group_projection_hash: String,
    coder_group_context_hash: String,
    reviewer_group_matrix_hash: String,
    compiler_version: String,
}

#[derive(Serialize)]
struct WorkItemProjectionBinding {
    id: String,
    work_item_revision_id: String,
    canonical_contract_hash: String,
    human_projection_hash: String,
    coder_projection_hash: String,
    reviewer_projection_hash: String,
    compiler_version: String,
}

pub fn candidate_package_fingerprint(
    request: &PlanRepairRequest,
    manifest: &PlanAmendmentManifest,
    plan_projection: &PlanProjectionBundle,
    work_item_projections: &[WorkItemProjectionBundle],
    validation: &PlanValidationReportArtifact,
    impact: &ContractImpactReport,
) -> Result<String, PlanRepairError> {
    let input = CandidatePackageFingerprintInput {
        request: normalized_request_binding(request),
        manifest: normalized_manifest(manifest)?,
        plan_projection: normalized_plan_projection(plan_projection),
        work_item_projections: normalized_work_item_projections(work_item_projections),
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

fn normalized_request_binding(request: &PlanRepairRequest) -> CandidateRequestBinding {
    let mut repair_target = request.repair_target.clone();
    repair_target.logical_work_item_ids = sorted_unique(&repair_target.logical_work_item_ids);
    repair_target.work_item_revision_ids = sorted_unique(&repair_target.work_item_revision_ids);
    CandidateRequestBinding {
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
        request_fingerprint: request.fingerprint.clone(),
    }
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

fn normalized_plan_projection(projection: &PlanProjectionBundle) -> PlanProjectionBinding {
    PlanProjectionBinding {
        id: projection.id.clone(),
        plan_revision_id: projection.plan_revision_id.clone(),
        dependency_graph_revision_id: projection.dependency_graph_revision_id.clone(),
        work_item_projection_bundle_refs: sorted_unique(
            &projection.work_item_projection_bundle_refs,
        ),
        human_group_projection_hash: projection.human_group_projection_hash.clone(),
        coder_group_context_hash: projection.coder_group_context_hash.clone(),
        reviewer_group_matrix_hash: projection.reviewer_group_matrix_hash.clone(),
        compiler_version: projection.compiler_version.clone(),
    }
}

fn normalized_work_item_projections(
    projections: &[WorkItemProjectionBundle],
) -> Vec<WorkItemProjectionBinding> {
    let mut bindings = projections
        .iter()
        .map(|projection| WorkItemProjectionBinding {
            id: projection.id.clone(),
            work_item_revision_id: projection.work_item_revision_id.clone(),
            canonical_contract_hash: projection.canonical_contract_hash.clone(),
            human_projection_hash: projection.human_projection_hash.clone(),
            coder_projection_hash: projection.coder_projection_hash.clone(),
            reviewer_projection_hash: projection.reviewer_projection_hash.clone(),
            compiler_version: projection.compiler_version.clone(),
        })
        .collect::<Vec<_>>();
    bindings.sort_by(|left, right| left.id.cmp(&right.id));
    bindings
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
