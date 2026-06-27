use crate::product::coding_models::{CodingAttemptStatus, CodingExecutionStage};
use crate::product::models::{
    ProviderName, WorkspaceMessageRecord, WorkspaceSessionRecord, WorkspaceSessionStatus,
    WorkspaceType,
};
use crate::product::test_executor::planned_test_commands_from_markdown;

use super::{
    CodingExecutionAttempt, CodingWsInMessage, ProviderConfigSnapshot,
    is_coding_ws_message_allowed, select_work_item_markdown,
    should_resume_runner_after_gate_response,
};

#[test]
fn falls_back_to_assistant_artifact_when_persisted_markdown_lacks_commands() {
    let session = WorkspaceSessionRecord {
        id: "workspace_session_0001".to_string(),
        project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
        entity_id: "work_item_0001".to_string(),
        workspace_type: WorkspaceType::WorkItem,
        status: WorkspaceSessionStatus::Confirmed,
        author_provider: ProviderName::Codex,
        reviewer_provider: ProviderName::ClaudeCode,
        review_rounds: 1,
        superpowers_enabled: true,
        openspec_enabled: true,
        provider_conversations: Vec::new(),
        messages: vec![WorkspaceMessageRecord {
            role: "assistant".to_string(),
            content: "```artifact\n# Work Item\n\n## 验证命令\n\n```bash\nuv run python -m unittest discover -s tests -v\n```\n```"
                .to_string(),
            created_at: "2026-05-28T00:00:00Z".to_string(),
        }],
        created_at: "2026-05-28T00:00:00Z".to_string(),
        updated_at: "2026-05-28T00:00:00Z".to_string(),
    };

    let selected = select_work_item_markdown(
        Some("# Work Item\n\n## 验证命令\n\n首选无第三方测试依赖命令：".to_string()),
        &session,
    )
    .expect("selected markdown");

    assert!(selected.contains("uv run python -m unittest discover -s tests -v"));
    assert_eq!(
        planned_test_commands_from_markdown(&selected)[0].command,
        vec![
            "uv", "run", "python", "-m", "unittest", "discover", "-s", "tests", "-v"
        ]
    );
}

#[test]
fn blocked_attempt_allows_gate_response_messages() {
    assert!(is_coding_ws_message_allowed(
        &CodingAttemptStatus::Blocked,
        &CodingExecutionStage::Testing,
        &CodingWsInMessage::GateResponse {
            gate_id: "coding_blocked_gate_0001".to_string(),
            action_id: "retry_test_plan".to_string(),
            extra_context: None,
        },
    ));
    assert!(is_coding_ws_message_allowed(
        &CodingAttemptStatus::Blocked,
        &CodingExecutionStage::Testing,
        &CodingWsInMessage::AbortAttempt,
    ));
}

#[test]
fn manual_continue_gate_response_does_not_auto_resume_runner() {
    let mut attempt = CodingExecutionAttempt {
        id: "coding_attempt_0001".to_string(),
        project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
        work_item_id: "work_item_0001".to_string(),
        attempt_no: 1,
        scope: crate::product::coding_models::CodingAttemptScope::WorkItem,
        status: CodingAttemptStatus::Blocked,
        stage: CodingExecutionStage::Rework,
        base_branch: "HEAD".to_string(),
        branch_name: "aria/work-items/work_item_0001/attempt-1".to_string(),
        worktree_path: None,
        provider_config_snapshot: ProviderConfigSnapshot {
            author: ProviderName::Fake,
            reviewer: Some(ProviderName::Fake),
            review_rounds: 1,
        },
        provider_conversations: Vec::new(),
        rework_count: 2,
        max_auto_rework: 2,
        work_item_group_id: None,
        current_work_item_id: Some("work_item_0001".to_string()),
        active_unit_id: None,
        head_commit: None,
        pushed_remote: None,
        review_request_id: None,
        created_at: "2026-06-12T00:00:00Z".to_string(),
        updated_at: "2026-06-12T00:00:00Z".to_string(),
        completed_at: None,
    };

    assert!(!should_resume_runner_after_gate_response(
        "manual_continue",
        &attempt
    ));
    assert!(!should_resume_runner_after_gate_response(
        "accept_risk",
        &attempt
    ));
    assert!(should_resume_runner_after_gate_response(
        "retry_test_plan",
        &attempt
    ));
    assert!(should_resume_runner_after_gate_response(
        "retry_internal_review",
        &attempt
    ));

    attempt.status = CodingAttemptStatus::WaitingForHuman;
    assert!(should_resume_runner_after_gate_response(
        "retry_analyst",
        &attempt
    ));

    attempt.status = CodingAttemptStatus::Running;
    assert!(!should_resume_runner_after_gate_response(
        "retry_test_plan",
        &attempt
    ));
}

#[test]
fn waiting_rework_attempt_allows_continue_rework_message() {
    assert!(is_coding_ws_message_allowed(
        &CodingAttemptStatus::WaitingForHuman,
        &CodingExecutionStage::Rework,
        &CodingWsInMessage::ContinueRework {
            extra_context: None,
        },
    ));
}
