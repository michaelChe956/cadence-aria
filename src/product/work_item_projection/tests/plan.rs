use std::collections::BTreeMap;

use super::compiled_plan_fixture;
use crate::product::work_item_contract::{
    ContractCompatibilityPolicy, DependencyContractEdge, DependencyContractGraph,
    RequiredDependencyContract,
};
use crate::product::work_item_projection::{
    PlanProjectionCompileInput, PlanProjectionCompiler, ProjectionCompileError,
    validate_plan_projection_coverage,
};

#[test]
fn work_item_projection_plan_compiles_stable_topology_flow_matrix_and_edges() {
    let (graph, work_items) = compiled_plan_fixture();
    let compiled = compile_plan(&graph, &work_items).unwrap();

    assert_eq!(
        compiled.coder.ordered_logical_work_item_ids,
        vec!["wi_independent", "wi_provider", "wi_consumer"]
    );
    assert_eq!(compiled.coder.dependency_edges, graph.edges);
    assert_eq!(compiled.reviewer.dependency_edges, graph.edges);
    assert_eq!(
        compiled
            .human
            .work_items
            .iter()
            .map(|item| item.logical_work_item_id.as_str())
            .collect::<Vec<_>>(),
        vec!["wi_independent", "wi_provider", "wi_consumer"]
    );
    assert_eq!(compiled.human.contract_flow.len(), 1);
    let flow = &compiled.human.contract_flow[0];
    assert_eq!(flow.from, "wi_provider");
    assert_eq!(flow.to, "wi_consumer");
    assert_eq!(flow.contract_id, "contract.shared");
    assert_eq!(flow.required_capabilities, vec!["capability.a"]);
    assert_eq!(
        flow.provided_capabilities,
        vec!["capability.a", "capability.b"]
    );
    assert!(flow.missing_capabilities.is_empty());
    assert!(compiled.human.risks.is_empty());
    assert!(!compiled.human.normative);
    assert!(!compiled.human.used_by_provider);
    assert_eq!(compiled.reviewer.work_items.len(), graph.contracts.len());
    assert_eq!(
        compiled.coder.group_write_scopes["wi_consumer"],
        work_items["wi_consumer"].coder.write_policy
    );
    assert!(validate_plan_projection_coverage(&graph, &compiled, &work_items).is_valid());
}

#[test]
fn work_item_projection_plan_reports_only_structured_contract_gaps_as_risks() {
    let (mut graph, mut work_items) = compiled_plan_fixture();
    graph.edges[0].required_contracts[0]
        .required_capabilities
        .push("capability.missing".to_string());
    work_items.get_mut("wi_consumer").unwrap().human.title =
        "Free text must not become a risk".to_string();

    let compiled = compile_plan(&graph, &work_items).unwrap();

    assert_eq!(
        compiled.human.contract_flow[0].missing_capabilities,
        vec!["capability.missing"]
    );
    assert_eq!(compiled.human.risks.len(), 1);
    assert!(compiled.human.risks[0].contains("capability.missing"));
    assert!(!compiled.human.risks[0].contains("Free text"));
}

#[test]
fn work_item_projection_plan_rejects_cycle_unknown_nodes_and_missing_projection() {
    let (graph, work_items) = compiled_plan_fixture();

    let mut cyclic = graph.clone();
    cyclic.edges.push(DependencyContractEdge {
        from: "wi_consumer".to_string(),
        to: "wi_provider".to_string(),
        required_contracts: vec![RequiredDependencyContract {
            contract_id: "contract.consumer".to_string(),
            required_capabilities: vec!["consumer.ready".to_string()],
            compatibility_policy: ContractCompatibilityPolicy::RequireAll,
        }],
    });
    assert_validation_error(compile_plan(&cyclic, &work_items));

    let mut unknown = graph.clone();
    unknown.edges.push(DependencyContractEdge {
        from: "wi_provider".to_string(),
        to: "wi_unknown".to_string(),
        required_contracts: vec![],
    });
    assert_validation_error(compile_plan(&unknown, &work_items));

    let mut missing = work_items.clone();
    missing.remove("wi_consumer");
    assert_validation_error(compile_plan(&graph, &missing));

    let mut extra = work_items;
    extra.insert("wi_unknown".to_string(), extra["wi_provider"].clone());
    assert_validation_error(compile_plan(&graph, &extra));
}

#[test]
fn work_item_projection_plan_validation_detects_work_item_edge_and_matrix_mismatches() {
    let (graph, work_items) = compiled_plan_fixture();
    let baseline = compile_plan(&graph, &work_items).unwrap();

    let mut changed = baseline.clone();
    changed.human.work_items.pop();
    assert_has_code(
        validate_plan_projection_coverage(&graph, &changed, &work_items),
        "plan_projection_work_item_mismatch",
    );

    changed = baseline.clone();
    changed.coder.dependency_edges.clear();
    assert_has_code(
        validate_plan_projection_coverage(&graph, &changed, &work_items),
        "plan_projection_edge_mismatch",
    );

    changed = baseline.clone();
    changed.reviewer.work_items[0]
        .criterion_refs
        .push("AC-INVENTED".to_string());
    let first = validate_plan_projection_coverage(&graph, &changed, &work_items);
    let second = validate_plan_projection_coverage(&graph, &changed, &work_items);
    assert_has_code(first.clone(), "plan_projection_matrix_mismatch");
    assert_eq!(first, second);

    changed = baseline.clone();
    changed.reviewer.design_traceability_refs.clear();
    assert_has_code(
        validate_plan_projection_coverage(&graph, &changed, &work_items),
        "plan_projection_matrix_mismatch",
    );
}

#[test]
fn work_item_projection_plan_validation_rejects_invented_risks() {
    let (graph, work_items) = compiled_plan_fixture();
    let mut changed = compile_plan(&graph, &work_items).unwrap();
    changed
        .human
        .risks
        .push("Invented free-text risk".to_string());
    assert_has_code(
        validate_plan_projection_coverage(&graph, &changed, &work_items),
        "plan_projection_risk_mismatch",
    );
}

fn compile_plan(
    graph: &DependencyContractGraph,
    work_items: &BTreeMap<
        String,
        crate::product::work_item_projection::CompiledWorkItemProjections,
    >,
) -> Result<crate::product::work_item_projection::CompiledPlanProjections, ProjectionCompileError> {
    PlanProjectionCompiler.compile(PlanProjectionCompileInput {
        plan_id: "plan_0001",
        goal: "Compile the plan",
        split_reason: "Explicit dependency boundary",
        source_refs: &["design_0001".to_string(), "story_0001".to_string()],
        dependency_graph: graph,
        work_item_projections: work_items,
    })
}

fn assert_validation_error(
    result: Result<
        crate::product::work_item_projection::CompiledPlanProjections,
        ProjectionCompileError,
    >,
) {
    assert!(matches!(result, Err(ProjectionCompileError::Validation(_))));
}

fn assert_has_code(
    report: crate::product::work_item_projection::ProjectionValidationReport,
    code: &str,
) {
    assert!(
        report.findings.iter().any(|finding| finding.code == code),
        "missing finding {code}: {:?}",
        report.findings
    );
}
