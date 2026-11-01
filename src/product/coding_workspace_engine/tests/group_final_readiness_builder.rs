use super::group_final_readiness_support::*;
use super::*;
use crate::product::coding_models::{GroupDependencyGateSnapshot, GroupDependencyGateStatus};
use crate::product::coding_models::{GroupFinalReadinessDiagnosticKind, GroupFinalReadinessStatus};
use crate::web::types::GroupCodingProgressDto;

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

#[tokio::test]
async fn group_work_item_progress_assembles_persisted_completed_running_and_pending_units() {
    let fixture = readiness_fixture();
    let units = fixture
        .store
        .list_coding_units(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .expect("units");
    let completed = &units[0];
    let running = &units[1];
    let pending = &units[2];

    fs::write(fixture.worktree.join("progress.rs"), "completed\n").expect("progress change");
    run_test_git(&fixture.worktree, &["add", "progress.rs"]);
    run_test_git(&fixture.worktree, &["commit", "-m", "progress completed"]);
    let completion_commit = git_stdout(&fixture.worktree, &["rev-parse", "HEAD"])
        .trim()
        .to_string();
    let run = seed_completed_run(
        &fixture,
        completed,
        Some(completion_commit.clone()),
        Vec::new(),
    );
    seed_handoff(&fixture, completed, &run);
    fixture
        .store
        .save_code_review_report(
            &fixture.attempt,
            &review_report(
                &fixture.attempt,
                "progress_review_0001",
                &run,
                "2026-08-07T00:00:00Z",
                ReviewVerdict::Approve,
                "completed review",
            ),
        )
        .expect("review report");
    fixture
        .store
        .update_coding_unit_status(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
            &completed.id,
            CodingExecutionUnitStatus::Completed,
            None,
        )
        .expect("complete unit");
    fixture
        .store
        .update_coding_unit_status(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
            &running.id,
            CodingExecutionUnitStatus::Running,
            None,
        )
        .expect("run unit");
    fixture
        .store
        .update_attempt_head_commit(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
            Some(completion_commit.clone()),
        )
        .expect("attempt head");
    fixture
        .store
        .write_group_dependency_gate_snapshot(
            &fixture.attempt,
            &GroupDependencyGateSnapshot {
                attempt_id: fixture.attempt.id.clone(),
                status: GroupDependencyGateStatus::Waiting,
                selected_unit_id: None,
                pending_unit_ids: vec![pending.id.clone()],
                reason_code: Some("dependency_waiting".to_string()),
                message: Some("waiting for completed upstream handoff".to_string()),
                dependency_unit_id: Some(completed.id.clone()),
                handoff_id: None,
                dependency_work_item_revision_id: Some(completed.work_item_revision_id.clone()),
                handoff_work_item_revision_id: None,
                plan_revision_id: "plan_revision_0001".to_string(),
                created_at: "2026-08-07T00:00:00Z".to_string(),
            },
        )
        .expect("dependency gate snapshot");

    fixture
        .store
        .seed_running_attempt_for_test(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .expect("running attempt");
    fixture
        .store
        .update_attempt_stage(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
            CodingExecutionStage::Coding,
        )
        .expect("coding stage");
    let attempt = fixture
        .store
        .get_attempt(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .expect("reloaded attempt");
    let (progress, aggregate) =
        crate::web::handlers::build_group_work_item_progress(&fixture.store, &attempt)
            .expect("group progress");
    assert_eq!(
        aggregate,
        GroupCodingProgressDto {
            total: 3,
            pending: 1,
            active: 1,
            completed: 1,
            failed_or_blocked: 0,
        }
    );
    let completed_progress = progress
        .iter()
        .find(|item| item.unit_id == completed.id)
        .expect("completed progress");
    assert_eq!(completed_progress.status, "completed");
    assert_eq!(completed_progress.stage, None);
    assert_eq!(
        completed_progress.current_commit.as_deref(),
        Some(completion_commit.as_str())
    );
    assert_eq!(
        completed_progress.final_commit.as_deref(),
        Some(completion_commit.as_str())
    );
    assert_eq!(
        completed_progress
            .code_review
            .as_ref()
            .map(|report| report.id.as_str()),
        Some("progress_review_0001")
    );
    assert_eq!(
        completed_progress.handoff_revision_id.as_deref(),
        Some("handoff_revision_0001")
    );
    assert_eq!(completed_progress.plan_revision_id, "plan_revision_0001");

    let running_progress = progress
        .iter()
        .find(|item| item.unit_id == running.id)
        .expect("running progress");
    assert_eq!(running_progress.status, "running");
    assert_eq!(running_progress.stage.as_deref(), Some("coding"));
    assert_eq!(
        running_progress.current_commit.as_deref(),
        Some(completion_commit.as_str())
    );
    assert_eq!(running_progress.final_commit, None);
    assert_eq!(running_progress.code_review, None);
    assert_eq!(running_progress.handoff_revision_id, None);

    let pending_progress = progress
        .iter()
        .find(|item| item.unit_id == pending.id)
        .expect("pending progress");
    assert_eq!(pending_progress.status, "pending");
    assert_eq!(pending_progress.stage, None);
    assert_eq!(pending_progress.current_commit, None);
    assert_eq!(pending_progress.final_commit, None);
    assert_eq!(
        pending_progress.failure_or_blocked_reason.as_deref(),
        Some("dependency_waiting")
    );
    assert_eq!(pending_progress.plan_revision_id, "plan_revision_0001");
}
