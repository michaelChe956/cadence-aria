use super::group_final_readiness_support::*;
use super::*;
use crate::cross_cutting::provider_registry::ProviderRegistry;
use crate::product::coding_models::{
    CasOutcome, GroupFinalReadinessDiagnosticKind, GroupFinalReadinessStatus,
    GroupReviewReductionReport, GroupReviewShardReport,
};
use crate::product::lifecycle_store::UpsertIssueSharedWorktreeInput;
use crate::web::coding_ws_handler::execute_start_coding_flow;
use crate::web::runtime::WebRuntime;
use crate::web::state::WebAppState;

fn seed_legacy_group_review_reports(
    store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
) -> (GroupReviewShardReport, GroupReviewReductionReport) {
    let snapshot_hash = "legacy_group_review_snapshot";
    store
        .activate_group_review_snapshot(&attempt.id, snapshot_hash)
        .expect("activate legacy group-review snapshot");
    let shard = GroupReviewShardReport {
        id: "group_review_shard_legacy_0001".to_string(),
        attempt_id: attempt.id.clone(),
        snapshot_hash: snapshot_hash.to_string(),
        shard_id: "legacy_0001".to_string(),
        ordered_unit_run_ids: Vec::new(),
        partition_rationale: vec!["historical shard".to_string()],
        verdict: ReviewVerdict::Approve,
        findings: Vec::new(),
        unresolved_obligations: Vec::new(),
        selected_diff_refs: Vec::new(),
        raw_provider_output_refs: Vec::new(),
        role_run_ids: Vec::new(),
        run_failure_code: None,
    };
    assert!(matches!(
        store
            .write_group_review_shard_report_cas(&attempt.id, shard.clone())
            .expect("write legacy shard report"),
        CasOutcome::Written
    ));
    let reduction = GroupReviewReductionReport {
        id: "group_review_reduction_legacy_0001".to_string(),
        attempt_id: attempt.id.clone(),
        snapshot_hash: snapshot_hash.to_string(),
        shard_report_ids: vec![shard.id.clone()],
        verdict: ReviewVerdict::Approve,
        findings: Vec::new(),
        impact_scope: vec!["historical group".to_string()],
        pr_description: "legacy reduction report".to_string(),
        commit_message_suggestion: "legacy group review".to_string(),
        provenance: Vec::new(),
        raw_provider_output_refs: Vec::new(),
        role_run_ids: Vec::new(),
        run_failure_code: None,
    };
    assert!(matches!(
        store
            .write_group_review_reduction_report_cas(&attempt.id, reduction.clone())
            .expect("write legacy reduction report"),
        CasOutcome::Written
    ));
    (shard, reduction)
}

#[tokio::test]
async fn legacy_attempt_with_reduction_artifact_recovers_to_human_final_without_provider_call() {
    let fixture = readiness_fixture();
    let running = seed_complete_group_readiness(&fixture);
    let legacy = fixture
        .store
        .update_attempt_stage(
            &running.project_id,
            &running.issue_id,
            &running.id,
            CodingExecutionStage::InternalPrReview,
        )
        .expect("historical internal review stage");
    let (shard, reduction) = seed_legacy_group_review_reports(&fixture.store, &legacy);
    let legacy_role_run = fixture
        .store
        .create_role_run(
            &legacy,
            CodingExecutionStage::InternalPrReview,
            CodingProviderRole::InternalReviewer,
            CodingRoleRunTrigger::Initial,
            None,
        )
        .expect("historical group-review role run");
    fixture
        .store
        .update_role_run_status(
            &legacy.project_id,
            &legacy.issue_id,
            &legacy.id,
            &legacy_role_run.id,
            CodingRoleRunStatus::Completed,
            Some("legacy reduction persisted before restart".to_string()),
        )
        .expect("complete historical group-review role run");
    seed_runner_revision_history(&fixture);

    // An empty registry makes any accidental internal-review, shard, or reduction provider
    // lookup fail. A successful recovery therefore proves the legacy path stays human-only.
    let state = WebAppState::with_provider_registry(
        fixture._root.path().to_path_buf(),
        WebRuntime::new_fake(fixture._root.path().to_path_buf()),
        ProviderRegistry::new(),
    );
    let (event_tx, _event_rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(
        fixture.store.clone(),
        GitWorkspaceService::new(),
        event_tx.clone(),
    );
    let (command_tx, command_rx) = mpsc::channel(1);
    command_tx
        .send(CodingRunnerCommand::StageGateConfirm {
            stage: CodingExecutionStage::InternalPrReview,
        })
        .await
        .expect("legacy stage gate confirmation");

    execute_start_coding_flow(
        &state,
        &fixture.store,
        &engine,
        &event_tx,
        command_rx,
        &legacy,
    )
    .await
    .expect("legacy recovery must enter human final confirmation");

    let recovered = fixture
        .store
        .get_attempt(&legacy.project_id, &legacy.issue_id, &legacy.id)
        .expect("recovered attempt");
    assert_eq!(recovered.stage, CodingExecutionStage::FinalConfirm);
    assert_eq!(recovered.status, CodingAttemptStatus::WaitingForHuman);
    assert!(
        fixture
            .store
            .get_group_final_readiness_snapshot(&recovered)
            .expect("readiness snapshot")
            .is_some_and(|snapshot| snapshot.status == GroupFinalReadinessStatus::Complete)
    );
    let internal_reviewer_runs = fixture
        .store
        .list_role_runs(&recovered.project_id, &recovered.issue_id, &recovered.id)
        .expect("role runs")
        .into_iter()
        .filter(|run| run.role == CodingProviderRole::InternalReviewer)
        .collect::<Vec<_>>();
    assert_eq!(
        internal_reviewer_runs.len(),
        1,
        "legacy recovery must not create another InternalPrReview provider run"
    );
    assert_eq!(internal_reviewer_runs[0].id, legacy_role_run.id);
    let reductions = fixture
        .store
        .list_group_review_reduction_reports(&recovered.id)
        .expect("legacy reduction reader");
    assert_eq!(reductions.len(), 1);
    assert_eq!(reductions[0].id, reduction.id);
    assert_eq!(reductions[0].pr_description, reduction.pr_description);
    let shards = fixture
        .store
        .list_group_review_shard_reports(&recovered.id)
        .expect("legacy shard reader");
    assert_eq!(shards.len(), 1);
    assert_eq!(shards[0].id, shard.id);
    assert_eq!(shards[0].partition_rationale, shard.partition_rationale);
}

#[tokio::test]
async fn legacy_identity_mismatch_records_diagnostic_and_does_not_start_reduction() {
    let fixture = readiness_fixture();
    let running = seed_complete_group_readiness(&fixture);
    let unit = first_unit(&fixture);
    let run = fixture
        .store
        .list_coding_unit_runs(&running, &unit.id)
        .expect("unit runs")
        .into_iter()
        .next()
        .expect("completed unit run");
    seed_handoff_with_id_and_commit(
        &fixture,
        &unit,
        &run,
        "handoff_revision_legacy_identity_mismatch",
        "unverified_historical_commit",
    );
    let legacy = fixture
        .store
        .update_attempt_stage(
            &running.project_id,
            &running.issue_id,
            &running.id,
            CodingExecutionStage::InternalPrReview,
        )
        .expect("historical internal review stage");
    seed_runner_revision_history(&fixture);

    let state = WebAppState::with_provider_registry(
        fixture._root.path().to_path_buf(),
        WebRuntime::new_fake(fixture._root.path().to_path_buf()),
        ProviderRegistry::new(),
    );
    let (event_tx, _event_rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(
        fixture.store.clone(),
        GitWorkspaceService::new(),
        event_tx.clone(),
    );
    let (command_tx, command_rx) = mpsc::channel(1);
    command_tx
        .send(CodingRunnerCommand::StageGateConfirm {
            stage: CodingExecutionStage::InternalPrReview,
        })
        .await
        .expect("legacy identity-mismatch stage gate confirmation");

    execute_start_coding_flow(
        &state,
        &fixture.store,
        &engine,
        &event_tx,
        command_rx,
        &legacy,
    )
    .await
    .expect("identity-mismatched legacy recovery must remain visible to a human");

    let recovered = fixture
        .store
        .get_attempt(&legacy.project_id, &legacy.issue_id, &legacy.id)
        .expect("recovered attempt");
    assert_eq!(recovered.stage, CodingExecutionStage::FinalConfirm);
    assert_eq!(recovered.status, CodingAttemptStatus::WaitingForHuman);
    let snapshot = fixture
        .store
        .get_group_final_readiness_snapshot(&recovered)
        .expect("readiness snapshot")
        .expect("incomplete readiness snapshot");
    assert_eq!(snapshot.status, GroupFinalReadinessStatus::Incomplete);
    assert!(snapshot.diagnostics.iter().any(|diagnostic| {
        diagnostic.kind == GroupFinalReadinessDiagnosticKind::IdentityMismatch
    }));
    assert!(
        fixture
            .store
            .list_group_review_shard_reports(&recovered.id)
            .expect("shard reports")
            .is_empty()
    );
    assert!(
        fixture
            .store
            .list_group_review_reduction_reports(&recovered.id)
            .expect("reduction reports")
            .is_empty()
    );
    assert!(matches!(
        engine
            .handle_final_confirm(&recovered.project_id, &recovered.issue_id, &recovered.id)
            .await,
        Err(CodingWorkspaceEngineError::FinalConfirmNotReady(ref id)) if id == &recovered.id
    ));
}

#[test]
fn legacy_shard_and_reduction_reports_remain_readable() {
    let fixture = readiness_fixture();
    let (shard, reduction) = seed_legacy_group_review_reports(&fixture.store, &fixture.attempt);

    let shards = fixture
        .store
        .list_group_review_shard_reports(&fixture.attempt.id)
        .expect("historical shard reports remain readable");
    assert_eq!(shards.len(), 1);
    assert_eq!(shards[0].id, shard.id);
    assert_eq!(shards[0].partition_rationale, shard.partition_rationale);
    let reductions = fixture
        .store
        .list_group_review_reduction_reports(&fixture.attempt.id)
        .expect("historical reduction reports remain readable");
    assert_eq!(reductions.len(), 1);
    assert_eq!(reductions[0].id, reduction.id);
    assert_eq!(reductions[0].shard_report_ids, reduction.shard_report_ids);
}

#[tokio::test]
async fn fresh_group_enters_waiting_final_confirm_without_any_group_provider_run() {
    let fixture = readiness_fixture();
    let running = seed_complete_group_readiness(&fixture);
    let running = fixture
        .store
        .update_attempt_stage(
            &running.project_id,
            &running.issue_id,
            &running.id,
            CodingExecutionStage::ReviewRequest,
        )
        .expect("review request stage");
    seed_runner_revision_history(&fixture);
    let remote = fixture._root.path().join("origin.git");
    let status = std::process::Command::new("git")
        .args(["init", "--bare", remote.to_str().expect("remote path")])
        .status()
        .expect("create bare origin");
    assert!(status.success(), "create bare origin");
    run_test_git(&fixture.worktree, &["branch", "aria/issues/issue_0001"]);
    run_test_git(
        &fixture.worktree,
        &[
            "remote",
            "add",
            "origin",
            remote.to_str().expect("remote path"),
        ],
    );

    // An empty registry proves this tail path cannot invoke a Group Reviewer, shard,
    // reduction, or any other provider after unit-level evidence is complete.
    let state = WebAppState::with_provider_registry(
        fixture._root.path().to_path_buf(),
        WebRuntime::new_fake(fixture._root.path().to_path_buf()),
        ProviderRegistry::new(),
    );
    let (event_tx, _event_rx) = mpsc::channel(16);
    let store = fixture.store.clone();
    let engine =
        CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), event_tx.clone());
    let (_command_tx, command_rx) = mpsc::channel(1);

    execute_start_coding_flow(&state, &store, &engine, &event_tx, command_rx, &running)
        .await
        .expect("runner transitions directly to human final confirmation");

    let prepared = store
        .get_attempt(&running.project_id, &running.issue_id, &running.id)
        .expect("prepared attempt");
    assert_eq!(prepared.stage, CodingExecutionStage::FinalConfirm);
    assert_eq!(prepared.status, CodingAttemptStatus::WaitingForHuman);
    assert!(
        store
            .get_group_final_readiness_snapshot(&prepared)
            .expect("stored readiness")
            .is_some_and(|snapshot| snapshot.status == GroupFinalReadinessStatus::Complete)
    );
    assert!(
        store
            .list_role_runs(&prepared.project_id, &prepared.issue_id, &prepared.id)
            .expect("role runs")
            .iter()
            .all(|run| run.role != CodingProviderRole::InternalReviewer)
    );
    assert!(
        store
            .get_timeline_nodes(&prepared.project_id, &prepared.issue_id, &prepared.id)
            .expect("timeline")
            .iter()
            .all(|node| node.stage != CodingExecutionStage::InternalPrReview)
    );
    assert!(
        store
            .list_group_review_shard_reports(&prepared.id)
            .expect("shard reports")
            .is_empty()
    );
    assert!(
        store
            .list_group_review_reduction_reports(&prepared.id)
            .expect("reduction reports")
            .is_empty()
    );
}

#[tokio::test]
async fn preparing_complete_group_creates_human_final_confirm_without_group_provider_run() {
    let fixture = readiness_fixture();
    let running = seed_complete_group_readiness(&fixture);

    let prepared = fixture
        .engine
        .prepare_group_final_confirm_from_readiness(&running)
        .await
        .expect("prepare human final confirmation");

    assert_eq!(prepared.stage, CodingExecutionStage::FinalConfirm);
    assert_eq!(prepared.status, CodingAttemptStatus::WaitingForHuman);
    assert!(
        fixture
            .store
            .get_group_final_readiness_snapshot(&prepared)
            .expect("stored readiness")
            .is_some_and(|snapshot| snapshot.status == GroupFinalReadinessStatus::Complete)
    );
    assert!(
        fixture
            .store
            .list_role_runs(&prepared.project_id, &prepared.issue_id, &prepared.id)
            .expect("role runs")
            .iter()
            .all(|run| run.role != CodingProviderRole::InternalReviewer)
    );
    assert!(
        fixture
            .store
            .get_timeline_nodes(&prepared.project_id, &prepared.issue_id, &prepared.id)
            .expect("timeline")
            .iter()
            .all(|node| node.stage != CodingExecutionStage::InternalPrReview)
    );
    assert!(
        fixture
            .store
            .list_group_review_shard_reports(&prepared.id)
            .expect("shard reports")
            .is_empty()
    );
    assert!(
        fixture
            .store
            .list_group_review_reduction_reports(&prepared.id)
            .expect("reduction reports")
            .is_empty()
    );
    assert!(
        fixture
            .store
            .get_timeline_nodes(&prepared.project_id, &prepared.issue_id, &prepared.id)
            .expect("timeline")
            .iter()
            .any(|node| {
                node.stage == CodingExecutionStage::FinalConfirm
                    && node.status == CodingTimelineNodeStatus::Pending
                    && node.title == "等待人工最终确认"
            })
    );
}

#[tokio::test]
async fn preparing_group_final_confirm_twice_reuses_pending_timeline_node() {
    let fixture = readiness_fixture();
    let running = seed_complete_group_readiness(&fixture);

    let first = fixture
        .engine
        .prepare_group_final_confirm_from_readiness(&running)
        .await
        .expect("prepare initial human final confirmation");
    let second = fixture
        .engine
        .prepare_group_final_confirm_from_readiness(&first)
        .await
        .expect("repeat preparation reuses pending final confirmation");

    assert_eq!(second.stage, CodingExecutionStage::FinalConfirm);
    assert_eq!(second.status, CodingAttemptStatus::WaitingForHuman);
    let final_confirm_nodes = fixture
        .store
        .get_timeline_nodes(&second.project_id, &second.issue_id, &second.id)
        .expect("timeline nodes")
        .into_iter()
        .filter(|node| node.stage == CodingExecutionStage::FinalConfirm)
        .collect::<Vec<_>>();
    assert_eq!(
        final_confirm_nodes.len(),
        1,
        "repeat preparation must reuse the pending human final-confirm timeline node"
    );
    assert_eq!(
        final_confirm_nodes[0].status,
        CodingTimelineNodeStatus::Pending
    );
}

#[tokio::test]
async fn incomplete_readiness_cannot_be_final_confirmed() {
    let fixture = readiness_fixture();
    for unit in fixture
        .store
        .list_coding_units(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .expect("units")
    {
        fixture
            .store
            .update_coding_unit_status(
                &fixture.attempt.project_id,
                &fixture.attempt.issue_id,
                &fixture.attempt.id,
                &unit.id,
                CodingExecutionUnitStatus::Completed,
                Some("independent review approved".to_string()),
            )
            .expect("complete unit");
    }
    let running = fixture
        .store
        .update_attempt_status(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running attempt");
    let prepared = fixture
        .engine
        .prepare_group_final_confirm_from_readiness(&running)
        .await
        .expect("prepare incomplete readiness for human diagnosis");
    let snapshot = fixture
        .store
        .get_group_final_readiness_snapshot(&prepared)
        .expect("stored readiness")
        .expect("incomplete readiness snapshot");
    assert_eq!(snapshot.status, GroupFinalReadinessStatus::Incomplete);
    assert!(snapshot.diagnostics.iter().any(|diagnostic| {
        diagnostic.kind == GroupFinalReadinessDiagnosticKind::UnitRunMissing
    }));

    let error = fixture
        .engine
        .handle_final_confirm(&prepared.project_id, &prepared.issue_id, &prepared.id)
        .await
        .expect_err("incomplete snapshot must block final confirm");
    assert!(matches!(
        error,
        CodingWorkspaceEngineError::FinalConfirmNotReady(ref id) if id == &prepared.id
    ));
    let persisted = fixture
        .store
        .get_attempt(&prepared.project_id, &prepared.issue_id, &prepared.id)
        .expect("persisted attempt");
    assert_eq!(persisted.stage, CodingExecutionStage::FinalConfirm);
    assert_eq!(persisted.status, CodingAttemptStatus::WaitingForHuman);
}

#[tokio::test]
async fn final_confirm_keeps_range_scope_and_dirty_worktree_gate() {
    let fixture = readiness_fixture();
    let running = seed_complete_group_readiness(&fixture);
    let prepared = fixture
        .engine
        .prepare_group_final_confirm_from_readiness(&running)
        .await
        .expect("prepare final confirmation");
    let lifecycle = LifecycleStore::new(fixture.store.paths());
    lifecycle
        .upsert_issue_shared_worktree(UpsertIssueSharedWorktreeInput {
            project_id: prepared.project_id.clone(),
            issue_id: prepared.issue_id.clone(),
            repository_id: "repository_0001".to_string(),
            branch_name: prepared.branch_name.clone(),
            worktree_path: fixture.worktree.clone(),
            base_branch: prepared.base_branch.clone(),
        })
        .expect("shared worktree");
    let lock_work_item_id = fixture
        .store
        .list_coding_units(&prepared.project_id, &prepared.issue_id, &prepared.id)
        .expect("completed units")
        .into_iter()
        .max_by_key(|unit| unit.order_index)
        .expect("last completed unit")
        .logical_work_item_id;
    lifecycle
        .try_acquire_issue_worktree_lock(
            &prepared.project_id,
            &prepared.issue_id,
            &lock_work_item_id,
            &prepared.id,
        )
        .expect("shared worktree lock");
    fs::write(
        fixture.worktree.join("manual-residue.txt"),
        "leave this alone\n",
    )
    .expect("dirty residue");
    let head_before = git_stdout(&fixture.worktree, &["rev-parse", "HEAD"]);

    let error = fixture
        .engine
        .handle_final_confirm(&prepared.project_id, &prepared.issue_id, &prepared.id)
        .await
        .expect_err("dirty worktree must remain a final-confirm gate");

    assert!(matches!(
        error,
        CodingWorkspaceEngineError::SharedWorktreeDirtyManualGate(_)
    ));
    assert_eq!(
        fs::read_to_string(fixture.worktree.join("manual-residue.txt")).expect("residue remains"),
        "leave this alone\n"
    );
    assert_eq!(
        git_stdout(&fixture.worktree, &["rev-parse", "HEAD"]),
        head_before,
        "final confirm must not create a commit for residue"
    );
    assert!(
        fixture
            .store
            .list_rework_instructions(&prepared.project_id, &prepared.issue_id, &prepared.id)
            .expect("rework instructions")
            .is_empty()
    );
    let lineage = WorkItemRevisionStore::new(fixture.store.paths())
        .get_plan_lineage(
            &prepared.project_id,
            &prepared.issue_id,
            prepared.work_item_group_id.as_deref().expect("plan id"),
        )
        .expect("plan lineage");
    assert!(
        WorkItemRevisionStore::new(fixture.store.paths())
            .list_open_repair_requests(&lineage)
            .expect("repair requests")
            .is_empty()
    );
}
