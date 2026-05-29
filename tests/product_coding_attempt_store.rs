use std::path::PathBuf;

use cadence_aria::product::app_paths::ProductAppPaths;
use cadence_aria::product::coding_attempt_store::{CodingAttemptStore, CreateCodingAttemptInput};
use cadence_aria::product::coding_models::{
    CodeReviewReport, CodingAgentRole, CodingAttemptStatus, CodingContextNote,
    CodingExecutionStage, CodingProviderRole, CodingReworkInstruction,
    CodingRoleProviderConfigSnapshot, CodingStageGateStatus, CodingTimelineNode,
    CodingTimelineNodeStatus, FindingSeverity, InternalPrReview, PushStatus, RemoteKind,
    ReviewFinding, ReviewRequest, ReviewRequestKind, ReviewVerdict, TestCommand, TestCommandStatus,
    TestingOverallStatus, TestingReport,
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
fn store_persists_context_notes_in_attempt_scope() {
    let root = tempdir().expect("tempdir");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(create_input("work_item_0001"))
        .expect("create attempt");

    let note = store
        .create_context_note(&attempt.id, "请优先使用 unittest".to_string())
        .expect("create context note");

    assert_eq!(
        note,
        CodingContextNote {
            id: "coding_context_note_0001".to_string(),
            attempt_id: attempt.id.clone(),
            content: "请优先使用 unittest".to_string(),
            created_at: note.created_at.clone(),
            consumed_by_rework_round: None,
        }
    );
    assert_eq!(
        store
            .list_context_notes("project_0001", "issue_0001", &attempt.id)
            .expect("list context notes"),
        vec![note]
    );
}

#[test]
fn store_lists_unconsumed_context_notes_and_marks_rework_round() {
    let root = tempdir().expect("tempdir");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(create_input("work_item_0001"))
        .expect("create attempt");

    let first = store
        .create_context_note(&attempt.id, "第一次补充".to_string())
        .expect("first context note");
    let second = store
        .create_context_note(&attempt.id, "第二次补充".to_string())
        .expect("second context note");

    store
        .mark_context_notes_consumed(
            "project_0001",
            "issue_0001",
            &attempt.id,
            std::slice::from_ref(&first.id),
            1,
        )
        .expect("mark first consumed");

    let unconsumed = store
        .list_unconsumed_context_notes("project_0001", "issue_0001", &attempt.id)
        .expect("list unconsumed");
    assert_eq!(unconsumed, vec![second.clone()]);

    store
        .mark_context_notes_consumed(
            "project_0001",
            "issue_0001",
            &attempt.id,
            std::slice::from_ref(&second.id),
            2,
        )
        .expect("mark second consumed");

    let notes = store
        .list_context_notes("project_0001", "issue_0001", &attempt.id)
        .expect("list notes");
    assert_eq!(notes[0].consumed_by_rework_round, Some(1));
    assert_eq!(notes[1].consumed_by_rework_round, Some(2));
    assert!(
        store
            .list_unconsumed_context_notes("project_0001", "issue_0001", &attempt.id)
            .expect("list unconsumed after all consumed")
            .is_empty()
    );
}

#[test]
fn saves_reads_and_consumes_latest_coding_rework_instruction() {
    let root = tempdir().expect("tempdir");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(create_input("work_item_0001"))
        .expect("create attempt");
    let first = CodingReworkInstruction {
        id: "coding_rework_instruction_0001".to_string(),
        attempt_id: attempt.id.clone(),
        source_stage: CodingExecutionStage::Testing,
        rework_round: 1,
        summary: "测试失败".to_string(),
        fix_hints: vec!["修复 failing test".to_string()],
        questions: Vec::new(),
        created_at: "2026-05-29T00:00:00Z".to_string(),
        consumed_by_node_id: None,
        consumed_at: None,
    };
    let second = CodingReworkInstruction {
        id: "coding_rework_instruction_0002".to_string(),
        attempt_id: attempt.id.clone(),
        source_stage: CodingExecutionStage::CodeReview,
        rework_round: 2,
        summary: "移除运行产物".to_string(),
        fix_hints: vec!["不要提交 __pycache__".to_string()],
        questions: vec!["确认 diff 只包含业务文件".to_string()],
        created_at: "2026-05-29T00:01:00Z".to_string(),
        consumed_by_node_id: None,
        consumed_at: None,
    };

    store
        .save_rework_instruction(&first)
        .expect("save first instruction");
    assert_eq!(
        store
            .latest_unconsumed_rework_instruction("project_0001", "issue_0001", &attempt.id)
            .expect("latest first"),
        Some(first.clone())
    );
    store
        .mark_rework_instruction_consumed(
            "project_0001",
            "issue_0001",
            &attempt.id,
            &first.id,
            "coding_node_0002",
        )
        .expect("consume first instruction");
    assert_eq!(
        store
            .latest_unconsumed_rework_instruction("project_0001", "issue_0001", &attempt.id)
            .expect("latest after first consume"),
        None
    );
    store
        .save_rework_instruction(&second)
        .expect("save second instruction");

    assert_eq!(
        store
            .latest_unconsumed_rework_instruction("project_0001", "issue_0001", &attempt.id)
            .expect("latest unconsumed"),
        Some(second.clone())
    );

    let consumed = store
        .mark_rework_instruction_consumed(
            "project_0001",
            "issue_0001",
            &attempt.id,
            &second.id,
            "coding_node_0003",
        )
        .expect("consume instruction");

    assert_eq!(
        consumed.consumed_by_node_id.as_deref(),
        Some("coding_node_0003")
    );
    assert!(consumed.consumed_at.is_some());
    assert_eq!(
        store
            .latest_unconsumed_rework_instruction("project_0001", "issue_0001", &attempt.id)
            .expect("latest after consume"),
        None
    );
}

#[test]
fn store_persists_role_provider_config_snapshot_in_attempt_scope() {
    let root = tempdir().expect("tempdir");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(create_input("work_item_0001"))
        .expect("create attempt");

    let initial = store
        .get_role_provider_config_snapshot("project_0001", "issue_0001", &attempt.id)
        .expect("initial role provider snapshot");
    assert_eq!(initial.coder, ProviderName::Fake);
    assert_eq!(initial.tester, ProviderName::Fake);
    assert_eq!(initial.code_reviewer, ProviderName::Fake);

    let updated = CodingRoleProviderConfigSnapshot {
        coder: ProviderName::Fake,
        tester: ProviderName::Codex,
        analyst: ProviderName::Fake,
        code_reviewer: ProviderName::Codex,
        internal_reviewer: ProviderName::Fake,
        review_rounds: 1,
    };
    store
        .update_role_provider_config_snapshot(
            "project_0001",
            "issue_0001",
            &attempt.id,
            updated.clone(),
        )
        .expect("update role provider snapshot");

    assert_eq!(
        store
            .get_role_provider_config_snapshot("project_0001", "issue_0001", &attempt.id)
            .expect("updated role provider snapshot"),
        updated
    );
}

#[test]
fn store_persists_and_resolves_stage_gates_in_attempt_scope() {
    let root = tempdir().expect("tempdir");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(create_input("work_item_0001"))
        .expect("create attempt");

    let provider_snapshot = CodingRoleProviderConfigSnapshot::from(ProviderConfigSnapshot {
        author: ProviderName::Codex,
        reviewer: Some(ProviderName::Fake),
        review_rounds: 1,
    });
    let gate = store
        .create_stage_gate(
            &attempt.id,
            CodingExecutionStage::Testing,
            CodingProviderRole::Tester,
            "2026-05-28T00:00:05Z".to_string(),
            provider_snapshot.clone(),
        )
        .expect("create stage gate");

    assert_eq!(gate.gate_id, "coding_stage_gate_0001");
    assert_eq!(gate.attempt_id, attempt.id);
    assert_eq!(gate.stage, CodingExecutionStage::Testing);
    assert_eq!(gate.role, CodingProviderRole::Tester);
    assert_eq!(gate.provider_snapshot, provider_snapshot);
    assert_eq!(gate.status, CodingStageGateStatus::Open);
    assert_eq!(
        store
            .list_open_stage_gates("project_0001", "issue_0001", &attempt.id)
            .expect("open stage gates")
            .len(),
        1
    );

    let confirmed = store
        .update_stage_gate_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            &gate.gate_id,
            CodingStageGateStatus::Confirmed,
        )
        .expect("confirm stage gate");

    assert_eq!(confirmed.status, CodingStageGateStatus::Confirmed);
    assert!(
        store
            .list_open_stage_gates("project_0001", "issue_0001", &attempt.id)
            .expect("open stage gates")
            .is_empty()
    );
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
        impact_scope: vec!["src/lib.rs".to_string()],
        pr_description: "实现 work item".to_string(),
        commit_message_suggestion: "feat: implement work item".to_string(),
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
