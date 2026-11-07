//! P4（任务5）机械迁移：WorkItemPlanBatch 臂。原 `provider_run.rs`
//! `:793–1004`（task_body.rs 迁移前 `:655–859`）逐语句迁移：batch fresh
//! 接管（不另建 batch、不重置 repair 预算、explicit fresh builder）、循环
//! 每轮新 split handle、AwaitConfirmation 仍 feedback=None 继续原 loop 的
//! 语义原样保留（与 draft 臂的 break 差异**不统一**）。
//!
//! 迁移映射（ArmExit 合同，设计 §3.3）：
//! - 原 drive 级 `return` 的出口（含 while 循环内全部出口）按原顺序完成该
//!   分支收口后返回 `Stop`——父 drive 立即返回：不继续 followups、不补一次
//!   通用 finish、不补错误帧；驱动失败支（`mark_active_run_finished` 后直接
//!   return，无 drop engine 显式语句、无 Error 帧）的差异原样映射；
//! - 本臂无 `break`：AwaitConfirmation 置 `feedback = None` 后由 while 条件
//!   （活动节点不再是 WorkItemBatchRun）自然退出 loop，落到函数尾的
//!   `Continue(engine)`——followups 与正常尾部由父 drive 继续；
//! - `command_rx` 只移动一次（`Option<Option<String>>` 的「fresh 冻结为空」
//!   与「非 fresh」区分语义原样保留）。
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

/// WorkItemPlanBatch 臂（原 `:793–1004`，机械迁移）。
pub(crate) fn drive_plan_batch_arm<'e, 'd>(
    drive: ArmDrive<'e, 'd>,
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
                engine.mark_active_run_finished(run_label);
                drop(engine);
                let err = WsOutMessage::Error { message };
                let _ = boxed_child(|| send_json_outbound(outbound_tx_for_task, &err)).await;
                return ArmExit::Stop;
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
                engine.mark_active_run_finished(run_label);
                drop(engine);
                let err = WsOutMessage::Error {
                    message: "work item batch run node unavailable".to_string(),
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
                        "发送给 WorkItemBatch provider 的本地校验修复提示词"
                    } else {
                        "发送给 WorkItemBatch provider 的完整提示词"
                    },
                    Some(author_provider.clone()),
                )
            })
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
                        message: format!("work item batch draft generate failed: {message}"),
                    };
                    let _ = boxed_child(|| send_json_outbound(outbound_tx_for_task, &err)).await;
                    return ArmExit::Stop;
                }
            };
            close_plan_split_run(plan_split_run.as_ref(), &batch_prompt, &full_output);
            let candidate = match parse_work_item_draft_output(structured_output) {
                Ok(candidate) => candidate,
                Err(error) => {
                    engine.mark_active_run_finished(run_label);
                    drop(engine);
                    let err = WsOutMessage::Error {
                        message: format!("work item batch draft parse failed: {}", error.message),
                    };
                    let _ = boxed_child(|| send_json_outbound(outbound_tx_for_task, &err)).await;
                    return ArmExit::Stop;
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
