use super::*;
use crate::product::coding_models::{
    CodeReviewReport, CodingExecutionUnit, CodingUnitRun, CodingUnitRunStatus,
    GroupFinalReadinessDiagnosticKind, GroupFinalReadinessStatus, ReviewFinding,
};
use crate::product::models::HandoffRevision;
use crate::product::work_item_projection::renderer_for;

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
    seed_handoff_with_commit(
        fixture,
        unit,
        run,
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
    let revision_store = WorkItemRevisionStore::new(fixture.store.paths());
    let lineage = revision_store
        .get_plan_lineage(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            "work_item_plan_0001",
        )
        .expect("lineage");
    let handoff = HandoffRevision {
        id: "handoff_revision_0001".to_string(),
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
