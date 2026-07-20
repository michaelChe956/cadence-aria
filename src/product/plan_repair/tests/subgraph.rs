use std::collections::BTreeMap;

use super::super::{SubgraphReplanReadiness, SubgraphReplanRequest, SubgraphReplanner};
use crate::product::work_item_contract::{
    CanonicalWorkItemContract, ContractCompatibilityPolicy, DependencyContractEdge,
    DependencyContractGraph, PromisedOutputContract, RequiredDependencyContract,
    RequiredInputContract, canonical_contract_fixture,
};

mod publication;

fn chain_contract(
    id: &str,
    input: Option<(&str, &str)>,
    output_contract_id: &str,
) -> CanonicalWorkItemContract {
    let mut contract = canonical_contract_fixture(id);
    contract.identity.logical_work_item_id = id.to_string();
    contract.input_contracts = input
        .map(|(provider, contract_id)| {
            vec![RequiredInputContract {
                contract_id: contract_id.to_string(),
                provider_logical_work_item_id: provider.to_string(),
                required_capabilities: vec![format!("capability_{contract_id}")],
                compatibility_policy: ContractCompatibilityPolicy::RequireAll,
            }]
        })
        .unwrap_or_default();
    contract.output_contracts = vec![PromisedOutputContract {
        contract_id: output_contract_id.to_string(),
        capabilities: vec![format!("capability_{output_contract_id}")],
    }];
    contract.handoff_contract.provided_contract_refs = vec![output_contract_id.to_string()];
    contract
}

fn required_edge(from: &str, to: &str, contract_id: &str) -> DependencyContractEdge {
    required_edge_with_capabilities(
        from,
        to,
        contract_id,
        vec![format!("capability_{contract_id}")],
    )
}

fn required_edge_with_capabilities(
    from: &str,
    to: &str,
    contract_id: &str,
    required_capabilities: Vec<String>,
) -> DependencyContractEdge {
    DependencyContractEdge {
        from: from.to_string(),
        to: to.to_string(),
        required_contracts: vec![RequiredDependencyContract {
            contract_id: contract_id.to_string(),
            required_capabilities,
            compatibility_policy: ContractCompatibilityPolicy::RequireAll,
        }],
    }
}

fn chain_graph(ids: &[&str]) -> DependencyContractGraph {
    let contracts = ids
        .iter()
        .enumerate()
        .map(|(index, id)| {
            let input = index
                .checked_sub(1)
                .map(|previous| (ids[previous], format!("contract_{}", ids[previous])));
            let mut contract = chain_contract(
                id,
                input
                    .as_ref()
                    .map(|(provider, contract_id)| (*provider, contract_id.as_str())),
                &format!("contract_{id}"),
            );
            if index + 1 == ids.len() {
                contract.handoff_contract.provided_contract_refs.clear();
            }
            ((*id).to_string(), contract)
        })
        .collect();
    let edges = ids
        .windows(2)
        .map(|pair| required_edge(pair[0], pair[1], &format!("contract_{}", pair[0])))
        .collect();
    DependencyContractGraph { contracts, edges }
}

fn request(
    changed: &[&str],
    replacements: Vec<CanonicalWorkItemContract>,
    replacement_mapping: BTreeMap<String, Vec<String>>,
) -> SubgraphReplanRequest {
    SubgraphReplanRequest {
        plan_id: "work_item_plan_0001".to_string(),
        base_plan_revision_id: "plan_revision_0001".to_string(),
        repair_request_id: "plan_repair_request_0001".to_string(),
        changed_logical_work_item_ids: changed.iter().map(|id| (*id).to_string()).collect(),
        replacement_contracts: replacements,
        replacement_mapping,
        story_spec_refs_changed: false,
        design_spec_refs_changed: false,
    }
}

fn split_b_request(output_contract_id: &str) -> SubgraphReplanRequest {
    let first = chain_contract("wi_b1", Some(("wi_a", "contract_wi_a")), "contract_wi_b1");
    let second = chain_contract(
        "wi_b2",
        Some(("wi_b1", "contract_wi_b1")),
        output_contract_id,
    );
    request(
        &["wi_b"],
        vec![first, second],
        BTreeMap::from([(
            "wi_b".to_string(),
            vec!["wi_b1".to_string(), "wi_b2".to_string()],
        )]),
    )
}

#[test]
fn plan_repair_subgraph_replan_preserves_unchanged_boundaries() {
    let result = SubgraphReplanner::default()
        .analyze(
            &chain_graph(&["wi_a", "wi_b", "wi_c", "wi_d"]),
            &split_b_request("contract_wi_b"),
        )
        .unwrap();

    assert_eq!(result.input_boundary, vec!["wi_a"]);
    assert_eq!(result.output_boundary, vec!["wi_c"]);
    assert_eq!(result.replacement_mapping["wi_b"], vec!["wi_b1", "wi_b2"]);
    assert_eq!(result.affected_logical_work_items, vec!["wi_b"]);
    assert!(!result.full_replan_required);
    assert_eq!(result.readiness, SubgraphReplanReadiness::PublicationReady);
    assert_eq!(
        result.rebuilt_graph.unwrap().edges,
        vec![
            required_edge("wi_a", "wi_b1", "contract_wi_a"),
            required_edge("wi_b1", "wi_b2", "contract_wi_b1"),
            required_edge("wi_b2", "wi_c", "contract_wi_b"),
            required_edge("wi_c", "wi_d", "contract_wi_c"),
        ]
    );
}

#[test]
fn plan_repair_subgraph_expansion_is_not_publication_ready_without_all_affected_replacements() {
    let result = SubgraphReplanner::default()
        .analyze(
            &chain_graph(&["wi_a", "wi_b", "wi_c"]),
            &split_b_request("contract_replacement_only"),
        )
        .unwrap();

    assert_eq!(result.input_boundary, vec!["wi_a"]);
    assert!(
        result
            .affected_logical_work_items
            .contains(&"wi_c".to_string())
    );
    assert!(result.output_boundary.is_empty());
    assert!(!result.full_replan_required);
    assert_eq!(result.readiness, SubgraphReplanReadiness::ScopeAnalysis);
    assert!(result.rebuilt_graph.is_none());
}

#[test]
fn plan_repair_subgraph_input_expansion_does_not_return_an_invalid_publication_graph() {
    let mut replacement = split_b_request("contract_wi_b");
    replacement.replacement_contracts[0].input_contracts[0].contract_id =
        "contract_missing_input".to_string();
    replacement.replacement_contracts[0].input_contracts[0].required_capabilities =
        vec!["capability_contract_missing_input".to_string()];

    let result = SubgraphReplanner::default()
        .analyze(&chain_graph(&["wi_a", "wi_b", "wi_c"]), &replacement)
        .unwrap();

    assert!(
        result
            .affected_logical_work_items
            .contains(&"wi_a".to_string())
    );
    assert_eq!(result.readiness, SubgraphReplanReadiness::ScopeAnalysis);
    assert!(result.rebuilt_graph.is_none());
}

#[test]
fn plan_repair_subgraph_merge_preserves_many_to_one_replacement_mapping() {
    let merged = chain_contract("wi_bc", Some(("wi_a", "contract_wi_a")), "contract_wi_c");
    let result = SubgraphReplanner::default()
        .analyze(
            &chain_graph(&["wi_a", "wi_b", "wi_c", "wi_d"]),
            &request(
                &["wi_b", "wi_c"],
                vec![merged],
                BTreeMap::from([
                    ("wi_b".to_string(), vec!["wi_bc".to_string()]),
                    ("wi_c".to_string(), vec!["wi_bc".to_string()]),
                ]),
            ),
        )
        .unwrap();

    assert_eq!(result.input_boundary, vec!["wi_a"]);
    assert_eq!(result.output_boundary, vec!["wi_d"]);
    assert_eq!(result.replacement_mapping["wi_b"], vec!["wi_bc"]);
    assert_eq!(result.replacement_mapping["wi_c"], vec!["wi_bc"]);
    assert!(result.rebuilt_graph.unwrap().edges.contains(&required_edge(
        "wi_bc",
        "wi_d",
        "contract_wi_c"
    )));
}

#[test]
fn plan_repair_subgraph_distributes_required_contracts_across_split_replacements() {
    let mut graph = chain_graph(&["wi_a", "wi_b", "wi_c"]);
    graph.contracts.get_mut("wi_b").unwrap().output_contracts = vec![
        PromisedOutputContract {
            contract_id: "contract_alpha".to_string(),
            capabilities: vec!["alpha_ready".to_string()],
        },
        PromisedOutputContract {
            contract_id: "contract_beta".to_string(),
            capabilities: vec!["beta_ready".to_string()],
        },
    ];
    graph.edges[1].required_contracts = vec![
        RequiredDependencyContract {
            contract_id: "contract_alpha".to_string(),
            required_capabilities: vec!["alpha_ready".to_string()],
            compatibility_policy: ContractCompatibilityPolicy::RequireAll,
        },
        RequiredDependencyContract {
            contract_id: "contract_beta".to_string(),
            required_capabilities: vec!["beta_ready".to_string()],
            compatibility_policy: ContractCompatibilityPolicy::RequireAll,
        },
    ];
    let mut first = chain_contract("wi_b1", Some(("wi_a", "contract_wi_a")), "contract_alpha");
    first.output_contracts[0].capabilities = vec!["alpha_ready".to_string()];
    let mut second = chain_contract("wi_b2", Some(("wi_b1", "contract_alpha")), "contract_beta");
    second.input_contracts[0].required_capabilities = vec!["alpha_ready".to_string()];
    second.output_contracts[0].capabilities = vec!["beta_ready".to_string()];
    let mut consumer = chain_contract("wi_c2", None, "contract_wi_c");
    consumer.handoff_contract.provided_contract_refs.clear();
    consumer.input_contracts = vec![
        RequiredInputContract {
            contract_id: "contract_alpha".to_string(),
            provider_logical_work_item_id: "wi_b1".to_string(),
            required_capabilities: vec!["alpha_ready".to_string()],
            compatibility_policy: ContractCompatibilityPolicy::RequireAll,
        },
        RequiredInputContract {
            contract_id: "contract_beta".to_string(),
            provider_logical_work_item_id: "wi_b2".to_string(),
            required_capabilities: vec!["beta_ready".to_string()],
            compatibility_policy: ContractCompatibilityPolicy::RequireAll,
        },
    ];
    let result = SubgraphReplanner::default()
        .analyze(
            &graph,
            &request(
                &["wi_b"],
                vec![first, second, consumer],
                BTreeMap::from([
                    (
                        "wi_b".to_string(),
                        vec!["wi_b1".to_string(), "wi_b2".to_string()],
                    ),
                    ("wi_c".to_string(), vec!["wi_c2".to_string()]),
                ]),
            ),
        )
        .unwrap();

    assert_eq!(result.readiness, SubgraphReplanReadiness::PublicationReady);
    let graph = result.rebuilt_graph.unwrap();
    assert!(graph.edges.contains(&required_edge_with_capabilities(
        "wi_b1",
        "wi_c2",
        "contract_alpha",
        vec!["alpha_ready".to_string()],
    )));
    assert!(graph.edges.contains(&required_edge_with_capabilities(
        "wi_b2",
        "wi_c2",
        "contract_beta",
        vec!["beta_ready".to_string()],
    )));
}

#[test]
fn plan_repair_subgraph_distributes_required_capabilities_across_split_replacements() {
    let mut graph = chain_graph(&["wi_a", "wi_b", "wi_c"]);
    graph.contracts.get_mut("wi_b").unwrap().output_contracts = vec![PromisedOutputContract {
        contract_id: "contract_shared".to_string(),
        capabilities: vec!["alpha_ready".to_string(), "beta_ready".to_string()],
    }];
    graph
        .contracts
        .get_mut("wi_b")
        .unwrap()
        .handoff_contract
        .provided_contract_refs = vec!["contract_shared".to_string()];
    graph.contracts.get_mut("wi_c").unwrap().input_contracts = vec![RequiredInputContract {
        contract_id: "contract_shared".to_string(),
        provider_logical_work_item_id: "wi_b".to_string(),
        required_capabilities: vec!["alpha_ready".to_string(), "beta_ready".to_string()],
        compatibility_policy: ContractCompatibilityPolicy::RequireAll,
    }];
    graph.edges[1] = required_edge_with_capabilities(
        "wi_b",
        "wi_c",
        "contract_shared",
        vec!["alpha_ready".to_string(), "beta_ready".to_string()],
    );

    let mut first = chain_contract("wi_b1", Some(("wi_a", "contract_wi_a")), "contract_shared");
    first.output_contracts[0].capabilities = vec!["alpha_ready".to_string()];
    let mut second = chain_contract(
        "wi_b2",
        Some(("wi_b1", "contract_shared")),
        "contract_shared",
    );
    second.input_contracts[0].required_capabilities = vec!["alpha_ready".to_string()];
    second.output_contracts[0].capabilities = vec!["beta_ready".to_string()];

    let result = SubgraphReplanner::default()
        .analyze(
            &graph,
            &request(
                &["wi_b"],
                vec![first, second],
                BTreeMap::from([(
                    "wi_b".to_string(),
                    vec!["wi_b1".to_string(), "wi_b2".to_string()],
                )]),
            ),
        )
        .unwrap();

    assert_eq!(result.readiness, SubgraphReplanReadiness::PublicationReady);
    let graph = result.rebuilt_graph.unwrap();
    assert!(graph.edges.contains(&required_edge_with_capabilities(
        "wi_b1",
        "wi_c",
        "contract_shared",
        vec!["alpha_ready".to_string()],
    )));
    assert!(graph.edges.contains(&required_edge_with_capabilities(
        "wi_b2",
        "wi_c",
        "contract_shared",
        vec!["beta_ready".to_string()],
    )));
}

#[test]
fn plan_repair_subgraph_rejects_ambiguous_equivalent_output_providers() {
    let graph = chain_graph(&["wi_a", "wi_b", "wi_c"]);
    let mut first = chain_contract("wi_b1", Some(("wi_a", "contract_wi_a")), "contract_wi_b");
    first.output_contracts[0].capabilities = vec!["capability_contract_wi_b".to_string()];
    let mut second = chain_contract("wi_b2", Some(("wi_b1", "contract_wi_b")), "contract_wi_b");
    second.output_contracts[0].capabilities = vec!["capability_contract_wi_b".to_string()];

    assert!(
        SubgraphReplanner::default()
            .analyze(
                &graph,
                &request(
                    &["wi_b"],
                    vec![first, second],
                    BTreeMap::from([(
                        "wi_b".to_string(),
                        vec!["wi_b1".to_string(), "wi_b2".to_string()],
                    )]),
                ),
            )
            .is_err()
    );
}

#[test]
fn plan_repair_subgraph_marks_full_replan_for_whole_graph_or_source_ref_change() {
    let mut whole_graph = split_b_request("contract_replacement_only");
    whole_graph.replacement_contracts[0].input_contracts[0].contract_id =
        "contract_missing_input".to_string();
    whole_graph.replacement_contracts[0].input_contracts[0].required_capabilities =
        vec!["capability_contract_missing_input".to_string()];
    let expanded = SubgraphReplanner::default()
        .analyze(&chain_graph(&["wi_a", "wi_b", "wi_c"]), &whole_graph)
        .unwrap();
    assert_eq!(
        expanded.affected_logical_work_items,
        vec!["wi_a", "wi_b", "wi_c"]
    );
    assert!(expanded.full_replan_required);

    for (story_changed, design_changed) in [(true, false), (false, true)] {
        let mut source_changed = split_b_request("contract_wi_b");
        source_changed.story_spec_refs_changed = story_changed;
        source_changed.design_spec_refs_changed = design_changed;
        let result = SubgraphReplanner::default()
            .analyze(&chain_graph(&["wi_a", "wi_b", "wi_c"]), &source_changed)
            .unwrap();
        assert!(result.full_replan_required);
        assert_eq!(result.readiness, SubgraphReplanReadiness::ScopeAnalysis);
        assert!(result.rebuilt_graph.is_none());
    }
}

#[test]
fn plan_repair_subgraph_rejects_missing_or_ambiguous_replacement_identity() {
    let graph = chain_graph(&["wi_a", "wi_b", "wi_c"]);
    let mut missing_mapping = split_b_request("contract_wi_b");
    missing_mapping.replacement_mapping.clear();
    assert!(
        SubgraphReplanner::default()
            .analyze(&graph, &missing_mapping)
            .is_err()
    );

    let mut duplicate_replacement = split_b_request("contract_wi_b");
    duplicate_replacement
        .replacement_contracts
        .push(duplicate_replacement.replacement_contracts[0].clone());
    assert!(
        SubgraphReplanner::default()
            .analyze(&graph, &duplicate_replacement)
            .is_err()
    );

    let mut duplicate_mapping = split_b_request("contract_wi_b");
    duplicate_mapping
        .replacement_mapping
        .get_mut("wi_b")
        .unwrap()
        .push("wi_b1".to_string());
    assert!(
        SubgraphReplanner::default()
            .analyze(&graph, &duplicate_mapping)
            .is_err()
    );
}
