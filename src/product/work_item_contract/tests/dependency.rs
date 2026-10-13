use std::collections::{BTreeMap, BTreeSet};

use super::canonical_contract_fixture;
use crate::product::{
    models::DependencyGraphRevision,
    work_item_contract::{
        CanonicalWorkItemContract, ContractCapabilityCoverage, ContractCompatibilityPolicy,
        ContractDependencyItemFact, ContractHandoffConsumption, ContractValidationFinding,
        DependencyContractEdge, DependencyContractGraph, PromisedOutputContract,
        RequiredDependencyContract, RequiredInputContract, build_dependency_contract_graph,
        project_contract_capability_coverage, project_contract_reviewer_coverage,
        validate_dependency_contract_graph,
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

#[test]
fn p2_reviewer_handoff_projection_reports_empty_consumer_set_field_exactly() {
    let mut provider = provider_contract_fixture(&["capability.present"]);
    provider.handoff_contract.provided_contract_refs = vec!["CT-005".to_string()];
    let consumer = consumer_contract_fixture(&[], ContractCompatibilityPolicy::RequireAny);
    let graph = build_dependency_contract_graph(&[provider, consumer]).expect("graph");
    let projection = project_contract_reviewer_coverage(&graph);
    assert_eq!(
        projection.handoff_consumption,
        vec![ContractHandoffConsumption {
            provider: "WI-01".to_string(),
            contract_ref: "CT-005".to_string(),
            consumers: Vec::new(),
            consumed: false,
        }]
    );
    let report = validate_dependency_contract_graph(&graph);
    assert_eq!(
        report
            .findings
            .iter()
            .filter(|finding| {
                finding.code == "unconsumed_required_handoff"
                    && finding.logical_work_item_id.as_deref() == Some("WI-01")
                    && finding.contract_ref.as_deref() == Some("CT-005")
            })
            .count(),
        1
    );
}

#[test]
fn p2_reviewer_handoff_projection_consumers_match_validator_for_multiple_consumers() {
    let mut provider = provider_contract_fixture(&["capability.present"]);
    provider.output_contracts[0].contract_id = "CT-005".to_string();
    provider.handoff_contract.provided_contract_refs = vec!["CT-005".to_string()];
    let mut first = consumer_contract_fixture(
        &["capability.present"],
        ContractCompatibilityPolicy::RequireAll,
    );
    first.input_contracts[0].contract_id = "CT-005".to_string();
    let mut second = first.clone();
    second.identity.logical_work_item_id = "WI-03".to_string();
    let graph = build_dependency_contract_graph(&[provider, first, second]).expect("graph");
    let projection = project_contract_reviewer_coverage(&graph);
    assert_eq!(
        projection.handoff_consumption,
        vec![ContractHandoffConsumption {
            provider: "WI-01".to_string(),
            contract_ref: "CT-005".to_string(),
            consumers: vec!["WI-02".to_string(), "WI-03".to_string()],
            consumed: true,
        }]
    );
    assert!(
        validate_dependency_contract_graph(&graph)
            .findings
            .iter()
            .all(|finding| {
                finding.code != "unconsumed_required_handoff"
                    || finding.contract_ref.as_deref() != Some("CT-005")
            })
    );
}

#[test]
fn p2_reviewer_dependency_and_scope_projection_facts_are_field_exact_and_stable() {
    let mut first = provider_contract_fixture(&["capability.present"]);
    first.depends_on = vec!["WI-02".to_string()];
    first.write_policy.exclusive_scopes = vec!["src/shared/**".to_string()];
    first.write_policy.forbidden_scopes.clear();
    let mut second = consumer_contract_fixture(
        &["capability.present"],
        ContractCompatibilityPolicy::RequireAll,
    );
    second.depends_on = vec!["WI-01".to_string()];
    second.write_policy.exclusive_scopes.clear();
    second.write_policy.forbidden_scopes = vec!["src/shared/**".to_string()];
    let required = RequiredDependencyContract {
        contract_id: "contract.workflow".to_string(),
        required_capabilities: vec!["capability.present".to_string()],
        compatibility_policy: ContractCompatibilityPolicy::RequireAll,
    };
    let graph = DependencyContractGraph {
        contracts: BTreeMap::from([("WI-01".to_string(), first), ("WI-02".to_string(), second)]),
        edges: vec![
            DependencyContractEdge {
                from: "WI-01".to_string(),
                to: "WI-02".to_string(),
                required_contracts: vec![required.clone(), required.clone()],
            },
            DependencyContractEdge {
                from: "WI-01".to_string(),
                to: "WI-02".to_string(),
                required_contracts: vec![required.clone()],
            },
            DependencyContractEdge {
                from: "WI-02".to_string(),
                to: "WI-01".to_string(),
                required_contracts: vec![required.clone()],
            },
            DependencyContractEdge {
                from: "WI-404".to_string(),
                to: "WI-02".to_string(),
                required_contracts: vec![required],
            },
        ],
    };
    let first_projection = project_contract_reviewer_coverage(&graph);
    let second_projection = project_contract_reviewer_coverage(&graph);
    assert_eq!(first_projection, second_projection);
    assert_eq!(
        first_projection.dependency_graph.depends_on,
        vec![
            ContractDependencyItemFact {
                work_item_id: "WI-01".to_string(),
                depends_on: vec!["WI-02".to_string()],
            },
            ContractDependencyItemFact {
                work_item_id: "WI-02".to_string(),
                depends_on: vec!["WI-01".to_string()],
            },
        ]
    );
    assert_eq!(
        first_projection.dependency_graph.cycles,
        vec![vec!["WI-01".to_string(), "WI-02".to_string()]]
    );
    assert_eq!(
        first_projection.dependency_graph.unknown_providers[0].provider,
        "WI-404"
    );
    assert!(
        first_projection
            .dependency_graph
            .duplicate_edges
            .iter()
            .any(|fact| fact.from == "WI-01" && fact.to == "WI-02")
    );
    assert!(
        first_projection
            .write_scope_conflicts
            .iter()
            .any(|fact| fact.left_work_item_id == "WI-01"
                && fact.left_scope == "src/shared/**"
                && fact.right_work_item_id == "WI-02"
                && fact.right_scope == "src/shared/**")
    );
    let report = validate_dependency_contract_graph(&graph);
    for code in [
        "duplicate_dependency_contract_edge",
        "dependency_cycle",
        "unknown_provider_logical_work_item",
    ] {
        assert!(report.findings.iter().any(|finding| finding.code == code));
    }
}

fn finding_count(graph: &DependencyContractGraph, code: &str) -> usize {
    validate_dependency_contract_graph(graph)
        .findings
        .iter()
        .filter(|finding| finding.code == code)
        .count()
}

#[test]
fn reviewer_capability_coverage_projection_complete_graph_is_field_exact() {
    let provider = provider_contract_fixture(&["capability.b", "capability.a"]);
    let consumer =
        consumer_contract_fixture(&["capability.a"], ContractCompatibilityPolicy::RequireAll);
    let graph = build_dependency_contract_graph(&[provider, consumer]).unwrap();

    assert_eq!(
        project_contract_capability_coverage(&graph),
        vec![ContractCapabilityCoverage {
            from: "WI-01".into(),
            to: "WI-02".into(),
            contract_id: "contract.workflow".into(),
            required_capabilities: vec!["capability.a".into()],
            provided_capabilities: vec!["capability.a".into(), "capability.b".into()],
            missing_capabilities: vec![],
            compatibility_policy: ContractCompatibilityPolicy::RequireAll,
        }]
    );
}

#[test]
fn reviewer_capability_coverage_projection_gap_matches_validator_missing_set() {
    let provider = provider_contract_fixture(&["capability.present"]);
    let consumer = consumer_contract_fixture(
        &["capability.missing", "capability.present"],
        ContractCompatibilityPolicy::RequireAll,
    );
    let graph = build_dependency_contract_graph(&[provider, consumer]).unwrap();
    let coverage = project_contract_capability_coverage(&graph);
    let report = validate_dependency_contract_graph(&graph);
    let projected_missing = coverage
        .iter()
        .flat_map(|entry| entry.missing_capabilities.iter())
        .cloned()
        .collect::<BTreeSet<_>>();
    let validator_missing = report
        .findings
        .iter()
        .filter(|finding| finding.code == "required_capability_missing")
        .filter_map(|finding| finding.capability_ref.clone())
        .collect::<BTreeSet<_>>();
    assert_eq!(projected_missing, validator_missing);
}

#[test]
fn reviewer_capability_coverage_projection_require_all_multi_gap_is_sorted_and_matches_validator() {
    let provider = provider_contract_fixture(&["capability.present"]);
    let consumer = consumer_contract_fixture(
        &["capability.z", "capability.present", "capability.a"],
        ContractCompatibilityPolicy::RequireAll,
    );
    let graph = build_dependency_contract_graph(&[provider, consumer]).unwrap();
    let coverage = project_contract_capability_coverage(&graph);
    let report = validate_dependency_contract_graph(&graph);
    let projected_missing = coverage
        .iter()
        .flat_map(|entry| entry.missing_capabilities.iter())
        .cloned()
        .collect::<BTreeSet<_>>();
    let validator_missing = report
        .findings
        .iter()
        .filter(|finding| finding.code == "required_capability_missing")
        .filter_map(|finding| finding.capability_ref.clone())
        .collect::<BTreeSet<_>>();

    assert_eq!(projected_missing, validator_missing);
    assert_eq!(
        coverage[0].missing_capabilities,
        vec!["capability.a", "capability.z"]
    );
}

#[test]
fn reviewer_capability_coverage_projection_missing_contract_fails_closed() {
    let mut consumer = consumer_contract_fixture(
        &["capability.missing"],
        ContractCompatibilityPolicy::RequireAll,
    );
    consumer.input_contracts[0].contract_id = "contract.unknown".to_string();
    let provider = provider_contract_fixture(&["capability.present"]);
    let graph = build_dependency_contract_graph(&[provider, consumer]).unwrap();
    let coverage = project_contract_capability_coverage(&graph);

    assert_eq!(coverage.len(), 1);
    assert!(coverage[0].provided_capabilities.is_empty());
    assert_eq!(coverage[0].missing_capabilities, vec!["capability.missing"]);
}

#[test]
fn reviewer_capability_coverage_projection_require_any_semantics_are_stable() {
    let provider = provider_contract_fixture(&["capability.present"]);
    let any_complete = consumer_contract_fixture(
        &["capability.missing", "capability.present"],
        ContractCompatibilityPolicy::RequireAny,
    );
    let mut any_gap = consumer_contract_fixture(
        &["capability.z", "capability.a"],
        ContractCompatibilityPolicy::RequireAny,
    );
    any_gap.identity.logical_work_item_id = "WI-03".into();
    any_gap.input_contracts[0].provider_logical_work_item_id = "WI-01".into();
    let complete_graph =
        build_dependency_contract_graph(&[provider.clone(), any_complete]).unwrap();
    let gap_graph = build_dependency_contract_graph(&[provider, any_gap]).unwrap();

    assert!(
        project_contract_capability_coverage(&complete_graph)[0]
            .missing_capabilities
            .is_empty()
    );
    assert_eq!(
        project_contract_capability_coverage(&gap_graph)[0].missing_capabilities,
        vec!["capability.a", "capability.z"]
    );
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

    let duplicate_findings = report
        .findings
        .iter()
        .filter(|finding| finding.code == "duplicate_dependency_contract_edge")
        .collect::<Vec<_>>();

    assert_eq!(duplicate_findings.len(), 1);
    assert_eq!(
        duplicate_findings[0].logical_work_item_id.as_deref(),
        Some("WI-02")
    );
    assert!(duplicate_findings[0].message.contains("WI-01"));
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
    consumer
        .input_contracts
        .push(consumer.input_contracts[0].clone());
    let graph = build_dependency_contract_graph(&[provider, consumer]).unwrap();

    let report = validate_dependency_contract_graph(&graph);

    let duplicate_findings = report
        .findings
        .iter()
        .filter(|finding| finding.code == "duplicate_dependency_contract_edge")
        .collect::<Vec<_>>();

    assert_eq!(duplicate_findings.len(), 1);
    assert_eq!(
        duplicate_findings[0].logical_work_item_id.as_deref(),
        Some("WI-02")
    );
    assert_eq!(
        duplicate_findings[0].contract_ref.as_deref(),
        Some("contract.workflow")
    );
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
    assert_eq!(
        first
            .findings
            .iter()
            .filter(|finding| finding.code == "duplicate_dependency_contract_edge")
            .count(),
        1
    );
    assert_eq!(
        first
            .findings
            .iter()
            .filter(|finding| finding.code == "unknown_provider_logical_work_item")
            .count(),
        1
    );
}

#[test]
fn canonical_work_item_dependency_validation_deduplicates_missing_contract_findings() {
    let mut provider = provider_contract_fixture(&["workflow_explicit_completion"]);
    provider.output_contracts[0].contract_id = "contract.other".to_string();
    provider.handoff_contract.provided_contract_refs.clear();
    let mut consumer = consumer_contract_fixture(
        &["workflow_explicit_completion"],
        ContractCompatibilityPolicy::RequireAll,
    );
    consumer
        .input_contracts
        .push(consumer.input_contracts[0].clone());
    let graph = build_dependency_contract_graph(&[provider, consumer]).unwrap();

    let report = validate_dependency_contract_graph(&graph);

    assert_eq!(
        report
            .findings
            .iter()
            .filter(|finding| finding.code == "duplicate_dependency_contract_edge")
            .count(),
        1
    );
    assert_eq!(
        report
            .findings
            .iter()
            .filter(|finding| finding.code == "required_contract_missing")
            .count(),
        1
    );
}

#[test]
fn canonical_work_item_dependency_validation_deduplicates_missing_capability_findings() {
    let provider = provider_contract_fixture(&["workflow_explicit_completion"]);
    let mut consumer = consumer_contract_fixture(
        &["finalization_failure"],
        ContractCompatibilityPolicy::RequireAll,
    );
    consumer
        .input_contracts
        .push(consumer.input_contracts[0].clone());
    let graph = build_dependency_contract_graph(&[provider, consumer]).unwrap();

    let report = validate_dependency_contract_graph(&graph);

    assert_eq!(
        report
            .findings
            .iter()
            .filter(|finding| finding.code == "duplicate_dependency_contract_edge")
            .count(),
        1
    );
    let capability_findings = report
        .findings
        .iter()
        .filter(|finding| finding.code == "required_capability_missing")
        .collect::<Vec<_>>();
    assert_eq!(capability_findings.len(), 1);
    assert_eq!(
        capability_findings[0].capability_ref.as_deref(),
        Some("finalization_failure")
    );
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

// F2a（REQ-SG-01/02、REQ-PLN-09）：canonical 引用供需 typed findings。
// 断言基于真实 fixture 的图/报告/序列化观察，不匹配中文文案。

use crate::product::planning_failure::{
    PlanningCycleWitnessHop, PlanningEdgeSource, PlanningFailure, PlanningFailureEvidence,
    PlanningFailureKind, PlanningFailureOrigin, PlanningFailureSet, PlanningRecoveryDisposition,
    PlanningStage, StructuralAuthorizationFacts, StructuralAuthorizationKind, classify_failure_set,
    is_complete_cycle_witness,
};

/// 三节点环：WI-01 消费 WI-02 的契约、WI-02 消费 WI-03、WI-03 消费 WI-01。
/// 有效边为 WI-02→WI-01、WI-03→WI-02、WI-01→WI-03（provider→consumer）。
fn three_node_cycle_contracts() -> Vec<CanonicalWorkItemContract> {
    fn node(
        logical_work_item_id: &str,
        output_contract: &str,
        input_from_provider: &str,
        input_contract: &str,
    ) -> CanonicalWorkItemContract {
        let mut node = canonical_contract_fixture(logical_work_item_id);
        node.input_contracts.clear();
        node.output_contracts = vec![PromisedOutputContract {
            contract_id: output_contract.to_string(),
            capabilities: vec![format!("{output_contract}.capability")],
        }];
        node.handoff_contract.provided_contract_refs = vec![output_contract.to_string()];
        if !input_from_provider.is_empty() {
            node.input_contracts = vec![RequiredInputContract {
                contract_id: input_contract.to_string(),
                provider_logical_work_item_id: input_from_provider.to_string(),
                required_capabilities: vec![format!("{input_contract}.capability")],
                compatibility_policy: ContractCompatibilityPolicy::RequireAll,
            }];
        }
        node
    }
    vec![
        node("WI-01", "contract.first", "WI-02", "contract.second"),
        node("WI-02", "contract.second", "WI-03", "contract.third"),
        node("WI-03", "contract.third", "WI-01", "contract.first"),
    ]
}

#[test]
fn sg_f2a_canonical_payload_cycle_witness_is_directed_not_sorted_set() {
    let graph = build_dependency_contract_graph(&three_node_cycle_contracts()).unwrap();
    let report = validate_dependency_contract_graph(&graph);

    let cycle = report
        .findings
        .iter()
        .find(|finding| finding.code == "dependency_cycle")
        .expect("cycle must be reported");
    let Some(PlanningFailureKind::DependencyCycle { witness }) = cycle.kind.as_ref() else {
        panic!("dependency_cycle finding must carry typed DependencyCycle payload");
    };
    // typed payload 是真实有向路径（DFS 栈序：WI-01→WI-03→WI-02→WI-01），
    // 不是 sort/dedup 节点集合（WI-01→WI-02→WI-03）；每一跳都确在本图
    // 邻接中且来源为 canonical Inputs（REQ-PLN-10：残余/排序集合不得画箭头）。
    let canonical_input = vec![PlanningEdgeSource::CanonicalInput];
    assert_eq!(
        witness,
        &vec![
            PlanningCycleWitnessHop {
                from: "WI-01".to_string(),
                to: "WI-03".to_string(),
                sources: canonical_input.clone(),
            },
            PlanningCycleWitnessHop {
                from: "WI-03".to_string(),
                to: "WI-02".to_string(),
                sources: canonical_input.clone(),
            },
            PlanningCycleWitnessHop {
                from: "WI-02".to_string(),
                to: "WI-01".to_string(),
                sources: canonical_input,
            },
        ]
    );
    assert!(is_complete_cycle_witness(witness));
    // 旧 canonical 判定规则不变：finding 身份与消息仍由稳定排序集合渲染。
    assert_eq!(
        cycle.message,
        "dependency cycle contains WI-01 -> WI-02 -> WI-03"
    );
    assert_eq!(
        cycle.logical_work_item_id.as_deref(),
        Some("WI-01"),
        "finding identity stays on the sorted-set head (旧规则)"
    );
}

#[test]
fn sg_f2a_canonical_payload_self_dependency_witness_is_single_real_hop() {
    let mut solo = provider_contract_fixture(&["capability.self"]);
    solo.output_contracts[0].contract_id = "contract.self".to_string();
    solo.handoff_contract.provided_contract_refs = vec!["contract.self".to_string()];
    solo.input_contracts = vec![RequiredInputContract {
        contract_id: "contract.self".to_string(),
        provider_logical_work_item_id: "WI-01".to_string(),
        required_capabilities: vec!["capability.self".to_string()],
        compatibility_policy: ContractCompatibilityPolicy::RequireAll,
    }];
    let graph = build_dependency_contract_graph(&[solo]).unwrap();
    let report = validate_dependency_contract_graph(&graph);

    let cycle = report
        .findings
        .iter()
        .find(|finding| finding.code == "dependency_cycle")
        .expect("self dependency must be reported as a cycle");
    let Some(PlanningFailureKind::DependencyCycle { witness }) = cycle.kind.as_ref() else {
        panic!("self dependency must carry typed DependencyCycle payload");
    };
    assert_eq!(
        witness,
        &vec![PlanningCycleWitnessHop {
            from: "WI-01".to_string(),
            to: "WI-01".to_string(),
            sources: vec![PlanningEdgeSource::CanonicalInput],
        }]
    );
    assert!(is_complete_cycle_witness(witness));
}

#[test]
fn sg_f2a_canonical_payload_supply_demand_gap_ids_not_lost() {
    // 未知 provider：consumer WI-02 真实引用 WI-404，三元 ID 全部保留。
    let mut unknown = consumer_contract_fixture(
        &["workflow_explicit_completion"],
        ContractCompatibilityPolicy::RequireAll,
    );
    unknown.input_contracts[0].provider_logical_work_item_id = "WI-404".to_string();
    let graph = build_dependency_contract_graph(&[unknown]).unwrap();
    let finding = validate_dependency_contract_graph(&graph)
        .findings
        .into_iter()
        .find(|finding| finding.code == "unknown_provider_logical_work_item")
        .expect("unknown provider must be reported");
    assert_eq!(
        finding.kind,
        Some(PlanningFailureKind::UnknownProvider {
            provider: "WI-404".to_string(),
            consumer: "WI-02".to_string(),
            contract_id: "contract.workflow".to_string(),
        })
    );

    // provider 存在但未声明被消费契约：供需缺口不丢 ID。
    let mut provider = provider_contract_fixture(&["workflow_explicit_completion"]);
    provider.output_contracts[0].contract_id = "contract.other".to_string();
    provider.handoff_contract.provided_contract_refs.clear();
    let consumer = consumer_contract_fixture(
        &["workflow_explicit_completion"],
        ContractCompatibilityPolicy::RequireAll,
    );
    let graph = build_dependency_contract_graph(&[provider, consumer]).unwrap();
    let finding = validate_dependency_contract_graph(&graph)
        .findings
        .into_iter()
        .find(|finding| finding.code == "required_contract_missing")
        .expect("missing contract must be reported");
    assert_eq!(
        finding.kind,
        Some(PlanningFailureKind::RequiredContractMissing {
            provider: "WI-01".to_string(),
            consumer: "WI-02".to_string(),
            contract_id: "contract.workflow".to_string(),
        })
    );

    // required capability 缺口：四元 ID 精确到 capability 级。
    let provider = provider_contract_fixture(&["workflow_explicit_completion"]);
    let consumer = consumer_contract_fixture(
        &["workflow_explicit_completion", "finalization_failure"],
        ContractCompatibilityPolicy::RequireAll,
    );
    let graph = build_dependency_contract_graph(&[provider, consumer]).unwrap();
    let finding = validate_dependency_contract_graph(&graph)
        .findings
        .into_iter()
        .find(|finding| finding.code == "required_capability_missing")
        .expect("missing capability must be reported");
    assert_eq!(
        finding.kind,
        Some(PlanningFailureKind::RequiredCapabilityMissing {
            provider: "WI-01".to_string(),
            consumer: "WI-02".to_string(),
            contract_id: "contract.workflow".to_string(),
            capability: "finalization_failure".to_string(),
        })
    );

    // kind 单源映射：稳定 code/category 与供需族分支一致。
    let kind = PlanningFailureKind::RequiredCapabilityMissing {
        provider: "WI-01".to_string(),
        consumer: "WI-02".to_string(),
        contract_id: "contract.workflow".to_string(),
        capability: "finalization_failure".to_string(),
    };
    assert_eq!(kind.stable_code(), "required_capability_missing");
    assert_eq!(
        PlanningFailureKind::UnknownProvider {
            provider: "p".to_string(),
            consumer: "c".to_string(),
            contract_id: "ct".to_string(),
        }
        .stable_code(),
        "unknown_provider_logical_work_item"
    );
    assert_eq!(
        PlanningFailureKind::RequiredContractMissing {
            provider: "p".to_string(),
            consumer: "c".to_string(),
            contract_id: "ct".to_string(),
        }
        .category()
        .as_str(),
        "dependency"
    );
    // 新 kind serde 形状：封闭 tag + snake_case，roundtrip 不变形。
    let value = serde_json::to_value(&kind).unwrap();
    assert_eq!(value["kind"], "required_capability_missing");
    assert_eq!(
        serde_json::from_value::<PlanningFailureKind>(value).unwrap(),
        kind
    );
}

#[test]
fn sg_f2a_canonical_payload_real_input_references_not_deleted() {
    // typed 校验是纯投影：检出缺口不删除/改写真实 Inputs 引用，
    // 图边保留真实供需边（REQ-PLN-09：不把未参与声明边冒称实际边，
    // 也不因检出删真实 Input）。
    let mut provider = provider_contract_fixture(&["workflow_explicit_completion"]);
    provider.output_contracts[0].contract_id = "contract.other".to_string();
    provider.handoff_contract.provided_contract_refs.clear();
    let consumer = consumer_contract_fixture(
        &["workflow_explicit_completion", "finalization_failure"],
        ContractCompatibilityPolicy::RequireAll,
    );
    let contracts = vec![provider, consumer];
    let before = contracts.clone();

    let graph = build_dependency_contract_graph(&contracts).unwrap();
    let report = validate_dependency_contract_graph(&graph);
    assert!(!report.is_valid());
    assert_eq!(contracts, before, "入参 contract 不被校验改写");

    let graph_consumer = graph.contracts.get("WI-02").unwrap();
    assert_eq!(graph_consumer.input_contracts.len(), 1);
    assert_eq!(
        graph_consumer.input_contracts[0].provider_logical_work_item_id,
        "WI-01"
    );
    assert_eq!(
        graph_consumer.input_contracts[0].contract_id,
        "contract.workflow"
    );
    assert_eq!(
        graph_consumer.input_contracts[0].required_capabilities,
        vec![
            "workflow_explicit_completion".to_string(),
            "finalization_failure".to_string()
        ]
    );
    assert!(
        graph
            .edges
            .iter()
            .any(|edge| edge.from == "WI-01" && edge.to == "WI-02"),
        "真实供需边保留在图中，不因缺口消失"
    );
}

/// F2a 测试共享：SC 修订 turn 活动授权事实（attempt 1/2 已消费 1 次）。
fn sg_f2a_active_sc_facts() -> StructuralAuthorizationFacts {
    StructuralAuthorizationFacts {
        authorization_id: "grant-sc-7f31".to_string(),
        authorization_kind: StructuralAuthorizationKind::ScTurn,
        parent_id: "turn-4140".to_string(),
        source_hash: "9f1c0a4d8b7e6253c0af1d2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b1c2".to_string(),
        context_digest: "ctx-5102beef".to_string(),
        active: true,
        anchor_waiting: false,
        launch_unknown: false,
        attempt_no: 1,
        max_attempts: 2,
    }
}

fn sg_f2a_failure_from_kind(kind: PlanningFailureKind) -> PlanningFailure {
    PlanningFailure {
        kind,
        origin: PlanningFailureOrigin::CanonicalContract,
        stage: PlanningStage::Plan,
        diagnostic_ref: "diag-f2a-01".to_string(),
        source_ref: "plan-source/rev-31".to_string(),
        source_hash: "9f1c0a4d8b7e6253c0af1d2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b1c2".to_string(),
        evidence: PlanningFailureEvidence::default(),
    }
}

#[test]
fn sg_f2a_canonical_payload_old_json_unknown_not_auto() {
    let provider = provider_contract_fixture(&["workflow_explicit_completion"]);
    let consumer = consumer_contract_fixture(
        &["workflow_explicit_completion", "finalization_failure"],
        ContractCompatibilityPolicy::RequireAll,
    );
    let graph = build_dependency_contract_graph(&[provider, consumer]).unwrap();
    let finding = validate_dependency_contract_graph(&graph)
        .findings
        .into_iter()
        .find(|finding| finding.code == "required_capability_missing")
        .expect("capability gap must be reported");
    let typed = finding
        .kind
        .clone()
        .expect("typed producer must write Some(kind)");

    // 旧 JSON（无 kind 字段）：解析为 None，再序列化不新增字段（字节合同，
    // plan §0.5 哈希兼容——旧 PlanValidationReportArtifact 记录不失效）。
    let mut legacy = serde_json::to_value(&finding).unwrap();
    let legacy_object = legacy.as_object_mut().unwrap();
    assert!(legacy_object.remove("kind").is_some());
    let legacy_finding: ContractValidationFinding = serde_json::from_value(legacy.clone()).unwrap();
    assert_eq!(legacy_finding.kind, None);
    assert_eq!(
        serde_json::to_value(&legacy_finding).unwrap(),
        legacy,
        "None 再序列化不得回添 kind 字段"
    );

    // None → Unknown 投影：活动授权下整批仍 UnknownStop（不自动）；
    // 资格来自 typed payload，不来自 code/文案。
    let facts = sg_f2a_active_sc_facts();
    let unknown_set = PlanningFailureSet {
        failures: vec![sg_f2a_failure_from_kind(PlanningFailureKind::Unknown {
            code: legacy_finding.code.clone(),
        })],
    };
    assert_eq!(
        classify_failure_set(&unknown_set, &facts),
        PlanningRecoveryDisposition::UnknownStop
    );
    let typed_set = PlanningFailureSet {
        failures: vec![sg_f2a_failure_from_kind(typed)],
    };
    assert_eq!(
        classify_failure_set(&typed_set, &facts),
        PlanningRecoveryDisposition::StructuralAuthor
    );
}
