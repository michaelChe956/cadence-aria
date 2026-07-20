use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::product::models::DependencyGraphRevision;
use crate::product::work_item_contract::{
    CanonicalWorkItemContract, ContractCompatibilityPolicy, DependencyContractEdge,
    DependencyContractGraph, RequiredDependencyContract, RequiredInputContract,
};

use super::PlanRepairError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubgraphReplanRequest {
    pub plan_id: String,
    pub dependency_graph_revision_id: String,
    pub changed_logical_work_item_ids: Vec<String>,
    pub replacement_contracts: Vec<CanonicalWorkItemContract>,
    pub replacement_mapping: BTreeMap<String, Vec<String>>,
    pub story_spec_refs_changed: bool,
    pub design_spec_refs_changed: bool,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubgraphReplanResult {
    pub input_boundary: Vec<String>,
    pub output_boundary: Vec<String>,
    pub affected_logical_work_items: Vec<String>,
    pub replacement_mapping: BTreeMap<String, Vec<String>>,
    pub dependency_graph_revision: DependencyGraphRevision,
    pub full_replan_required: bool,
}

#[derive(Debug, Default)]
pub struct SubgraphReplanner {
    _private: (),
}

impl SubgraphReplanner {
    pub fn replan(
        &self,
        graph: &DependencyContractGraph,
        request: &SubgraphReplanRequest,
    ) -> Result<SubgraphReplanResult, PlanRepairError> {
        let replacement_contracts = validate_request(graph, request)?;
        let changed = request
            .changed_logical_work_item_ids
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>();
        let mut affected = changed.clone();

        loop {
            let boundaries = boundaries(graph, &affected);
            let mut expansion = BTreeSet::new();
            for edge in &boundaries.input_edges {
                if !input_boundary_satisfied(graph, request, &replacement_contracts, edge) {
                    expansion.insert(edge.from.clone());
                }
            }
            for edge in &boundaries.output_edges {
                if !output_boundary_satisfied(request, &replacement_contracts, edge) {
                    expansion.insert(edge.to.clone());
                }
            }
            let before = affected.len();
            affected.extend(expansion);
            if affected.len() == before {
                break;
            }
        }

        let boundaries = boundaries(graph, &affected);
        let dependency_graph_revision =
            build_dependency_graph_revision(graph, request, &replacement_contracts, &changed)?;
        Ok(SubgraphReplanResult {
            input_boundary: boundary_nodes(&boundaries.input_edges, |edge| &edge.from),
            output_boundary: boundary_nodes(&boundaries.output_edges, |edge| &edge.to),
            affected_logical_work_items: affected.iter().cloned().collect(),
            replacement_mapping: normalized_mapping(&request.replacement_mapping),
            dependency_graph_revision,
            full_replan_required: affected.len() == graph.contracts.len()
                || request.story_spec_refs_changed
                || request.design_spec_refs_changed,
        })
    }
}

struct BoundaryEdges<'a> {
    input_edges: Vec<&'a DependencyContractEdge>,
    output_edges: Vec<&'a DependencyContractEdge>,
}

fn validate_request<'a>(
    graph: &DependencyContractGraph,
    request: &'a SubgraphReplanRequest,
) -> Result<BTreeMap<&'a str, &'a CanonicalWorkItemContract>, PlanRepairError> {
    for (field, value) in [
        ("plan_id", request.plan_id.as_str()),
        (
            "dependency_graph_revision_id",
            request.dependency_graph_revision_id.as_str(),
        ),
        ("created_at", request.created_at.as_str()),
    ] {
        if value.trim().is_empty() {
            return Err(invalid(format!("subgraph replan {field} is blank")));
        }
    }
    let changed = request
        .changed_logical_work_item_ids
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    if changed.is_empty() || changed.len() != request.changed_logical_work_item_ids.len() {
        return Err(invalid(
            "subgraph replan changed identities are empty or duplicated",
        ));
    }
    if changed
        .iter()
        .any(|logical_id| !graph.contracts.contains_key(*logical_id))
    {
        return Err(invalid(
            "subgraph replan changed identity is absent from the dependency graph",
        ));
    }
    let mapping_keys = request
        .replacement_mapping
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    if mapping_keys != changed
        || request
            .replacement_mapping
            .values()
            .any(|replacement_ids| replacement_ids.is_empty())
    {
        return Err(invalid(
            "subgraph replacement mapping must cover every changed identity exactly",
        ));
    }

    let mut replacements = BTreeMap::new();
    for contract in &request.replacement_contracts {
        let id = contract.identity.logical_work_item_id.trim();
        if id.is_empty() || replacements.insert(id, contract).is_some() {
            return Err(invalid(
                "subgraph replacement contract identities are blank or duplicated",
            ));
        }
        if graph.contracts.contains_key(id) && !changed.contains(id) {
            return Err(invalid(
                "subgraph replacement identity collides with an unchanged work item",
            ));
        }
    }
    let mapped_replacements = request
        .replacement_mapping
        .values()
        .flatten()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    if mapped_replacements != replacements.keys().copied().collect() {
        return Err(invalid(
            "subgraph replacement mapping and replacement contracts disagree",
        ));
    }
    Ok(replacements)
}

fn boundaries<'a>(
    graph: &'a DependencyContractGraph,
    affected: &BTreeSet<String>,
) -> BoundaryEdges<'a> {
    let mut input_edges = graph
        .edges
        .iter()
        .filter(|edge| !affected.contains(&edge.from) && affected.contains(&edge.to))
        .collect::<Vec<_>>();
    let mut output_edges = graph
        .edges
        .iter()
        .filter(|edge| affected.contains(&edge.from) && !affected.contains(&edge.to))
        .collect::<Vec<_>>();
    input_edges.sort_by(|left, right| (&left.from, &left.to).cmp(&(&right.from, &right.to)));
    output_edges.sort_by(|left, right| (&left.from, &left.to).cmp(&(&right.from, &right.to)));
    BoundaryEdges {
        input_edges,
        output_edges,
    }
}

fn input_boundary_satisfied(
    graph: &DependencyContractGraph,
    request: &SubgraphReplanRequest,
    replacements: &BTreeMap<&str, &CanonicalWorkItemContract>,
    edge: &DependencyContractEdge,
) -> bool {
    let Some(provider) = graph.contracts.get(&edge.from) else {
        return false;
    };
    let consumer_inputs = request
        .replacement_mapping
        .get(&edge.to)
        .map(|ids| {
            ids.iter()
                .filter_map(|id| replacements.get(id.as_str()).copied())
                .flat_map(|contract| contract.input_contracts.iter())
                .collect::<Vec<_>>()
        })
        .unwrap_or_else(|| {
            graph
                .contracts
                .get(&edge.to)
                .map(|contract| contract.input_contracts.iter().collect())
                .unwrap_or_default()
        });
    consumer_inputs.iter().any(|input| {
        input.provider_logical_work_item_id == edge.from
            && input_contract_satisfied(provider, input)
    })
}

fn output_boundary_satisfied(
    request: &SubgraphReplanRequest,
    replacements: &BTreeMap<&str, &CanonicalWorkItemContract>,
    edge: &DependencyContractEdge,
) -> bool {
    let Some(ids) = request.replacement_mapping.get(&edge.from) else {
        return true;
    };
    ids.iter()
        .filter_map(|id| replacements.get(id.as_str()).copied())
        .any(|provider| provider_satisfies_requirements(provider, &edge.required_contracts))
}

fn input_contract_satisfied(
    provider: &CanonicalWorkItemContract,
    input: &RequiredInputContract,
) -> bool {
    provider_satisfies_requirements(
        provider,
        &[RequiredDependencyContract {
            contract_id: input.contract_id.clone(),
            required_capabilities: input.required_capabilities.clone(),
            compatibility_policy: input.compatibility_policy.clone(),
        }],
    )
}

fn provider_satisfies_requirements(
    provider: &CanonicalWorkItemContract,
    requirements: &[RequiredDependencyContract],
) -> bool {
    requirements.iter().all(|required| {
        let outputs = provider
            .output_contracts
            .iter()
            .filter(|output| output.contract_id == required.contract_id)
            .collect::<Vec<_>>();
        if outputs.is_empty() {
            return false;
        }
        let provided = outputs
            .iter()
            .flat_map(|output| output.capabilities.iter().map(String::as_str))
            .collect::<BTreeSet<_>>();
        let needed = required
            .required_capabilities
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        match required.compatibility_policy {
            ContractCompatibilityPolicy::RequireAll => needed.is_subset(&provided),
            ContractCompatibilityPolicy::RequireAny => {
                needed.is_empty() || !needed.is_disjoint(&provided)
            }
        }
    })
}

fn build_dependency_graph_revision(
    graph: &DependencyContractGraph,
    request: &SubgraphReplanRequest,
    replacements: &BTreeMap<&str, &CanonicalWorkItemContract>,
    changed: &BTreeSet<String>,
) -> Result<DependencyGraphRevision, PlanRepairError> {
    let mut edges = BTreeMap::<(String, String), Vec<RequiredDependencyContract>>::new();
    for edge in &graph.edges {
        match (changed.contains(&edge.from), changed.contains(&edge.to)) {
            (false, false) => insert_edge(&mut edges, edge.clone()),
            (false, true) | (true, true) => {}
            (true, false) => {
                let providers = request
                    .replacement_mapping
                    .get(&edge.from)
                    .into_iter()
                    .flatten()
                    .filter_map(|id| {
                        replacements
                            .get(id.as_str())
                            .filter(|contract| {
                                provider_satisfies_requirements(contract, &edge.required_contracts)
                            })
                            .map(|_| id.clone())
                    })
                    .collect::<Vec<_>>();
                if providers.len() > 1 {
                    return Err(invalid(format!(
                        "subgraph output boundary {} -> {} has ambiguous replacement providers",
                        edge.from, edge.to
                    )));
                }
                if let Some(provider) = providers.first() {
                    insert_edge(
                        &mut edges,
                        DependencyContractEdge {
                            from: provider.clone(),
                            to: edge.to.clone(),
                            required_contracts: edge.required_contracts.clone(),
                        },
                    );
                }
            }
        }
    }
    for contract in replacements.values() {
        for input in &contract.input_contracts {
            if changed.contains(&input.provider_logical_work_item_id) {
                return Err(invalid(
                    "replacement input references a superseded logical work item identity",
                ));
            }
            if !graph
                .contracts
                .contains_key(&input.provider_logical_work_item_id)
                && !replacements.contains_key(input.provider_logical_work_item_id.as_str())
            {
                return Err(invalid(
                    "replacement input references an unknown logical work item identity",
                ));
            }
            insert_edge(
                &mut edges,
                DependencyContractEdge {
                    from: input.provider_logical_work_item_id.clone(),
                    to: contract.identity.logical_work_item_id.clone(),
                    required_contracts: vec![RequiredDependencyContract {
                        contract_id: input.contract_id.clone(),
                        required_capabilities: input.required_capabilities.clone(),
                        compatibility_policy: input.compatibility_policy.clone(),
                    }],
                },
            );
        }
    }
    let edges = edges
        .into_iter()
        .map(|((from, to), mut required_contracts)| {
            required_contracts.sort_by(|left, right| {
                (
                    &left.contract_id,
                    compatibility_rank(&left.compatibility_policy),
                    &left.required_capabilities,
                )
                    .cmp(&(
                        &right.contract_id,
                        compatibility_rank(&right.compatibility_policy),
                        &right.required_capabilities,
                    ))
            });
            required_contracts.dedup();
            DependencyContractEdge {
                from,
                to,
                required_contracts,
            }
        })
        .collect();
    Ok(DependencyGraphRevision {
        id: request.dependency_graph_revision_id.clone(),
        plan_id: request.plan_id.clone(),
        edges,
        created_at: request.created_at.clone(),
    })
}

fn insert_edge(
    edges: &mut BTreeMap<(String, String), Vec<RequiredDependencyContract>>,
    edge: DependencyContractEdge,
) {
    edges
        .entry((edge.from, edge.to))
        .or_default()
        .extend(edge.required_contracts);
}

fn boundary_nodes<F>(edges: &[&DependencyContractEdge], select: F) -> Vec<String>
where
    F: Fn(&DependencyContractEdge) -> &String,
{
    edges
        .iter()
        .map(|edge| select(edge).clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn normalized_mapping(mapping: &BTreeMap<String, Vec<String>>) -> BTreeMap<String, Vec<String>> {
    mapping
        .iter()
        .map(|(old, replacements)| {
            let mut replacements = replacements.clone();
            replacements.sort();
            replacements.dedup();
            (old.clone(), replacements)
        })
        .collect()
}

fn compatibility_rank(policy: &ContractCompatibilityPolicy) -> u8 {
    match policy {
        ContractCompatibilityPolicy::RequireAll => 0,
        ContractCompatibilityPolicy::RequireAny => 1,
    }
}

fn invalid(message: impl Into<String>) -> PlanRepairError {
    PlanRepairError::InvalidRepairTarget(message.into())
}
