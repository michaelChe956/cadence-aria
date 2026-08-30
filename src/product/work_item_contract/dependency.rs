use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

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

fn report_duplicate_edges(
    graph: &DependencyContractGraph,
    findings: &mut Vec<ContractValidationFinding>,
) {
    let mut seen = BTreeSet::new();
    for edge in &graph.edges {
        if !seen.insert((edge.from.as_str(), edge.to.as_str())) {
            findings.push(error_finding(
                "duplicate_dependency_contract_edge",
                &edge.to,
                None,
                None,
                format!(
                    "dependency contract edge {} -> {} is duplicated",
                    edge.from, edge.to
                ),
            ));
        }

        let mut seen_required_contracts = BTreeSet::new();
        for required in &edge.required_contracts {
            if !seen_required_contracts.insert(required.contract_id.as_str()) {
                findings.push(error_finding(
                    "duplicate_dependency_contract_edge",
                    &edge.to,
                    Some(&required.contract_id),
                    None,
                    format!(
                        "dependency contract {} is duplicated on edge {} -> {}",
                        required.contract_id, edge.from, edge.to
                    ),
                ));
            }
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

fn report_contract_requirements(
    graph: &DependencyContractGraph,
    findings: &mut Vec<ContractValidationFinding>,
) {
    for edge in &graph.edges {
        let Some(provider) = graph.contracts.get(&edge.from) else {
            for required in &edge.required_contracts {
                findings.push(error_finding(
                    "unknown_provider_logical_work_item",
                    &edge.from,
                    Some(&required.contract_id),
                    None,
                    format!(
                        "consumer {} references unknown provider {} for contract {}",
                        edge.to, edge.from, required.contract_id
                    ),
                ));
            }
            continue;
        };

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

fn report_unconsumed_handoffs(
    graph: &DependencyContractGraph,
    findings: &mut Vec<ContractValidationFinding>,
) {
    let consumed_contracts = graph
        .contracts
        .values()
        .flat_map(|consumer| {
            consumer.input_contracts.iter().map(|input| {
                (
                    input.provider_logical_work_item_id.as_str(),
                    input.contract_id.as_str(),
                )
            })
        })
        .collect::<BTreeSet<_>>();

    for (logical_work_item_id, contract) in &graph.contracts {
        for contract_ref in contract
            .handoff_contract
            .provided_contract_refs
            .iter()
            .collect::<BTreeSet<_>>()
        {
            if !consumed_contracts.contains(&(logical_work_item_id.as_str(), contract_ref.as_str()))
            {
                findings.push(error_finding(
                    "unconsumed_required_handoff",
                    logical_work_item_id,
                    Some(contract_ref),
                    None,
                    format!(
                        "handoff contract {contract_ref} from {logical_work_item_id} is not referenced by a consumer input contract"
                    ),
                ));
            }
        }
    }
}

fn report_dependency_cycles(
    graph: &DependencyContractGraph,
    findings: &mut Vec<ContractValidationFinding>,
) {
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

    for cycle in cycles {
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
