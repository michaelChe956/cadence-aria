use std::collections::BTreeMap;

use crate::product::models::{
    DependencyGraphRevision, HandoffRevision, HumanPresentationRevision, PlanProjectionBundle,
    PlanRepairReviewAttestation, PlanValidationReportArtifact, WorkItemProjectionBundle,
};
use crate::product::work_item_contract::{
    ContractValidationReport, build_dependency_contract_graph,
};
use crate::product::work_item_projection::{
    PlanProjectionCompileInput, PlanProjectionCompiler, ProjectionValidationReport,
    WorkItemProjectionCompiler, projection_hashes,
};
use crate::web::workspace_ws_types::{
    WorkItemPlanReviewAction, WorkItemPlanReviewComplete, WorkItemPlanReviewScope,
    WorkItemPlanReviewVerdict,
};

use super::{PLAN_ID, WORK_ITEM_ID, logical_work_item, test_store_and_plan, work_item_revision};

#[test]
fn work_item_revision_store_persists_scoped_revision_artifacts() {
    let (_temp, store, plan) = test_store_and_plan();
    let logical = logical_work_item();
    store.put_logical_work_item(&plan, &logical).unwrap();
    let revision = work_item_revision();
    store.put_work_item_revision(&plan, &revision).unwrap();

    let compiled_work_item = WorkItemProjectionCompiler
        .compile(&revision.canonical_contract, &revision.id)
        .unwrap();
    let work_item_hashes = projection_hashes(&compiled_work_item).unwrap();
    let validation = PlanValidationReportArtifact {
        id: "plan_validation_report_0001".to_string(),
        plan_id: PLAN_ID.to_string(),
        plan_revision_id: "plan_revision_0001".to_string(),
        plan_projection_bundle_id: "plan_projection_bundle_0001".to_string(),
        contract_validation: ContractValidationReport { findings: vec![] },
        projection_validation: ProjectionValidationReport { findings: vec![] },
        created_at: "2026-07-17T00:00:05Z".to_string(),
    };
    store
        .put_plan_validation_report(&plan, &validation)
        .unwrap();
    assert_eq!(
        store
            .get_plan_validation_report(&plan, &validation.id)
            .unwrap(),
        validation
    );

    let work_item_projection = WorkItemProjectionBundle {
        id: "work_item_projection_bundle_0001".to_string(),
        work_item_revision_id: revision.id.clone(),
        canonical_contract_hash: revision.canonical_contract_hash.clone(),
        projection_schema_version: 1,
        compiler_version: "compiler-v1".to_string(),
        human_projection: compiled_work_item.human.clone(),
        coder_projection: compiled_work_item.coder.clone(),
        reviewer_projection: compiled_work_item.reviewer.clone(),
        human_projection_hash: work_item_hashes.human,
        coder_projection_hash: work_item_hashes.coder,
        reviewer_projection_hash: work_item_hashes.reviewer,
        created_at: "2026-07-17T00:00:06Z".to_string(),
    };
    store
        .put_work_item_projection_bundle(&plan, &work_item_projection)
        .unwrap();
    assert_eq!(
        store
            .get_work_item_projection_bundle(&plan, &work_item_projection.id)
            .unwrap(),
        work_item_projection
    );

    let mut plan_contract = revision.canonical_contract.clone();
    plan_contract.input_contracts.clear();
    plan_contract
        .handoff_contract
        .provided_contract_refs
        .clear();
    let graph = build_dependency_contract_graph(&[plan_contract.clone()]).unwrap();
    let plan_work_item = WorkItemProjectionCompiler
        .compile(&plan_contract, &revision.id)
        .unwrap();
    let work_items = BTreeMap::from([(WORK_ITEM_ID.to_string(), plan_work_item)]);
    let expected_revision_ids = BTreeMap::from([(WORK_ITEM_ID.to_string(), revision.id.clone())]);
    let compiled_plan = PlanProjectionCompiler
        .compile(PlanProjectionCompileInput {
            plan_id: PLAN_ID,
            goal: "Persist scoped projection artifacts",
            split_reason: "Single work item",
            source_refs: vec!["design_spec_0001".to_string()],
            dependency_graph: &graph,
            work_item_projections: &work_items,
            expected_work_item_revision_ids: expected_revision_ids,
        })
        .unwrap();
    let plan_projection = PlanProjectionBundle {
        id: "plan_projection_bundle_0001".to_string(),
        plan_revision_id: "plan_revision_0001".to_string(),
        dependency_graph_revision_id: "dependency_graph_revision_0001".to_string(),
        work_item_projection_bundle_refs: vec![work_item_projection.id.clone()],
        human_group_projection: compiled_plan.human,
        coder_group_context: compiled_plan.coder,
        reviewer_group_matrix: compiled_plan.reviewer,
        human_group_projection_hash: "human_group_hash".to_string(),
        coder_group_context_hash: "coder_group_hash".to_string(),
        reviewer_group_matrix_hash: "reviewer_group_hash".to_string(),
        compiler_version: "compiler-v1".to_string(),
        created_at: "2026-07-17T00:00:07Z".to_string(),
    };
    store
        .put_plan_projection_bundle(&plan, &plan_projection)
        .unwrap();
    assert_eq!(
        store
            .get_plan_projection_bundle(&plan, &plan_projection.id)
            .unwrap(),
        plan_projection
    );

    assert_presentation_roundtrip(&store, &plan, &plan_projection.id);

    let dependency = DependencyGraphRevision {
        id: "dependency_graph_revision_0001".to_string(),
        plan_id: PLAN_ID.to_string(),
        edges: vec![crate::product::work_item_contract::DependencyContractEdge {
            from: WORK_ITEM_ID.to_string(),
            to: "logical_work_item_0002".to_string(),
            required_contracts: vec![],
        }],
        created_at: "2026-07-17T00:00:10Z".to_string(),
    };
    store
        .put_dependency_graph_revision(&plan, &dependency)
        .unwrap();
    assert_eq!(
        store
            .get_dependency_graph_revision(&plan, &dependency.id)
            .unwrap(),
        dependency
    );

    let handoff = HandoffRevision {
        id: "handoff_revision_0001".to_string(),
        logical_work_item_id: WORK_ITEM_ID.to_string(),
        work_item_revision_id: revision.id,
        coding_unit_run_id: "coding_unit_run_0001".to_string(),
        provided_contracts: vec!["contract_0001".to_string()],
        provided_capabilities: BTreeMap::from([(
            "capability_0001".to_string(),
            vec!["contract_0001".to_string()],
        )]),
        contract_hash: "contract_hash_0001".to_string(),
        commit_sha: "0123456789abcdef".to_string(),
        created_at: "2026-07-17T00:00:11Z".to_string(),
    };
    store.put_handoff_revision(&plan, &handoff).unwrap();
    assert_eq!(
        store
            .get_handoff_revision(&plan, WORK_ITEM_ID, &handoff.id)
            .unwrap(),
        handoff
    );
}

#[test]
fn plan_repair_review_attestation_store_is_scoped_immutable_and_idempotent() {
    let (_temp, store, plan) = test_store_and_plan();
    let attestation = PlanRepairReviewAttestation {
        id: "plan_repair_review_attestation_0001".to_string(),
        request_id: "plan_repair_request_0001".to_string(),
        amendment_id: "plan_amendment_0001".to_string(),
        plan_id: PLAN_ID.to_string(),
        base_plan_revision_id: "plan_revision_0001".to_string(),
        reviewed_plan_revision_id: "plan_revision_0002".to_string(),
        plan_projection_bundle_id: "plan_projection_bundle_0002".to_string(),
        generation_round_id: "repair_round_0001".to_string(),
        accepted_impact_scope: vec![WORK_ITEM_ID.to_string()],
        risk_acceptance_reason: None,
        candidate_package_artifact_id: "plan_repair_candidate_package_0001".to_string(),
        candidate_package_fingerprint: "candidate_package_fingerprint_0001".to_string(),
        review: WorkItemPlanReviewComplete {
            verdict: WorkItemPlanReviewVerdict::Pass,
            review_scope: WorkItemPlanReviewScope::Outline,
            target_outline_id: None,
            generation_round_id: "repair_round_0001".to_string(),
            draft_id: None,
            batch_id: None,
            review_action: WorkItemPlanReviewAction::Continue,
            gates: Vec::new(),
            affects_items: Vec::new(),
            warnings: Vec::new(),
        },
        created_at: "2026-07-18T00:00:02Z".to_string(),
    };
    let mut missing_fingerprint = attestation.clone();
    missing_fingerprint.id = "plan_repair_review_attestation_missing_fingerprint_0001".to_string();
    missing_fingerprint.candidate_package_fingerprint.clear();
    assert!(matches!(
        store.put_plan_repair_review_attestation(&plan, &missing_fingerprint),
        Err(crate::product::json_store::ProductStoreError::IdentityMismatch { .. })
    ));

    store
        .put_plan_repair_review_attestation(&plan, &attestation)
        .unwrap();
    store
        .put_plan_repair_review_attestation(&plan, &attestation)
        .unwrap();
    assert_eq!(
        store
            .get_plan_repair_review_attestation(&plan, &attestation.id)
            .unwrap(),
        attestation
    );
    let mut conflicting = attestation.clone();
    conflicting.reviewed_plan_revision_id = "plan_revision_wrong".to_string();
    assert!(matches!(
        store.put_plan_repair_review_attestation(&plan, &conflicting),
        Err(crate::product::json_store::ProductStoreError::IdentityMismatch { .. })
    ));
}

fn assert_presentation_roundtrip(
    store: &super::WorkItemRevisionStore,
    plan: &crate::product::models::WorkItemPlanLineage,
    plan_projection_bundle_id: &str,
) {
    let first = HumanPresentationRevision {
        id: "human_presentation_revision_0001".to_string(),
        source_plan_projection_bundle_id: Some(plan_projection_bundle_id.to_string()),
        source_work_item_projection_bundle_id: None,
        supersedes: None,
        human_summary: "first".to_string(),
        why_split: None,
        dependency_explanation: vec![],
        risk_explanation: vec![],
        source_refs: vec![],
        normative: false,
        used_by_provider: false,
        created_at: "2026-07-17T00:00:08Z".to_string(),
    };
    let mut latest = first.clone();
    latest.id = "human_presentation_revision_0002".to_string();
    latest.supersedes = Some(first.id.clone());
    latest.human_summary = "latest".to_string();
    latest.created_at = "2026-07-17T00:00:09Z".to_string();
    store
        .put_human_presentation_revision(plan, &latest)
        .unwrap();
    store.put_human_presentation_revision(plan, &first).unwrap();
    assert_eq!(
        store
            .get_latest_human_presentation_revision(plan, plan_projection_bundle_id)
            .unwrap(),
        Some(latest)
    );
}
