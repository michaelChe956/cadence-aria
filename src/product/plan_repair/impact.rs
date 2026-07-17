use std::collections::{BTreeMap, BTreeSet, VecDeque};

use serde::{Deserialize, Serialize};

use crate::product::work_item_contract::{
    DependencyContractEdge, DependencyContractGraph, RequiredDependencyContract,
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
        for edge in graph
            .edges
            .iter()
            .filter(|edge| edge.from == delta.logical_work_item_id)
        {
            let matching_contracts = edge
                .required_contracts
                .iter()
                .filter(|required| required_contract_matches(required, delta))
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

fn required_contract_matches(required: &RequiredDependencyContract, delta: &ContractDelta) -> bool {
    let required_capabilities = required
        .required_capabilities
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    match delta.kind {
        ContractDeltaKind::BreakingContractChange => {
            delta.removed_contracts.contains(&required.contract_id)
                || delta
                    .removed_capabilities
                    .iter()
                    .chain(&delta.changed_capabilities)
                    .any(|capability| required_capabilities.contains(capability.as_str()))
        }
        ContractDeltaKind::CompatibleContractExtension => {
            delta.added_contracts.contains(&required.contract_id)
                || delta
                    .added_capabilities
                    .iter()
                    .any(|capability| required_capabilities.contains(capability.as_str()))
        }
        ContractDeltaKind::InformativeOnly
        | ContractDeltaKind::ImplementationGuidance
        | ContractDeltaKind::TopologyChange => false,
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
