//! P3（任务4）机械迁移：legacy outline 三族臂（WorkItemPlanLegacyAuthor /
//! OutlineRevision / OutlineRebuild）。原 `provider_run.rs` include
//! `work_item_plan_legacy_author.inc.rs` 全文转为真正函数；三 variant 在
//! `task_body.rs` 分发点移入精确载荷：Legacy=(None,false,None)、
//! Revision=(feedback,true,None)、Rebuild=(None,false,Some(rebuilt))。
//!
//! 迁移映射（ArmExit 合同，设计 §3.3）：
//! - 原 drive 级 `return` 的出口（含 AutoRevision 循环内全部出口）按原顺序
//!   完成该分支收口后返回 `Stop`——父 drive 立即返回：不继续 followups、
//!   不补一次通用 finish、不补错误帧；
//! - 原 `break` 仍只退出相应 loop（本臂无 break；循环所有路径均收口返回，
//!   不存在穿出 loop 的正常路径，故无 Continue 出口——若未来引入 break，
//!   落点应是 `Continue(engine)`）；
//! - `rebuilt` 由「共通裸 `(**rebuilt).clone()` 副本」改为直接把 run_kind 的
//!   `Box<ResolvedPlanningContext>` 按值移入本臂，臂内以 `as_deref()` 借用
//!   现载荷，不再 clone 全文结构（设计 §3.3 rebuild_context 行）。
//!
//! 三守卫不可混用（r16 现场死锁根修，缺陷#10家族——设计 §3.4）：
//! - engine 的 `MutexGuard` 按值属 drive/当前臂：drop 取消 drive 必须释放它，
//!   且先于兜底重取锁；
//! - idle `provider_drive_guard` 是外层 `Option<WorkspaceProviderDriveGuard>`
//!   （臂借 `&mut`，drop drive **不等于**释放 idle）；
//! - `entity_run_lease` 是独立外层 binding（不传给臂）。
//!
//! 不得用「provider_drive_guard 随 drive 一律释放」的歧义说法代替三者合同。

use super::task_body::{ArmDrive, ArmExit};
use super::*;

/// legacy outline 三族臂（原 include 全文迁移）。`outline_revision_feedback`
/// / `is_outline_revision_run` / `rebuild_context` 按 variant 从 run_kind 匹配
/// 点精确移入（不在共通前奏复制）；五次 AutoRevision 上限与每轮新 split
/// handle 原样保留。
pub(crate) async fn drive_legacy_outline_arm<'e>(
    drive: ArmDrive<'e, '_>,
    outline_revision_feedback: Option<String>,
    is_outline_revision_run: bool,
    rebuild_context: Option<Box<ResolvedPlanningContext>>,
) -> ArmExit<'e> {
    let ArmDrive {
        engine,
        provider: provider_for_run,
        command_rx,
        manager: manager_for_task,
        run_context: run_context_clone,
        outbound_tx: outbound_tx_for_task,
        run_label,
        run_token,
        provider_drive_guard,
        run_cancel,
        ..
    } = drive;
    let mut engine = engine;
    // P3：借现 Box（as_deref），不再裸 clone 整份 ResolvedPlanningContext。
    let rebuilt = rebuild_context.as_deref();
    let lifecycle_for_run = LifecycleStore::new(run_context_clone.app_paths.clone());
    let app_paths_for_run = run_context_clone.app_paths.clone();
    let session_record_for_run = run_context_clone.session_record.clone();
    let mut command_rx = command_rx;

    let request = match build_work_item_plan_generate_request(&engine, &lifecycle_for_run)
        .map_err(|e| format!("build request failed: {e}"))
    {
        Ok(r) => r,
        Err(message) => {
            engine.mark_active_run_finished(run_label);
            drop(engine);
            let err = WsOutMessage::Error { message };
            let _ = send_json_outbound(outbound_tx_for_task, &err).await;
            return ArmExit::Stop;
        }
    };

    let repository = match workspace_repository_for_session(
        &app_paths_for_run,
        &lifecycle_for_run,
        &session_record_for_run,
    ) {
        Ok(r) => r,
        Err(error) => {
            engine.mark_active_run_finished(run_label);
            drop(engine);
            let err = WsOutMessage::Error {
                message: format!("load repository failed: {error}"),
            };
            let _ = send_json_outbound(outbound_tx_for_task, &err).await;
            return ArmExit::Stop;
        }
    };

    let issue = match IssueStore::new(app_paths_for_run.clone()).get(
        &session_record_for_run.project_id,
        &session_record_for_run.issue_id,
    ) {
        Ok(i) => i,
        Err(error) => {
            engine.mark_active_run_finished(run_label);
            drop(engine);
            let err = WsOutMessage::Error {
                message: format!("load issue failed: {error}"),
            };
            let _ = send_json_outbound(outbound_tx_for_task, &err).await;
            return ArmExit::Stop;
        }
    };

    let author_provider = engine.session().author_provider.clone();
    let plan_launch = match resolve_plan_author_launch(&engine) {
        Ok(launch) => launch,
        Err(error) => {
            engine.mark_active_run_finished(run_label);
            drop(engine);
            let err = WsOutMessage::Error {
                message: format!("logical plan launch failed: {error}"),
            };
            let _ = send_json_outbound(outbound_tx_for_task, &err).await;
            return ArmExit::Stop;
        }
    };
    let routing_context = plan_launch.routing_context();
    // B2b2（Task 5.3，tasks.md 5.3 / 设计 §5.3）：LC 显式 fresh
    // recovery run——dispatch（claim 受理）已把 recovery 推进
    // FreshRunning 并 spawn 本 run。outline 反馈族三 tag
    //（author/revision/rebuild）从 typed 拒接管为真实 full
    // fresh：prompt 自足（旧 outline 全文/feedback/review/完整
    // story/design/repository 材料），旧 provider 会话不回捞
    //（resume=None，全新节点）；正常 resume 的增量返修与首发
    // full 出生路径保持不变（FreshRunning 之外的记录零影响）。
    let fresh_outline_source_context = lifecycle_for_run
        .get_workspace_session(&session_record_for_run.id)
        .ok()
        .and_then(|record| record.lc_resume_recovery)
        .filter(|recovery| {
            recovery.state == crate::product::models::LcResumeRecoveryState::FreshRunning
        })
        .map(|recovery| recovery.run_context)
        .filter(|context| {
            matches!(
                context,
                crate::product::models::LcRecoveryRunContext::LegacyOutlineAuthor { .. }
                    | crate::product::models::LcRecoveryRunContext::LegacyOutlineRevision { .. }
                    | crate::product::models::LcRecoveryRunContext::LegacyOutlineRebuild { .. }
            )
        });

    // 首次生成：使用完整 prompt + context_resolutions；
    // review/AutoRevision：使用同一会话增量返修 prompt，不重复完整上下文；
    // 显式 fresh recovery：full fresh 返修 prompt（自足，不依赖旧会话）。
    let mut invocation = if let Some(source_context) = fresh_outline_source_context.as_ref() {
        match WorkItemSplitEngine::build_outline_fresh_revision_invocation(
            &request,
            &lifecycle_for_run,
            &issue,
            &repository,
            author_provider,
            source_context,
            &routing_context,
        ) {
            Ok(invocation) => invocation,
            Err(error) => {
                engine.mark_active_run_finished(run_label);
                let err = WsOutMessage::Error {
                    message: format!(
                        "outline fresh revision invocation failed: {}",
                        error.message
                    ),
                };
                let _ = send_json_outbound(outbound_tx_for_task, &err).await;
                return ArmExit::Stop;
            }
        }
    } else if is_outline_revision_run {
        let feedback = outline_revision_feedback
            .as_deref()
            .unwrap_or("用户在 Author Confirm 阶段要求返修当前 Outline");
        match WorkItemSplitEngine::build_outline_revision_invocation(
            &request,
            &issue,
            &repository,
            author_provider,
            feedback,
            &routing_context,
        ) {
            Ok(invocation) => invocation,
            Err(error) => {
                engine.mark_active_run_finished(run_label);
                let err = WsOutMessage::Error {
                    message: format!("outline revision invocation failed: {}", error.message),
                };
                let _ = send_json_outbound(outbound_tx_for_task, &err).await;
                return ArmExit::Stop;
            }
        }
    } else {
        let context_resolutions = match load_work_item_plan_outline_context_resolutions(
            &app_paths_for_run,
            &session_record_for_run,
            &request,
            &lifecycle_for_run,
            &issue,
        ) {
            Ok(resolutions) => resolutions,
            Err(message) => {
                engine.mark_active_run_finished(run_label);
                drop(engine);
                let err = WsOutMessage::Error { message };
                let _ = send_json_outbound(outbound_tx_for_task, &err).await;
                return ArmExit::Stop;
            }
        };
        match WorkItemSplitEngine::build_outline_invocation(
            &request,
            &lifecycle_for_run,
            &issue,
            &repository,
            author_provider,
            &context_resolutions,
            &routing_context,
        ) {
            Ok(invocation) => invocation,
            Err(error) => {
                engine.mark_active_run_finished(run_label);
                let err = WsOutMessage::Error {
                    message: format!("split generate failed: {}", error.message),
                };
                let _ = send_json_outbound(outbound_tx_for_task, &err).await;
                return ArmExit::Stop;
            }
        }
    };

    // B3：StaleContext 重建 —— 用 rebuilt planning context 构造新 run：
    // 聚合根 cwd（provider_context_root）作为 worktree，注入
    // inventory/effective members 到 prompt（不沿用旧会话内容）。
    if let Some(rebuilt) = rebuilt {
        invocation.worktree_path = rebuilt.cwd.to_string_lossy().to_string();
        invocation.prompt.push_str(
            &crate::product::workspace_engine::aggregate_work_item_target_scope_prompt(
                &rebuilt.inventory_injection.rendered,
                &rebuilt.snapshot.effective_member_ids,
            ),
        );
    }

    let node_id = if rebuilt.is_some() || fresh_outline_source_context.is_some() {
        // B3/5.3：重建与显式 fresh run 都不沿用中断会话的
        // OutlineRun 节点（漂移作废，fresh_node_id 由新节点
        // 落 conversation 后回绑），新建节点。
        engine.begin_work_item_plan_outline_run().await
    } else if engine.active_node_type()
        == Some(crate::web::workspace_ws_types::TimelineNodeType::WorkItemPlanOutlineRun)
    {
        match engine.active_timeline_node_id() {
            Some(node_id) => node_id,
            None => {
                engine.mark_active_run_finished(run_label);
                drop(engine);
                let err = WsOutMessage::Error {
                    message: "work item plan outline run node unavailable".to_string(),
                };
                let _ = send_json_outbound(outbound_tx_for_task, &err).await;
                return ArmExit::Stop;
            }
        }
    } else {
        engine.begin_work_item_plan_outline_run().await
    };
    engine
        .emit_provider_prompt_event(
            &node_id,
            invocation.prompt.clone(),
            if fresh_outline_source_context.is_some() {
                "发送给 WorkItemPlan provider 的显式 fresh 完整返修提示词"
            } else if is_outline_revision_run {
                "发送给 WorkItemPlan provider 的增量返修提示词"
            } else {
                "发送给 WorkItemPlan provider 的完整提示词"
            },
            Some(invocation.author_provider.clone()),
        )
        .await;
    // B3 round3/5.3：StaleContext 重建 run 与显式 fresh recovery run 的
    // provider **全新启动** —— 不携带 provider_resume_session_id
    //（不读取旧 Author conversation，避免以 --resume 复用旧
    // provider 原生会话；fresh 的旧 native id 已被 superseded 记录
    // 作废，不回捞）；legacy 正常 resume（SameContext/传统单仓）保持
    // 现有行为（携带会话 id）。
    let provider_input = if rebuilt.is_some() || fresh_outline_source_context.is_some() {
        engine.build_work_item_plan_streaming_input_fresh(
            invocation.provider_type.clone(),
            invocation.prompt.clone(),
            invocation.worktree_path.clone(),
            invocation.author_provider.clone(),
            PlanAuthorOutputContract::Structured,
        )
    } else {
        engine.build_work_item_plan_streaming_input(
            invocation.provider_type.clone(),
            invocation.prompt.clone(),
            invocation.worktree_path.clone(),
            invocation.author_provider.clone(),
            PlanAuthorOutputContract::Structured,
        )
    };
    // REQ-PIB-02（T2.3）：基线不可解析 → 终止本轮 run（可观测错误出站）。
    let provider_input = match provider_input {
        Ok(provider_input) => provider_input,
        Err(message) => {
            engine.mark_active_run_finished(run_label);
            drop(engine);
            let err = WsOutMessage::Error { message };
            let _ = send_json_outbound(outbound_tx_for_task, &err).await;
            return ArmExit::Stop;
        }
    };
    let provider_input = engine.attach_tool_policy_audit(provider_input);
    // Task 1b 段②生产臂:初次 outline run 先 begin split run 身份。
    let plan_split_run = match begin_plan_split_run_if_logical(
        &plan_launch,
        &lifecycle_for_run,
        &session_record_for_run.project_id,
        &session_record_for_run.issue_id,
        &invocation.author_provider,
        &engine.session().session_id,
    ) {
        Ok(run) => run,
        Err(message) => {
            engine.mark_active_run_finished(run_label);
            drop(engine);
            let err = WsOutMessage::Error { message };
            let _ = send_json_outbound(outbound_tx_for_task, &err).await;
            return ArmExit::Stop;
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
    // 新 BLOCKER 修复：rebuilt snapshot 仅在 provider 成功启动后 commit。
    // provider 启动失败不落盘 —— 重连仍判 StaleContext（避免再次 TOCTOU）。
    if let Some(rebuilt) = rebuilt {
        commit_rebuilt_snapshot_after_provider_start(
            &app_paths_for_run,
            rebuilt,
            &provider_session,
        );
    }
    let full_output = match engine
        .drive_work_item_plan_provider_session_to_output(
            provider_session,
            &mut command_rx,
            node_id,
            invocation.author_provider.clone(),
        )
        .await
    {
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
    close_plan_split_run(plan_split_run.as_ref(), &invocation.prompt, &full_output);
    let mut outcome =
        match complete_work_item_plan_outline_author_from_output(&mut engine, &full_output).await {
            Ok(o) => o,
            Err(message) => {
                engine
                    .finish_active_run_with_failed_node(message.clone())
                    .await;
                drop(engine);
                let err = WsOutMessage::Error { message };
                let _ = send_json_outbound(outbound_tx_for_task, &err).await;
                return ArmExit::Stop;
            }
        };

    // 本地循环处理 AutoRevision：基于同一会话增量返修，不重复完整上下文。
    // 循环上限由 `complete_work_item_plan_author` 内部的
    // `work_item_plan_author_retry_count` 控制（达到 3 次后返回 HumanConfirm）；
    // 下方的 `revision_iterations` 作为硬兜底。
    let mut revision_iterations = 0;
    loop {
        match outcome {
            WorkItemPlanAuthorOutcome::AuthorConfirm => {
                engine.mark_active_run_finished(run_label);
                drop(engine);
                drop(provider_drive_guard.take());
                manager_for_task.finish_run(run_token).await;
                return ArmExit::Stop;
            }
            WorkItemPlanAuthorOutcome::HumanConfirm { reason: _ } => {
                engine.mark_active_run_finished(run_label);
                drop(engine);
                drop(provider_drive_guard.take());
                manager_for_task.finish_run(run_token).await;
                return ArmExit::Stop;
            }
            WorkItemPlanAuthorOutcome::AutoRevision { findings } => {
                revision_iterations += 1;
                if revision_iterations > 5 {
                    engine.mark_active_run_finished(run_label);
                    drop(engine);
                    let err = WsOutMessage::Error {
                        message: "work item plan author revision exceeded hard limit".to_string(),
                    };
                    let _ = send_json_outbound(outbound_tx_for_task, &err).await;
                    return ArmExit::Stop;
                }

                let feedback = combine_outline_auto_retry_feedback(
                    outline_revision_feedback.as_deref(),
                    &findings,
                );
                let retry_of_node_id = engine
                    .active_timeline_node_id()
                    .unwrap_or_else(|| "timeline_node_unknown".to_string());
                let retry_error = work_item_plan_retry_error(&findings);
                let author_provider = engine.session().author_provider.clone();
                let plan_launch = match resolve_plan_author_launch(&engine) {
                    Ok(launch) => launch,
                    Err(error) => {
                        engine.mark_active_run_finished(run_label);
                        drop(engine);
                        let err = WsOutMessage::Error {
                            message: format!("logical plan launch failed: {error}"),
                        };
                        let _ = send_json_outbound(outbound_tx_for_task, &err).await;
                        return ArmExit::Stop;
                    }
                };
                let routing_context = plan_launch.routing_context();
                invocation = match WorkItemSplitEngine::build_outline_revision_invocation(
                    &request,
                    &issue,
                    &repository,
                    author_provider,
                    &feedback,
                    &routing_context,
                ) {
                    Ok(invocation) => invocation,
                    Err(error) => {
                        engine.mark_active_run_finished(run_label);
                        drop(engine);
                        let err = WsOutMessage::Error {
                            message: format!(
                                "outline revision invocation failed: {}",
                                error.message
                            ),
                        };
                        let _ = send_json_outbound(outbound_tx_for_task, &err).await;
                        return ArmExit::Stop;
                    }
                };
                let node_id = engine
                    .begin_work_item_plan_outline_auto_retry_run(
                        retry_of_node_id,
                        revision_iterations + 1,
                        retry_error.code.clone(),
                        retry_error,
                    )
                    .await;
                engine
                    .emit_provider_prompt_event(
                        &node_id,
                        invocation.prompt.clone(),
                        "发送给 WorkItemPlan provider 的增量返修提示词",
                        Some(invocation.author_provider.clone()),
                    )
                    .await;
                let provider_input = engine.build_work_item_plan_streaming_input(
                    invocation.provider_type.clone(),
                    invocation.prompt.clone(),
                    invocation.worktree_path.clone(),
                    invocation.author_provider.clone(),
                    PlanAuthorOutputContract::Structured,
                );
                // REQ-PIB-02（T2.3）：基线不可解析 → 终止本轮 run。
                let provider_input = match provider_input {
                    Ok(provider_input) => provider_input,
                    Err(message) => {
                        engine.mark_active_run_finished(run_label);
                        drop(engine);
                        let err = WsOutMessage::Error { message };
                        let _ = send_json_outbound(outbound_tx_for_task, &err).await;
                        return ArmExit::Stop;
                    }
                };
                let provider_input = engine.attach_tool_policy_audit(provider_input);
                // Task 1b 段②生产臂:AutoRevision retry 每轮 begin
                // 新 handle(retry 不复用旧身份)。
                let plan_split_run = match begin_plan_split_run_if_logical(
                    &plan_launch,
                    &lifecycle_for_run,
                    &session_record_for_run.project_id,
                    &session_record_for_run.issue_id,
                    &invocation.author_provider,
                    &engine.session().session_id,
                ) {
                    Ok(run) => run,
                    Err(message) => {
                        engine.mark_active_run_finished(run_label);
                        drop(engine);
                        let err = WsOutMessage::Error { message };
                        let _ = send_json_outbound(outbound_tx_for_task, &err).await;
                        return ArmExit::Stop;
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
                        invocation.author_provider.clone(),
                    )
                    .await
                {
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
                close_plan_split_run(plan_split_run.as_ref(), &invocation.prompt, &full_output);
                outcome = match complete_work_item_plan_outline_author_from_output(
                    &mut engine,
                    &full_output,
                )
                .await
                {
                    Ok(o) => o,
                    Err(message) => {
                        engine
                            .finish_active_run_with_failed_node(message.clone())
                            .await;
                        drop(engine);
                        let err = WsOutMessage::Error { message };
                        let _ = send_json_outbound(outbound_tx_for_task, &err).await;
                        return ArmExit::Stop;
                    }
                };
            }
        }
    }
}
