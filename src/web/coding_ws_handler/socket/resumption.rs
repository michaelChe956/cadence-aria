use tokio::sync::mpsc;

use crate::product::coding_attempt_store::CodingAttemptStore;
use crate::product::coding_models::{
    CodingAttemptStatus, CodingExecutionAttempt, CodingExecutionStage,
};
use crate::product::coding_workspace_engine::CodingWorkspaceEngine;
use crate::product::coding_workspace_runner::CodingRunnerCommand;
use crate::product::git_workspace_service::GitWorkspaceService;
use crate::web::coding_ws_handler::CodingWsOutMessage;
use crate::web::state::{CodingAttemptRunKey, WebAppState};

use crate::web::coding_ws_handler::spawn_coding_runner;

/// attach 时半启动 attempt（3b 第 2 死点）runner 恢复的结果。
#[derive(Debug)]
pub(crate) enum ResumedAttemptRunner {
    /// 无需动作：attempt 非半启动态、注册表已有 runner 或恢复预约占用中
    /// （幂等负向：纯快照，行为与既有 attach 完全一致）。
    NotNeeded,
    /// runner 已按 StartCoding 分支的既有 spawn 路径重启。
    Restarted {
        command_tx: mpsc::Sender<CodingRunnerCommand>,
    },
}

/// attach 快照后调用：Running + WorktreePrepare/Coding 且注册表无 runner 的
/// 半启动 attempt（旧 runner 随旧 WS 断开已退出）自动重启 runner，避免
/// 「服务端无 runner + 客户端在 Coding 态无法重发 StartCoding」的互等死寂。
///
/// 触发判定先于任何锁执行，保证普通 attach（含 Hello/Ping 快路径）零开销；
/// 命中后与 `prepare_coding_message` 相同的串行化次序（attempt 锁 → 变更
/// 租约）下重读 attempt 并复查触发条件，防止并发 attach / 消息处理交叠双启
/// （双启最终由 `insert_cancellable` 拒绝兜底）。
pub(crate) async fn ensure_runner_for_resumed_attempt(
    state: &WebAppState,
    coding_store: &CodingAttemptStore,
    event_tx: &mpsc::Sender<CodingWsOutMessage>,
    attempt_key: &CodingAttemptRunKey,
    attempt: &CodingExecutionAttempt,
) -> ResumedAttemptRunner {
    if !resumed_attempt_needs_runner(&state.coding_runs, attempt_key, attempt) {
        return ResumedAttemptRunner::NotNeeded;
    }
    // 锁绑定存活到函数末尾（`_` 前缀仅消除未读警告，不提前释放）。
    let _attempt_guard = state.coding_runs.lock_attempt(attempt_key).await;
    let _mutation_lease = state.coding_runs.lock_attempt_mutation(attempt_key).await;
    let current =
        match coding_store.get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id) {
            Ok(current) => current,
            Err(error) => {
                tracing::warn!(
                    attempt_id = attempt.id.as_str(),
                    %error,
                    "semi-started runner restart aborted: attempt reload failed"
                );
                return ResumedAttemptRunner::NotNeeded;
            }
        };
    if !resumed_attempt_needs_runner(&state.coding_runs, attempt_key, &current) {
        return ResumedAttemptRunner::NotNeeded;
    }
    let engine = CodingWorkspaceEngine::new(
        coding_store.clone(),
        GitWorkspaceService::new(),
        event_tx.clone(),
    );
    let prepared = match engine.prepare_resumed_attempt_for_runner(&current).await {
        Ok(prepared) => prepared,
        Err(error) => {
            tracing::warn!(
                attempt_id = attempt.id.as_str(),
                %error,
                "semi-started runner restart aborted: materialization preparation failed"
            );
            return ResumedAttemptRunner::NotNeeded;
        }
    };
    match spawn_coding_runner(
        state.clone(),
        coding_store.clone(),
        event_tx.clone(),
        prepared,
    ) {
        Some(command_tx) => ResumedAttemptRunner::Restarted { command_tx },
        None => {
            tracing::warn!(
                attempt_id = attempt.id.as_str(),
                "semi-started runner restart refused by coding run registry"
            );
            ResumedAttemptRunner::NotNeeded
        }
    }
}

/// 半启动判定：Running + stage∈{WorktreePrepare, Coding} + 注册表既无
/// runner 也无恢复预约（活跑中重连 / 恢复进行中均不动作）。
pub(crate) fn resumed_attempt_needs_runner(
    coding_runs: &crate::web::state::CodingRunRegistry,
    attempt_key: &CodingAttemptRunKey,
    attempt: &CodingExecutionAttempt,
) -> bool {
    attempt.status == CodingAttemptStatus::Running
        && matches!(
            attempt.stage,
            CodingExecutionStage::WorktreePrepare | CodingExecutionStage::Coding
        )
        && !coding_runs.attempt_is_reserved_or_running(attempt_key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::web::state::CodingRunRegistry;

    fn resumed_attempt(
        status: CodingAttemptStatus,
        stage: CodingExecutionStage,
    ) -> CodingExecutionAttempt {
        CodingExecutionAttempt {
            id: "coding_attempt_0001".to_string(),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            work_item_id: "work_item_0001".to_string(),
            attempt_no: 1,
            scope: crate::product::coding_models::CodingAttemptScope::WorkItem,
            status,
            version: 1,
            manual_recovery_reason: None,
            admission_ticket_consumed_at: None,
            admission_kind: crate::product::coding_models::CodingAdmissionKind::LegacyGroup,
            stage,
            base_branch: "HEAD".to_string(),
            branch_name: "aria/work-items/work_item_0001/attempt-1".to_string(),
            worktree_path: None,
            provider_config_snapshot: crate::web::workspace_ws_types::ProviderConfigSnapshot {
                author: crate::product::models::ProviderName::Fake,
                reviewer: Some(crate::product::models::ProviderName::Fake),
                review_rounds: 1,
                permission_modes: Default::default(),
            },
            provider_conversations: Vec::new(),
            rework_count: 0,
            max_auto_rework: 2,
            work_item_group_id: None,
            current_work_item_id: None,
            active_unit_id: None,
            head_commit: None,
            pushed_remote: None,
            review_request_id: None,
            target_snapshot: None,
            created_at: "2026-09-03T00:00:00Z".to_string(),
            updated_at: "2026-09-03T00:00:00Z".to_string(),
            completed_at: None,
        }
    }

    fn attempt_key() -> CodingAttemptRunKey {
        CodingAttemptRunKey::new("project_0001", "issue_0001", "coding_attempt_0001")
    }

    #[test]
    fn running_coding_and_worktree_prepare_without_runner_need_recovery() {
        let registry = CodingRunRegistry::default();
        assert!(resumed_attempt_needs_runner(
            &registry,
            &attempt_key(),
            &resumed_attempt(CodingAttemptStatus::Running, CodingExecutionStage::Coding)
        ));
        assert!(resumed_attempt_needs_runner(
            &registry,
            &attempt_key(),
            &resumed_attempt(
                CodingAttemptStatus::Running,
                CodingExecutionStage::WorktreePrepare
            )
        ));
    }

    #[test]
    fn other_statuses_stages_or_live_runners_stay_snapshot_only() {
        let registry = CodingRunRegistry::default();
        let key = attempt_key();
        // 非半启动 stage / 非执行态 status：不动作。
        assert!(!resumed_attempt_needs_runner(
            &registry,
            &key,
            &resumed_attempt(
                CodingAttemptStatus::Running,
                CodingExecutionStage::PrepareContext
            )
        ));
        assert!(!resumed_attempt_needs_runner(
            &registry,
            &key,
            &resumed_attempt(
                CodingAttemptStatus::Running,
                CodingExecutionStage::CodeReview
            )
        ));
        assert!(!resumed_attempt_needs_runner(
            &registry,
            &key,
            &resumed_attempt(CodingAttemptStatus::Blocked, CodingExecutionStage::Coding)
        ));
        assert!(!resumed_attempt_needs_runner(
            &registry,
            &key,
            &resumed_attempt(
                CodingAttemptStatus::AwaitingManualRecovery,
                CodingExecutionStage::Coding
            )
        ));
        // 活 runner 或恢复预约占用：幂等负向，不动作。
        let (command_tx, _command_rx) = tokio::sync::mpsc::channel(1);
        registry
            .insert_cancellable(&key, command_tx)
            .expect("live runner registration");
        assert!(!resumed_attempt_needs_runner(
            &registry,
            &key,
            &resumed_attempt(CodingAttemptStatus::Running, CodingExecutionStage::Coding)
        ));
    }
}
