//! P0 1.3（REQ-WIGA-05）Task 10：无 driver 的人工门与 compile recovery REST。
//!
//! 与 WS 共用同一 manager run 与事件路由（`provider_run_context` 的
//! `connection_id=None`），不挂 attachment、不抢 driver lease、不新建第二个
//! engine。`expected_gate_id` 与当前 active gate/timeline node 比对（不匹配
//! 409 零副作用）；忙态 409；引擎语义拒绝/compile 失败 422（不宣称
//! Confirmed）；受理（含幂等重放与 AlreadyClosed）200。

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use serde_json::json;

use crate::product::workspace_engine::{HumanGateCloseDecision, HumanGateFeedbackInput};
use crate::web::error::{ApiError, ApiResult};
use crate::web::handlers::workspace_choice::resolve_session_manager;
use crate::web::state::WebAppState;
use crate::web::types::{HumanActionRequest, HumanActionState, HumanActionStatus};
use crate::web::workspace_ws_handler::{
    HumanGateFeedbackEffect, HumanGateTerminationEffect, ProviderRunKind,
    apply_human_gate_feedback, apply_human_gate_termination, spawn_provider_run_from_handler,
};

pub async fn post_workspace_human_action(
    State(state): State<WebAppState>,
    Path(session_id): Path<String>,
    Json(request): Json<HumanActionRequest>,
) -> ApiResult<(StatusCode, Json<HumanActionStatus>)> {
    let manager = resolve_session_manager(&state, &session_id).await?;
    let (command_id, expected_gate_id) = command_and_gate(&request);
    if command_id.trim().is_empty() {
        return Err(ApiError::runtime(
            "invalid_command_id",
            "command_id must not be blank",
            json!({ "session_id": session_id }),
        ));
    }
    // 门身份比对在引擎调用前完成：不匹配零副作用（不误答过期门）。
    let gate_id = {
        let engine = manager.engine();
        let engine = engine.lock().await;
        engine.active_timeline_node_id()
    };
    if gate_id.as_deref() != Some(expected_gate_id.as_str()) {
        return Err(ApiError::runtime(
            "human_action_gate_mismatch",
            "expected_gate_id does not match the current active gate",
            json!({
                "expected_gate_id": expected_gate_id,
                "current_gate_id": gate_id,
                "session_id": session_id,
            }),
        ));
    }
    let gate_id = gate_id.unwrap_or_default();
    let engine = manager.engine();
    handle_request(&state, &manager, &session_id, engine, gate_id, request)
        .await
        .map(|status| {
            let code = if status.state == HumanActionState::Busy {
                StatusCode::CONFLICT
            } else {
                StatusCode::OK
            };
            (code, Json(status))
        })
}

async fn handle_request(
    state: &WebAppState,
    manager: &std::sync::Arc<crate::web::workspace_session::WorkspaceSessionManager>,
    session_id: &str,
    engine: std::sync::Arc<tokio::sync::Mutex<crate::product::workspace_engine::WorkspaceEngine>>,
    gate_id: String,
    request: HumanActionRequest,
) -> ApiResult<HumanActionStatus> {
    let (command_id, expected_gate_id) = command_and_gate(&request);
    match request {
        HumanActionRequest::Feedback { command_id, feedback, .. } => {
            let effect = apply_human_gate_feedback(
                engine,
                HumanGateFeedbackInput {
                    command_id: command_id.clone(),
                    feedback,
                },
            )
            .await;
            match effect {
                HumanGateFeedbackEffect::TurnOpened { turn, prompt, .. } => {
                    // 经唯一 manager run 与事件路由驱动修订 turn（connection_id=None：
                    // 不挂 attachment、不抢 lease）；outbound 汇入丢弃通道——会话事件
                    // 由 event router 广播给观察者，错误细节以 durable 状态为准。
                    let (sink_tx, _sink_rx) = tokio::sync::mpsc::channel(1);
                    let run_context = manager.provider_run_context(state.workspace_runs.clone());
                    spawn_provider_run_from_handler(
                        run_context,
                        ProviderRunKind::HumanGateScManualRevision {
                            turn_id: turn.turn_id,
                            prompt,
                        },
                        sink_tx,
                    )
                    .await
                    .map_err(|message| {
                        ApiError::runtime(
                            "human_action_run_spawn_failed",
                            message,
                            json!({ "command_id": command_id, "session_id": session_id }),
                        )
                    })?;
                    Ok(accepted(command_id, gate_id))
                }
                HumanGateFeedbackEffect::Replayed { .. } => Ok(accepted(command_id, gate_id)),
                HumanGateFeedbackEffect::Busy { .. } => Ok(busy(command_id, gate_id)),
                HumanGateFeedbackEffect::Rejected { code, reason } => Err(ApiError::runtime(
                    "human_action_rejected",
                    reason,
                    json!({
                        "rejection_code": code,
                        "command_id": command_id,
                        "gate_id": gate_id,
                    }),
                )),
                HumanGateFeedbackEffect::EngineError(message) => Err(ApiError::runtime(
                    "human_action_engine_error",
                    message,
                    json!({ "command_id": command_id, "session_id": session_id }),
                )),
            }
        }
        request @ (HumanActionRequest::Approve { .. } | HumanActionRequest::Abandon { .. }) => {
            let decision = match &request {
                HumanActionRequest::Approve { .. } => HumanGateCloseDecision::Approve,
                _ => HumanGateCloseDecision::Abandon,
            };
            let effect = apply_human_gate_termination(engine, decision).await;
            match effect {
                HumanGateTerminationEffect::Confirmed
                | HumanGateTerminationEffect::Abandoned
                | HumanGateTerminationEffect::AlreadyClosed { .. } => {
                    Ok(accepted(command_id, gate_id))
                }
                HumanGateTerminationEffect::Busy { .. } => Ok(busy(command_id, gate_id)),
                HumanGateTerminationEffect::CompileFailed {
                    message,
                    findings_context,
                } => Err(ApiError::runtime(
                    "single_candidate_approval_compile_failed",
                    message,
                    findings_context.unwrap_or_else(|| {
                        json!({ "command_id": command_id, "gate_id": gate_id })
                    }),
                )),
                HumanGateTerminationEffect::EngineError(message) => Err(ApiError::runtime(
                    "human_action_engine_error",
                    message,
                    json!({ "command_id": command_id, "session_id": session_id }),
                )),
            }
        }
        HumanActionRequest::CompileRecovery { command_id, action, reason, .. } => {
            let outcome = {
                let mut engine = engine.lock().await;
                engine
                    .handle_work_item_plan_compile_recovery_action(action, reason)
                    .await
            };
            match outcome {
                Ok(_) => Ok(accepted(command_id, gate_id)),
                Err(message) => Err(ApiError::runtime(
                    "invalid_compile_recovery_action",
                    message,
                    json!({ "command_id": command_id, "gate_id": gate_id }),
                )),
            }
        }
    }
}

fn command_and_gate(request: &HumanActionRequest) -> (String, String) {
    match request {
        HumanActionRequest::Approve { command_id, expected_gate_id }
        | HumanActionRequest::Abandon { command_id, expected_gate_id } => {
            (command_id.clone(), expected_gate_id.clone())
        }
        HumanActionRequest::Feedback { command_id, expected_gate_id, .. }
        | HumanActionRequest::CompileRecovery { command_id, expected_gate_id, .. } => {
            (command_id.clone(), expected_gate_id.clone())
        }
    }
}

fn accepted(command_id: String, gate_id: String) -> HumanActionStatus {
    HumanActionStatus {
        command_id,
        state: HumanActionState::Accepted,
        gate_id,
    }
}

fn busy(command_id: String, gate_id: String) -> HumanActionStatus {
    HumanActionStatus {
        command_id,
        state: HumanActionState::Busy,
        gate_id,
    }
}
