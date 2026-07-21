use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::product::models::ContractDeltaKind;
use crate::product::work_item_contract::{
    CanonicalWorkItemContract, ContractCompatibilityPolicy, RequiredInputContract,
};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ContractCapabilityAssociation {
    pub contract_id: String,
    pub capability: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContractDelta {
    pub logical_work_item_id: String,
    pub previous_revision_id: String,
    pub next_revision_id: String,
    pub kind: ContractDeltaKind,
    pub added_contracts: Vec<String>,
    pub removed_contracts: Vec<String>,
    pub added_capabilities: Vec<String>,
    pub removed_capabilities: Vec<String>,
    pub changed_capabilities: Vec<String>,
    pub added_capability_associations: Vec<ContractCapabilityAssociation>,
    pub removed_capability_associations: Vec<ContractCapabilityAssociation>,
    pub acceptance_changed: bool,
    pub verification_changed: bool,
    pub write_policy_changed: bool,
}

pub fn compute_contract_delta(
    previous_revision_id: &str,
    previous: &CanonicalWorkItemContract,
    next_revision_id: &str,
    next: &CanonicalWorkItemContract,
) -> ContractDelta {
    let previous_outputs = normalized_outputs(previous);
    let next_outputs = normalized_outputs(next);
    let previous_capabilities = previous_outputs
        .capability_owners
        .keys()
        .cloned()
        .collect::<BTreeSet<_>>();
    let next_capabilities = next_outputs
        .capability_owners
        .keys()
        .cloned()
        .collect::<BTreeSet<_>>();
    let previous_associations = capability_associations(&previous_outputs);
    let next_associations = capability_associations(&next_outputs);

    let added_contracts =
        set_difference(&next_outputs.contract_ids, &previous_outputs.contract_ids);
    let removed_contracts =
        set_difference(&previous_outputs.contract_ids, &next_outputs.contract_ids);
    let added_capabilities = set_difference(&next_capabilities, &previous_capabilities);
    let removed_capabilities = set_difference(&previous_capabilities, &next_capabilities);
    let added_capability_associations = next_associations
        .difference(&previous_associations)
        .cloned()
        .collect::<Vec<_>>();
    let removed_capability_associations = previous_associations
        .difference(&next_associations)
        .cloned()
        .collect::<Vec<_>>();
    let changed_capabilities = previous_capabilities
        .intersection(&next_capabilities)
        .filter(|capability| {
            previous_outputs.capability_owners.get(*capability)
                != next_outputs.capability_owners.get(*capability)
        })
        .cloned()
        .collect::<Vec<_>>();

    let acceptance_changed = previous.acceptance_criteria != next.acceptance_criteria;
    let verification_changed = previous.verification_checks != next.verification_checks;
    let write_policy_changed = previous.write_policy != next.write_policy;
    let topology_changed = previous.identity.logical_work_item_id
        != next.identity.logical_work_item_id
        || normalized_inputs(&previous.input_contracts) != normalized_inputs(&next.input_contracts);
    let guidance_changed = normative_guidance_changed(previous, next);

    let kind = if topology_changed {
        ContractDeltaKind::TopologyChange
    } else if !removed_contracts.is_empty()
        || !removed_capabilities.is_empty()
        || !removed_capability_associations.is_empty()
    {
        ContractDeltaKind::BreakingContractChange
    } else if !added_contracts.is_empty()
        || !added_capabilities.is_empty()
        || !added_capability_associations.is_empty()
    {
        ContractDeltaKind::CompatibleContractExtension
    } else if guidance_changed {
        ContractDeltaKind::ImplementationGuidance
    } else {
        ContractDeltaKind::InformativeOnly
    };

    ContractDelta {
        logical_work_item_id: previous.identity.logical_work_item_id.clone(),
        previous_revision_id: previous_revision_id.to_string(),
        next_revision_id: next_revision_id.to_string(),
        kind,
        added_contracts,
        removed_contracts,
        added_capabilities,
        removed_capabilities,
        changed_capabilities,
        added_capability_associations,
        removed_capability_associations,
        acceptance_changed,
        verification_changed,
        write_policy_changed,
    }
}

fn capability_associations(outputs: &NormalizedOutputs) -> BTreeSet<ContractCapabilityAssociation> {
    outputs
        .capability_owners
        .iter()
        .flat_map(|(capability, contract_ids)| {
            contract_ids
                .iter()
                .map(|contract_id| ContractCapabilityAssociation {
                    contract_id: contract_id.clone(),
                    capability: capability.clone(),
                })
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct NormalizedInputContract {
    provider_logical_work_item_id: String,
    contract_id: String,
    required_capabilities: Vec<String>,
    compatibility_policy_rank: u8,
}

fn normalized_inputs(inputs: &[RequiredInputContract]) -> BTreeSet<NormalizedInputContract> {
    inputs
        .iter()
        .map(|input| NormalizedInputContract {
            provider_logical_work_item_id: input.provider_logical_work_item_id.clone(),
            contract_id: input.contract_id.clone(),
            required_capabilities: sorted_unique(&input.required_capabilities),
            compatibility_policy_rank: match input.compatibility_policy {
                ContractCompatibilityPolicy::RequireAll => 0,
                ContractCompatibilityPolicy::RequireAny => 1,
            },
        })
        .collect()
}

struct NormalizedOutputs {
    contract_ids: BTreeSet<String>,
    capability_owners: BTreeMap<String, BTreeSet<String>>,
}

fn normalized_outputs(contract: &CanonicalWorkItemContract) -> NormalizedOutputs {
    let mut capability_owners = BTreeMap::<String, BTreeSet<String>>::new();
    for output in &contract.output_contracts {
        for capability in &output.capabilities {
            capability_owners
                .entry(capability.clone())
                .or_default()
                .insert(output.contract_id.clone());
        }
    }

    let contract_ids = contract
        .output_contracts
        .iter()
        .map(|output| output.contract_id.clone())
        .collect::<BTreeSet<_>>();
    NormalizedOutputs {
        contract_ids,
        capability_owners,
    }
}

fn normative_guidance_changed(
    previous: &CanonicalWorkItemContract,
    next: &CanonicalWorkItemContract,
) -> bool {
    previous.schema_version != next.schema_version
        || previous.identity.title != next.identity.title
        || previous.identity.kind != next.identity.kind
        || previous.goal != next.goal
        || previous.non_goals != next.non_goals
        || previous.tasks != next.tasks
        || previous.write_policy != next.write_policy
        || previous.acceptance_criteria != next.acceptance_criteria
        || previous.verification_checks != next.verification_checks
        || previous.handoff_contract != next.handoff_contract
        || previous.blocker_rules != next.blocker_rules
        || previous.design_traceability != next.design_traceability
}

fn set_difference(left: &BTreeSet<String>, right: &BTreeSet<String>) -> Vec<String> {
    left.difference(right).cloned().collect()
}

fn sorted_unique(values: &[String]) -> Vec<String> {
    values
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}
