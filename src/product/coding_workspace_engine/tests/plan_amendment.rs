use std::collections::BTreeMap;
use std::sync::Arc;

use tempfile::TempDir;

use super::*;
use crate::product::checkpoint_store::CheckpointStore;
use crate::product::coding_models::{
    CodingAmendmentApplicationPhase, CodingUnitRun, CodingUnitRunStatus,
};
use crate::product::lifecycle_store::CreateWorkspaceSessionInput;
use crate::product::models::{
    AmendmentResumeMode, AmendmentResumeTarget, ContractDeltaKind, PlanAmendmentManifest,
    PlanDefectClass, PlanDefectEvidence, PlanRepairRequest, PlanRepairRequestStatus,
    PlanRepairSessionStage, RepairTarget, RepairTargetKind, WorkItemRevisionReplacement,
    WorkspaceSessionStatus, WorkspaceType,
};
use crate::product::work_item_revision_store::register_repair_request_status_failpoint;
use crate::product::workspace_engine::{EngineEvent, WorkspaceEngine, WorkspaceSession};

mod identity;
mod recovery;
mod review_fix_async_lock;
mod review_fix_delivery;
mod review_fix_identity;
mod review_fix_replay;
mod review_fix_unit_runs;
mod support;
use support::{prepare_application_phase, seed_unit_run};

struct AmendmentFixture {
    _root: TempDir,
    store: CodingAttemptStore,
    revision_store: WorkItemRevisionStore,
    attempt: CodingExecutionAttempt,
    plan: WorkItemPlanLineage,
    manifest: PlanAmendmentManifest,
    child_session_id: String,
    engine: CodingWorkspaceEngine,
    _event_rx: mpsc::Receiver<CodingWsOutMessage>,
}

#[tokio::test]
async fn revalidate_amendment_resumes_at_code_review_with_revalidation_status() {
    let fixture = amendment_fixture_with_resume_mode(AmendmentResumeMode::Revalidate).await;

    let resumed = fixture
        .engine
        .apply_plan_amendment(&fixture.attempt, &fixture.manifest)
        .await
        .expect("apply revalidation amendment");

    assert_eq!(resumed.stage, CodingExecutionStage::CodeReview);
    let active_unit = fixture
        .store
        .get_active_coding_unit(&resumed.project_id, &resumed.issue_id, &resumed.id)
        .unwrap()
        .expect("revalidation amendment must retain an active unit");
    assert_eq!(
        active_unit.status,
        CodingExecutionUnitStatus::NeedsRevalidation
    );
    assert_eq!(
        fixture
            .store
            .list_unit_runs_by_logical_id(&resumed, &active_unit.logical_work_item_id)
            .unwrap()
            .last()
            .expect("revalidation unit run must exist")
            .status,
        CodingUnitRunStatus::NeedsRevalidation
    );
}

#[tokio::test]
async fn coding_amendment_reexecutes_only_manifest_affected_units() {
    let fixture = amendment_fixture().await;
    let rework_count = fixture.attempt.rework_count;
    let old_completed = fixture
        .store
        .list_unit_runs_by_logical_id(&fixture.attempt, "work_item_0001")
        .unwrap()
        .remove(0);

    let updated = fixture
        .engine
        .apply_plan_amendment(&fixture.attempt, &fixture.manifest)
        .await
        .expect("apply canonical amendment");

    assert_eq!(updated.status, CodingAttemptStatus::Running);
    assert_eq!(updated.stage, CodingExecutionStage::Coding);
    assert_eq!(
        updated.current_work_item_id.as_deref(),
        Some("work_item_0001")
    );
    assert_eq!(updated.rework_count, rework_count);
    let binding = fixture.store.get_plan_binding(&updated).unwrap();
    assert_eq!(binding.bound_plan_revision_id, "plan_revision_0002");
    assert_eq!(
        binding.applied_amendment_ids,
        vec![fixture.manifest.id.clone()]
    );
    assert_eq!(
        fixture
            .store
            .validate_group_attempt_integrity(&updated)
            .expect("amended group attempt must resolve its new revision binding")
            .plan_revision_id,
        "plan_revision_0002"
    );

    let revised_runs = fixture
        .store
        .list_unit_runs_by_logical_id(&updated, "work_item_0001")
        .unwrap();
    assert_eq!(revised_runs.len(), 2);
    assert_eq!(revised_runs[0], old_completed);
    assert_eq!(revised_runs[0].status, CodingUnitRunStatus::Completed);
    assert_eq!(
        revised_runs[1].work_item_revision_id,
        "work_item_revision_0101"
    );
    assert_eq!(revised_runs[1].status, CodingUnitRunStatus::Running);
    assert_eq!(revised_runs[1].unit_rework_count, 0);
    assert_eq!(revised_runs[1].verification_retry_count, 0);
    assert_eq!(revised_runs[1].operational_retry_count, 0);

    let revalidation_runs = fixture
        .store
        .list_unit_runs_by_logical_id(&updated, "work_item_0002")
        .unwrap();
    assert_eq!(revalidation_runs.len(), 2);
    assert_eq!(revalidation_runs[0].status, CodingUnitRunStatus::Superseded);
    assert_eq!(
        revalidation_runs[1].status,
        CodingUnitRunStatus::NeedsRevalidation
    );
    assert!(
        fixture
            .store
            .list_unit_runs_by_logical_id(&updated, "work_item_0003")
            .unwrap()
            .is_empty()
    );
    let plan = fixture
        .revision_store
        .get_plan_lineage(&updated.project_id, &updated.issue_id, &fixture.plan.id)
        .unwrap();
    assert_eq!(plan.active_amendment_id, None);
    assert_eq!(
        fixture
            .revision_store
            .get_repair_request(&plan, &fixture.manifest.repair_request_id)
            .unwrap()
            .status,
        PlanRepairRequestStatus::Applied
    );

    let replayed = fixture
        .engine
        .apply_plan_amendment(&updated, &fixture.manifest)
        .await
        .expect("replay completed amendment");
    assert_eq!(replayed.status, CodingAttemptStatus::Running);
    assert_eq!(
        fixture
            .store
            .list_unit_runs_by_logical_id(&replayed, "work_item_0001")
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        fixture
            .store
            .list_unit_runs_by_logical_id(&replayed, "work_item_0002")
            .unwrap()
            .len(),
        2
    );
}

#[tokio::test]
async fn group_validation_keeps_the_bound_plan_revision_after_plan_repair_publishes() {
    let fixture = amendment_fixture().await;

    let authoritative = fixture
        .store
        .validate_group_attempt_integrity(&fixture.attempt)
        .expect("existing attempt must keep its original plan binding");

    assert_eq!(authoritative.plan_revision_id, "plan_revision_0001");
    assert_eq!(
        authoritative
            .units
            .iter()
            .map(|unit| unit.work_item_revision_id.as_str())
            .collect::<Vec<_>>(),
        vec![
            "work_item_revision_0001",
            "work_item_revision_0002",
            "work_item_revision_0003",
        ]
    );
}

#[tokio::test]
async fn coding_amendment_completed_journal_reconciles_unfinished_finalization() {
    let fixture = amendment_fixture().await;
    let applied = fixture
        .engine
        .apply_plan_amendment(&fixture.attempt, &fixture.manifest)
        .await
        .unwrap();
    reset_completed_application_to_unfinished_finalization(&fixture, &applied);

    let recovered = fixture
        .engine
        .recover_plan_amendment(&applied)
        .await
        .expect("recover completed application finalization");

    assert_eq!(recovered.status, CodingAttemptStatus::Running);
    let journal = fixture
        .store
        .get_amendment_application_journal(&recovered, &fixture.manifest.id)
        .unwrap();
    assert_eq!(journal.phase, CodingAmendmentApplicationPhase::Completed);
    assert_eq!(journal.error, None);
    let plan = fixture
        .revision_store
        .get_plan_lineage(&recovered.project_id, &recovered.issue_id, &fixture.plan.id)
        .unwrap();
    assert_eq!(plan.active_amendment_id, None);
    assert_eq!(
        fixture
            .revision_store
            .get_repair_request(&plan, &fixture.manifest.repair_request_id)
            .unwrap()
            .status,
        PlanRepairRequestStatus::Applied
    );
    let lifecycle = LifecycleStore::new(fixture.store.paths());
    let snapshot = lifecycle
        .load_plan_repair_session_state(
            &recovered.project_id,
            &recovered.issue_id,
            &fixture.child_session_id,
        )
        .unwrap()
        .unwrap();
    assert_eq!(snapshot.stage, PlanRepairSessionStage::Completed);
    assert_eq!(
        lifecycle
            .get_workspace_session(&fixture.child_session_id)
            .unwrap()
            .status,
        WorkspaceSessionStatus::Terminated
    );
    assert_eq!(
        fixture
            .store
            .list_unit_runs_by_logical_id(&recovered, "work_item_0001")
            .unwrap()
            .len(),
        2
    );
}

#[tokio::test]
async fn coding_amendment_recovers_every_durable_phase_without_duplicate_runs() {
    for phase in [
        CodingAmendmentApplicationPhase::Started,
        CodingAmendmentApplicationPhase::PlanBindingWritten,
        CodingAmendmentApplicationPhase::UnitRunsWritten,
        CodingAmendmentApplicationPhase::ResumeTargetWritten,
    ] {
        let fixture = amendment_fixture().await;
        prepare_application_phase(&fixture, phase.clone());

        let recovered = fixture
            .engine
            .recover_plan_amendment(&fixture.attempt)
            .await
            .unwrap_or_else(|error| panic!("recover {phase:?}: {error}"));

        assert_eq!(recovered.status, CodingAttemptStatus::Running, "{phase:?}");
        assert_eq!(
            fixture
                .store
                .list_unit_runs_by_logical_id(&recovered, "work_item_0001")
                .unwrap()
                .len(),
            2,
            "{phase:?}"
        );
        assert_eq!(
            fixture
                .store
                .list_unit_runs_by_logical_id(&recovered, "work_item_0002")
                .unwrap()
                .len(),
            2,
            "{phase:?}"
        );
    }
}

#[tokio::test]
async fn coding_amendment_identity_mismatch_is_zero_write() {
    let fixture = amendment_fixture().await;
    let binding_before = fixture.store.get_plan_binding(&fixture.attempt).unwrap();
    let mut forged = fixture.manifest.clone();
    forged.repair_request_id = "plan_repair_request_forged".to_string();

    let error = fixture
        .engine
        .apply_plan_amendment(&fixture.attempt, &forged)
        .await
        .expect_err("forged canonical identity must fail closed");

    assert!(error.to_string().contains("identity_mismatch"));
    assert_eq!(
        fixture
            .store
            .list_amendment_application_journals(&fixture.attempt)
            .unwrap(),
        Vec::new()
    );
    assert_eq!(
        fixture.store.get_plan_binding(&fixture.attempt).unwrap(),
        binding_before
    );
    assert_eq!(
        fixture
            .store
            .get_attempt(
                &fixture.attempt.project_id,
                &fixture.attempt.issue_id,
                &fixture.attempt.id,
            )
            .unwrap()
            .status,
        CodingAttemptStatus::AwaitingPlanAmendment
    );
}

#[tokio::test]
async fn coding_amendment_dirty_worktree_creates_gate_without_starting_journal() {
    let fixture = amendment_fixture().await;
    std::fs::write(
        fixture
            .attempt
            .worktree_path
            .as_ref()
            .unwrap()
            .join("dirty.txt"),
        "uncommitted amendment blocker",
    )
    .unwrap();

    let error = fixture
        .engine
        .apply_plan_amendment(&fixture.attempt, &fixture.manifest)
        .await
        .expect_err("dirty worktree must block application");

    assert!(
        error
            .to_string()
            .contains("worktree_dirty_before_plan_amendment")
    );
    let gates = fixture
        .store
        .list_open_blocked_gates(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .unwrap();
    assert_eq!(
        gates
            .iter()
            .filter(|gate| {
                gate.reason_code.as_deref() == Some("worktree_dirty_before_plan_amendment")
            })
            .count(),
        1
    );
    assert!(
        fixture
            .store
            .list_amendment_application_journals(&fixture.attempt)
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn coding_amendment_failure_keeps_last_phase_error_and_provider_gate() {
    let fixture = amendment_fixture().await;
    let missing_bundle = fixture
        .store
        .paths()
        .issue_root(&fixture.attempt.project_id, &fixture.attempt.issue_id)
        .join("work-item-revisions")
        .join(&fixture.plan.id)
        .join("work-item-projection-bundles/projection_bundle_0101.json");
    std::fs::remove_file(missing_bundle).unwrap();

    fixture
        .engine
        .apply_plan_amendment(&fixture.attempt, &fixture.manifest)
        .await
        .expect_err("materialization failure must be journaled");

    let failed = fixture
        .store
        .get_attempt(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .unwrap();
    assert_eq!(failed.status, CodingAttemptStatus::AmendmentApplyFailed);
    let journal = fixture
        .store
        .get_amendment_application_journal(&failed, &fixture.manifest.id)
        .unwrap();
    assert_eq!(
        journal.phase,
        CodingAmendmentApplicationPhase::PlanBindingWritten
    );
    assert!(journal.error.is_some());
    assert!(
        fixture
            .store
            .ensure_provider_run_allowed(&failed)
            .unwrap_err()
            .to_string()
            .contains("plan_amendment_blocks_provider_run")
    );
    let plan = fixture
        .revision_store
        .get_plan_lineage(&failed.project_id, &failed.issue_id, &fixture.plan.id)
        .unwrap();
    assert_eq!(
        plan.active_amendment_id.as_deref(),
        Some(fixture.manifest.id.as_str())
    );
}

#[tokio::test]
async fn coding_amendment_completed_finalization_failure_is_recoverable() {
    let fixture = amendment_fixture().await;
    let failpoint = register_repair_request_status_failpoint(
        &fixture.revision_store,
        &fixture.plan,
        &fixture.manifest.repair_request_id,
        PlanRepairRequestStatus::Applied,
    );

    fixture
        .engine
        .apply_plan_amendment(&fixture.attempt, &fixture.manifest)
        .await
        .expect_err("post-completion finalization failure must surface");
    let failed = fixture
        .store
        .get_attempt(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .unwrap();
    assert_eq!(failed.status, CodingAttemptStatus::AmendmentApplyFailed);
    let journal = fixture
        .store
        .get_amendment_application_journal(&failed, &fixture.manifest.id)
        .unwrap();
    assert_eq!(journal.phase, CodingAmendmentApplicationPhase::Completed);
    assert!(journal.error.is_some());
    drop(failpoint);

    let recovered = fixture
        .engine
        .recover_plan_amendment(&failed)
        .await
        .expect("recover finalization from durable Completed");

    assert_eq!(recovered.status, CodingAttemptStatus::Running);
    assert_eq!(
        fixture
            .store
            .get_amendment_application_journal(&recovered, &fixture.manifest.id)
            .unwrap()
            .error,
        None
    );
}

async fn amendment_fixture() -> AmendmentFixture {
    amendment_fixture_with_resume_mode(AmendmentResumeMode::Reexecute).await
}

async fn amendment_fixture_with_resume_mode(resume_mode: AmendmentResumeMode) -> AmendmentFixture {
    let root = TempDir::new().unwrap();
    let worktree = root.path().join("worktree");
    std::fs::create_dir_all(&worktree).unwrap();
    init_test_git_repo(&worktree);
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let store = CodingAttemptStore::new(paths.clone());
    let initial = store
        .create_group_attempt(CreateGroupCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            current_work_item_id: "work_item_0001".to_string(),
            base_branch: "main".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: Some(worktree),
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Codex,
                reviewer: Some(ProviderName::ClaudeCode),
                review_rounds: 1,
                permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
            },
            target_snapshot: None,
            max_auto_rework: 2,
        })
        .unwrap();
    seed_group_attempt_fixture(&store, &initial, true, false);
    let revision_store = WorkItemRevisionStore::new(paths.clone());
    let plan = revision_store
        .get_plan_lineage("project_0001", "issue_0001", "work_item_plan_0001")
        .unwrap();
    let mut attempt = store
        .update_attempt_status(
            &initial.project_id,
            &initial.issue_id,
            &initial.id,
            CodingAttemptStatus::Running,
        )
        .unwrap();
    let units = store
        .list_coding_units(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .unwrap();
    seed_unit_run(
        &store,
        &plan,
        &attempt,
        &units[0],
        "coding_unit_run_completed",
        CodingUnitRunStatus::Completed,
        Some("commit_completed"),
    );
    store
        .update_coding_unit_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &units[0].id,
            CodingExecutionUnitStatus::Completed,
            Some("completed before amendment".to_string()),
        )
        .unwrap();
    store
        .update_coding_unit_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &units[1].id,
            CodingExecutionUnitStatus::Running,
            None,
        )
        .unwrap();
    attempt = store
        .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .unwrap();
    seed_unit_run(
        &store,
        &plan,
        &attempt,
        &units[1],
        "coding_unit_run_blocked",
        CodingUnitRunStatus::Running,
        None,
    );
    attempt = store
        .update_attempt_stage(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingExecutionStage::CodeReview,
        )
        .unwrap();

    let lifecycle = LifecycleStore::new(paths.clone());
    let parent = lifecycle
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: attempt.project_id.clone(),
            issue_id: attempt.issue_id.clone(),
            entity_id: plan.id.clone(),
            workspace_type: WorkspaceType::WorkItemPlan,
            author_provider: ProviderName::Codex,
            reviewer_provider: ProviderName::ClaudeCode,
            review_rounds: 1,
            superpowers_enabled: true,
            openspec_enabled: true,
        })
        .unwrap();
    let request = PlanRepairRequest {
        id: "plan_repair_request_0001".to_string(),
        plan_id: plan.id.clone(),
        base_plan_revision_id: "plan_revision_0001".to_string(),
        trigger_attempt_id: attempt.id.clone(),
        trigger_unit_run_id: "coding_unit_run_blocked".to_string(),
        trigger_review_id: Some("code_review_report_0001".to_string()),
        trigger_finding_id: "code_review_report_0001_finding_0001".to_string(),
        amendment_id: None,
        defect_class: PlanDefectClass::UpstreamContractInvalid,
        reason_code: "upstream_contract_invalid".to_string(),
        repair_target: RepairTarget {
            kind: RepairTargetKind::UpstreamWorkItem,
            logical_work_item_ids: vec!["work_item_0001".to_string()],
            work_item_revision_ids: vec!["work_item_revision_0001".to_string()],
        },
        contract_refs: vec!["contract_work_item_0001".to_string()],
        capability_refs: vec!["capability_work_item_0001".to_string()],
        evidence: vec![PlanDefectEvidence {
            kind: "review".to_string(),
            source_ref: "code_review_report_0001".to_string(),
            message: "upstream contract needs repair".to_string(),
        }],
        fingerprint: "plan_repair_fingerprint_0001".to_string(),
        status: PlanRepairRequestStatus::Open,
        created_at: "2026-07-19T00:00:00Z".to_string(),
        updated_at: "2026-07-19T00:00:00Z".to_string(),
    };
    let (workspace_tx, _workspace_rx) = mpsc::channel::<EngineEvent>(8);
    let mut workspace_engine = WorkspaceEngine::new_persistent(
        Arc::new(CheckpointStore::new(root.path().join("checkpoints"))),
        lifecycle.clone(),
        workspace_tx,
        WorkspaceSession::from_record(parent),
    );
    let child = workspace_engine.start_plan_repair(request).await.unwrap();
    let reconciliation = store.reconcile_linked_plan_repair_pause(&attempt).unwrap();
    attempt = reconciliation.attempt;

    let old_revision = revision_store
        .get_work_item_revision(&plan, "work_item_0001", "work_item_revision_0001")
        .unwrap();
    let old_bundle = revision_store
        .get_work_item_projection_bundle(&plan, &old_revision.work_item_projection_bundle_id)
        .unwrap();
    let mut revised = old_revision.clone();
    revised.id = "work_item_revision_0101".to_string();
    revised.source_draft_revision_id = "draft_revision_0101".to_string();
    revised.work_item_projection_bundle_id = "projection_bundle_0101".to_string();
    revised.created_at = "2026-07-19T00:00:01Z".to_string();
    revision_store
        .put_work_item_revision(&plan, &revised)
        .unwrap();
    let mut revised_bundle = old_bundle;
    revised_bundle.id = revised.work_item_projection_bundle_id.clone();
    revised_bundle.work_item_revision_id = revised.id.clone();
    revised_bundle.coder_projection.work_item_revision_id = revised.id.clone();
    revised_bundle.reviewer_projection.work_item_revision_id = revised.id.clone();
    let revised_hashes = projection_hashes(
        &crate::product::work_item_projection::CompiledWorkItemProjections {
            human: revised_bundle.human_projection.clone(),
            coder: revised_bundle.coder_projection.clone(),
            reviewer: revised_bundle.reviewer_projection.clone(),
        },
    )
    .unwrap();
    revised_bundle.human_projection_hash = revised_hashes.human;
    revised_bundle.coder_projection_hash = revised_hashes.coder;
    revised_bundle.reviewer_projection_hash = revised_hashes.reviewer;
    revised_bundle.created_at = "2026-07-19T00:00:01Z".to_string();
    revision_store
        .put_work_item_projection_bundle(&plan, &revised_bundle)
        .unwrap();
    let logical = revision_store
        .get_logical_work_item(&plan, "work_item_0001")
        .unwrap();
    revision_store
        .set_active_work_item_revision(
            &plan,
            &logical,
            Some("work_item_revision_0001"),
            &revised.id,
        )
        .unwrap();
    let previous_plan = revision_store
        .get_plan_revision(
            &plan.project_id,
            &plan.issue_id,
            &plan.id,
            "plan_revision_0001",
        )
        .unwrap();
    let mut next_bindings = previous_plan.work_item_bindings.clone();
    next_bindings.insert("work_item_0001".to_string(), revised.id.clone());
    let next_plan = WorkItemPlanRevision {
        id: "plan_revision_0002".to_string(),
        plan_id: plan.id.clone(),
        revision_no: 2,
        supersedes: Some(previous_plan.id.clone()),
        reason: PlanRevisionReason::RepairUpstreamContract,
        work_item_bindings: next_bindings,
        dependency_graph_revision_id: previous_plan.dependency_graph_revision_id,
        validation_report_ref: "validation_report_0002".to_string(),
        plan_projection_bundle_id: "plan_projection_bundle_0002".to_string(),
        created_at: "2026-07-19T00:00:01Z".to_string(),
    };
    let previous_plan_projection = revision_store
        .get_plan_projection_bundle(&plan, &previous_plan.plan_projection_bundle_id)
        .unwrap();
    let mut next_plan_projection = previous_plan_projection;
    next_plan_projection.id = next_plan.plan_projection_bundle_id.clone();
    next_plan_projection.plan_revision_id = next_plan.id.clone();
    next_plan_projection.dependency_graph_revision_id =
        next_plan.dependency_graph_revision_id.clone();
    next_plan_projection.work_item_projection_bundle_refs = next_plan_projection
        .work_item_projection_bundle_refs
        .iter()
        .map(|bundle_id| {
            if bundle_id == &old_revision.work_item_projection_bundle_id {
                revised.work_item_projection_bundle_id.clone()
            } else {
                bundle_id.clone()
            }
        })
        .collect();
    next_plan_projection.created_at = "2026-07-19T00:00:01Z".to_string();
    revision_store
        .put_plan_projection_bundle(&plan, &next_plan_projection)
        .unwrap();
    revision_store.put_plan_revision(&plan, &next_plan).unwrap();
    let published_request = revision_store
        .get_repair_request(&plan, "plan_repair_request_0001")
        .unwrap();
    let amendment_id = published_request.amendment_id.clone().unwrap();
    revision_store
        .publish_active_plan_amendment_revision(
            &plan,
            &amendment_id,
            "plan_revision_0001",
            "plan_revision_0002",
            "2026-07-19T00:00:02Z",
        )
        .unwrap();
    let manifest = PlanAmendmentManifest {
        id: amendment_id,
        repair_request_id: published_request.id.clone(),
        previous_plan_revision_id: "plan_revision_0001".to_string(),
        new_plan_revision_id: "plan_revision_0002".to_string(),
        revised_work_items: BTreeMap::from([(
            "work_item_0001".to_string(),
            WorkItemRevisionReplacement {
                previous_revision_id: "work_item_revision_0001".to_string(),
                next_revision_id: revised.id,
                delta_kind: ContractDeltaKind::ImplementationGuidance,
            },
        )]),
        superseded_revisions: vec!["work_item_revision_0001".to_string()],
        dependency_graph_changes: Vec::new(),
        contract_deltas: Vec::new(),
        unaffected_units: vec!["work_item_0003".to_string()],
        revalidation_required_units: vec!["work_item_0002".to_string()],
        stale_units: Vec::new(),
        replacement_units: BTreeMap::new(),
        resume_target: AmendmentResumeTarget {
            logical_work_item_id: "work_item_0001".to_string(),
            mode: resume_mode,
        },
        created_at: "2026-07-19T00:00:02Z".to_string(),
    };
    revision_store
        .put_amendment_manifest(&plan, &manifest)
        .unwrap();
    let published_request = revision_store
        .update_repair_request_status(
            &plan,
            &manifest.repair_request_id,
            PlanRepairRequestStatus::Published,
        )
        .unwrap();
    let mut snapshot = lifecycle
        .load_plan_repair_session_state(&attempt.project_id, &attempt.issue_id, &child.id)
        .unwrap()
        .unwrap();
    snapshot.request = published_request;
    snapshot.stage = PlanRepairSessionStage::Published;
    snapshot.amendment = Some(manifest.clone());
    lifecycle
        .save_plan_repair_session_state(
            &attempt.project_id,
            &attempt.issue_id,
            &child.id,
            &snapshot,
        )
        .unwrap();
    lifecycle
        .update_workspace_session_status(&child.id, WorkspaceSessionStatus::WaitingForHuman)
        .unwrap();
    let (event_tx, mut socket_event_rx) = mpsc::channel(16);
    let (observed_event_tx, event_rx) = mpsc::channel(16);
    tokio::spawn(async move {
        while let Some(event) = socket_event_rx.recv().await {
            crate::web::coding_ws_handler::delivery_ack::confirm_plan_amendment_socket_write(
                &event,
            );
            if observed_event_tx.send(event).await.is_err() {
                break;
            }
        }
    });
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), event_tx);

    AmendmentFixture {
        _root: root,
        store,
        revision_store,
        attempt,
        plan,
        manifest,
        child_session_id: child.id,
        engine,
        _event_rx: event_rx,
    }
}

fn reset_completed_application_to_unfinished_finalization(
    fixture: &AmendmentFixture,
    applied: &CodingExecutionAttempt,
) {
    let lifecycle = LifecycleStore::new(fixture.store.paths());
    let plan = fixture
        .revision_store
        .get_plan_lineage(&applied.project_id, &applied.issue_id, &fixture.plan.id)
        .unwrap();
    fixture
        .revision_store
        .acquire_active_amendment(&plan, &fixture.manifest.id)
        .unwrap();
    let published_request = fixture
        .revision_store
        .update_repair_request_status(
            &plan,
            &fixture.manifest.repair_request_id,
            PlanRepairRequestStatus::Published,
        )
        .unwrap();
    let mut snapshot = lifecycle
        .load_plan_repair_session_state(
            &applied.project_id,
            &applied.issue_id,
            &fixture.child_session_id,
        )
        .unwrap()
        .unwrap();
    snapshot.request = published_request;
    snapshot.stage = PlanRepairSessionStage::Published;
    lifecycle
        .save_plan_repair_session_state(
            &applied.project_id,
            &applied.issue_id,
            &fixture.child_session_id,
            &snapshot,
        )
        .unwrap();
    lifecycle
        .update_workspace_session_status(
            &fixture.child_session_id,
            WorkspaceSessionStatus::WaitingForHuman,
        )
        .unwrap();
    let mut applying = fixture
        .store
        .get_attempt(&applied.project_id, &applied.issue_id, &applied.id)
        .unwrap();
    applying.status = CodingAttemptStatus::ApplyingPlanAmendment;
    fixture
        .store
        .write_coding_attempt_for_test(&applying)
        .unwrap();
}
