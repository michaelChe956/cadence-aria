use std::collections::BTreeMap;

use super::canonical_contract_fixture;
use crate::product::{
    models::DependencyGraphRevision,
    work_item_contract::{
        CanonicalWorkItemContract, ContractCompatibilityPolicy, DependencyContractEdge,
        DependencyContractGraph, PromisedOutputContract, RequiredDependencyContract,
        RequiredInputContract, build_dependency_contract_graph, validate_dependency_contract_graph,
    },
};

fn provider_contract_fixture(capabilities: &[&str]) -> CanonicalWorkItemContract {
    let mut provider = canonical_contract_fixture("WI-01");
    provider.input_contracts.clear();
    provider.output_contracts = vec![PromisedOutputContract {
        contract_id: "contract.workflow".to_string(),
        capabilities: capabilities
            .iter()
            .map(|capability| (*capability).to_string())
            .collect(),
    }];
    provider.handoff_contract.provided_contract_refs = vec!["contract.workflow".to_string()];
    provider
}

fn consumer_contract_fixture(
    required_capabilities: &[&str],
    compatibility_policy: ContractCompatibilityPolicy,
) -> CanonicalWorkItemContract {
    let mut consumer = canonical_contract_fixture("WI-02");
    consumer.input_contracts = vec![RequiredInputContract {
        contract_id: "contract.workflow".to_string(),
        provider_logical_work_item_id: "WI-01".to_string(),
        required_capabilities: required_capabilities
            .iter()
            .map(|capability| (*capability).to_string())
            .collect(),
        compatibility_policy,
    }];
    consumer.output_contracts.clear();
    consumer.handoff_contract.provided_contract_refs.clear();
    consumer
}

fn finding_count(graph: &DependencyContractGraph, code: &str) -> usize {
    validate_dependency_contract_graph(graph)
        .findings
        .iter()
        .filter(|finding| finding.code == code)
        .count()
}

#[test]
fn canonical_work_item_dependency_graph_builds_provider_to_consumer_edge() {
    let provider = provider_contract_fixture(&["workflow_explicit_completion"]);
    let consumer = consumer_contract_fixture(
        &["workflow_explicit_completion"],
        ContractCompatibilityPolicy::RequireAll,
    );

    let graph = build_dependency_contract_graph(&[consumer, provider]).unwrap();

    assert_eq!(
        graph.contracts.keys().collect::<Vec<_>>(),
        vec!["WI-01", "WI-02"]
    );
    assert_eq!(
        graph.edges,
        vec![DependencyContractEdge {
            from: "WI-01".to_string(),
            to: "WI-02".to_string(),
            required_contracts: vec![RequiredDependencyContract {
                contract_id: "contract.workflow".to_string(),
                required_capabilities: vec!["workflow_explicit_completion".to_string()],
                compatibility_policy: ContractCompatibilityPolicy::RequireAll,
            }],
        }]
    );
}

#[test]
fn canonical_work_item_dependency_edge_roundtrips_through_serde() {
    let edge = DependencyContractEdge {
        from: "WI-01".to_string(),
        to: "WI-02".to_string(),
        required_contracts: vec![RequiredDependencyContract {
            contract_id: "contract.workflow".to_string(),
            required_capabilities: vec!["workflow_explicit_completion".to_string()],
            compatibility_policy: ContractCompatibilityPolicy::RequireAny,
        }],
    };

    let value = serde_json::to_value(&edge).unwrap();
    assert_eq!(
        serde_json::from_value::<DependencyContractEdge>(value).unwrap(),
        edge
    );
}

#[test]
fn canonical_work_item_dependency_graph_rejects_duplicate_logical_identity() {
    let first = provider_contract_fixture(&["workflow_explicit_completion"]);
    let mut duplicate = first.clone();
    duplicate.identity.title = "Duplicate identity".to_string();

    let report = build_dependency_contract_graph(&[first, duplicate]).unwrap_err();

    assert!(!report.is_valid());
    assert!(report.findings.iter().any(|finding| {
        finding.code == "duplicate_logical_work_item_identity"
            && finding.logical_work_item_id.as_deref() == Some("WI-01")
    }));
}

#[test]
fn canonical_work_item_dependency_graph_has_stable_edge_and_contract_order() {
    let provider_one = provider_contract_fixture(&["capability.z", "capability.a"]);
    let mut provider_two = provider_contract_fixture(&["capability.b"]);
    provider_two.identity.logical_work_item_id = "WI-02".to_string();
    provider_two.output_contracts[0].contract_id = "contract.secondary".to_string();
    provider_two.handoff_contract.provided_contract_refs = vec!["contract.secondary".to_string()];

    let mut consumer = canonical_contract_fixture("WI-03");
    consumer.input_contracts = vec![
        RequiredInputContract {
            contract_id: "contract.secondary".to_string(),
            provider_logical_work_item_id: "WI-02".to_string(),
            required_capabilities: vec!["capability.b".to_string()],
            compatibility_policy: ContractCompatibilityPolicy::RequireAll,
        },
        RequiredInputContract {
            contract_id: "contract.workflow".to_string(),
            provider_logical_work_item_id: "WI-01".to_string(),
            required_capabilities: vec!["capability.z".to_string(), "capability.a".to_string()],
            compatibility_policy: ContractCompatibilityPolicy::RequireAll,
        },
    ];
    consumer.output_contracts.clear();
    consumer.handoff_contract.provided_contract_refs.clear();

    let first = build_dependency_contract_graph(&[
        consumer.clone(),
        provider_two.clone(),
        provider_one.clone(),
    ])
    .unwrap();
    consumer.input_contracts.reverse();
    let second = build_dependency_contract_graph(&[provider_one, consumer, provider_two]).unwrap();

    assert_eq!(
        first.contracts.keys().collect::<Vec<_>>(),
        second.contracts.keys().collect::<Vec<_>>()
    );
    assert_eq!(first.edges, second.edges);
    assert_eq!(
        first
            .edges
            .iter()
            .map(|edge| (edge.from.as_str(), edge.to.as_str()))
            .collect::<Vec<_>>(),
        vec![("WI-01", "WI-03"), ("WI-02", "WI-03")]
    );
    assert_eq!(
        first.edges[0].required_contracts[0].required_capabilities,
        vec!["capability.a", "capability.z"]
    );
}

#[test]
fn canonical_work_item_dependency_validation_accepts_satisfied_graph() {
    let provider = provider_contract_fixture(&["workflow_explicit_completion"]);
    let consumer = consumer_contract_fixture(
        &["workflow_explicit_completion"],
        ContractCompatibilityPolicy::RequireAll,
    );
    let graph = build_dependency_contract_graph(&[provider, consumer]).unwrap();

    let report = validate_dependency_contract_graph(&graph);

    assert!(report.findings.is_empty());
    assert!(report.is_valid());
}

#[test]
fn canonical_work_item_dependency_validation_reports_unknown_provider() {
    let mut consumer = consumer_contract_fixture(
        &["workflow_explicit_completion"],
        ContractCompatibilityPolicy::RequireAll,
    );
    consumer.input_contracts[0].provider_logical_work_item_id = "WI-404".to_string();
    let graph = build_dependency_contract_graph(&[consumer]).unwrap();

    let report = validate_dependency_contract_graph(&graph);

    assert!(report.findings.iter().any(|finding| {
        finding.code == "unknown_provider_logical_work_item"
            && finding.logical_work_item_id.as_deref() == Some("WI-404")
            && finding.contract_ref.as_deref() == Some("contract.workflow")
            && !finding.message.is_empty()
    }));
}

#[test]
fn canonical_work_item_dependency_validation_reports_cycle() {
    let mut first = provider_contract_fixture(&["first"]);
    first.output_contracts[0].contract_id = "contract.first".to_string();
    first.handoff_contract.provided_contract_refs = vec!["contract.first".to_string()];
    first.input_contracts = vec![RequiredInputContract {
        contract_id: "contract.second".to_string(),
        provider_logical_work_item_id: "WI-02".to_string(),
        required_capabilities: vec!["second".to_string()],
        compatibility_policy: ContractCompatibilityPolicy::RequireAll,
    }];

    let mut second = provider_contract_fixture(&["second"]);
    second.identity.logical_work_item_id = "WI-02".to_string();
    second.output_contracts[0].contract_id = "contract.second".to_string();
    second.handoff_contract.provided_contract_refs = vec!["contract.second".to_string()];
    second.input_contracts = vec![RequiredInputContract {
        contract_id: "contract.first".to_string(),
        provider_logical_work_item_id: "WI-01".to_string(),
        required_capabilities: vec!["first".to_string()],
        compatibility_policy: ContractCompatibilityPolicy::RequireAll,
    }];

    let graph = build_dependency_contract_graph(&[first, second]).unwrap();
    let report = validate_dependency_contract_graph(&graph);

    assert!(report.findings.iter().any(|finding| {
        finding.code == "dependency_cycle"
            && finding.logical_work_item_id.as_deref() == Some("WI-01")
            && finding.message.contains("WI-02")
    }));
}

#[test]
fn canonical_work_item_dependency_validation_reports_missing_contract() {
    let mut provider = provider_contract_fixture(&["workflow_explicit_completion"]);
    provider.output_contracts[0].contract_id = "contract.other".to_string();
    provider.handoff_contract.provided_contract_refs.clear();
    let consumer = consumer_contract_fixture(
        &["workflow_explicit_completion"],
        ContractCompatibilityPolicy::RequireAll,
    );
    let graph = build_dependency_contract_graph(&[provider, consumer]).unwrap();

    let report = validate_dependency_contract_graph(&graph);

    assert!(report.findings.iter().any(|finding| {
        finding.code == "required_contract_missing"
            && finding.logical_work_item_id.as_deref() == Some("WI-02")
            && finding.contract_ref.as_deref() == Some("contract.workflow")
    }));
}

#[test]
fn canonical_work_item_dependency_validation_reports_missing_capability() {
    let provider = provider_contract_fixture(&["workflow_explicit_completion"]);
    let consumer = consumer_contract_fixture(
        &["workflow_explicit_completion", "finalization_failure"],
        ContractCompatibilityPolicy::RequireAll,
    );

    let graph = build_dependency_contract_graph(&[provider, consumer]).unwrap();
    let report = validate_dependency_contract_graph(&graph);

    assert!(report.findings.iter().any(|finding| {
        finding.code == "required_capability_missing"
            && finding.logical_work_item_id.as_deref() == Some("WI-02")
            && finding.contract_ref.as_deref() == Some("contract.workflow")
            && finding.capability_ref.as_deref() == Some("finalization_failure")
    }));
}

#[test]
fn canonical_work_item_dependency_require_all_reports_every_missing_capability() {
    let provider = provider_contract_fixture(&[]);
    let consumer = consumer_contract_fixture(
        &["capability.b", "capability.a"],
        ContractCompatibilityPolicy::RequireAll,
    );
    let graph = build_dependency_contract_graph(&[provider, consumer]).unwrap();

    let report = validate_dependency_contract_graph(&graph);
    let capabilities = report
        .findings
        .iter()
        .filter(|finding| finding.code == "required_capability_missing")
        .filter_map(|finding| finding.capability_ref.as_deref())
        .collect::<Vec<_>>();

    assert_eq!(capabilities, vec!["capability.a", "capability.b"]);
}

#[test]
fn canonical_work_item_dependency_require_any_accepts_one_available_capability() {
    let provider = provider_contract_fixture(&["capability.b"]);
    let consumer = consumer_contract_fixture(
        &["capability.a", "capability.b"],
        ContractCompatibilityPolicy::RequireAny,
    );
    let graph = build_dependency_contract_graph(&[provider, consumer]).unwrap();

    assert_eq!(finding_count(&graph, "required_capability_missing"), 0);
}

#[test]
fn canonical_work_item_dependency_require_any_reports_when_no_capability_is_available() {
    let provider = provider_contract_fixture(&["capability.other"]);
    let consumer = consumer_contract_fixture(
        &["capability.b", "capability.a"],
        ContractCompatibilityPolicy::RequireAny,
    );
    let graph = build_dependency_contract_graph(&[provider, consumer]).unwrap();

    let report = validate_dependency_contract_graph(&graph);
    let capabilities = report
        .findings
        .iter()
        .filter(|finding| finding.code == "required_capability_missing")
        .filter_map(|finding| finding.capability_ref.as_deref())
        .collect::<Vec<_>>();

    assert_eq!(capabilities, vec!["capability.a", "capability.b"]);
}

#[test]
fn canonical_work_item_dependency_require_any_accepts_empty_requirement() {
    let provider = provider_contract_fixture(&[]);
    let consumer = consumer_contract_fixture(&[], ContractCompatibilityPolicy::RequireAny);
    let graph = build_dependency_contract_graph(&[provider, consumer]).unwrap();

    assert_eq!(finding_count(&graph, "required_capability_missing"), 0);
}

#[test]
fn canonical_work_item_dependency_validation_reports_unconsumed_required_handoff() {
    let provider = provider_contract_fixture(&["workflow_explicit_completion"]);
    let graph = build_dependency_contract_graph(&[provider]).unwrap();

    let report = validate_dependency_contract_graph(&graph);

    assert!(report.findings.iter().any(|finding| {
        finding.code == "unconsumed_required_handoff"
            && finding.logical_work_item_id.as_deref() == Some("WI-01")
            && finding.contract_ref.as_deref() == Some("contract.workflow")
    }));
}

#[test]
fn canonical_work_item_dependency_validation_reports_duplicate_edge() {
    let provider = provider_contract_fixture(&["workflow_explicit_completion"]);
    let consumer = consumer_contract_fixture(
        &["workflow_explicit_completion"],
        ContractCompatibilityPolicy::RequireAll,
    );
    let mut graph = build_dependency_contract_graph(&[provider, consumer]).unwrap();
    graph.edges.push(graph.edges[0].clone());

    let report = validate_dependency_contract_graph(&graph);

    assert!(report.findings.iter().any(|finding| {
        finding.code == "duplicate_dependency_contract_edge"
            && finding.logical_work_item_id.as_deref() == Some("WI-02")
            && finding.message.contains("WI-01")
    }));
}

#[test]
fn canonical_work_item_dependency_validation_reports_duplicate_required_contract_edge() {
    let provider = provider_contract_fixture(&["workflow_explicit_completion"]);
    let mut consumer = consumer_contract_fixture(
        &["workflow_explicit_completion"],
        ContractCompatibilityPolicy::RequireAll,
    );
    consumer
        .input_contracts
        .push(consumer.input_contracts[0].clone());
    let graph = build_dependency_contract_graph(&[provider, consumer]).unwrap();

    let report = validate_dependency_contract_graph(&graph);

    assert!(report.findings.iter().any(|finding| {
        finding.code == "duplicate_dependency_contract_edge"
            && finding.logical_work_item_id.as_deref() == Some("WI-02")
            && finding.contract_ref.as_deref() == Some("contract.workflow")
    }));
}

#[test]
fn canonical_work_item_dependency_handoff_consumption_ignores_human_text() {
    let provider = provider_contract_fixture(&["workflow_explicit_completion"]);
    let mut unrelated = consumer_contract_fixture(&[], ContractCompatibilityPolicy::RequireAny);
    unrelated.input_contracts.clear();
    unrelated.identity.title = "Consumes contract.workflow from WI-01".to_string();
    let graph = build_dependency_contract_graph(&[provider, unrelated]).unwrap();

    let report = validate_dependency_contract_graph(&graph);

    assert!(report.findings.iter().any(|finding| {
        finding.code == "unconsumed_required_handoff"
            && finding.logical_work_item_id.as_deref() == Some("WI-01")
            && finding.contract_ref.as_deref() == Some("contract.workflow")
    }));
}

#[test]
fn canonical_work_item_dependency_validation_orders_findings_deterministically() {
    let mut consumer = consumer_contract_fixture(
        &["capability.b", "capability.a"],
        ContractCompatibilityPolicy::RequireAll,
    );
    consumer.input_contracts[0].provider_logical_work_item_id = "WI-404".to_string();
    let mut graph = build_dependency_contract_graph(&[consumer]).unwrap();
    graph.edges.push(graph.edges[0].clone());

    let first = validate_dependency_contract_graph(&graph);
    let second = validate_dependency_contract_graph(&graph);
    let codes = first
        .findings
        .iter()
        .map(|finding| finding.code.as_str())
        .collect::<Vec<_>>();

    assert_eq!(first, second);
    assert!(codes.windows(2).all(|pair| pair[0] <= pair[1]));
}

#[test]
fn canonical_work_item_dependency_graph_revision_uses_typed_edges() {
    let edge = DependencyContractEdge {
        from: "WI-01".to_string(),
        to: "WI-02".to_string(),
        required_contracts: vec![RequiredDependencyContract {
            contract_id: "contract.workflow".to_string(),
            required_capabilities: vec!["workflow_explicit_completion".to_string()],
            compatibility_policy: ContractCompatibilityPolicy::RequireAll,
        }],
    };
    let revision = DependencyGraphRevision {
        id: "dependency_graph_revision_0001".to_string(),
        plan_id: "plan_0001".to_string(),
        edges: vec![edge.clone()],
        created_at: "2026-07-17T00:00:00Z".to_string(),
    };

    let value = serde_json::to_value(&revision).unwrap();
    assert_eq!(
        serde_json::from_value::<DependencyGraphRevision>(value).unwrap(),
        revision
    );
    assert_eq!(revision.edges, vec![edge]);
}

#[test]
fn canonical_work_item_dependency_graph_can_be_constructed_from_explicit_parts() {
    let provider = provider_contract_fixture(&["workflow_explicit_completion"]);
    let graph = DependencyContractGraph {
        contracts: BTreeMap::from([("WI-01".to_string(), provider)]),
        edges: vec![],
    };

    assert_eq!(graph.contracts.len(), 1);
}
