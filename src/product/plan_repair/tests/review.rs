use std::collections::BTreeMap;

use crate::product::work_item_contract::{
    CanonicalWorkItemContract, ContractCompatibilityPolicy, DependencyContractEdge,
    DependencyContractGraph, PromisedOutputContract,
};

use super::{
    ContractCapabilityAssociation, ContractDeltaKind, ContractImpactAnalyzer,
    compute_contract_delta, delta_fixture, dependency_contract_fixture, execution_state_fixture,
    provider_contract_fixture, required_edge,
};

fn association(contract_id: &str) -> ContractCapabilityAssociation {
    ContractCapabilityAssociation {
        contract_id: contract_id.to_string(),
        capability: "shared_capability".to_string(),
    }
}

fn provider_with_outputs(outputs: &[(&str, &[&str])]) -> CanonicalWorkItemContract {
    let mut provider = provider_contract_fixture(&[]);
    provider.output_contracts = outputs
        .iter()
        .map(|(contract_id, capabilities)| PromisedOutputContract {
            contract_id: (*contract_id).to_string(),
            capabilities: capabilities
                .iter()
                .map(|capability| (*capability).to_string())
                .collect(),
        })
        .collect();
    provider.handoff_contract.provided_contract_refs = outputs
        .iter()
        .map(|(contract_id, _)| (*contract_id).to_string())
        .collect();
    provider
}

fn graph_with_provider_and_edges(
    provider: CanonicalWorkItemContract,
    edges: Vec<DependencyContractEdge>,
) -> DependencyContractGraph {
    let mut contracts = BTreeMap::from([("WI-01".to_string(), provider)]);
    for edge in &edges {
        contracts
            .entry(edge.to.clone())
            .or_insert_with(|| dependency_contract_fixture(&edge.to));
    }
    DependencyContractGraph { contracts, edges }
}

fn edge_with_policy(
    to: &str,
    contract_id: &str,
    capabilities: &[&str],
    compatibility_policy: ContractCompatibilityPolicy,
) -> DependencyContractEdge {
    let mut edge = required_edge("WI-01", to, contract_id, capabilities);
    edge.required_contracts[0].compatibility_policy = compatibility_policy;
    edge
}

fn contract_with_capability_owners(
    owners: &[&str],
) -> crate::product::work_item_contract::CanonicalWorkItemContract {
    let mut contract = provider_contract_fixture(&[]);
    contract.output_contracts = ["contract_x", "contract_y"]
        .into_iter()
        .map(|contract_id| PromisedOutputContract {
            contract_id: contract_id.to_string(),
            capabilities: if owners.contains(&contract_id) {
                vec!["shared_capability".to_string()]
            } else {
                Vec::new()
            },
        })
        .collect();
    contract.handoff_contract.provided_contract_refs =
        vec!["contract_x".to_string(), "contract_y".to_string()];
    contract
}

#[test]
fn contract_delta_classifies_capability_owner_expansion_as_compatible() {
    let previous = contract_with_capability_owners(&["contract_x"]);
    let next = contract_with_capability_owners(&["contract_x", "contract_y"]);

    let delta = compute_contract_delta("revision_1", &previous, "revision_2", &next);

    assert_eq!(delta.kind, ContractDeltaKind::CompatibleContractExtension);
    assert_eq!(delta.changed_capabilities, vec!["shared_capability"]);
    assert_eq!(
        delta.added_capability_associations,
        vec![association("contract_y")]
    );
    assert!(delta.removed_capability_associations.is_empty());
}

#[test]
fn contract_delta_classifies_capability_owner_contraction_as_breaking() {
    let previous = contract_with_capability_owners(&["contract_x", "contract_y"]);
    let next = contract_with_capability_owners(&["contract_x"]);

    let delta = compute_contract_delta("revision_1", &previous, "revision_2", &next);

    assert_eq!(delta.kind, ContractDeltaKind::BreakingContractChange);
    assert_eq!(delta.changed_capabilities, vec!["shared_capability"]);
    assert!(delta.added_capability_associations.is_empty());
    assert_eq!(
        delta.removed_capability_associations,
        vec![association("contract_y")]
    );
}

#[test]
fn contract_delta_classifies_capability_owner_move_as_breaking() {
    let previous = contract_with_capability_owners(&["contract_x"]);
    let next = contract_with_capability_owners(&["contract_y"]);

    let delta = compute_contract_delta("revision_1", &previous, "revision_2", &next);

    assert_eq!(delta.kind, ContractDeltaKind::BreakingContractChange);
    assert_eq!(delta.changed_capabilities, vec!["shared_capability"]);
    assert_eq!(
        delta.added_capability_associations,
        vec![association("contract_y")]
    );
    assert_eq!(
        delta.removed_capability_associations,
        vec![association("contract_x")]
    );
}

#[test]
fn contract_delta_distinguishes_guidance_from_identical_contracts() {
    let previous = provider_contract_fixture(&["stable"]);
    let identical = previous.clone();
    let mut guidance = previous.clone();
    guidance.acceptance_criteria[0].statement = "Updated normative criterion".to_string();

    let no_op = compute_contract_delta("revision_1", &previous, "revision_2", &identical);
    let changed = compute_contract_delta("revision_1", &previous, "revision_2", &guidance);

    assert_eq!(no_op.kind, ContractDeltaKind::InformativeOnly);
    assert_eq!(changed.kind, ContractDeltaKind::ImplementationGuidance);
    assert!(changed.acceptance_changed);
}

#[test]
fn contract_impact_breaking_respects_require_all_and_require_any() {
    let graph = graph_with_provider_and_edges(
        provider_with_outputs(&[("contract_x", &["b"])]),
        vec![
            edge_with_policy(
                "WI-require-all",
                "contract_x",
                &["a", "b"],
                ContractCompatibilityPolicy::RequireAll,
            ),
            edge_with_policy(
                "WI-require-any",
                "contract_x",
                &["a", "b"],
                ContractCompatibilityPolicy::RequireAny,
            ),
        ],
    );
    let mut delta = delta_fixture(ContractDeltaKind::BreakingContractChange);
    delta.removed_capabilities = vec!["a".to_string()];
    delta.removed_capability_associations = vec![ContractCapabilityAssociation {
        contract_id: "contract_x".to_string(),
        capability: "a".to_string(),
    }];

    let report = ContractImpactAnalyzer
        .analyze_static(&graph, &delta, &execution_state_fixture(&[]))
        .unwrap();

    assert_eq!(report.direct_revalidation, vec!["WI-require-all"]);
    assert_eq!(report.unaffected, vec!["WI-require-any"]);
}

#[test]
fn contract_impact_compatible_require_any_revalidates_only_first_satisfaction() {
    let graph = graph_with_provider_and_edges(
        provider_with_outputs(&[("contract_x", &["a", "b"])]),
        vec![
            edge_with_policy(
                "WI-require-all",
                "contract_x",
                &["a", "b"],
                ContractCompatibilityPolicy::RequireAll,
            ),
            edge_with_policy(
                "WI-require-any-existing",
                "contract_x",
                &["a", "b"],
                ContractCompatibilityPolicy::RequireAny,
            ),
            edge_with_policy(
                "WI-require-any-first",
                "contract_x",
                &["a"],
                ContractCompatibilityPolicy::RequireAny,
            ),
        ],
    );
    let mut delta = delta_fixture(ContractDeltaKind::CompatibleContractExtension);
    delta.added_capabilities = vec!["a".to_string()];
    delta.added_capability_associations = vec![ContractCapabilityAssociation {
        contract_id: "contract_x".to_string(),
        capability: "a".to_string(),
    }];

    let report = ContractImpactAnalyzer
        .analyze_static(&graph, &delta, &execution_state_fixture(&[]))
        .unwrap();

    assert_eq!(
        report.direct_revalidation,
        vec!["WI-require-all", "WI-require-any-first"]
    );
    assert_eq!(report.unaffected, vec!["WI-require-any-existing"]);
}

#[test]
fn contract_impact_owner_expansion_is_scoped_to_added_association_contract() {
    let graph = graph_with_provider_and_edges(
        provider_with_outputs(&[
            ("contract_x", &["shared_capability"]),
            ("contract_y", &["shared_capability"]),
        ]),
        vec![
            required_edge(
                "WI-01",
                "WI-existing-owner",
                "contract_x",
                &["shared_capability"],
            ),
            required_edge(
                "WI-01",
                "WI-new-owner",
                "contract_y",
                &["shared_capability"],
            ),
        ],
    );
    let mut delta = delta_fixture(ContractDeltaKind::CompatibleContractExtension);
    delta.changed_capabilities = vec!["shared_capability".to_string()];
    delta.added_capability_associations = vec![association("contract_y")];

    let report = ContractImpactAnalyzer
        .analyze_static(&graph, &delta, &execution_state_fixture(&[]))
        .unwrap();

    assert_eq!(report.direct_revalidation, vec!["WI-new-owner"]);
    assert_eq!(report.unaffected, vec!["WI-existing-owner"]);
}

#[test]
fn contract_impact_explanation_paths_preserve_contract_data_and_deduplicate_edges() {
    let duplicate = edge_with_policy(
        "WI-consumer",
        "contract_x",
        &["b", "a", "a"],
        ContractCompatibilityPolicy::RequireAll,
    );
    let graph = graph_with_provider_and_edges(
        provider_with_outputs(&[("contract_x", &["a", "b"])]),
        vec![duplicate.clone(), duplicate],
    );
    let mut delta = delta_fixture(ContractDeltaKind::CompatibleContractExtension);
    delta.added_capabilities = vec!["a".to_string()];
    delta.added_capability_associations = vec![ContractCapabilityAssociation {
        contract_id: "contract_x".to_string(),
        capability: "a".to_string(),
    }];

    let report = ContractImpactAnalyzer
        .analyze_static(&graph, &delta, &execution_state_fixture(&[]))
        .unwrap();

    assert_eq!(report.direct_revalidation, vec!["WI-consumer"]);
    assert_eq!(report.explanation_paths.len(), 1);
    assert_eq!(report.explanation_paths[0].from, "WI-01");
    assert_eq!(report.explanation_paths[0].to, "WI-consumer");
    assert_eq!(report.explanation_paths[0].contract_id, "contract_x");
    assert_eq!(report.explanation_paths[0].capability_refs, vec!["a", "b"]);
}
