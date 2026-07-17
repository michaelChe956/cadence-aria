use std::collections::{BTreeMap, BTreeSet, VecDeque};

use serde::{Deserialize, Serialize};

use crate::product::work_item_contract::{
    CanonicalWorkItemContract, ContractCompatibilityPolicy, DependencyContractEdge,
    DependencyContractGraph, RequiredDependencyContract,
};

use super::{ContractDelta, ContractDeltaKind, PlanRepairError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnitExecutionSnapshot {
    pub logical_work_item_id: String,
    pub work_item_revision_id: String,
    pub completed_handoff_revision_id: Option<String>,
    pub has_started: bool,
    pub has_completed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanExecutionState {
    pub units: BTreeMap<String, UnitExecutionSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImpactExplanationPath {
    pub from: String,
    pub to: String,
    pub contract_id: String,
    pub capability_refs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContractImpactReport {
    pub unaffected: Vec<String>,
    pub direct_revalidation: Vec<String>,
    pub direct_stale: Vec<String>,
    pub conditional_downstream: Vec<String>,
    pub explanation_paths: Vec<ImpactExplanationPath>,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct ContractImpactAnalyzer;

impl ContractImpactAnalyzer {
    pub fn analyze_static(
        &self,
        graph: &DependencyContractGraph,
        delta: &ContractDelta,
        execution: &PlanExecutionState,
    ) -> Result<ContractImpactReport, PlanRepairError> {
        if !graph.contracts.contains_key(&delta.logical_work_item_id) {
            return Err(PlanRepairError::InvalidFinding(format!(
                "contract delta source {} is missing from dependency graph",
                delta.logical_work_item_id
            )));
        }

        if delta.kind == ContractDeltaKind::TopologyChange {
            return Ok(empty_report());
        }

        if matches!(
            delta.kind,
            ContractDeltaKind::InformativeOnly | ContractDeltaKind::ImplementationGuidance
        ) {
            return Ok(ContractImpactReport {
                unaffected: unaffected_units(graph, delta, &BTreeSet::new(), &BTreeSet::new()),
                ..empty_report()
            });
        }

        let mut direct_revalidation = BTreeSet::new();
        let mut direct_stale = BTreeSet::new();
        let mut explanation_paths = Vec::new();
        let provider = graph
            .contracts
            .get(&delta.logical_work_item_id)
            .expect("delta source existence was checked above");
        for edge in graph
            .edges
            .iter()
            .filter(|edge| edge.from == delta.logical_work_item_id)
        {
            let matching_contracts = edge
                .required_contracts
                .iter()
                .filter(|required| required_contract_is_impacted(required, delta, provider))
                .collect::<Vec<_>>();
            if matching_contracts.is_empty() {
                continue;
            }

            match delta.kind {
                ContractDeltaKind::BreakingContractChange
                    if execution
                        .units
                        .get(&edge.to)
                        .is_some_and(|unit| unit.has_started || unit.has_completed) =>
                {
                    direct_stale.insert(edge.to.clone());
                }
                ContractDeltaKind::BreakingContractChange
                | ContractDeltaKind::CompatibleContractExtension => {
                    direct_revalidation.insert(edge.to.clone());
                }
                ContractDeltaKind::InformativeOnly
                | ContractDeltaKind::ImplementationGuidance
                | ContractDeltaKind::TopologyChange => {}
            }
            explanation_paths.extend(
                matching_contracts
                    .into_iter()
                    .map(|required| explanation_path(edge, required)),
            );
        }

        let direct = direct_revalidation
            .union(&direct_stale)
            .cloned()
            .collect::<BTreeSet<_>>();
        let (conditional_downstream, downstream_paths) =
            collect_conditional_downstream(graph, delta, &direct);
        explanation_paths.extend(downstream_paths);
        sort_and_deduplicate_paths(&mut explanation_paths);

        Ok(ContractImpactReport {
            unaffected: unaffected_units(graph, delta, &direct, &conditional_downstream),
            direct_revalidation: direct_revalidation.into_iter().collect(),
            direct_stale: direct_stale.into_iter().collect(),
            conditional_downstream: conditional_downstream.into_iter().collect(),
            explanation_paths,
        })
    }
}

fn required_contract_is_impacted(
    required: &RequiredDependencyContract,
    delta: &ContractDelta,
    next_provider: &CanonicalWorkItemContract,
) -> bool {
    let required_capabilities = required
        .required_capabilities
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let next_provided = provided_capabilities(next_provider, &required.contract_id);
    match delta.kind {
        ContractDeltaKind::BreakingContractChange => {
            if delta.removed_contracts.contains(&required.contract_id) {
                return true;
            }
            let relevant_loss = delta
                .removed_capability_associations
                .iter()
                .any(|association| {
                    association.contract_id == required.contract_id
                        && required_capabilities.contains(&association.capability)
                });
            relevant_loss && !compatibility_policy_is_satisfied(required, next_provided.as_ref())
        }
        ContractDeltaKind::CompatibleContractExtension => {
            let contract_added =
                delta.added_contracts.contains(&required.contract_id) && next_provided.is_some();
            let newly_available = delta
                .added_capability_associations
                .iter()
                .filter(|association| {
                    association.contract_id == required.contract_id
                        && required_capabilities.contains(&association.capability)
                        && next_provided
                            .as_ref()
                            .is_some_and(|provided| provided.contains(&association.capability))
                })
                .map(|association| association.capability.clone())
                .collect::<BTreeSet<_>>();

            match required.compatibility_policy {
                ContractCompatibilityPolicy::RequireAll => {
                    contract_added || !newly_available.is_empty()
                }
                ContractCompatibilityPolicy::RequireAny => {
                    if required_capabilities.is_empty() {
                        return contract_added;
                    }
                    let next_available = next_provided
                        .as_ref()
                        .map(|provided| {
                            required_capabilities
                                .intersection(provided)
                                .cloned()
                                .collect::<BTreeSet<_>>()
                        })
                        .unwrap_or_default();
                    let previous_available = if contract_added {
                        BTreeSet::new()
                    } else {
                        next_available
                            .difference(&newly_available)
                            .cloned()
                            .collect()
                    };
                    previous_available.is_empty() && !next_available.is_empty()
                }
            }
        }
        ContractDeltaKind::InformativeOnly
        | ContractDeltaKind::ImplementationGuidance
        | ContractDeltaKind::TopologyChange => false,
    }
}

fn provided_capabilities(
    provider: &CanonicalWorkItemContract,
    contract_id: &str,
) -> Option<BTreeSet<String>> {
    let matching_outputs = provider
        .output_contracts
        .iter()
        .filter(|output| output.contract_id == contract_id)
        .collect::<Vec<_>>();
    if matching_outputs.is_empty() {
        return None;
    }
    Some(
        matching_outputs
            .into_iter()
            .flat_map(|output| output.capabilities.iter().cloned())
            .collect(),
    )
}

fn compatibility_policy_is_satisfied(
    required: &RequiredDependencyContract,
    provided: Option<&BTreeSet<String>>,
) -> bool {
    let Some(provided) = provided else {
        return false;
    };
    let required_capabilities = required
        .required_capabilities
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    match required.compatibility_policy {
        ContractCompatibilityPolicy::RequireAll => required_capabilities.is_subset(provided),
        ContractCompatibilityPolicy::RequireAny => {
            required_capabilities.is_empty() || !required_capabilities.is_disjoint(provided)
        }
    }
}

fn collect_conditional_downstream(
    graph: &DependencyContractGraph,
    delta: &ContractDelta,
    direct: &BTreeSet<String>,
) -> (BTreeSet<String>, Vec<ImpactExplanationPath>) {
    let mut queue = direct.iter().cloned().collect::<VecDeque<_>>();
    let mut visited = direct.clone();
    visited.insert(delta.logical_work_item_id.clone());
    let mut conditional = BTreeSet::new();
    let mut explanation_paths = Vec::new();

    while let Some(from) = queue.pop_front() {
        let mut outgoing = graph
            .edges
            .iter()
            .filter(|edge| edge.from == from)
            .collect::<Vec<_>>();
        outgoing.sort_by(|left, right| {
            (&left.to, &left.required_contracts.len())
                .cmp(&(&right.to, &right.required_contracts.len()))
        });
        for edge in outgoing {
            if edge.to == delta.logical_work_item_id || direct.contains(&edge.to) {
                continue;
            }
            explanation_paths.extend(
                edge.required_contracts
                    .iter()
                    .map(|required| explanation_path(edge, required)),
            );
            conditional.insert(edge.to.clone());
            if visited.insert(edge.to.clone()) {
                queue.push_back(edge.to.clone());
            }
        }
    }

    (conditional, explanation_paths)
}

fn explanation_path(
    edge: &DependencyContractEdge,
    required: &RequiredDependencyContract,
) -> ImpactExplanationPath {
    ImpactExplanationPath {
        from: edge.from.clone(),
        to: edge.to.clone(),
        contract_id: required.contract_id.clone(),
        capability_refs: required
            .required_capabilities
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect(),
    }
}

fn sort_and_deduplicate_paths(paths: &mut Vec<ImpactExplanationPath>) {
    paths.sort_by(|left, right| {
        (
            &left.from,
            &left.to,
            &left.contract_id,
            &left.capability_refs,
        )
            .cmp(&(
                &right.from,
                &right.to,
                &right.contract_id,
                &right.capability_refs,
            ))
    });
    paths.dedup();
}

fn unaffected_units(
    graph: &DependencyContractGraph,
    delta: &ContractDelta,
    direct: &BTreeSet<String>,
    conditional: &BTreeSet<String>,
) -> Vec<String> {
    graph
        .contracts
        .keys()
        .filter(|logical_work_item_id| {
            *logical_work_item_id != &delta.logical_work_item_id
                && !direct.contains(*logical_work_item_id)
                && !conditional.contains(*logical_work_item_id)
        })
        .cloned()
        .collect()
}

fn empty_report() -> ContractImpactReport {
    ContractImpactReport {
        unaffected: Vec::new(),
        direct_revalidation: Vec::new(),
        direct_stale: Vec::new(),
        conditional_downstream: Vec::new(),
        explanation_paths: Vec::new(),
    }
}
