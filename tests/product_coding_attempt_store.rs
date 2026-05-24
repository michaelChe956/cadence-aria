use std::path::PathBuf;

use cadence_aria::product::app_paths::ProductAppPaths;
use cadence_aria::product::coding_attempt_store::{CodingAttemptStore, CreateCodingAttemptInput};
use cadence_aria::product::coding_models::{
    CodeReviewReport, CodingAgentRole, CodingAttemptStatus, CodingExecutionStage,
    CodingTimelineNode, CodingTimelineNodeStatus, FindingSeverity, InternalPrReview, PushStatus,
    RemoteKind, ReviewFinding, ReviewRequest, ReviewRequestKind, ReviewVerdict, TestCommand,
    TestCommandStatus, TestingOverallStatus, TestingReport,
};
use cadence_aria::product::models::ProviderName;
use cadence_aria::web::workspace_ws_types::ProviderConfigSnapshot;
use tempfile::tempdir;

#[test]
fn create_attempt_assigns_attempt_number_and_blocks_active_attempts() {
    let root = tempdir().expect("tempdir");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));

    let first = store
        .create_attempt(create_input("work_item_0001"))
        .expect("create first attempt");
    assert_eq!(first.id, "coding_attempt_0001");
    assert_eq!(first.attempt_no, 1);
    assert_eq!(first.status, CodingAttemptStatus::Created);
    assert_eq!(first.stage, CodingExecutionStage::PrepareContext);

    let active = store
        .get_active_attempt("project_0001", "issue_0001", "work_item_0001")
        .expect("active lookup")
        .expect("active attempt");
    assert_eq!(active.id, first.id);

    let duplicate = store.create_attempt(create_input("work_item_0001"));
    assert!(
        duplicate.is_err(),
        "active created attempt should block duplicates"
    );

    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &first.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &first.id,
            CodingAttemptStatus::Blocked,
        )
        .expect("blocked");
    assert!(
        store
            .create_attempt(create_input("work_item_0001"))
            .is_err(),
        "blocked attempt is still resumable and active"
    );

    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &first.id,
            CodingAttemptStatus::Aborted,
        )
        .expect("aborted");
    let second = store
        .create_attempt(create_input("work_item_0001"))
        .expect("create second attempt after terminal status");
    assert_eq!(second.id, "coding_attempt_0002");
    assert_eq!(second.attempt_no, 2);
}

#[test]
fn store_persists_reports_reviews_and_timeline_for_snapshot_recovery() {
    let root = tempdir().expect("tempdir");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(create_input("work_item_0001"))
        .expect("create attempt");

    let testing = sample_testing_report(&attempt.id);
    let code_review = sample_code_review_report(&attempt.id);
    let review_request = sample_review_request(&attempt.id);
    let internal_review = sample_internal_review(&attempt.id, &review_request.id);
    let node = sample_node(&attempt.id);

    store
        .save_testing_report(&testing)
        .expect("save testing report");
    store
        .save_code_review_report(&code_review)
        .expect("save code review");
    store
        .save_review_request(&review_request)
        .expect("save review request");
    store
        .save_internal_pr_review(&internal_review)
        .expect("save internal review");
    store.save_timeline_node(node.clone()).expect("save node");

    assert_eq!(
        store
            .get_testing_report(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                &testing.id
            )
            .expect("load testing"),
        testing
    );
    assert_eq!(
        store
            .list_code_review_reports(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("load code reviews"),
        vec![code_review]
    );
    assert_eq!(
        store
            .get_review_request(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                &review_request.id,
            )
            .expect("load review request"),
        review_request
    );
    assert_eq!(
        store
            .get_internal_pr_review(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                &internal_review.id,
            )
            .expect("load internal review"),
        internal_review
    );

    store
        .update_timeline_node_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            &node.id,
            CodingTimelineNodeStatus::Completed,
            Some("完成".to_string()),
            Some("2026-05-23T00:02:00Z".to_string()),
        )
        .expect("update node");
    let nodes = store
        .get_timeline_nodes("project_0001", "issue_0001", &attempt.id)
        .expect("load nodes");
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0].status, CodingTimelineNodeStatus::Completed);
    assert_eq!(nodes[0].summary.as_deref(), Some("完成"));
}

#[test]
fn status_and_stage_transitions_reject_invalid_backwards_moves() {
    let root = tempdir().expect("tempdir");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(create_input("work_item_0001"))
        .expect("create attempt");

    assert!(
        store
            .update_attempt_status(
                "project_0001",
                "issue_0001",
                &attempt.id,
                CodingAttemptStatus::Completed,
            )
            .is_err(),
        "created cannot jump directly to completed"
    );

    store
        .update_attempt_stage(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingExecutionStage::Testing,
        )
        .expect("forward stage");
    assert!(
        store
            .update_attempt_stage(
                "project_0001",
                "issue_0001",
                &attempt.id,
                CodingExecutionStage::Coding,
            )
            .is_err(),
        "stage cannot move backwards outside rework"
    );
    store
        .update_attempt_stage(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingExecutionStage::Rework,
        )
        .expect("enter rework");
}

fn create_input(work_item_id: &str) -> CreateCodingAttemptInput {
    CreateCodingAttemptInput {
        project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
        work_item_id: work_item_id.to_string(),
        base_branch: "main".to_string(),
        branch_name: format!("aria/work-items/{work_item_id}/attempt-1"),
        worktree_path: None,
        provider_config_snapshot: ProviderConfigSnapshot {
            author: ProviderName::Fake,
            reviewer: Some(ProviderName::Fake),
            review_rounds: 1,
        },
        max_auto_rework: 2,
    }
}

fn sample_testing_report(attempt_id: &str) -> TestingReport {
    TestingReport {
        id: "testing_report_0001".to_string(),
        attempt_id: attempt_id.to_string(),
        commands: vec![TestCommand {
            command: vec!["cargo".to_string(), "test".to_string()],
            cwd: PathBuf::from("/tmp/worktree"),
            exit_code: Some(0),
            duration_ms: 100,
            stdout_ref: "artifacts/stdout.txt".to_string(),
            stderr_ref: "artifacts/stderr.txt".to_string(),
            status: TestCommandStatus::Passed,
        }],
        overall_status: TestingOverallStatus::Passed,
        provider_claim: None,
        backend_verified: true,
        started_at: "2026-05-23T00:00:00Z".to_string(),
        completed_at: Some("2026-05-23T00:01:00Z".to_string()),
    }
}

fn sample_code_review_report(attempt_id: &str) -> CodeReviewReport {
    CodeReviewReport {
        id: "code_review_0001".to_string(),
        attempt_id: attempt_id.to_string(),
        round: 1,
        verdict: ReviewVerdict::Approve,
        findings: vec![sample_finding()],
        tested_evidence_refs: vec!["testing_report_0001".to_string()],
        diff_refs: vec!["diff_0001".to_string()],
        summary: "通过".to_string(),
        created_at: "2026-05-23T00:01:00Z".to_string(),
    }
}

fn sample_review_request(attempt_id: &str) -> ReviewRequest {
    ReviewRequest {
        id: "review_request_0001".to_string(),
        attempt_id: attempt_id.to_string(),
        kind: ReviewRequestKind::GitBranchOnly,
        remote_kind: RemoteKind::GenericGit,
        remote: "origin".to_string(),
        base_branch: "main".to_string(),
        branch_name: "aria/work-items/work_item_0001/attempt-1".to_string(),
        commit_sha: "abc123".to_string(),
        push_status: PushStatus::Pushed,
        external_url: None,
        manual_instructions: vec![],
        created_at: "2026-05-23T00:02:00Z".to_string(),
        updated_at: "2026-05-23T00:02:00Z".to_string(),
    }
}

fn sample_internal_review(attempt_id: &str, review_request_id: &str) -> InternalPrReview {
    InternalPrReview {
        id: "internal_review_0001".to_string(),
        attempt_id: attempt_id.to_string(),
        review_request_id: review_request_id.to_string(),
        verdict: ReviewVerdict::Approve,
        findings: vec![sample_finding()],
        tested_evidence_refs: vec!["testing_report_0001".to_string()],
        diff_refs: vec!["diff_0001".to_string()],
        summary: "最终审查通过".to_string(),
        created_at: "2026-05-23T00:03:00Z".to_string(),
    }
}

fn sample_finding() -> ReviewFinding {
    ReviewFinding {
        severity: FindingSeverity::Info,
        file_path: Some("src/lib.rs".to_string()),
        line: Some(1),
        message: "ok".to_string(),
        required_action: None,
        source_stage: CodingExecutionStage::CodeReview,
    }
}

fn sample_node(attempt_id: &str) -> CodingTimelineNode {
    CodingTimelineNode {
        id: "coding_node_0001".to_string(),
        attempt_id: attempt_id.to_string(),
        stage: CodingExecutionStage::Testing,
        title: "测试".to_string(),
        status: CodingTimelineNodeStatus::Running,
        agent_role: Some(CodingAgentRole::Tester),
        summary: None,
        started_at: "2026-05-23T00:01:00Z".to_string(),
        completed_at: None,
        artifact_refs: vec![],
    }
}
