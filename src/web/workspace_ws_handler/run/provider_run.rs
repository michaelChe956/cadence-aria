use super::*;
use crate::product::workspace_engine::PlanAuthorOutputContract;

#[path = "provider_run/entry.rs"]
mod entry;
pub(crate) use entry::{
    ProviderRunStartMode, spawn_provider_run_claiming_idle, spawn_provider_run_from_event,
    spawn_provider_run_from_handler,
};

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
    tokio::spawn(async move {
        // D2.1a：run lease 随任务全生命周期持有——drive/取消臂收口/最终
        // store writes 完成前不释放（supersede 接力 run 自行另持新 lease）。
        let _entity_run_lease = entity_run_lease;
        let mut provider_drive_guard = Some(provider_drive_guard);
        let mut engine = engine_for_run.lock().await;
        engine.use_run_token(run_cancel.clone());
        // B3：StaleContext 重建携带的 rebuilt planning context（provider run 构造新会话
        // 使用：cwd/inventory/policy digest）。在 `match run_kind` 前提取，供共享 arm 使用。
        let rebuild_context = match &run_kind {
            ProviderRunKind::WorkItemPlanOutlineRebuild { rebuilt } => Some((**rebuilt).clone()),
            _ => None,
        };
        // r16 现场死锁根修(缺陷 #10 家族,授权:run 注册/取消链):驱动体可能
        // 悬停在 select 之外的裸 await(现场形态:provider 启动链/SC 臂体内
        // 链),engine MutexGuard 随任务体永驻,supersede 取消 token 后新 run
        // 的 engine.lock().await 永久排队,WS/HTTP 全冻。取消臂兜底:drive
        // 体被 drop 时 guard 随之释放,run 注册由 finish_run 幂等回收(非本人
        // token 自动复位,不误伤接替 run)。主 select 臂的既有取消观察
        //(provider_drive.rs cancel.cancelled)保持不变——本臂覆盖其盲区。
        let supersede_reclaim_manager = manager_for_task.clone();
        let drive = async {
            match run_kind {
                ProviderRunKind::Author { content } => {
                    engine
                        .handle_user_message_from_run(
                            content,
                            provider_for_run.clone(),
                            command_rx,
                            &run_context_clone.session_record,
                        )
                        .await;
                }
                ProviderRunKind::AuthorChoiceFollowup { content } => {
                    engine
                        .handle_author_choice_followup_from_run(
                            content,
                            provider_for_run.clone(),
                            command_rx,
                            &run_context_clone.session_record,
                        )
                        .await;
                }
                ProviderRunKind::Revision => {
                    engine
                        .drive_revision_session(provider_for_run.clone(), command_rx)
                        .await;
                }
                ProviderRunKind::ReviewOnly => {
                    if engine.logical_provider_gateway().is_some() {
                        engine.drive_review_session_via_gateway(command_rx).await;
                    } else {
                        engine
                            .drive_review_session(provider_for_run.clone(), command_rx)
                            .await;
                    }
                    // SG6b（REQ-CG-03:74 / SG6a.3⑥移交「帧时序」）：本
                    // run 期间 grant 绑定 turn 由 Running 收口为 Completed
                    //（verdict 面）时，在此发送 human_gate_turn_completed
                    // 回呈同代候选——迟到/重复 verdict（turn 已 Completed
                    // 进入本 run 前）不重复发帧。
                    if let Ok(record) = LifecycleStore::new(run_context_clone.app_paths.clone())
                        .get_workspace_session(&engine.session().session_id)
                    {
                        let closed_turn = record
                            .structural_governance_state
                            .as_ref()
                            .and_then(|state| state.current_grant.as_ref())
                            .and_then(|grant| grant.turn_id.clone())
                            .and_then(|turn_id| {
                                LifecycleStore::new(run_context_clone.app_paths.clone())
                                    .get_human_gate_turn(&record.id, &turn_id)
                                    .ok()
                                    .filter(|turn| {
                                        turn.status
                                            == crate::product::models::HumanGateTurnStatus::Completed
                                    })
                                    .filter(|turn| turn.result_artifact_ref.is_some())
                                    .map(|turn| (turn_id, turn.result_artifact_ref))
                            });
                        if let Some((turn_id, Some(artifact_ref))) = closed_turn {
                            let _ = send_json_outbound(
                                &outbound_tx_for_task,
                                &WsOutMessage::HumanGateTurnCompleted {
                                    turn_id,
                                    artifact_ref,
                                },
                            )
                            .await;
                        }
                    }
                }
                ProviderRunKind::WorkItemPlanLegacyAuthor
                | ProviderRunKind::WorkItemPlanOutlineRevision { .. }
                | ProviderRunKind::WorkItemPlanOutlineRebuild { .. } => {
                    include!("provider_run/work_item_plan_legacy_author.inc.rs")
                }
                ProviderRunKind::WorkItemPlanSingleCandidateAuthor => {
                    let mut command_rx = command_rx;
                    // B2c（Task 5.5，设计 §5.3 sc_author 行）：FreshRunning 的
                    // sc_author recovery → LcFreshContext 重入（绕过
                    // reserve/rearm、resume=None 全新 native；verdict 在场走
                    // fresh 返修 prompt 携上一版 source 全文）；其余形态按
                    // Ordinary（mismatch precheck 先于新业务 reserve）。
                    let recovery_entry = match lc_fresh_recovery_record_if_running(
                        &run_context_clone.app_paths,
                        &engine.session().session_id,
                    ) {
                        Ok(Some(recovery)) => match &recovery.run_context {
                            crate::product::models::LcRecoveryRunContext::ScAuthor {
                                source_revision_ref,
                                ..
                            } => {
                                single_candidate::SingleCandidateAuthorRecoveryEntry::LcFreshContext {
                                    source_revision_ref: source_revision_ref.clone(),
                                }
                            }
                            _ => single_candidate::SingleCandidateAuthorRecoveryEntry::Ordinary,
                        },
                        Ok(None) => single_candidate::SingleCandidateAuthorRecoveryEntry::Ordinary,
                        Err(message) => {
                            engine.mark_active_run_finished(&run_label);
                            drop(engine);
                            drop(provider_drive_guard.take());
                            manager_for_task.finish_run(run_token).await;
                            let _ = send_json_outbound(
                                &outbound_tx_for_task,
                                &WsOutMessage::Error { message },
                            )
                            .await;
                            return;
                        }
                    };
                    match single_candidate::run_single_candidate_author(
                        &mut engine,
                        provider_for_run.clone(),
                        run_cancel.clone(),
                        &mut command_rx,
                        recovery_entry,
                        &run_context_clone,
                    )
                    .await
                    {
                        Ok(single_candidate::SingleCandidateProviderRunOutcome::Completed) => {}
                        Err(single_candidate::SingleCandidateProviderRunError::AlreadyFinished) => {
                            engine.mark_active_run_finished(&run_label);
                            drop(engine);
                            drop(provider_drive_guard.take());
                            manager_for_task.finish_run(run_token).await;
                            return;
                        }
                        Err(single_candidate::SingleCandidateProviderRunError::Superseded) => {
                            // 败者让位：键的持有者（健康在途 run 或已完结会话）继续
                            // 拥有本次启动。迟到 run 静默退场——不落 failed 节点、
                            // 不广播 Error、不改写 durable phase（k3 P2）。
                            engine.mark_active_run_finished(&run_label);
                            drop(engine);
                            drop(provider_drive_guard.take());
                            manager_for_task.finish_run(run_token).await;
                            return;
                        }
                        Err(single_candidate::SingleCandidateProviderRunError::Message(
                            message,
                        )) => {
                            engine
                                .finish_active_run_with_failed_node(message.clone())
                                .await;
                            drop(engine);
                            // 缺陷 #10：终态失败路径必须释放 run 注册——对齐同族
                            // AlreadyFinished/Superseded/AdmissionWaiting 分支的既有
                            // finish_run 惯例。修复前提前 return 跳过释放——provider
                            // 已死但 active_run 永驻，is_active_run() 误报
                            // sc_recovery_busy，显式恢复面被僵死注册阻塞
                            //（E2E v1.1 §3.3，曾需人工 WS Abort 清除）。
                            drop(provider_drive_guard.take());
                            manager_for_task.finish_run(run_token).await;
                            let _ = send_json_outbound(
                                &outbound_tx_for_task,
                                &WsOutMessage::Error { message },
                            )
                            .await;
                            return;
                        }
                        Err(
                            single_candidate::SingleCandidateProviderRunError::AdmissionWaiting {
                                reason_code,
                                detail,
                                missing_materials,
                                allowed_actions,
                                manual_guidance,
                                trust_waiting,
                            },
                        ) => {
                            // C-1：LC admission waiting——durable phase 已由运行内
                            // 回落 Prepare 面，此处以无 failed 节点收尾（waiting 不是
                            // 失败），并把缺失材料与允许动作作为可操作指引上浮；
                            // 6.1：trust 缺失时附同源只读人工登记指导（生成场景
                            // retry_action），不再是裸判别码。
                            engine.mark_active_run_finished(&run_label);
                            drop(engine);
                            drop(provider_drive_guard.take());
                            manager_for_task.finish_run(run_token).await;
                            let message = format_single_candidate_admission_waiting(
                                &reason_code,
                                &detail,
                                &missing_materials,
                                &allowed_actions,
                                manual_guidance.as_deref(),
                            );
                            let _ = send_json_outbound(
                                &outbound_tx_for_task,
                                &WsOutMessage::Error { message },
                            )
                            .await;
                            // 指引卡简化（owner 2026-10-12 直批）：trust 缺失时
                            // 紧随 error 帧发送结构化指导帧——前端「两行说明＋
                            // 可复制命令窗」直渲（providers/canonical_root/
                            // command_markdown 同源只读投影），不解析平铺串；
                            // 平铺 error/manual_guidance 原样保留（旧客户端兼容）。
                            if let Some(waiting) = trust_waiting {
                                let _ = send_json_outbound(
                                    &outbound_tx_for_task,
                                    &WsOutMessage::trust_missing_guidance_from_waiting(&waiting),
                                )
                                .await;
                            }
                            return;
                        }
                        Err(
                            single_candidate::SingleCandidateProviderRunError::NativeSupersededWaiting {
                                reason_code,
                                superseded_native_session_id,
                                recovery_id,
                            },
                        ) => {
                            // B2c（Task 5.5，设计 §5.3 SC 纪律①②）：SC native
                            // mismatch 的 typed 等待——AdmissionWaiting 同型收口
                            // （无 failed 节点，等待消息上浮）；durable 已由
                            // `persist_single_candidate_author_superseded_wait`
                            // 收口为 Waiting recovery，原 reservation key/phase
                            // 保留（不 generic Failed、不 rearm），等待显式
                            // fresh（sc_author → LcFreshContext）接管。
                            engine.mark_active_run_finished(&run_label);
                            drop(engine);
                            drop(provider_drive_guard.take());
                            manager_for_task.finish_run(run_token).await;
                            let superseded_native =
                                superseded_native_session_id.as_deref().unwrap_or("unknown");
                            let message = format!(
                                "single candidate author native resume superseded ({reason_code}): \
                                 the old native session {superseded_native} can no longer be resumed \
                                 (recovery {recovery_id}); no provider was started and the original \
                                 reservation/phase are preserved; 请通过显式恢复操作启动新原生会话继续修订（explicit fresh）"
                            );
                            let _ = send_json_outbound(
                                &outbound_tx_for_task,
                                &WsOutMessage::Error { message },
                            )
                            .await;
                            return;
                        }
                    }
                }
                ProviderRunKind::WorkItemPlanDraft { feedback } => {
                    let mut command_rx = command_rx;
                    // B2b3（Task 5.4，设计 §5.3 legacy_draft 行）：显式
                    // fresh 接管——feedback 消费 durable 冻结 payload（take
                    // 前；不再借内存 pending），stale 收口后活动 draft
                    // provider 节点可重建（业务 generation 不重建），构建走
                    // explicit fresh builder（resume=None，不回捞旧 native）。
                    let mut draft_fresh_payload: Option<Option<String>> = None;
                    let draft_fresh_outline = match lc_fresh_recovery_record_if_running(
                        &run_context_clone.app_paths,
                        &engine.session().session_id,
                    ) {
                        Ok(Some(recovery)) => match &recovery.run_context {
                            crate::product::models::LcRecoveryRunContext::LegacyDraft {
                                pending_feedback,
                                active_outline_id,
                                ..
                            } => {
                                draft_fresh_payload = Some(pending_feedback.clone());
                                Some(active_outline_id.clone())
                            }
                            _ => None,
                        },
                        Ok(None) => None,
                        Err(message) => {
                            engine.mark_active_run_finished(&run_label);
                            drop(engine);
                            let err = WsOutMessage::Error { message };
                            let _ = send_json_outbound(&outbound_tx_for_task, &err).await;
                            return;
                        }
                    };
                    let is_draft_fresh = draft_fresh_payload.is_some();
                    let mut feedback = match draft_fresh_payload {
                        Some(payload) => payload,
                        None => feedback.or_else(|| engine.pending_revision_context.clone()),
                    };
                    if let Some(fresh_outline_id) = &draft_fresh_outline
                        && engine.active_node_type()
                            != Some(
                                crate::web::workspace_ws_types::TimelineNodeType::WorkItemDraftRun,
                            )
                    {
                        if let Err(message) = engine
                            .start_serial_work_item_draft_run_for(fresh_outline_id)
                            .await
                        {
                            engine.mark_active_run_finished(&run_label);
                            drop(engine);
                            let err = WsOutMessage::Error { message };
                            let _ = send_json_outbound(&outbound_tx_for_task, &err).await;
                            return;
                        }
                    }
                    while engine.active_node_type()
                        == Some(crate::web::workspace_ws_types::TimelineNodeType::WorkItemDraftRun)
                    {
                        let Some(node_id) = engine.active_timeline_node_id() else {
                            engine.mark_active_run_finished(&run_label);
                            drop(engine);
                            let err = WsOutMessage::Error {
                                message: "work item draft run node unavailable".to_string(),
                            };
                            let _ = send_json_outbound(&outbound_tx_for_task, &err).await;
                            return;
                        };
                        let plan_launch = match resolve_plan_author_launch(&engine) {
                            Ok(launch) => launch,
                            Err(error) => {
                                engine.mark_active_run_finished(&run_label);
                                drop(engine);
                                let err = WsOutMessage::Error {
                                    message: format!("logical plan launch failed: {error}"),
                                };
                                let _ = send_json_outbound(&outbound_tx_for_task, &err).await;
                                return;
                            }
                        };
                        let routing_context = plan_launch.routing_context();
                        // B2b3：fresh 全程走 explicit fresh builder（RetryOnce
                        // 循环内同纪律——每轮 native 启动都不回捞旧会话）。
                        let provider_input = if is_draft_fresh {
                            engine.build_current_work_item_draft_streaming_input_explicit_fresh(
                                feedback.as_deref(),
                                &routing_context,
                            )
                        } else {
                            engine.build_current_work_item_draft_streaming_input(
                                feedback.as_deref(),
                                &routing_context,
                            )
                        };
                        let provider_input = match provider_input {
                            Ok(input) => input,
                            Err(message) => {
                                engine
                                    .finish_active_run_with_failed_node(message.clone())
                                    .await;
                                drop(engine);
                                let err = WsOutMessage::Error { message };
                                let _ = send_json_outbound(&outbound_tx_for_task, &err).await;
                                return;
                            }
                        };
                        let author_provider = engine.session().author_provider.clone();
                        engine
                            .emit_provider_prompt_event(
                                &node_id,
                                provider_input.prompt.clone(),
                                if feedback.is_some() {
                                    "发送给 WorkItemDraft provider 的增量返修提示词"
                                } else {
                                    "发送给 WorkItemDraft provider 的完整提示词"
                                },
                                Some(author_provider.clone()),
                            )
                            .await;
                        let provider_input = engine.attach_tool_policy_audit(provider_input);
                        // Task 1b 段②生产臂:RetryOnce 循环每轮 begin 新 handle。
                        let draft_prompt = provider_input.prompt.clone();
                        let plan_split_run = match begin_plan_split_run_if_logical(
                            &plan_launch,
                            &crate::product::lifecycle_store::LifecycleStore::new(
                                run_context_clone.app_paths.clone(),
                            ),
                            &run_context_clone.session_record.project_id,
                            &run_context_clone.session_record.issue_id,
                            &author_provider,
                            &engine.session().session_id,
                        ) {
                            Ok(run) => run,
                            Err(message) => {
                                engine.mark_active_run_finished(&run_label);
                                drop(engine);
                                let err = WsOutMessage::Error { message };
                                let _ = send_json_outbound(&outbound_tx_for_task, &err).await;
                                return;
                            }
                        };
                        // T6 fix-loop r2：trust 分类已下沉
                        // start_work_item_plan_author 的 Logical 分支共享口。
                        let provider_session = start_work_item_plan_author(
                            plan_launch,
                            provider_for_run.clone(),
                            provider_input,
                            run_cancel.clone(),
                            plan_split_run.as_ref(),
                        )
                        .await;
                        let full_output = match engine
                            .drive_work_item_plan_provider_session_to_output(
                                provider_session,
                                &mut command_rx,
                                node_id,
                                author_provider,
                            )
                            .await
                        {
                            Ok(output) => output,
                            Err(message) => {
                                fail_plan_split_run(
                                    plan_split_run.as_ref(),
                                    &format!("provider session drive failed: {message}"),
                                );
                                engine.mark_active_run_finished(&run_label);
                                return;
                            }
                        };
                        let structured_output =
                            match parse_work_item_split_structured_output(&full_output) {
                                Ok(output) => output,
                                Err(message) => {
                                    fail_plan_split_run(
                                        plan_split_run.as_ref(),
                                        &format!("structured output parse failed: {message}"),
                                    );
                                    engine.mark_active_run_finished(&run_label);
                                    drop(engine);
                                    let err = WsOutMessage::Error {
                                        message: format!(
                                            "work item draft generate failed: {message}"
                                        ),
                                    };
                                    let _ = send_json_outbound(&outbound_tx_for_task, &err).await;
                                    return;
                                }
                            };
                        close_plan_split_run(plan_split_run.as_ref(), &draft_prompt, &full_output);
                        let candidate = match parse_work_item_draft_output(structured_output) {
                            Ok(candidate) => candidate,
                            Err(error) => {
                                engine.mark_active_run_finished(&run_label);
                                drop(engine);
                                let err = WsOutMessage::Error {
                                    message: format!(
                                        "work item draft parse failed: {}",
                                        error.message
                                    ),
                                };
                                let _ = send_json_outbound(&outbound_tx_for_task, &err).await;
                                return;
                            }
                        };
                        match engine
                            .complete_work_item_draft_author(candidate, feedback.as_deref())
                            .await
                        {
                            Ok(WorkItemDraftAuthorOutcome::RetryOnce {
                                feedback: repair_feedback,
                                ..
                            }) => feedback = Some(repair_feedback),
                            Ok(WorkItemDraftAuthorOutcome::AwaitConfirmation) => break,
                            Err(message) => {
                                engine.mark_active_run_finished(&run_label);
                                drop(engine);
                                let err = WsOutMessage::Error { message };
                                let _ = send_json_outbound(&outbound_tx_for_task, &err).await;
                                return;
                            }
                        }
                    }
                }
                ProviderRunKind::WorkItemPlanBatch => {
                    let mut command_rx = command_rx;
                    // B2b3（Task 5.4，设计 §5.3 legacy_batch 行）：显式
                    // fresh 接管——feedback 消费 durable 冻结 payload（take
                    // 前；不借内存 pending），stale 收口后活动 batch provider
                    // 节点可重建（不另建 batch、不重置 repair 预算，同 batch
                    // 完成器收口），构建走 explicit fresh builder（resume=None）。
                    let mut batch_fresh_payload: Option<Option<String>> = None;
                    match lc_fresh_recovery_record_if_running(
                        &run_context_clone.app_paths,
                        &engine.session().session_id,
                    ) {
                        Ok(Some(recovery)) => {
                            if let crate::product::models::LcRecoveryRunContext::LegacyBatch {
                                pending_feedback,
                                ..
                            } = &recovery.run_context
                            {
                                batch_fresh_payload = Some(pending_feedback.clone());
                            }
                        }
                        Ok(None) => {}
                        Err(message) => {
                            engine.mark_active_run_finished(&run_label);
                            drop(engine);
                            let err = WsOutMessage::Error { message };
                            let _ = send_json_outbound(&outbound_tx_for_task, &err).await;
                            return;
                        }
                    }
                    let is_batch_fresh = batch_fresh_payload.is_some();
                    let mut feedback = match batch_fresh_payload {
                        Some(payload) => payload,
                        None => engine.pending_revision_context.clone(),
                    };
                    if is_batch_fresh
                        && engine.active_node_type()
                            != Some(
                                crate::web::workspace_ws_types::TimelineNodeType::WorkItemBatchRun,
                            )
                    {
                        engine.begin_work_item_batch_fresh_recovery_run().await;
                    }
                    while engine.active_node_type()
                        == Some(crate::web::workspace_ws_types::TimelineNodeType::WorkItemBatchRun)
                    {
                        let Some(node_id) = engine.active_timeline_node_id() else {
                            engine.mark_active_run_finished(&run_label);
                            drop(engine);
                            let err = WsOutMessage::Error {
                                message: "work item batch run node unavailable".to_string(),
                            };
                            let _ = send_json_outbound(&outbound_tx_for_task, &err).await;
                            return;
                        };
                        let plan_launch = match resolve_plan_author_launch(&engine) {
                            Ok(launch) => launch,
                            Err(error) => {
                                engine.mark_active_run_finished(&run_label);
                                drop(engine);
                                let err = WsOutMessage::Error {
                                    message: format!("logical plan launch failed: {error}"),
                                };
                                let _ = send_json_outbound(&outbound_tx_for_task, &err).await;
                                return;
                            }
                        };
                        let routing_context = plan_launch.routing_context();
                        // B2b3：fresh 全程走 explicit fresh builder（循环内
                        // 每轮 native 启动都不回捞旧会话，同完整门）。
                        let provider_input = if is_batch_fresh {
                            engine
                                .build_current_work_item_batch_draft_streaming_input_explicit_fresh(
                                    feedback.as_deref(),
                                    &routing_context,
                                )
                        } else {
                            engine.build_current_work_item_batch_draft_streaming_input(
                                feedback.as_deref(),
                                &routing_context,
                            )
                        };
                        let provider_input = match provider_input {
                            Ok(input) => input,
                            Err(message) => {
                                engine
                                    .finish_active_run_with_failed_node(message.clone())
                                    .await;
                                drop(engine);
                                let err = WsOutMessage::Error { message };
                                let _ = send_json_outbound(&outbound_tx_for_task, &err).await;
                                return;
                            }
                        };
                        let author_provider = engine.session().author_provider.clone();
                        engine
                            .emit_provider_prompt_event(
                                &node_id,
                                provider_input.prompt.clone(),
                                if feedback.is_some() {
                                    "发送给 WorkItemBatch provider 的本地校验修复提示词"
                                } else {
                                    "发送给 WorkItemBatch provider 的完整提示词"
                                },
                                Some(author_provider.clone()),
                            )
                            .await;
                        let provider_input = engine.attach_tool_policy_audit(provider_input);
                        // Task 1b 段②生产臂:RetryOnce 循环每轮 begin 新 handle。
                        let batch_prompt = provider_input.prompt.clone();
                        let plan_split_run = match begin_plan_split_run_if_logical(
                            &plan_launch,
                            &crate::product::lifecycle_store::LifecycleStore::new(
                                run_context_clone.app_paths.clone(),
                            ),
                            &run_context_clone.session_record.project_id,
                            &run_context_clone.session_record.issue_id,
                            &author_provider,
                            &engine.session().session_id,
                        ) {
                            Ok(run) => run,
                            Err(message) => {
                                engine.mark_active_run_finished(&run_label);
                                drop(engine);
                                let err = WsOutMessage::Error { message };
                                let _ = send_json_outbound(&outbound_tx_for_task, &err).await;
                                return;
                            }
                        };
                        // T6 fix-loop r2：trust 分类已下沉
                        // start_work_item_plan_author 的 Logical 分支共享口。
                        let provider_session = start_work_item_plan_author(
                            plan_launch,
                            provider_for_run.clone(),
                            provider_input,
                            run_cancel.clone(),
                            plan_split_run.as_ref(),
                        )
                        .await;
                        let full_output = match engine
                            .drive_work_item_plan_provider_session_to_output(
                                provider_session,
                                &mut command_rx,
                                node_id,
                                author_provider,
                            )
                            .await
                        {
                            Ok(output) => output,
                            Err(message) => {
                                fail_plan_split_run(
                                    plan_split_run.as_ref(),
                                    &format!("provider session drive failed: {message}"),
                                );
                                engine.mark_active_run_finished(&run_label);
                                return;
                            }
                        };
                        let structured_output =
                            match parse_work_item_split_structured_output(&full_output) {
                                Ok(output) => output,
                                Err(message) => {
                                    fail_plan_split_run(
                                        plan_split_run.as_ref(),
                                        &format!("structured output parse failed: {message}"),
                                    );
                                    engine.mark_active_run_finished(&run_label);
                                    drop(engine);
                                    let err = WsOutMessage::Error {
                                        message: format!(
                                            "work item batch draft generate failed: {message}"
                                        ),
                                    };
                                    let _ = send_json_outbound(&outbound_tx_for_task, &err).await;
                                    return;
                                }
                            };
                        close_plan_split_run(plan_split_run.as_ref(), &batch_prompt, &full_output);
                        let candidate = match parse_work_item_draft_output(structured_output) {
                            Ok(candidate) => candidate,
                            Err(error) => {
                                engine.mark_active_run_finished(&run_label);
                                drop(engine);
                                let err = WsOutMessage::Error {
                                    message: format!(
                                        "work item batch draft parse failed: {}",
                                        error.message
                                    ),
                                };
                                let _ = send_json_outbound(&outbound_tx_for_task, &err).await;
                                return;
                            }
                        };
                        match engine
                            .complete_work_item_batch_draft_author(candidate, feedback.as_deref())
                            .await
                        {
                            Ok(WorkItemDraftAuthorOutcome::RetryOnce {
                                feedback: repair_feedback,
                                ..
                            }) => feedback = Some(repair_feedback),
                            Ok(WorkItemDraftAuthorOutcome::AwaitConfirmation) => feedback = None,
                            Err(message) => {
                                engine.mark_active_run_finished(&run_label);
                                drop(engine);
                                let err = WsOutMessage::Error { message };
                                let _ = send_json_outbound(&outbound_tx_for_task, &err).await;
                                return;
                            }
                        }
                    }
                }
                ProviderRunKind::WorkItemPlanRevision { feedback } => {
                    workspace_ws_work_item_plan_revision_arm!(
                        engine,
                        run_context_clone,
                        provider_for_run,
                        run_cancel,
                        command_rx,
                        run_label,
                        outbound_tx_for_task,
                        manager_for_task,
                        run_token,
                        provider_drive_guard,
                        feedback
                    );
                }
                ProviderRunKind::HumanGateScManualRevision { turn_id, prompt } => {
                    use crate::product::models::HumanGateTurnFailureClass;
                    let mut command_rx = command_rx;
                    // B2d（Task 5.6，设计 §5.3 sc_manual_revision 行）：
                    // FreshRunning 的 sc_manual recovery → 同 turn fresh 重入
                    //（重装同 turn prompt、resume=None 全新 native、
                    // run_sc_manual_revision_turn 同 turn 完成回原 human gate，
                    // 不新 turn/attempt/预算）；Ordinary 先行 native mismatch
                    // precheck——`FrozenBindingUnavailable` 家族转 typed 等待
                    //（persist 后原 turn/gate/预算保留，不 fail turn、不
                    // rearm、零 provider start）。读取失败按 Message 面收口
                    //（不确定等待/失败归属时不落 turn 失败）。
                    let manual_fresh_recovery = match lc_fresh_recovery_record_if_running(
                        &run_context_clone.app_paths,
                        &engine.session().session_id,
                    ) {
                        Ok(Some(recovery)) => matches!(
                            &recovery.run_context,
                            crate::product::models::LcRecoveryRunContext::ScManualRevision {
                                turn_id: recovery_turn_id,
                                ..
                            } if recovery_turn_id == &turn_id
                        ),
                        Ok(None) => false,
                        Err(message) => {
                            engine.mark_active_run_finished(&run_label);
                            drop(engine);
                            drop(provider_drive_guard.take());
                            manager_for_task.finish_run(run_token).await;
                            let _ = send_json_outbound(
                                &outbound_tx_for_task,
                                &WsOutMessage::Error { message },
                            )
                            .await;
                            return;
                        }
                    };
                    if !manual_fresh_recovery {
                        match engine.precheck_sc_manual_revision_resume() {
                            Ok(None) => {}
                            Ok(Some(reason_code)) => {
                                match engine
                                    .persist_sc_manual_revision_superseded_wait(
                                        &turn_id,
                                        reason_code,
                                    )
                                    .await
                                {
                                    Ok(recovery) => {
                                        // typed 等待收口：无 failed 节点（等待
                                        // 不是失败）、turn/gate/预算原样保留，
                                        // 等待消息上浮显式 fresh 出口。
                                        engine.mark_active_run_finished(&run_label);
                                        drop(engine);
                                        drop(provider_drive_guard.take());
                                        manager_for_task.finish_run(run_token).await;
                                        let superseded_native = recovery
                                            .superseded_native_session_id
                                            .as_deref()
                                            .unwrap_or("unknown");
                                        let message = format!(
                                            "sc manual revision native resume superseded ({reason_code}): \
                                             the old native session {superseded_native} can no longer be resumed \
                                             (recovery {}); no provider was started and the original \
                                             turn/gate/budget are preserved; 请通过显式恢复操作启动新原生会话继续同turn修订（explicit fresh）",
                                            recovery.recovery_id
                                        );
                                        let _ = send_json_outbound(
                                            &outbound_tx_for_task,
                                            &WsOutMessage::Error { message },
                                        )
                                        .await;
                                        return;
                                    }
                                    Err(message) => {
                                        let _ = engine
                                            .fail_human_gate_turn(
                                                &turn_id,
                                                HumanGateTurnFailureClass::ProviderErr,
                                            )
                                            .await;
                                        engine.mark_active_run_finished(&run_label);
                                        drop(engine);
                                        drop(provider_drive_guard.take());
                                        manager_for_task.finish_run(run_token).await;
                                        let _ = send_json_outbound(
                                            &outbound_tx_for_task,
                                            &WsOutMessage::HumanGateTurnFailed {
                                                turn_id: turn_id.clone(),
                                                failure_class: "provider_err".to_string(),
                                                message,
                                            },
                                        )
                                        .await;
                                        return;
                                    }
                                }
                            }
                            Err(message) => {
                                let _ = engine
                                    .fail_human_gate_turn(
                                        &turn_id,
                                        HumanGateTurnFailureClass::ProviderErr,
                                    )
                                    .await;
                                engine.mark_active_run_finished(&run_label);
                                drop(engine);
                                drop(provider_drive_guard.take());
                                manager_for_task.finish_run(run_token).await;
                                let _ = send_json_outbound(
                                    &outbound_tx_for_task,
                                    &WsOutMessage::HumanGateTurnFailed {
                                        turn_id: turn_id.clone(),
                                        failure_class: "provider_err".to_string(),
                                        message,
                                    },
                                )
                                .await;
                                return;
                            }
                        }
                    }
                    // SG6a.2：durable 活动 grant（phase=Attempt2Claimed）＝
                    // 本 turn 的 attempt2 已被唯一 claim（冻结机器指令
                    // prompt 身份在案）。冻结全文只随 claim 时的 in-flight
                    // run 内联交付，跨 spawn 不可重建——原反馈 prompt 重入
                    // 会错消费唯一 attempt2（REQ-CG-03 机器指令驱动；P2
                    // 修复 aa823108 恢复车道语义的 spawn 侧防线）。读取
                    // 失败按 pending 处理（fail-closed 等待，零 start），
                    // 终结收口归 SG6a.3。
                    let structural_claim_pending = LifecycleStore::new(
                        run_context_clone.app_paths.clone(),
                    )
                    .get_workspace_session(&engine.session().session_id)
                    .map(|record| {
                        record
                            .structural_governance_state
                            .as_ref()
                            .and_then(|state| state.current_grant.as_ref())
                            .is_some_and(|grant| {
                                grant.turn_id.as_deref() == Some(turn_id.as_str())
                                    && grant.phase
                                        == Some(crate::product::models::workspace::StructuralGrantPhase::Attempt2Claimed)
                            })
                    })
                    .unwrap_or(true);
                    if structural_claim_pending && !manual_fresh_recovery {
                        engine.mark_active_run_finished(&run_label);
                        drop(engine);
                        drop(provider_drive_guard.take());
                        manager_for_task.finish_run(run_token).await;
                        let _ = send_json_outbound(
                            &outbound_tx_for_task,
                            &WsOutMessage::ProtocolError {
                                code: SC_MANUAL_STRUCTURAL_CLAIM_PENDING_CODE.to_string(),
                                message: format!(
                                    "sc manual revision turn {turn_id} has a claimed structural attempt2 \
                                     (frozen machine-instruction prompt identity on record); the frozen \
                                     delivery is consumed by the in-flight run and cannot be reassembled \
                                     by a new spawn; no provider was started and the turn/gate/budget \
                                     are preserved; waiting for structural closure"
                                ),
                                context: None,
                            },
                        )
                        .await;
                        return;
                    }
                    let mut attempt_prompt = if manual_fresh_recovery || prompt.is_empty() {
                        let turn = match LifecycleStore::new(run_context_clone.app_paths.clone())
                            .get_human_gate_turn(&engine.session().session_id, &turn_id)
                        {
                            Ok(turn) => turn,
                            Err(error) => {
                                let message = format!("load human gate turn failed: {error}");
                                let _ = engine
                                    .fail_human_gate_turn(
                                        &turn_id,
                                        HumanGateTurnFailureClass::ProviderErr,
                                    )
                                    .await;
                                engine.mark_active_run_finished(&run_label);
                                drop(engine);
                                let _ = send_json_outbound(
                                    &outbound_tx_for_task,
                                    &WsOutMessage::HumanGateTurnFailed {
                                        turn_id,
                                        failure_class: "provider_err".to_string(),
                                        message,
                                    },
                                )
                                .await;
                                return;
                            }
                        };
                        // B2d：fresh 重入按 §5.3 同 turn 语义重装——重读同
                        // turn feedback 并对 candidate hash 复验（漂移按业务
                        // 校验规则 validation_reject 收口，与恢复面 source-hash
                        // 失配同族；不用旧 turn 权限强行修订漂移后的候选）。
                        let built = if manual_fresh_recovery {
                            engine.build_sc_manual_revision_prompt_for_fresh_turn(&turn)
                        } else {
                            engine.build_sc_manual_revision_prompt_for_turn(&turn.feedback_text)
                        };
                        match built {
                            Ok(prompt) => prompt,
                            Err(message) => {
                                let (failure_class, class_label) = if message
                                    .starts_with("HUMAN_GATE_REVISION_CANDIDATE_DRIFTED")
                                {
                                    (
                                        HumanGateTurnFailureClass::ValidationReject,
                                        "validation_reject",
                                    )
                                } else {
                                    (HumanGateTurnFailureClass::ProviderErr, "provider_err")
                                };
                                let _ = engine.fail_human_gate_turn(&turn_id, failure_class).await;
                                engine.mark_active_run_finished(&run_label);
                                drop(engine);
                                let _ = send_json_outbound(
                                    &outbound_tx_for_task,
                                    &WsOutMessage::HumanGateTurnFailed {
                                        turn_id,
                                        failure_class: class_label.to_string(),
                                        message,
                                    },
                                )
                                .await;
                                return;
                            }
                        }
                    } else {
                        prompt
                    };

                    // Durable state must prove Running before the provider is started. This is
                    // deliberately inside the spawned task, after the handler has cancelled any
                    // previous run, so a crash cannot leave Reserved while a provider is active.
                    if let Err(message) = engine.mark_human_gate_turn_running(&turn_id) {
                        let _ = engine
                            .fail_human_gate_turn(&turn_id, HumanGateTurnFailureClass::ProviderErr)
                            .await;
                        engine.mark_active_run_finished(&run_label);
                        drop(engine);
                        let _ = send_json_outbound(
                            &outbound_tx_for_task,
                            &WsOutMessage::HumanGateTurnFailed {
                                turn_id,
                                failure_class: "provider_err".to_string(),
                                message,
                            },
                        )
                        .await;
                        return;
                    }

                    // F-49/A6：门修订轮必须与普通 SC 修订同构地落在 author 节点上。
                    // 此前取 `active_timeline_node_id()`——门内活动节点就是 HumanConfirm
                    // 门节点，修订 prompt/输出流/artifact_ref 与产物版本 source_node_id
                    // 全部写进门节点 detail；门节点在对话流 rebuild 判 role=null，整节点
                    // 零条目，用户侧「author 修订步不可见」。选节点规则（复用活动
                    // AuthorRun / 新建）见 `begin_work_item_plan_human_gate_revision_run`。
                    let node_id = engine.begin_work_item_plan_human_gate_revision_run().await;
                    let author_provider = engine.session().author_provider.clone();
                    // B2b1(Task 5.2,REQ-PLN-08):修订 turn 与 fresh 轮的
                    // envelope target 同源恒聚合根锚(E native anchor 消费面
                    // 已收敛进 `resolve_plan_author_launch`,成员锚参数移除
                    // ——r27 的锚漂移机制在 root 锚下结构性消失)。SG6a.2:
                    // sameauthor/gateway key——launch 循环外解析一次,两次
                    // 物理 attempt 共用同一 gateway 启动身份。
                    let launch = resolve_plan_author_launch(&engine);
                    let launch = match launch {
                        Ok(launch) => launch,
                        Err(error) => {
                            let message = error.details.clone();
                            let _ = engine
                                .fail_human_gate_turn(
                                    &turn_id,
                                    HumanGateTurnFailureClass::ProviderErr,
                                )
                                .await;
                            engine.mark_active_run_finished(&run_label);
                            drop(engine);
                            let _ = send_json_outbound(
                                &outbound_tx_for_task,
                                &WsOutMessage::HumanGateTurnFailed {
                                    turn_id,
                                    failure_class: "provider_err".to_string(),
                                    message,
                                },
                            )
                            .await;
                            return;
                        }
                    };
                    // SG6a.2(REQ-CG-03):同 turn 物理 attempt 循环——含原
                    // 调用最多 2,claim 与真实 start 分账。attempt1=原反馈
                    // prompt;attempt1 结构拒收携案 claim 时以冻结机器指令
                    // prompt 内联驱动 attempt2(冻结交付 join 拒→零 start)。
                    let mut structural_claim_consumed = false;
                    loop {
                        engine
                            .emit_provider_prompt_event(
                                &node_id,
                                attempt_prompt.clone(),
                                "发送给 SC human-gate revision provider 的完整修订提示词",
                                Some(author_provider.clone()),
                            )
                            .await;
                        let worktree_path = engine
                            .session()
                            .repository_path
                            .as_ref()
                            .cloned()
                            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
                        // B2d：fresh 重入全新 native（resume_provider_session_id=None
                        // 结构保证——`build_work_item_plan_streaming_input_fresh`）；
                        // Ordinary 保持既有 resume 语义（行为不变）。
                        let provider_input = if manual_fresh_recovery {
                            engine.build_work_item_plan_streaming_input_fresh(
                            crate::product::work_item_split_engine::types::provider_name_to_type(
                                &author_provider,
                            ),
                            attempt_prompt.clone(),
                            worktree_path.to_string_lossy().to_string(),
                            author_provider.clone(),
                            PlanAuthorOutputContract::Structured,
                        )
                        } else {
                            engine.build_work_item_plan_streaming_input(
                            crate::product::work_item_split_engine::types::provider_name_to_type(
                                &author_provider,
                            ),
                            attempt_prompt.clone(),
                            worktree_path.to_string_lossy().to_string(),
                            author_provider.clone(),
                            PlanAuthorOutputContract::Structured,
                        )
                        };
                        let provider_input = match provider_input {
                            Ok(provider_input) => provider_input,
                            Err(message) => {
                                // REQ-PIB-02（T2.3）：基线不可解析 → 门内轮次失败（可观测）。
                                let _ = engine
                                    .fail_human_gate_turn(
                                        &turn_id,
                                        HumanGateTurnFailureClass::ProviderErr,
                                    )
                                    .await;
                                engine.mark_active_run_finished(&run_label);
                                drop(engine);
                                let _ = send_json_outbound(
                                    &outbound_tx_for_task,
                                    &WsOutMessage::HumanGateTurnFailed {
                                        turn_id,
                                        failure_class: "provider_err".to_string(),
                                        message,
                                    },
                                )
                                .await;
                                return;
                            }
                        };
                        let provider_input = engine.attach_tool_policy_audit(provider_input);
                        // Task 1b 段②生产臂:门修订轮单次 provider run 同样绑定
                        // split run 身份;Markdown 修订产物按原文存档收口。
                        let plan_split_run = match begin_plan_split_run_if_logical(
                            &launch,
                            &crate::product::lifecycle_store::LifecycleStore::new(
                                run_context_clone.app_paths.clone(),
                            ),
                            &run_context_clone.session_record.project_id,
                            &run_context_clone.session_record.issue_id,
                            &author_provider,
                            &engine.session().session_id,
                        ) {
                            Ok(run) => run,
                            Err(message) => {
                                let _ = engine
                                    .fail_human_gate_turn(
                                        &turn_id,
                                        HumanGateTurnFailureClass::ProviderErr,
                                    )
                                    .await;
                                engine.mark_active_run_finished(&run_label);
                                drop(engine);
                                let _ = send_json_outbound(
                                    &outbound_tx_for_task,
                                    &WsOutMessage::HumanGateTurnFailed {
                                        turn_id,
                                        failure_class: "provider_err".to_string(),
                                        message,
                                    },
                                )
                                .await;
                                return;
                            }
                        };
                        // T6 fix-loop r2：trust 分类已下沉
                        // start_work_item_plan_author 的 Logical 分支共享口。
                        let provider_session = start_work_item_plan_author(
                            launch.clone(),
                            provider_for_run.clone(),
                            provider_input,
                            run_cancel.clone(),
                            plan_split_run.as_ref(),
                        )
                        .await;
                        let full_output = match engine
                            .drive_work_item_plan_provider_session_to_output(
                                provider_session,
                                &mut command_rx,
                                node_id.clone(),
                                author_provider.clone(),
                            )
                            .await
                        {
                            Ok(output) => output,
                            Err(message) => {
                                fail_plan_split_run(
                                    plan_split_run.as_ref(),
                                    &format!("provider session drive failed: {message}"),
                                );
                                let _ = engine
                                    .fail_human_gate_turn(
                                        &turn_id,
                                        HumanGateTurnFailureClass::ProviderErr,
                                    )
                                    .await;
                                engine.mark_active_run_finished(&run_label);
                                drop(engine);
                                let _ = send_json_outbound(
                                    &outbound_tx_for_task,
                                    &WsOutMessage::HumanGateTurnFailed {
                                        turn_id,
                                        failure_class: "provider_err".to_string(),
                                        message,
                                    },
                                )
                                .await;
                                return;
                            }
                        };
                        close_plan_split_run_with_markdown(
                            plan_split_run.as_ref(),
                            &attempt_prompt,
                            &full_output,
                        );
                        match engine
                        .run_sc_manual_revision_turn(&turn_id, full_output.clone())
                        .await
                    {
                        Ok(crate::product::workspace_engine::ScManualRevisionResult::Accepted { artifact_ref }) => {
                            // SG6b（REQ-CG-03:74 / SG6a.3⑥移交「帧时序」）：
                            // human_gate_turn_completed 只在 turn durable
                            // Completed（synthetic 收口）后回呈；有 reviewer
                            // 分支 turn 仍 Running（verdict 面收口），帧由
                            // ReviewOnly 臂在 verdict 收口后发送。
                            let turn_closed = LifecycleStore::new(
                                run_context_clone.app_paths.clone(),
                            )
                            .get_human_gate_turn(&engine.session().session_id, &turn_id)
                            .map(|turn| {
                                turn.status
                                    == crate::product::models::HumanGateTurnStatus::Completed
                            })
                            .unwrap_or(false);
                            if turn_closed {
                                let _ = send_json_outbound(
                                    &outbound_tx_for_task,
                                    &WsOutMessage::HumanGateTurnCompleted { turn_id: turn_id.clone(), artifact_ref },
                                ).await;
                            }
                            break;
                        }
                        Ok(crate::product::workspace_engine::ScManualRevisionResult::ValidationRejected { diagnostics, claim }) => {
                            let message = diagnostics.join("; ");
                            let carry = match claim {
                                // 唯一 claim 恰消费一次：attempt2 的再拒收
                                // 恒不携案（Replayed/Exhausted 零新 claim）。
                                Some(carry) if !structural_claim_consumed => carry,
                                _ => {
                                    // SG6a.2 in-band 回显：turn 非终态
                                    // （REQ-CG-03「修订不过校验」），不冒充
                                    // HumanGateTurnFailed 终态帧；机器修复
                                    // 窗口/原门/等待面承接，终结收口归
                                    // SG6a.3。
                                    let _ = send_json_outbound(
                                        &outbound_tx_for_task,
                                        &WsOutMessage::ProtocolError {
                                            code: SC_MANUAL_REVISION_VALIDATION_REJECTED_CODE.to_string(),
                                            message,
                                            context: None,
                                        },
                                    ).await;
                                    break;
                                }
                            };
                            // 冻结交付精确 join（SG5.3）：input/fingerprint
                            // 拒绝在任何真实 start 之前稳定码回显——零
                            // start、turn 不终态、typed 等待。
                            let turn_now = match LifecycleStore::new(run_context_clone.app_paths.clone())
                                .get_human_gate_turn(&engine.session().session_id, &turn_id)
                            {
                                Ok(turn) => turn,
                                Err(error) => {
                                    let message = format!("load human gate turn failed: {error}");
                                    let _ = engine.fail_human_gate_turn(&turn_id, HumanGateTurnFailureClass::ProviderErr).await;
                                    engine.mark_active_run_finished(&run_label);
                                    drop(engine);
                                    let _ = send_json_outbound(
                                        &outbound_tx_for_task,
                                        &WsOutMessage::HumanGateTurnFailed {
                                            turn_id: turn_id.clone(),
                                            failure_class: "provider_err".to_string(),
                                            message,
                                        },
                                    ).await;
                                    return;
                                }
                            };
                            // 交付 join 的候选基线窗口（SG6a.2 P3）：布置
                            // 失败中间候选为 join 对账视图、精确 join——
                            // 基线还原合同由 `deliver_attempt2_frozen_prompt`
                            // 统一承载（attempt2 未成行不污染内存修订基
                            // 线，见其注释）。
                            let frozen_prompt = match deliver_attempt2_frozen_prompt(
                                &mut engine,
                                &turn_now,
                                full_output.as_str(),
                                &carry,
                            ) {
                                Ok(frozen_prompt) => frozen_prompt,
                                Err(message) => {
                                    engine.mark_active_run_finished(&run_label);
                                    drop(engine);
                                    drop(provider_drive_guard.take());
                                    manager_for_task.finish_run(run_token).await;
                                    let _ = send_json_outbound(
                                        &outbound_tx_for_task,
                                        &WsOutMessage::ProtocolError {
                                            code: SC_MANUAL_ATTEMPT2_DELIVERY_REJECTED_CODE.to_string(),
                                            message,
                                            context: None,
                                        },
                                    ).await;
                                    return;
                                }
                            };
                            // 实际 launch observation 落 ledger（REQ-SG-05
                            // claim 与真实 start 分账；claim 已保证 key 缺
                            // 席，watchdog 同款 serde 投影＋内存同步）。
                            // 登记失败 fail-closed：零 start、typed 等待。
                            let provider_text = serde_json::to_value(&author_provider)
                                .ok()
                                .and_then(|value| value.as_str().map(ToString::to_string))
                                .unwrap_or_else(|| "unknown".to_string());
                            let started_at = chrono::Utc::now().to_rfc3339();
                            match LifecycleStore::new(run_context_clone.app_paths.clone())
                                .claim_provider_start_with_details(
                                    &engine.session().session_id,
                                    &carry.provider_start_key,
                                    Some(provider_text.as_str()),
                                    Some(started_at.clone()),
                                )
                            {
                                Ok(true) => {
                                    engine.session.provider_start_ledger.push(
                                        crate::product::work_item_plan_policy::ProviderStartLedgerEntry {
                                            provider_start_idempotency_key: carry.provider_start_key.clone(),
                                            started: true,
                                            provider: Some(provider_text),
                                            started_at: Some(started_at),
                                        },
                                    );
                                }
                                Ok(false) => {
                                    // 崩溃重放：实际 launch 观察已在案
                                    // ——幂等续跑同 attempt，不二次计。
                                }
                                Err(error) => {
                                    engine.mark_active_run_finished(&run_label);
                                    drop(engine);
                                    drop(provider_drive_guard.take());
                                    manager_for_task.finish_run(run_token).await;
                                    let _ = send_json_outbound(
                                        &outbound_tx_for_task,
                                        &WsOutMessage::ProtocolError {
                                            code: SC_MANUAL_ATTEMPT2_DELIVERY_REJECTED_CODE.to_string(),
                                            message: format!(
                                                "STRUCTURAL_ATTEMPT2_LAUNCH_REGISTRATION_FAILED: {error}; no provider was started and the turn/gate/budget are preserved"
                                            ),
                                            context: None,
                                        },
                                    ).await;
                                    return;
                                }
                            }
                            structural_claim_consumed = true;
                            attempt_prompt = frozen_prompt;
                            continue;
                        }
                        Err(message) => {
                            let _ = engine.fail_human_gate_turn(
                                &turn_id,
                                HumanGateTurnFailureClass::ProviderErr,
                            ).await;
                            let _ = send_json_outbound(
                                &outbound_tx_for_task,
                                &WsOutMessage::HumanGateTurnFailed {
                                    turn_id: turn_id.clone(),
                                    failure_class: "provider_err".to_string(),
                                    message,
                                },
                            ).await;
                            break;
                        }
                    }
                    } // SG6a.2：同 turn attempt 循环收口（恰一次 claim 消费）
                }
            }
            workspace_ws_provider_run_followups!(
                engine,
                provider_registry_for_run,
                manager_for_task,
                run_token,
                run_label,
                outbound_tx_for_task,
                run_cancel,
                run_context_clone,
                provider_drive_guard
            );
            engine.mark_active_run_finished(&run_label);
            drop(engine);
            drop(provider_drive_guard.take());
            manager_for_task.finish_run(run_token).await;
        };
        tokio::select! {
            _ = run_cancel.cancelled() => {
                // 蒸发链顺手项（取消臂回显化）：drive 被 drop，run 永不收口——
                // SC 修订 turn 会永停 Reserved/Running（后续 feedback 全被
                // gate_busy 卡死）且零回显。此处兜底：打点 + durable
                // Failed(cancelled) + HumanGateTurnFailed 回显帧；刚自然收口的
                // turn 幂等跳过（见助手注释）。
                eprintln!(
                    "[aria-cancellation] workspace provider_run_cancelled_dropped_drive session_id={} run_label={} revision_turn={:?}",
                    session_id, run_label, cancelled_revision_turn_id
                );
                if let Some(turn_id) = cancelled_revision_turn_id {
                    fail_cancelled_human_gate_revision_turn(
                        &engine_for_run,
                        &cancel_arm_outbound_tx,
                        &session_id,
                        &turn_id,
                    )
                    .await;
                }
                let _ = supersede_reclaim_manager.finish_run(run_token).await;
            }
            _ = drive => {}
        }
    });

    Ok(true)
}

include!("provider_run/single_candidate_admission_waiting.inc.rs");
