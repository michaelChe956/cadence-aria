use super::*;
use crate::product::workspace_engine::{HumanGateCloseDecision, HumanGateCloseOutcome};

pub(crate) async fn handle_plan_amendment_confirmation_from_handler(
    app_state: WebAppState,
    engine: Arc<Mutex<WorkspaceEngine>>,
    outbound_tx: mpsc::Sender<OutboundControl>,
    amendment_id: String,
) {
    let result = {
        let mut engine = engine.lock().await;
        engine
            .confirm_and_publish_plan_amendment(&amendment_id, "workspace_user")
            .await
            .map(|_| {
                engine.plan_repair_session_state().map(|snapshot| {
                    (
                        engine.session().project_id.clone(),
                        engine.session().issue_id.clone(),
                        snapshot.request.trigger_attempt_id.clone(),
                    )
                })
            })
    };
    match result {
        Ok(Some((project_id, issue_id, attempt_id))) => {
            if let Err(error) =
                activate_published_plan_amendment(&app_state, &project_id, &issue_id, &attempt_id)
                    .await
            {
                let message = WsOutMessage::ProtocolError {
                    code: "PLAN_AMENDMENT_ACTIVATION_FAILED".to_string(),
                    message: format!("{error:?}"),
                    context: Some(serde_json::json!({ "amendment_id": amendment_id })),
                };
                let _ = send_json_outbound(&outbound_tx, &message).await;
                return;
            }
            let state = engine.lock().await.build_session_state();
            let _ = send_json_outbound(&outbound_tx, &state).await;
        }
        Ok(None) => {
            let message = WsOutMessage::ProtocolError {
                code: "PLAN_AMENDMENT_CONFIRMATION_FAILED".to_string(),
                message: "published plan amendment state is missing".to_string(),
                context: Some(serde_json::json!({ "amendment_id": amendment_id })),
            };
            let _ = send_json_outbound(&outbound_tx, &message).await;
        }
        Err(error) => {
            let message = WsOutMessage::ProtocolError {
                code: "PLAN_AMENDMENT_CONFIRMATION_FAILED".to_string(),
                message: format!("{error:?}"),
                context: Some(serde_json::json!({ "amendment_id": amendment_id })),
            };
            let _ = send_json_outbound(&outbound_tx, &message).await;
        }
    }
}

pub(crate) async fn handle_plan_amendment_cancel_from_handler(
    engine: Arc<Mutex<WorkspaceEngine>>,
    outbound_tx: mpsc::Sender<OutboundControl>,
    amendment_id: String,
    reason: Option<String>,
) {
    let result = {
        let mut engine = engine.lock().await;
        engine.cancel_plan_amendment(&amendment_id, reason).await
    };
    match result {
        Ok(()) => {
            let state = engine.lock().await.build_session_state();
            let _ = send_json_outbound(&outbound_tx, &state).await;
        }
        Err(error) => {
            let message = WsOutMessage::ProtocolError {
                code: "PLAN_AMENDMENT_CANCEL_FAILED".to_string(),
                message: format!("{error:?}"),
                context: Some(serde_json::json!({ "amendment_id": amendment_id })),
            };
            let _ = send_json_outbound(&outbound_tx, &message).await;
        }
    }
}

/// 非 SC 流（story/design 等）的 `Confirm` 帧直连引擎确认（L2 退役收口：
/// 原 human-confirm 决策桥接随 `HumanConfirm` 消息族删除，approve 帧面按
/// T4 §4 登记形态保留）。
pub(crate) async fn handle_confirm_from_handler(
    run_context: ProviderRunContext,
    outbound_tx: mpsc::Sender<OutboundControl>,
) {
    let outcome = {
        let mut engine = run_context.engine.lock().await;
        engine.handle_confirm().await
    };
    if let Err(message) = outcome {
        let err = WsOutMessage::ProtocolError {
            code: "INVALID_HUMAN_CONFIRM_ACTION".to_string(),
            message,
            context: None,
        };
        let _ = send_json_outbound(&outbound_tx, &err).await;
    }
}

/// P0 1.3（REQ-WIGA-05）Task 10：SC 人工门 feedback 的引擎裁决（可返回值）
/// ——WS 序列化与 REST human-actions 共用；本层不发送任何 OutboundControl，
/// 调用方按各自通道（WS 帧 / HTTP 状态码）序列化。
pub(crate) enum HumanGateFeedbackEffect {
    TurnOpened {
        turn: crate::product::models::HumanGateTurn,
        remaining_budget: u32,
        prompt: String,
    },
    Replayed {
        turn: crate::product::models::HumanGateTurn,
    },
    Busy {
        turn_id: String,
    },
    Rejected {
        code: String,
        reason: String,
    },
    EngineError(String),
}

pub(crate) async fn apply_human_gate_feedback(
    engine: Arc<Mutex<WorkspaceEngine>>,
    input: HumanGateFeedbackInput,
) -> HumanGateFeedbackEffect {
    let outcome = {
        let mut engine = engine.lock().await;
        engine.handle_human_gate_feedback(input).await
    };
    match outcome {
        Ok(HumanGateCommandOutcome::TurnOpened {
            turn,
            remaining_budget,
            prompt,
        }) => HumanGateFeedbackEffect::TurnOpened {
            turn,
            remaining_budget,
            prompt,
        },
        Ok(HumanGateCommandOutcome::Replayed { turn }) => {
            HumanGateFeedbackEffect::Replayed { turn }
        }
        Ok(HumanGateCommandOutcome::Busy { turn_id }) => HumanGateFeedbackEffect::Busy { turn_id },
        Ok(HumanGateCommandOutcome::Rejected { code, reason }) => {
            HumanGateFeedbackEffect::Rejected { code, reason }
        }
        Err(message) => HumanGateFeedbackEffect::EngineError(message),
    }
}

/// P0 1.3（REQ-WIGA-05）Task 10：SC 人工门关门（approve/abandon）的引擎裁决
/// ——同上共用；confirm 后 compile 失败的结构化 findings 一并返回（WS 帧
/// context 与 REST 422 details 同源）。
pub(crate) enum HumanGateTerminationEffect {
    Confirmed,
    Abandoned,
    AlreadyClosed {
        status: crate::product::models::WorkspaceSessionStatus,
    },
    Busy {
        turn_id: String,
    },
    CompileFailed {
        message: String,
        findings_context: Option<serde_json::Value>,
    },
    EngineError(String),
}

pub(crate) async fn apply_human_gate_termination(
    engine: Arc<Mutex<WorkspaceEngine>>,
    decision: HumanGateCloseDecision,
) -> HumanGateTerminationEffect {
    let is_confirm = matches!(decision, HumanGateCloseDecision::Approve);
    let outcome = {
        let mut engine = engine.lock().await;
        engine.handle_human_gate_termination(decision).await
    };
    match outcome {
        Ok(HumanGateCloseOutcome::Busy { turn_id }) => {
            HumanGateTerminationEffect::Busy { turn_id }
        }
        Ok(HumanGateCloseOutcome::Confirmed) => HumanGateTerminationEffect::Confirmed,
        Ok(HumanGateCloseOutcome::Abandoned) => HumanGateTerminationEffect::Abandoned,
        // F7 项 2：先到者已关门，迟到 confirm 幂等 no-op；可见提示事件由 engine
        // 发出（HUMAN_GATE_ALREADY_CLOSED），序列化方不重复发送。
        Ok(HumanGateCloseOutcome::AlreadyClosed { status }) => {
            HumanGateTerminationEffect::AlreadyClosed { status }
        }
        Err(message) => {
            // F7 项 1（历史观察项族 12）：confirm 后 compile 失败的 validator
            // findings 经 ProtocolError.context 上抛给 WS 客户端；其余错误维持
            // 既有 Error 通道。
            let findings_context = if is_confirm {
                engine
                    .lock()
                    .await
                    .last_human_gate_close_compile_failure_context()
            } else {
                None
            };
            if findings_context.is_some() {
                HumanGateTerminationEffect::CompileFailed {
                    message,
                    findings_context,
                }
            } else {
                HumanGateTerminationEffect::EngineError(message)
            }
        }
    }
}

pub(crate) async fn handle_human_gate_feedback_from_handler(
    run_context: ProviderRunContext,
    outbound_tx: mpsc::Sender<OutboundControl>,
    input: HumanGateFeedbackInput,
) {
    let effect = apply_human_gate_feedback(run_context.engine.clone(), input).await;
    let saved_turn = match effect {
        HumanGateFeedbackEffect::TurnOpened {
            turn,
            remaining_budget,
            prompt,
        } => Some((turn, remaining_budget, prompt)),
        HumanGateFeedbackEffect::Replayed { turn } => {
            let remaining_budget = run_context
                .engine
                .lock()
                .await
                .session()
                .human_gate_snapshot
                .as_ref()
                .map_or(0, |snapshot| snapshot.manual_repairs_remaining);
            let _ = send_json_outbound(
                &outbound_tx,
                &WsOutMessage::HumanGateTurnOpen {
                    turn_id: turn.turn_id,
                    command_id: turn.command_id,
                    remaining_budget,
                },
            )
            .await;
            None
        }
        HumanGateFeedbackEffect::Busy { turn_id } => {
            let _ =
                send_json_outbound(&outbound_tx, &WsOutMessage::HumanGateBusy { turn_id }).await;
            None
        }
        HumanGateFeedbackEffect::Rejected { code, reason } => {
            let _ = send_json_outbound(
                &outbound_tx,
                &WsOutMessage::ProtocolError {
                    code,
                    message: reason,
                    context: None,
                },
            )
            .await;
            None
        }
        HumanGateFeedbackEffect::EngineError(message) => {
            let _ = send_json_outbound(&outbound_tx, &WsOutMessage::Error { message }).await;
            None
        }
    };
    if let Some((turn, remaining_budget, prompt)) = saved_turn {
        let _ = send_json_outbound(
            &outbound_tx,
            &WsOutMessage::HumanGateTurnOpen {
                turn_id: turn.turn_id.clone(),
                command_id: turn.command_id.clone(),
                remaining_budget,
            },
        )
        .await;
        let run_kind = ProviderRunKind::HumanGateScManualRevision {
            turn_id: turn.turn_id,
            prompt,
        };
        if let Err(message) =
            spawn_provider_run_from_handler(run_context, run_kind, outbound_tx.clone()).await
        {
            let _ = send_json_outbound(&outbound_tx, &WsOutMessage::Error { message }).await;
        }
    }
}

pub(crate) async fn handle_human_gate_termination_from_handler(
    run_context: ProviderRunContext,
    outbound_tx: mpsc::Sender<OutboundControl>,
    decision: HumanGateCloseDecision,
) {
    let effect = apply_human_gate_termination(run_context.engine.clone(), decision).await;
    let message = match effect {
        HumanGateTerminationEffect::Busy { turn_id } => {
            Some(WsOutMessage::HumanGateBusy { turn_id })
        }
        HumanGateTerminationEffect::Confirmed
        | HumanGateTerminationEffect::Abandoned
        | HumanGateTerminationEffect::AlreadyClosed { .. } => None,
        HumanGateTerminationEffect::CompileFailed {
            message,
            findings_context,
        } => Some(WsOutMessage::ProtocolError {
            code: "SINGLE_CANDIDATE_APPROVAL_COMPILE_FAILED".to_string(),
            message,
            context: findings_context,
        }),
        HumanGateTerminationEffect::EngineError(message) => {
            Some(WsOutMessage::Error { message })
        }
    };
    if let Some(message) = message {
        let _ = send_json_outbound(&outbound_tx, &message).await;
    }
}

mod advance;
mod inbound;
pub(crate) use advance::handle_advance_from_handler;
pub(crate) use inbound::{WorkspaceInboundContext, handle_workspace_inbound_message};
#[cfg(test)]
pub(crate) use inbound::{
    finish_interrupted_recovery_spawn_error, provider_run_kind_for_interrupted_recovery,
};
