use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::product::models::DependencyGraphRevision;
use crate::product::work_item_contract::{
    CanonicalWorkItemContract, ContractCompatibilityPolicy, DependencyContractEdge,
    DependencyContractGraph, RequiredDependencyContract, RequiredInputContract,
    build_dependency_contract_graph, validate_dependency_contract_graph,
};

use super::PlanRepairError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubgraphReplanRequest {
    pub plan_id: String,
    pub base_plan_revision_id: String,
    pub repair_request_id: String,
    pub changed_logical_work_item_ids: Vec<String>,
    pub replacement_contracts: Vec<CanonicalWorkItemContract>,
    pub replacement_mapping: BTreeMap<String, Vec<String>>,
    pub story_spec_refs_changed: bool,
    pub design_spec_refs_changed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubgraphReplanReadiness {
    ScopeAnalysis,
    PublicationReady,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubgraphReplanResult {
    pub base_plan_revision_id: String,
    pub base_dependency_graph_revision_id: String,
    pub input_boundary: Vec<String>,
    pub output_boundary: Vec<String>,
    pub affected_logical_work_items: Vec<String>,
    pub replacement_mapping: BTreeMap<String, Vec<String>>,
    pub readiness: SubgraphReplanReadiness,
    pub dependency_graph_revision: Option<DependencyGraphRevision>,
    pub full_replan_required: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SubgraphReplanAnalysis {
    pub input_boundary: Vec<String>,
    pub output_boundary: Vec<String>,
    pub affected_logical_work_items: Vec<String>,
    pub replacement_mapping: BTreeMap<String, Vec<String>>,
    pub readiness: SubgraphReplanReadiness,
    pub rebuilt_graph: Option<DependencyContractGraph>,
    pub full_replan_required: bool,
}

#[derive(Debug, Default)]
pub struct SubgraphReplanner {
    _private: (),
}

impl SubgraphReplanner {
    pub(crate) fn analyze(
        &self,
        graph: &DependencyContractGraph,
        request: &SubgraphReplanRequest,
    ) -> Result<SubgraphReplanAnalysis, PlanRepairError> {
        let validated = validate_request(graph, request)?;
        let mut affected = validated.changed.clone();
        affected.extend(validated.mapping_keys.iter().cloned());

        loop {
            let boundary = boundaries(graph, &affected);
            let mut expansion = BTreeSet::new();
            for edge in &boundary.input_edges {
                if input_boundary_status(
                    graph,
                    &request.replacement_mapping,
                    &validated.replacements,
                    edge,
                )? == BoundaryStatus::Unsatisfied
                {
                    expansion.insert(edge.from.clone());
                }
            }
            for edge in &boundary.output_edges {
                if output_boundary_status(
                    &request.replacement_mapping,
                    &validated.replacements,
                    edge,
                )? == BoundaryStatus::Unsatisfied
                {
                    expansion.insert(edge.to.clone());
                }
            }
            let before = affected.len();
            affected.extend(expansion);
            if affected.len() == before {
                break;
            }
        }

        let boundary = boundaries(graph, &affected);
        let full_replan_required = affected.len() == graph.contracts.len()
            || request.story_spec_refs_changed
            || request.design_spec_refs_changed;
        let mapping_complete = validated.mapping_keys == affected;
        let source_refs_changed =
            request.story_spec_refs_changed || request.design_spec_refs_changed;
        let (readiness, rebuilt_graph) = if mapping_complete && !source_refs_changed {
            let graph = rebuild_typed_graph(
                graph,
                &affected,
                &request.replacement_mapping,
                &validated.replacements,
            )?;
            (SubgraphReplanReadiness::PublicationReady, Some(graph))
        } else {
            (SubgraphReplanReadiness::ScopeAnalysis, None)
        };

        Ok(SubgraphReplanAnalysis {
            input_boundary: boundary_nodes(&boundary.input_edges, |edge| &edge.from),
            output_boundary: boundary_nodes(&boundary.output_edges, |edge| &edge.to),
            affected_logical_work_items: affected.into_iter().collect(),
            replacement_mapping: normalized_mapping(&request.replacement_mapping),
            readiness,
            rebuilt_graph,
            full_replan_required,
        })
    }
}

struct ValidatedRequest<'a> {
    changed: BTreeSet<String>,
    mapping_keys: BTreeSet<String>,
    replacements: BTreeMap<&'a str, &'a CanonicalWorkItemContract>,
}

fn validate_request<'a>(
    graph: &DependencyContractGraph,
    request: &'a SubgraphReplanRequest,
) -> Result<ValidatedRequest<'a>, PlanRepairError> {
    for (field, value) in [
        ("plan_id", request.plan_id.as_str()),
        (
            "base_plan_revision_id",
            request.base_plan_revision_id.as_str(),
        ),
        ("repair_request_id", request.repair_request_id.as_str()),
    ] {
        if value.trim().is_empty() {
            return Err(invalid(format!("subgraph replan {field} is blank")));
        }
    }
    let changed = request
        .changed_logical_work_item_ids
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    if changed.is_empty() || changed.len() != request.changed_logical_work_item_ids.len() {
        return Err(invalid(
            "subgraph replan changed identities are empty or duplicated",
        ));
    }
    if changed.iter().any(|id| !graph.contracts.contains_key(id)) {
        return Err(invalid(
            "subgraph replan changed identity is absent from the dependency graph",
        ));
    }

    let mapping_keys = request
        .replacement_mapping
        .keys()
        .cloned()
        .collect::<BTreeSet<_>>();
    if !changed.is_subset(&mapping_keys)
        || mapping_keys
            .iter()
            .any(|id| !graph.contracts.contains_key(id))
        || request
            .replacement_mapping
            .values()
            .any(|ids| ids.is_empty() || ids.iter().collect::<BTreeSet<_>>().len() != ids.len())
    {
        return Err(invalid(
            "subgraph replacement mapping must uniquely cover every changed identity",
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
        if graph.contracts.contains_key(id) && !mapping_keys.contains(id) {
            return Err(invalid(
                "subgraph replacement identity collides with an unaffected work item",
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
    Ok(ValidatedRequest {
        changed,
        mapping_keys,
        replacements,
    })
}

struct BoundaryEdges<'a> {
    input_edges: Vec<&'a DependencyContractEdge>,
    output_edges: Vec<&'a DependencyContractEdge>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BoundaryStatus {
    Unknown,
    Satisfied,
    Unsatisfied,
}

fn input_boundary_status(
    graph: &DependencyContractGraph,
    mapping: &BTreeMap<String, Vec<String>>,
    replacements: &BTreeMap<&str, &CanonicalWorkItemContract>,
    edge: &DependencyContractEdge,
) -> Result<BoundaryStatus, PlanRepairError> {
    let Some(ids) = mapping.get(&edge.to) else {
        return Ok(BoundaryStatus::Unknown);
    };
    let Some(provider) = graph.contracts.get(&edge.from) else {
        return Ok(BoundaryStatus::Unsatisfied);
    };
    let inputs = ids
        .iter()
        .filter_map(|id| replacements.get(id.as_str()).copied())
        .flat_map(|contract| contract.input_contracts.iter())
        .filter(|input| input.provider_logical_work_item_id == edge.from)
        .collect::<Vec<_>>();
    if inputs
        .iter()
        .any(|input| !input_contract_satisfied(provider, input))
    {
        return Ok(BoundaryStatus::Unsatisfied);
    }
    for required in &edge.required_contracts {
        if !inputs.iter().any(|input| {
            input.contract_id == required.contract_id && input_contract_satisfied(provider, input)
        }) {
            return Ok(BoundaryStatus::Unsatisfied);
        }
    }
    Ok(BoundaryStatus::Satisfied)
}

fn output_boundary_status(
    mapping: &BTreeMap<String, Vec<String>>,
    replacements: &BTreeMap<&str, &CanonicalWorkItemContract>,
    edge: &DependencyContractEdge,
) -> Result<BoundaryStatus, PlanRepairError> {
    let Some(ids) = mapping.get(&edge.from) else {
        return Ok(BoundaryStatus::Unknown);
    };
    for required in &edge.required_contracts {
        if assign_required_contract(ids, replacements, required, &edge.from, &edge.to)?.is_none() {
            return Ok(BoundaryStatus::Unsatisfied);
        }
    }
    Ok(BoundaryStatus::Satisfied)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RequirementAssignment {
    provider_id: String,
    required_capabilities: Vec<String>,
}

fn assign_required_contract(
    replacement_ids: &[String],
    replacements: &BTreeMap<&str, &CanonicalWorkItemContract>,
    required: &RequiredDependencyContract,
    old_provider: &str,
    consumer: &str,
) -> Result<Option<Vec<RequirementAssignment>>, PlanRepairError> {
    let providers = replacement_ids
        .iter()
        .filter_map(|id| {
            replacements
                .get(id.as_str())
                .map(|contract| (id.as_str(), *contract))
        })
        .collect::<Vec<_>>();
    let required_capabilities = required
        .required_capabilities
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();

    match required.compatibility_policy {
        ContractCompatibilityPolicy::RequireAll if !required_capabilities.is_empty() => {
            let mut assignments = BTreeMap::<String, Vec<String>>::new();
            for capability in required_capabilities {
                let candidates = providers
                    .iter()
                    .filter(|(_, provider)| {
                        provider_provides_capability(provider, &required.contract_id, &capability)
                    })
                    .map(|(id, _)| *id)
                    .collect::<Vec<_>>();
                match candidates.as_slice() {
                    [] => return Ok(None),
                    [provider_id] => assignments
                        .entry((*provider_id).to_string())
                        .or_default()
                        .push(capability),
                    _ => {
                        return Err(ambiguous_provider(
                            old_provider,
                            consumer,
                            &required.contract_id,
                            Some(&capability),
                        ));
                    }
                }
            }
            Ok(Some(
                assignments
                    .into_iter()
                    .map(
                        |(provider_id, required_capabilities)| RequirementAssignment {
                            provider_id,
                            required_capabilities,
                        },
                    )
                    .collect(),
            ))
        }
        ContractCompatibilityPolicy::RequireAll | ContractCompatibilityPolicy::RequireAny => {
            let candidates = providers
                .iter()
                .filter(|(_, provider)| provider_satisfies_requirement(provider, required))
                .map(|(id, _)| *id)
                .collect::<Vec<_>>();
            match candidates.as_slice() {
                [] => Ok(None),
                [provider_id] => Ok(Some(vec![RequirementAssignment {
                    provider_id: (*provider_id).to_string(),
                    required_capabilities: required_capabilities.into_iter().collect(),
                }])),
                _ => Err(ambiguous_provider(
                    old_provider,
                    consumer,
                    &required.contract_id,
                    None,
                )),
            }
        }
    }
}

fn rebuild_typed_graph(
    graph: &DependencyContractGraph,
    affected: &BTreeSet<String>,
    mapping: &BTreeMap<String, Vec<String>>,
    replacements: &BTreeMap<&str, &CanonicalWorkItemContract>,
) -> Result<DependencyContractGraph, PlanRepairError> {
    let mut contracts = graph
        .contracts
        .iter()
        .filter(|(id, _)| !affected.contains(*id))
        .map(|(_, contract)| rewrite_unaffected_consumer(contract, mapping, replacements))
        .collect::<Result<Vec<_>, _>>()?;
    contracts.extend(replacements.values().map(|contract| (*contract).clone()));
    let rebuilt =
        build_dependency_contract_graph(&contracts).map_err(PlanRepairError::ContractValidation)?;
    let validation = validate_dependency_contract_graph(&rebuilt);
    if !validation.is_valid() {
        return Err(PlanRepairError::ContractValidation(validation));
    }
    Ok(rebuilt)
}

fn rewrite_unaffected_consumer(
    contract: &CanonicalWorkItemContract,
    mapping: &BTreeMap<String, Vec<String>>,
    replacements: &BTreeMap<&str, &CanonicalWorkItemContract>,
) -> Result<CanonicalWorkItemContract, PlanRepairError> {
    let mut rewritten = contract.clone();
    let mut inputs = Vec::new();
    for input in &contract.input_contracts {
        let Some(replacement_ids) = mapping.get(&input.provider_logical_work_item_id) else {
            inputs.push(input.clone());
            continue;
        };
        let required = RequiredDependencyContract {
            contract_id: input.contract_id.clone(),
            required_capabilities: input.required_capabilities.clone(),
            compatibility_policy: input.compatibility_policy.clone(),
        };
        let Some(assignments) = assign_required_contract(
            replacement_ids,
            replacements,
            &required,
            &input.provider_logical_work_item_id,
            &contract.identity.logical_work_item_id,
        )?
        else {
            return Err(invalid(format!(
                "subgraph output boundary {} -> {} cannot be rewired for {}",
                input.provider_logical_work_item_id,
                contract.identity.logical_work_item_id,
                input.contract_id
            )));
        };
        inputs.extend(
            assignments
                .into_iter()
                .map(|assignment| RequiredInputContract {
                    contract_id: input.contract_id.clone(),
                    provider_logical_work_item_id: assignment.provider_id,
                    required_capabilities: assignment.required_capabilities,
                    compatibility_policy: input.compatibility_policy.clone(),
                }),
        );
    }
    inputs.sort_by(|left, right| {
        (
            &left.provider_logical_work_item_id,
            &left.contract_id,
            compatibility_rank(&left.compatibility_policy),
            &left.required_capabilities,
        )
            .cmp(&(
                &right.provider_logical_work_item_id,
                &right.contract_id,
                compatibility_rank(&right.compatibility_policy),
                &right.required_capabilities,
            ))
    });
    rewritten.input_contracts = inputs;
    Ok(rewritten)
}

fn input_contract_satisfied(
    provider: &CanonicalWorkItemContract,
    input: &RequiredInputContract,
) -> bool {
    provider_satisfies_requirement(
        provider,
        &RequiredDependencyContract {
            contract_id: input.contract_id.clone(),
            required_capabilities: input.required_capabilities.clone(),
            compatibility_policy: input.compatibility_policy.clone(),
        },
    )
}

fn provider_satisfies_requirement(
    provider: &CanonicalWorkItemContract,
    required: &RequiredDependencyContract,
) -> bool {
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
}

fn provider_provides_capability(
    provider: &CanonicalWorkItemContract,
    contract_id: &str,
    capability: &str,
) -> bool {
    provider.output_contracts.iter().any(|output| {
        output.contract_id == contract_id
            && output
                .capabilities
                .iter()
                .any(|provided| provided == capability)
    })
}

fn ambiguous_provider(
    old_provider: &str,
    consumer: &str,
    contract_id: &str,
    capability: Option<&str>,
) -> PlanRepairError {
    invalid(format!(
        "subgraph output boundary {old_provider} -> {consumer} has ambiguous providers for {contract_id}{}",
        capability
            .map(|capability| format!(" capability {capability}"))
            .unwrap_or_default()
    ))
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
