use std::collections::{BTreeMap, BTreeSet};

use crate::product::models::{
    DependencyGraphRevision, PlanProjectionBundle, PlanRevisionReason, WorkItemProjectionBundle,
    WorkItemRevision,
};
use crate::product::work_item_contract::{CanonicalWorkItemContract, canonical_contract_hash};
use crate::product::work_item_projection::{
    CompiledPlanProjections, CompiledWorkItemProjections, ProjectionValidationReport,
    plan_projection_hashes, projection_hashes,
};
use crate::product::work_item_revision_store::WorkItemRevisionStore;
use crate::product::workspace_engine::WorkspaceEngine;

pub(super) struct PlanReviewContext {
    pub(super) story_design_traceability: Vec<String>,
    pub(super) canonical_contract_candidates: Vec<CanonicalWorkItemContract>,
    pub(super) dependency_contract_graph: DependencyGraphRevision,
    pub(super) plan_projection_bundle_candidate: PlanProjectionBundle,
    pub(super) work_item_projection_bundle_candidates: Vec<WorkItemProjectionBundle>,
    pub(super) projection_validation_report: ProjectionValidationReport,
    pub(super) contract_delta: Vec<String>,
    pub(super) impact_analysis: Vec<String>,
    pub(super) repair_evidence: Vec<String>,
}

pub(super) fn load_plan_review_context(
    engine: &WorkspaceEngine,
    projection: &PlanProjectionBundle,
) -> Result<PlanReviewContext, String> {
    let lifecycle = engine
        .lifecycle_store
        .as_ref()
        .ok_or_else(|| "lifecycle_store unavailable for Plan Review Context".to_string())?;
    let store = WorkItemRevisionStore::new(lifecycle.app_paths());
    let lineage = store
        .get_plan_lineage(
            &engine.session.project_id,
            &engine.session.issue_id,
            &engine.session.entity_id,
        )
        .map_err(|error| format!("load Plan Review lineage failed: {error}"))?;
    let revision = store
        .get_plan_revision(
            &engine.session.project_id,
            &engine.session.issue_id,
            &lineage.id,
            &projection.plan_revision_id,
        )
        .map_err(|error| format!("load Plan Review revision failed: {error}"))?;
    if lineage.active_revision_id.as_deref() != Some(projection.plan_revision_id.as_str()) {
        return Err("Plan Review active revision binding mismatch".to_string());
    }
    if revision.reason != PlanRevisionReason::InitialCompile {
        return Err("Plan Review Context only supports initial plan publication".to_string());
    }
    let stored_projection = store
        .get_plan_projection_bundle(&lineage, &projection.id)
        .map_err(|error| format!("load Plan projection failed: {error}"))?;
    if revision.plan_projection_bundle_id != projection.id || stored_projection != *projection {
        return Err("Plan Review projection artifact binding mismatch".to_string());
    }
    if revision.dependency_graph_revision_id != projection.dependency_graph_revision_id {
        return Err("Plan Review dependency graph binding mismatch".to_string());
    }
    if projection.human_group_projection.plan_id != lineage.id
        || projection.coder_group_context.plan_id != lineage.id
        || projection.reviewer_group_matrix.plan_id != lineage.id
    {
        return Err("Plan Review plan projection identity mismatch".to_string());
    }
    validate_plan_projection_hashes(projection)?;
    let dependency_contract_graph = store
        .get_dependency_graph_revision(&lineage, &projection.dependency_graph_revision_id)
        .map_err(|error| format!("load Dependency Contract Graph failed: {error}"))?;
    let projection_validation_report = store
        .get_plan_validation_report(&lineage, &revision.validation_report_ref)
        .map_err(|error| format!("load Projection Validation Report failed: {error}"))?
        .projection_validation;
    let work_item_revisions = revision
        .work_item_bindings
        .iter()
        .map(|(logical_id, revision_id)| {
            store
                .get_work_item_revision(&lineage, logical_id, revision_id)
                .map_err(|error| format!("load Canonical Contract Candidate failed: {error}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let expected_projection_refs = work_item_revisions
        .iter()
        .map(|revision| revision.work_item_projection_bundle_id.clone())
        .collect::<BTreeSet<_>>();
    let actual_projection_refs = projection
        .work_item_projection_bundle_refs
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    if expected_projection_refs.len() != work_item_revisions.len()
        || actual_projection_refs.len() != projection.work_item_projection_bundle_refs.len()
        || expected_projection_refs != actual_projection_refs
    {
        return Err("Plan Review WorkItem projection reference set mismatch".to_string());
    }
    let work_item_projection_bundle_candidates = projection
        .work_item_projection_bundle_refs
        .iter()
        .map(|bundle_id| {
            store
                .get_work_item_projection_bundle(&lineage, bundle_id)
                .map_err(|error| format!("load WorkItem projection failed: {error}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    validate_work_item_projection_bindings(
        &work_item_revisions,
        &work_item_projection_bundle_candidates,
    )?;
    let canonical_contract_candidates = work_item_revisions
        .iter()
        .map(|revision| revision.canonical_contract.clone())
        .collect();
    let initial_work_item_ids = revision
        .work_item_bindings
        .keys()
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");

    Ok(PlanReviewContext {
        story_design_traceability: projection.human_group_projection.source_refs.clone(),
        canonical_contract_candidates,
        dependency_contract_graph,
        plan_projection_bundle_candidate: projection.clone(),
        work_item_projection_bundle_candidates,
        projection_validation_report,
        contract_delta: vec!["initial_plan_publication: no previous contract delta".to_string()],
        impact_analysis: vec![format!("initial_full_set: {initial_work_item_ids}")],
        repair_evidence: vec!["initial_plan_publication: no repair evidence".to_string()],
    })
}

fn validate_work_item_projection_bindings(
    revisions: &[WorkItemRevision],
    bundles: &[WorkItemProjectionBundle],
) -> Result<(), String> {
    let bundles_by_id = bundles
        .iter()
        .map(|bundle| (bundle.id.as_str(), bundle))
        .collect::<BTreeMap<_, _>>();
    for revision in revisions {
        let bundle = bundles_by_id
            .get(revision.work_item_projection_bundle_id.as_str())
            .ok_or_else(|| "Plan Review WorkItem projection bundle missing".to_string())?;
        let logical_id = revision.logical_work_item_id.as_str();
        let actual_contract_hash = canonical_contract_hash(&revision.canonical_contract)
            .map_err(|error| format!("hash Canonical Contract Candidate failed: {error}"))?;
        let projection_hashes = projection_hashes(&CompiledWorkItemProjections {
            human: bundle.human_projection.clone(),
            coder: bundle.coder_projection.clone(),
            reviewer: bundle.reviewer_projection.clone(),
        })
        .map_err(|error| format!("hash WorkItem projection failed: {error}"))?;
        if revision.canonical_contract_hash != actual_contract_hash
            || revision.canonical_contract.identity.logical_work_item_id != logical_id
            || bundle.work_item_revision_id != revision.id
            || bundle.canonical_contract_hash != revision.canonical_contract_hash
            || bundle.human_projection_hash != projection_hashes.human
            || bundle.coder_projection_hash != projection_hashes.coder
            || bundle.reviewer_projection_hash != projection_hashes.reviewer
            || bundle.human_projection.logical_work_item_id != logical_id
            || bundle.coder_projection.work_item_revision_id != revision.id
            || bundle.reviewer_projection.work_item_revision_id != revision.id
        {
            return Err(format!(
                "Plan Review WorkItem projection binding mismatch for `{logical_id}`"
            ));
        }
    }
    Ok(())
}

fn validate_plan_projection_hashes(projection: &PlanProjectionBundle) -> Result<(), String> {
    let hashes = plan_projection_hashes(&CompiledPlanProjections {
        human: projection.human_group_projection.clone(),
        coder: projection.coder_group_context.clone(),
        reviewer: projection.reviewer_group_matrix.clone(),
    })
    .map_err(|error| format!("hash Plan projection failed: {error}"))?;
    if projection.human_group_projection_hash != hashes.human
        || projection.coder_group_context_hash != hashes.coder
        || projection.reviewer_group_matrix_hash != hashes.reviewer
    {
        return Err("Plan Review plan projection payload hash mismatch".to_string());
    }
    Ok(())
}
