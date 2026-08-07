use super::*;
use crate::cross_cutting::provider_registry::ProviderRegistry;
use crate::product::coding_models::{
    CodeReviewReport, CodingExecutionUnit, CodingUnitRun, CodingUnitRunStatus,
    GroupFinalReadinessDiagnosticKind, GroupFinalReadinessStatus, ReviewFinding,
};
use crate::product::lifecycle_store::{
    CreateWorkspaceSessionInput, UpsertIssueSharedWorktreeInput,
};
use crate::product::models::HandoffRevision;
use crate::product::models::WorkspaceType;
use crate::product::work_item_projection::renderer_for;
use crate::web::coding_ws_handler::execute_start_coding_flow;
use crate::web::runtime::WebRuntime;
use crate::web::state::WebAppState;
use crate::web::workspace_ws_types::{
    ArtifactPayload, ArtifactVersion, WorkItemRevisionHistoryDto,
};

struct ReadinessFixture {
    _root: tempfile::TempDir,
    worktree: std::path::PathBuf,
    store: CodingAttemptStore,
    engine: CodingWorkspaceEngine,
    attempt: CodingExecutionAttempt,
    start_commit: String,
}

fn readiness_fixture() -> ReadinessFixture {
    let root = tempdir().expect("tempdir");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree");
    init_test_git_repo(&worktree);
    let start_commit = git_stdout(&worktree, &["rev-parse", "HEAD"])
        .trim()
        .to_string();
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_group_attempt(CreateGroupCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            current_work_item_id: "work_item_0001".to_string(),
            base_branch: start_commit.clone(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: Some(worktree.clone()),
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Codex,
                reviewer: Some(ProviderName::ClaudeCode),
                review_rounds: 1,
                permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
            },
            max_auto_rework: 2,
        })
        .expect("group attempt");
    seed_schema_v2_group_attempt_fixture(&store, &attempt, true, false, &[]);
    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    ReadinessFixture {
        _root: root,
        worktree,
        store,
        engine,
        attempt,
        start_commit,
    }
}

fn first_unit(fixture: &ReadinessFixture) -> CodingExecutionUnit {
    fixture
        .store
        .list_coding_units(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .expect("units")
        .into_iter()
        .next()
        .expect("first unit")
}

fn seed_completed_run(
    fixture: &ReadinessFixture,
    unit: &CodingExecutionUnit,
    completion_commit: Option<String>,
    handoffs: Vec<String>,
) -> CodingUnitRun {
    let revision_store = WorkItemRevisionStore::new(fixture.store.paths());
    let lineage = revision_store
        .get_plan_lineage(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            "work_item_plan_0001",
        )
        .expect("lineage");
    let revision = revision_store
        .get_work_item_revision(
            &lineage,
            &unit.logical_work_item_id,
            &unit.work_item_revision_id,
        )
        .expect("revision");
    assert_eq!(
        revision.id, unit.work_item_revision_id,
        "fixture run must use unit's materialized revision"
    );
    let bundle = revision_store
        .get_work_item_projection_bundle(&lineage, &revision.work_item_projection_bundle_id)
        .expect("bundle");
    let providers = fixture
        .store
        .get_role_provider_config_snapshot(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .expect("provider snapshot");
    let coder_renderer = renderer_for(&providers.coder)
        .renderer_version()
        .to_string();
    let reviewer_renderer = renderer_for(&providers.code_reviewer)
        .renderer_version()
        .to_string();
    let run = CodingUnitRun {
        id: format!("coding_unit_run_{}", unit.order_index + 1),
        unit_id: unit.id.clone(),
        execution_no: 1,
        work_item_revision_id: revision.id,
        resolved_handoff_revision_ids: handoffs,
        canonical_contract_hash: bundle.canonical_contract_hash,
        projection_bundle_id: bundle.id,
        projection_compiler_version: bundle.compiler_version,
        coder_provider_renderer_version: coder_renderer,
        reviewer_provider_renderer_version: reviewer_renderer,
        internal_reviewer_provider_renderer_version: None,
        coder_projection_hash: bundle.coder_projection_hash,
        reviewer_projection_hash: bundle.reviewer_projection_hash,
        coder_execution_context_hash: None,
        reviewer_execution_context_hash: None,
        internal_reviewer_execution_context_hash: None,
        status: CodingUnitRunStatus::Completed,
        unit_rework_count: 1,
        verification_retry_count: 0,
        operational_retry_count: 0,
        plan_repair_count: 0,
        start_commit: Some(fixture.start_commit.clone()),
        completion_commit,
        created_at: "2026-08-07T00:00:00Z".to_string(),
        updated_at: "2026-08-07T00:00:00Z".to_string(),
    };
    fixture
        .store
        .create_coding_unit_run(&fixture.attempt, &run)
        .expect("completed run");
    run
}

fn seed_handoff(fixture: &ReadinessFixture, unit: &CodingExecutionUnit, run: &CodingUnitRun) {
    seed_handoff_with_id_and_commit(
        fixture,
        unit,
        run,
        "handoff_revision_0001",
        run.completion_commit
            .as_deref()
            .unwrap_or(fixture.start_commit.as_str()),
    );
}

fn seed_handoff_with_commit(
    fixture: &ReadinessFixture,
    unit: &CodingExecutionUnit,
    run: &CodingUnitRun,
    commit_sha: &str,
) {
    seed_handoff_with_id_and_commit(fixture, unit, run, "handoff_revision_0001", commit_sha);
}

fn seed_handoff_with_id_and_commit(
    fixture: &ReadinessFixture,
    unit: &CodingExecutionUnit,
    run: &CodingUnitRun,
    id: &str,
    commit_sha: &str,
) {
    let revision_store = WorkItemRevisionStore::new(fixture.store.paths());
    let lineage = revision_store
        .get_plan_lineage(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            "work_item_plan_0001",
        )
        .expect("lineage");
    let handoff = HandoffRevision {
        id: id.to_string(),
        logical_work_item_id: unit.logical_work_item_id.clone(),
        work_item_revision_id: run.work_item_revision_id.clone(),
        coding_unit_run_id: run.id.clone(),
        provided_contracts: Vec::new(),
        provided_capabilities: std::collections::BTreeMap::new(),
        contract_hash: "handoff_contract_hash".to_string(),
        commit_sha: commit_sha.to_string(),
        created_at: "2026-08-07T00:00:00Z".to_string(),
    };
    revision_store
        .put_handoff_revision(&lineage, &handoff)
        .expect("handoff");
    fixture
        .store
        .update_coding_unit_latest_handoff_revision_id(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
            &unit.id,
            Some(handoff.id.clone()),
        )
        .expect("latest handoff pointer");
}

fn seed_runner_revision_history(fixture: &ReadinessFixture) {
    let lifecycle = LifecycleStore::new(fixture.store.paths());
    let session = lifecycle
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: fixture.attempt.project_id.clone(),
            issue_id: fixture.attempt.issue_id.clone(),
            entity_id: fixture
                .attempt
                .work_item_group_id
                .clone()
                .expect("group plan id"),
            workspace_type: WorkspaceType::WorkItemPlan,
            author_provider: ProviderName::Codex,
            reviewer_provider: ProviderName::ClaudeCode,
            review_rounds: 1,
            superpowers_enabled: true,
            openspec_enabled: true,
        })
        .expect("plan workspace session");
    lifecycle
        .save_artifact_versions(
            &session.id,
            &[ArtifactVersion {
                version: 1,
                payload: ArtifactPayload::WorkItemRevisionHistory {
                    history: Box::new(WorkItemRevisionHistoryDto {
                        entries: Vec::new(),
                    }),
                },
                generated_by: ProviderName::Codex,
                reviewed_by: None,
                review_verdict: None,
                confirmed_by: None,
                is_current: true,
                created_at: "2026-08-07T00:00:00Z".to_string(),
                source_node_id: "timeline_node_compile".to_string(),
            }],
        )
        .expect("revision history artifact");
}

fn seed_complete_group_readiness(fixture: &ReadinessFixture) -> CodingExecutionAttempt {
    let completion = fixture.start_commit.clone();
    for unit in fixture
        .store
        .list_coding_units(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .expect("units")
    {
        let run = seed_completed_run(fixture, &unit, Some(completion.clone()), Vec::new());
        seed_handoff_with_id_and_commit(
            fixture,
            &unit,
            &run,
            &format!("handoff_revision_{:04}", unit.order_index + 1),
            &completion,
        );
        fixture
            .store
            .save_code_review_report(
                &fixture.attempt,
                &review_report(
                    &fixture.attempt,
                    &format!("code_review_report_{:04}", unit.order_index + 1),
                    &run,
                    "2026-08-07T00:00:00Z",
                    ReviewVerdict::Approve,
                    "independent review approved",
                ),
            )
            .expect("review report");
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
        fixture
            .store
            .update_coding_unit_completion_commit(
                &fixture.attempt.project_id,
                &fixture.attempt.issue_id,
                &fixture.attempt.id,
                &unit.id,
                Some(completion.clone()),
            )
            .expect("unit completion commit");
    }
    let mut attempt = fixture
        .store
        .get_attempt(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .expect("stored attempt");
    attempt.head_commit = Some(completion);
    fixture
        .store
        .save_coding_attempt(&attempt)
        .expect("attempt head");
    fixture
        .store
        .update_attempt_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running attempt")
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

fn review_report(
    attempt: &CodingExecutionAttempt,
    id: &str,
    run: &CodingUnitRun,
    created_at: &str,
    verdict: ReviewVerdict,
    summary: &str,
) -> CodeReviewReport {
    CodeReviewReport {
        id: id.to_string(),
        attempt_id: attempt.id.clone(),
        round: 1,
        verdict,
        findings: vec![ReviewFinding {
            severity: FindingSeverity::Info,
            file_path: Some("rework.rs".to_string()),
            line: Some(1),
            message: format!("review evidence {id}"),
            required_action: None,
            source_stage: CodingExecutionStage::CodeReview,
            evidence: Vec::new(),
            plan_defect_evidence: Vec::new(),
            related_requirements: Vec::new(),
            related_design_constraints: Vec::new(),
            related_work_item_tasks: Vec::new(),
            defect_class: crate::product::models::PlanDefectClass::ImplementationDefect,
            reason_code: None,
            contract_refs: Vec::new(),
            capability_refs: Vec::new(),
            repair_target: None,
            recommended_route: crate::product::models::PlanDefectRoute::CoderRework,
            confidence: None,
        }],
        tested_evidence_refs: Vec::new(),
        diff_refs: Vec::new(),
        summary: summary.to_string(),
        created_at: created_at.to_string(),
        raw_provider_output_ref: Some(format!("provider-raw/{id}.txt")),
        role_run_id: None,
        run_no: Some(1),
        unit_run_id: Some(run.id.clone()),
    }
}

fn complete_other_units(fixture: &ReadinessFixture, completion: &str) {
    for unit in fixture
        .store
        .list_coding_units(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .expect("units")
        .into_iter()
        .skip(1)
    {
        let run = seed_completed_run(fixture, &unit, Some(completion.to_string()), Vec::new());
        let revision_store = WorkItemRevisionStore::new(fixture.store.paths());
        let lineage = revision_store
            .get_plan_lineage(
                &fixture.attempt.project_id,
                &fixture.attempt.issue_id,
                "work_item_plan_0001",
            )
            .expect("lineage");
        let handoff = HandoffRevision {
            id: format!("handoff_revision_{}", unit.order_index + 1),
            logical_work_item_id: unit.logical_work_item_id.clone(),
            work_item_revision_id: run.work_item_revision_id.clone(),
            coding_unit_run_id: run.id.clone(),
            provided_contracts: Vec::new(),
            provided_capabilities: std::collections::BTreeMap::new(),
            contract_hash: format!("handoff_contract_hash_{}", unit.order_index + 1),
            commit_sha: completion.to_string(),
            created_at: "2026-08-07T00:00:00Z".to_string(),
        };
        revision_store
            .put_handoff_revision(&lineage, &handoff)
            .expect("handoff");
        fixture
            .store
            .update_coding_unit_latest_handoff_revision_id(
                &fixture.attempt.project_id,
                &fixture.attempt.issue_id,
                &fixture.attempt.id,
                &unit.id,
                Some(handoff.id),
            )
            .expect("latest handoff pointer");
        fixture
            .store
            .save_code_review_report(
                &fixture.attempt,
                &review_report(
                    &fixture.attempt,
                    &format!("code_review_report_{}", unit.order_index + 1),
                    &run,
                    "2026-08-07T00:00:00Z",
                    ReviewVerdict::Approve,
                    "approved",
                ),
            )
            .expect("review report");
    }
}

#[tokio::test]
async fn readiness_includes_coder_commit_and_rework_commit_for_one_unit_run() {
    let fixture = readiness_fixture();
    let unit = first_unit(&fixture);
    fs::write(fixture.worktree.join("coder.rs"), "coder\n").expect("coder change");
    run_test_git(&fixture.worktree, &["add", "coder.rs"]);
    run_test_git(&fixture.worktree, &["commit", "-m", "coder change"]);
    let coder_commit = git_stdout(&fixture.worktree, &["rev-parse", "HEAD"])
        .trim()
        .to_string();
    fs::write(fixture.worktree.join("rework.rs"), "rework\n").expect("rework change");
    run_test_git(&fixture.worktree, &["add", "rework.rs"]);
    run_test_git(&fixture.worktree, &["commit", "-m", "review rework"]);
    let rework_commit = git_stdout(&fixture.worktree, &["rev-parse", "HEAD"])
        .trim()
        .to_string();
    let run = seed_completed_run(
        &fixture,
        &unit,
        Some(rework_commit.clone()),
        vec!["upstream_handoff_revision_0001".to_string()],
    );
    seed_handoff(&fixture, &unit, &run);
    fixture
        .store
        .save_code_review_report(
            &fixture.attempt,
            &review_report(
                &fixture.attempt,
                "code_review_report_old",
                &run,
                "2026-08-07T00:00:00Z",
                ReviewVerdict::RequestChanges,
                "old review",
            ),
        )
        .expect("old review");
    fixture
        .store
        .save_code_review_report(
            &fixture.attempt,
            &review_report(
                &fixture.attempt,
                "code_review_report_latest",
                &run,
                "2026-08-07T01:00:00Z",
                ReviewVerdict::Approve,
                "latest independent review",
            ),
        )
        .expect("latest review");
    complete_other_units(&fixture, &rework_commit);

    let snapshot = fixture
        .engine
        .build_group_final_readiness_snapshot(&fixture.attempt)
        .await
        .expect("readiness snapshot");
    let unit_snapshot = snapshot.units.first().expect("first snapshot unit");
    assert_eq!(snapshot.status, GroupFinalReadinessStatus::Complete);
    assert_eq!(
        unit_snapshot.commit_shas,
        vec![coder_commit, rework_commit.clone()]
    );
    assert_eq!(
        unit_snapshot.diff_ref,
        format!("{}..{}", fixture.start_commit, rework_commit)
    );
    assert_eq!(
        unit_snapshot.code_review_report_id.as_deref(),
        Some("code_review_report_latest")
    );
    assert_eq!(unit_snapshot.review_verdict, Some(ReviewVerdict::Approve));
    assert_eq!(
        unit_snapshot.review_summary.as_deref(),
        Some("latest independent review")
    );
    assert_eq!(
        unit_snapshot.review_findings,
        Some(vec![ReviewFinding {
            severity: FindingSeverity::Info,
            file_path: Some("rework.rs".to_string()),
            line: Some(1),
            message: "review evidence code_review_report_latest".to_string(),
            required_action: None,
            source_stage: CodingExecutionStage::CodeReview,
            evidence: Vec::new(),
            plan_defect_evidence: Vec::new(),
            related_requirements: Vec::new(),
            related_design_constraints: Vec::new(),
            related_work_item_tasks: Vec::new(),
            defect_class: crate::product::models::PlanDefectClass::ImplementationDefect,
            reason_code: None,
            contract_refs: Vec::new(),
            capability_refs: Vec::new(),
            repair_target: None,
            recommended_route: crate::product::models::PlanDefectRoute::CoderRework,
            confidence: None,
        }]),
    );
    assert_eq!(
        unit_snapshot.review_raw_provider_output_ref.as_deref(),
        Some("provider-raw/code_review_report_latest.txt")
    );
    assert_eq!(
        unit_snapshot.handoff_revision_id.as_deref(),
        Some("handoff_revision_0001")
    );
    assert_eq!(
        unit_snapshot.plan_revision_id.as_deref(),
        Some("plan_revision_0001")
    );
    assert_eq!(
        fixture
            .store
            .get_group_final_readiness_snapshot(&fixture.attempt)
            .expect("persisted snapshot"),
        Some(snapshot),
    );
}

#[tokio::test]
async fn readiness_marks_equal_start_and_completion_as_empty_observation() {
    let fixture = readiness_fixture();
    let unit = first_unit(&fixture);
    fs::write(fixture.worktree.join("empty.diff"), "same final diff\n").expect("change");
    run_test_git(&fixture.worktree, &["add", "empty.diff"]);
    run_test_git(&fixture.worktree, &["commit", "-m", "same final diff"]);
    let final_commit = git_stdout(&fixture.worktree, &["rev-parse", "HEAD"])
        .trim()
        .to_string();
    let run = seed_completed_run(
        &fixture,
        &unit,
        Some(fixture.start_commit.clone()),
        Vec::new(),
    );
    seed_handoff(&fixture, &unit, &run);
    fixture
        .store
        .save_code_review_report(
            &fixture.attempt,
            &review_report(
                &fixture.attempt,
                "code_review_report_0001",
                &run,
                "2026-08-07T00:00:00Z",
                ReviewVerdict::Approve,
                "approved",
            ),
        )
        .expect("review report");
    complete_other_units(&fixture, &final_commit);

    let snapshot = fixture
        .engine
        .build_group_final_readiness_snapshot(&fixture.attempt)
        .await
        .expect("readiness snapshot");
    let unit_snapshot = snapshot.units.first().expect("first snapshot unit");
    assert_eq!(snapshot.status, GroupFinalReadinessStatus::Complete);
    assert!(unit_snapshot.empty_observation);
    assert!(unit_snapshot.commit_shas.is_empty());
    assert!(unit_snapshot.diff_ref.is_empty());
}

#[tokio::test]
async fn readiness_keeps_units_with_missing_run_or_completion_commit() {
    let missing_run = readiness_fixture();
    let snapshot = missing_run
        .engine
        .build_group_final_readiness_snapshot(&missing_run.attempt)
        .await
        .expect("persist incomplete missing-run snapshot");
    let first = snapshot.units.first().expect("first unit");
    assert_eq!(snapshot.status, GroupFinalReadinessStatus::Incomplete);
    assert!(first.unit_run_id.is_none());
    assert!(first.start_commit.is_none());
    assert!(first.completion_commit.is_none());
    assert!(snapshot.diagnostics.iter().any(|diagnostic| {
        diagnostic.kind == GroupFinalReadinessDiagnosticKind::UnitRunMissing
            && diagnostic.unit_id.as_deref() == Some(first.unit_id.as_str())
    }));

    let missing_completion = readiness_fixture();
    let unit = first_unit(&missing_completion);
    let run = seed_completed_run(&missing_completion, &unit, None, Vec::new());
    let snapshot = missing_completion
        .engine
        .build_group_final_readiness_snapshot(&missing_completion.attempt)
        .await
        .expect("persist incomplete missing-completion snapshot");
    let first = snapshot.units.first().expect("first unit");
    assert_eq!(first.unit_run_id.as_deref(), Some(run.id.as_str()));
    assert_eq!(
        first.start_commit.as_deref(),
        Some(missing_completion.start_commit.as_str())
    );
    assert!(first.completion_commit.is_none());
    assert!(snapshot.diagnostics.iter().any(|diagnostic| {
        diagnostic.kind == GroupFinalReadinessDiagnosticKind::CompletionCommitMissing
            && diagnostic.unit_id.as_deref() == Some(first.unit_id.as_str())
    }));
}

#[tokio::test]
async fn readiness_marks_mismatched_published_handoff_as_identity_mismatch() {
    let fixture = readiness_fixture();
    let unit = first_unit(&fixture);
    let run = seed_completed_run(
        &fixture,
        &unit,
        Some(fixture.start_commit.clone()),
        Vec::new(),
    );
    seed_handoff_with_commit(&fixture, &unit, &run, "wrong_completion_commit");
    fixture
        .store
        .save_code_review_report(
            &fixture.attempt,
            &review_report(
                &fixture.attempt,
                "code_review_report_0001",
                &run,
                "2026-08-07T00:00:00Z",
                ReviewVerdict::Approve,
                "approved",
            ),
        )
        .expect("review report");

    let snapshot = fixture
        .engine
        .build_group_final_readiness_snapshot(&fixture.attempt)
        .await
        .expect("persist incomplete identity-mismatch snapshot");
    let first = snapshot.units.first().expect("first unit");
    assert_eq!(snapshot.status, GroupFinalReadinessStatus::Incomplete);
    assert!(first.handoff_revision_id.is_none());
    assert!(snapshot.diagnostics.iter().any(|diagnostic| {
        diagnostic.kind == GroupFinalReadinessDiagnosticKind::IdentityMismatch
            && diagnostic.unit_id.as_deref() == Some(first.unit_id.as_str())
    }));
}

#[tokio::test]
async fn readiness_marks_missing_published_handoff_as_handoff_missing() {
    let fixture = readiness_fixture();
    let unit = first_unit(&fixture);
    let run = seed_completed_run(
        &fixture,
        &unit,
        Some(fixture.start_commit.clone()),
        Vec::new(),
    );
    fixture
        .store
        .update_coding_unit_latest_handoff_revision_id(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
            &unit.id,
            Some("handoff_revision_missing".to_string()),
        )
        .expect("missing latest handoff pointer");
    fixture
        .store
        .save_code_review_report(
            &fixture.attempt,
            &review_report(
                &fixture.attempt,
                "code_review_report_0001",
                &run,
                "2026-08-07T00:00:00Z",
                ReviewVerdict::Approve,
                "approved",
            ),
        )
        .expect("review report");

    let snapshot = fixture
        .engine
        .build_group_final_readiness_snapshot(&fixture.attempt)
        .await
        .expect("persist incomplete missing-handoff snapshot");
    let first = snapshot.units.first().expect("first unit");
    assert_eq!(snapshot.status, GroupFinalReadinessStatus::Incomplete);
    assert!(first.handoff_revision_id.is_none());
    assert!(snapshot.diagnostics.iter().any(|diagnostic| {
        diagnostic.kind == GroupFinalReadinessDiagnosticKind::HandoffMissing
            && diagnostic.unit_id.as_deref() == Some(first.unit_id.as_str())
    }));
}

#[tokio::test]
async fn readiness_persists_diagnostic_for_missing_review_handoff_or_binding() {
    for (case, omit_review, omit_handoff, replace_binding) in [
        ("review", true, false, false),
        ("handoff", false, true, false),
        ("binding", false, false, true),
    ] {
        let fixture = readiness_fixture();
        let unit = first_unit(&fixture);
        let run = seed_completed_run(
            &fixture,
            &unit,
            Some(fixture.start_commit.clone()),
            Vec::new(),
        );
        if !omit_handoff {
            seed_handoff(&fixture, &unit, &run);
        }
        if !omit_review {
            fixture
                .store
                .save_code_review_report(
                    &fixture.attempt,
                    &review_report(
                        &fixture.attempt,
                        "code_review_report_0001",
                        &run,
                        "2026-08-07T00:00:00Z",
                        ReviewVerdict::Approve,
                        "approved",
                    ),
                )
                .expect("review report");
        }
        if replace_binding {
            let mut binding = fixture
                .store
                .get_plan_binding(&fixture.attempt)
                .expect("binding");
            binding.bound_plan_revision_id = "plan_revision_mismatch".to_string();
            let path = fixture
                .store
                .attempt_dir(
                    &fixture.attempt.project_id,
                    &fixture.attempt.issue_id,
                    &fixture.attempt.id,
                )
                .join("plan-binding.json");
            crate::product::json_store::write_json(&path, &binding).expect("replace binding");
        }

        let snapshot = fixture
            .engine
            .build_group_final_readiness_snapshot(&fixture.attempt)
            .await
            .unwrap_or_else(|error| panic!("{case}: readiness snapshot: {error}"));
        assert_eq!(
            snapshot.status,
            GroupFinalReadinessStatus::Incomplete,
            "{case}"
        );
        let expected = match case {
            "review" => GroupFinalReadinessDiagnosticKind::CodeReviewMissing,
            "handoff" => GroupFinalReadinessDiagnosticKind::HandoffMissing,
            "binding" => GroupFinalReadinessDiagnosticKind::PlanBindingMismatch,
            _ => unreachable!(),
        };
        assert!(
            snapshot
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.kind == expected),
            "{case}: {:#?}",
            snapshot.diagnostics
        );
        assert_eq!(
            fixture
                .store
                .get_group_final_readiness_snapshot(&fixture.attempt)
                .expect("persisted snapshot"),
            Some(snapshot),
            "{case}"
        );
    }
}
