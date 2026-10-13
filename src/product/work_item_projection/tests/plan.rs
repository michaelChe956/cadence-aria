use std::collections::BTreeMap;

use super::{compiled_plan_fixture, expected_plan_revision_ids};
use crate::product::planning_failure::{
    PlanningFailure, PlanningFailureEvidence, PlanningFailureKind, PlanningFailureOrigin,
    PlanningFailureSet, PlanningRecoveryDisposition, PlanningStage, StructuralAuthorizationFacts,
    StructuralAuthorizationKind, classify_failure_set, is_complete_cycle_witness,
};
use crate::product::work_item_contract::{
    ContractCompatibilityPolicy, DependencyContractEdge, DependencyContractGraph,
    RequiredDependencyContract, validate_dependency_contract_graph,
};
use crate::product::work_item_projection::{
    PlanProjectionCompileInput, PlanProjectionCompiler, PlanProjectionValidationInput,
    ProjectionCompileError, ProjectionValidationFinding, ProjectionValidationReport,
    validate_plan_projection_coverage,
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

#[test]
fn sg_f2c_projection_origin_inherits_canonical_dependency_kind_and_ids() {
    let (graph, work_items) = compiled_plan_fixture();

    // 供需缺口：F2a canonical typed payload 原样继承进 projection 校验，
    // 四元 ID（provider/consumer/contract/capability）不因投影层丢失。
    let mut gap = graph.clone();
    gap.edges[0].required_contracts[0]
        .required_capabilities
        .push("capability.missing".to_string());
    let canonical_gap = validate_dependency_contract_graph(&gap)
        .findings
        .into_iter()
        .find(|finding| finding.code == "required_capability_missing")
        .expect("canonical capability gap must be reported");
    let Err(ProjectionCompileError::Validation(gap_report)) = compile_plan(&gap, &work_items)
    else {
        panic!("capability gap must reject plan projection compile");
    };
    let gap_finding = gap_report
        .findings
        .iter()
        .find(|finding| finding.code == "required_capability_missing")
        .expect("projection must re-report the canonical capability gap");
    assert_eq!(
        gap_finding.kind, canonical_gap.kind,
        "projection finding 继承 canonical typed kind，不从 message 拆码"
    );
    assert_eq!(
        gap_finding.kind,
        Some(PlanningFailureKind::RequiredCapabilityMissing {
            provider: "wi_provider".to_string(),
            consumer: "wi_consumer".to_string(),
            contract_id: "contract.shared".to_string(),
            capability: "capability.missing".to_string(),
        })
    );
    assert_kind_codes_match_findings(&gap_report);
    // 同 dependency 失败保 IDs/kind 即保 L1 资格：活动授权下整批
    // StructuralAuthor（解不唯一不影响资格，REQ-SG-01）。
    assert_eq!(
        classify_failure_set(
            &sg_f2c_failure_set_from_report(&gap_report),
            &sg_f2c_active_sc_facts()
        ),
        PlanningRecoveryDisposition::StructuralAuthor
    );

    // 依赖环：DependencyCycle witness（真实有向路径）随 kind 一并继承。
    let cyclic = cyclic_graph_fixture(&graph);
    let canonical_cycle = validate_dependency_contract_graph(&cyclic)
        .findings
        .into_iter()
        .find(|finding| finding.code == "dependency_cycle")
        .expect("canonical cycle must be reported");
    let Err(ProjectionCompileError::Validation(cycle_report)) = compile_plan(&cyclic, &work_items)
    else {
        panic!("cycle must reject plan projection compile");
    };
    let cycle_finding = cycle_report
        .findings
        .iter()
        .find(|finding| finding.code == "dependency_cycle")
        .expect("projection must re-report the canonical cycle");
    assert_eq!(
        cycle_finding.kind, canonical_cycle.kind,
        "cycle typed payload（含 witness）原样继承"
    );
    let Some(PlanningFailureKind::DependencyCycle { witness }) = cycle_finding.kind.as_ref() else {
        panic!("inherited cycle kind must carry the DependencyCycle witness");
    };
    // 保 IDs：witness 每跳端点都是本 fixture 的真实逻辑 WI ID。
    for hop in witness {
        assert!(
            ["wi_provider", "wi_consumer"].contains(&hop.from.as_str())
                && ["wi_provider", "wi_consumer"].contains(&hop.to.as_str()),
            "witness hop must preserve real logical WI IDs: {hop:?}"
        );
    }
    assert!(is_complete_cycle_witness(witness));
    assert_kind_codes_match_findings(&cycle_report);
    assert_eq!(
        classify_failure_set(
            &sg_f2c_failure_set_from_report(&cycle_report),
            &sg_f2c_active_sc_facts()
        ),
        PlanningRecoveryDisposition::StructuralAuthor
    );
}

#[test]
fn sg_f2c_projection_origin_internal_invariants_are_operational_without_author_drive() {
    let (graph, work_items) = compiled_plan_fixture();
    let baseline = compile_plan(&graph, &work_items).unwrap();

    // 逻辑/物理投影不一致（physical coder projection 丢边）：拒绝并 typed
    // 为 projection 内部 Operational——编译器对 canonical 输入的确定性投影
    // 漂移是系统故障，不驱 author（REQ-SG-01/03）。
    let mut changed = baseline.clone();
    changed.coder.dependency_edges.clear();
    let report = validate_plan(&graph, &changed, &work_items);
    let edge_finding = report
        .findings
        .iter()
        .find(|finding| finding.code == "plan_projection_edge_mismatch")
        .expect("edge mismatch must be reported");
    assert_eq!(
        edge_finding.kind.as_ref(),
        Some(&PlanningFailureKind::Operational {
            stable_code: "plan_projection_edge_mismatch".to_string(),
            cause: edge_finding.message.clone(),
        })
    );
    // 不从 Display 拆码：stable_code 恒为分支 code 常量；classification 只
    // 读 kind，cause 文本仅诊断。
    assert_eq!(
        edge_finding
            .kind
            .as_ref()
            .map(PlanningFailureKind::stable_code),
        Some("plan_projection_edge_mismatch")
    );
    assert_kind_codes_match_findings(&report);

    // 物理投影成员与逻辑图不同构：同样拒绝为 Operational。
    let mut popped = baseline;
    popped.human.work_items.pop();
    let mismatch_report = validate_plan(&graph, &popped, &work_items);
    let mismatch_finding = mismatch_report
        .findings
        .iter()
        .find(|finding| finding.code == "plan_projection_work_item_mismatch")
        .expect("work item mismatch must be reported");
    assert!(matches!(
        mismatch_finding.kind,
        Some(PlanningFailureKind::Operational { .. })
    ));
    assert_kind_codes_match_findings(&mismatch_report);

    // compile 拒绝逻辑/物理不一致（缺 work item projection），typed Operational。
    let mut missing = work_items.clone();
    missing.remove("wi_consumer");
    let Err(ProjectionCompileError::Validation(compile_report)) = compile_plan(&graph, &missing)
    else {
        panic!("missing projection must reject compile");
    };
    let compile_finding = compile_report
        .findings
        .iter()
        .find(|finding| finding.code == "plan_projection_work_item_mismatch")
        .expect("compile must report the projection mismatch");
    assert!(matches!(
        compile_finding.kind,
        Some(PlanningFailureKind::Operational { .. })
    ));
    assert_kind_codes_match_findings(&compile_report);

    // plan.rs Kahn 排序阻断：残余节点集合不是有向 witness（REQ-PLN-10），
    // 不编造 DependencyCycle payload；拒绝结果 typed Operational，消费面
    // 拿 typed report，不重 parse Display 文案。
    let cyclic = cyclic_graph_fixture(&graph);
    let Err(ProjectionCompileError::Validation(kahn_report)) =
        super::super::stable_topological_order(&cyclic, &work_items)
    else {
        panic!("kahn ordering must reject the cyclic graph");
    };
    assert_eq!(kahn_report.findings.len(), 1);
    assert_eq!(
        kahn_report.findings[0].code,
        "plan_projection_edge_mismatch"
    );
    assert!(matches!(
        kahn_report.findings[0].kind,
        Some(PlanningFailureKind::Operational { .. })
    ));

    // projection 内部错不驱 author：活动授权下整批 OperationalWait。
    assert_eq!(
        classify_failure_set(
            &sg_f2c_failure_set_from_report(&report),
            &sg_f2c_active_sc_facts()
        ),
        PlanningRecoveryDisposition::OperationalWait
    );

    // 混入继承的 L1 依赖失败也整批先停：不部分自动。
    let mut mixed = sg_f2c_failure_set_from_report(&report);
    mixed.failures.push(sg_f2c_failure_from_kind(
        PlanningFailureKind::RequiredCapabilityMissing {
            provider: "wi_provider".to_string(),
            consumer: "wi_consumer".to_string(),
            contract_id: "contract.shared".to_string(),
            capability: "capability.missing".to_string(),
        },
    ));
    assert_eq!(
        classify_failure_set(&mixed, &sg_f2c_active_sc_facts()),
        PlanningRecoveryDisposition::OperationalWait
    );
}

#[test]
fn sg_f2c_projection_origin_old_records_fold_unknown_with_byte_contract() {
    let (graph, work_items) = compiled_plan_fixture();
    let mut changed = compile_plan(&graph, &work_items).unwrap();
    changed.coder.dependency_edges.clear();
    let report = validate_plan(&graph, &changed, &work_items);
    let finding = report
        .findings
        .iter()
        .find(|finding| finding.code == "plan_projection_edge_mismatch")
        .expect("edge mismatch must be reported")
        .clone();

    // 旧 JSON（无 kind 字段）：解析为 None，再序列化不回添字段（字节合同，
    // plan §0.5——旧 PlanValidationReportArtifact 记录不失效）。
    let mut legacy = serde_json::to_value(&finding).unwrap();
    let legacy_object = legacy.as_object_mut().unwrap();
    assert!(legacy_object.remove("kind").is_some());
    let legacy_finding: ProjectionValidationFinding =
        serde_json::from_value(legacy.clone()).unwrap();
    assert_eq!(legacy_finding.kind, None);
    assert_eq!(
        serde_json::to_value(&legacy_finding).unwrap(),
        legacy,
        "None 再序列化不得回添 kind 字段"
    );

    // Unknown 旧 record 只折叠：按 Unknown 折影，活动授权下整批
    // UnknownStop，不补 L1 资格、不把系统故障伪装成 author 结构修复。
    let folded = PlanningFailureSet {
        failures: vec![sg_f2c_failure_from_kind(PlanningFailureKind::Unknown {
            code: legacy_finding.code.clone(),
        })],
    };
    assert_eq!(
        classify_failure_set(&folded, &sg_f2c_active_sc_facts()),
        PlanningRecoveryDisposition::UnknownStop
    );

    // typed 新记录：同 fixture 系统故障 → OperationalWait，沿原安全等待。
    assert_eq!(
        classify_failure_set(
            &sg_f2c_failure_set_from_report(&report),
            &sg_f2c_active_sc_facts()
        ),
        PlanningRecoveryDisposition::OperationalWait
    );
}

/// F2c 测试共享：SC 修订 turn 活动授权事实（attempt 1/2）。
fn sg_f2c_active_sc_facts() -> StructuralAuthorizationFacts {
    StructuralAuthorizationFacts {
        authorization_id: "grant-sc-f2c".to_string(),
        authorization_kind: StructuralAuthorizationKind::ScTurn,
        parent_id: "turn-f2c".to_string(),
        source_hash: "f2c-source-hash-0000000000000000000000000000000000000000".to_string(),
        context_digest: "ctx-f2c".to_string(),
        active: true,
        anchor_waiting: false,
        launch_unknown: false,
        attempt_no: 1,
        max_attempts: 2,
    }
}

/// 把 projection 校验 report 包装成失败集合（F2c 消费面）：kind 缺失只可能
/// 来自旧 JSON 记录，按 Unknown 折影；typed kind 原样进入分类。
fn sg_f2c_failure_set_from_report(report: &ProjectionValidationReport) -> PlanningFailureSet {
    PlanningFailureSet {
        failures: report
            .findings
            .iter()
            .map(|finding| PlanningFailure {
                kind: finding
                    .kind
                    .clone()
                    .unwrap_or_else(|| PlanningFailureKind::Unknown {
                        code: finding.code.clone(),
                    }),
                origin: PlanningFailureOrigin::Projection,
                stage: PlanningStage::Plan,
                diagnostic_ref: format!("diag-f2c-{}", finding.code),
                source_ref: "plan-source/f2c".to_string(),
                source_hash: "f2c-source-hash-0000000000000000000000000000000000000000".to_string(),
                evidence: PlanningFailureEvidence {
                    affected_ids: finding
                        .contract_ref
                        .clone()
                        .into_iter()
                        .chain([finding.projection.clone()])
                        .collect(),
                    details: BTreeMap::new(),
                },
            })
            .collect(),
    }
}

fn sg_f2c_failure_from_kind(kind: PlanningFailureKind) -> PlanningFailure {
    PlanningFailure {
        kind,
        origin: PlanningFailureOrigin::Projection,
        stage: PlanningStage::Plan,
        diagnostic_ref: "diag-f2c-mixed".to_string(),
        source_ref: "plan-source/f2c".to_string(),
        source_hash: "f2c-source-hash-0000000000000000000000000000000000000000".to_string(),
        evidence: PlanningFailureEvidence::default(),
    }
}

/// F2c 无遗漏扫描：report 中每条 Some(kind) 的稳定 code 与 finding code
/// 单源一致（不从 Display/文案拆码）。
fn assert_kind_codes_match_findings(report: &ProjectionValidationReport) {
    for finding in &report.findings {
        if let Some(kind) = finding.kind.as_ref() {
            assert_eq!(
                kind.stable_code(),
                finding.code,
                "kind stable code must stay single-sourced from the finding code"
            );
        }
    }
}

/// 两节点供需环：wi_provider→wi_consumer（canonical 输入）＋反向声明边。
fn cyclic_graph_fixture(graph: &DependencyContractGraph) -> DependencyContractGraph {
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
    cyclic
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
        source_refs: vec!["design_0001".to_string(), "story_0001".to_string()],
        dependency_graph: graph,
        work_item_projections: work_items,
        expected_work_item_revision_ids: expected_revision_ids.clone(),
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
