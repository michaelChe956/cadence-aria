//! S0（任务1）机械提取：provider run 任务 body 具名化。
//!
//! 原 `provider_run.rs` `tokio::spawn(async move { ... })` 内约 1360 行匿名
//! 块逐字迁移为具名 `provider_run_task_body`（此步在原深 caller 直接构造、
//! 尚不加 trampoline；Box 边界与 drive body 具名化属后续任务）。基础四臂与
//! SC author 臂已机械提取为具名 async fn（basic_arms.rs /
//! single_candidate_arm.rs）；draft/batch/revision/SC manual/legacy 暂留本
//! 体（include/宏原样），后续任务逐族迁移。
//!
//! 三守卫不可混用（r16 现场死锁根修，缺陷#10家族——设计 §3.4）：
//! - engine 的 `MutexGuard` 按值属 drive/当前臂：drop 取消 drive 必须释放它，
//!   且先于兜底重取锁；
//! - idle `provider_drive_guard` 是外层 `Option<WorkspaceProviderDriveGuard>`
//!   （臂借 `&mut`，drop drive **不等于**释放 idle）；
//! - `entity_run_lease` 是独立外层 binding（覆盖 drive、取消兜底与最终
//!   store 写，接替 run 自行另 acquire 新 lease）。
//!
//! 不得用「provider_drive_guard 随 drive 一律释放」的歧义说法代替三者合同。

use super::*;

use super::basic_arms::{
    drive_author_arm, drive_author_choice_followup_arm, drive_review_only_arm, drive_revision_arm,
};
use super::sc_manual_revision::drive_human_gate_sc_revision_arm;
use super::single_candidate_arm::drive_single_candidate_author_arm;

/// 臂级控制流合同（设计 §3.3）：不是新的业务错误抽象。`Continue` 携带按值
/// 交还的 engine MutexGuard（原正常穿过 match 末尾的路径）；`Stop` 表示臂内
/// 已按原顺序完成该分支收口并退出了原 drive 级 `return` 的路径——父 drive
/// 遇 Stop 立即返回：不继续 followups、不补一次通用 finish、不补错误帧。
pub(crate) enum ArmExit<'e> {
    /// 正常穿过 match：guard 交给下一段（followups + 正常尾部）。
    Continue(tokio::sync::MutexGuard<'e, WorkspaceEngine>),
    /// 原 drive 级 return：臂内收口已完成，drive 立即结束。
    Stop,
}

/// 具名臂共享入参（设计 §3.3 窄参数合同，不克隆整份 ProviderRunContext）：
/// engine guard 与单一 owned `command_rx` 按值进入当前臂；其余共享量按引用
/// （原 provider_for_run / manager_for_task / run_context_clone /
/// outbound_tx_for_task / run_label / run_token / provider_drive_guard /
/// run_cancel）。
pub(crate) struct ArmDrive<'e, 'd> {
    /// engine MutexGuard：按值进臂（见文件头三守卫合同）。`'e` 与
    /// `ArmExit<'e>` 同源——Continue 交还 guard 只携带 `'e`，不把 `'d`
    /// 借用延长到臂返回之后。
    pub(crate) engine: tokio::sync::MutexGuard<'e, WorkspaceEngine>,
    /// 单一 owned Receiver：进入当前臂，不 clone/重建第二消费者。
    pub(crate) command_rx: mpsc::Receiver<ProviderCommand>,
    /// 本 run 选定的 provider。
    pub(crate) provider: Arc<dyn StreamingProviderAdapter>,
    /// manager（原 manager_for_task）。
    pub(crate) manager: &'d Arc<crate::web::workspace_session::WorkspaceSessionManager>,
    /// 运行上下文（原 run_context_clone）。
    pub(crate) run_context: &'d ProviderRunContext,
    /// 出站帧通道（原 outbound_tx_for_task）。
    pub(crate) outbound_tx: &'d mpsc::Sender<OutboundControl>,
    /// run 标签（原 run_label）。
    pub(crate) run_label: &'d str,
    /// run token。
    pub(crate) run_token: u64,
    /// idle 守卫：外层 Option 的可变借用（臂仅既有分支 `.take()`）。
    pub(crate) provider_drive_guard: &'d mut Option<crate::web::state::WorkspaceProviderDriveGuard>,
    /// 取消 token：安装 engine token 与按值 provider API 处才 clone。
    pub(crate) run_cancel: &'d CancellationToken,
}

/// 具名任务 body：参数即原匿名块被 move 的逐捕获（顺序对齐原绑定出现次序）。
#[allow(clippy::too_many_arguments)] // S0 机械提取：原 spawn 块捕获逐参数化，不合并包一层
pub(crate) async fn provider_run_task_body(
    run_kind: ProviderRunKind,
    run_context_clone: ProviderRunContext,
    engine_for_run: Arc<Mutex<WorkspaceEngine>>,
    manager_for_task: Arc<crate::web::workspace_session::WorkspaceSessionManager>,
    provider_registry_for_run: Arc<ProviderRegistry>,
    provider_for_run: Arc<dyn StreamingProviderAdapter>,
    outbound_tx_for_task: mpsc::Sender<OutboundControl>,
    cancel_arm_outbound_tx: mpsc::Sender<OutboundControl>,
    session_id: String,
    run_label: String,
    run_token: u64,
    run_cancel: CancellationToken,
    command_rx: mpsc::Receiver<ProviderCommand>,
    provider_drive_guard: crate::web::state::WorkspaceProviderDriveGuard,
    entity_run_lease: Option<crate::product::entity_operation_store::EntityRunLease>,
    outline_revision_feedback: Option<String>,
    is_outline_revision_run: bool,
    cancelled_revision_turn_id: Option<String>,
) {
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
            // S0 机械提取分发：engine MutexGuard 按值进当前臂（ArmDrive），
            // Continue 交还、Stop 即 drive 级 return（见 ArmExit 合同）。
            ProviderRunKind::Author { content } => {
                engine = match drive_author_arm(
                    ArmDrive {
                        engine,
                        command_rx,
                        provider: provider_for_run.clone(),
                        manager: &manager_for_task,
                        run_context: &run_context_clone,
                        outbound_tx: &outbound_tx_for_task,
                        run_label: &run_label,
                        run_token,
                        provider_drive_guard: &mut provider_drive_guard,
                        run_cancel: &run_cancel,
                    },
                    content,
                )
                .await
                {
                    ArmExit::Continue(engine) => engine,
                    ArmExit::Stop => return,
                };
            }
            ProviderRunKind::AuthorChoiceFollowup { content } => {
                engine = match drive_author_choice_followup_arm(
                    ArmDrive {
                        engine,
                        command_rx,
                        provider: provider_for_run.clone(),
                        manager: &manager_for_task,
                        run_context: &run_context_clone,
                        outbound_tx: &outbound_tx_for_task,
                        run_label: &run_label,
                        run_token,
                        provider_drive_guard: &mut provider_drive_guard,
                        run_cancel: &run_cancel,
                    },
                    content,
                )
                .await
                {
                    ArmExit::Continue(engine) => engine,
                    ArmExit::Stop => return,
                };
            }
            ProviderRunKind::Revision => {
                engine = match drive_revision_arm(ArmDrive {
                    engine,
                    command_rx,
                    provider: provider_for_run.clone(),
                    manager: &manager_for_task,
                    run_context: &run_context_clone,
                    outbound_tx: &outbound_tx_for_task,
                    run_label: &run_label,
                    run_token,
                    provider_drive_guard: &mut provider_drive_guard,
                    run_cancel: &run_cancel,
                })
                .await
                {
                    ArmExit::Continue(engine) => engine,
                    ArmExit::Stop => return,
                };
            }
            ProviderRunKind::ReviewOnly => {
                engine = match drive_review_only_arm(ArmDrive {
                    engine,
                    command_rx,
                    provider: provider_for_run.clone(),
                    manager: &manager_for_task,
                    run_context: &run_context_clone,
                    outbound_tx: &outbound_tx_for_task,
                    run_label: &run_label,
                    run_token,
                    provider_drive_guard: &mut provider_drive_guard,
                    run_cancel: &run_cancel,
                })
                .await
                {
                    ArmExit::Continue(engine) => engine,
                    ArmExit::Stop => return,
                };
            }
            ProviderRunKind::WorkItemPlanLegacyAuthor
            | ProviderRunKind::WorkItemPlanOutlineRevision { .. }
            | ProviderRunKind::WorkItemPlanOutlineRebuild { .. } => {
                include!("work_item_plan_legacy_author.inc.rs")
            }
            ProviderRunKind::WorkItemPlanSingleCandidateAuthor => {
                engine = match drive_single_candidate_author_arm(ArmDrive {
                    engine,
                    command_rx,
                    provider: provider_for_run.clone(),
                    manager: &manager_for_task,
                    run_context: &run_context_clone,
                    outbound_tx: &outbound_tx_for_task,
                    run_label: &run_label,
                    run_token,
                    provider_drive_guard: &mut provider_drive_guard,
                    run_cancel: &run_cancel,
                })
                .await
                {
                    ArmExit::Continue(engine) => engine,
                    ArmExit::Stop => return,
                };
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
                        != Some(crate::web::workspace_ws_types::TimelineNodeType::WorkItemDraftRun)
                    && let Err(message) = engine
                        .start_serial_work_item_draft_run_for(fresh_outline_id)
                        .await
                {
                    engine.mark_active_run_finished(&run_label);
                    drop(engine);
                    let err = WsOutMessage::Error { message };
                    let _ = send_json_outbound(&outbound_tx_for_task, &err).await;
                    return;
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
                                    message: format!("work item draft generate failed: {message}"),
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
                                message: format!("work item draft parse failed: {}", error.message),
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
                        != Some(crate::web::workspace_ws_types::TimelineNodeType::WorkItemBatchRun)
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
                        engine.build_current_work_item_batch_draft_streaming_input_explicit_fresh(
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
                engine = match drive_human_gate_sc_revision_arm(
                    ArmDrive {
                        engine,
                        command_rx,
                        provider: provider_for_run.clone(),
                        manager: &manager_for_task,
                        run_context: &run_context_clone,
                        outbound_tx: &outbound_tx_for_task,
                        run_label: &run_label,
                        run_token,
                        provider_drive_guard: &mut provider_drive_guard,
                        run_cancel: &run_cancel,
                    },
                    turn_id,
                    prompt,
                )
                .await
                {
                    ArmExit::Continue(engine) => engine,
                    ArmExit::Stop => return,
                };
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
    // 取消观察与有界收口窗口（含 drive 被 drop 的原兜底）见 cancel_settle.inc.rs。
    drive_until_cancelled_settles(
        drive,
        &run_cancel,
        cancelled_revision_turn_id,
        &engine_for_run,
        &cancel_arm_outbound_tx,
        &session_id,
        &run_label,
        &supersede_reclaim_manager,
        run_token,
    )
    .await;
}
