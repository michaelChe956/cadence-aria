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

/// F-16：`awaiting_manual_recovery` 的显式恢复通道。F-14 fail-closed 后
/// attempt 停在人工恢复态（abort-only），此前无任何 retry/recover 通道
/// （KimiUpgrade v25 wire 三探测全拒、源码四重印证）。恢复链 = 状态门放行
/// `RecoverCoding` → store `recover_attempt_from_manual_recovery`（重验
/// 路由/快照/policy 的 admission CAS 回 Running、清除 reason、重锚 marker）
/// → socket 分支复用 StartCoding 同款 spawn 重启 runner。本测试在 lib 层
/// 复刻该组合，并钉死 F-14 零回归面（attach 依旧零动作）与死因 durable 尾帧。
#[tokio::test]
async fn recover_coding_channel_re_admits_and_restarts_runner() {
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

    let quarantined = store
        .get_attempt("project_0001", "issue_0001", &attempt_id)
        .expect("quarantined attempt");
    assert_eq!(
        quarantined.status,
        CodingAttemptStatus::AwaitingManualRecovery
    );
    let version_at_manual_recovery = quarantined.version;

    // 状态门：人工恢复态必须放行显式恢复动作（F-16 红面）。
    assert!(crate::web::coding_ws_handler::is_coding_ws_message_allowed(
        &CodingAttemptStatus::AwaitingManualRecovery,
        &CodingExecutionStage::Coding,
        &crate::web::coding_ws_handler::CodingWsInMessage::RecoverCoding,
    ));
    // F-14 零回归：attach 半启动重启对人工恢复态依旧零动作。
    let resume =
        ensure_runner_for_resumed_attempt(&state, &store, &event_tx, &attempt_key, &quarantined)
            .await;
    assert!(matches!(resume, ResumedAttemptRunner::NotNeeded));

    // 恢复 CAS：重验 admission 回 Running，清除 reason、重锚 marker。
    let recovered = store
        .recover_attempt_from_manual_recovery("project_0001", "issue_0001", &attempt_id)
        .expect("explicit recovery channel");
    assert_eq!(recovered.status, CodingAttemptStatus::Running);
    assert_eq!(recovered.version, version_at_manual_recovery + 1);
    assert!(recovered.admission_ticket_consumed_at.is_some());
    assert_eq!(recovered.manual_recovery_reason, None);

    // socket RecoverCoding 分支同款 spawn：runner 重启并再次死于 provider
    // 启动前（空注册表），再次 fail-closed 回人工恢复——恢复通道全程可见。
    let command_tx = spawn_coding_runner(state.clone(), store.clone(), event_tx.clone(), recovered)
        .expect("recovered runner spawned");
    command_tx
        .send(CodingRunnerCommand::StageGateConfirm {
            stage: CodingExecutionStage::Coding,
        })
        .await
        .expect("confirm coding stage gate after recovery");
    tokio::time::timeout(Duration::from_secs(5), async {
        while state.coding_runs.runner_count(&attempt_key) != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("recovered runner must exit after the same deterministic failure");

    let final_attempt = store
        .get_attempt("project_0001", "issue_0001", &attempt_id)
        .expect("final attempt");
    assert_eq!(
        final_attempt.status,
        CodingAttemptStatus::AwaitingManualRecovery,
        "恢复后的再次死亡必须再次 fail-closed（同款 F-14 语义）"
    );
    assert_eq!(
        final_attempt.manual_recovery_reason.as_deref(),
        Some("coding_runner_failed_while_running")
    );

    // 死因可考：durable 尾帧（System/SystemEvent）记录稳定 reason + 原始
    // 错误串，且两次死亡各留一条。
    let entries = store
        .list_chat_entries("project_0001", "issue_0001", &attempt_id)
        .expect("chat entries");
    let diagnostics: Vec<&crate::product::coding_models::CodingChatEntry> = entries
        .iter()
        .filter(|entry| {
            matches!(
                &entry.entry_type,
                crate::product::coding_models::CodingEntryType::SystemEvent {
                    event_type,
                    ..
                } if event_type == "manual_recovery_transition"
            )
        })
        .collect();
    assert_eq!(
        diagnostics.len(),
        2,
        "两次 pre-provider 死亡必须各留一条 durable 诊断尾帧"
    );
    for entry in &diagnostics {
        assert_eq!(
            entry.role,
            crate::product::coding_models::CodingAgentRole::System
        );
        match &entry.entry_type {
            crate::product::coding_models::CodingEntryType::SystemEvent { message, .. } => {
                assert!(
                    message.starts_with("coding_runner_failed_while_running: "),
                    "诊断尾帧必须含原始错误串：{message}"
                );
            }
            other => panic!("unexpected entry type: {other:?}"),
        }
    }

    // 第二次死亡的 protocol error 仍可见（fail-visible 不吞没）。
    let mut saw_protocol_error = false;
    while let Ok(event) = event_rx.try_recv() {
        if let CodingWsOutMessage::CodingProtocolError { code, .. } = &event
            && code == "coding_start_failed"
        {
            saw_protocol_error = true;
        }
    }
    assert!(saw_protocol_error, "恢复后的 runner 失败必须仍可见");
}
