use serde_json::json;

use crate::product::work_item_contract::{
    BlockerRoute, CanonicalWorkItemContract, ContractCompatibilityPolicy, DependencyContractEdge,
    DependencyContractGraph, PromisedOutputContract, RequiredDependencyContract,
    RequiredInputContract, canonical_contract_fixture,
};

use super::{
    ContractCapabilityAssociation, ContractDelta, ContractDeltaKind, ContractImpactAnalyzer,
    NormalizedPlanDefectRoute, PlanDefectClass, PlanDefectConfidence, PlanDefectEvidence,
    PlanDefectFinding, PlanDefectRoute, PlanDefectSeverity, PlanExecutionState, PlanRepairError,
    PlanRepairRequest, RepairTarget, RepairTargetKind, UnitExecutionSnapshot,
    compute_contract_delta, default_route, normalize_blocker_route, plan_defect_fingerprint,
};

mod amendment;
mod review;
mod review_fix;
mod subgraph;

fn repair_request_json() -> serde_json::Value {
    json!({
        "id": "plan_repair_request_0001",
        "plan_id": "work_item_plan_0001",
        "base_plan_revision_id": "plan_revision_0001",
        "trigger_attempt_id": "coding_attempt_0001",
        "trigger_unit_run_id": "unit_run_0001",
        "trigger_review_id": "review_0001",
        "trigger_finding_id": "finding_0001",
        "amendment_id": null,
        "defect_class": "current_work_item_invalid",
        "reason_code": "contract_invalid",
        "repair_target": {
            "kind": "current_work_item",
            "logical_work_item_ids": ["logical_work_item_0001"],
            "work_item_revision_ids": ["work_item_revision_0001"]
        },
        "contract_refs": ["contract_0001"],
        "capability_refs": ["capability_0001"],
        "evidence": [{
            "kind": "review_finding",
            "source_ref": "review_0001#finding_0001",
            "message": "contract mismatch"
        }],
        "fingerprint": "fingerprint_0001",
        "status": "open",
        "created_at": "2026-07-17T00:00:04Z",
        "updated_at": "2026-07-17T00:00:04Z"
    })
}

#[test]
fn plan_repair_request_serde_is_closed_and_evidence_is_typed() {
    let mut unknown_request_field = repair_request_json();
    unknown_request_field["unexpected"] = json!(true);
    assert!(serde_json::from_value::<PlanRepairRequest>(unknown_request_field).is_err());

    let mut unknown_evidence_field = repair_request_json();
    unknown_evidence_field["evidence"][0]["unexpected"] = json!(true);
    assert!(serde_json::from_value::<PlanRepairRequest>(unknown_evidence_field).is_err());

    let mut weak_evidence = repair_request_json();
    weak_evidence["evidence"] = json!([{"kind": "review", "id": "finding_0001"}]);
    assert!(serde_json::from_value::<PlanRepairRequest>(weak_evidence).is_err());
}

fn target_fixture(
    kind: RepairTargetKind,
    logical_work_item_ids: &[&str],
    work_item_revision_ids: &[&str],
) -> RepairTarget {
    RepairTarget {
        kind,
        logical_work_item_ids: logical_work_item_ids
            .iter()
            .map(|value| (*value).to_string())
            .collect(),
        work_item_revision_ids: work_item_revision_ids
            .iter()
            .map(|value| (*value).to_string())
            .collect(),
    }
}

fn finding_fixture(
    defect_class: PlanDefectClass,
    repair_target: Option<RepairTarget>,
    contract_refs: &[&str],
    capability_refs: &[&str],
) -> PlanDefectFinding {
    let recommended_route = default_route(&defect_class);
    PlanDefectFinding {
        finding_id: "finding_0001".to_string(),
        severity: PlanDefectSeverity::Error,
        defect_class,
        reason_code: "contract_invalid".to_string(),
        message: "contract mismatch".to_string(),
        evidence: vec![PlanDefectEvidence {
            kind: "review_finding".to_string(),
            source_ref: "review_0001#finding_0001".to_string(),
            message: "contract mismatch".to_string(),
        }],
        contract_refs: contract_refs
            .iter()
            .map(|value| (*value).to_string())
            .collect(),
        capability_refs: capability_refs
            .iter()
            .map(|value| (*value).to_string())
            .collect(),
        repair_target,
        recommended_route,
        confidence: PlanDefectConfidence::High,
    }
}

#[test]
fn plan_repair_default_route_covers_every_defect_class() {
    let cases = [
        (
            PlanDefectClass::ImplementationDefect,
            PlanDefectRoute::CoderRework,
        ),
        (
            PlanDefectClass::VerificationIncomplete,
            PlanDefectRoute::VerificationRetry,
        ),
        (
            PlanDefectClass::CurrentWorkItemInvalid,
            PlanDefectRoute::PlanRepair,
        ),
        (
            PlanDefectClass::UpstreamContractInvalid,
            PlanDefectRoute::PlanRepair,
        ),
        (
            PlanDefectClass::DependencyGraphInvalid,
            PlanDefectRoute::PlanRepair,
        ),
        (
            PlanDefectClass::DesignAmendmentRequired,
            PlanDefectRoute::DesignAmendment,
        ),
        (
            PlanDefectClass::StoryAmendmentRequired,
            PlanDefectRoute::StoryAmendment,
        ),
        (
            PlanDefectClass::OperationalBlocker,
            PlanDefectRoute::OperationalGate,
        ),
    ];

    for (class, expected) in cases {
        assert_eq!(default_route(&class), expected);
    }
}

#[test]
fn plan_repair_normalizes_every_blocker_route_without_losing_target_kind() {
    let cases = [
        (
            BlockerRoute::CoderRework,
            PlanDefectRoute::CoderRework,
            None,
        ),
        (
            BlockerRoute::VerificationRetry,
            PlanDefectRoute::VerificationRetry,
            None,
        ),
        (
            BlockerRoute::PlanRepairCurrent,
            PlanDefectRoute::PlanRepair,
            Some(RepairTargetKind::CurrentWorkItem),
        ),
        (
            BlockerRoute::PlanRepairUpstream,
            PlanDefectRoute::PlanRepair,
            Some(RepairTargetKind::UpstreamWorkItem),
        ),
        (
            BlockerRoute::SubgraphReplan,
            PlanDefectRoute::PlanRepair,
            Some(RepairTargetKind::Subgraph),
        ),
        (
            BlockerRoute::StoryAmendment,
            PlanDefectRoute::StoryAmendment,
            None,
        ),
        (
            BlockerRoute::DesignAmendment,
            PlanDefectRoute::DesignAmendment,
            None,
        ),
        (
            BlockerRoute::OperationalGate,
            PlanDefectRoute::OperationalGate,
            None,
        ),
    ];

    for (route, expected_route, required_target_kind) in cases {
        assert_eq!(
            normalize_blocker_route(route),
            NormalizedPlanDefectRoute {
                route: expected_route,
                required_target_kind,
            }
        );
    }
}

#[test]
fn plan_repair_finding_validation_requires_consistent_route_class_and_target() {
    let valid_targets = [
        (
            PlanDefectClass::CurrentWorkItemInvalid,
            RepairTargetKind::CurrentWorkItem,
        ),
        (
            PlanDefectClass::UpstreamContractInvalid,
            RepairTargetKind::UpstreamWorkItem,
        ),
        (
            PlanDefectClass::DependencyGraphInvalid,
            RepairTargetKind::Subgraph,
        ),
    ];
    for (class, kind) in valid_targets {
        let finding = finding_fixture(
            class,
            Some(target_fixture(
                kind,
                &["logical_work_item_0001"],
                &["work_item_revision_0001"],
            )),
            &[],
            &[],
        );
        assert!(finding.validate().is_ok());
    }

    let implementation = finding_fixture(PlanDefectClass::ImplementationDefect, None, &[], &[]);
    assert!(implementation.validate().is_ok());

    let mut wrong_route = implementation.clone();
    wrong_route.recommended_route = PlanDefectRoute::PlanRepair;
    assert!(matches!(
        wrong_route.validate(),
        Err(PlanRepairError::InvalidFinding(_))
    ));

    let wrong_target = finding_fixture(
        PlanDefectClass::UpstreamContractInvalid,
        Some(target_fixture(
            RepairTargetKind::CurrentWorkItem,
            &["logical_work_item_0001"],
            &["work_item_revision_0001"],
        )),
        &[],
        &[],
    );
    assert!(matches!(
        wrong_target.validate(),
        Err(PlanRepairError::InvalidRepairTarget(_))
    ));

    let unexpected_target = finding_fixture(
        PlanDefectClass::ImplementationDefect,
        Some(target_fixture(
            RepairTargetKind::CurrentWorkItem,
            &["logical_work_item_0001"],
            &["work_item_revision_0001"],
        )),
        &[],
        &[],
    );
    assert!(matches!(
        unexpected_target.validate(),
        Err(PlanRepairError::InvalidRepairTarget(_))
    ));
}

#[test]
fn plan_repair_finding_serde_rejects_unknown_fields_and_routes() {
    let finding = finding_fixture(
        PlanDefectClass::CurrentWorkItemInvalid,
        Some(target_fixture(
            RepairTargetKind::CurrentWorkItem,
            &["logical_work_item_0001"],
            &["work_item_revision_0001"],
        )),
        &[],
        &[],
    );
    let mut unknown_field = serde_json::to_value(&finding).unwrap();
    unknown_field["unexpected"] = json!(true);
    assert!(serde_json::from_value::<PlanDefectFinding>(unknown_field).is_err());

    let mut unknown_route = serde_json::to_value(&finding).unwrap();
    unknown_route["recommended_route"] = json!("unknown_route");
    assert!(serde_json::from_value::<PlanDefectFinding>(unknown_route).is_err());

    let mut unknown_target_field = serde_json::to_value(&finding.repair_target).unwrap();
    unknown_target_field["unexpected"] = json!(true);
    assert!(serde_json::from_value::<RepairTarget>(unknown_target_field).is_err());
}

#[test]
fn plan_repair_fingerprint_is_stable_for_reordered_refs_and_target_ids() {
    let left = finding_fixture(
        PlanDefectClass::UpstreamContractInvalid,
        Some(target_fixture(
            RepairTargetKind::UpstreamWorkItem,
            &["logical_b", "logical_a"],
            &["revision_b", "revision_a"],
        )),
        &["contract_b", "contract_a"],
        &["capability_b", "capability_a"],
    );
    let right = finding_fixture(
        PlanDefectClass::UpstreamContractInvalid,
        Some(target_fixture(
            RepairTargetKind::UpstreamWorkItem,
            &["logical_a", "logical_b"],
            &["revision_a", "revision_b"],
        )),
        &["contract_a", "contract_b"],
        &["capability_a", "capability_b"],
    );

    let left_fingerprint = plan_defect_fingerprint("plan_revision_0001", &left);
    assert_eq!(
        left_fingerprint,
        plan_defect_fingerprint("plan_revision_0001", &right)
    );
    assert_eq!(left_fingerprint.len(), 64);
    assert!(
        left_fingerprint
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    );
}

#[test]
fn plan_repair_fingerprint_treats_duplicate_refs_as_set_members() {
    let duplicated = finding_fixture(
        PlanDefectClass::DependencyGraphInvalid,
        Some(target_fixture(
            RepairTargetKind::Subgraph,
            &["logical_a", "logical_a"],
            &["revision_a", "revision_a"],
        )),
        &["contract_a", "contract_a"],
        &["capability_a", "capability_a"],
    );
    let unique = finding_fixture(
        PlanDefectClass::DependencyGraphInvalid,
        Some(target_fixture(
            RepairTargetKind::Subgraph,
            &["logical_a"],
            &["revision_a"],
        )),
        &["contract_a"],
        &["capability_a"],
    );

    assert_eq!(
        plan_defect_fingerprint("plan_revision_0001", &duplicated),
        plan_defect_fingerprint("plan_revision_0001", &unique)
    );
}

#[test]
fn plan_repair_fingerprint_normalizes_reason_code() {
    let canonical = finding_fixture(
        PlanDefectClass::DependencyGraphInvalid,
        Some(target_fixture(
            RepairTargetKind::Subgraph,
            &["logical_a"],
            &["revision_a"],
        )),
        &[],
        &[],
    );
    let mut differently_formatted = canonical.clone();
    differently_formatted.reason_code = "  CONTRACT_INVALID  ".to_string();

    assert_eq!(
        plan_defect_fingerprint("plan_revision_0001", &canonical),
        plan_defect_fingerprint("plan_revision_0001", &differently_formatted)
    );
}

#[test]
fn plan_repair_fingerprint_changes_for_different_base_or_target_semantics() {
    let current = finding_fixture(
        PlanDefectClass::CurrentWorkItemInvalid,
        Some(target_fixture(
            RepairTargetKind::CurrentWorkItem,
            &["logical_a"],
            &["revision_a"],
        )),
        &["contract_a"],
        &["capability_a"],
    );
    let changed_target = finding_fixture(
        PlanDefectClass::CurrentWorkItemInvalid,
        Some(target_fixture(
            RepairTargetKind::CurrentWorkItem,
            &["logical_b"],
            &["revision_b"],
        )),
        &["contract_a"],
        &["capability_a"],
    );

    assert_ne!(
        plan_defect_fingerprint("plan_revision_0001", &current),
        plan_defect_fingerprint("plan_revision_0002", &current)
    );
    assert_ne!(
        plan_defect_fingerprint("plan_revision_0001", &current),
        plan_defect_fingerprint("plan_revision_0001", &changed_target)
    );
}

#[test]
fn plan_repair_fingerprint_ignores_non_identity_evidence() {
    let mut left = finding_fixture(
        PlanDefectClass::CurrentWorkItemInvalid,
        Some(target_fixture(
            RepairTargetKind::CurrentWorkItem,
            &["logical_a"],
            &["revision_a"],
        )),
        &["contract_a"],
        &["capability_a"],
    );
    let mut right = left.clone();
    right.message = "different presentation".to_string();
    right.evidence.push(PlanDefectEvidence {
        kind: "test_failure".to_string(),
        source_ref: "test_0001".to_string(),
        message: "failed".to_string(),
    });
    right.confidence = PlanDefectConfidence::Medium;
    left.severity = PlanDefectSeverity::Warning;

    assert_eq!(
        plan_defect_fingerprint("plan_revision_0001", &left),
        plan_defect_fingerprint("plan_revision_0001", &right)
    );
}

fn provider_contract_fixture(capabilities: &[&str]) -> CanonicalWorkItemContract {
    let mut provider = canonical_contract_fixture("WI-01");
    provider.input_contracts.clear();
    provider.output_contracts = vec![PromisedOutputContract {
        contract_id: "contract_x".to_string(),
        capabilities: capabilities
            .iter()
            .map(|capability| (*capability).to_string())
            .collect(),
    }];
    provider.handoff_contract.provided_contract_refs = vec!["contract_x".to_string()];
    provider
}

fn dependency_contract_fixture(logical_work_item_id: &str) -> CanonicalWorkItemContract {
    let mut contract = canonical_contract_fixture(logical_work_item_id);
    contract.input_contracts.clear();
    contract.output_contracts.clear();
    contract.handoff_contract.provided_contract_refs.clear();
    contract
}

fn required_edge(
    from: &str,
    to: &str,
    contract_id: &str,
    capabilities: &[&str],
) -> DependencyContractEdge {
    DependencyContractEdge {
        from: from.to_string(),
        to: to.to_string(),
        required_contracts: vec![RequiredDependencyContract {
            contract_id: contract_id.to_string(),
            required_capabilities: capabilities
                .iter()
                .map(|capability| (*capability).to_string())
                .collect(),
            compatibility_policy: ContractCompatibilityPolicy::RequireAll,
        }],
    }
}

fn dependency_graph_fixture() -> DependencyContractGraph {
    let mut contracts: std::collections::BTreeMap<_, _> =
        ["WI-01", "WI-02", "WI-03", "WI-04", "WI-05"]
            .into_iter()
            .map(|id| (id.to_string(), dependency_contract_fixture(id)))
            .collect();
    contracts.insert(
        "WI-01".to_string(),
        provider_contract_fixture(&[
            "workflow_explicit_completion",
            "finalization_failure",
            "failure_message",
        ]),
    );
    DependencyContractGraph {
        contracts,
        edges: vec![
            required_edge(
                "WI-01",
                "WI-02",
                "contract_x",
                &[
                    "workflow_explicit_completion",
                    "finalization_failure",
                    "failure_message",
                ],
            ),
            required_edge(
                "WI-01",
                "WI-05",
                "contract_x",
                &["workflow_explicit_completion"],
            ),
            required_edge("WI-02", "WI-03", "contract_y", &["registration_ready"]),
        ],
    }
}

fn execution_state_fixture(started_consumers: &[&str]) -> PlanExecutionState {
    PlanExecutionState {
        units: started_consumers
            .iter()
            .map(|logical_work_item_id| {
                (
                    (*logical_work_item_id).to_string(),
                    UnitExecutionSnapshot {
                        logical_work_item_id: (*logical_work_item_id).to_string(),
                        work_item_revision_id: format!("revision_{logical_work_item_id}"),
                        completed_handoff_revision_id: None,
                        has_started: true,
                        has_completed: false,
                    },
                )
            })
            .collect(),
    }
}

fn delta_fixture(kind: ContractDeltaKind) -> ContractDelta {
    ContractDelta {
        logical_work_item_id: "WI-01".to_string(),
        previous_revision_id: "work_item_revision_0001".to_string(),
        next_revision_id: "work_item_revision_0002".to_string(),
        kind,
        added_contracts: Vec::new(),
        removed_contracts: Vec::new(),
        added_capabilities: Vec::new(),
        removed_capabilities: Vec::new(),
        changed_capabilities: Vec::new(),
        added_capability_associations: Vec::new(),
        removed_capability_associations: Vec::new(),
        acceptance_changed: false,
        verification_changed: false,
        write_policy_changed: false,
    }
}

#[test]
fn contract_delta_classifies_added_finalization_capabilities_as_compatible_extension() {
    let previous = provider_contract_fixture(&["workflow_explicit_completion"]);
    let next = provider_contract_fixture(&[
        "finalization_failure",
        "workflow_explicit_completion",
        "failure_message",
        "failure_message",
    ]);

    let delta = compute_contract_delta(
        "work_item_revision_0001",
        &previous,
        "work_item_revision_0002",
        &next,
    );

    assert_eq!(delta.kind, ContractDeltaKind::CompatibleContractExtension);
    assert_eq!(delta.logical_work_item_id, "WI-01");
    assert_eq!(
        delta.added_capabilities,
        vec!["failure_message", "finalization_failure"]
    );
    assert!(delta.changed_capabilities.is_empty());
}

#[test]
fn contract_delta_classifies_removed_and_moved_capabilities_as_breaking() {
    let mut previous = provider_contract_fixture(&["stable", "moved", "removed"]);
    previous.output_contracts.push(PromisedOutputContract {
        contract_id: "contract_y".to_string(),
        capabilities: Vec::new(),
    });
    let mut next = provider_contract_fixture(&["stable"]);
    next.output_contracts.push(PromisedOutputContract {
        contract_id: "contract_y".to_string(),
        capabilities: vec!["moved".to_string()],
    });

    let delta = compute_contract_delta("revision_1", &previous, "revision_2", &next);

    assert_eq!(delta.kind, ContractDeltaKind::BreakingContractChange);
    assert_eq!(delta.removed_capabilities, vec!["removed"]);
    assert_eq!(delta.changed_capabilities, vec!["moved"]);
}

#[test]
fn contract_delta_normalizes_inputs_before_detecting_topology_change() {
    let mut previous = provider_contract_fixture(&["stable"]);
    previous.input_contracts = vec![RequiredInputContract {
        contract_id: "contract_input".to_string(),
        provider_logical_work_item_id: "WI-00".to_string(),
        required_capabilities: vec!["b".to_string(), "a".to_string(), "a".to_string()],
        compatibility_policy: ContractCompatibilityPolicy::RequireAll,
    }];
    let mut reordered = previous.clone();
    reordered.input_contracts[0].required_capabilities = vec!["a".to_string(), "b".to_string()];

    assert_eq!(
        compute_contract_delta("revision_1", &previous, "revision_2", &reordered).kind,
        ContractDeltaKind::InformativeOnly
    );

    reordered.input_contracts[0].compatibility_policy = ContractCompatibilityPolicy::RequireAny;
    assert_eq!(
        compute_contract_delta("revision_1", &previous, "revision_2", &reordered).kind,
        ContractDeltaKind::TopologyChange
    );
}

#[test]
fn contract_impact_marks_only_started_consumers_of_removed_capability_stale() {
    let mut graph = dependency_graph_fixture();
    graph.contracts.get_mut("WI-01").unwrap().output_contracts[0]
        .capabilities
        .retain(|capability| capability != "finalization_failure");
    let mut delta = delta_fixture(ContractDeltaKind::BreakingContractChange);
    delta.removed_capabilities = vec!["finalization_failure".to_string()];
    delta.removed_capability_associations = vec![ContractCapabilityAssociation {
        contract_id: "contract_x".to_string(),
        capability: "finalization_failure".to_string(),
    }];

    let report = ContractImpactAnalyzer
        .analyze_static(&graph, &delta, &execution_state_fixture(&["WI-02"]))
        .unwrap();

    assert_eq!(report.direct_stale, vec!["WI-02"]);
    assert!(report.direct_revalidation.is_empty());
    assert_eq!(report.conditional_downstream, vec!["WI-03"]);
    assert_eq!(report.unaffected, vec!["WI-04", "WI-05"]);
    assert_eq!(report.explanation_paths.len(), 2);
    assert_eq!(report.explanation_paths[0].from, "WI-01");
    assert_eq!(report.explanation_paths[0].to, "WI-02");
    assert_eq!(report.explanation_paths[0].contract_id, "contract_x");
    assert_eq!(
        report.explanation_paths[0].capability_refs,
        vec![
            "failure_message",
            "finalization_failure",
            "workflow_explicit_completion"
        ]
    );
    assert_eq!(report.explanation_paths[1].from, "WI-02");
    assert_eq!(report.explanation_paths[1].to, "WI-03");
    assert_eq!(report.explanation_paths[1].contract_id, "contract_y");
    assert_eq!(
        report.explanation_paths[1].capability_refs,
        vec!["registration_ready"]
    );
}

#[test]
fn contract_impact_revalidates_unstarted_breaking_consumers() {
    let mut graph = dependency_graph_fixture();
    graph
        .contracts
        .get_mut("WI-01")
        .unwrap()
        .output_contracts
        .clear();
    let mut delta = delta_fixture(ContractDeltaKind::BreakingContractChange);
    delta.removed_contracts = vec!["contract_x".to_string()];

    let report = ContractImpactAnalyzer
        .analyze_static(&graph, &delta, &execution_state_fixture(&[]))
        .unwrap();

    assert!(report.direct_stale.is_empty());
    assert_eq!(report.direct_revalidation, vec!["WI-02", "WI-05"]);
    assert_eq!(report.conditional_downstream, vec!["WI-03"]);
    assert_eq!(report.unaffected, vec!["WI-04"]);
}

#[test]
fn contract_impact_revalidates_only_consumers_requiring_added_capabilities() {
    let graph = dependency_graph_fixture();
    let mut delta = delta_fixture(ContractDeltaKind::CompatibleContractExtension);
    delta.added_capabilities = vec![
        "failure_message".to_string(),
        "finalization_failure".to_string(),
    ];
    delta.added_capability_associations = vec![
        ContractCapabilityAssociation {
            contract_id: "contract_x".to_string(),
            capability: "failure_message".to_string(),
        },
        ContractCapabilityAssociation {
            contract_id: "contract_x".to_string(),
            capability: "finalization_failure".to_string(),
        },
    ];

    let report = ContractImpactAnalyzer
        .analyze_static(
            &graph,
            &delta,
            &execution_state_fixture(&["WI-02", "WI-05"]),
        )
        .unwrap();

    assert_eq!(report.direct_revalidation, vec!["WI-02"]);
    assert!(report.direct_stale.is_empty());
    assert_eq!(report.conditional_downstream, vec!["WI-03"]);
    assert_eq!(report.unaffected, vec!["WI-04", "WI-05"]);
}

#[test]
fn contract_impact_guidance_and_informative_deltas_leave_consumers_unaffected() {
    let graph = dependency_graph_fixture();
    for kind in [
        ContractDeltaKind::ImplementationGuidance,
        ContractDeltaKind::InformativeOnly,
    ] {
        let report = ContractImpactAnalyzer
            .analyze_static(
                &graph,
                &delta_fixture(kind),
                &execution_state_fixture(&["WI-02"]),
            )
            .unwrap();

        assert_eq!(report.unaffected, vec!["WI-02", "WI-03", "WI-04", "WI-05"]);
        assert!(report.direct_revalidation.is_empty());
        assert!(report.direct_stale.is_empty());
        assert!(report.conditional_downstream.is_empty());
        assert!(report.explanation_paths.is_empty());
    }
}

#[test]
fn contract_impact_topology_change_defers_all_impact_to_subgraph_replanner() {
    let report = ContractImpactAnalyzer
        .analyze_static(
            &dependency_graph_fixture(),
            &delta_fixture(ContractDeltaKind::TopologyChange),
            &execution_state_fixture(&["WI-02"]),
        )
        .unwrap();

    assert!(report.unaffected.is_empty());
    assert!(report.direct_revalidation.is_empty());
    assert!(report.direct_stale.is_empty());
    assert!(report.conditional_downstream.is_empty());
    assert!(report.explanation_paths.is_empty());
}

#[test]
fn contract_impact_rejects_delta_source_missing_from_graph() {
    let mut delta = delta_fixture(ContractDeltaKind::BreakingContractChange);
    delta.logical_work_item_id = "WI-missing".to_string();

    let error = ContractImpactAnalyzer
        .analyze_static(
            &dependency_graph_fixture(),
            &delta,
            &execution_state_fixture(&[]),
        )
        .unwrap_err();

    assert!(matches!(error, PlanRepairError::InvalidFinding(_)));
}
