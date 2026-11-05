//! S0（任务1）机械提取：provider run 基础四臂（Author / AuthorChoiceFollowup
//! / Revision / ReviewOnly）。原 `provider_run.rs` 匿名 async 块 `:333–401`
//! 逐字迁移；本四臂原路径均正常穿过 match（无 drive 级 return），故恒返回
//! `Continue(engine)`，`Stop` 出口仅由分发点对齐 ArmExit 合同保留。臂内
//! 业务条件/收口顺序零变化。
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
/// Author 臂（原 `:333–342`，机械迁移）：单轮 user message 驱动。
pub(crate) async fn drive_author_arm<'e>(drive: ArmDrive<'e, '_>, content: String) -> ArmExit<'e> {
    let ArmDrive {
        mut engine,
        provider: provider_for_run,
        command_rx,
        run_context: run_context_clone,
        ..
    } = drive;
    engine
        .handle_user_message_from_run(
            content,
            provider_for_run.clone(),
            command_rx,
            &run_context_clone.session_record,
        )
        .await;
    ArmExit::Continue(engine)
}

/// AuthorChoiceFollowup 臂（原 `:343–352`，机械迁移）：选择追问续驱。
pub(crate) async fn drive_author_choice_followup_arm<'e>(
    drive: ArmDrive<'e, '_>,
    content: String,
) -> ArmExit<'e> {
    let ArmDrive {
        mut engine,
        provider: provider_for_run,
        command_rx,
        run_context: run_context_clone,
        ..
    } = drive;
    engine
        .handle_author_choice_followup_from_run(
            content,
            provider_for_run.clone(),
            command_rx,
            &run_context_clone.session_record,
        )
        .await;
    ArmExit::Continue(engine)
}

/// Revision 臂（原 `:353–357`，机械迁移）：receiver 仅移动一次。
pub(crate) async fn drive_revision_arm<'e>(drive: ArmDrive<'e, '_>) -> ArmExit<'e> {
    let ArmDrive {
        mut engine,
        provider: provider_for_run,
        command_rx,
        ..
    } = drive;
    engine
        .drive_revision_session(provider_for_run.clone(), command_rx)
        .await;
    ArmExit::Continue(engine)
}

/// ReviewOnly 臂（原 `:358–401`，机械迁移）：gateway/传统 review 两路与
/// Completed 回显都在臂内。
pub(crate) async fn drive_review_only_arm<'e>(drive: ArmDrive<'e, '_>) -> ArmExit<'e> {
    let ArmDrive {
        mut engine,
        provider: provider_for_run,
        command_rx,
        run_context: run_context_clone,
        outbound_tx: outbound_tx_for_task,
        ..
    } = drive;
    if engine.logical_provider_gateway().is_some() {
        engine.drive_review_session_via_gateway(command_rx).await;
    } else {
        engine
            .drive_review_session(provider_for_run.clone(), command_rx)
            .await;
    }
    // SG6b（REQ-CG-03:74 / SG6a.3⑥移交「帧时序」）：本 run 期间 grant
    // 绑定 turn 由 Running 收口为 Completed（verdict 面）时，在此发送
    // human_gate_turn_completed 回呈同代候选——迟到/重复 verdict（turn 已
    // Completed 进入本 run 前）不重复发帧。
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
                        turn.status == crate::product::models::HumanGateTurnStatus::Completed
                    })
                    .filter(|turn| turn.result_artifact_ref.is_some())
                    .map(|turn| (turn_id, turn.result_artifact_ref))
            });
        if let Some((turn_id, Some(artifact_ref))) = closed_turn {
            let _ = send_json_outbound(
                outbound_tx_for_task,
                &WsOutMessage::HumanGateTurnCompleted {
                    turn_id,
                    artifact_ref,
                },
            )
            .await;
        }
    }
    ArmExit::Continue(engine)
}
