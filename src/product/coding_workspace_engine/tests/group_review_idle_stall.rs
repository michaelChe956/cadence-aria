// P4-b（REQ-PTR-IDLE）红测：internal reviewer（group review shard/reduction
// executor，恒 InternalReviewer）接入 P4-a 共享 idle 检测的编排器行为。
// ①一次挂起恢复后继续原审查；②两次挂起恰两次调用即耗尽，落
// `code_review_provider_stalled` 门（按 stage 恢复 retry_internal_review），
// 而非吃满 transport 3 次预算；③/④ idle 与 transport 预算相互独立。

use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_attempt_store::{CodingAttemptStore, CreateCodingAttemptInput};
use crate::product::coding_models::{CodingAttemptStatus, CodingExecutionStage};
use crate::product::coding_workspace_engine::group_review_errors::{
    GroupReviewExecutionError, GroupReviewOrchestrationError,
};
use crate::product::coding_workspace_engine::group_review_orchestrator::{
    FakeGroupReviewExecutor, GroupReviewExecutionResult, GroupReviewOrchestrator,
};
use crate::product::models::ProviderName;
use crate::web::workspace_ws_types::ProviderConfigSnapshot;
use tempfile::TempDir;

fn idle_stall(phase: &str) -> GroupReviewExecutionError {
    GroupReviewExecutionError::IdleStall {
        phase: phase.to_string(),
        idle_secs: 5,
        idle_limit_secs: 1,
    }
}

fn success(output: &str) -> Result<GroupReviewExecutionResult, GroupReviewExecutionError> {
    Ok(GroupReviewExecutionResult {
        full_output: output.to_string(),
        role_run_id: None,
    })
}

fn execution_store() -> (TempDir, CodingAttemptStore, String) {
    let root = TempDir::new().expect("tempdir");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            work_item_id: "work_item_0001".to_string(),
            base_branch: "main".to_string(),
            branch_name: "aria/work-item".to_string(),
            worktree_path: None,
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Codex,
                reviewer: None,
                review_rounds: 1,
                permission_modes: Default::default(),
            },
            target_snapshot: None,
            max_auto_rework: 1,
        })
        .expect("attempt");
    (root, store, attempt.id)
}

#[tokio::test]
async fn internal_reviewer_idle_stall_once_recovers_and_review_continues() {
    let (_root, store, _attempt_id) = execution_store();
    let executor =
        FakeGroupReviewExecutor::new(vec![Err(idle_stall("shard")), success("review output")]);
    let orchestrator = GroupReviewOrchestrator::new(&executor, &store);

    let result = orchestrator
        .execute_with_retry_result("shard prompt", 3)
        .await
        .expect("one idle stall recovers on fresh retry");
    assert_eq!(result.full_output, "review output");
    // 同 provider fresh 重试恰一次：两次调用、同 prompt。
    assert_eq!(executor.prompts(), vec!["shard prompt", "shard prompt"]);
}

#[tokio::test]
async fn internal_reviewer_idle_stall_twice_exhausts_in_two_calls_not_third_transport() {
    // 建门路径会 Running→Blocked;Created 直转 Blocked 被状态机拒绝,
    // 故复用父模块把 attempt 置 Running 的现成 fixture。
    let (_root, store, attempt) = super::running_attempt_with_worktree();
    let attempt_id = attempt.id.clone();
    // 第三次结果故意不给：若编排器把挂起当 transport 吃满 3 次预算，Fake 会以
    // internal_exhausted 兜底并让 prompts 断言失败。
    let executor =
        FakeGroupReviewExecutor::new(vec![Err(idle_stall("shard")), Err(idle_stall("shard"))]);
    let orchestrator = GroupReviewOrchestrator::new(&executor, &store);

    let error = orchestrator
        .execute_with_retry_result("shard prompt", 3)
        .await
        .expect_err("two idle stalls exhaust");
    assert!(matches!(error, GroupReviewExecutionError::IdleStall { .. }));
    // 两次挂起只调 2 次，不占 transport 3 次预算。
    assert_eq!(executor.prompts(), vec!["shard prompt", "shard prompt"]);

    let gate = orchestrator
        .create_failure_gate(
            &attempt,
            "coding_node_0001",
            &GroupReviewOrchestrationError::Executor(idle_stall("shard")),
        )
        .expect("code_review_provider_stalled gate");
    assert_eq!(
        gate.reason_code.as_deref(),
        Some("code_review_provider_stalled")
    );
    assert_eq!(gate.stage, Some(CodingExecutionStage::InternalPrReview));
    // P4-a 备好的动作集：按 stage 恢复（retry_internal_review）+ 终止。
    let action_ids: Vec<&str> = gate
        .available_actions
        .iter()
        .map(|action| action.action_id.as_str())
        .collect();
    assert_eq!(action_ids, vec!["retry_internal_review", "abort"]);
    assert_eq!(
        store
            .find_attempt_by_id(&attempt_id)
            .expect("attempt")
            .status,
        CodingAttemptStatus::Blocked
    );
}

#[tokio::test]
async fn idle_stall_budget_is_independent_from_transport_budget() {
    let (_root, store, _attempt_id) = execution_store();
    let executor = FakeGroupReviewExecutor::new(vec![
        Err(idle_stall("reduction")),
        Err(GroupReviewExecutionError::Transport("timeout".to_string())),
        Err(idle_stall("reduction")),
    ]);
    let orchestrator = GroupReviewOrchestrator::new(&executor, &store);

    let error = orchestrator
        .execute_with_retry_result("reduction prompt", 3)
        .await
        .expect_err("second idle stall exhausts even with transport budget left");
    assert!(matches!(error, GroupReviewExecutionError::IdleStall { .. }));
    // idle 挂起不消耗 transport 次数：挂起→transport→挂起，共 3 次调用后由
    // 第 2 次挂起收口。
    assert_eq!(executor.prompts().len(), 3);
}

#[tokio::test]
async fn transport_retry_budget_still_counts_three_attempts() {
    let (_root, store, _attempt_id) = execution_store();
    let executor = FakeGroupReviewExecutor::new(vec![
        Err(GroupReviewExecutionError::Transport("timeout".to_string())),
        Err(GroupReviewExecutionError::Transport("timeout".to_string())),
        success("third attempt output"),
    ]);
    let orchestrator = GroupReviewOrchestrator::new(&executor, &store);

    let result = orchestrator
        .execute_with_retry_result("shard prompt", 3)
        .await
        .expect("pure transport keeps the 3-attempt budget");
    assert_eq!(result.full_output, "third attempt output");
    assert_eq!(executor.prompts().len(), 3);
}
