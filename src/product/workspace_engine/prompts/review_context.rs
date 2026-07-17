use crate::product::models::{
    DependencyGraphRevision, PlanProjectionBundle, WorkItemProjectionBundle,
};
use crate::product::work_item_contract::CanonicalWorkItemContract;
use crate::product::work_item_projection::ProjectionValidationReport;
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
    let stored_projection = store
        .get_plan_projection_bundle(&lineage, &projection.id)
        .map_err(|error| format!("load Plan projection failed: {error}"))?;
    if revision.plan_projection_bundle_id != projection.id || stored_projection != *projection {
        return Err("Plan Review projection artifact binding mismatch".to_string());
    }
    let dependency_contract_graph = store
        .get_dependency_graph_revision(&lineage, &projection.dependency_graph_revision_id)
        .map_err(|error| format!("load Dependency Contract Graph failed: {error}"))?;
    let projection_validation_report = store
        .get_plan_validation_report(&lineage, &revision.validation_report_ref)
        .map_err(|error| format!("load Projection Validation Report failed: {error}"))?
        .projection_validation;
    let canonical_contract_candidates = revision
        .work_item_bindings
        .iter()
        .map(|(logical_id, revision_id)| {
            store
                .get_work_item_revision(&lineage, logical_id, revision_id)
                .map(|revision| revision.canonical_contract)
                .map_err(|error| format!("load Canonical Contract Candidate failed: {error}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let work_item_projection_bundle_candidates = projection
        .work_item_projection_bundle_refs
        .iter()
        .map(|bundle_id| {
            store
                .get_work_item_projection_bundle(&lineage, bundle_id)
                .map_err(|error| format!("load WorkItem projection failed: {error}"))
        })
        .collect::<Result<Vec<_>, _>>()?;

    Ok(PlanReviewContext {
        story_design_traceability: projection.human_group_projection.source_refs.clone(),
        canonical_contract_candidates,
        dependency_contract_graph,
        plan_projection_bundle_candidate: projection.clone(),
        work_item_projection_bundle_candidates,
        projection_validation_report,
        contract_delta: vec!["initial_plan_publication: no prior contract delta".to_string()],
        impact_analysis: revision.work_item_bindings.keys().cloned().collect(),
        repair_evidence: engine
            .session
            .messages
            .iter()
            .filter(|message| !matches!(message.role.as_str(), "assistant" | "provider"))
            .map(|message| format!("[{}] {}", message.role, message.content))
            .collect(),
    })
}
