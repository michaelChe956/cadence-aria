use std::collections::BTreeMap;

use super::{compiled_plan_fixture, expected_plan_revision_ids};
use crate::product::work_item_contract::{
    ContractCompatibilityPolicy, DependencyContractEdge, DependencyContractGraph,
    RequiredDependencyContract,
};
use crate::product::work_item_projection::{
    PlanProjectionCompileInput, PlanProjectionCompiler, PlanProjectionValidationInput,
    ProjectionCompileError, ProjectionValidationReport, validate_plan_projection_coverage,
};

const PLAN_ID: &str = "plan_0001";

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
    assert!(validate_plan(&graph, &compiled, &work_items).is_valid());
}

#[test]
fn work_item_projection_plan_rejects_canonical_projection_drift() {
    let (graph, baseline) = compiled_plan_fixture();

    let mut changed = baseline.clone();
    changed.get_mut("wi_consumer").unwrap().human.title = "Drifted title".to_string();
    assert_compile_finding(
        compile_plan(&graph, &changed),
        "projection_contract_mismatch",
        "plan.work_items.wi_consumer.human",
        Some("wi_consumer"),
    );

    changed = baseline.clone();
    changed
        .get_mut("wi_consumer")
        .unwrap()
        .coder
        .required_input_contracts
        .clear();
    assert_compile_finding(
        compile_plan(&graph, &changed),
        "projection_missing_contract_ref",
        "plan.work_items.wi_consumer.coder.inputs",
        Some("contract.shared"),
    );

    changed = baseline.clone();
    changed
        .get_mut("wi_consumer")
        .unwrap()
        .reviewer
        .output_contract_checks
        .clear();
    assert_compile_finding(
        compile_plan(&graph, &changed),
        "projection_missing_contract_ref",
        "plan.work_items.wi_consumer.reviewer.outputs",
        Some("contract.consumer"),
    );

    changed = baseline.clone();
    changed
        .get_mut("wi_consumer")
        .unwrap()
        .reviewer
        .scope_policy
        .exclusive_scopes
        .push("invented/scope".to_string());
    assert_compile_finding(
        compile_plan(&graph, &changed),
        "projection_contract_mismatch",
        "plan.work_items.wi_consumer.reviewer.scope",
        Some("wi_consumer"),
    );

    changed = baseline;
    changed
        .get_mut("wi_consumer")
        .unwrap()
        .coder
        .acceptance_criteria
        .clear();
    assert_compile_finding(
        compile_plan(&graph, &changed),
        "projection_missing_contract_ref",
        "plan.work_items.wi_consumer.coder.acceptance_criteria",
        Some("AC-001"),
    );
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
    assert_compile_finding(
        compile_plan(&cyclic, &work_items),
        "dependency_cycle",
        "plan.dependency_graph",
        None,
    );

    let mut unknown = graph.clone();
    unknown.edges.push(DependencyContractEdge {
        from: "wi_unknown".to_string(),
        to: "wi_consumer".to_string(),
        required_contracts: vec![RequiredDependencyContract {
            contract_id: "contract.unknown".to_string(),
            required_capabilities: vec!["unknown.ready".to_string()],
            compatibility_policy: ContractCompatibilityPolicy::RequireAll,
        }],
    });
    assert_compile_finding(
        compile_plan(&unknown, &work_items),
        "unknown_provider_logical_work_item",
        "plan.dependency_graph",
        Some("contract.unknown"),
    );

    let mut duplicate_edge = graph.clone();
    duplicate_edge.edges.push(duplicate_edge.edges[0].clone());
    assert_compile_finding(
        compile_plan(&duplicate_edge, &work_items),
        "duplicate_dependency_contract_edge",
        "plan.dependency_graph",
        None,
    );

    let mut duplicate_contract = graph.clone();
    let duplicate = duplicate_contract.edges[0].required_contracts[0].clone();
    duplicate_contract.edges[0]
        .required_contracts
        .push(duplicate);
    assert_compile_finding(
        compile_plan(&duplicate_contract, &work_items),
        "duplicate_dependency_contract_edge",
        "plan.dependency_graph",
        Some("contract.shared"),
    );

    let mut missing_capability = graph.clone();
    missing_capability.edges[0].required_contracts[0]
        .required_capabilities
        .push("capability.missing".to_string());
    assert_compile_finding(
        compile_plan(&missing_capability, &work_items),
        "required_capability_missing",
        "plan.dependency_graph",
        Some("contract.shared"),
    );

    let mut missing = work_items.clone();
    missing.remove("wi_consumer");
    assert_compile_finding(
        compile_plan(&graph, &missing),
        "plan_projection_work_item_mismatch",
        "plan.work_items",
        Some("wi_consumer"),
    );

    let mut extra = work_items;
    extra.insert("wi_unknown".to_string(), extra["wi_provider"].clone());
    assert_compile_finding(
        compile_plan(&graph, &extra),
        "plan_projection_work_item_mismatch",
        "plan.work_items",
        Some("wi_unknown"),
    );

    let mut missing_binding = expected_plan_revision_ids();
    missing_binding.remove("wi_consumer");
    assert_compile_finding(
        compile_plan_with(&graph, &compiled_plan_fixture().1, &missing_binding),
        "plan_projection_revision_binding_keys_mismatch",
        "plan.revision_bindings",
        Some("wi_consumer"),
    );

    let mut extra_binding = expected_plan_revision_ids();
    extra_binding.insert("wi_unknown".to_string(), "revision_unknown".to_string());
    assert_compile_finding(
        compile_plan_with(&graph, &compiled_plan_fixture().1, &extra_binding),
        "plan_projection_revision_binding_keys_mismatch",
        "plan.revision_bindings",
        Some("wi_unknown"),
    );
}

#[test]
fn work_item_projection_plan_validation_detects_work_item_edge_and_matrix_mismatches() {
    let (graph, work_items) = compiled_plan_fixture();
    let baseline = compile_plan(&graph, &work_items).unwrap();

    let mut changed = baseline.clone();
    changed.human.work_items.pop();
    assert_has_code(
        validate_plan(&graph, &changed, &work_items),
        "plan_projection_work_item_mismatch",
    );

    changed = baseline.clone();
    changed.coder.dependency_edges.clear();
    assert_has_code(
        validate_plan(&graph, &changed, &work_items),
        "plan_projection_edge_mismatch",
    );

    changed = baseline.clone();
    changed.reviewer.work_items[0]
        .criterion_refs
        .push("AC-INVENTED".to_string());
    let first = validate_plan(&graph, &changed, &work_items);
    let second = validate_plan(&graph, &changed, &work_items);
    assert_has_code(first.clone(), "plan_projection_matrix_mismatch");
    assert_eq!(first, second);

    changed = baseline.clone();
    changed.reviewer.design_traceability_refs.clear();
    assert_has_code(
        validate_plan(&graph, &changed, &work_items),
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
        validate_plan(&graph, &changed, &work_items),
        "plan_projection_risk_mismatch",
    );
}

#[test]
fn work_item_projection_plan_validation_binds_external_plan_and_source_refs() {
    let (graph, work_items) = compiled_plan_fixture();
    let baseline = compile_plan(&graph, &work_items).unwrap();

    let mut wrong_plan = baseline.clone();
    wrong_plan.human.plan_id = "plan_wrong".to_string();
    wrong_plan.coder.plan_id = "plan_wrong".to_string();
    wrong_plan.reviewer.plan_id = "plan_wrong".to_string();
    let report = validate_plan(&graph, &wrong_plan, &work_items);
    for projection in ["plan.human", "plan.coder", "plan.reviewer"] {
        assert_finding(
            &report,
            "plan_projection_plan_binding_mismatch",
            projection,
            Some(PLAN_ID),
        );
    }

    let mut missing = baseline.clone();
    missing
        .human
        .source_refs
        .retain(|source_ref| source_ref != "story_0001");
    assert_finding(
        &validate_plan(&graph, &missing, &work_items),
        "plan_projection_source_ref_missing",
        "plan.human",
        Some("story_0001"),
    );

    let mut invented = baseline.clone();
    invented.human.source_refs.push("invented_0001".to_string());
    assert_finding(
        &validate_plan(&graph, &invented, &work_items),
        "plan_projection_source_ref_invented",
        "plan.human",
        Some("invented_0001"),
    );

    let mut duplicate = baseline;
    duplicate.human.source_refs.push("design_0001".to_string());
    assert_finding(
        &validate_plan(&graph, &duplicate, &work_items),
        "plan_projection_source_ref_duplicate",
        "plan.human",
        Some("design_0001"),
    );
}

#[test]
fn work_item_projection_plan_validation_rejects_all_role_wrong_revision_bindings() {
    let (graph, mut work_items) = compiled_plan_fixture();
    let baseline = compile_plan(&graph, &work_items).unwrap();
    let changed = work_items.get_mut("wi_consumer").unwrap();
    changed.coder.work_item_revision_id = "revision_wrong".to_string();
    changed.reviewer.work_item_revision_id = "revision_wrong".to_string();

    let report = validate_plan(&graph, &baseline, &work_items);
    for projection in [
        "plan.work_items.wi_consumer.coder",
        "plan.work_items.wi_consumer.reviewer",
    ] {
        assert_finding(
            &report,
            "projection_revision_binding_mismatch",
            projection,
            Some("revision_wi_consumer"),
        );
    }
}

#[test]
fn work_item_projection_plan_rejects_empty_expected_revision_bindings() {
    let (graph, baseline) = compiled_plan_fixture();

    for revision_id in ["", "  \t"] {
        let mut work_items = baseline.clone();
        let projection = work_items.get_mut("wi_consumer").unwrap();
        projection.coder.work_item_revision_id = revision_id.to_string();
        projection.reviewer.work_item_revision_id = revision_id.to_string();
        let mut expected_revision_ids = expected_plan_revision_ids();
        expected_revision_ids.insert("wi_consumer".to_string(), revision_id.to_string());

        let Err(ProjectionCompileError::Validation(report)) =
            compile_plan_with(&graph, &work_items, &expected_revision_ids)
        else {
            panic!("expected empty plan revision binding validation failure");
        };
        for projection in [
            "plan.work_items.wi_consumer.work_item.revision_binding",
            "plan.work_items.wi_consumer.coder",
            "plan.work_items.wi_consumer.reviewer",
        ] {
            assert_finding(
                &report,
                "projection_revision_binding_invalid",
                projection,
                None,
            );
        }
    }
}

#[test]
fn work_item_projection_plan_preserves_logical_context_for_identical_local_findings() {
    let (graph, mut work_items) = compiled_plan_fixture();
    let baseline = compile_plan(&graph, &work_items).unwrap();
    for logical_id in ["wi_consumer", "wi_provider"] {
        work_items
            .get_mut(logical_id)
            .unwrap()
            .coder
            .acceptance_criteria
            .clear();
    }

    let report = validate_plan(&graph, &baseline, &work_items);
    for logical_id in ["wi_consumer", "wi_provider"] {
        assert_finding(
            &report,
            "projection_missing_contract_ref",
            &format!("plan.work_items.{logical_id}.coder.acceptance_criteria"),
            Some("AC-001"),
        );
    }
    assert_eq!(
        report
            .findings
            .iter()
            .filter(|finding| {
                finding.code == "projection_missing_contract_ref"
                    && finding.contract_ref.as_deref() == Some("AC-001")
                    && finding.projection.ends_with(".coder.acceptance_criteria")
            })
            .count(),
        2
    );

    assert_compile_finding(
        compile_plan(&graph, &work_items),
        "projection_missing_contract_ref",
        "plan.work_items.wi_consumer.coder.acceptance_criteria",
        Some("AC-001"),
    );
}

#[test]
fn work_item_projection_plan_require_any_flow_uses_compatibility_semantics() {
    let (graph, work_items) = compiled_plan_fixture();

    let mut partial = graph.clone();
    partial.edges[0].required_contracts[0].compatibility_policy =
        ContractCompatibilityPolicy::RequireAny;
    partial.edges[0].required_contracts[0].required_capabilities =
        vec!["capability.a".to_string(), "capability.missing".to_string()];
    let compiled = compile_plan(&partial, &work_items).unwrap();
    assert!(
        compiled.human.contract_flow[0]
            .missing_capabilities
            .is_empty()
    );
    assert!(compiled.human.risks.is_empty());

    let mut none = graph.clone();
    none.edges[0].required_contracts[0].compatibility_policy =
        ContractCompatibilityPolicy::RequireAny;
    none.edges[0].required_contracts[0].required_capabilities =
        vec!["capability.x".to_string(), "capability.y".to_string()];
    let flow = super::super::contract_flow(&none);
    assert_eq!(
        flow[0].missing_capabilities,
        vec!["capability.x", "capability.y"]
    );
    let risks = super::super::risks_from_flow(&flow);
    assert_eq!(risks.len(), 1);
    assert_compile_finding(
        compile_plan(&none, &work_items),
        "required_capability_missing",
        "plan.dependency_graph",
        Some("contract.shared"),
    );

    let mut empty = graph;
    empty.edges[0].required_contracts[0].compatibility_policy =
        ContractCompatibilityPolicy::RequireAny;
    empty.edges[0].required_contracts[0]
        .required_capabilities
        .clear();
    let compiled = compile_plan(&empty, &work_items).unwrap();
    assert!(
        compiled.human.contract_flow[0]
            .missing_capabilities
            .is_empty()
    );
    assert!(compiled.human.risks.is_empty());
}

fn compile_plan(
    graph: &DependencyContractGraph,
    work_items: &BTreeMap<
        String,
        crate::product::work_item_projection::CompiledWorkItemProjections,
    >,
) -> Result<crate::product::work_item_projection::CompiledPlanProjections, ProjectionCompileError> {
    compile_plan_with(graph, work_items, &expected_plan_revision_ids())
}

fn compile_plan_with(
    graph: &DependencyContractGraph,
    work_items: &BTreeMap<
        String,
        crate::product::work_item_projection::CompiledWorkItemProjections,
    >,
    expected_revision_ids: &BTreeMap<String, String>,
) -> Result<crate::product::work_item_projection::CompiledPlanProjections, ProjectionCompileError> {
    PlanProjectionCompiler.compile(PlanProjectionCompileInput {
        plan_id: PLAN_ID,
        goal: "Compile the plan",
        split_reason: "Explicit dependency boundary",
        source_refs: &["design_0001".to_string(), "story_0001".to_string()],
        dependency_graph: graph,
        work_item_projections: work_items,
        expected_work_item_revision_ids: expected_revision_ids,
    })
}

fn validate_plan(
    graph: &DependencyContractGraph,
    compiled: &crate::product::work_item_projection::CompiledPlanProjections,
    work_items: &BTreeMap<
        String,
        crate::product::work_item_projection::CompiledWorkItemProjections,
    >,
) -> ProjectionValidationReport {
    validate_plan_projection_coverage(PlanProjectionValidationInput {
        expected_plan_id: PLAN_ID,
        expected_source_refs: &["design_0001".to_string(), "story_0001".to_string()],
        expected_work_item_revision_ids: &expected_plan_revision_ids(),
        dependency_graph: graph,
        compiled,
        work_item_projections: work_items,
    })
}

fn assert_compile_finding(
    result: Result<
        crate::product::work_item_projection::CompiledPlanProjections,
        ProjectionCompileError,
    >,
    code: &str,
    projection: &str,
    contract_ref: Option<&str>,
) {
    let Err(ProjectionCompileError::Validation(report)) = result else {
        panic!("expected projection validation error");
    };
    assert_finding(&report, code, projection, contract_ref);
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

fn assert_finding(
    report: &ProjectionValidationReport,
    code: &str,
    projection: &str,
    contract_ref: Option<&str>,
) {
    assert!(
        report.findings.iter().any(|finding| {
            finding.code == code
                && finding.projection == projection
                && finding.contract_ref.as_deref() == contract_ref
        }),
        "missing finding ({code}, {projection}, {contract_ref:?}): {:?}",
        report.findings
    );
}
