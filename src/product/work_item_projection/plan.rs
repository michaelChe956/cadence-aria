use std::collections::{BTreeMap, BTreeSet};

use crate::product::work_item_contract::{
    ContractCompatibilityPolicy, DependencyContractGraph, DesignTraceabilityRef,
};

use super::validation::{normalized_source_refs, validate_plan_compile_context};
use super::{
    CoderGroupContext, CompiledPlanProjections, CompiledWorkItemProjections, HumanContractFlowEdge,
    HumanGroupProjection, HumanGroupWorkItemSummary, PlanProjectionCompileInput,
    PlanProjectionValidationInput, ProjectionCompileError, ProjectionValidationFinding,
    ProjectionValidationReport, ReviewerGroupMatrix, ReviewerGroupMatrixEntry,
    validate_plan_projection_coverage,
};

#[derive(Debug, Default)]
pub struct PlanProjectionCompiler;

impl PlanProjectionCompiler {
    pub fn compile(
        &self,
        input: PlanProjectionCompileInput<'_>,
    ) -> Result<CompiledPlanProjections, ProjectionCompileError> {
        let context_validation = validate_plan_compile_context(
            input.dependency_graph,
            input.expected_work_item_revision_ids,
            input.work_item_projections,
        );
        if !context_validation.is_valid() {
            return Err(ProjectionCompileError::Validation(context_validation));
        }
        let ordered_ids =
            stable_topological_order(input.dependency_graph, input.work_item_projections)?;
        let contract_flow = contract_flow(input.dependency_graph);
        let risks = risks_from_flow(&contract_flow);
        let compiled = CompiledPlanProjections {
            human: HumanGroupProjection {
                plan_id: input.plan_id.to_string(),
                goal: input.goal.to_string(),
                split_reason: input.split_reason.to_string(),
                work_items: human_work_items(
                    &ordered_ids,
                    input.dependency_graph,
                    input.work_item_projections,
                ),
                contract_flow,
                risks,
                source_refs: normalized_source_refs(input.source_refs),
                normative: false,
                used_by_provider: false,
            },
            coder: CoderGroupContext {
                plan_id: input.plan_id.to_string(),
                ordered_logical_work_item_ids: ordered_ids.clone(),
                dependency_edges: input.dependency_graph.edges.clone(),
                group_write_scopes: ordered_ids
                    .iter()
                    .map(|logical_id| {
                        (
                            logical_id.clone(),
                            input.work_item_projections[logical_id]
                                .coder
                                .write_policy
                                .clone(),
                        )
                    })
                    .collect(),
            },
            reviewer: ReviewerGroupMatrix {
                plan_id: input.plan_id.to_string(),
                work_items: reviewer_work_items(&ordered_ids, input.work_item_projections),
                dependency_edges: input.dependency_graph.edges.clone(),
                design_traceability_refs: design_traceability_refs(input.dependency_graph),
            },
        };

        let validation = validate_plan_projection_coverage(PlanProjectionValidationInput {
            expected_plan_id: input.plan_id,
            expected_source_refs: input.source_refs,
            expected_work_item_revision_ids: input.expected_work_item_revision_ids,
            dependency_graph: input.dependency_graph,
            compiled: &compiled,
            work_item_projections: input.work_item_projections,
        });
        if validation.is_valid() {
            Ok(compiled)
        } else {
            Err(ProjectionCompileError::Validation(validation))
        }
    }
}

pub(crate) fn stable_topological_order(
    graph: &DependencyContractGraph,
    work_items: &BTreeMap<String, CompiledWorkItemProjections>,
) -> Result<Vec<String>, ProjectionCompileError> {
    let graph_ids = graph.contracts.keys().cloned().collect::<BTreeSet<_>>();
    let projection_ids = work_items.keys().cloned().collect::<BTreeSet<_>>();
    if graph_ids != projection_ids {
        return validation_error(
            "plan_projection_work_item_mismatch",
            "dependency graph and work item projection IDs differ",
        );
    }

    let mut indegree = graph_ids
        .iter()
        .map(|logical_id| (logical_id.clone(), 0_usize))
        .collect::<BTreeMap<_, _>>();
    let mut adjacency = graph_ids
        .iter()
        .map(|logical_id| (logical_id.clone(), Vec::<String>::new()))
        .collect::<BTreeMap<_, _>>();
    for edge in &graph.edges {
        if !graph_ids.contains(&edge.from) || !graph_ids.contains(&edge.to) {
            return validation_error(
                "plan_projection_edge_mismatch",
                "dependency graph edge references an unknown work item",
            );
        }
        adjacency
            .get_mut(&edge.from)
            .expect("known edge source")
            .push(edge.to.clone());
        *indegree.get_mut(&edge.to).expect("known edge target") += 1;
    }

    let mut layer = indegree
        .iter()
        .filter_map(|(logical_id, count)| (*count == 0).then_some(logical_id.clone()))
        .collect::<BTreeSet<_>>();
    let mut ordered = Vec::with_capacity(graph_ids.len());
    while !layer.is_empty() {
        let current = std::mem::take(&mut layer);
        let mut next_layer = BTreeSet::new();
        for logical_id in current {
            ordered.push(logical_id.clone());
            for consumer in &adjacency[&logical_id] {
                let count = indegree.get_mut(consumer).expect("known consumer");
                *count -= 1;
                if *count == 0 {
                    next_layer.insert(consumer.clone());
                }
            }
        }
        layer = next_layer;
    }

    if ordered.len() != graph_ids.len() {
        return validation_error(
            "plan_projection_edge_mismatch",
            "dependency graph contains a cycle",
        );
    }
    Ok(ordered)
}

pub(crate) fn contract_flow(graph: &DependencyContractGraph) -> Vec<HumanContractFlowEdge> {
    graph
        .edges
        .iter()
        .flat_map(|edge| {
            edge.required_contracts.iter().map(|required| {
                let provided_capabilities = graph
                    .contracts
                    .get(&edge.from)
                    .into_iter()
                    .flat_map(|provider| provider.output_contracts.iter())
                    .filter(|output| output.contract_id == required.contract_id)
                    .flat_map(|output| output.capabilities.iter().cloned())
                    .collect::<BTreeSet<_>>();
                let required_capabilities = required
                    .required_capabilities
                    .iter()
                    .cloned()
                    .collect::<BTreeSet<_>>();
                let missing_capabilities = match required.compatibility_policy {
                    ContractCompatibilityPolicy::RequireAll => required_capabilities
                        .difference(&provided_capabilities)
                        .cloned()
                        .collect(),
                    ContractCompatibilityPolicy::RequireAny
                        if required_capabilities.is_empty()
                            || !required_capabilities.is_disjoint(&provided_capabilities) =>
                    {
                        Vec::new()
                    }
                    ContractCompatibilityPolicy::RequireAny => {
                        required_capabilities.iter().cloned().collect()
                    }
                };
                HumanContractFlowEdge {
                    from: edge.from.clone(),
                    to: edge.to.clone(),
                    contract_id: required.contract_id.clone(),
                    required_capabilities: required_capabilities.into_iter().collect(),
                    provided_capabilities: provided_capabilities.into_iter().collect(),
                    missing_capabilities,
                }
            })
        })
        .collect()
}

pub(crate) fn risks_from_flow(contract_flow: &[HumanContractFlowEdge]) -> Vec<String> {
    contract_flow
        .iter()
        .filter(|flow| !flow.missing_capabilities.is_empty())
        .map(|flow| {
            format!(
                "contract {} on {} -> {} is missing capabilities: {}",
                flow.contract_id,
                flow.from,
                flow.to,
                flow.missing_capabilities.join(", ")
            )
        })
        .collect()
}

pub(crate) fn human_work_items(
    ordered_ids: &[String],
    graph: &DependencyContractGraph,
    work_items: &BTreeMap<String, CompiledWorkItemProjections>,
) -> Vec<HumanGroupWorkItemSummary> {
    ordered_ids
        .iter()
        .map(|logical_id| {
            let canonical = &graph.contracts[logical_id];
            let projection = &work_items[logical_id];
            HumanGroupWorkItemSummary {
                logical_work_item_id: logical_id.clone(),
                title: projection.human.title.clone(),
                goal: projection.human.goal.clone(),
                depends_on: canonical
                    .input_contracts
                    .iter()
                    .map(|input| input.provider_logical_work_item_id.clone())
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect(),
                provides: canonical
                    .output_contracts
                    .iter()
                    .map(|output| output.contract_id.clone())
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect(),
                scope_summary: projection.human.scope_summary.clone(),
            }
        })
        .collect()
}

pub(crate) fn reviewer_work_items(
    ordered_ids: &[String],
    work_items: &BTreeMap<String, CompiledWorkItemProjections>,
) -> Vec<ReviewerGroupMatrixEntry> {
    ordered_ids
        .iter()
        .map(|logical_id| {
            let reviewer = &work_items[logical_id].reviewer;
            ReviewerGroupMatrixEntry {
                logical_work_item_id: logical_id.clone(),
                criterion_refs: reviewer.criterion_refs.clone(),
                input_contract_refs: reviewer
                    .input_contract_checks
                    .iter()
                    .map(|input| input.contract_id.clone())
                    .collect(),
                output_contract_refs: reviewer
                    .output_contract_checks
                    .iter()
                    .map(|output| output.contract_id.clone())
                    .collect(),
            }
        })
        .collect()
}

pub(crate) fn design_traceability_refs(
    graph: &DependencyContractGraph,
) -> Vec<DesignTraceabilityRef> {
    graph
        .contracts
        .values()
        .flat_map(|contract| contract.design_traceability.iter())
        .map(|trace| {
            (
                (
                    trace.source_type.clone(),
                    trace.source_id.clone(),
                    trace.requirement_id.clone(),
                ),
                trace.clone(),
            )
        })
        .collect::<BTreeMap<_, _>>()
        .into_values()
        .collect()
}

fn validation_error<T>(code: &str, message: &str) -> Result<T, ProjectionCompileError> {
    Err(ProjectionCompileError::Validation(
        ProjectionValidationReport {
            findings: vec![ProjectionValidationFinding {
                code: code.to_string(),
                projection: "plan".to_string(),
                contract_ref: None,
                message: message.to_string(),
            }],
        },
    ))
}
