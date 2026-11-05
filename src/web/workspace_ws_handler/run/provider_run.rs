use super::*;
use crate::product::workspace_engine::PlanAuthorOutputContract;

#[path = "provider_run/entry.rs"]
mod entry;
pub(crate) use entry::{
    ProviderRunStartMode, spawn_provider_run_claiming_idle, spawn_provider_run_from_event,
    spawn_provider_run_from_handler,
};
#[path = "provider_run/basic_arms.rs"]
mod basic_arms;
#[path = "provider_run/sc_manual_revision.rs"]
mod sc_manual_revision;
#[path = "provider_run/single_candidate_arm.rs"]
mod single_candidate_arm;
#[path = "provider_run/task_body.rs"]
mod task_body;
pub(crate) use task_body::provider_run_task_body;
// S0 体积基线（stack_budget）仅测试用窄再导出；库构建不经此处。
#[cfg(test)]
pub(crate) use basic_arms::{
    drive_author_arm, drive_author_choice_followup_arm, drive_review_only_arm, drive_revision_arm,
};
#[cfg(test)]
pub(crate) use sc_manual_revision::drive_human_gate_sc_revision_arm;
#[cfg(test)]
pub(crate) use single_candidate_arm::drive_single_candidate_author_arm;
#[cfg(test)]
pub(crate) use task_body::ArmDrive;

/// B2b3（Task 5.4，设计 §5.3）：读取本会话当前 LC 显式 fresh 恢复记录——
/// 仅 `FreshRunning` 时返回 `Some`（臂内据此区分 fresh 消费与正常续跑：
/// payload 冻结 feedback/节点重建/fresh builder）；正常 run（无恢复/
/// Waiting/Resolved）返回 `None` 不受影响。读取失败 fail-closed 上浮，
/// 不把恢复态吞成正常 run。
pub(super) fn lc_fresh_recovery_record_if_running(
    app_paths: &crate::product::app_paths::ProductAppPaths,
    session_id: &str,
) -> Result<Option<crate::product::models::LcResumeRecoveryRecord>, String> {
    let record = crate::product::lifecycle_store::LifecycleStore::new(app_paths.clone())
        .get_workspace_session(session_id)
        .map_err(|error| format!("load workspace session for fresh recovery failed: {error}"))?;
    Ok(record.lc_resume_recovery.filter(|recovery| {
        recovery.state == crate::product::models::LcResumeRecoveryState::FreshRunning
    }))
}

/// 蒸发链顺手项（取消臂回显化，诊断 local://diag-manual-revision-evaporate.md
/// §a-①）：`tokio::select!` 取消臂 drop drive 后，SC 修订 turn 此前永停
/// Reserved/Running——无失败落盘、无回显帧，后续 feedback 全被 gate_busy 卡死。
/// 本助手是取消臂的收口核心（供取消臂与测试共享）：durable
/// Failed(cancelled) + `HumanGateTurnFailed` 回显帧 + eprintln 打点（对齐
/// aria-cancellation 风格）。返回 `true` 表示发生了真实迁移；刚自然收口的
/// turn 由 `fail_human_gate_turn` 幂等跳过（不回显误导帧）。
pub(crate) async fn fail_cancelled_human_gate_revision_turn(
    engine: &Arc<Mutex<crate::product::workspace_engine::WorkspaceEngine>>,
    outbound_tx: &mpsc::Sender<OutboundControl>,
    session_id: &str,
    turn_id: &str,
) -> bool {
    let mut engine = engine.lock().await;
    match engine
        .fail_human_gate_turn(
            turn_id,
            crate::product::models::HumanGateTurnFailureClass::Cancelled,
        )
        .await
    {
        Ok(true) => {
            drop(engine);
            eprintln!(
                "[aria-cancellation] workspace provider_run_cancelled_fail_turn session_id={session_id} turn_id={turn_id} failure_class=cancelled"
            );
            let _ = send_json_outbound(
                outbound_tx,
                &WsOutMessage::HumanGateTurnFailed {
                    turn_id: turn_id.to_string(),
                    failure_class: "cancelled".to_string(),
                    message:
                        "修订运行已取消（被新命令取代或连接中止），本轮反馈未执行；请重新提交反馈"
                            .to_string(),
                },
            )
            .await;
            true
        }
        Ok(false) => false,
        Err(message) => {
            eprintln!(
                "[aria-cancellation] workspace provider_run_cancelled_fail_turn_error session_id={session_id} turn_id={turn_id} error={message}"
            );
            false
        }
    }
}

/// SG6a.2（REQ-CG-03「修订不过校验」）：SC 修订 turn 的 in-band 结构
/// 拒收回显稳定码——engine 侧 turn 不因拒收终态化，本帧不冒充
/// `HumanGateTurnFailed` 终态（机器修复窗口/原门/等待面承接）。
const SC_MANUAL_REVISION_VALIDATION_REJECTED_CODE: &str = "SC_MANUAL_REVISION_VALIDATION_REJECTED";
/// SG6a.2：attempt2 冻结交付 join 拒绝（SG5.3 稳定码族以 message 携带）
/// ——零 provider start、turn 不终态、typed 等待。
const SC_MANUAL_ATTEMPT2_DELIVERY_REJECTED_CODE: &str = "SC_MANUAL_ATTEMPT2_DELIVERY_REJECTED";
/// SG6a.2：durable 已 claim 的结构 attempt2 跨 spawn 等待——冻结机器
/// 指令 prompt 由 claim 时的 in-flight run 内联消费，跨 spawn 不可重建
/// （P2 修复 aa823108 恢复车道语义的 spawn 侧防线）。
const SC_MANUAL_STRUCTURAL_CLAIM_PENDING_CODE: &str = "SC_MANUAL_STRUCTURAL_CLAIM_PENDING";

/// 取消后 drive 的收口窗口：`run_cancel` 取消时 drive 内建取消臂会执行
/// `finish_aborted_run`（节点 Failed + 回 PrepareContext + StageChange 帧），
/// 这些步骤是短促的本地收口（flush/durable 写/事件帧）。窗口只用于等它跑完，
/// 真卡死（裸 await 悬停/长阻塞 provider）仍按原兜底路径收尾——同时必须显著
/// 小于接替 run 的 engine 锁回收预算（1s 有界，见 provider_run_events
/// 的 supersede_reclaims_engine_lock 用例），也远小于 manager
/// `abort_active_run_and_wait_completion` 的 30s 兜底。
const PROVIDER_RUN_CANCELLED_SETTLE: std::time::Duration = std::time::Duration::from_millis(250);

/// SG6a.2（P3 修复轮）：attempt2 冻结交付 join 的候选基线窗口——按
/// SG5.3 join 口径把失败中间候选布置为内存修订基线（refs/版本/评审
/// 不提升，REQ-CG-03 坏 source 零提升不受影响），执行精确 join。
///
/// join 使命完成（Ok/Err 皆然）后 MUST 统一还原先前基线：attempt2 未
/// 成行（join 拒/身份失配/崩溃重放/二次拒收/任何后续出口）不得污染内
/// 存投影——否则后续 `handle_human_gate_feedback` 以残留的失败候选
/// hash 落 durable 新 turn.source_hash（≠当前修订基线），恢复车道
/// MarkFailed(ValidationReject) 会误杀合法修订轮。
pub(crate) fn deliver_attempt2_frozen_prompt(
    engine: &mut crate::product::workspace_engine::WorkspaceEngine,
    turn: &crate::product::models::HumanGateTurn,
    failed_source: &str,
    carry: &crate::product::workspace_engine::conversational_gate::ScRevisionClaimCarry,
) -> Result<String, String> {
    // 布置：claim 绑定的失败候选=合法中间 hash（join 对账视图）；保留
    // 先前基线供 join 使命结束后还原（P3）。
    let previous_baseline = engine.session.artifact.clone();
    engine.session.artifact = Some(crate::web::workspace_ws_types::ArtifactPayload::Markdown {
        markdown: failed_source.to_string(),
        diff: None,
    });
    let joined =
        engine.build_sc_revision_delivery_for_resume(&turn.command_id, turn, Some(&carry.frozen));
    // join 使命完成（Ok/Err 皆然）统一还原：attempt2 未成行不得污染内存
    // 修订基线（P3——后续 handle_human_gate_feedback 的 durable
    // turn.source_hash 单源读该投影）。
    engine.session.artifact = previous_baseline;
    joined
}

/// 批 2 T2（REQ-WIGA-09）：session 精确绑定的 enrollment 已 terminated 时
/// 拒绝 provider run spawn。仅当 enrollment 显式绑定该 session（session_id
/// 全等）才拦；manual/无绑定/异 session 不拦（手动路径零变化）。
fn ensure_session_enrollment_not_terminated(
    app_paths: &crate::product::app_paths::ProductAppPaths,
    session_record: &crate::product::models::WorkspaceSessionRecord,
) -> Result<(), String> {
    let store =
        crate::product::issue_automation_store::IssueAutomationStore::new(app_paths.clone());
    let enrollment = store
        .get(&session_record.project_id, &session_record.issue_id)
        .map_err(|error| format!("automation enrollment read failed: {error}"))?;
    if let Some(enrollment) = enrollment
        && enrollment.session_id.as_deref() == Some(session_record.id.as_str())
        && let Some(terminated_at) = enrollment.terminated_at
    {
        return Err(format!(
            "automation_plan_terminated: session {} plan terminated at {terminated_at}",
            session_record.id
        ));
    }
    Ok(())
}

pub(super) async fn spawn_provider_run_with_start_mode(
    run_context: ProviderRunContext,
    run_kind: ProviderRunKind,
    outbound_tx: mpsc::Sender<OutboundControl>,
    start_mode: ProviderRunStartMode,
) -> Result<bool, String> {
    let run_context_clone = run_context.clone();
    let ProviderRunContext {
        provider_registry,
        manager,
        engine,
        workspace_runs,
        session_id,
        connection_id,
        app_paths,
        session_record,
    } = run_context;

    // D2.1a（fix-cascade-deletion Task 3.1；REQ-DEL-06，plan §0.4）：任何
    // engine/session 锁之前取得完整 run lease——本函数是 author/review/
    // revision/event-recovery 全部真实入口的单一收口；lease 随任务全生命
    // 周期持有（move 进下方 spawn，drive/收口/最终 store writes 完成后释放），
    // 删除侧 try-exclusive 立即 entity_busy。Deleting/Deleted/Busy 稳定拒，
    // 不复用旧 Arc；旧无 owner 档案（None）保持原行为。
    let entity_run_lease =
        crate::product::provider_workspace_runner::acquire_run_lease_for_session(
            &app_paths,
            &session_record,
        )
        .map_err(|error| match error {
            crate::product::entity_operation_store::EntityOperationError::Deleting => {
                "entity_deleting: workspace session owner entity is being deleted".to_string()
            }
            crate::product::entity_operation_store::EntityOperationError::Deleted => {
                "entity_deleted: workspace session owner entity has been deleted".to_string()
            }
            crate::product::entity_operation_store::EntityOperationError::Busy => {
                "entity_busy: workspace session owner entity admission contended".to_string()
            }
            crate::product::entity_operation_store::EntityOperationError::Store(inner) => {
                format!("entity_admission_failed: workspace session run admission failed: {inner}")
            }
        })?;
    // 批 2 T2（REQ-WIGA-09）：自动化绑定的 plan session 在任何真实 spawn 前
    // 重验 enrollment——已 terminated（终止在途/已完成）一律拒绝新 provider
    // run（迟到事件/续驱不得复活终止链）；非自动化绑定 session 原样放行。
    ensure_session_enrollment_not_terminated(&app_paths, &session_record)?;

    // Handler-originated starts always supersede the active provider run: a user
    // message must cancel a streaming run before waiting for the engine mutex
    // (held by the stream owner while driving its provider session). Engine-
    // originated relays are filtered by `spawn_provider_run_from_event`: same-
    // node duplicates drained; ReviewOnly relays yield to an in-flight run whose
    // followups own the review continuation; repair relays (WorkItemPlan kinds,
    // emitted by a run that delegated via policy routing) keep the supersede
    // hand-off. P1 WIGA Task 5：auto 认领（ClaimIfIdle）不走本段——supersede
    // 只属于显式 human/driver 重跑。
    if matches!(start_mode, ProviderRunStartMode::SupersedeFromAttachment) {
        // 诊断打点（claude×轻 握手谜团第 2 轮，不改行为）：新 run 接替取消旧 run
        // 的唯一裁决点迁入 manager，令所有连接共享同一临界区。
        eprintln!(
            "[aria-cancellation] workspace handler_run_supersede trigger=handler_run_supersede session_id={} kind={:?}",
            session_id, run_kind
        );

        // socket 发起的 supersede 在进入 engine 锁前原子核验 attachment epoch；已被
        // 接管的 stale driver 会在触碰 active run 前返回 STALE_DRIVER_LEASE。
        manager
            .abort_active_run_from_attachment(connection_id.as_deref())
            .await?;
    }
    let provider_name = {
        let engine = engine.lock().await;
        match &run_kind {
            ProviderRunKind::Author { .. }
            | ProviderRunKind::AuthorChoiceFollowup { .. }
            | ProviderRunKind::Revision => engine.session().author_provider.clone(),
            ProviderRunKind::ReviewOnly => {
                // C2 Task 5（REQ-CRO-05）：reviewer 缺失（空 effective）不启动
                // ReviewOnly run——绝不以 Codex 顶替。
                let reviewer = engine.session().reviewer_provider.clone();
                match reviewer {
                    Some(reviewer) => reviewer,
                    None => {
                        return Err(
                            "reviewer_configuration_missing: review run not started".to_string()
                        );
                    }
                }
            }
            ProviderRunKind::WorkItemPlanLegacyAuthor
            | ProviderRunKind::WorkItemPlanSingleCandidateAuthor
            | ProviderRunKind::WorkItemPlanOutlineRevision { .. }
            | ProviderRunKind::WorkItemPlanOutlineRebuild { .. }
            | ProviderRunKind::WorkItemPlanDraft { .. }
            | ProviderRunKind::WorkItemPlanBatch
            | ProviderRunKind::WorkItemPlanRevision { .. }
            | ProviderRunKind::HumanGateScManualRevision { .. } => {
                engine.session().author_provider.clone()
            }
        }
    };
    let provider_for_run = {
        let Some(provider) = provider_registry.get(&provider_name) else {
            return Err(format!("provider unavailable: {provider_name:?}"));
        };
        provider
    };

    let target_node_id = {
        let engine = engine.lock().await;
        engine.active_timeline_node_id()
    };
    let registered = match start_mode {
        ProviderRunStartMode::SupersedeFromAttachment => manager
            .start_run_from_attachment(connection_id.as_deref(), run_kind.clone(), target_node_id)
            .await
            .map(Some)?,
        ProviderRunStartMode::ClaimIfIdle => {
            manager
                .try_start_run_if_idle(run_kind.clone(), target_node_id)
                .await?
        }
    };
    let Some((run_id, run_token, run_cancel, command_rx, _node_id)) = registered else {
        // 认领窗口内活 run 已被他人启动：不 supersede，交由其持有者驱动。
        return Ok(false);
    };
    let run_label = format!("run-{run_id}");
    // provider drive 期标记（idle 关闭守卫扩展）：从 run 任务启动到结束，该 session
    // 的 idle 守卫都不主动关连接；run 所有权已由 manager 跨 socket 保存。
    let provider_drive_guard = workspace_runs.begin_provider_drive(&session_id);

    {
        let mut engine = engine.lock().await;
        engine.mark_active_run_started(run_label.clone());
    }

    let engine_for_run = engine.clone();
    let manager_for_task = manager.clone();
    let provider_registry_for_run = provider_registry.clone();
    let outbound_tx_for_task = outbound_tx.clone();
    let outline_revision_feedback = match &run_kind {
        ProviderRunKind::WorkItemPlanOutlineRevision { feedback } => feedback.clone(),
        _ => None,
    };
    let is_outline_revision_run = matches!(
        &run_kind,
        ProviderRunKind::WorkItemPlanOutlineRevision { .. }
    );
    // 蒸发链顺手项（取消臂回显化）：run_kind 被 move 进 drive 前取出 SC 修订
    // turn id 与回显通道克隆——取消臂 drop drive 后据此终态化 turn 并回显。
    let cancelled_revision_turn_id = match &run_kind {
        ProviderRunKind::HumanGateScManualRevision { turn_id, .. } => Some(turn_id.clone()),
        _ => None,
    };
    let cancel_arm_outbound_tx = outbound_tx_for_task.clone();
    // S0 机械提取：spawn 处直接构造具名 task body future（此步不加
    // trampoline；Box 边界属后续任务）。原匿名 async 块逐字迁移至
    // provider_run/task_body.rs；参数即原块被 move 的逐捕获。
    tokio::spawn(provider_run_task_body(
        run_kind,
        run_context_clone,
        engine_for_run,
        manager_for_task,
        provider_registry_for_run,
        provider_for_run,
        outbound_tx_for_task,
        cancel_arm_outbound_tx,
        session_id,
        run_label,
        run_token,
        run_cancel,
        command_rx,
        provider_drive_guard,
        entity_run_lease,
        outline_revision_feedback,
        is_outline_revision_run,
        cancelled_revision_turn_id,
    ));

    Ok(true)
}

include!("provider_run/single_candidate_admission_waiting.inc.rs");
include!("provider_run/cancel_settle.inc.rs");
