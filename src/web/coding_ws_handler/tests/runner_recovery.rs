use std::time::Duration;

use tokio::sync::mpsc;

use crate::cross_cutting::provider_registry::ProviderRegistry;
use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_attempt_store::{CodingAttemptStore, CreateCodingAttemptInput};
use crate::product::coding_models::{
    CodingAttemptStatus, CodingExecutionStage, CodingProviderPermissionMode,
    CodingRolePermissionModes, CodingRoleProviderConfigSnapshot,
};
use crate::product::coding_workspace_runner::CodingRunnerCommand;
use crate::product::models::ProviderName;
use crate::web::coding_ws_handler::runner::spawn_coding_runner;
use crate::web::coding_ws_handler::socket::{
    ResumedAttemptRunner, ensure_runner_for_resumed_attempt,
};
use crate::web::coding_ws_handler::{CodingWsOutMessage, OutboundEventReceiver};
use crate::web::runtime::WebRuntime;
use crate::web::state::{CodingAttemptRunKey, WebAppState};
use crate::web::workspace_ws_types::ProviderConfigSnapshot;

/// F-14 红测：blocked gate 放行（retry_coding）/ attach 重启拉起的 runner，在
/// stage gate 过期或确认之后、provider 启动之前死亡（现场形态：coder provider
/// 启动失败——coding_attempt_0556a410 的 gate 0006/0007 相继过期、attempt
/// updated_at 冻结在放行时刻、provider 零启动）时，不得把 attempt 留在
/// Running：那会让 attach 侧 `ensure_runner_for_resumed_attempt` 无限重启
/// runner、反复创建 5s stage gate、再次死亡——「gate 反复过期 + provider 零
/// 启动」静默死循环。必须 fail-closed 转 AwaitingManualRecovery（resumption
/// B 兜底同款语义），重连不再重启，失败事件仍可见。
#[tokio::test]
async fn runner_dying_before_provider_moves_running_attempt_to_manual_recovery() {
    let root = tempfile::tempdir().expect("root");
    let worktree = root.path().join("worktree");
    std::fs::create_dir_all(&worktree).expect("worktree");
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
                author: ProviderName::KimiCode,
                reviewer: Some(ProviderName::KimiCode),
                review_rounds: 1,
                permission_modes: Default::default(),
            },
            target_snapshot: None,
            max_auto_rework: 2,
        })
        .expect("create attempt");
    let attempt = store
        .seed_running_attempt_for_test(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("admitted running attempt");
    let attempt = store
        .update_attempt_stage(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingExecutionStage::Coding,
        )
        .expect("coding stage");
    store
        .update_role_provider_config_snapshot(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingRoleProviderConfigSnapshot {
                coder: ProviderName::KimiCode,
                code_reviewer: ProviderName::KimiCode,
                internal_reviewer: ProviderName::KimiCode,
                review_rounds: 1,
                permission_modes: CodingRolePermissionModes {
                    coder: CodingProviderPermissionMode::Auto,
                    code_reviewer: CodingProviderPermissionMode::Auto,
                    internal_reviewer: CodingProviderPermissionMode::Auto,
                },
            },
        )
        .expect("role config points coder at unregistered provider");

    // registry 为空：runner 穿过 coding stage gate 后 provider_for 必然失败，
    // 复刻现场「gate 过期/确认后 runner 死于 provider 启动前」的断链形态。
    let mut state = WebAppState::with_provider_registry(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
        ProviderRegistry::new(),
    );
    state.test_provider_enabled = true;

    let attempt_id = attempt.id.clone();
    let (event_tx, event_rx) = mpsc::channel(64);
    let mut event_rx = OutboundEventReceiver::new(event_rx);
    let attempt_key = CodingAttemptRunKey::new("project_0001", "issue_0001", attempt_id.as_str());
    let command_tx = spawn_coding_runner(state.clone(), store.clone(), event_tx.clone(), attempt)
        .expect("resume runner spawned (run registry empty)");
    // 预置 coding stage gate 确认，跳过 5s 倒计时。
    command_tx
        .send(CodingRunnerCommand::StageGateConfirm {
            stage: CodingExecutionStage::Coding,
        })
        .await
        .expect("confirm coding stage gate");

    tokio::time::timeout(Duration::from_secs(5), async {
        while state.coding_runs.runner_count(&attempt_key) != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("failed runner must exit and clear its registration");

    let final_attempt = store
        .get_attempt("project_0001", "issue_0001", &attempt_id)
        .expect("final attempt");
    assert_eq!(
        final_attempt.status,
        CodingAttemptStatus::AwaitingManualRecovery,
        "F-14: runner 在 provider 启动前死亡不得把 attempt 留在 Running（僵尸 + 重连空转）"
    );
    assert_eq!(
        final_attempt.manual_recovery_reason.as_deref(),
        Some("coding_runner_failed_while_running"),
        "必须持久化稳定 reason 码供人工恢复面诊断"
    );
    assert!(
        final_attempt.admission_ticket_consumed_at.is_none(),
        "离开 Running 进入人工恢复必须在同一次锁内结束 admission 会话"
    );

    // 重连 attach 不得再重启注定失败的 runner（打破「gate 反复过期」循环）。
    let resume =
        ensure_runner_for_resumed_attempt(&state, &store, &event_tx, &attempt_key, &final_attempt)
            .await;
    assert!(
        matches!(resume, ResumedAttemptRunner::NotNeeded),
        "AwaitingManualRecovery 不属于半启动重启集合，attach 必须零动作"
    );
    assert_eq!(
        state.coding_runs.runner_count(&attempt_key),
        0,
        "attach 不得重启已 fail-closed 的 attempt runner"
    );

    // 失败必须保持可见：protocol error 事件不因 fail-closed 被吞没。
    let mut saw_protocol_error = false;
    while let Ok(event) = event_rx.try_recv() {
        if let CodingWsOutMessage::CodingProtocolError { code, .. } = &event
            && code == "coding_start_failed"
        {
            saw_protocol_error = true;
        }
    }
    assert!(
        saw_protocol_error,
        "runner 失败必须仍发出 coding_start_failed protocol error"
    );
}
