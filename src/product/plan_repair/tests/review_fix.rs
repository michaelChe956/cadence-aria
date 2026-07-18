use super::amendment::{
    assert_no_coding_binding, confirmation, persist_review_attestation, plan_repair_engine_fixture,
};

use crate::product::models::AmendmentResumeMode;
use crate::product::plan_repair::{
    PlanExecutionState, UnitExecutionSnapshot, compute_contract_delta,
};
use crate::product::work_item_contract::{
    ContractCompatibilityPolicy, DependencyContractEdge, DependencyContractGraph,
    PromisedOutputContract, RequiredDependencyContract, RequiredInputContract,
    canonical_contract_fixture,
};

#[test]
fn plan_repair_candidate_package_fingerprint_is_stable_for_reordered_sets() {
    let fixture = plan_repair_engine_fixture();
    let prepared = fixture.engine.prepare_amendment(&fixture.request).unwrap();
    let mut reordered_manifest = prepared.manifest.clone();
    reordered_manifest.superseded_revisions.reverse();
    reordered_manifest.unaffected_units.reverse();
    reordered_manifest.revalidation_required_units.reverse();
    reordered_manifest.stale_units.reverse();
    reordered_manifest.contract_deltas.reverse();
    for delta in &mut reordered_manifest.contract_deltas {
        delta.added_contracts.reverse();
        delta.removed_contracts.reverse();
        delta.added_capabilities.reverse();
        delta.removed_capabilities.reverse();
        delta.changed_capabilities.reverse();
        delta.added_capability_associations.reverse();
        delta.removed_capability_associations.reverse();
    }
    let mut reordered_bundles = prepared.work_item_projection_bundles.clone();
    reordered_bundles.reverse();
    let mut reordered_impact = prepared.impact_report.clone();
    reordered_impact.unaffected.reverse();
    reordered_impact.direct_revalidation.reverse();
    reordered_impact.direct_stale.reverse();
    reordered_impact.conditional_downstream.reverse();
    reordered_impact.explanation_paths.reverse();

    let original = super::super::candidate_package_fingerprint(
        &fixture.request,
        &prepared.manifest,
        &prepared.plan_projection_bundle,
        &prepared.work_item_projection_bundles,
        &prepared.validation_report,
        &prepared.impact_report,
    )
    .unwrap();
    let reordered = super::super::candidate_package_fingerprint(
        &fixture.request,
        &reordered_manifest,
        &prepared.plan_projection_bundle,
        &reordered_bundles,
        &prepared.validation_report,
        &reordered_impact,
    )
    .unwrap();

    assert_eq!(original, reordered);
}

#[test]
fn plan_repair_candidate_package_fingerprint_changes_for_manifest_provenance() {
    let fixture = plan_repair_engine_fixture();
    let prepared = fixture.engine.prepare_amendment(&fixture.request).unwrap();
    let fingerprint = |manifest: &crate::product::models::PlanAmendmentManifest| {
        super::super::candidate_package_fingerprint(
            &fixture.request,
            manifest,
            &prepared.plan_projection_bundle,
            &prepared.work_item_projection_bundles,
            &prepared.validation_report,
            &prepared.impact_report,
        )
        .unwrap()
    };
    let original = fingerprint(&prepared.manifest);

    let mut replacement = prepared.manifest.clone();
    replacement
        .replacement_units
        .insert("wi_registration".to_string(), vec!["wi_ops".to_string()]);
    assert_ne!(fingerprint(&replacement), original);

    let mut resume = prepared.manifest.clone();
    resume.resume_target.mode = AmendmentResumeMode::Reexecute;
    assert_ne!(fingerprint(&resume), original);

    let mut delta = prepared.manifest.clone();
    delta.contract_deltas[0]
        .added_capabilities
        .push("tampered_capability".to_string());
    assert_ne!(fingerprint(&delta), original);
}

#[test]
fn contract_impact_multi_delta_stale_takes_precedence_over_revalidation() {
    let mut provider_a_before = canonical_contract_fixture("wi_provider_a");
    provider_a_before.input_contracts.clear();
    provider_a_before.output_contracts = vec![PromisedOutputContract {
        contract_id: "contract.a".to_string(),
        capabilities: vec!["base".to_string()],
    }];
    let mut provider_a_after = provider_a_before.clone();
    provider_a_after.output_contracts[0]
        .capabilities
        .push("new".to_string());

    let mut provider_b_before = canonical_contract_fixture("wi_provider_b");
    provider_b_before.input_contracts.clear();
    provider_b_before.output_contracts = vec![PromisedOutputContract {
        contract_id: "contract.b".to_string(),
        capabilities: vec!["removed".to_string()],
    }];
    let mut provider_b_after = provider_b_before.clone();
    provider_b_after.output_contracts[0].capabilities.clear();

    let mut consumer = canonical_contract_fixture("wi_consumer");
    consumer.input_contracts = vec![
        RequiredInputContract {
            contract_id: "contract.a".to_string(),
            provider_logical_work_item_id: "wi_provider_a".to_string(),
            required_capabilities: vec!["new".to_string()],
            compatibility_policy: ContractCompatibilityPolicy::RequireAll,
        },
        RequiredInputContract {
            contract_id: "contract.b".to_string(),
            provider_logical_work_item_id: "wi_provider_b".to_string(),
            required_capabilities: vec!["removed".to_string()],
            compatibility_policy: ContractCompatibilityPolicy::RequireAll,
        },
    ];
    let graph = DependencyContractGraph {
        contracts: std::collections::BTreeMap::from([
            ("wi_provider_a".to_string(), provider_a_after.clone()),
            ("wi_provider_b".to_string(), provider_b_after.clone()),
            ("wi_consumer".to_string(), consumer),
        ]),
        edges: vec![
            DependencyContractEdge {
                from: "wi_provider_a".to_string(),
                to: "wi_consumer".to_string(),
                required_contracts: vec![RequiredDependencyContract {
                    contract_id: "contract.a".to_string(),
                    required_capabilities: vec!["new".to_string()],
                    compatibility_policy: ContractCompatibilityPolicy::RequireAll,
                }],
            },
            DependencyContractEdge {
                from: "wi_provider_b".to_string(),
                to: "wi_consumer".to_string(),
                required_contracts: vec![RequiredDependencyContract {
                    contract_id: "contract.b".to_string(),
                    required_capabilities: vec!["removed".to_string()],
                    compatibility_policy: ContractCompatibilityPolicy::RequireAll,
                }],
            },
        ],
    };
    let execution = PlanExecutionState {
        units: std::collections::BTreeMap::from([(
            "wi_consumer".to_string(),
            UnitExecutionSnapshot {
                logical_work_item_id: "wi_consumer".to_string(),
                work_item_revision_id: "work_item_revision_consumer_0001".to_string(),
                completed_handoff_revision_id: None,
                has_started: true,
                has_completed: false,
            },
        )]),
    };
    let deltas = vec![
        compute_contract_delta(
            "work_item_revision_a_0001",
            &provider_a_before,
            "work_item_revision_a_0002",
            &provider_a_after,
        ),
        compute_contract_delta(
            "work_item_revision_b_0001",
            &provider_b_before,
            "work_item_revision_b_0002",
            &provider_b_after,
        ),
    ];

    let impact = super::super::aggregate_plan_repair_impact(
        &graph,
        &deltas,
        &execution,
        [&"wi_provider_a".to_string(), &"wi_provider_b".to_string()].into_iter(),
    )
    .unwrap();

    assert_eq!(impact.direct_stale, vec!["wi_consumer"]);
    assert!(impact.direct_revalidation.is_empty());
    assert!(impact.unaffected.iter().all(|unit| unit != "wi_consumer"));
}

#[test]
fn plan_repair_final_scope_merge_keeps_impact_partitions_mutually_exclusive() {
    let fixture = plan_repair_engine_fixture();
    let prepared = fixture.engine.prepare_amendment(&fixture.request).unwrap();
    let mut manifest = prepared.manifest;
    manifest.revalidation_required_units =
        vec!["wi_registration".to_string(), "wi_docs".to_string()];
    manifest.stale_units = vec!["wi_registration".to_string()];
    manifest.unaffected_units = vec!["wi_registration".to_string(), "wi_ops".to_string()];
    let known = prepared
        .next_plan_revision
        .work_item_bindings
        .keys()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    let accepted = ["wi_registration".to_string(), "wi_docs".to_string()]
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>();

    let final_manifest = super::super::final_plan_amendment_manifest(&manifest, &known, &accepted);

    assert_eq!(final_manifest.stale_units, vec!["wi_registration"]);
    assert_eq!(final_manifest.revalidation_required_units, vec!["wi_docs"]);
    assert_eq!(final_manifest.unaffected_units, vec!["wi_ops"]);
    let stale = final_manifest
        .stale_units
        .iter()
        .collect::<std::collections::BTreeSet<_>>();
    let revalidation = final_manifest
        .revalidation_required_units
        .iter()
        .collect::<std::collections::BTreeSet<_>>();
    let unaffected = final_manifest
        .unaffected_units
        .iter()
        .collect::<std::collections::BTreeSet<_>>();
    assert!(stale.is_disjoint(&revalidation));
    assert!(stale.is_disjoint(&unaffected));
    assert!(revalidation.is_disjoint(&unaffected));
}

#[test]
fn plan_repair_publish_records_plan_published_request_status_failures_and_recovers_on_replay() {
    let fixture = plan_repair_engine_fixture();
    let prepared = fixture.engine.prepare_amendment(&fixture.request).unwrap();
    fixture.engine.persist_candidate(&prepared).unwrap();
    let attestation = persist_review_attestation(&fixture, &prepared);
    fixture
        .store
        .update_repair_request_status(
            &fixture.plan,
            &fixture.request.id,
            crate::product::models::PlanRepairRequestStatus::AwaitingConfirmation,
        )
        .unwrap();
    let confirmation = confirmation(&attestation, &["wi_registration"], None);

    for expected_path in ["initial_publish", "active_revision_replay"] {
        let failpoint =
            crate::product::work_item_revision_store::register_repair_request_status_failpoint(
                &fixture.store,
                &fixture.plan,
                &fixture.request.id,
                crate::product::models::PlanRepairRequestStatus::Published,
            );

        let error = fixture
            .engine
            .publish_amendment(prepared.clone(), confirmation.clone())
            .unwrap_err();

        assert!(matches!(
            error,
            super::super::PlanRepairError::Store(
                crate::product::json_store::ProductStoreError::Io(message)
            ) if message.contains("repair_request_status_failpoint")
        ));
        let plan = fixture
            .store
            .get_plan_lineage("project_0001", "issue_0001", &fixture.plan.id)
            .unwrap();
        assert_eq!(
            plan.active_revision_id.as_deref(),
            Some(prepared.next_plan_revision.id.as_str()),
            "{expected_path} must keep the published Plan pointer"
        );
        assert_eq!(
            plan.active_amendment_id.as_deref(),
            Some(prepared.manifest.id.as_str()),
            "{expected_path} must keep the amendment lock"
        );
        let journal = fixture
            .store
            .get_plan_amendment_publication_journal(&plan, &prepared.publication_ids.journal_id)
            .unwrap();
        assert_eq!(
            journal.phase,
            crate::product::models::PlanAmendmentPublicationPhase::PlanPublished
        );
        assert!(journal.error.is_some(), "{expected_path}");
        assert!(journal.recovery.is_some(), "{expected_path}");
        assert_eq!(
            fixture
                .store
                .get_repair_request(&plan, &fixture.request.id)
                .unwrap()
                .status,
            crate::product::models::PlanRepairRequestStatus::AwaitingConfirmation
        );
        assert_no_coding_binding(&fixture);
        drop(failpoint);
    }

    let published = fixture
        .engine
        .publish_amendment(prepared.clone(), confirmation)
        .unwrap();
    assert_eq!(published.id, prepared.manifest.id);
    let plan = fixture
        .store
        .get_plan_lineage("project_0001", "issue_0001", &fixture.plan.id)
        .unwrap();
    let journal = fixture
        .store
        .get_plan_amendment_publication_journal(&plan, &prepared.publication_ids.journal_id)
        .unwrap();
    assert_eq!(journal.error, None);
    assert_eq!(journal.recovery, None);
    assert_eq!(
        fixture
            .store
            .get_repair_request(&plan, &fixture.request.id)
            .unwrap()
            .status,
        crate::product::models::PlanRepairRequestStatus::Published
    );
    assert_no_coding_binding(&fixture);
}

#[test]
fn plan_repair_publish_rejects_attestation_reuse_after_candidate_package_tampering() {
    for tamper in [
        "replacement_units",
        "resume_target",
        "contract_delta_association",
        "impact",
        "validation",
        "plan_projection",
        "work_item_projection",
    ] {
        let fixture = plan_repair_engine_fixture();
        let mut prepared = fixture.engine.prepare_amendment(&fixture.request).unwrap();
        fixture.engine.persist_candidate(&prepared).unwrap();
        let attestation = persist_review_attestation(&fixture, &prepared);
        fixture
            .store
            .update_repair_request_status(
                &fixture.plan,
                &fixture.request.id,
                crate::product::models::PlanRepairRequestStatus::AwaitingConfirmation,
            )
            .unwrap();

        match tamper {
            "replacement_units" => {
                prepared
                    .manifest
                    .replacement_units
                    .insert("wi_registration".to_string(), vec!["wi_ops".to_string()]);
            }
            "resume_target" => {
                prepared.manifest.resume_target.mode = AmendmentResumeMode::Reexecute;
            }
            "contract_delta_association" => {
                prepared.manifest.contract_deltas[0]
                    .added_capability_associations
                    .push(super::super::ContractCapabilityAssociation {
                        contract_id: "contract.tampered".to_string(),
                        capability: "tampered_capability".to_string(),
                    });
            }
            "impact" => {
                prepared
                    .impact_report
                    .conditional_downstream
                    .push("wi_ops".to_string());
            }
            "validation" => {
                prepared.validation_report.contract_validation.findings.push(
                    crate::product::work_item_contract::ContractValidationFinding {
                        code: "tampered_validation".to_string(),
                        severity:
                            crate::product::work_item_contract::ContractFindingSeverity::Warning,
                        logical_work_item_id: Some("wi_core".to_string()),
                        contract_ref: Some("contract.workflow".to_string()),
                        capability_ref: None,
                        message: "tampered validation content".to_string(),
                    },
                );
            }
            "plan_projection" => {
                prepared.plan_projection_bundle.human_group_projection_hash =
                    "tampered_plan_projection_hash".to_string();
            }
            "work_item_projection" => {
                prepared.work_item_projection_bundles[0].human_projection_hash =
                    "tampered_work_item_projection_hash".to_string();
            }
            _ => unreachable!(),
        }

        let error = fixture
            .engine
            .publish_amendment(
                prepared.clone(),
                confirmation(&attestation, &["wi_registration"], None),
            )
            .unwrap_err();

        assert!(
            matches!(
                error,
                super::super::PlanRepairError::InvalidRepairTarget(message)
                    if message.contains("review attestation provenance mismatch")
            ),
            "tamper case {tamper}"
        );
        let plan = fixture
            .store
            .get_plan_lineage("project_0001", "issue_0001", &fixture.plan.id)
            .unwrap();
        assert_eq!(
            plan.active_revision_id.as_deref(),
            Some(fixture.request.base_plan_revision_id.as_str()),
            "tamper case {tamper}"
        );
        assert!(
            fixture
                .store
                .find_plan_amendment_publication_journal(&plan, &prepared.manifest.id)
                .unwrap()
                .is_none(),
            "tamper case {tamper}"
        );
        assert_no_coding_binding(&fixture);
    }
}
