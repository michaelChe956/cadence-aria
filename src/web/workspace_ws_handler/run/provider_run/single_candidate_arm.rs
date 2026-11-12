//! S0（任务1）机械提取：SingleCandidateAuthor 臂。原 `provider_run.rs`
//! `:407–572` 逐字迁移：恢复入口判定（FreshRunning → LcFreshContext）、
//! `run_single_candidate_author` 的六类出口收口顺序原样保留；原 drive 级
//! `return` 的出口映射 `ArmExit::Stop`（分发点不补 followups/通用 finish/
//! 错误帧），`Completed` 正常穿过 match 映射 `Continue(engine)`。
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
/// SingleCandidateAuthor 臂（原 `:407–572`，机械迁移）。
pub(crate) fn drive_single_candidate_author_arm<'e, 'd>(
    drive: ArmDrive<'e, 'd>,
) -> Pin<Box<dyn Future<Output = ArmExit<'e>> + Send + 'd>>
where
    'e: 'd,
{
    Box::pin(async move {
        let ArmDrive {
            mut engine,
            provider: provider_for_run,
            command_rx,
            manager: manager_for_task,
            run_context: run_context_clone,
            outbound_tx: outbound_tx_for_task,
            run_label,
            run_token,
            provider_drive_guard,
            run_cancel,
            // S2：ArmDrive 新增 drive 尾部字段（provider_registry/rebuild_context
            // 等）本臂不消费——`..` 忽略，不改变臂内绑定面。
            ..
        } = drive;
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
                } => single_candidate::SingleCandidateAuthorRecoveryEntry::LcFreshContext {
                    source_revision_ref: source_revision_ref.clone(),
                },
                _ => single_candidate::SingleCandidateAuthorRecoveryEntry::Ordinary,
            },
            Ok(None) => single_candidate::SingleCandidateAuthorRecoveryEntry::Ordinary,
            Err(message) => {
                engine.mark_active_run_finished(run_label);
                drop(engine);
                drop(provider_drive_guard.take());
                boxed_child(|| manager_for_task.finish_run(run_token)).await;
                let _ = send_json_outbound(outbound_tx_for_task, &WsOutMessage::Error { message })
                    .await;
                return ArmExit::Stop;
            }
        };
        // 治本（Task 14b）：重子调用装箱——SC author 的 future 状态不再内联在本臂
        // 帧里（本臂帧只留胖指针；poll 帧预留随之下沉）。
        let single_candidate_result = boxed_child(|| {
            single_candidate::run_single_candidate_author(
                &mut engine,
                provider_for_run.clone(),
                run_cancel.clone(),
                &mut command_rx,
                recovery_entry,
                run_context_clone,
            )
        })
        .await;
        // Task 4（REQ-WSC-28C，tasks 2.3）：SC run 会话尾统一收口——任一出口
        // 都先把仍在 Claimed 未回填的教学 attempt 持久化为 LaunchUnknown
        //（provider 资源注销在 adapter 的 release_after_session_tail 内，
        // 验证点在调用侧；provider_resources 不引 store）。
        engine.migrate_claimed_sc_teaching_attempts_to_unknown();
        match single_candidate_result {
            Ok(single_candidate::SingleCandidateProviderRunOutcome::Completed) => {}
            Err(single_candidate::SingleCandidateProviderRunError::AlreadyFinished) => {
                engine.mark_active_run_finished(run_label);
                drop(engine);
                drop(provider_drive_guard.take());
                boxed_child(|| manager_for_task.finish_run(run_token)).await;
                return ArmExit::Stop;
            }
            Err(single_candidate::SingleCandidateProviderRunError::Superseded) => {
                // 败者让位：键的持有者（健康在途 run 或已完结会话）继续
                // 拥有本次启动。迟到 run 静默退场——不落 failed 节点、
                // 不广播 Error、不改写 durable phase（k3 P2）。
                engine.mark_active_run_finished(run_label);
                drop(engine);
                drop(provider_drive_guard.take());
                boxed_child(|| manager_for_task.finish_run(run_token)).await;
                return ArmExit::Stop;
            }
            Err(single_candidate::SingleCandidateProviderRunError::Message(message)) => {
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
                boxed_child(|| manager_for_task.finish_run(run_token)).await;
                let _ = send_json_outbound(outbound_tx_for_task, &WsOutMessage::Error { message })
                    .await;
                return ArmExit::Stop;
            }
            Err(single_candidate::SingleCandidateProviderRunError::AdmissionWaiting {
                reason_code,
                detail,
                missing_materials,
                allowed_actions,
                manual_guidance,
                trust_waiting,
            }) => {
                // C-1：LC admission waiting——durable phase 已由运行内
                // 回落 Prepare 面，此处以无 failed 节点收尾（waiting 不是
                // 失败），并把缺失材料与允许动作作为可操作指引上浮；
                // 6.1：trust 缺失时附同源只读人工登记指导（生成场景
                // retry_action），不再是裸判别码。
                engine.mark_active_run_finished(run_label);
                drop(engine);
                drop(provider_drive_guard.take());
                boxed_child(|| manager_for_task.finish_run(run_token)).await;
                let message = format_single_candidate_admission_waiting(
                    &reason_code,
                    &detail,
                    &missing_materials,
                    &allowed_actions,
                    manual_guidance.as_deref(),
                );
                let _ = send_json_outbound(outbound_tx_for_task, &WsOutMessage::Error { message })
                    .await;
                // 指引卡简化（owner 2026-10-12 直批）：trust 缺失时
                // 紧随 error 帧发送结构化指导帧——前端「两行说明＋
                // 可复制命令窗」直渲（providers/canonical_root/
                // command_markdown 同源只读投影），不解析平铺串；
                // 平铺 error/manual_guidance 原样保留（旧客户端兼容）。
                if let Some(waiting) = trust_waiting {
                    let _ = send_json_outbound(
                        outbound_tx_for_task,
                        &WsOutMessage::trust_missing_guidance_from_waiting(&waiting),
                    )
                    .await;
                }
                return ArmExit::Stop;
            }
            Err(single_candidate::SingleCandidateProviderRunError::NativeSupersededWaiting {
                reason_code,
                superseded_native_session_id,
                recovery_id,
            }) => {
                // B2c（Task 5.5，设计 §5.3 SC 纪律①②）：SC native
                // mismatch 的 typed 等待——AdmissionWaiting 同型收口
                // （无 failed 节点，等待消息上浮）；durable 已由
                // `persist_single_candidate_author_superseded_wait`
                // 收口为 Waiting recovery，原 reservation key/phase
                // 保留（不 generic Failed、不 rearm），等待显式
                // fresh（sc_author → LcFreshContext）接管。
                engine.mark_active_run_finished(run_label);
                drop(engine);
                drop(provider_drive_guard.take());
                boxed_child(|| manager_for_task.finish_run(run_token)).await;
                let superseded_native =
                    superseded_native_session_id.as_deref().unwrap_or("unknown");
                let message = format!(
                    "single candidate author native resume superseded ({reason_code}): \
                                 the old native session {superseded_native} can no longer be resumed \
                                 (recovery {recovery_id}); no provider was started and the original \
                                 reservation/phase are preserved; 请通过显式恢复操作启动新原生会话继续修订（explicit fresh）"
                );
                let _ = send_json_outbound(outbound_tx_for_task, &WsOutMessage::Error { message })
                    .await;
                return ArmExit::Stop;
            }
        }
        ArmExit::Continue(engine)
    })
}
