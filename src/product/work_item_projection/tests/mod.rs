use std::collections::BTreeMap;

use crate::product::work_item_contract::{
    CanonicalWorkItemContract, ContractCompatibilityPolicy, DependencyContractGraph,
    PromisedOutputContract, RequiredInputContract, build_dependency_contract_graph,
    canonical_contract_fixture,
};
use crate::product::work_item_projection::{
    CompiledWorkItemProjections, WorkItemProjectionCompiler,
};

mod coder;
mod human;
mod plan;
mod reviewer;
mod validation;

pub(super) fn contract_fixture() -> CanonicalWorkItemContract {
    canonical_contract_fixture("wi_consumer")
}

pub(super) fn compiled_fixture() -> CompiledWorkItemProjections {
    WorkItemProjectionCompiler
        .compile(&contract_fixture(), "work_item_revision_0001")
        .unwrap()
}

pub(super) fn compiled_plan_fixture() -> (
    DependencyContractGraph,
    BTreeMap<String, CompiledWorkItemProjections>,
) {
    let mut provider = canonical_contract_fixture("wi_provider");
    provider.identity.title = "Provide shared contract".to_string();
    provider.input_contracts.clear();
    provider.output_contracts = vec![PromisedOutputContract {
        contract_id: "contract.shared".to_string(),
        capabilities: vec!["capability.a".to_string(), "capability.b".to_string()],
    }];
    provider.handoff_contract.provided_contract_refs = vec!["contract.shared".to_string()];

    let mut consumer = canonical_contract_fixture("wi_consumer");
    consumer.identity.title = "Consume shared contract".to_string();
    consumer.input_contracts = vec![RequiredInputContract {
        contract_id: "contract.shared".to_string(),
        provider_logical_work_item_id: "wi_provider".to_string(),
        required_capabilities: vec!["capability.a".to_string()],
        compatibility_policy: ContractCompatibilityPolicy::RequireAll,
    }];
    consumer.output_contracts = vec![PromisedOutputContract {
        contract_id: "contract.consumer".to_string(),
        capabilities: vec!["consumer.ready".to_string()],
    }];
    consumer.handoff_contract.provided_contract_refs.clear();

    let mut independent = canonical_contract_fixture("wi_independent");
    independent.identity.title = "Independent work".to_string();
    independent.input_contracts.clear();
    independent.output_contracts = vec![PromisedOutputContract {
        contract_id: "contract.independent".to_string(),
        capabilities: vec!["independent.ready".to_string()],
    }];
    independent.handoff_contract.provided_contract_refs.clear();

    let graph =
        build_dependency_contract_graph(&[consumer.clone(), provider.clone(), independent.clone()])
            .unwrap();
    let projections = [provider, consumer, independent]
        .into_iter()
        .map(|contract| {
            let logical_id = contract.identity.logical_work_item_id.clone();
            let revision_id = format!("revision_{logical_id}");
            let compiled = WorkItemProjectionCompiler
                .compile(&contract, &revision_id)
                .unwrap();
            (logical_id, compiled)
        })
        .collect();

    (graph, projections)
}

pub(super) fn expected_plan_revision_ids() -> BTreeMap<String, String> {
    BTreeMap::from([
        (
            "wi_consumer".to_string(),
            "revision_wi_consumer".to_string(),
        ),
        (
            "wi_independent".to_string(),
            "revision_wi_independent".to_string(),
        ),
        (
            "wi_provider".to_string(),
            "revision_wi_provider".to_string(),
        ),
    ])
}
