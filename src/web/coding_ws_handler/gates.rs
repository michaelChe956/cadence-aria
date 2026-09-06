use chrono::{Duration as ChronoDuration, Utc};
use tokio::sync::mpsc;
use tokio::time::{Duration, Instant};

use crate::product::coding_attempt_store::CodingAttemptStore;
use crate::product::coding_models::{
    CodingExecutionAttempt, CodingExecutionStage, CodingGateAction, CodingGateActionType,
    CodingGateKind, CodingGateRequired as CodingGateRequiredModel, CodingStageGateState,
    CodingStageGateStatus,
};
use crate::product::coding_workspace_engine::{CodingWorkspaceEngine, CodingWorkspaceEngineError};
use crate::product::coding_workspace_runner::{
    CodingRunnerCommand, coding_provider_role_for_stage,
};
use crate::web::workspace_ws_types::{
    WsExecutionEvent, WsExecutionEventKind, WsExecutionEventStatus,
};

use super::{CodingWsOutMessage, build_coding_session_state, update_provider_selection};

const STAGE_GATE_COUNTDOWN_SECONDS: u64 = 5;

/// 「过期自动继续」观测标记（3.6 F7-B 项 2）：倒计时耗尽自动放行时补发一条
/// 独立可观测事件，含 gate id/阶段/倒计时耗尽原因，供 driver 与报告按 status
/// 分列 confirmed vs expired_continue（3.5 台账遗留观察：两路径下游无差别、
/// 审计口径混同）。
///
/// 载体刻意选用 `coding_execution_event`（title=`stage_gate_auto_continue`）而
/// 非新增顶层消息类型：driver（abdddb41 起 stage_gate 豁免）对 coding_execution_event
/// 不解析 payload（case break），不会触发 unknown_ws_message 停机；倒计时与
/// 自动继续控制流零变化（发送失败不阻断，后续 session_state 快照行为不变）。
fn stage_gate_auto_continue_event(gate: &CodingStageGateState) -> WsExecutionEvent {
    WsExecutionEvent {
        event_id: format!("stage_gate_auto_continue_{}", gate.gate_id),
        node_id: None,
        agent: None,
        kind: WsExecutionEventKind::Turn,
        status: WsExecutionEventStatus::Completed,
        title: "stage_gate_auto_continue".to_string(),
        detail: Some(format!(
            "stage gate {} for {:?} countdown exhausted after {}s; auto-continue",
            gate.gate_id, gate.stage, STAGE_GATE_COUNTDOWN_SECONDS
        )),
        command: None,
        cwd: None,
        output: None,
        exit_code: None,
    }
}

pub(crate) async fn await_stage_gate(
    command_rx: &mut mpsc::Receiver<CodingRunnerCommand>,
    coding_store: &CodingAttemptStore,
    engine: &CodingWorkspaceEngine,
    event_tx: &mpsc::Sender<CodingWsOutMessage>,
    attempt: &CodingExecutionAttempt,
    stage: CodingExecutionStage,
) -> Result<Option<CodingExecutionAttempt>, CodingWorkspaceEngineError> {
    let Some(role) = coding_provider_role_for_stage(&stage) else {
        return Ok(Some(attempt.clone()));
    };
    let mut current =
        coding_store.get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
    let provider_snapshot = coding_store.get_role_provider_config_snapshot(
        &current.project_id,
        &current.issue_id,
        &current.id,
    )?;
    let mut deadline = Instant::now() + Duration::from_secs(STAGE_GATE_COUNTDOWN_SECONDS);
    let expires_at = stage_gate_expires_at();
    let mut gate = coding_store.create_stage_gate(
        &current,
        stage.clone(),
        role,
        expires_at,
        provider_snapshot,
    )?;
    emit_stage_gate(engine, event_tx, coding_store, &current, &gate).await?;

    loop {
        tokio::select! {
            _ = engine.cancellation_token().cancelled() => {
                return Err(CodingWorkspaceEngineError::Aborted);
            }
            _ = tokio::time::sleep_until(deadline) => {
                let _ = coding_store.update_stage_gate_status(
                    &current.project_id,
                    &current.issue_id,
                    &current.id,
                    &gate.gate_id,
                    CodingStageGateStatus::Expired,
                )?;
                // 观测标记（仅可观测，不改控制流）：失败不阻断，后续快照行为不变
                let _ = send_event(
                    engine,
                    event_tx,
                    CodingWsOutMessage::CodingExecutionEvent {
                        event: stage_gate_auto_continue_event(&gate),
                    },
                )
                .await;
                let snapshot = build_coding_session_state(coding_store, current.clone())?;
                send_event(engine, event_tx, snapshot).await?;
                return Ok(Some(current));
            }
            command = command_rx.recv() => {
                let Some(command) = command else {
                    tokio::select! {
                        _ = tokio::time::sleep_until(deadline) => {}
                        _ = engine.cancellation_token().cancelled() => {
                            return Err(CodingWorkspaceEngineError::Aborted);
                        }
                    }
                    let _ = coding_store.update_stage_gate_status(
                        &current.project_id,
                        &current.issue_id,
                        &current.id,
                        &gate.gate_id,
                        CodingStageGateStatus::Expired,
                    )?;
                    // 观测标记（仅可观测，不改控制流）：command 通道关闭后仍按期
                    // 过期自动继续，同样发标记保持 confirmed vs expired 分列
                    let _ = send_event(
                        engine,
                        event_tx,
                        CodingWsOutMessage::CodingExecutionEvent {
                            event: stage_gate_auto_continue_event(&gate),
                        },
                    )
                    .await;
                    let snapshot = build_coding_session_state(coding_store, current.clone())?;
                    send_event(engine, event_tx, snapshot).await?;
                    return Ok(Some(current));
                };
                match command {
                    CodingRunnerCommand::StageGateConfirm { stage: confirm_stage }
                        if confirm_stage == stage =>
                    {
                        let _ = coding_store.update_stage_gate_status(
                            &current.project_id,
                            &current.issue_id,
                            &current.id,
                            &gate.gate_id,
                            CodingStageGateStatus::Confirmed,
                        )?;
                        let snapshot = build_coding_session_state(coding_store, current.clone())?;
                        send_event(engine, event_tx, snapshot).await?;
                        return Ok(Some(current));
                    }
                    CodingRunnerCommand::StageGateConfirm { .. } => {
                        send_event(engine, event_tx, CodingWsOutMessage::CodingProtocolError {
                                code: "coding_stage_gate_mismatch".to_string(),
                                message: "stage gate confirm did not match the open stage gate".to_string(),
                            })
                            .await?;
                    }
                    CodingRunnerCommand::PermissionResponse { .. }
                    | CodingRunnerCommand::ChoiceResponse { .. }
                    | CodingRunnerCommand::RetryPush => {}
                    CodingRunnerCommand::ProviderSelect { role, provider } => {
                        let (updated, changed_role, changed_provider) =
                            match update_provider_selection(
                                coding_store,
                                &current,
                                &role,
                                provider,
                            ) {
                                Ok(result) => result,
                                Err(error) => {
                                    send_event(engine, event_tx, CodingWsOutMessage::CodingProtocolError {
                                            code: "coding_provider_select_failed".to_string(),
                                            message: error.to_string(),
                                        })
                                        .await?;
                                    continue;
                                }
                            };
                        current = updated;
                        let provider_snapshot =
                            coding_store.get_role_provider_config_snapshot(
                                &current.project_id,
                                &current.issue_id,
                                &current.id,
                            )?;
                        deadline =
                            Instant::now() + Duration::from_secs(STAGE_GATE_COUNTDOWN_SECONDS);
                        gate = coding_store.refresh_stage_gate(
                            &current.project_id,
                            &current.issue_id,
                            &current.id,
                            &gate.gate_id,
                            stage_gate_expires_at(),
                            provider_snapshot,
                        )?;
                        send_event(engine, event_tx, CodingWsOutMessage::CodingProviderConfigUpdated {
                                role: changed_role,
                                provider: changed_provider,
                            })
                            .await?;
                        emit_stage_gate(engine, event_tx, coding_store, &current, &gate).await?;
                    }
                    CodingRunnerCommand::AbortAttempt => {
                        let _ = coding_store.update_stage_gate_status(
                            &current.project_id,
                            &current.issue_id,
                            &current.id,
                            &gate.gate_id,
                            CodingStageGateStatus::Cancelled,
                        )?;
                        let updated = engine
                            .handle_abort(&current.project_id, &current.issue_id, &current.id)
                            .await?;
                        crate::web::coding_ws_handler::state::emit_current_session_state(
                            event_tx,
                            coding_store,
                            &updated,
                            engine.cancellation_token(),
                        )
                        .await?;
                        return Ok(None);
                    }
                }
            }
        }
    }
}

pub(crate) async fn emit_stage_gate(
    engine: &CodingWorkspaceEngine,
    event_tx: &mpsc::Sender<CodingWsOutMessage>,
    coding_store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
    gate: &CodingStageGateState,
) -> Result<(), CodingWorkspaceEngineError> {
    send_event(
        engine,
        event_tx,
        CodingWsOutMessage::CodingGateRequired {
            gate: stage_gate_required(gate.clone()),
        },
    )
    .await?;
    let snapshot = build_coding_session_state(coding_store, attempt.clone())?;
    send_event(engine, event_tx, snapshot).await?;
    Ok(())
}

async fn send_event(
    engine: &CodingWorkspaceEngine,
    event_tx: &mpsc::Sender<CodingWsOutMessage>,
    event: CodingWsOutMessage,
) -> Result<(), CodingWorkspaceEngineError> {
    let permit = tokio::select! {
        biased;
        _ = engine.cancellation_token().cancelled() => {
            return Err(CodingWorkspaceEngineError::Aborted);
        }
        permit = event_tx.reserve() => permit,
    };
    let permit = permit.map_err(|_| {
        CodingWorkspaceEngineError::ProviderStream("coding_event_channel_closed".to_string())
    })?;
    permit.send(event);
    Ok(())
}

fn stage_gate_expires_at() -> String {
    (Utc::now() + ChronoDuration::seconds(STAGE_GATE_COUNTDOWN_SECONDS as i64)).to_rfc3339()
}

pub(crate) fn stage_gate_required(gate: CodingStageGateState) -> CodingGateRequiredModel {
    CodingGateRequiredModel {
        gate_id: gate.gate_id,
        kind: CodingGateKind::StageGate,
        title: format!("{:?} Stage Gate", gate.stage),
        description: format!(
            "Waiting to start {:?} with {} provider until {}",
            gate.stage, gate.role, gate.expires_at
        ),
        stage: Some(gate.stage),
        role: Some(gate.role),
        expires_at: Some(gate.expires_at),
        provider_snapshot: Some(gate.provider_snapshot),
        available_actions: vec![
            CodingGateAction {
                action_id: "confirm_stage".to_string(),
                label: "立即开始".to_string(),
                action_type: CodingGateActionType::ConfirmStage,
            },
            CodingGateAction {
                action_id: "abort".to_string(),
                label: "中止 Attempt".to_string(),
                action_type: CodingGateActionType::Abort,
            },
        ],
        reason_code: None,
        evidence_refs: Vec::new(),
        raw_provider_output_ref: None,
        diagnostic: None,
    }
}
