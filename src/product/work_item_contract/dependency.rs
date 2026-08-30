use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::cross_cutting::worktree::scopes_may_overlap;

use super::{
    CanonicalWorkItemContract, ContractCompatibilityPolicy, ContractValidationFinding,
    ContractValidationReport,
    validation::{error_finding, sorted_report},
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequiredDependencyContract {
    pub contract_id: String,
    pub required_capabilities: Vec<String>,
    pub compatibility_policy: ContractCompatibilityPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DependencyContractEdge {
    pub from: String,
    pub to: String,
    pub required_contracts: Vec<RequiredDependencyContract>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DependencyContractGraph {
    pub contracts: BTreeMap<String, CanonicalWorkItemContract>,
    pub edges: Vec<DependencyContractEdge>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ContractCapabilityCoverage {
    pub from: String,
    pub to: String,
    pub contract_id: String,
    pub required_capabilities: Vec<String>,
    pub provided_capabilities: Vec<String>,
    pub missing_capabilities: Vec<String>,
    pub compatibility_policy: ContractCompatibilityPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ContractHandoffConsumption {
    pub provider: String,
    pub contract_ref: String,
    pub consumers: Vec<String>,
    pub consumed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ContractDependencyItemFact {
    pub work_item_id: String,
    pub depends_on: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ContractDependencyDuplicateFact {
    pub from: String,
    pub to: String,
    pub contract_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ContractUnknownProviderFact {
    pub provider: String,
    pub consumer: String,
    pub contract_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ContractDependencyEdgeFact {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ContractDependencyGraphFacts {
    pub depends_on: Vec<ContractDependencyItemFact>,
    pub declared_edges: Vec<ContractDependencyEdgeFact>,
    pub contract_edges: Vec<DependencyContractEdge>,
    pub cycles: Vec<Vec<String>>,
    pub duplicate_edges: Vec<ContractDependencyDuplicateFact>,
    pub unknown_providers: Vec<ContractUnknownProviderFact>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ContractWriteScopeKind {
    Exclusive,
    Forbidden,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ContractWriteScopeConflict {
    pub left_work_item_id: String,
    pub left_kind: ContractWriteScopeKind,
    pub left_scope: String,
    pub right_work_item_id: String,
    pub right_kind: ContractWriteScopeKind,
    pub right_scope: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ContractReviewerCoverageProjection {
    pub capability_coverage: Vec<ContractCapabilityCoverage>,
    pub dependency_graph: ContractDependencyGraphFacts,
    pub handoff_consumption: Vec<ContractHandoffConsumption>,
    pub write_scope_conflicts: Vec<ContractWriteScopeConflict>,
}

pub fn build_dependency_contract_graph(
    contracts: &[CanonicalWorkItemContract],
) -> Result<DependencyContractGraph, ContractValidationReport> {
    let mut indexed_contracts = BTreeMap::new();
    let mut findings = Vec::new();

    for contract in contracts {
        let logical_work_item_id = &contract.identity.logical_work_item_id;
        if indexed_contracts
            .insert(logical_work_item_id.clone(), contract.clone())
            .is_some()
        {
            findings.push(error_finding(
                "duplicate_logical_work_item_identity",
                logical_work_item_id,
                None,
                None,
                format!("logical work item identity {logical_work_item_id} is not unique"),
            ));
        }
    }

    if !findings.is_empty() {
        return Err(sorted_report(findings));
    }

    let mut required_contracts_by_edge =
        BTreeMap::<(String, String), Vec<RequiredDependencyContract>>::new();
    for (consumer_id, consumer) in &indexed_contracts {
        for input in &consumer.input_contracts {
            let mut required_capabilities = input.required_capabilities.clone();
            required_capabilities.sort();
            required_contracts_by_edge
                .entry((
                    input.provider_logical_work_item_id.clone(),
                    consumer_id.clone(),
                ))
                .or_default()
                .push(RequiredDependencyContract {
                    contract_id: input.contract_id.clone(),
                    required_capabilities,
                    compatibility_policy: input.compatibility_policy.clone(),
                });
        }
    }

    let edges = required_contracts_by_edge
        .into_iter()
        .map(|((from, to), mut required_contracts)| {
            required_contracts.sort_by(|left, right| {
                (
                    &left.contract_id,
                    compatibility_policy_rank(&left.compatibility_policy),
                    &left.required_capabilities,
                )
                    .cmp(&(
                        &right.contract_id,
                        compatibility_policy_rank(&right.compatibility_policy),
                        &right.required_capabilities,
                    ))
            });
            DependencyContractEdge {
                from,
                to,
                required_contracts,
            }
        })
        .collect();

    Ok(DependencyContractGraph {
        contracts: indexed_contracts,
        edges,
    })
}

pub fn validate_dependency_contract_graph(
    graph: &DependencyContractGraph,
) -> ContractValidationReport {
    let mut findings = Vec::new();

    report_duplicate_edges(graph, &mut findings);
    report_dependency_cycles(graph, &mut findings);
    report_contract_requirements(graph, &mut findings);
    report_unconsumed_handoffs(graph, &mut findings);

    sorted_report(findings)
}

fn duplicate_edge_facts(graph: &DependencyContractGraph) -> Vec<ContractDependencyDuplicateFact> {
    let mut facts = Vec::new();
    let mut seen = BTreeSet::new();
    for edge in &graph.edges {
        if !seen.insert((edge.from.as_str(), edge.to.as_str())) {
            facts.push(ContractDependencyDuplicateFact {
                from: edge.from.clone(),
                to: edge.to.clone(),
                contract_id: None,
            });
        }
        let mut seen_required_contracts = BTreeSet::new();
        for required in &edge.required_contracts {
            if !seen_required_contracts.insert(required.contract_id.as_str()) {
                facts.push(ContractDependencyDuplicateFact {
                    from: edge.from.clone(),
                    to: edge.to.clone(),
                    contract_id: Some(required.contract_id.clone()),
                });
            }
        }
    }
    facts.sort_by(|left, right| {
        (&left.from, &left.to, &left.contract_id).cmp(&(&right.from, &right.to, &right.contract_id))
    });
    facts
}

fn report_duplicate_edges(
    graph: &DependencyContractGraph,
    findings: &mut Vec<ContractValidationFinding>,
) {
    for fact in duplicate_edge_facts(graph) {
        if let Some(contract_id) = fact.contract_id {
            findings.push(error_finding(
                "duplicate_dependency_contract_edge",
                &fact.to,
                Some(&contract_id),
                None,
                format!(
                    "dependency contract {} is duplicated on edge {} -> {}",
                    contract_id, fact.from, fact.to
                ),
            ));
        } else {
            findings.push(error_finding(
                "duplicate_dependency_contract_edge",
                &fact.to,
                None,
                None,
                format!(
                    "dependency contract edge {} -> {} is duplicated",
                    fact.from, fact.to
                ),
            ));
        }
    }
}

fn provided_capabilities_for_contract<'a>(
    provider: Option<&'a CanonicalWorkItemContract>,
    contract_id: &str,
) -> BTreeSet<&'a str> {
    provider
        .into_iter()
        .flat_map(|provider| provider.output_contracts.iter())
        .filter(|output| output.contract_id == contract_id)
        .flat_map(|output| output.capabilities.iter().map(String::as_str))
        .collect()
}

fn missing_capabilities_for_policy(
    required_capabilities: &[String],
    provided_capabilities: &BTreeSet<&str>,
    compatibility_policy: &ContractCompatibilityPolicy,
) -> Vec<String> {
    let required_capabilities = required_capabilities
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    match compatibility_policy {
        ContractCompatibilityPolicy::RequireAll => required_capabilities
            .difference(provided_capabilities)
            .map(|capability| (*capability).to_string())
            .collect(),
        ContractCompatibilityPolicy::RequireAny
            if !required_capabilities.is_empty()
                && required_capabilities.is_disjoint(provided_capabilities) =>
        {
            required_capabilities
                .into_iter()
                .map(str::to_string)
                .collect()
        }
        ContractCompatibilityPolicy::RequireAny => Vec::new(),
    }
}

pub(crate) fn project_contract_capability_coverage(
    graph: &DependencyContractGraph,
) -> Vec<ContractCapabilityCoverage> {
    graph
        .edges
        .iter()
        .flat_map(|edge| {
            edge.required_contracts.iter().map(|required| {
                let provided_capabilities = provided_capabilities_for_contract(
                    graph.contracts.get(&edge.from),
                    &required.contract_id,
                );
                let provided_capabilities = provided_capabilities
                    .iter()
                    .map(|capability| (*capability).to_string())
                    .collect::<Vec<_>>();
                let provided_set = provided_capabilities
                    .iter()
                    .map(String::as_str)
                    .collect::<BTreeSet<_>>();
                ContractCapabilityCoverage {
                    from: edge.from.clone(),
                    to: edge.to.clone(),
                    contract_id: required.contract_id.clone(),
                    required_capabilities: required
                        .required_capabilities
                        .iter()
                        .cloned()
                        .collect::<BTreeSet<_>>()
                        .into_iter()
                        .collect(),
                    missing_capabilities: missing_capabilities_for_policy(
                        &required.required_capabilities,
                        &provided_set,
                        &required.compatibility_policy,
                    ),
                    provided_capabilities,
                    compatibility_policy: required.compatibility_policy.clone(),
                }
            })
        })
        .collect()
}

fn unknown_provider_facts(graph: &DependencyContractGraph) -> Vec<ContractUnknownProviderFact> {
    let mut facts = graph
        .edges
        .iter()
        .filter(|edge| !graph.contracts.contains_key(&edge.from))
        .flat_map(|edge| {
            edge.required_contracts
                .iter()
                .map(|required| ContractUnknownProviderFact {
                    provider: edge.from.clone(),
                    consumer: edge.to.clone(),
                    contract_id: required.contract_id.clone(),
                })
        })
        .collect::<Vec<_>>();
    facts.sort_by(|left, right| {
        (&left.provider, &left.consumer, &left.contract_id).cmp(&(
            &right.provider,
            &right.consumer,
            &right.contract_id,
        ))
    });
    facts
}

fn report_contract_requirements(
    graph: &DependencyContractGraph,
    findings: &mut Vec<ContractValidationFinding>,
) {
    for fact in unknown_provider_facts(graph) {
        findings.push(error_finding(
            "unknown_provider_logical_work_item",
            &fact.provider,
            Some(&fact.contract_id),
            None,
            format!(
                "consumer {} references unknown provider {} for contract {}",
                fact.consumer, fact.provider, fact.contract_id
            ),
        ));
    }
    for edge in graph
        .edges
        .iter()
        .filter(|edge| graph.contracts.contains_key(&edge.from))
    {
        let provider = graph
            .contracts
            .get(&edge.from)
            .expect("existing provider must be available");
        for required in &edge.required_contracts {
            let provided_capabilities =
                provided_capabilities_for_contract(Some(provider), &required.contract_id);
            if provided_capabilities.is_empty()
                && !provider
                    .output_contracts
                    .iter()
                    .any(|output| output.contract_id == required.contract_id)
            {
                findings.push(error_finding(
                    "required_contract_missing",
                    &edge.to,
                    Some(&required.contract_id),
                    None,
                    format!(
                        "provider {} does not provide contract {} required by {}",
                        edge.from, required.contract_id, edge.to
                    ),
                ));
                continue;
            }

            let missing_capabilities = missing_capabilities_for_policy(
                &required.required_capabilities,
                &provided_capabilities,
                &required.compatibility_policy,
            );

            for capability in missing_capabilities {
                findings.push(error_finding(
                    "required_capability_missing",
                    &edge.to,
                    Some(&required.contract_id),
                    Some(&capability),
                    format!(
                        "provider {} contract {} lacks capability {capability} required by {}",
                        edge.from, required.contract_id, edge.to
                    ),
                ));
            }
        }
    }
}

pub(crate) fn project_contract_handoff_consumption(
    graph: &DependencyContractGraph,
) -> Vec<ContractHandoffConsumption> {
    graph
        .contracts
        .iter()
        .flat_map(|(provider, contract)| {
            contract
                .handoff_contract
                .provided_contract_refs
                .iter()
                .collect::<BTreeSet<_>>()
                .into_iter()
                .map(|contract_ref| {
                    let mut consumers = graph
                        .contracts
                        .values()
                        .filter(|consumer| {
                            consumer.input_contracts.iter().any(|input| {
                                input.provider_logical_work_item_id == *provider
                                    && input.contract_id == *contract_ref
                            })
                        })
                        .map(|consumer| consumer.identity.logical_work_item_id.clone())
                        .collect::<Vec<_>>();
                    consumers.sort();
                    consumers.dedup();
                    ContractHandoffConsumption {
                        provider: provider.clone(),
                        contract_ref: contract_ref.clone(),
                        consumed: !consumers.is_empty(),
                        consumers,
                    }
                })
        })
        .collect()
}

fn report_unconsumed_handoffs(
    graph: &DependencyContractGraph,
    findings: &mut Vec<ContractValidationFinding>,
) {
    for handoff in project_contract_handoff_consumption(graph)
        .into_iter()
        .filter(|handoff| !handoff.consumed)
    {
        findings.push(error_finding(
            "unconsumed_required_handoff",
            &handoff.provider,
            Some(&handoff.contract_ref),
            None,
            format!(
                "handoff contract {} from {} is not referenced by a consumer input contract",
                handoff.contract_ref, handoff.provider
            ),
        ));
    }
}

fn dependency_cycle_facts(graph: &DependencyContractGraph) -> Vec<Vec<String>> {
    let mut adjacency = graph
        .contracts
        .keys()
        .map(|logical_work_item_id| (logical_work_item_id.as_str(), BTreeSet::new()))
        .collect::<BTreeMap<_, _>>();
    for edge in &graph.edges {
        if graph.contracts.contains_key(&edge.from) && graph.contracts.contains_key(&edge.to) {
            adjacency
                .entry(edge.from.as_str())
                .or_default()
                .insert(edge.to.as_str());
        }
    }

    let mut states = BTreeMap::new();
    let mut stack = Vec::new();
    let mut cycles = BTreeSet::new();
    for node in adjacency.keys().copied() {
        if states.get(node).copied().unwrap_or(VisitState::Unvisited) == VisitState::Unvisited {
            visit_node(node, &adjacency, &mut states, &mut stack, &mut cycles);
        }
    }
    cycles
        .into_iter()
        .map(|cycle| cycle.into_iter().map(str::to_string).collect())
        .collect()
}

fn report_dependency_cycles(
    graph: &DependencyContractGraph,
    findings: &mut Vec<ContractValidationFinding>,
) {
    for cycle in dependency_cycle_facts(graph) {
        let logical_work_item_id = cycle.first().expect("cycle must contain a node");
        findings.push(error_finding(
            "dependency_cycle",
            logical_work_item_id,
            None,
            None,
            format!("dependency cycle contains {}", cycle.join(" -> ")),
        ));
    }
}

fn visit_node<'a>(
    node: &'a str,
    adjacency: &BTreeMap<&'a str, BTreeSet<&'a str>>,
    states: &mut BTreeMap<&'a str, VisitState>,
    stack: &mut Vec<&'a str>,
    cycles: &mut BTreeSet<Vec<&'a str>>,
) {
    states.insert(node, VisitState::Visiting);
    stack.push(node);

    if let Some(neighbors) = adjacency.get(node) {
        for neighbor in neighbors {
            match states
                .get(neighbor)
                .copied()
                .unwrap_or(VisitState::Unvisited)
            {
                VisitState::Unvisited => {
                    visit_node(neighbor, adjacency, states, stack, cycles);
                }
                VisitState::Visiting => {
                    if let Some(cycle_start) =
                        stack.iter().position(|stack_node| stack_node == neighbor)
                    {
                        let mut cycle = stack[cycle_start..].to_vec();
                        cycle.sort_unstable();
                        cycle.dedup();
                        cycles.insert(cycle);
                    }
                }
                VisitState::Visited => {}
            }
        }
    }

    stack.pop();
    states.insert(node, VisitState::Visited);
}

pub(crate) fn project_contract_reviewer_coverage(
    graph: &DependencyContractGraph,
) -> ContractReviewerCoverageProjection {
    ContractReviewerCoverageProjection {
        capability_coverage: project_contract_capability_coverage(graph),
        dependency_graph: project_contract_dependency_graph_facts(graph),
        handoff_consumption: project_contract_handoff_consumption(graph),
        write_scope_conflicts: project_contract_write_scope_conflicts(graph),
    }
}

fn project_contract_dependency_graph_facts(
    graph: &DependencyContractGraph,
) -> ContractDependencyGraphFacts {
    let depends_on = graph
        .contracts
        .iter()
        .map(|(work_item_id, contract)| ContractDependencyItemFact {
            work_item_id: work_item_id.clone(),
            depends_on: contract
                .depends_on
                .iter()
                .cloned()
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect(),
        })
        .collect::<Vec<_>>();
    let mut declared_edges = depends_on
        .iter()
        .flat_map(|item| {
            item.depends_on
                .iter()
                .map(|dependency| ContractDependencyEdgeFact {
                    from: dependency.clone(),
                    to: item.work_item_id.clone(),
                })
        })
        .collect::<Vec<_>>();
    declared_edges.sort_by(|left, right| (&left.from, &left.to).cmp(&(&right.from, &right.to)));
    let mut contract_edges = graph.edges.clone();
    contract_edges.sort_by(|left, right| {
        let left_contracts = left
            .required_contracts
            .iter()
            .map(|required| {
                (
                    required.contract_id.as_str(),
                    compatibility_policy_rank(&required.compatibility_policy),
                    required.required_capabilities.as_slice(),
                )
            })
            .collect::<Vec<_>>();
        let right_contracts = right
            .required_contracts
            .iter()
            .map(|required| {
                (
                    required.contract_id.as_str(),
                    compatibility_policy_rank(&required.compatibility_policy),
                    required.required_capabilities.as_slice(),
                )
            })
            .collect::<Vec<_>>();
        (&left.from, &left.to, left_contracts).cmp(&(&right.from, &right.to, right_contracts))
    });
    ContractDependencyGraphFacts {
        depends_on,
        declared_edges,
        contract_edges,
        cycles: dependency_cycle_facts(graph),
        duplicate_edges: duplicate_edge_facts(graph),
        unknown_providers: unknown_provider_facts(graph),
    }
}

fn project_contract_write_scope_conflicts(
    graph: &DependencyContractGraph,
) -> Vec<ContractWriteScopeConflict> {
    let contracts = graph.contracts.iter().collect::<Vec<_>>();
    let mut facts = Vec::new();
    for (index, (left_id, left)) in contracts.iter().enumerate() {
        for (right_id, right) in contracts.iter().skip(index + 1) {
            for (left_kind, left_scopes) in [
                (
                    ContractWriteScopeKind::Exclusive,
                    &left.write_policy.exclusive_scopes,
                ),
                (
                    ContractWriteScopeKind::Forbidden,
                    &left.write_policy.forbidden_scopes,
                ),
            ] {
                for (right_kind, right_scopes) in [
                    (
                        ContractWriteScopeKind::Exclusive,
                        &right.write_policy.exclusive_scopes,
                    ),
                    (
                        ContractWriteScopeKind::Forbidden,
                        &right.write_policy.forbidden_scopes,
                    ),
                ] {
                    if matches!(left_kind, ContractWriteScopeKind::Forbidden)
                        && matches!(right_kind, ContractWriteScopeKind::Forbidden)
                    {
                        continue;
                    }
                    for left_scope in left_scopes {
                        for right_scope in right_scopes {
                            if scopes_may_overlap(
                                std::slice::from_ref(left_scope),
                                std::slice::from_ref(right_scope),
                                true,
                            ) {
                                facts.push(ContractWriteScopeConflict {
                                    left_work_item_id: (*left_id).clone(),
                                    left_kind: left_kind.clone(),
                                    left_scope: left_scope.clone(),
                                    right_work_item_id: (*right_id).clone(),
                                    right_kind: right_kind.clone(),
                                    right_scope: right_scope.clone(),
                                });
                            }
                        }
                    }
                }
            }
        }
    }
    facts.sort_by(|left, right| {
        (
            &left.left_work_item_id,
            &left.left_kind,
            &left.left_scope,
            &left.right_work_item_id,
            &left.right_kind,
            &left.right_scope,
        )
            .cmp(&(
                &right.left_work_item_id,
                &right.left_kind,
                &right.left_scope,
                &right.right_work_item_id,
                &right.right_kind,
                &right.right_scope,
            ))
    });
    facts
}

fn compatibility_policy_rank(policy: &ContractCompatibilityPolicy) -> u8 {
    match policy {
        ContractCompatibilityPolicy::RequireAll => 0,
        ContractCompatibilityPolicy::RequireAny => 1,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VisitState {
    Unvisited,
    Visiting,
    Visited,
}
