use std::time::Duration;

use tokio::sync::mpsc;

use crate::cross_cutting::provider_registry::ProviderRegistry;
use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_attempt_store::{
    ClaimCodingStartOutcome, CodingAttemptStore, CreateCodingAttemptInput,
};
use crate::product::coding_models::{
    CodingAttemptStatus, CodingExecutionAttempt, CodingExecutionStage,
    CodingProviderPermissionMode, CodingRolePermissionModes, CodingRoleProviderConfigSnapshot,
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
                code_reviewer: Some(ProviderName::KimiCode),
                internal_reviewer: Some(ProviderName::KimiCode),
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

/// Task 2.8 fix round 1（P1）：LC 作用域 attempt 的 gateway 工厂组装失败
/// （此处以双工厂 root 投影漂移触发：record.aggregate_root ≠ manifest
/// provider_context_root）必须 fail-closed 中止启动——不得 `.ok()` 静默
/// 降级成「无 gateway 继续」，那会把「root 不一致 zero spawn」反转成无
/// 门禁窗口。断言：runner 零 provider 启动即退出、attempt 转
/// AwaitingManualRecovery（稳定 reason 码）、死因经 manual-recovery
/// diagnostic 与 coding_start_failed protocol error 双通道可见（携带工厂
/// 失败原文）。对照面：另两个消费方（workspace manager / aggregate
/// driver）同错误 map_err 传播。
#[tokio::test]
async fn lc_gateway_factory_build_failure_fails_closed_before_provider_spawn() {
    use crate::product::issue_store::{CreateProductIssueInput, IssueStore};
    use crate::product::logical_codebase::store::LogicalCodebaseRecord;
    use crate::product::logical_codebase::{
        LogicalCodebaseManifest, LogicalCodebaseStore, LogicalRepositoryId,
        RepositoryCheckoutId,
    };
    use crate::product::project_store::{CreateProjectInput, ProjectStore};

    let root = tempfile::tempdir().expect("root");
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let project = ProjectStore::new(paths.clone())
        .create(CreateProjectInput {
            name: "gateway root drift project".to_string(),
            description: None,
        })
        .expect("create project");
    let lc_id = "lc_drift".to_string();

    // 双工厂 root 投影漂移：登记工厂冻结根 A，manifest 权威根 B（A≠B 且都
    // 真实存在）——resolver 解析成功，工厂组装在 root assertion 处失败。
    let registration_root = root.path().join("authority a");
    let manifest_root = root.path().join("authority b");
    std::fs::create_dir_all(&registration_root).expect("create registration root");
    std::fs::create_dir_all(&manifest_root).expect("create manifest root");
    let record_dir = paths.logical_codebase_record_root(&project.id, &lc_id);
    std::fs::create_dir_all(&record_dir).expect("create lc record dir");
    crate::product::json_store::write_json(
        &record_dir.join("record.json"),
        &LogicalCodebaseRecord {
            id: lc_id.clone(),
            name: "lc_drift".to_string(),
            aggregate_root: registration_root,
            created_at: "2026-10-02T00:00:00Z".to_string(),
        },
    )
    .expect("write lc record");
    LogicalCodebaseStore::for_lc(paths.clone(), lc_id.clone())
        .save_manifest(
            &project.id,
            &LogicalCodebaseManifest::new(&project.id, manifest_root, vec![]),
        )
        .expect("save lc manifest");
    let issue = IssueStore::new(paths.clone())
        .create(CreateProductIssueInput {
            project_id: project.id.clone(),
            repo_id: None,
            logical_codebase_id: Some(lc_id.clone()),
            title: "gateway drift issue".to_string(),
            description: None,
            change_id: None,
            base_branch: None,
        })
        .expect("create lc-attributed issue");

    // LC 作用域 running attempt（target_snapshot 冻结逻辑 target）。
    let worktree = root.path().join("worktree");
    std::fs::create_dir_all(&worktree).expect("worktree");
    let store = CodingAttemptStore::new(paths.clone());
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            project_id: project.id.clone(),
            issue_id: issue.id.clone(),
            work_item_id: "work_item_0001".to_string(),
            base_branch: "HEAD".to_string(),
            branch_name: "aria/work-items/work_item_0001/attempt-1".to_string(),
            worktree_path: Some(worktree.clone()),
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::ClaudeCode,
                reviewer: Some(ProviderName::ClaudeCode),
                review_rounds: 1,
                permission_modes: Default::default(),
            },
            target_snapshot: Some(crate::product::coding_models::AttemptTargetSnapshot {
                logical_repository_id: LogicalRepositoryId(uuid::Uuid::new_v4()),
                checkout_id: RepositoryCheckoutId(uuid::Uuid::new_v4()),
                physical_repository_id: "repository_0001".to_string(),
                canonical_path: worktree,
                git_dir_identity: "git-dir-identity".to_string(),
                revision: None,
                policy_digest: String::new(),
                membership_revision: 1,
                captured_at: "2026-10-02T00:00:00Z".to_string(),
                capture_source: "runner_recovery".to_string(),
            }),
            max_auto_rework: 2,
        })
        .expect("create attempt");
    let attempt = store
        .seed_running_attempt_for_test(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("admitted running attempt");

    let state = WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    );
    assert!(
        state.gateway_factory().is_some(),
        "default app state must carry the logical gateway factory"
    );

    let attempt_key = CodingAttemptRunKey::from_attempt(&attempt);
    let attempt_id = attempt.id.clone();
    let (event_tx, event_rx) = mpsc::channel(64);
    let mut event_rx = OutboundEventReceiver::new(event_rx);
    let _command_tx = spawn_coding_runner(state.clone(), store.clone(), event_tx, attempt)
        .expect("runner spawned");

    tokio::time::timeout(Duration::from_secs(5), async {
        while state.coding_runs.runner_count(&attempt_key) != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("runner must exit after the fail-closed factory error");

    // 零 provider 启动 + fail-closed 状态：attempt 不留 Running（否则 attach
    // 侧重启形成无门禁窗口循环），转人工恢复并持久稳定 reason 码。
    let final_attempt = store
        .get_attempt(&project.id, &issue.id, &attempt_id)
        .expect("final attempt");
    assert_eq!(
        final_attempt.status,
        CodingAttemptStatus::AwaitingManualRecovery,
        "LC gateway factory build failure must abort before provider spawn"
    );
    assert_eq!(
        final_attempt.manual_recovery_reason.as_deref(),
        Some("coding_runner_failed_while_running")
    );

    // 错误可见（通道 1）：manual-recovery diagnostic 携带工厂失败原文与
    // 漂移字段，而非退化为含糊的 gateway-missing 错误。
    let entries = store
        .list_chat_entries(&project.id, &issue.id, &attempt_id)
        .expect("chat entries");
    let diagnostic_visible = entries.iter().any(|entry| {
        matches!(
            &entry.entry_type,
            crate::product::coding_models::CodingEntryType::SystemEvent { message, .. }
 if message.contains("logical gateway factory build failed")
                && message.contains("registration_root")
        )
    });
    assert!(
        diagnostic_visible,
        "manual-recovery diagnostic must carry the factory failure detail"
    );

    // 错误可见（通道 2）：protocol error 事件不吞没，且消息含失败原文。
    let mut saw_protocol_error = false;
    while let Ok(event) = event_rx.try_recv() {
        if let CodingWsOutMessage::CodingProtocolError { code, message } = &event
            && code == "coding_start_failed"
            && message.contains("logical gateway factory build failed")
        {
            saw_protocol_error = true;
        }
    }
    assert!(
        saw_protocol_error,
        "factory build failure must surface as coding_start_failed protocol error"
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
                code_reviewer: Some(ProviderName::KimiCode),
                internal_reviewer: Some(ProviderName::KimiCode),
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

// ---------------------------------------------------------------------------
// P2 Task 6（tasks.md §3.2）：无 attach 启动扫描——已认领/在途 coding run
// 在 serve_web 构造 state 后先行恢复，不再以 socket attach 为唯一触发。
// ---------------------------------------------------------------------------

struct StartupReconcileFixture {
    _root: tempfile::TempDir,
    store: CodingAttemptStore,
    attempt: CodingExecutionAttempt,
}

impl StartupReconcileFixture {
    fn attempt_key(&self) -> CodingAttemptRunKey {
        CodingAttemptRunKey::from_attempt(&self.attempt)
    }

    fn store(&self) -> CodingAttemptStore {
        self.store.clone()
    }

    /// 「重启进程」替身：同 `.aria`、全新 WebAppState（内存 registry 清零）。
    fn restart_state(&self) -> WebAppState {
        let root = self._root.path().to_path_buf();
        WebAppState::new(root.clone(), WebRuntime::new_fake(root))
    }

    fn reload(&self) -> crate::product::coding_models::CodingExecutionAttempt {
        self.store()
            .get_attempt(
                &self.attempt.project_id,
                &self.attempt.issue_id,
                &self.attempt.id,
            )
            .expect("reload attempt")
    }

    async fn reconcile(&self, state: &WebAppState) -> usize {
        crate::web::autopilot_orchestrator::reconcile_claimed_coding_runs_once(state)
            .await
            .expect("startup reconcile ok")
    }
}

/// 播种启动扫描现场 attempt：可选 Task 4 durable claim（推进到指定 phase）、
/// 可选 durable admission + Running + Coding stage（`seed_running_attempt_for_test`
/// + stage 推进，模拟旧进程崩溃残留的半启动现场，不伪造外部 provider 调用）。
fn seed_startup_attempt(
    claim_phase: Option<crate::product::coding_models::CodingStartPhase>,
    running: bool,
) -> StartupReconcileFixture {
    let root = tempfile::tempdir().expect("root");
    let worktree = root.path().join("worktree");
    std::fs::create_dir_all(&worktree).expect("worktree");
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    // 启动扫描沿 ProjectStore → IssueStore → attempt 遍历：补权威记录。
    crate::product::project_store::ProjectStore::new(paths.clone())
        .create(crate::product::project_store::CreateProjectInput {
            name: "startup reconcile project".to_string(),
            description: None,
        })
        .expect("seed project");
    crate::product::issue_store::IssueStore::new(paths.clone())
        .create(crate::product::issue_store::CreateProductIssueInput {
            project_id: "project_0001".to_string(),
            repo_id: None,
            logical_codebase_id: None,
            title: "startup reconcile issue".to_string(),
            description: None,
            change_id: None,
            base_branch: None,
        })
        .expect("seed issue");
    let store = CodingAttemptStore::new(paths.clone());
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            work_item_id: "work_item_0001".to_string(),
            base_branch: "HEAD".to_string(),
            branch_name: "aria/work-items/work_item_0001/attempt-1".to_string(),
            worktree_path: Some(worktree),
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Fake,
                reviewer: Some(ProviderName::Fake),
                review_rounds: 1,
                permission_modes: Default::default(),
            },
            target_snapshot: None,
            max_auto_rework: 2,
        })
        .expect("create attempt");
    store
        .update_role_provider_config_snapshot(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingRoleProviderConfigSnapshot {
                coder: ProviderName::Fake,
                code_reviewer: Some(ProviderName::Fake),
                internal_reviewer: Some(ProviderName::Fake),
                review_rounds: 1,
                permission_modes: CodingRolePermissionModes {
                    coder: CodingProviderPermissionMode::Auto,
                    code_reviewer: CodingProviderPermissionMode::Auto,
                    internal_reviewer: CodingProviderPermissionMode::Auto,
                },
            },
        )
        .expect("role config points at fake providers");
    let mut attempt = attempt;
    if let Some(phase) = claim_phase {
        let command_id = format!("startup-reconcile-{}", attempt.id);
        let claimed = store
            .claim_coding_start(
                &attempt,
                &command_id,
                &crate::product::coding_models::CodingStartOrigin::Manual,
            )
            .expect("task4 durable claim");
        let claimed = match claimed {
            ClaimCodingStartOutcome::Claimed(saved) => saved,
            ClaimCodingStartOutcome::Existing(_) => panic!("fresh attempt must claim cleanly"),
        };
        if phase != crate::product::coding_models::CodingStartPhase::Claimed {
            store
                .advance_coding_start_phase(&claimed, &command_id, phase)
                .expect("advance claim phase");
        }
        attempt = claimed;
    }
    if running {
        attempt = store
            .seed_running_attempt_for_test(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("seed durable running admission");
        attempt = store
            .update_attempt_stage(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                CodingExecutionStage::Coding,
            )
            .expect("coding stage");
    }
    StartupReconcileFixture {
        _root: root,
        store,
        attempt,
    }
}

/// 主线：Running + 已持久 ProviderMayHaveStarted claim 的半启动 attempt，
/// 进程「重启」后无任何 WS attach，启动扫描即恢复 runner；幂等不双启、
/// 不新建 attempt。
#[tokio::test]
async fn startup_reconciles_claimed_running_coding_without_attach() {
    let fixture = seed_startup_attempt(
        Some(crate::product::coding_models::CodingStartPhase::ProviderMayHaveStarted),
        true,
    );
    let fresh = fixture.restart_state();
    assert_eq!(fresh.coding_runs.runner_count(&fixture.attempt_key()), 0);
    fixture.reconcile(&fresh).await;
    assert_eq!(
        fresh.coding_runs.runner_count(&fixture.attempt_key()),
        1,
        "startup reconcile must resume the claimed running attempt without attach"
    );
    assert_eq!(
        fixture
            .store()
            .list_attempts_for_issue("project_0001", "issue_0001")
            .expect("list attempts")
            .len(),
        1,
        "recovery must resume the original attempt, not create a new one"
    );
    assert_eq!(fixture.reload().status, CodingAttemptStatus::Running);
    // 幂等：重复扫描不得双启（registry/claim 去重）。
    fixture.reconcile(&fresh).await;
    assert_eq!(fresh.coding_runs.runner_count(&fixture.attempt_key()), 1);
}

/// Claimed 未跨 barrier 的安全窗：同 command/origin 幂等续启（Task 4 单一
/// 恢复方法），不隐式新 command。
#[tokio::test]
async fn startup_reconcile_resumes_claimed_first_start_with_same_command() {
    let fixture =
        seed_startup_attempt(Some(crate::product::coding_models::CodingStartPhase::Claimed), false);
    let fresh = fixture.restart_state();
    fixture.reconcile(&fresh).await;
    assert_eq!(
        fresh.coding_runs.runner_count(&fixture.attempt_key()),
        1,
        "claimed-not-crossed-barrier attempt must resume via the same command"
    );
    let reloaded = fixture.reload();
    assert_eq!(
        reloaded.start_claim.as_ref().expect("claim kept").command_id,
        format!("startup-reconcile-{}", reloaded.id),
        "recovery must replay the durable claim command identity"
    );
    assert_eq!(
        fixture
            .store()
            .list_attempts_for_issue("project_0001", "issue_0001")
            .unwrap()
            .len(),
        1
    );
}

/// Created + ProviderMayHaveStarted + 无可信 ledger：外部副作用不可证明 →
/// durable AwaitingManualRecovery（UI 可诊断），零 runner。
#[tokio::test]
async fn startup_unproven_provider_window_moves_created_attempt_to_manual_recovery() {
    let fixture = seed_startup_attempt(
        Some(crate::product::coding_models::CodingStartPhase::ProviderMayHaveStarted),
        false,
    );
    let fresh = fixture.restart_state();
    fixture.reconcile(&fresh).await;
    assert_eq!(fresh.coding_runs.runner_count(&fixture.attempt_key()), 0);
    let reloaded = fixture.reload();
    assert_eq!(
        reloaded.status,
        CodingAttemptStatus::AwaitingManualRecovery,
        "unproven provider window must fail closed for manual triage"
    );
    assert!(reloaded.manual_recovery_reason.is_some());
    // 人工恢复态二次扫描零动作。
    fixture.reconcile(&fresh).await;
    assert_eq!(fresh.coding_runs.runner_count(&fixture.attempt_key()), 0);
}

/// Created 无 claim 不隐式启动；legacy Running（无 claim、有既有准入证据）
/// 按原规则恢复；Failed/Aborted/Completed 一律不恢复。
#[tokio::test]
async fn startup_reconcile_skips_terminal_and_unclaimed_but_recovers_legacy_running() {
    let idle = seed_startup_attempt(None, false);
    let fresh = idle.restart_state();
    idle.reconcile(&fresh).await;
    assert_eq!(
        fresh.coding_runs.runner_count(&idle.attempt_key()),
        0,
        "created attempt without claim must not auto-start"
    );
    assert_eq!(idle.reload().status, CodingAttemptStatus::Created);

    let legacy = seed_startup_attempt(None, true);
    let legacy_state = legacy.restart_state();
    legacy.reconcile(&legacy_state).await;
    assert_eq!(
        legacy_state.coding_runs.runner_count(&legacy.attempt_key()),
        1,
        "legacy running attempt with durable admission must recover by the original rules"
    );

    for status in [
        CodingAttemptStatus::Failed,
        CodingAttemptStatus::Aborted,
        CodingAttemptStatus::Completed,
    ] {
        let terminal = seed_startup_attempt(None, false);
        let mut attempt = terminal.reload();
        attempt.status = status.clone();
        terminal
            .store()
            .write_coding_attempt_for_test(&attempt)
            .expect("seed terminal attempt");
        let terminal_state = terminal.restart_state();
        terminal.reconcile(&terminal_state).await;
        assert_eq!(
            terminal_state.coding_runs.runner_count(&terminal.attempt_key()),
            0,
            "{status:?} must not recover"
        );
        assert_eq!(terminal.reload().status, status);
    }
}

/// P2 Task 5 迁移回归修复：Running+ReviewRequest 的死 runner 原由 WS
/// StartCoding 直启路径隐式复活，迁移后唯一人工通道是 attach/启动扫描的
/// 半启动恢复。启动扫描必须覆盖该阶段（review 已完成的 durable 事实，
/// 恢复只续推进不重跑 review）；Running+PrepareContext 为过渡/异常态，
/// 保持 fail-safe 不自动复活。
#[tokio::test]
async fn startup_reconcile_recovers_legacy_running_review_request_stage() {
    let fixture = seed_startup_attempt(None, true);
    fixture
        .store()
        .update_attempt_stage(
            "project_0001",
            "issue_0001",
            &fixture.reload().id,
            CodingExecutionStage::ReviewRequest,
        )
        .expect("seed review request stage");
    let fresh = fixture.restart_state();
    fixture.reconcile(&fresh).await;
    assert_eq!(
        fresh.coding_runs.runner_count(&fixture.attempt_key()),
        1,
        "startup reconcile must resume a running review-request attempt"
    );
    assert_eq!(fixture.reload().stage, CodingExecutionStage::ReviewRequest);

    // 对照：Running+PrepareContext 是过渡/异常态，不自动复活。
    let transitional = seed_startup_attempt(None, false);
    let mut transitional_attempt = transitional.reload();
    transitional_attempt.status = CodingAttemptStatus::Running;
    transitional
        .store()
        .write_coding_attempt_for_test(&transitional_attempt)
        .expect("seed running prepare-context attempt");
    let transitional_state = transitional.restart_state();
    transitional.reconcile(&transitional_state).await;
    assert_eq!(
        transitional_state.coding_runs.runner_count(&transitional.attempt_key()),
        0,
        "running prepare-context attempt must stay fail-safe for manual triage"
    );
}
