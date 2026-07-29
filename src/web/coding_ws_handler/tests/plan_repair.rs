use std::collections::BTreeMap;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::*;
use crate::cross_cutting::provider_adapter::ProviderAdapterError;
use crate::cross_cutting::streaming_provider::{
    ProviderSession, StreamingProviderAdapter, StreamingProviderInput,
};
use crate::product::coding_attempt_store::{
    CreateCodingExecutionUnitInput, CreateGroupCodingAttemptInput,
};
use crate::product::coding_models::{
    CodingAttemptPlanBinding, CodingExecutionUnitStatus, CodingUnitRun, CodingUnitRunStatus,
    InternalPrReview,
};
use crate::product::coding_workspace_engine::CodingWorkspaceEngine;
use crate::product::git_workspace_service::GitWorkspaceService;
use crate::product::issue_store::{CreateProductIssueInput, IssueStore};
use crate::product::lifecycle_store::CreateWorkspaceSessionInput;
use crate::product::models::{
    AmendmentResumeMode, AmendmentResumeTarget, DependencyGraphRevision, LogicalWorkItem,
    PlanAmendmentManifest, PlanDefectClass, PlanDefectEvidence, PlanDefectRoute,
    PlanProjectionBundle, PlanRepairRequestStatus, PlanRepairSessionStage, PlanRevisionReason,
    RepairTarget, RepairTargetKind, VerificationPlanRevision, WorkItemPlanLineage,
    WorkItemPlanRevision, WorkItemProjectionBundle, WorkItemRevision,
};
use crate::product::plan_repair::PlanDefectConfidence;
use crate::product::repository_store::{CreateRepositoryInput, RepositoryStore};
use crate::product::work_item_contract::{
    BlockerRoute, BlockerRule, CanonicalWorkItemContract, ContractCompatibilityPolicy,
    HandoffContract, PromisedOutputContract, RequiredInputContract, WorkItemContractIdentity,
    WorkItemGoal, WorkItemWritePolicy, canonical_contract_hash,
};
use crate::product::work_item_projection::{
    CoderGroupContext, CompiledPlanProjections, HumanGroupProjection, HumanGroupWorkItemSummary,
    ReviewerGroupMatrix, ReviewerGroupMatrixEntry, WorkItemProjectionCompiler,
    plan_projection_hashes, projection_hashes,
};
use crate::product::work_item_revision_store::WorkItemRevisionStore;

mod delivery_ack;
mod failed_review_recovery;
mod identity;
mod provider_start_failure_recovery;
mod reconciliation;
mod runner_amendment_recovery;
mod support;
mod typed_sources;

use support::*;

struct PlanRepairFixture {
    _tmp: TempDir,
    store: CodingAttemptStore,
    revision_store: WorkItemRevisionStore,
    attempt: CodingExecutionAttempt,
    plan: WorkItemPlanLineage,
    projection: crate::product::work_item_projection::ReviewerWorkItemProjection,
    event_rx: mpsc::Receiver<CodingWsOutMessage>,
    engine: CodingWorkspaceEngine,
}

#[derive(Debug, Clone, Copy)]
enum ProviderEntry {
    Coder,
    CoderRework,
    CodeReviewer,
    InternalReviewer,
    GroupFinalReviewer,
}

#[derive(Clone, Default)]
struct CountingProvider {
    starts: Arc<AtomicUsize>,
}

impl CountingProvider {
    fn starts(&self) -> usize {
        self.starts.load(Ordering::SeqCst)
    }
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for CountingProvider {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        Err(ProviderAdapterError::command_missing(
            "provider must not start during plan amendment",
        ))
    }
}

#[tokio::test]
async fn coding_plan_repair_pauses_unit_without_incrementing_rework_count() {
    let mut fixture = plan_repair_fixture();
    let before = fixture.store.get_active_unit_run(&fixture.attempt).unwrap();
    let report = plan_defect_report(plan_defect_finding("evidence_a"));

    let after = fixture
        .engine
        .start_plan_repair_from_review(
            &fixture.attempt,
            &report.id,
            "code_review_report_0001_finding_0001",
            &report.findings[0],
            &fixture.projection,
        )
        .await
        .unwrap();
    let run = fixture.store.get_active_unit_run(&after).unwrap();

    assert_eq!(after.status, CodingAttemptStatus::AwaitingPlanAmendment);
    assert_eq!(run.status, CodingUnitRunStatus::BlockedByPlanDefect);
    assert_eq!(run.unit_rework_count, before.unit_rework_count);
    assert_eq!(
        run.verification_retry_count,
        before.verification_retry_count
    );
    assert_eq!(run.operational_retry_count, before.operational_retry_count);
    assert_eq!(run.plan_repair_count, before.plan_repair_count + 1);
    assert_eq!(
        fixture
            .revision_store
            .list_open_repair_requests(&fixture.plan)
            .unwrap()
            .len(),
        1
    );
    let timeline = fixture
        .store
        .get_timeline_nodes(&after.project_id, &after.issue_id, &after.id)
        .unwrap();
    assert!(timeline.iter().any(|node| {
        node.title == "Plan Repair" && node.status == CodingTimelineNodeStatus::Blocked
    }));
    assert!(matches!(
        fixture.event_rx.recv().await,
        Some(CodingWsOutMessage::CodingTimelineNodeCreated { .. })
    ));
    let event = fixture.event_rx.recv().await.expect("plan repair event");
    let encoded = serde_json::to_value(&event).unwrap();
    assert_eq!(encoded["type"], "plan_repair_required");
    assert_eq!(encoded["request"]["status"], "in_progress");
    assert!(encoded["session_link"].is_object());
    assert_eq!(
        serde_json::from_value::<CodingWsOutMessage>(encoded).unwrap(),
        event
    );
}

#[tokio::test]
async fn coding_plan_repair_duplicate_finding_reuses_open_request() {
    let fixture = plan_repair_fixture();
    let first = plan_defect_report(plan_defect_finding("evidence_a"));
    let after = fixture
        .engine
        .start_plan_repair_from_review(
            &fixture.attempt,
            &first.id,
            "code_review_report_0001_finding_0001",
            &first.findings[0],
            &fixture.projection,
        )
        .await
        .unwrap();
    let mut duplicate = plan_defect_report(plan_defect_finding("evidence_b"));
    duplicate.id = "code_review_report_0002".to_string();
    fixture
        .engine
        .start_plan_repair_from_review(
            &after,
            &duplicate.id,
            "code_review_report_0002_finding_0001",
            &duplicate.findings[0],
            &fixture.projection,
        )
        .await
        .unwrap();

    let requests = fixture
        .revision_store
        .list_open_repair_requests(&fixture.plan)
        .unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].evidence.len(), 2);
    assert_eq!(
        fixture
            .store
            .get_active_unit_run(&after)
            .unwrap()
            .plan_repair_count,
        1
    );
    let lifecycle = LifecycleStore::new(fixture.store.paths());
    assert_eq!(
        lifecycle
            .list_session_links(&after.project_id, &after.issue_id)
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        lifecycle
            .list_workspace_sessions(&after.project_id, &after.issue_id)
            .unwrap()
            .len(),
        2,
        "duplicate finding must reuse the parent and existing repair child",
    );
}

#[tokio::test]
async fn coding_plan_repair_reconnect_state_contains_linked_snapshot() {
    let fixture = plan_repair_fixture();
    let report = plan_defect_report(plan_defect_finding("evidence_a"));
    let after = fixture
        .engine
        .start_plan_repair_from_review(
            &fixture.attempt,
            &report.id,
            "code_review_report_0001_finding_0001",
            &report.findings[0],
            &fixture.projection,
        )
        .await
        .unwrap();

    let snapshot = build_coding_session_state(&fixture.store, after).unwrap();
    let encoded = serde_json::to_value(snapshot).unwrap();
    assert_eq!(
        encoded["linked_plan_repair"]["request"]["trigger_attempt_id"],
        fixture.attempt.id
    );
    assert_eq!(
        encoded["linked_plan_repair"]["link"]["relation"],
        "plan_repair"
    );
}

#[tokio::test]
async fn coding_plan_repair_reconnect_state_resolves_published_request() {
    let fixture = plan_repair_fixture();
    let report = plan_defect_report(plan_defect_finding("published_reconnect"));
    let started = fixture
        .engine
        .start_plan_repair_from_review(
            &fixture.attempt,
            &report.id,
            "code_review_report_0001_finding_0001",
            &report.findings[0],
            &fixture.projection,
        )
        .await
        .unwrap();
    let request = fixture
        .revision_store
        .list_open_repair_requests(&fixture.plan)
        .unwrap()
        .into_iter()
        .next()
        .unwrap();
    let published = fixture
        .revision_store
        .update_repair_request_status(
            &fixture.plan,
            &request.id,
            PlanRepairRequestStatus::Published,
        )
        .unwrap();
    let lifecycle = LifecycleStore::new(fixture.store.paths());
    let link = lifecycle
        .list_session_links(&started.project_id, &started.issue_id)
        .unwrap()
        .into_iter()
        .next()
        .unwrap();
    let mut repair_snapshot = lifecycle
        .load_plan_repair_session_state(
            &started.project_id,
            &started.issue_id,
            &link.child_session_id,
        )
        .unwrap()
        .unwrap();
    repair_snapshot.request = published;

    for (status, stage) in [
        (
            CodingAttemptStatus::ApplyingPlanAmendment,
            PlanRepairSessionStage::ApplyingAmendment,
        ),
        (
            CodingAttemptStatus::AmendmentApplyFailed,
            PlanRepairSessionStage::AmendmentApplyFailed,
        ),
    ] {
        repair_snapshot.stage = stage;
        lifecycle
            .save_plan_repair_session_state(
                &started.project_id,
                &started.issue_id,
                &link.child_session_id,
                &repair_snapshot,
            )
            .unwrap();
        let attempt = fixture
            .store
            .update_attempt_status(&started.project_id, &started.issue_id, &started.id, status)
            .unwrap();

        let state = build_coding_session_state(&fixture.store, attempt).unwrap();
        let encoded = serde_json::to_value(state).unwrap();
        assert_eq!(
            encoded["linked_plan_repair"]["request"]["status"],
            "published"
        );
    }
}

#[test]
fn coding_amendment_updated_roundtrips() {
    let message = CodingWsOutMessage::PlanAmendmentUpdated {
        event_id: "coding_plan_amendment_updated_attempt_0001_plan_amendment_0001".to_string(),
        amendment: Box::new(PlanAmendmentManifest {
            id: "plan_amendment_0001".to_string(),
            repair_request_id: "plan_repair_request_0001".to_string(),
            previous_plan_revision_id: "plan_revision_0001".to_string(),
            new_plan_revision_id: "plan_revision_0002".to_string(),
            revised_work_items: BTreeMap::new(),
            superseded_revisions: Vec::new(),
            dependency_graph_changes: Vec::new(),
            contract_deltas: Vec::new(),
            unaffected_units: vec!["wi_unchanged".to_string()],
            revalidation_required_units: Vec::new(),
            stale_units: vec!["wi_current".to_string()],
            replacement_units: BTreeMap::new(),
            resume_target: AmendmentResumeTarget {
                logical_work_item_id: "wi_current".to_string(),
                mode: AmendmentResumeMode::Reexecute,
            },
            created_at: "2026-07-19T00:00:00Z".to_string(),
        }),
    };

    let encoded = serde_json::to_value(&message).unwrap();
    assert_eq!(encoded["type"], "plan_amendment_updated");
    assert_eq!(
        encoded["event_id"],
        "coding_plan_amendment_updated_attempt_0001_plan_amendment_0001"
    );
    assert_eq!(encoded["amendment"]["resume_target"]["mode"], "reexecute");
    assert_eq!(
        serde_json::from_value::<CodingWsOutMessage>(encoded).unwrap(),
        message,
    );
}

#[tokio::test]
async fn coding_plan_repair_ambiguous_parent_workspace_fails_closed_without_link() {
    let fixture = plan_repair_fixture();
    LifecycleStore::new(fixture.store.paths())
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: fixture.attempt.project_id.clone(),
            issue_id: fixture.attempt.issue_id.clone(),
            entity_id: fixture.plan.id.clone(),
            workspace_type: WorkspaceType::WorkItemPlan,
            author_provider: ProviderName::Codex,
            reviewer_provider: ProviderName::ClaudeCode,
            review_rounds: 1,
            superpowers_enabled: true,
            openspec_enabled: true,
        })
        .unwrap();
    let report = plan_defect_report(plan_defect_finding("evidence_a"));

    let error = fixture
        .engine
        .start_plan_repair_from_review(
            &fixture.attempt,
            &report.id,
            "code_review_report_0001_finding_0001",
            &report.findings[0],
            &fixture.projection,
        )
        .await
        .expect_err("ambiguous canonical parent must fail closed");

    assert!(
        error
            .to_string()
            .contains("parent WorkItemPlan workspace is ambiguous")
    );
    assert!(
        fixture
            .revision_store
            .list_open_repair_requests(&fixture.plan)
            .unwrap()
            .is_empty()
    );
    assert!(
        LifecycleStore::new(fixture.store.paths())
            .list_session_links(&fixture.attempt.project_id, &fixture.attempt.issue_id)
            .unwrap()
            .is_empty(),
        "fail-closed parent resolution must not fabricate a link",
    );
}

#[tokio::test]
async fn coding_plan_repair_group_internal_review_uses_unique_completed_unit_run() {
    let fixture = plan_repair_fixture_with_dependency(false);
    seed_completed_upstream_binding(&fixture);
    let active = fixture.store.get_active_unit_run(&fixture.attempt).unwrap();
    let completed = fixture
        .store
        .complete_coding_unit_run(&fixture.attempt, &active.id, "commit_0002")
        .unwrap();
    fixture
        .store
        .update_coding_unit_status(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
            &active.unit_id,
            CodingExecutionUnitStatus::Completed,
            Some("completed before group final review".to_string()),
        )
        .unwrap();
    let mut attempt = fixture
        .store
        .get_attempt(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .unwrap();
    assert!(attempt.active_unit_id.is_none());
    attempt.stage = CodingExecutionStage::InternalPrReview;
    fixture.store.save_coding_attempt(&attempt).unwrap();
    let review = internal_plan_defect_review(&attempt, plan_defect_finding("group_review"));

    let updated = fixture
        .engine
        .start_plan_repair_from_internal_review(&attempt, &review)
        .await
        .unwrap();

    assert_eq!(updated.status, CodingAttemptStatus::AwaitingPlanAmendment);
    let requests = fixture
        .revision_store
        .list_open_repair_requests(&fixture.plan)
        .unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].trigger_unit_run_id, completed.id);
    assert_eq!(
        fixture
            .store
            .list_coding_unit_runs(&updated, &completed.unit_id)
            .unwrap()
            .into_iter()
            .find(|run| run.id == completed.id)
            .unwrap()
            .status,
        CodingUnitRunStatus::BlockedByPlanDefect,
    );
}

#[tokio::test]
async fn coding_plan_repair_group_internal_review_ambiguous_run_fails_closed() {
    let fixture = plan_repair_fixture_with_dependency(false);
    seed_completed_upstream_binding(&fixture);
    let active = fixture.store.get_active_unit_run(&fixture.attempt).unwrap();
    fixture
        .store
        .complete_coding_unit_run(&fixture.attempt, &active.id, "commit_0002")
        .unwrap();
    fixture
        .store
        .update_coding_unit_status(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
            &active.unit_id,
            CodingExecutionUnitStatus::Completed,
            None,
        )
        .unwrap();
    let duplicate = fixture
        .store
        .create_coding_unit(CreateCodingExecutionUnitInput {
            attempt_id: fixture.attempt.id.clone(),
            project_id: fixture.attempt.project_id.clone(),
            issue_id: fixture.attempt.issue_id.clone(),
            plan_id: fixture.plan.id.clone(),
            logical_work_item_id: "wi_current".to_string(),
            work_item_revision_id: active.work_item_revision_id.clone(),
            dependency_logical_work_item_ids: Vec::new(),
            order_index: 2,
            status: CodingExecutionUnitStatus::Completed,
        })
        .unwrap();
    fixture
        .store
        .create_coding_unit_run(
            &fixture.attempt,
            &CodingUnitRun {
                id: "coding_unit_run_0002".to_string(),
                unit_id: duplicate.id,
                execution_no: 1,
                work_item_revision_id: active.work_item_revision_id.clone(),
                resolved_handoff_revision_ids: Vec::new(),
                canonical_contract_hash: active.canonical_contract_hash.clone(),
                projection_bundle_id: active.projection_bundle_id.clone(),
                projection_compiler_version: active.projection_compiler_version.clone(),
                coder_provider_renderer_version: active.coder_provider_renderer_version.clone(),
                reviewer_provider_renderer_version: active
                    .reviewer_provider_renderer_version
                    .clone(),
                internal_reviewer_provider_renderer_version: None,
                coder_projection_hash: active.coder_projection_hash.clone(),
                reviewer_projection_hash: active.reviewer_projection_hash.clone(),
                coder_execution_context_hash: None,
                reviewer_execution_context_hash: None,
                internal_reviewer_execution_context_hash: None,
                status: CodingUnitRunStatus::Completed,
                unit_rework_count: 0,
                verification_retry_count: 0,
                operational_retry_count: 0,
                plan_repair_count: 0,
                start_commit: Some("commit_0001".to_string()),
                completion_commit: Some("commit_0002".to_string()),
                created_at: "2026-07-19T00:00:00Z".to_string(),
                updated_at: "2026-07-19T00:00:00Z".to_string(),
            },
        )
        .unwrap();
    let mut attempt = fixture
        .store
        .get_attempt(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .unwrap();
    attempt.stage = CodingExecutionStage::InternalPrReview;
    fixture.store.save_coding_attempt(&attempt).unwrap();
    let review = internal_plan_defect_review(&attempt, plan_defect_finding("ambiguous"));

    let error = fixture
        .engine
        .start_plan_repair_from_internal_review(&attempt, &review)
        .await
        .expect_err("ambiguous authoritative unit run must fail closed");

    assert!(error.to_string().contains("trigger binding is ambiguous"));
    assert!(
        fixture
            .revision_store
            .list_open_repair_requests(&fixture.plan)
            .unwrap()
            .is_empty()
    );
    assert!(
        LifecycleStore::new(fixture.store.paths())
            .list_session_links(&attempt.project_id, &attempt.issue_id)
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn coding_plan_repair_provider_runs_fail_closed_during_amendment() {
    for status in [
        CodingAttemptStatus::AwaitingPlanAmendment,
        CodingAttemptStatus::ApplyingPlanAmendment,
        CodingAttemptStatus::AmendmentApplyFailed,
    ] {
        for entry in [
            ProviderEntry::Coder,
            ProviderEntry::CoderRework,
            ProviderEntry::CodeReviewer,
            ProviderEntry::InternalReviewer,
            ProviderEntry::GroupFinalReviewer,
        ] {
            assert_provider_entry_blocked(status.clone(), entry).await;
        }
    }
}

#[tokio::test]
async fn coding_plan_repair_failed_review_recovery_writes_no_journal_during_amendment() {
    for status in [
        CodingAttemptStatus::AwaitingPlanAmendment,
        CodingAttemptStatus::ApplyingPlanAmendment,
        CodingAttemptStatus::AmendmentApplyFailed,
    ] {
        let fixture = plan_repair_fixture();
        let mut attempt = fixture.attempt.clone();
        attempt.status = status.clone();
        fixture.store.save_coding_attempt(&attempt).unwrap();

        let error = fixture
            .engine
            .recover_failed_code_review_for_attempt(&attempt, "coding_gate_0001")
            .await
            .expect_err("failed review recovery must be blocked");

        assert!(
            error
                .to_string()
                .contains("plan_amendment_blocks_provider_run"),
            "unexpected recovery error for {status:?}: {error}",
        );
        assert!(
            fixture
                .store
                .get_failed_code_review_recovery_journal(
                    &attempt.project_id,
                    &attempt.issue_id,
                    &attempt.id,
                )
                .unwrap()
                .is_none(),
            "recovery journal must remain absent for {status:?}",
        );
    }
}

async fn assert_provider_entry_blocked(status: CodingAttemptStatus, entry: ProviderEntry) {
    let fixture = plan_repair_fixture();
    let mut attempt = fixture.attempt.clone();
    attempt.status = status.clone();
    fixture.store.save_coding_attempt(&attempt).unwrap();
    let before = fixture
        .store
        .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .unwrap();
    let before_timeline = fixture
        .store
        .get_timeline_nodes(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .unwrap();
    let before_role_runs = fixture
        .store
        .list_role_runs(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .unwrap();
    let before_rework_instructions = fixture
        .store
        .list_rework_instructions(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .unwrap();
    let provider = CountingProvider::default();
    let error = match entry {
        ProviderEntry::Coder => fixture
            .engine
            .execute_coding(
                &attempt,
                &provider,
                &crate::product::coding_workspace_engine::CodingExecutionContext::default(),
            )
            .await
            .expect_err("coder must be blocked")
            .to_string(),
        ProviderEntry::CoderRework => {
            let (_command_tx, mut command_rx) = mpsc::channel(1);
            let report = plan_defect_report(plan_defect_finding("provider_guard"));
            fixture
                .engine
                .execute_coder_fix_from_review(
                    &attempt,
                    &report,
                    &crate::product::coding_workspace_engine::CodingExecutionContext::default(),
                    &provider,
                    &mut command_rx,
                )
                .await
                .expect_err("coder rework must be blocked")
                .to_string()
        }
        ProviderEntry::CodeReviewer => fixture
            .engine
            .execute_code_review(&attempt, &provider)
            .await
            .expect_err("code reviewer must be blocked")
            .to_string(),
        ProviderEntry::InternalReviewer => fixture
            .engine
            .execute_internal_pr_review(&attempt, &provider)
            .await
            .expect_err("internal reviewer must be blocked")
            .to_string(),
        ProviderEntry::GroupFinalReviewer => {
            let (_command_tx, mut command_rx) = mpsc::channel(1);
            fixture
                .engine
                .execute_group_final_review_with_commands(&attempt, &provider, &mut command_rx)
                .await
                .expect_err("group final reviewer must be blocked")
                .to_string()
        }
    };

    assert!(
        error.contains("plan_amendment_blocks_provider_run"),
        "{entry:?} returned {error:?} for {status:?}",
    );
    assert_eq!(provider.starts(), 0, "{entry:?} started provider");
    let after = fixture
        .store
        .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .unwrap();
    assert_eq!(after.status, before.status, "{entry:?} changed status");
    assert_eq!(after.stage, before.stage, "{entry:?} changed stage");
    assert_eq!(
        after.rework_count, before.rework_count,
        "{entry:?} changed rework count",
    );
    assert_eq!(
        fixture
            .store
            .get_timeline_nodes(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .unwrap(),
        before_timeline,
        "{entry:?} changed timeline",
    );
    assert_eq!(
        fixture
            .store
            .list_role_runs(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .unwrap(),
        before_role_runs,
        "{entry:?} changed role runs",
    );
    assert_eq!(
        fixture
            .store
            .list_rework_instructions(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .unwrap(),
        before_rework_instructions,
        "{entry:?} changed rework instructions",
    );
}
