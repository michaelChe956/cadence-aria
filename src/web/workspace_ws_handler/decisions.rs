use super::*;
use crate::product::workspace_engine::WorkspaceConfirmOutcome;
use crate::product::workspace_engine::conversational_gate::ConfirmationTrustWaiting;
use crate::product::workspace_engine::{HumanGateCloseDecision, HumanGateCloseOutcome};

/// T5.1（REQ-REG-15/D4）：确认 trust typed 停等的 WS/REST 同源投影——稳定
/// code `workspace_confirmation_trust_waiting`。中文 message 展示原因、门未
/// 关闭、人工处置与「完成后请重新选择该门并确认；失败不自动重试」（不把
/// 指导只藏 context）；context 携带已知身份与 retry_action/manual_guidance/
/// retryable:true（Home），Context 变体只返回已知身份（不编造 provider/
/// root/key、不给错误根 trust 指引）。WS ProtocolError 与 REST ApiError 共用
/// 本投影，reason_code 原样保留（不靠 message 前缀识别）。
pub(crate) const WORKSPACE_CONFIRMATION_TRUST_WAITING_CODE: &str =
    "workspace_confirmation_trust_waiting";

pub(crate) struct ConfirmationTrustWaitingProjection {
    pub(crate) code: &'static str,
    pub(crate) message: String,
    pub(crate) context: serde_json::Value,
}

pub(crate) fn confirmation_trust_waiting_projection(
    waiting: &ConfirmationTrustWaiting,
) -> ConfirmationTrustWaitingProjection {
    match waiting {
        ConfirmationTrustWaiting::Home {
            session_id,
            project_id,
            lc_id,
            waiting,
        } => {
            let provider = crate::product::logical_codebase::provider_trust::provider_wire_label(
                &waiting.provider,
            );
            ConfirmationTrustWaitingProjection {
                code: WORKSPACE_CONFIRMATION_TRUST_WAITING_CODE,
                message: format!(
                    "确认 trust 未就绪（provider {provider}，root {}）：{}。人工处置：{}。{}；\
                     修复前确认门保持未关闭，完成后请重新选择该门并确认；失败不自动重试。",
                    waiting.canonical_root.display(),
                    waiting.message,
                    waiting.manual_guidance,
                    waiting.retry_action,
                ),
                context: serde_json::json!({
                    "session_id": session_id,
                    "project_id": project_id,
                    "lc_id": lc_id,
                    "provider": provider,
                    "canonical_root": waiting.canonical_root.display().to_string(),
                    "trust_key": waiting.trust_key,
                    "reason_code": waiting.reason_code,
                    "retry_action": waiting.retry_action,
                    "manual_guidance": waiting.manual_guidance,
                    // 指引卡简化（owner 2026-10-12 直批）：结构化命令段
                    //（entry 缺失才有值），消费方直渲命令窗，不解析平铺串。
                    "command_markdown": waiting.command_markdown,
                    "retryable": true,
                }),
            }
        }
        ConfirmationTrustWaiting::Context {
            session_id,
            project_id,
            issue_id,
            issue_lc_id,
            frozen_root,
            reason_code,
            message,
        } => ConfirmationTrustWaitingProjection {
            code: WORKSPACE_CONFIRMATION_TRUST_WAITING_CODE,
            message: format!(
                "确认 trust 上下文停等（reason_code={reason_code}）：{message} \
                 修复前确认门保持未关闭，完成后请重新选择该门并确认；失败不自动重试。"
            ),
            context: serde_json::json!({
                "session_id": session_id,
                "project_id": project_id,
                "issue_id": issue_id,
                "reason_code": reason_code,
                "retryable": true,
                "issue_lc_id": issue_lc_id,
                "frozen_root": frozen_root.as_ref().map(|root| root.display().to_string()),
            }),
        },
    }
}

pub(crate) async fn handle_plan_amendment_confirmation_from_handler(
    app_state: WebAppState,
    engine: Arc<Mutex<WorkspaceEngine>>,
    manager: Arc<crate::web::workspace_session::WorkspaceSessionManager>,
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
            let mut state = engine.lock().await.build_session_state();
            // 对外出口统一以 durable enrollment 覆盖 automation 归属
            //（裸帧携带引擎缺省 client/off 会让前端 automationManaged
            // 误判、重现接管死路，见真实链 2026-10-24）。
            manager.project_automation_ownership(&mut state);
            // REQ-HGC-04：修订确认/取消后的补帧同样携带「继续自动化」入口事实。
            manager.project_plan_continue_available(&mut state);
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
    manager: Arc<crate::web::workspace_session::WorkspaceSessionManager>,
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
            let mut state = engine.lock().await.build_session_state();
            // 对外出口统一以 durable enrollment 覆盖 automation 归属
            //（裸帧携带引擎缺省 client/off 会让前端 automationManaged
            // 误判、重现接管死路，见真实链 2026-10-24）。
            manager.project_automation_ownership(&mut state);
            // REQ-HGC-04：修订确认/取消后的补帧同样携带「继续自动化」入口事实。
            manager.project_plan_continue_available(&mut state);
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
    match outcome {
        // T5.1（REQ-REG-15/D4）：非 SC 确认 trust 未 Ready 的 typed 停等——
        // 与 SC termination 臂同源投影为稳定 code 的 ProtocolError（过渡期
        // 曾静默吞掉 Ok(TrustWaiting)，客户端零回执）；门保持开态、零定稿
        // 副作用（引擎面 4.1 锁定），不 abort、不落 generic Error。
        Ok(WorkspaceConfirmOutcome::TrustWaiting { waiting }) => {
            let projection = confirmation_trust_waiting_projection(&waiting);
            let err = WsOutMessage::ProtocolError {
                code: projection.code.to_string(),
                message: projection.message,
                context: Some(projection.context),
            };
            let _ = send_json_outbound(&outbound_tx, &err).await;
        }
        Ok(_) => {}
        Err(message) => {
            let err = WsOutMessage::ProtocolError {
                code: "INVALID_HUMAN_CONFIRM_ACTION".to_string(),
                message,
                context: None,
            };
            let _ = send_json_outbound(&outbound_tx, &err).await;
        }
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
// 关门效果按值返回 WS/REST 投影;findings 臂携带结构化上下文,Box 会波及两处消费。
#[allow(clippy::large_enum_variant)]
pub(crate) enum HumanGateTerminationEffect {
    Confirmed,
    Abandoned,
    AlreadyClosed {
        /// 关门时的 session 状态：保留供诊断/后续响应扩展；当前无读取点。
        #[allow(dead_code)]
        status: crate::product::models::WorkspaceSessionStatus,
    },
    Busy {
        turn_id: String,
    },
    CompileFailed {
        message: String,
        findings_context: Option<serde_json::Value>,
    },
    /// T5.1（REQ-REG-15/D4）：确认 trust 未 Ready 的 typed 停等——门保持
    /// 开态、零 durable 副作用；序列化方（WS ProtocolError / REST 非成功
    /// ApiError）共用 `confirmation_trust_waiting_projection` 同源投影。
    TrustWaiting {
        waiting: ConfirmationTrustWaiting,
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
        Ok(HumanGateCloseOutcome::Busy { turn_id }) => HumanGateTerminationEffect::Busy { turn_id },
        Ok(HumanGateCloseOutcome::Confirmed) => HumanGateTerminationEffect::Confirmed,
        Ok(HumanGateCloseOutcome::Abandoned) => HumanGateTerminationEffect::Abandoned,
        // F7 项 2：先到者已关门，迟到 confirm 幂等 no-op；可见提示事件由 engine
        // 发出（HUMAN_GATE_ALREADY_CLOSED），序列化方不重复发送。
        Ok(HumanGateCloseOutcome::AlreadyClosed { status }) => {
            HumanGateTerminationEffect::AlreadyClosed { status }
        }
        // T5.1（REQ-REG-15/D4）：确认 trust 未 Ready 的 typed 停等——替换
        // T4.1 过渡臂（EngineError）：保留 typed waiting
        // 交由序列化方同源投影为稳定 code 的 ProtocolError/ApiError（不
        // abort、不 compile failure、不 generic EngineError）；门保持开态、
        // 零 durable 副作用（引擎面 4.1 锁定）。
        Ok(HumanGateCloseOutcome::TrustWaiting { waiting }) => {
            HumanGateTerminationEffect::TrustWaiting { waiting }
        }
        Err(message) => {
            // F7 项 1 → SG3b：confirm 后 compile 失败的 typed
            // PlanningCompileFailure（failure_id/approval/source context）
            // 经 out.rs 纯投影上抛 WS ProtocolError.context；其余错误维持
            // 既有 Error 通道。
            let failure = if is_confirm {
                engine
                    .lock()
                    .await
                    .last_human_gate_close_compile_failure()
                    .cloned()
            } else {
                None
            };
            if let Some(failure) = failure {
                HumanGateTerminationEffect::CompileFailed {
                    message,
                    findings_context: Some(
                        serde_json::to_value(failure.wire_presentation()).unwrap_or_default(),
                    ),
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
        HumanGateTerminationEffect::TrustWaiting { waiting } => {
            // T5.1：稳定 code 的 ProtocolError（同源 message/context），门未
            // 关闭、无 close 广播——客户端据指导修复后重新确认。
            let projection = confirmation_trust_waiting_projection(&waiting);
            Some(WsOutMessage::ProtocolError {
                code: projection.code.to_string(),
                message: projection.message,
                context: Some(projection.context),
            })
        }
        HumanGateTerminationEffect::CompileFailed {
            message,
            findings_context,
        } => Some(WsOutMessage::ProtocolError {
            code: "SINGLE_CANDIDATE_APPROVAL_COMPILE_FAILED".to_string(),
            message,
            context: findings_context,
        }),
        HumanGateTerminationEffect::EngineError(message) => Some(WsOutMessage::Error { message }),
    };
    if let Some(message) = message {
        let _ = send_json_outbound(&outbound_tx, &message).await;
    }
}

mod advance;
mod inbound;
mod register_trust_entry;
pub(crate) use advance::handle_advance_from_handler;
pub(crate) use inbound::{WorkspaceInboundContext, handle_workspace_inbound_message};
#[cfg(test)]
pub(crate) use inbound::{
    finish_interrupted_recovery_spawn_error, provider_run_kind_for_interrupted_recovery,
};
pub(crate) use register_trust_entry::handle_register_trust_entry_from_handler;
