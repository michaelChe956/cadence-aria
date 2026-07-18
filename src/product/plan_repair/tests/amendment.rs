use std::collections::BTreeMap;

use tempfile::TempDir;

use crate::product::app_paths::ProductAppPaths;
use crate::product::models::{
    LogicalWorkItem, PlanAmendmentConfirmation, PlanAmendmentPublicationPhase, PlanDefectClass,
    PlanRepairRequest, PlanRepairRequestStatus, PlanRepairReviewAttestation, PlanRevisionReason,
    RepairTarget, RepairTargetKind, WorkItemDraftRevision, WorkItemPlanLineage,
    WorkItemPlanRevision,
};
use crate::product::runtime_binding_store::RuntimeBindingStore;
use crate::product::work_item_contract::{
    CanonicalWorkItemContract, ContractCompatibilityPolicy, PromisedOutputContract,
    RequiredInputContract, build_dependency_contract_graph, canonical_contract_fixture,
};
use crate::product::work_item_revision_store::{
    InitialWorkItemPublicationIds, PlanAmendmentPublicationCheckpoint, WorkItemRevisionStore,
    register_plan_amendment_publication_failpoint,
};
use crate::product::workspace_engine::compile_work_item_revision;
use crate::web::workspace_ws_types::{
    WorkItemPlanReviewAction, WorkItemPlanReviewComplete, WorkItemPlanReviewScope,
    WorkItemPlanReviewVerdict,
};

use super::super::PlanRepairEngine;

pub(super) struct PlanRepairEngineFixture {
    pub(super) _temp: TempDir,
    pub(super) store: WorkItemRevisionStore,
    pub(super) plan: WorkItemPlanLineage,
    pub(super) request: PlanRepairRequest,
    pub(super) engine: PlanRepairEngine,
}

#[test]
fn plan_repair_prepares_upstream_only_amendment_for_compatible_extension() {
    let fixture = plan_repair_engine_fixture();

    let prepared = fixture.engine.prepare_amendment(&fixture.request).unwrap();

    assert_eq!(
        prepared
            .manifest
            .revised_work_items
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        vec!["wi_core"]
    );
    assert_eq!(
        prepared.manifest.revalidation_required_units,
        vec!["wi_registration"]
    );
    assert!(prepared.manifest.stale_units.is_empty());
    assert_eq!(
        prepared.next_plan_revision.work_item_bindings["wi_registration"],
        "work_item_revision_wi_registration_0001"
    );
    assert_eq!(
        prepared.next_plan_revision.work_item_bindings["wi_docs"],
        "work_item_revision_wi_docs_0001"
    );
    assert_eq!(
        prepared.next_plan_revision.work_item_bindings["wi_ops"],
        "work_item_revision_wi_ops_0001"
    );

    let stored = fixture
        .store
        .get_plan_lineage("project_0001", "issue_0001", &fixture.plan.id)
        .unwrap();
    assert_eq!(
        stored.active_revision_id.as_deref(),
        Some("plan_revision_0001")
    );
    assert_eq!(
        fixture
            .store
            .get_logical_work_item(&fixture.plan, "wi_core")
            .unwrap()
            .active_revision_id
            .as_deref(),
        Some("work_item_revision_wi_core_0001")
    );

    fixture.engine.persist_candidate(&prepared).unwrap();
    assert_eq!(
        fixture
            .store
            .get_plan_revision(
                "project_0001",
                "issue_0001",
                &fixture.plan.id,
                &prepared.next_plan_revision.id,
            )
            .unwrap(),
        prepared.next_plan_revision
    );
    assert_eq!(
        fixture
            .store
            .get_plan_lineage("project_0001", "issue_0001", &fixture.plan.id)
            .unwrap()
            .active_revision_id
            .as_deref(),
        Some("plan_revision_0001")
    );
}

#[test]
fn plan_repair_publish_rejects_changed_base_revision() {
    let fixture = plan_repair_engine_fixture();
    let prepared = fixture.engine.prepare_amendment(&fixture.request).unwrap();
    fixture.engine.persist_candidate(&prepared).unwrap();
    let attestation = persist_review_attestation(&fixture, &prepared);
    fixture
        .store
        .update_repair_request_status(
            &fixture.plan,
            &fixture.request.id,
            PlanRepairRequestStatus::AwaitingConfirmation,
        )
        .unwrap();
    let mut conflicting = prepared.next_plan_revision.clone();
    conflicting.id = "plan_revision_conflict_0003".to_string();
    conflicting.revision_no = 3;
    fixture
        .store
        .put_plan_revision(&fixture.plan, &conflicting)
        .unwrap();
    fixture
        .store
        .set_active_plan_revision(&fixture.plan, &conflicting.id)
        .unwrap();

    let error = fixture
        .engine
        .publish_amendment(
            prepared,
            PlanAmendmentConfirmation {
                amendment_id: "plan_amendment_0001".to_string(),
                base_plan_revision_id: "plan_revision_0001".to_string(),
                accepted_impact_scope: vec!["wi_registration".to_string()],
                risk_acceptance_reason: None,
                review_attestation_id: Some(attestation.id),
                confirmed_by: "user_0001".to_string(),
                confirmed_at: "2026-07-18T00:00:05Z".to_string(),
            },
        )
        .unwrap_err();

    assert!(matches!(
        error,
        super::super::PlanRepairError::AmendmentConflict { .. }
    ));
}

#[test]
fn plan_repair_publish_is_exact_replay_and_keeps_active_amendment_lock() {
    let fixture = plan_repair_engine_fixture();
    let prepared = fixture.engine.prepare_amendment(&fixture.request).unwrap();
    fixture.engine.persist_candidate(&prepared).unwrap();
    let attestation = persist_review_attestation(&fixture, &prepared);
    fixture
        .store
        .update_repair_request_status(
            &fixture.plan,
            &fixture.request.id,
            PlanRepairRequestStatus::AwaitingConfirmation,
        )
        .unwrap();
    let confirmation = confirmation(&attestation, &["wi_registration"], None);

    let published = fixture
        .engine
        .publish_amendment(prepared.clone(), confirmation.clone())
        .unwrap();
    let replayed = fixture
        .engine
        .publish_amendment(prepared.clone(), confirmation.clone())
        .unwrap();

    assert_eq!(replayed, published);
    let plan = fixture
        .store
        .get_plan_lineage("project_0001", "issue_0001", &fixture.plan.id)
        .unwrap();
    assert_eq!(
        plan.active_revision_id.as_deref(),
        Some(prepared.next_plan_revision.id.as_str())
    );
    assert_eq!(
        plan.active_amendment_id.as_deref(),
        Some(prepared.manifest.id.as_str())
    );
    assert_eq!(
        fixture
            .store
            .get_repair_request(&plan, &fixture.request.id)
            .unwrap()
            .status,
        PlanRepairRequestStatus::Published
    );
    let journal_id = format!("{}_publication_journal", prepared.manifest.id);
    let mut journal = fixture
        .store
        .get_plan_amendment_publication_journal(&plan, &journal_id)
        .unwrap();
    assert_eq!(journal.phase, PlanAmendmentPublicationPhase::PlanPublished);

    journal.phase = PlanAmendmentPublicationPhase::Prepared;
    journal.error = Some("simulated crash after active pointer CAS".to_string());
    crate::product::json_store::write_json(
        &fixture
            ._temp
            .path()
            .join(".aria/projects/project_0001/issues/issue_0001/work-item-revisions")
            .join(&fixture.plan.id)
            .join("amendment-publication-journals")
            .join(format!("{journal_id}.json")),
        &journal,
    )
    .unwrap();

    assert_eq!(
        fixture
            .engine
            .publish_amendment(prepared, confirmation)
            .unwrap(),
        published
    );
    assert_eq!(
        fixture
            .store
            .get_plan_amendment_publication_journal(&plan, &journal_id)
            .unwrap()
            .phase,
        PlanAmendmentPublicationPhase::PlanPublished
    );
    assert_no_coding_binding(&fixture);
}

#[test]
fn plan_repair_publish_recovers_each_journal_phase_without_releasing_lock_or_writing_binding() {
    let cases = [
        (
            PlanAmendmentPublicationCheckpoint::JournalPreparing,
            PlanAmendmentPublicationPhase::Preparing,
            "plan_revision_0001",
        ),
        (
            PlanAmendmentPublicationCheckpoint::FirstArtifactsWritten,
            PlanAmendmentPublicationPhase::Preparing,
            "plan_revision_0001",
        ),
        (
            PlanAmendmentPublicationCheckpoint::JournalPrepared,
            PlanAmendmentPublicationPhase::Prepared,
            "plan_revision_0001",
        ),
        (
            PlanAmendmentPublicationCheckpoint::ActivePlanRevisionPublished,
            PlanAmendmentPublicationPhase::Prepared,
            "plan_revision_0002",
        ),
        (
            PlanAmendmentPublicationCheckpoint::JournalPlanPublished,
            PlanAmendmentPublicationPhase::PlanPublished,
            "plan_revision_0002",
        ),
    ];

    for (checkpoint, failed_phase, failed_active_revision) in cases {
        let fixture = plan_repair_engine_fixture();
        let prepared = fixture.engine.prepare_amendment(&fixture.request).unwrap();
        fixture.engine.persist_candidate(&prepared).unwrap();
        let attestation = persist_review_attestation(&fixture, &prepared);
        fixture
            .store
            .update_repair_request_status(
                &fixture.plan,
                &fixture.request.id,
                PlanRepairRequestStatus::AwaitingConfirmation,
            )
            .unwrap();
        let confirmation = confirmation(&attestation, &["wi_registration"], None);
        let _failpoint = register_plan_amendment_publication_failpoint(
            &fixture.store,
            &fixture.plan,
            &prepared.publication_ids.journal_id,
            checkpoint,
        );

        let error = fixture
            .engine
            .publish_amendment(prepared.clone(), confirmation.clone())
            .unwrap_err();
        assert!(matches!(
            error,
            super::super::PlanRepairError::Store(
                crate::product::json_store::ProductStoreError::Io(message)
            ) if message.contains("amendment_publication_failpoint")
        ));

        let failed_plan = fixture
            .store
            .get_plan_lineage("project_0001", "issue_0001", &fixture.plan.id)
            .unwrap();
        assert_eq!(
            failed_plan.active_revision_id.as_deref(),
            Some(failed_active_revision)
        );
        assert_eq!(
            failed_plan.active_amendment_id.as_deref(),
            Some(prepared.manifest.id.as_str())
        );
        let failed_journal = fixture
            .store
            .get_plan_amendment_publication_journal(
                &failed_plan,
                &prepared.publication_ids.journal_id,
            )
            .unwrap();
        assert_eq!(failed_journal.phase, failed_phase);
        assert!(failed_journal.error.is_some());
        assert!(failed_journal.recovery.is_some());
        assert_eq!(
            fixture
                .store
                .get_repair_request(&failed_plan, &fixture.request.id)
                .unwrap()
                .status,
            PlanRepairRequestStatus::AwaitingConfirmation
        );
        assert_no_coding_binding(&fixture);

        let published = fixture
            .engine
            .publish_amendment(prepared.clone(), confirmation)
            .unwrap();
        assert_eq!(published.id, prepared.manifest.id);
        let recovered_plan = fixture
            .store
            .get_plan_lineage("project_0001", "issue_0001", &fixture.plan.id)
            .unwrap();
        assert_eq!(
            recovered_plan.active_revision_id.as_deref(),
            Some(prepared.next_plan_revision.id.as_str())
        );
        assert_eq!(
            recovered_plan.active_amendment_id.as_deref(),
            Some(prepared.manifest.id.as_str())
        );
        let recovered_journal = fixture
            .store
            .get_plan_amendment_publication_journal(
                &recovered_plan,
                &prepared.publication_ids.journal_id,
            )
            .unwrap();
        assert_eq!(
            recovered_journal.phase,
            PlanAmendmentPublicationPhase::PlanPublished
        );
        assert_eq!(recovered_journal.error, None);
        assert_eq!(recovered_journal.recovery, None);
        assert_eq!(
            fixture
                .store
                .get_repair_request(&recovered_plan, &fixture.request.id)
                .unwrap()
                .status,
            PlanRepairRequestStatus::Published
        );
        assert_no_coding_binding(&fixture);
    }
}

#[test]
fn plan_repair_publish_expands_revalidation_and_rejects_unreviewed_shrink() {
    let expanded_fixture = plan_repair_engine_fixture();
    let expanded = expanded_fixture
        .engine
        .prepare_amendment(&expanded_fixture.request)
        .unwrap();
    expanded_fixture
        .engine
        .persist_candidate(&expanded)
        .unwrap();
    let original_review = persist_review_attestation(&expanded_fixture, &expanded);
    expanded_fixture
        .store
        .update_repair_request_status(
            &expanded_fixture.plan,
            &expanded_fixture.request.id,
            PlanRepairRequestStatus::AwaitingConfirmation,
        )
        .unwrap();
    let expanded_manifest = expanded_fixture
        .engine
        .publish_amendment(
            expanded,
            confirmation(&original_review, &["wi_docs", "wi_registration"], None),
        )
        .unwrap();
    assert_eq!(
        expanded_manifest.revalidation_required_units,
        vec!["wi_docs", "wi_registration"]
    );
    assert_eq!(expanded_manifest.unaffected_units, vec!["wi_ops"]);

    let shrink_fixture = plan_repair_engine_fixture();
    let shrink = shrink_fixture
        .engine
        .prepare_amendment(&shrink_fixture.request)
        .unwrap();
    shrink_fixture.engine.persist_candidate(&shrink).unwrap();
    let old_review = persist_review_attestation(&shrink_fixture, &shrink);
    shrink_fixture
        .store
        .update_repair_request_status(
            &shrink_fixture.plan,
            &shrink_fixture.request.id,
            PlanRepairRequestStatus::AwaitingConfirmation,
        )
        .unwrap();
    let error = shrink_fixture
        .engine
        .publish_amendment(
            shrink.clone(),
            confirmation(&old_review, &[], Some("accept delayed registration risk")),
        )
        .unwrap_err();
    assert!(matches!(
        error,
        super::super::PlanRepairError::InvalidRepairTarget(_)
    ));
    assert!(
        shrink_fixture
            .store
            .find_plan_amendment_publication_journal(&shrink_fixture.plan, &shrink.manifest.id,)
            .unwrap()
            .is_none()
    );
}

pub(super) fn plan_repair_engine_fixture() -> PlanRepairEngineFixture {
    let temp = TempDir::new().unwrap();
    let store = WorkItemRevisionStore::new(ProductAppPaths::new(temp.path().join(".aria")));
    let mut plan = WorkItemPlanLineage {
        id: "work_item_plan_0001".to_string(),
        project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
        story_spec_refs: vec!["story_spec_0001".to_string()],
        design_spec_refs: vec!["design_spec_0001".to_string()],
        active_revision_id: None,
        active_amendment_id: None,
        created_at: "2026-07-18T00:00:00Z".to_string(),
        updated_at: "2026-07-18T00:00:00Z".to_string(),
    };
    store.put_plan_lineage(&plan).unwrap();

    let contracts = vec![
        core_contract(&["workflow_explicit_completion"]),
        consumer_contract(),
        unrelated_contract("wi_docs"),
        unrelated_contract("wi_ops"),
    ];
    let mut bindings = BTreeMap::new();
    for contract in &contracts {
        let logical_id = contract.identity.logical_work_item_id.clone();
        let revision_id = format!("work_item_revision_{logical_id}_0001");
        let draft = WorkItemDraftRevision {
            id: format!("work_item_draft_revision_{logical_id}_0001"),
            logical_work_item_id: logical_id.clone(),
            revision_no: 1,
            supersedes: None,
            revision_reason: PlanRevisionReason::InitialCompile,
            canonical_contract_candidate: contract.clone(),
            trigger_repair_request_id: None,
            created_at: "2026-07-18T00:00:01Z".to_string(),
        };
        let ids = InitialWorkItemPublicationIds {
            work_item_revision_id: revision_id.clone(),
            verification_plan_revision_id: format!("verification_plan_revision_{logical_id}_0001"),
            work_item_projection_bundle_id: format!(
                "work_item_projection_bundle_{logical_id}_0001"
            ),
        };
        let compiled = compile_work_item_revision(
            &draft,
            &crate::product::work_item_projection::WorkItemProjectionCompiler,
            &ids,
            "2026-07-18T00:00:01Z",
        )
        .unwrap();
        let logical = LogicalWorkItem {
            id: logical_id.clone(),
            plan_id: plan.id.clone(),
            title: contract.identity.title.clone(),
            active_revision_id: None,
            created_at: "2026-07-18T00:00:00Z".to_string(),
            updated_at: "2026-07-18T00:00:00Z".to_string(),
        };
        store.put_logical_work_item(&plan, &logical).unwrap();
        store.put_draft_revision(&plan, &draft).unwrap();
        store
            .put_verification_plan_revision(&plan, &compiled.verification_plan_revision)
            .unwrap();
        store
            .put_work_item_projection_bundle(&plan, &compiled.projection_bundle)
            .unwrap();
        store
            .put_work_item_revision(&plan, &compiled.work_item_revision)
            .unwrap();
        store
            .set_active_work_item_revision(&plan, &logical, None, &revision_id)
            .unwrap();
        bindings.insert(logical_id, revision_id);
    }
    let graph = build_dependency_contract_graph(&contracts).unwrap();
    store
        .put_dependency_graph_revision(
            &plan,
            &crate::product::models::DependencyGraphRevision {
                id: "dependency_graph_revision_0001".to_string(),
                plan_id: plan.id.clone(),
                edges: graph.edges,
                created_at: "2026-07-18T00:00:01Z".to_string(),
            },
        )
        .unwrap();
    store
        .put_plan_revision(
            &plan,
            &WorkItemPlanRevision {
                id: "plan_revision_0001".to_string(),
                plan_id: plan.id.clone(),
                revision_no: 1,
                supersedes: None,
                reason: PlanRevisionReason::InitialCompile,
                work_item_bindings: bindings,
                dependency_graph_revision_id: "dependency_graph_revision_0001".to_string(),
                validation_report_ref: "plan_validation_report_0001".to_string(),
                plan_projection_bundle_id: "plan_projection_bundle_0001".to_string(),
                created_at: "2026-07-18T00:00:01Z".to_string(),
            },
        )
        .unwrap();
    plan = store
        .set_active_plan_revision(&plan, "plan_revision_0001")
        .unwrap();
    plan = store
        .acquire_active_amendment(&plan, "plan_amendment_0001")
        .unwrap();

    let request = PlanRepairRequest {
        id: "plan_repair_request_0001".to_string(),
        plan_id: plan.id.clone(),
        base_plan_revision_id: "plan_revision_0001".to_string(),
        trigger_attempt_id: "coding_attempt_0001".to_string(),
        trigger_unit_run_id: "coding_unit_run_0001".to_string(),
        trigger_review_id: Some("code_review_0001".to_string()),
        trigger_finding_id: "finding_0001".to_string(),
        amendment_id: Some("plan_amendment_0001".to_string()),
        defect_class: PlanDefectClass::UpstreamContractInvalid,
        reason_code: "upstream_contract_invalid".to_string(),
        repair_target: RepairTarget {
            kind: RepairTargetKind::UpstreamWorkItem,
            logical_work_item_ids: vec!["wi_core".to_string()],
            work_item_revision_ids: vec!["work_item_revision_wi_core_0001".to_string()],
        },
        contract_refs: vec!["contract.workflow".to_string()],
        capability_refs: vec![
            "finalization_failure".to_string(),
            "failure_message".to_string(),
        ],
        evidence: vec![],
        fingerprint: "fingerprint_0001".to_string(),
        status: PlanRepairRequestStatus::InProgress,
        created_at: "2026-07-18T00:00:02Z".to_string(),
        updated_at: "2026-07-18T00:00:02Z".to_string(),
    };
    store.put_repair_request(&plan, &request).unwrap();
    let candidate = WorkItemDraftRevision {
        id: "work_item_draft_revision_wi_core_0002".to_string(),
        logical_work_item_id: "wi_core".to_string(),
        revision_no: 2,
        supersedes: Some("work_item_draft_revision_wi_core_0001".to_string()),
        revision_reason: PlanRevisionReason::RepairUpstreamContract,
        canonical_contract_candidate: core_contract(&[
            "workflow_explicit_completion",
            "finalization_failure",
            "failure_message",
        ]),
        trigger_repair_request_id: Some(request.id.clone()),
        created_at: "2026-07-18T00:00:03Z".to_string(),
    };
    let engine = PlanRepairEngine::new(store.clone(), plan.clone())
        .with_candidate_drafts(vec![candidate])
        .with_created_at("2026-07-18T00:00:03Z");

    PlanRepairEngineFixture {
        _temp: temp,
        store,
        plan,
        request,
        engine,
    }
}

fn core_contract(capabilities: &[&str]) -> CanonicalWorkItemContract {
    let mut contract = canonical_contract_fixture("wi_core");
    contract.input_contracts.clear();
    contract.output_contracts = vec![PromisedOutputContract {
        contract_id: "contract.workflow".to_string(),
        capabilities: capabilities
            .iter()
            .map(|value| (*value).to_string())
            .collect(),
    }];
    contract.handoff_contract.provided_contract_refs = vec!["contract.workflow".to_string()];
    contract
}

fn consumer_contract() -> CanonicalWorkItemContract {
    let mut contract = canonical_contract_fixture("wi_registration");
    contract.input_contracts = vec![RequiredInputContract {
        contract_id: "contract.workflow".to_string(),
        provider_logical_work_item_id: "wi_core".to_string(),
        required_capabilities: vec!["finalization_failure".to_string()],
        compatibility_policy: ContractCompatibilityPolicy::RequireAll,
    }];
    contract.output_contracts = vec![PromisedOutputContract {
        contract_id: "contract.registration".to_string(),
        capabilities: vec!["registration_ready".to_string()],
    }];
    contract.handoff_contract.provided_contract_refs.clear();
    contract
}

fn unrelated_contract(logical_id: &str) -> CanonicalWorkItemContract {
    let mut contract = canonical_contract_fixture(logical_id);
    contract.input_contracts.clear();
    contract.output_contracts = vec![PromisedOutputContract {
        contract_id: format!("contract.{logical_id}"),
        capabilities: vec![format!("{logical_id}_ready")],
    }];
    contract.handoff_contract.provided_contract_refs.clear();
    contract
}

pub(super) fn persist_review_attestation(
    fixture: &PlanRepairEngineFixture,
    prepared: &super::super::PreparedPlanAmendment,
) -> PlanRepairReviewAttestation {
    let attestation = PlanRepairReviewAttestation {
        id: "plan_repair_review_attestation_plan_amendment_0001_round_0001".to_string(),
        request_id: fixture.request.id.clone(),
        amendment_id: prepared.manifest.id.clone(),
        plan_id: fixture.plan.id.clone(),
        base_plan_revision_id: prepared.base_plan_revision_id.clone(),
        reviewed_plan_revision_id: prepared.next_plan_revision.id.clone(),
        plan_projection_bundle_id: prepared.plan_projection_bundle.id.clone(),
        generation_round_id: "round_0001".to_string(),
        accepted_impact_scope: vec!["wi_registration".to_string()],
        risk_acceptance_reason: None,
        candidate_package_artifact_id: prepared.candidate_package.id.clone(),
        candidate_package_fingerprint: super::super::candidate_package_fingerprint(
            &fixture.request,
            &prepared.manifest,
            &prepared.plan_projection_bundle,
            &prepared.work_item_projection_bundles,
            &prepared.validation_report,
            &prepared.impact_report,
        )
        .unwrap(),
        review: WorkItemPlanReviewComplete {
            verdict: WorkItemPlanReviewVerdict::Pass,
            review_scope: WorkItemPlanReviewScope::Outline,
            target_outline_id: None,
            generation_round_id: "round_0001".to_string(),
            draft_id: None,
            batch_id: None,
            review_action: WorkItemPlanReviewAction::Continue,
            gates: vec![],
            affects_items: vec![],
            warnings: vec![],
        },
        created_at: "2026-07-18T00:00:04Z".to_string(),
    };
    fixture
        .store
        .put_plan_repair_review_attestation(&fixture.plan, &attestation)
        .unwrap();
    attestation
}

pub(super) fn confirmation(
    attestation: &PlanRepairReviewAttestation,
    accepted_scope: &[&str],
    risk: Option<&str>,
) -> PlanAmendmentConfirmation {
    PlanAmendmentConfirmation {
        amendment_id: attestation.amendment_id.clone(),
        base_plan_revision_id: attestation.base_plan_revision_id.clone(),
        accepted_impact_scope: accepted_scope
            .iter()
            .map(|value| (*value).to_string())
            .collect(),
        risk_acceptance_reason: risk.map(str::to_string),
        review_attestation_id: Some(attestation.id.clone()),
        confirmed_by: "user_0001".to_string(),
        confirmed_at: "2026-07-18T00:00:05Z".to_string(),
    }
}

pub(super) fn assert_no_coding_binding(fixture: &PlanRepairEngineFixture) {
    let bindings =
        RuntimeBindingStore::new(ProductAppPaths::new(fixture._temp.path().join(".aria")))
            .list("project_0001", "issue_0001")
            .unwrap();
    assert!(bindings.is_empty());
}
