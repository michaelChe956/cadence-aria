use std::path::PathBuf;

use cadence_aria::product::coding_models::{
    CodeReviewReport, CodingAgentRole, CodingAttemptStatus, CodingExecutionAttempt,
    CodingExecutionStage, CodingGateAction, CodingGateActionType, CodingGateKind,
    CodingGateRequired, CodingTimelineNode, CodingTimelineNodeStatus, FindingSeverity,
    InternalPrReview, PushStatus, RemoteKind, ReviewFinding, ReviewRequest, ReviewRequestKind,
    ReviewVerdict, TestCommand, TestCommandStatus, TestingOverallStatus, TestingReport,
};
use cadence_aria::product::models::ProviderName;
use cadence_aria::web::workspace_ws_types::ProviderConfigSnapshot;
use serde_json::json;

#[test]
fn coding_attempt_serializes_stage_status_and_provider_snapshot() {
    let attempt = CodingExecutionAttempt {
        id: "coding_attempt_0001".to_string(),
        project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
        work_item_id: "work_item_0001".to_string(),
        attempt_no: 1,
        status: CodingAttemptStatus::Created,
        stage: CodingExecutionStage::PrepareContext,
        base_branch: "main".to_string(),
        branch_name: "aria/work-items/work_item_0001/attempt-1".to_string(),
        worktree_path: None,
        provider_config_snapshot: ProviderConfigSnapshot {
            author: ProviderName::Fake,
            reviewer: Some(ProviderName::Codex),
            review_rounds: 1,
        },
        rework_count: 0,
        max_auto_rework: 2,
        head_commit: None,
        pushed_remote: None,
        review_request_id: None,
        created_at: "2026-05-23T00:00:00Z".to_string(),
        updated_at: "2026-05-23T00:00:00Z".to_string(),
        completed_at: None,
    };

    let value = serde_json::to_value(&attempt).expect("serialize attempt");

    assert_eq!(value["status"], "created");
    assert_eq!(value["stage"], "prepare_context");
    assert_eq!(value["provider_config_snapshot"]["author"], "fake");

    let decoded: CodingExecutionAttempt =
        serde_json::from_value(value).expect("deserialize attempt");
    assert_eq!(decoded.status, CodingAttemptStatus::Created);
    assert_eq!(decoded.stage, CodingExecutionStage::PrepareContext);
}

#[test]
fn testing_and_review_reports_preserve_backend_evidence() {
    let command = TestCommand {
        command: vec!["cargo".to_string(), "test".to_string()],
        cwd: PathBuf::from("/tmp/worktree"),
        exit_code: Some(0),
        duration_ms: 1234,
        stdout_ref: "artifacts/stdout.txt".to_string(),
        stderr_ref: "artifacts/stderr.txt".to_string(),
        status: TestCommandStatus::Passed,
    };
    let testing = TestingReport {
        id: "testing_report_0001".to_string(),
        attempt_id: "coding_attempt_0001".to_string(),
        commands: vec![command],
        overall_status: TestingOverallStatus::Passed,
        provider_claim: Some(json!({"claimed": true})),
        backend_verified: true,
        started_at: "2026-05-23T00:01:00Z".to_string(),
        completed_at: Some("2026-05-23T00:02:00Z".to_string()),
    };
    let finding = ReviewFinding {
        severity: FindingSeverity::Warning,
        file_path: Some("src/lib.rs".to_string()),
        line: Some(42),
        message: "需要补充边界测试".to_string(),
        required_action: Some("添加 n=0 用例".to_string()),
        source_stage: CodingExecutionStage::CodeReview,
    };
    let code_review = CodeReviewReport {
        id: "code_review_0001".to_string(),
        attempt_id: "coding_attempt_0001".to_string(),
        round: 1,
        verdict: ReviewVerdict::RequestChanges,
        findings: vec![finding.clone()],
        tested_evidence_refs: vec!["testing_report_0001".to_string()],
        diff_refs: vec!["diff_0001".to_string()],
        summary: "需要返工".to_string(),
        created_at: "2026-05-23T00:03:00Z".to_string(),
    };
    let internal = InternalPrReview {
        id: "internal_review_0001".to_string(),
        attempt_id: "coding_attempt_0001".to_string(),
        review_request_id: "review_request_0001".to_string(),
        verdict: ReviewVerdict::Approve,
        findings: vec![finding],
        tested_evidence_refs: vec!["testing_report_0001".to_string()],
        diff_refs: vec!["diff_0001".to_string()],
        summary: "可以合入".to_string(),
        created_at: "2026-05-23T00:04:00Z".to_string(),
    };

    assert_eq!(
        serde_json::to_value(&testing).unwrap()["backend_verified"],
        true
    );
    assert_eq!(
        serde_json::to_value(&code_review).unwrap()["verdict"],
        "request_changes"
    );
    assert_eq!(
        serde_json::to_value(&internal).unwrap()["verdict"],
        "approve"
    );
}

#[test]
fn review_request_timeline_and_gate_actions_use_stable_wire_values() {
    let review_request = ReviewRequest {
        id: "review_request_0001".to_string(),
        attempt_id: "coding_attempt_0001".to_string(),
        kind: ReviewRequestKind::GitBranchOnly,
        remote_kind: RemoteKind::GenericGit,
        remote: "origin".to_string(),
        base_branch: "main".to_string(),
        branch_name: "aria/work-items/work_item_0001/attempt-1".to_string(),
        commit_sha: "abc123".to_string(),
        push_status: PushStatus::Pushed,
        external_url: None,
        manual_instructions: vec!["手动打开 review branch".to_string()],
        created_at: "2026-05-23T00:05:00Z".to_string(),
        updated_at: "2026-05-23T00:05:00Z".to_string(),
    };
    let node = CodingTimelineNode {
        id: "coding_node_0001".to_string(),
        attempt_id: "coding_attempt_0001".to_string(),
        stage: CodingExecutionStage::ReviewRequest,
        title: "创建 Review Request".to_string(),
        status: CodingTimelineNodeStatus::Running,
        agent_role: Some(CodingAgentRole::Git),
        summary: None,
        started_at: "2026-05-23T00:05:00Z".to_string(),
        completed_at: None,
        artifact_refs: vec!["review_request_0001".to_string()],
    };
    let gate = CodingGateRequired {
        gate_id: "gate_0001".to_string(),
        kind: CodingGateKind::Blocked,
        title: "Push 失败".to_string(),
        description: "需要用户选择下一步".to_string(),
        available_actions: vec![CodingGateAction {
            action_id: "retry".to_string(),
            label: "重试 Push".to_string(),
            action_type: CodingGateActionType::RetryPush,
        }],
    };

    assert_eq!(
        serde_json::to_value(&review_request).unwrap()["kind"],
        "git_branch_only"
    );
    assert_eq!(serde_json::to_value(&node).unwrap()["agent_role"], "git");
    assert_eq!(
        serde_json::to_value(&gate).unwrap()["available_actions"][0]["action_type"],
        "retry_push"
    );
}
