use std::collections::BTreeMap;

use crate::product::app_paths::ProductAppPaths;
use crate::product::models::{DependencyGraphRevision, WorkItemPlanLineage};
use crate::product::work_item_contract::{
    CanonicalWorkItemContract, ContractCompatibilityPolicy, DependencyContractEdge,
    DependencyContractGraph, PromisedOutputContract, RequiredDependencyContract,
    RequiredInputContract, canonical_contract_fixture,
};
use crate::product::work_item_revision_store::WorkItemRevisionStore;

use super::super::{PlanRepairEngine, SubgraphReplanRequest, SubgraphReplanner};

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
    DependencyContractEdge {
        from: from.to_string(),
        to: to.to_string(),
        required_contracts: vec![RequiredDependencyContract {
            contract_id: contract_id.to_string(),
            required_capabilities: vec![format!("capability_{contract_id}")],
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
            let contract = chain_contract(
                id,
                input
                    .as_ref()
                    .map(|(provider, contract_id)| (*provider, contract_id.as_str())),
                &format!("contract_{id}"),
            );
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
        dependency_graph_revision_id: "dependency_graph_revision_0002".to_string(),
        changed_logical_work_item_ids: changed.iter().map(|id| (*id).to_string()).collect(),
        replacement_contracts: replacements,
        replacement_mapping,
        story_spec_refs_changed: false,
        design_spec_refs_changed: false,
        created_at: "2026-07-20T00:00:00Z".to_string(),
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
        .replan(
            &chain_graph(&["wi_a", "wi_b", "wi_c", "wi_d"]),
            &split_b_request("contract_wi_b"),
        )
        .unwrap();

    assert_eq!(result.input_boundary, vec!["wi_a"]);
    assert_eq!(result.output_boundary, vec!["wi_c"]);
    assert_eq!(result.replacement_mapping["wi_b"], vec!["wi_b1", "wi_b2"]);
    assert_eq!(result.affected_logical_work_items, vec!["wi_b"]);
    assert!(!result.full_replan_required);
    assert_eq!(
        result.dependency_graph_revision,
        DependencyGraphRevision {
            id: "dependency_graph_revision_0002".to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            edges: vec![
                required_edge("wi_a", "wi_b1", "contract_wi_a"),
                required_edge("wi_b1", "wi_b2", "contract_wi_b1"),
                required_edge("wi_b2", "wi_c", "contract_wi_b"),
                required_edge("wi_c", "wi_d", "contract_wi_c"),
            ],
            created_at: "2026-07-20T00:00:00Z".to_string(),
        }
    );
}

#[test]
fn plan_repair_subgraph_expands_when_output_boundary_is_not_satisfied() {
    let result = SubgraphReplanner::default()
        .replan(
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
}

#[test]
fn plan_repair_subgraph_merge_preserves_many_to_one_replacement_mapping() {
    let merged = chain_contract("wi_bc", Some(("wi_a", "contract_wi_a")), "contract_wi_c");
    let result = SubgraphReplanner::default()
        .replan(
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
    assert!(
        result
            .dependency_graph_revision
            .edges
            .contains(&required_edge("wi_bc", "wi_d", "contract_wi_c"))
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
        .replan(&chain_graph(&["wi_a", "wi_b", "wi_c"]), &whole_graph)
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
            .replan(&chain_graph(&["wi_a", "wi_b", "wi_c"]), &source_changed)
            .unwrap();
        assert!(result.full_replan_required);
    }
}

#[test]
fn plan_repair_subgraph_rejects_missing_or_ambiguous_replacement_identity() {
    let graph = chain_graph(&["wi_a", "wi_b", "wi_c"]);
    let mut missing_mapping = split_b_request("contract_wi_b");
    missing_mapping.replacement_mapping.clear();
    assert!(
        SubgraphReplanner::default()
            .replan(&graph, &missing_mapping)
            .is_err()
    );

    let mut duplicate_replacement = split_b_request("contract_wi_b");
    duplicate_replacement
        .replacement_contracts
        .push(duplicate_replacement.replacement_contracts[0].clone());
    assert!(
        SubgraphReplanner::default()
            .replan(&graph, &duplicate_replacement)
            .is_err()
    );
}

#[test]
fn plan_repair_subgraph_engine_binds_replan_to_its_plan_lineage() {
    let tmp = tempfile::TempDir::new().unwrap();
    let engine = PlanRepairEngine::new(
        WorkItemRevisionStore::new(ProductAppPaths::new(tmp.path().join(".aria"))),
        WorkItemPlanLineage {
            id: "work_item_plan_0001".to_string(),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            story_spec_refs: vec!["story_spec_0001".to_string()],
            design_spec_refs: vec!["design_spec_0001".to_string()],
            active_revision_id: Some("plan_revision_0001".to_string()),
            active_amendment_id: Some("plan_amendment_0001".to_string()),
            created_at: "2026-07-20T00:00:00Z".to_string(),
            updated_at: "2026-07-20T00:00:00Z".to_string(),
        },
    );
    let graph = chain_graph(&["wi_a", "wi_b", "wi_c"]);
    let valid = split_b_request("contract_wi_b");
    assert!(engine.replan_subgraph(&graph, &valid).is_ok());

    let mut cross_plan = valid;
    cross_plan.plan_id = "work_item_plan_other".to_string();
    assert!(engine.replan_subgraph(&graph, &cross_plan).is_err());
}
