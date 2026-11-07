//! P4（任务5）机械迁移：WorkItemPlanDraft 臂。原 `provider_run.rs`
//! `:573–792`（task_body.rs 迁移前 `:442–652`）逐语句迁移：显式 fresh
//! 接管（冻结 payload / 节点重建 / explicit fresh builder）、RetryOnce
//! 循环与每轮新 split handle、AwaitConfirmation 的 break 语义原样保留。
//!
//! 迁移映射（ArmExit 合同，设计 §3.3）：
//! - 原 drive 级 `return` 的出口（含 while 循环内全部出口）按原顺序完成该
//!   分支收口后返回 `Stop`——父 drive 立即返回：不继续 followups、不补一次
//!   通用 finish、不补错误帧；驱动失败支（`mark_active_run_finished` 后直接
//!   return，无 drop engine 显式语句、无 Error 帧）的差异原样映射，不借
//!   拆分修注册/错误协议；
//! - 原 `break`（AwaitConfirmation）仍只退出 while loop，落到函数尾的
//!   `Continue(engine)`——followups 与正常尾部由父 drive 继续。
//! - `feedback` / `command_rx` 只移动一次（`Option<Option<String>>` 区分
//!   「fresh 冻结为空」与「非 fresh」的语义原样保留）。
//!
//! 三守卫不可混用（r16 现场死锁根修，缺陷#10家族——设计 §3.4）：
//! - engine 的 `MutexGuard` 按值属 drive/当前臂：drop 取消 drive 必须释放它，
//!   且先于兜底重取锁；
//! - idle `provider_drive_guard` 是外层 `Option<WorkspaceProviderDriveGuard>`
//!   （臂借 `&mut`，drop drive **不等于**释放 idle）；
//! - `entity_run_lease` 是独立外层 binding（不传给臂）。
//!
//! 不得用「provider_drive_guard 随 drive 一律释放」的歧义说法代替三者合同。

use super::task_body::{ArmDrive, ArmExit, boxed_child};
use super::*;
use std::pin::Pin;

/// WorkItemPlanDraft 臂（原 `:573–792`，机械迁移）。`feedback` 来自 run_kind
/// 载荷，按值移入；fresh 冻结 payload 在臂内以 durable 记录覆盖（take 前）。
pub(crate) fn drive_plan_draft_arm<'e, 'd>(
    drive: ArmDrive<'e, 'd>,
    feedback: Option<String>,
) -> Pin<Box<dyn Future<Output = ArmExit<'e>> + Send + 'd>>
where
    'e: 'd,
{
    Box::pin(async move {
        let ArmDrive {
            mut engine,
            command_rx,
            provider: provider_for_run,
            run_context: run_context_clone,
            outbound_tx: outbound_tx_for_task,
            run_label,
            run_cancel,
            // S2：ArmDrive 其余字段（manager/run_token/provider_drive_guard/
            // provider_registry）本臂不消费——`..` 忽略，不改变臂内绑定面。
            ..
        } = drive;
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
                engine.mark_active_run_finished(run_label);
                drop(engine);
                let err = WsOutMessage::Error { message };
                let _ = boxed_child(|| send_json_outbound(outbound_tx_for_task, &err)).await;
                return ArmExit::Stop;
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
            engine.mark_active_run_finished(run_label);
            drop(engine);
            let err = WsOutMessage::Error { message };
            let _ = boxed_child(|| send_json_outbound(outbound_tx_for_task, &err)).await;
            return ArmExit::Stop;
        }
        while engine.active_node_type()
            == Some(crate::web::workspace_ws_types::TimelineNodeType::WorkItemDraftRun)
        {
            let Some(node_id) = engine.active_timeline_node_id() else {
                engine.mark_active_run_finished(run_label);
                drop(engine);
                let err = WsOutMessage::Error {
                    message: "work item draft run node unavailable".to_string(),
                };
                let _ = boxed_child(|| send_json_outbound(outbound_tx_for_task, &err)).await;
                return ArmExit::Stop;
            };
            let plan_launch = match resolve_plan_author_launch(&engine) {
                Ok(launch) => launch,
                Err(error) => {
                    engine.mark_active_run_finished(run_label);
                    drop(engine);
                    let err = WsOutMessage::Error {
                        message: format!("logical plan launch failed: {error}"),
                    };
                    let _ = boxed_child(|| send_json_outbound(outbound_tx_for_task, &err)).await;
                    return ArmExit::Stop;
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
                    let _ = boxed_child(|| send_json_outbound(outbound_tx_for_task, &err)).await;
                    return ArmExit::Stop;
                }
            };
            let author_provider = engine.session().author_provider.clone();
            boxed_child(|| {
                engine.emit_provider_prompt_event(
                    &node_id,
                    provider_input.prompt.clone(),
                    if feedback.is_some() {
                        "发送给 WorkItemDraft provider 的增量返修提示词"
                    } else {
                        "发送给 WorkItemDraft provider 的完整提示词"
                    },
                    Some(author_provider.clone()),
                )
            })
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
                    engine.mark_active_run_finished(run_label);
                    drop(engine);
                    let err = WsOutMessage::Error { message };
                    let _ = boxed_child(|| send_json_outbound(outbound_tx_for_task, &err)).await;
                    return ArmExit::Stop;
                }
            };
            // T6 fix-loop r2：trust 分类已下沉
            // start_work_item_plan_author 的 Logical 分支共享口。
            let provider_session = boxed_child(|| {
                start_work_item_plan_author(
                    plan_launch,
                    provider_for_run.clone(),
                    provider_input,
                    run_cancel.clone(),
                    plan_split_run.as_ref(),
                )
            })
            .await;
            let full_output_result = boxed_child(|| {
                engine.drive_work_item_plan_provider_session_to_output(
                    provider_session,
                    &mut command_rx,
                    node_id,
                    author_provider,
                )
            })
            .await;
            let full_output = match full_output_result {
                Ok(output) => output,
                Err(message) => {
                    fail_plan_split_run(
                        plan_split_run.as_ref(),
                        &format!("provider session drive failed: {message}"),
                    );
                    engine.mark_active_run_finished(run_label);
                    return ArmExit::Stop;
                }
            };
            let structured_output = match parse_work_item_split_structured_output(&full_output) {
                Ok(output) => output,
                Err(message) => {
                    fail_plan_split_run(
                        plan_split_run.as_ref(),
                        &format!("structured output parse failed: {message}"),
                    );
                    engine.mark_active_run_finished(run_label);
                    drop(engine);
                    let err = WsOutMessage::Error {
                        message: format!("work item draft generate failed: {message}"),
                    };
                    let _ = boxed_child(|| send_json_outbound(outbound_tx_for_task, &err)).await;
                    return ArmExit::Stop;
                }
            };
            close_plan_split_run(plan_split_run.as_ref(), &draft_prompt, &full_output);
            let candidate = match parse_work_item_draft_output(structured_output) {
                Ok(candidate) => candidate,
                Err(error) => {
                    engine.mark_active_run_finished(run_label);
                    drop(engine);
                    let err = WsOutMessage::Error {
                        message: format!("work item draft parse failed: {}", error.message),
                    };
                    let _ = boxed_child(|| send_json_outbound(outbound_tx_for_task, &err)).await;
                    return ArmExit::Stop;
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
                    engine.mark_active_run_finished(run_label);
                    drop(engine);
                    let err = WsOutMessage::Error { message };
                    let _ = boxed_child(|| send_json_outbound(outbound_tx_for_task, &err)).await;
                    return ArmExit::Stop;
                }
            }
        }
        ArmExit::Continue(engine)
    })
}
