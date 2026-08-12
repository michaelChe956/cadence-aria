use super::*;
use crate::product::coding_models::{
    CodeReviewReport, CodingExecutionUnit, CodingUnitRun, CodingUnitRunStatus, ReviewFinding,
};
use crate::product::lifecycle_store::CreateWorkspaceSessionInput;
use crate::product::models::{HandoffRevision, WorkspaceType};
use crate::product::work_item_projection::renderer_for;
use crate::web::workspace_ws_types::{
    ArtifactPayload, ArtifactVersion, WorkItemRevisionHistoryDto,
};

pub(super) struct ReadinessFixture {
    pub(super) _root: tempfile::TempDir,
    pub(super) worktree: std::path::PathBuf,
    pub(super) store: CodingAttemptStore,
    pub(super) engine: CodingWorkspaceEngine,
    pub(super) attempt: CodingExecutionAttempt,
    pub(super) start_commit: String,
}

pub(super) fn readiness_fixture() -> ReadinessFixture {
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
            target_snapshot: None,
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

pub(super) fn first_unit(fixture: &ReadinessFixture) -> CodingExecutionUnit {
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

pub(super) fn seed_completed_run(
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

pub(super) fn seed_handoff(
    fixture: &ReadinessFixture,
    unit: &CodingExecutionUnit,
    run: &CodingUnitRun,
) {
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

pub(super) fn seed_handoff_with_commit(
    fixture: &ReadinessFixture,
    unit: &CodingExecutionUnit,
    run: &CodingUnitRun,
    commit_sha: &str,
) {
    seed_handoff_with_id_and_commit(fixture, unit, run, "handoff_revision_0001", commit_sha);
}

pub(super) fn seed_handoff_with_id_and_commit(
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

pub(super) fn seed_runner_revision_history(fixture: &ReadinessFixture) {
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

pub(super) fn seed_complete_group_readiness(fixture: &ReadinessFixture) -> CodingExecutionAttempt {
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

pub(super) fn review_report(
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

pub(super) fn complete_other_units(fixture: &ReadinessFixture, completion: &str) {
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
