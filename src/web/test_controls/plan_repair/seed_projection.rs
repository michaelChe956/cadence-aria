use crate::product::models::{DependencyGraphRevision, PlanProjectionBundle, WorkItemPlanRevision};
use crate::product::work_item_projection::{
    CoderGroupContext, CompiledPlanProjections, HumanGroupProjection, HumanGroupWorkItemSummary,
    ProjectionCompileError, ReviewerGroupMatrix, ReviewerGroupMatrixEntry, plan_projection_hashes,
};
use crate::product::workspace_engine::CompiledWorkItemRevision;

const ORDERED_LOGICAL_WORK_ITEM_IDS: &[&str] = &["wi_core", "wi_registration", "wi_unrelated"];

pub(super) fn compile_plan_projection_bundle(
    plan_revision: &WorkItemPlanRevision,
    dependency_graph: &DependencyGraphRevision,
    published_work_items: &[CompiledWorkItemRevision],
    story_spec_id: &str,
    design_spec_id: &str,
    created_at: &str,
) -> Result<PlanProjectionBundle, ProjectionCompileError> {
    let ordered_logical_work_item_ids = ORDERED_LOGICAL_WORK_ITEM_IDS
        .iter()
        .map(|logical_id| (*logical_id).to_string())
        .collect::<Vec<_>>();
    let compiled_plan = CompiledPlanProjections {
        human: HumanGroupProjection {
            plan_id: plan_revision.plan_id.clone(),
            goal: "Plan Repair fixture".to_string(),
            split_reason: "Fixture publishes complete Schema v2 revisions".to_string(),
            work_items: ordered_logical_work_item_ids
                .iter()
                .map(|logical_id| {
                    let item = published_work_item(published_work_items, logical_id);
                    HumanGroupWorkItemSummary {
                        logical_work_item_id: logical_id.clone(),
                        title: item.projection_bundle.human_projection.title.clone(),
                        goal: item.projection_bundle.human_projection.goal.clone(),
                        depends_on: dependency_graph
                            .edges
                            .iter()
                            .filter(|edge| edge.to == *logical_id)
                            .map(|edge| edge.from.clone())
                            .collect(),
                        provides: item
                            .projection_bundle
                            .human_projection
                            .outputs
                            .iter()
                            .map(|output| output.contract_id.clone())
                            .collect(),
                        scope_summary: item
                            .projection_bundle
                            .human_projection
                            .scope_summary
                            .clone(),
                    }
                })
                .collect(),
            contract_flow: Vec::new(),
            risks: Vec::new(),
            source_refs: vec![story_spec_id.to_string(), design_spec_id.to_string()],
            normative: false,
            used_by_provider: false,
        },
        coder: CoderGroupContext {
            plan_id: plan_revision.plan_id.clone(),
            ordered_logical_work_item_ids: ordered_logical_work_item_ids.clone(),
            dependency_edges: dependency_graph.edges.clone(),
            group_write_scopes: ordered_logical_work_item_ids
                .iter()
                .map(|logical_id| {
                    (
                        logical_id.clone(),
                        published_work_item(published_work_items, logical_id)
                            .projection_bundle
                            .coder_projection
                            .write_policy
                            .clone(),
                    )
                })
                .collect(),
        },
        reviewer: ReviewerGroupMatrix {
            plan_id: plan_revision.plan_id.clone(),
            work_items: ordered_logical_work_item_ids
                .iter()
                .map(|logical_id| ReviewerGroupMatrixEntry {
                    logical_work_item_id: logical_id.clone(),
                    criterion_refs: published_work_item(published_work_items, logical_id)
                        .projection_bundle
                        .reviewer_projection
                        .criterion_refs
                        .clone(),
                    input_contract_refs: Vec::new(),
                    output_contract_refs: Vec::new(),
                })
                .collect(),
            dependency_edges: dependency_graph.edges.clone(),
            design_traceability_refs: Vec::new(),
        },
    };
    let hashes = plan_projection_hashes(&compiled_plan)?;

    Ok(PlanProjectionBundle {
        id: plan_revision.plan_projection_bundle_id.clone(),
        plan_revision_id: plan_revision.id.clone(),
        dependency_graph_revision_id: plan_revision.dependency_graph_revision_id.clone(),
        work_item_projection_bundle_refs: ordered_logical_work_item_ids
            .iter()
            .map(|logical_id| {
                published_work_item(published_work_items, logical_id)
                    .projection_bundle
                    .id
                    .clone()
            })
            .collect(),
        human_group_projection: compiled_plan.human,
        coder_group_context: compiled_plan.coder,
        reviewer_group_matrix: compiled_plan.reviewer,
        human_group_projection_hash: hashes.human,
        coder_group_context_hash: hashes.coder,
        reviewer_group_matrix_hash: hashes.reviewer,
        compiler_version: "plan-projection-compiler-v1".to_string(),
        created_at: created_at.to_string(),
    })
}

fn published_work_item<'a>(
    published_work_items: &'a [CompiledWorkItemRevision],
    logical_work_item_id: &str,
) -> &'a CompiledWorkItemRevision {
    published_work_items
        .iter()
        .find(|item| item.work_item_revision.logical_work_item_id == logical_work_item_id)
        .expect("published work item projection")
}
