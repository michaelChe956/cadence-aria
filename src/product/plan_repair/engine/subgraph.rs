use std::collections::BTreeSet;

use crate::product::models::PlanRepairRequestStatus;
use crate::product::work_item_contract::{
    DependencyContractGraph, build_dependency_contract_graph,
};

use super::{PlanRepairEngine, invalid_target};
use crate::product::plan_repair::{
    PlanRepairError, SubgraphReplanReadiness, SubgraphReplanRequest, SubgraphReplanResult,
    SubgraphReplanner,
};

impl PlanRepairEngine {
    pub fn with_subgraph_replan_request(mut self, request: SubgraphReplanRequest) -> Self {
        self.subgraph_replan_request = Some(request);
        self
    }

    pub fn replan_subgraph(
        &self,
        request: &SubgraphReplanRequest,
    ) -> Result<SubgraphReplanResult, PlanRepairError> {
        let plan = self
            .store
            .get_plan_lineage(&self.plan.project_id, &self.plan.issue_id, &self.plan.id)
            .map_err(PlanRepairError::Store)?;
        if request.plan_id != plan.id
            || plan.active_revision_id.as_deref() != Some(request.base_plan_revision_id.as_str())
        {
            return Err(invalid_target(
                "subgraph replan request does not bind the active plan revision",
            ));
        }
        let repair_request = self
            .store
            .get_repair_request(&plan, &request.repair_request_id)
            .map_err(PlanRepairError::Store)?;
        if repair_request.plan_id != plan.id
            || repair_request.base_plan_revision_id != request.base_plan_revision_id
            || repair_request.amendment_id.as_deref() != plan.active_amendment_id.as_deref()
            || !matches!(
                repair_request.status,
                PlanRepairRequestStatus::InProgress | PlanRepairRequestStatus::AwaitingConfirmation
            )
        {
            return Err(invalid_target(
                "subgraph replan requires the authoritative active repair request",
            ));
        }
        let repair_targets = repair_request
            .repair_target
            .logical_work_item_ids
            .iter()
            .collect::<BTreeSet<_>>();
        if request
            .replacement_mapping
            .keys()
            .any(|id| !repair_targets.contains(id))
        {
            return Err(invalid_target(
                "subgraph replacement scope exceeds the persisted repair target",
            ));
        }

        let base_revision = self
            .store
            .get_plan_revision(
                &plan.project_id,
                &plan.issue_id,
                &plan.id,
                &request.base_plan_revision_id,
            )
            .map_err(PlanRepairError::Store)?;
        let stored_graph = self
            .store
            .get_dependency_graph_revision(&plan, &base_revision.dependency_graph_revision_id)
            .map_err(PlanRepairError::Store)?;
        let graph = self.load_authoritative_contract_graph(&plan, &base_revision)?;
        if stored_graph.plan_id != plan.id || stored_graph.edges != graph.edges {
            return Err(invalid_target(
                "active dependency graph revision differs from authoritative contracts",
            ));
        }

        let analysis = SubgraphReplanner::default().analyze(&graph, request)?;
        let dependency_graph_revision =
            if analysis.readiness == SubgraphReplanReadiness::PublicationReady {
                let replacement_ids = request
                    .replacement_mapping
                    .values()
                    .flatten()
                    .cloned()
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect::<Vec<_>>();
                let amendment_id = repair_request
                    .amendment_id
                    .as_deref()
                    .ok_or_else(|| invalid_target("subgraph repair request has no amendment id"))?;
                let ids = self
                    .store
                    .allocate_plan_amendment_publication_ids(
                        &plan,
                        amendment_id,
                        base_revision.revision_no + 1,
                        &replacement_ids,
                    )
                    .map_err(PlanRepairError::Store)?;
                let rebuilt = analysis.rebuilt_graph.as_ref().ok_or_else(|| {
                    invalid_target("publication-ready subgraph has no typed graph")
                })?;
                Some(crate::product::models::DependencyGraphRevision {
                    id: ids.dependency_graph_revision_id,
                    plan_id: plan.id.clone(),
                    edges: rebuilt.edges.clone(),
                    created_at: self.created_at.clone(),
                })
            } else {
                None
            };

        Ok(SubgraphReplanResult {
            base_plan_revision_id: base_revision.id,
            base_dependency_graph_revision_id: stored_graph.id,
            input_boundary: analysis.input_boundary,
            output_boundary: analysis.output_boundary,
            affected_logical_work_items: analysis.affected_logical_work_items,
            replacement_mapping: analysis.replacement_mapping,
            readiness: analysis.readiness,
            dependency_graph_revision,
            full_replan_required: analysis.full_replan_required,
        })
    }

    pub(super) fn load_authoritative_contract_graph(
        &self,
        plan: &crate::product::models::WorkItemPlanLineage,
        revision: &crate::product::models::WorkItemPlanRevision,
    ) -> Result<DependencyContractGraph, PlanRepairError> {
        let contracts = revision
            .work_item_bindings
            .iter()
            .map(|(logical_id, revision_id)| {
                self.store
                    .get_work_item_revision(plan, logical_id, revision_id)
                    .map(|revision| revision.canonical_contract)
                    .map_err(PlanRepairError::Store)
            })
            .collect::<Result<Vec<_>, _>>()?;
        build_dependency_contract_graph(&contracts).map_err(PlanRepairError::ContractValidation)
    }
}
