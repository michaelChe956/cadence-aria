use std::collections::BTreeMap;

use tempfile::TempDir;

use crate::product::app_paths::ProductAppPaths;
use crate::product::json_store::{ProductStoreError, write_json};
use crate::product::models::{
    AmendmentResumeMode, AmendmentResumeTarget, HumanPresentationRevision, LogicalWorkItem,
    PlanAmendmentManifest, PlanAmendmentPublicationJournal, PlanAmendmentPublicationPhase,
    PlanDefectClass, PlanDefectEvidence, PlanRepairRequest, PlanRepairRequestStatus,
    PlanRevisionReason, RepairTarget, RepairTargetKind, VerificationPlanRevision,
    WorkItemDraftRevision, WorkItemDraftRevisionStatus, WorkItemPlanLineage, WorkItemPlanRevision,
    WorkItemRevision, WorkItemRevisionReplacement,
};
use crate::product::work_item_contract::canonical_contract_fixture;

use super::WorkItemRevisionStore;

mod concurrency;
#[path = "tests/handoff_deletion.rs"]
mod handoff_deletion;
mod initial_publication;
mod projection_artifacts;
mod publication;
mod repair_status;

const PROJECT_ID: &str = "project_0001";
const ISSUE_ID: &str = "issue_0001";
const PLAN_ID: &str = "work_item_plan_0001";
const WORK_ITEM_ID: &str = "logical_work_item_0001";

fn test_store_and_plan() -> (TempDir, WorkItemRevisionStore, WorkItemPlanLineage) {
    let temp = TempDir::new().unwrap();
    let store = WorkItemRevisionStore::new(ProductAppPaths::new(temp.path().join(".aria")));
    let plan = plan_lineage();
    store.put_plan_lineage(&plan).unwrap();
    (temp, store, plan)
}

fn plan_lineage() -> WorkItemPlanLineage {
    WorkItemPlanLineage {
        id: PLAN_ID.to_string(),
        project_id: PROJECT_ID.to_string(),
        issue_id: ISSUE_ID.to_string(),
        story_spec_refs: vec!["story_spec_0001".to_string()],
        design_spec_refs: vec!["design_spec_0001".to_string()],
        active_revision_id: None,
        active_amendment_id: None,
        created_at: "2026-07-17T00:00:00Z".to_string(),
        updated_at: "2026-07-17T00:00:00Z".to_string(),
    }
}

fn plan_revision(id: &str, revision_no: u32) -> WorkItemPlanRevision {
    WorkItemPlanRevision {
        id: id.to_string(),
        plan_id: PLAN_ID.to_string(),
        revision_no,
        supersedes: (revision_no > 1).then(|| "plan_revision_0001".to_string()),
        reason: PlanRevisionReason::InitialCompile,
        work_item_bindings: BTreeMap::from([(
            WORK_ITEM_ID.to_string(),
            format!("work_item_revision_{revision_no:04}"),
        )]),
        dependency_graph_revision_id: format!("dependency_graph_revision_{revision_no:04}"),
        validation_report_ref: format!("plan_validation_report_{revision_no:04}"),
        plan_projection_bundle_id: format!("plan_projection_bundle_{revision_no:04}"),
        publication_provenance_ref: None,
        created_at: format!("2026-07-17T00:00:0{revision_no}Z"),
    }
}

fn logical_work_item() -> LogicalWorkItem {
    LogicalWorkItem {
        id: WORK_ITEM_ID.to_string(),
        plan_id: PLAN_ID.to_string(),
        title: "实现 Revision Store".to_string(),
        active_revision_id: None,
        created_at: "2026-07-17T00:00:00Z".to_string(),
        updated_at: "2026-07-17T00:00:00Z".to_string(),
    }
}

fn draft_revision() -> WorkItemDraftRevision {
    WorkItemDraftRevision {
        id: "work_item_draft_revision_0001".to_string(),
        logical_work_item_id: WORK_ITEM_ID.to_string(),
        revision_no: 1,
        supersedes: None,
        revision_reason: PlanRevisionReason::InitialCompile,
        canonical_contract_candidate: canonical_contract_fixture(WORK_ITEM_ID),
        trigger_repair_request_id: None,
        created_at: "2026-07-17T00:00:01Z".to_string(),
    }
}

fn work_item_revision() -> WorkItemRevision {
    WorkItemRevision {
        id: "work_item_revision_0001".to_string(),
        logical_work_item_id: WORK_ITEM_ID.to_string(),
        source_draft_revision_id: "work_item_draft_revision_0001".to_string(),
        canonical_contract: canonical_contract_fixture(WORK_ITEM_ID),
        canonical_contract_hash: "contract_hash_0001".to_string(),
        work_item_projection_bundle_id: "work_item_projection_bundle_0001".to_string(),
        verification_plan_revision_id: "verification_plan_revision_0001".to_string(),
        created_at: "2026-07-17T00:00:02Z".to_string(),
    }
}

fn verification_plan_revision() -> VerificationPlanRevision {
    VerificationPlanRevision {
        id: "verification_plan_revision_0001".to_string(),
        logical_work_item_id: WORK_ITEM_ID.to_string(),
        source_draft_revision_id: "work_item_draft_revision_0001".to_string(),
        verification_checks: canonical_contract_fixture(WORK_ITEM_ID).verification_checks,
        created_at: "2026-07-17T00:00:03Z".to_string(),
    }
}

fn repair_request(id: &str) -> PlanRepairRequest {
    PlanRepairRequest {
        id: id.to_string(),
        plan_id: PLAN_ID.to_string(),
        base_plan_revision_id: "plan_revision_0001".to_string(),
        trigger_attempt_id: "coding_attempt_0001".to_string(),
        trigger_unit_run_id: "unit_run_0001".to_string(),
        trigger_review_id: Some("review_0001".to_string()),
        trigger_finding_id: "finding_0001".to_string(),
        amendment_id: Some("amendment_0001".to_string()),
        defect_class: PlanDefectClass::CurrentWorkItemInvalid,
        reason_code: "contract_invalid".to_string(),
        repair_target: RepairTarget {
            kind: RepairTargetKind::CurrentWorkItem,
            logical_work_item_ids: vec![WORK_ITEM_ID.to_string()],
            work_item_revision_ids: vec!["work_item_revision_0001".to_string()],
        },
        contract_refs: vec!["contract_0001".to_string()],
        capability_refs: vec!["capability_0001".to_string()],
        evidence: vec![PlanDefectEvidence {
            kind: "review_finding".to_string(),
            source_ref: "review_0001#finding_0001".to_string(),
            message: "contract mismatch".to_string(),
        }],
        fingerprint: format!("fingerprint_{id}"),
        status: PlanRepairRequestStatus::Open,
        created_at: "2026-07-17T00:00:04Z".to_string(),
        updated_at: "2026-07-17T00:00:04Z".to_string(),
    }
}

#[test]
fn work_item_revision_store_rejects_overwriting_revision_with_different_content() {
    let (_temp, store, plan) = test_store_and_plan();
    let revision = plan_revision("plan_revision_0001", 1);

    store.put_plan_revision(&plan, &revision).unwrap();
    store.put_plan_revision(&plan, &revision).unwrap();

    let mut changed = revision;
    changed.reason = PlanRevisionReason::SubgraphReplan;
    let error = store.put_plan_revision(&plan, &changed).unwrap_err();

    assert!(matches!(error, ProductStoreError::IdentityMismatch { .. }));
}

#[test]
fn work_item_revision_store_never_resolves_revision_outside_issue_scope() {
    let (_temp, store, plan) = test_store_and_plan();
    store
        .put_plan_revision(&plan, &plan_revision("plan_revision_0001", 1))
        .unwrap();

    let error = store
        .get_plan_revision(PROJECT_ID, "issue_other", &plan.id, "plan_revision_0001")
        .unwrap_err();

    assert!(matches!(error, ProductStoreError::NotFound { .. }));
}

#[test]
fn work_item_revision_store_keeps_duplicate_plan_ids_isolated_by_issue_scope() {
    let temp = TempDir::new().unwrap();
    let store = WorkItemRevisionStore::new(ProductAppPaths::new(temp.path().join(".aria")));
    let first_plan = plan_lineage();
    let mut second_plan = first_plan.clone();
    second_plan.issue_id = "issue_0002".to_string();
    store.put_plan_lineage(&first_plan).unwrap();
    store.put_plan_lineage(&second_plan).unwrap();

    let first_revision = plan_revision("plan_revision_0001", 1);
    let mut second_revision = first_revision.clone();
    second_revision.reason = PlanRevisionReason::SubgraphReplan;
    store
        .put_plan_revision(&first_plan, &first_revision)
        .unwrap();
    store
        .put_plan_revision(&second_plan, &second_revision)
        .unwrap();

    assert_eq!(
        store
            .get_plan_revision(PROJECT_ID, ISSUE_ID, PLAN_ID, &first_revision.id)
            .unwrap(),
        first_revision
    );
    assert_eq!(
        store
            .get_plan_revision(
                PROJECT_ID,
                &second_plan.issue_id,
                PLAN_ID,
                &second_revision.id
            )
            .unwrap(),
        second_revision
    );
}

#[test]
fn work_item_revision_store_scopes_duplicate_logical_and_revision_ids_by_issue() {
    let temp = TempDir::new().unwrap();
    let store = WorkItemRevisionStore::new(ProductAppPaths::new(temp.path().join(".aria")));
    let first_plan = plan_lineage();
    let mut second_plan = first_plan.clone();
    second_plan.issue_id = "issue_0002".to_string();
    store.put_plan_lineage(&first_plan).unwrap();
    store.put_plan_lineage(&second_plan).unwrap();

    let first_logical = logical_work_item();
    let mut second_logical = first_logical.clone();
    second_logical.title = "Issue 2 独立 Work Item".to_string();
    store
        .put_logical_work_item(&first_plan, &first_logical)
        .unwrap();
    store
        .put_logical_work_item(&second_plan, &second_logical)
        .unwrap();

    let first_revision = work_item_revision();
    let mut second_revision = first_revision.clone();
    second_revision.canonical_contract.identity.title = "Issue 2 contract".to_string();
    second_revision.canonical_contract_hash = "contract_hash_issue_0002".to_string();
    store
        .put_work_item_revision(&first_plan, &first_revision)
        .unwrap();
    store
        .put_work_item_revision(&second_plan, &second_revision)
        .unwrap();

    let first_active = store
        .set_active_work_item_revision(&first_plan, &first_logical, None, &first_revision.id)
        .unwrap();
    let second_active = store
        .set_active_work_item_revision(&second_plan, &second_logical, None, &second_revision.id)
        .unwrap();

    assert_eq!(
        first_active.active_revision_id.as_deref(),
        Some(first_revision.id.as_str())
    );
    assert_eq!(
        second_active.active_revision_id.as_deref(),
        Some(second_revision.id.as_str())
    );
    assert_eq!(
        store
            .get_work_item_revision(&first_plan, WORK_ITEM_ID, &first_revision.id)
            .unwrap(),
        first_revision
    );
    assert_eq!(
        store
            .get_work_item_revision(&second_plan, WORK_ITEM_ID, &second_revision.id)
            .unwrap(),
        second_revision
    );
}

#[test]
fn work_item_revision_store_rejects_orphan_revision_without_scoped_lineage() {
    let temp = TempDir::new().unwrap();
    let store = WorkItemRevisionStore::new(ProductAppPaths::new(temp.path().join(".aria")));
    let revision = plan_revision("plan_revision_0001", 1);
    write_json(
        &store.plan_revision_path(PROJECT_ID, ISSUE_ID, PLAN_ID, &revision.id),
        &revision,
    )
    .unwrap();

    let error = store
        .get_plan_revision(PROJECT_ID, ISSUE_ID, PLAN_ID, &revision.id)
        .unwrap_err();

    assert!(matches!(error, ProductStoreError::NotFound { .. }));
}

#[test]
fn work_item_revision_store_validates_scope_and_payload_identity() {
    let (_temp, store, plan) = test_store_and_plan();
    let revision = plan_revision("plan_revision_0001", 1);
    store.put_plan_revision(&plan, &revision).unwrap();

    let mut corrupted = revision;
    corrupted.plan_id = "work_item_plan_other".to_string();
    write_json(
        &store.plan_revision_path(PROJECT_ID, ISSUE_ID, PLAN_ID, &corrupted.id),
        &corrupted,
    )
    .unwrap();

    let error = store
        .get_plan_revision(PROJECT_ID, ISSUE_ID, PLAN_ID, &corrupted.id)
        .unwrap_err();
    assert!(matches!(error, ProductStoreError::IdentityMismatch { .. }));

    let error = store
        .get_plan_lineage(PROJECT_ID, "issue_other", PLAN_ID)
        .unwrap_err();
    assert!(matches!(error, ProductStoreError::NotFound { .. }));
}

#[test]
fn work_item_revision_store_updates_active_plan_revision_with_compare_and_set() {
    let (_temp, store, plan) = test_store_and_plan();
    let first = plan_revision("plan_revision_0001", 1);
    let second = plan_revision("plan_revision_0002", 2);
    store.put_plan_revision(&plan, &first).unwrap();
    store.put_plan_revision(&plan, &second).unwrap();

    let active = store.set_active_plan_revision(&plan, &first.id).unwrap();
    assert_eq!(
        active.active_revision_id.as_deref(),
        Some(first.id.as_str())
    );

    let error = store
        .compare_and_set_active_plan_revision(&plan, "plan_revision_stale", &second.id)
        .unwrap_err();
    assert!(matches!(error, ProductStoreError::IdentityMismatch { .. }));

    let active = store
        .compare_and_set_active_plan_revision(&plan, &first.id, &second.id)
        .unwrap();
    assert_eq!(
        active.active_revision_id.as_deref(),
        Some(second.id.as_str())
    );
}

#[test]
fn work_item_revision_store_allows_only_one_active_amendment_per_plan() {
    let (_temp, store, plan) = test_store_and_plan();

    let acquired = store
        .acquire_active_amendment(&plan, "amendment_0001")
        .unwrap();
    assert_eq!(
        acquired.active_amendment_id.as_deref(),
        Some("amendment_0001")
    );
    store
        .acquire_active_amendment(&plan, "amendment_0001")
        .unwrap();

    let error = store
        .acquire_active_amendment(&plan, "amendment_0002")
        .unwrap_err();
    assert!(matches!(error, ProductStoreError::IdentityMismatch { .. }));

    let error = store
        .release_active_amendment(&plan, "amendment_0002")
        .unwrap_err();
    assert!(matches!(error, ProductStoreError::IdentityMismatch { .. }));

    let released = store
        .release_active_amendment(&plan, "amendment_0001")
        .unwrap();
    assert_eq!(released.active_amendment_id, None);
}

#[test]
fn work_item_revision_store_persists_work_item_revisions_and_mutable_state() {
    let (_temp, store, plan) = test_store_and_plan();
    let logical = logical_work_item();
    store.put_logical_work_item(&plan, &logical).unwrap();
    store.put_logical_work_item(&plan, &logical).unwrap();

    let mut changed_logical = logical.clone();
    changed_logical.title = "不允许覆盖".to_string();
    let error = store
        .put_logical_work_item(&plan, &changed_logical)
        .unwrap_err();
    assert!(matches!(error, ProductStoreError::IdentityMismatch { .. }));

    let draft = draft_revision();
    store.put_draft_revision(&plan, &draft).unwrap();
    store.put_draft_revision(&plan, &draft).unwrap();
    let state = store
        .update_draft_revision_state(&plan, &draft.id, WorkItemDraftRevisionStatus::Approved)
        .unwrap();
    assert_eq!(state.draft_revision_id, draft.id);
    assert_eq!(state.status, WorkItemDraftRevisionStatus::Approved);

    let revision = work_item_revision();
    store.put_work_item_revision(&plan, &revision).unwrap();
    assert_eq!(
        store
            .get_work_item_revision(&plan, WORK_ITEM_ID, &revision.id)
            .unwrap(),
        revision
    );

    let active = store
        .set_active_work_item_revision(&plan, &logical, None, &revision.id)
        .unwrap();
    assert_eq!(
        active.active_revision_id.as_deref(),
        Some(revision.id.as_str())
    );
    let error = store
        .set_active_work_item_revision(&plan, &logical, None, &revision.id)
        .unwrap_err();
    assert!(matches!(error, ProductStoreError::IdentityMismatch { .. }));

    let verification = verification_plan_revision();
    store
        .put_verification_plan_revision(&plan, &verification)
        .unwrap();
    assert_eq!(
        store
            .get_verification_plan_revision(&plan, &verification.id)
            .unwrap(),
        verification
    );
}

#[test]
fn plan_repair_store_updates_request_status_and_merges_evidence() {
    let (_temp, store, plan) = test_store_and_plan();
    let request = repair_request("plan_repair_request_0001");
    store.put_repair_request(&plan, &request).unwrap();
    store.put_repair_request(&plan, &request).unwrap();

    let updated = store
        .update_repair_request_status(&plan, &request.id, PlanRepairRequestStatus::InProgress)
        .unwrap();
    assert_eq!(updated.status, PlanRepairRequestStatus::InProgress);

    let extra = PlanDefectEvidence {
        kind: "test_failure".to_string(),
        source_ref: "test_0001".to_string(),
        message: "test failed".to_string(),
    };
    let merged = store
        .merge_repair_request_evidence(&plan, &request.id, vec![extra.clone(), extra.clone()])
        .unwrap();
    assert_eq!(
        merged.evidence,
        vec![request.evidence[0].clone(), extra.clone()]
    );
    assert_eq!(merged.base_plan_revision_id, request.base_plan_revision_id);
    assert_eq!(merged.defect_class, request.defect_class);
    assert_eq!(merged.reason_code, request.reason_code);
    assert_eq!(merged.repair_target, request.repair_target);
    assert_eq!(merged.contract_refs, request.contract_refs);
    assert_eq!(merged.capability_refs, request.capability_refs);
    assert_eq!(merged.fingerprint, request.fingerprint);

    let replayed = store
        .merge_repair_request_evidence(&plan, &request.id, vec![request.evidence[0].clone(), extra])
        .unwrap();
    assert_eq!(replayed.evidence, merged.evidence);
    assert_eq!(store.list_open_repair_requests(&plan).unwrap().len(), 1);

    store
        .update_repair_request_status(&plan, &request.id, PlanRepairRequestStatus::Applied)
        .unwrap();
    assert!(store.list_open_repair_requests(&plan).unwrap().is_empty());
}

#[test]
fn plan_repair_evidence_merge_is_scoped_to_full_plan_lineage() {
    let temp = TempDir::new().unwrap();
    let store = WorkItemRevisionStore::new(ProductAppPaths::new(temp.path().join(".aria")));
    let first_plan = plan_lineage();
    let mut second_plan = plan_lineage();
    second_plan.issue_id = "issue_0002".to_string();
    store.put_plan_lineage(&first_plan).unwrap();
    store.put_plan_lineage(&second_plan).unwrap();

    let first_request = repair_request("plan_repair_request_shared");
    let mut second_request = first_request.clone();
    second_request.evidence[0].source_ref = "review_0002#finding_0001".to_string();
    second_request.fingerprint = "fingerprint_second_issue".to_string();
    store
        .put_repair_request(&first_plan, &first_request)
        .unwrap();
    store
        .put_repair_request(&second_plan, &second_request)
        .unwrap();

    let extra = PlanDefectEvidence {
        kind: "test_failure".to_string(),
        source_ref: "test_0001".to_string(),
        message: "test failed".to_string(),
    };
    store
        .merge_repair_request_evidence(&first_plan, &first_request.id, vec![extra.clone()])
        .unwrap();

    let first_stored = store
        .list_open_repair_requests(&first_plan)
        .unwrap()
        .remove(0);
    let second_stored = store
        .list_open_repair_requests(&second_plan)
        .unwrap()
        .remove(0);
    assert!(first_stored.evidence.contains(&extra));
    assert!(!second_stored.evidence.contains(&extra));
    assert_eq!(second_stored, second_request);
}

#[test]
fn work_item_revision_store_persists_amendment_artifacts_immutably() {
    let (_temp, store, plan) = test_store_and_plan();
    let request = repair_request("plan_repair_request_0001");
    store.put_repair_request(&plan, &request).unwrap();

    let manifest = PlanAmendmentManifest {
        id: "amendment_0001".to_string(),
        repair_request_id: request.id,
        previous_plan_revision_id: "plan_revision_0001".to_string(),
        new_plan_revision_id: "plan_revision_0002".to_string(),
        revised_work_items: BTreeMap::from([(
            WORK_ITEM_ID.to_string(),
            WorkItemRevisionReplacement {
                previous_revision_id: "work_item_revision_0001".to_string(),
                next_revision_id: "work_item_revision_0002".to_string(),
                delta_kind: crate::product::models::ContractDeltaKind::BreakingContractChange,
            },
        )]),
        superseded_revisions: vec!["work_item_revision_0001".to_string()],
        dependency_graph_changes: vec![crate::product::models::DependencyGraphChange {
            kind: crate::product::models::DependencyGraphChangeKind::EdgeReplaced,
            previous: None,
            next: None,
        }],
        contract_deltas: vec![crate::product::plan_repair::ContractDelta {
            logical_work_item_id: WORK_ITEM_ID.to_string(),
            previous_revision_id: "work_item_revision_0001".to_string(),
            next_revision_id: "work_item_revision_0002".to_string(),
            kind: crate::product::models::ContractDeltaKind::BreakingContractChange,
            added_contracts: vec![],
            removed_contracts: vec!["contract_0001".to_string()],
            added_capabilities: vec![],
            removed_capabilities: vec!["capability_0001".to_string()],
            changed_capabilities: vec![],
            added_capability_associations: vec![],
            removed_capability_associations: vec![],
            acceptance_changed: false,
            verification_changed: false,
            write_policy_changed: false,
        }],
        unaffected_units: vec![],
        revalidation_required_units: vec![WORK_ITEM_ID.to_string()],
        stale_units: vec![],
        replacement_units: BTreeMap::from([(
            WORK_ITEM_ID.to_string(),
            vec!["logical_work_item_0002".to_string()],
        )]),
        resume_target: AmendmentResumeTarget {
            logical_work_item_id: WORK_ITEM_ID.to_string(),
            mode: AmendmentResumeMode::Revalidate,
        },
        created_at: "2026-07-17T00:00:12Z".to_string(),
    };
    store.put_amendment_manifest(&plan, &manifest).unwrap();
    store.put_amendment_manifest(&plan, &manifest).unwrap();
    assert_eq!(
        store.get_amendment_manifest(&plan, &manifest.id).unwrap(),
        manifest
    );

    let journal = PlanAmendmentPublicationJournal {
        id: "amendment_publication_journal_0001".to_string(),
        project_id: PROJECT_ID.to_string(),
        issue_id: ISSUE_ID.to_string(),
        plan_id: PLAN_ID.to_string(),
        amendment_id: manifest.id,
        request_id: "plan_repair_request_0001".to_string(),
        base_plan_revision_id: "plan_revision_0001".to_string(),
        new_plan_revision_id: "plan_revision_0002".to_string(),
        confirmation: None,
        artifact_fingerprint: "fingerprint_0001".to_string(),
        snapshot: None,
        phase: PlanAmendmentPublicationPhase::Prepared,
        error: None,
        recovery: None,
        created_at: "2026-07-17T00:00:13Z".to_string(),
        updated_at: "2026-07-17T00:00:13Z".to_string(),
    };
    store
        .put_plan_amendment_publication_journal(&plan, &journal)
        .unwrap();
    store
        .put_plan_amendment_publication_journal(&plan, &journal)
        .unwrap();

    let mut changed = journal;
    changed.phase = PlanAmendmentPublicationPhase::PlanPublished;
    let error = store
        .put_plan_amendment_publication_journal(&plan, &changed)
        .unwrap_err();
    assert!(matches!(error, ProductStoreError::IdentityMismatch { .. }));
}

#[test]
fn work_item_revision_store_rejects_unknown_or_escaping_scope() {
    let (_temp, store, plan) = test_store_and_plan();
    let mut unknown = logical_work_item();
    unknown.plan_id = "work_item_plan_unknown".to_string();
    let error = store.put_logical_work_item(&plan, &unknown).unwrap_err();
    assert!(matches!(error, ProductStoreError::IdentityMismatch { .. }));

    let error = store
        .get_plan_lineage("..", ISSUE_ID, &plan.id)
        .unwrap_err();
    assert!(matches!(error, ProductStoreError::PathEscape(_)));
}

fn assert_amendment_operation_rejects_escaping_scope_without_side_effects(acquire: bool) {
    for scope in ["project", "issue", "plan"] {
        let temp = TempDir::new().unwrap();
        let aria_root = temp.path().join(".aria");
        let store = WorkItemRevisionStore::new(ProductAppPaths::new(&aria_root));
        let mut plan = plan_lineage();
        match scope {
            "project" => plan.project_id = "../escaped-project".to_string(),
            "issue" => plan.issue_id = "../escaped-issue".to_string(),
            "plan" => plan.id = "../escaped-plan".to_string(),
            _ => unreachable!(),
        }
        let target_path = store.plan_lineage_path(&plan.project_id, &plan.issue_id, &plan.id);
        let lock_path = super::lock_path_for(&target_path);

        let error = if acquire {
            store
                .acquire_active_amendment(&plan, "amendment_0001")
                .unwrap_err()
        } else {
            store
                .release_active_amendment(&plan, "amendment_0001")
                .unwrap_err()
        };

        assert!(matches!(error, ProductStoreError::PathEscape(_)));
        assert!(!aria_root.exists(), "invalid {scope} created .aria");
        assert!(!target_path.parent().unwrap().exists());
        assert!(!lock_path.exists());
    }
}

#[test]
fn work_item_revision_store_acquire_amendment_validates_scope_before_lock_path() {
    assert_amendment_operation_rejects_escaping_scope_without_side_effects(true);
}

#[test]
fn work_item_revision_store_release_amendment_validates_scope_before_lock_path() {
    assert_amendment_operation_rejects_escaping_scope_without_side_effects(false);
}

#[test]
fn work_item_revision_store_latest_presentation_compares_rfc3339_instants() {
    let (_temp, store, plan) = test_store_and_plan();
    let source_id = "plan_projection_bundle_0001";
    let presentations = [
        ("human_presentation_revision_z", "2026-07-17T04:00:00Z"),
        (
            "human_presentation_revision_positive",
            "2026-07-17T08:00:00+05:00",
        ),
        (
            "human_presentation_revision_negative",
            "2026-07-17T00:30:00-04:00",
        ),
    ]
    .map(|(id, created_at)| HumanPresentationRevision {
        id: id.to_string(),
        source_plan_projection_bundle_id: Some(source_id.to_string()),
        source_work_item_projection_bundle_id: None,
        supersedes: None,
        human_summary: id.to_string(),
        why_split: None,
        dependency_explanation: vec![],
        risk_explanation: vec![],
        source_refs: vec![],
        normative: false,
        used_by_provider: false,
        created_at: created_at.to_string(),
    });
    for presentation in &presentations {
        store
            .put_human_presentation_revision(&plan, presentation)
            .unwrap();
    }

    let latest = store
        .get_latest_human_presentation_revision(&plan, source_id)
        .unwrap()
        .unwrap();

    assert_eq!(latest.id, "human_presentation_revision_negative");
}

#[test]
fn purge_plan_revisions_removes_revision_and_publication_dirs() {
    let temp = TempDir::new().unwrap();
    let store = WorkItemRevisionStore::new(ProductAppPaths::new(temp.path().join(".aria")));
    let issue_root = temp
        .path()
        .join(".aria")
        .join("projects")
        .join(PROJECT_ID)
        .join("issues")
        .join(ISSUE_ID);
    let plan_root = issue_root.join("work-item-revisions").join(PLAN_ID);
    std::fs::create_dir_all(plan_root.join("plan-revisions")).unwrap();
    std::fs::write(plan_root.join("lineage.json"), "{}").unwrap();
    let publications = issue_root
        .join("work-item-revision-publications")
        .join(PLAN_ID);
    std::fs::create_dir_all(&publications).unwrap();
    std::fs::write(publications.join("compile_0001.json"), "{}").unwrap();

    store
        .purge_plan_revisions(PROJECT_ID, ISSUE_ID, PLAN_ID)
        .unwrap();

    assert!(!plan_root.exists(), "plan_root 目录应被删除");
    assert!(!publications.exists(), "publications 目录应被删除");
}

#[test]
fn purge_plan_revisions_succeeds_when_dirs_absent() {
    let temp = TempDir::new().unwrap();
    let store = WorkItemRevisionStore::new(ProductAppPaths::new(temp.path().join(".aria")));
    // 不播种任何产物；NotFound 视为成功
    store
        .purge_plan_revisions(PROJECT_ID, ISSUE_ID, PLAN_ID)
        .unwrap();
}
