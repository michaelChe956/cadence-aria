use super::*;
use crate::cross_cutting::streaming_provider::ProviderSession;
use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_attempt_store::CreateCodingAttemptInput;
use crate::product::coding_models::CodingProviderRole;
use crate::product::models::{ProviderConversationRef, ProviderConversationRole};
use crate::web::workspace_ws_types::ProviderConfigSnapshot;
use tempfile::tempdir;

fn blocked_report_with(missing: Vec<String>, skipped: Vec<String>) -> TestingReport {
    TestingReport {
        id: "testing_report_0001".to_string(),
        attempt_id: "coding_attempt_0001".to_string(),
        role_run_id: None,
        run_no: None,
        commands: Vec::new(),
        overall_status: TestingOverallStatus::Blocked,
        provider_claim: None,
        backend_verified: true,
        started_at: "2026-06-10T00:00:00Z".to_string(),
        completed_at: Some("2026-06-10T00:00:01Z".to_string()),
        plan_id: Some("test_plan_0001".to_string()),
        plan_summary: Some("plan".to_string()),
        steps: Vec::new(),
        unplanned_commands: Vec::new(),
        unplanned_evidence: Vec::new(),
        missing_required_steps: missing,
        skipped_required_steps: skipped,
        context_warnings: Vec::new(),
        raw_provider_output_ref: None,
    }
}

mod gate_rework;
mod parser_prompt;
mod provider_driven;

fn running_attempt_with_worktree() -> (
    tempfile::TempDir,
    CodingAttemptStore,
    CodingExecutionAttempt,
) {
    let root = tempdir().expect("tempdir");
    let worktree = root.path().join("worktree");
    std::fs::create_dir_all(&worktree).expect("worktree dir");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            work_item_id: "work_item_0001".to_string(),
            base_branch: "HEAD".to_string(),
            branch_name: "aria/work-items/work_item_0001/attempt-1".to_string(),
            worktree_path: Some(worktree),
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Codex,
                reviewer: Some(ProviderName::ClaudeCode),
                review_rounds: 1,
            },
            max_auto_rework: 2,
        })
        .expect("create attempt");
    let attempt = store
        .update_attempt_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running attempt");
    (root, store, attempt)
}

fn test_attempt(id: &str) -> CodingExecutionAttempt {
    CodingExecutionAttempt {
        id: id.to_string(),
        project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
        work_item_id: "work_item_0001".to_string(),
        attempt_no: 1,
        status: CodingAttemptStatus::Running,
        stage: CodingExecutionStage::Coding,
        base_branch: "HEAD".to_string(),
        branch_name: "aria/work-items/work_item_0001/attempt-1".to_string(),
        worktree_path: None,
        provider_config_snapshot: ProviderConfigSnapshot {
            author: ProviderName::ClaudeCode,
            reviewer: Some(ProviderName::Codex),
            review_rounds: 1,
        },
        provider_conversations: Vec::new(),
        rework_count: 0,
        max_auto_rework: 2,
        head_commit: None,
        pushed_remote: None,
        review_request_id: None,
        created_at: "2026-06-01T00:00:00Z".to_string(),
        updated_at: "2026-06-01T00:00:00Z".to_string(),
        completed_at: None,
    }
}
