use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path as AxumPath, State};
use axum::response::IntoResponse;
use futures_util::{Sink, SinkExt, StreamExt};
use tokio::sync::mpsc;

use crate::product::advance_store::AdvanceStore;
use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_attempt_store::CodingAttemptStore;
use crate::product::coding_models::{
    CodingAdmissionKind, CodingAttemptStatus, CodingExecutionAttempt, CodingExecutionStage,
};
use crate::product::coding_workspace_engine::CodingWorkspaceEngine;
use crate::product::coding_workspace_runner::CodingRunnerCommand;
use crate::product::git_workspace_service::GitWorkspaceService;
use crate::web::state::{CodingAttemptRunKey, WebAppState};

use self::abort::abort_attempt_while_draining_events;
use super::delivery_ack::fail_plan_amendment_socket_write;
use super::outbound::{
    OutboundEventReceiver, flush_queued_coding_events, send_coding_event, send_coding_json,
};
use super::{
    CodingWsInMessage, CodingWsOutMessage, build_coding_session_state,
    coding_attempt_lookup_protocol_error, confirm_open_stage_gate, context_note_chat_entry,
    pending_choice_frames, provider_selection_targets_current_running_stage,
    should_resume_runner_after_gate_response, spawn_coding_runner, spawn_coding_runner_reserved,
    update_provider_permission_mode, update_provider_selection,
};

pub(crate) mod abort;
mod admission;
pub(crate) use admission::{CodingMessageAdmission, coding_message_admission};
#[cfg(test)]
pub(crate) use admission::{
    failed_code_review_recovery_request, unfinished_failed_code_review_recovery_message_allowed,
};
mod preparation;
pub(crate) use preparation::{
    CodingMessagePreparation, CodingMessagePreparationError, prepare_coding_message,
};
mod resumption;
#[cfg(test)]
pub(crate) use preparation::{CodingRecoveryPreparationProbe, prepare_coding_message_with_probe};
pub(crate) use resumption::{ResumedAttemptRunner, ensure_runner_for_resumed_attempt};

pub async fn coding_ws(
    ws: WebSocketUpgrade,
    AxumPath(attempt_id): AxumPath<String>,
    State(state): State<WebAppState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_coding_socket(socket, None, attempt_id, state))
        .into_response()
}

pub async fn scoped_coding_ws(
    ws: WebSocketUpgrade,
    AxumPath((project_id, issue_id, attempt_id)): AxumPath<(String, String, String)>,
    State(state): State<WebAppState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| {
        handle_coding_socket(socket, Some((project_id, issue_id)), attempt_id, state)
    })
    .into_response()
}

async fn handle_coding_socket(
    socket: WebSocket,
    scope: Option<(String, String)>,
    attempt_id: String,
    state: WebAppState,
) {
    let (mut socket_tx, mut socket_rx) = socket.split();
    let app_paths = ProductAppPaths::new(state.workspace_root.join(".aria"));
    let coding_store = CodingAttemptStore::new(app_paths);
    let advance_store = AdvanceStore::new(ProductAppPaths::new(state.workspace_root.join(".aria")));
    let attempt_result = match scope.as_ref() {
        Some((project_id, issue_id)) => coding_store.get_attempt(project_id, issue_id, &attempt_id),
        None => coding_store.get_attempt_by_id(&attempt_id),
    };
    let attempt = match attempt_result {
        Ok(attempt) => attempt,
        Err(error) => {
            let (code, message) = coding_attempt_lookup_protocol_error(&error);
            let _ = send_coding_json(
                &mut socket_tx,
                &CodingWsOutMessage::CodingProtocolError { code, message },
            )
            .await;
            return;
        }
    };
    let attempt_project_id = attempt.project_id.clone();
    let attempt_issue_id = attempt.issue_id.clone();
    let attempt_id = attempt.id.clone();
    let attempt_key = CodingAttemptRunKey::from_attempt(&attempt);
    let (socket_event_tx, event_rx) = mpsc::channel(1024);
    let socket_token = state
        .coding_sockets
        .register(&attempt_key, socket_event_tx.clone());
    // F-19：发射面换用 attempt 级 hub——runner/engine 事件经 registry 路由
    // fan-out 到所有存活 socket，驱动连接断开不再终结事件流；本 socket 的
    // event_rx 消费行为（快照/flush/ack）零回归。
    let event_tx = state.coding_sockets.hub_sender(&attempt_key);
    let resumed_attempt = attempt.clone();
    if let Ok(snapshot) = build_coding_session_state(&coding_store, attempt)
        && !send_coding_json(&mut socket_tx, &snapshot).await
    {
        state.coding_sockets.remove(&attempt_key, socket_token);
        return;
    }
    // F-43：attach 初帧补发未决 provider choice（同构 F-24 workspace WS 的
    // 重订阅补发）。快照里的 `pending_choices` 只重建卡片数据，不重建客户端
    // 按帧维护的「待答卡」接线；新连接（页面刷新/重开、第二观察者）漏补即
    // 「coder 等服务端、界面等入口」的刷新死锁。已答/过期 gate 不在 Open 集内，
    // 天然不重发。
    if !send_pending_choice_frames(&mut socket_tx, &coding_store, &resumed_attempt).await {
        state.coding_sockets.remove(&attempt_key, socket_token);
        return;
    }

    let mut event_rx = OutboundEventReceiver::new(event_rx);
    let mut runner_started = false;
    let mut runner_command_tx: Option<mpsc::Sender<CodingRunnerCommand>> = None;
    // 3b 第 2 死点：半启动 attempt（Running + WorktreePrepare/Coding + 注册表
    // 无 runner）在快照后自动重启 runner（复用 StartCoding 分支的既有 spawn
    // 路径），活 runner 重连/普通 attach 判定零动作、行为不变；重启失败转
    // AwaitingManualRecovery 并发可见事件，不发静默快照。
    match ensure_runner_for_resumed_attempt(
        &state,
        &coding_store,
        &event_tx,
        &attempt_key,
        &resumed_attempt,
    )
    .await
    {
        ResumedAttemptRunner::NotNeeded => {}
        ResumedAttemptRunner::Restarted { command_tx } => {
            runner_started = true;
            runner_command_tx = Some(command_tx);
        }
        ResumedAttemptRunner::ManualRecovery { reason, detail } => {
            let _ = send_coding_json(
                &mut socket_tx,
                &CodingWsOutMessage::CodingProtocolError {
                    code: "coding_runner_restart_failed".to_string(),
                    message: format!(
                        "attempt moved to awaiting_manual_recovery: {reason}: {detail}"
                    ),
                },
            )
            .await;
            let reloaded = match scope.as_ref() {
                Some((project_id, issue_id)) => {
                    coding_store.get_attempt(project_id, issue_id, &attempt_id)
                }
                None => coding_store.get_attempt_by_id(&attempt_id),
            };
            if let Ok(updated) = reloaded
                && let Ok(snapshot) = build_coding_session_state(&coding_store, updated)
            {
                let _ = send_coding_json(&mut socket_tx, &snapshot).await;
            }
        }
    }
    'socket: loop {
        tokio::select! {
            event = event_rx.recv() => {
                let Some(event) = event else {
                    continue;
                };
                if !send_coding_event(&mut socket_tx, &event).await {
                    break;
                }
            }
            message = socket_rx.next() => {
                let Some(message) = message else {
                    break;
                };
                let Ok(message) = message else {
                    break;
                };
                match message {
                    Message::Text(text) => {
                let Ok(inbound) = serde_json::from_str::<CodingWsInMessage>(&text) else {
                    let _ = send_coding_json(
                        &mut socket_tx,
                        &CodingWsOutMessage::CodingProtocolError {
                            code: "invalid_coding_ws_message".to_string(),
                            message: "invalid coding websocket message".to_string(),
                        },
                    )
                    .await;
                    continue;
                };
                let preparation = match prepare_coding_message(
                    &coding_store,
                    &state.coding_runs,
                    &event_tx,
                    (&attempt_project_id, &attempt_issue_id, &attempt_id),
                    &inbound,
                )
                .await {
                    Ok(preparation) => preparation,
                    Err(CodingMessagePreparationError::AttemptUnavailable) => break,
                    Err(CodingMessagePreparationError::Recovery(error)) => {
                        let code = if error
                            .to_string()
                            .contains("coding_failed_review_recovery_state_changed")
                        {
                            "coding_recovery_state_changed"
                        } else {
                            "coding_failed_review_recovery_failed"
                        };
                        let _ = send_coding_json(
                            &mut socket_tx,
                            &CodingWsOutMessage::CodingProtocolError {
                                code: code.to_string(),
                                message: error.to_string(),
                            },
                        )
                        .await;
                        continue;
                    }
                };
                let (current_attempt, mutation_lease) = match preparation {
                    CodingMessagePreparation::Hello => continue,
                    CodingMessagePreparation::Ping => {
                        if !send_coding_json(&mut socket_tx, &CodingWsOutMessage::CodingPong).await {
                            break;
                        }
                        continue;
                    }
                    CodingMessagePreparation::Rejected => {
                        let _ = send_coding_json(
                            &mut socket_tx,
                            &CodingWsOutMessage::CodingProtocolError {
                                code: "coding_message_not_allowed".to_string(),
                                message: "message is not allowed in current coding stage".to_string(),
                            },
                        )
                        .await;
                        continue;
                    }
                    CodingMessagePreparation::RecoveryAlreadyActive => {
                        let _ = send_coding_json(
                            &mut socket_tx,
                            &CodingWsOutMessage::CodingProtocolError {
                                code: "coding_recovery_already_active".to_string(),
                                message: "coding runner is already active for failed review recovery"
                                    .to_string(),
                            },
                        )
                        .await;
                        continue;
                    }
                    CodingMessagePreparation::Allowed {
                        attempt,
                        mutation_lease,
                    } => (attempt, mutation_lease),
                    CodingMessagePreparation::FailedReviewRecovery {
                        attempt: updated,
                        gate_id,
                        reservation,
                    } => {
                        let command_tx = match spawn_coding_runner_reserved(
                            state.clone(),
                            coding_store.clone(),
                            event_tx.clone(),
                            updated.clone(),
                            &gate_id,
                            reservation,
                        ) {
                            Ok(command_tx) => command_tx,
                            Err(error) => {
                                let _ = send_coding_json(
                                    &mut socket_tx,
                                    &CodingWsOutMessage::CodingProtocolError {
                                        code: "coding_recovery_runner_activation_failed"
                                            .to_string(),
                                        message: error.to_string(),
                                    },
                                )
                                .await;
                                continue;
                            }
                        };
                        runner_started = true;
                        runner_command_tx = Some(command_tx);
                        if let Ok(snapshot) = build_coding_session_state(&coding_store, updated) {
                            let _ = send_coding_json(&mut socket_tx, &snapshot).await;
                        }
                        continue;
                    }
                };
                // F-14/F-44：sc_advance attempt 的 durable-ready 门——StartCoding 与
                // RestartCoding 共用同款语义（半启动恢复 sc_advance_restart_blocked
                // 亦同源）：未绑定 group / 未 Ready / 读取失败均 fail-closed 拒绝。
                if matches!(
                    inbound,
                    CodingWsInMessage::StartCoding | CodingWsInMessage::RestartCoding
                ) {
                    let advance_ready = if current_attempt.admission_kind
                        == CodingAdmissionKind::ScAdvance
                    {
                        match current_attempt.work_item_group_id.as_deref() {
                            None => false,
                            Some(plan_id) => match advance_store.advance_is_ready_for_attempt(
                                &current_attempt.project_id,
                                &current_attempt.issue_id,
                                plan_id,
                                &current_attempt.id,
                            ) {
                                Ok(ready) => ready,
                                Err(error) => {
                                    drop(mutation_lease);
                                    let _ = send_coding_json(
                                        &mut socket_tx,
                                        &CodingWsOutMessage::CodingProtocolError {
                                            code: "SC_CODING_REQUIRES_ADVANCE".to_string(),
                                            message: format!(
                                                "cannot verify durable advance readiness: {error}"
                                            ),
                                        },
                                    )
                                    .await;
                                    continue;
                                }
                            },
                        }
                    } else {
                        true
                    };
                    if !advance_ready {
                        drop(mutation_lease);
                        let _ = send_coding_json(
                            &mut socket_tx,
                            &CodingWsOutMessage::CodingProtocolError {
                                code: "SC_CODING_REQUIRES_ADVANCE".to_string(),
                                message: "SC coding attempts must be made ready through advance before StartCoding".to_string(),
                            },
                        )
                        .await;
                        continue;
                    }
                }
                if inbound == CodingWsInMessage::StartCoding {
                    if runner_started {
                        drop(mutation_lease);
                        let _ = send_coding_json(
                            &mut socket_tx,
                            &CodingWsOutMessage::CodingProtocolError {
                                code: "coding_runner_already_started".to_string(),
                                message: "coding runner is already active for this socket".to_string(),
                            },
                        )
                        .await;
                        continue;
                    }
                    let Some(command_tx) = spawn_coding_runner(
                        state.clone(),
                        coding_store.clone(),
                        event_tx.clone(),
                        current_attempt.clone(),
                    ) else {
                        drop(mutation_lease);
                        let _ = send_coding_json(
                            &mut socket_tx,
                            &CodingWsOutMessage::CodingProtocolError {
                                code: "coding_runner_already_started".to_string(),
                                message: "coding runner is already active for this attempt"
                                    .to_string(),
                            },
                        )
                        .await;
                        continue;
                    };
                    runner_started = true;
                    runner_command_tx = Some(command_tx);
                    drop(mutation_lease);
                } else if inbound == CodingWsInMessage::RestartCoding {
                    // F-44：中止/失败终态的显式「重新开始」通道——重走 admission
                    // CAS 回到 Running（重验路由/快照/policy），再复用 StartCoding/
                    // RecoverCoding 同款 spawn 路径重启 runner，由 runner 既有阶段链
                    // 从当前 stage 续跑。失败 fail-visible（coding_restart_failed），
                    // 不吞错误；状态门保证该分支只在 Aborted/Failed 下可达。
                    match coding_store.restart_terminal_attempt_for_execution(
                        &current_attempt.project_id,
                        &current_attempt.issue_id,
                        &current_attempt.id,
                    ) {
                        Ok(updated) => {
                            let Some(command_tx) = spawn_coding_runner(
                                state.clone(),
                                coding_store.clone(),
                                event_tx.clone(),
                                updated.clone(),
                            ) else {
                                drop(mutation_lease);
                                let _ = send_coding_json(
                                    &mut socket_tx,
                                    &CodingWsOutMessage::CodingProtocolError {
                                        code: "coding_runner_already_started".to_string(),
                                        message:
                                            "coding runner is already active for this attempt"
                                                .to_string(),
                                    },
                                )
                                .await;
                                continue;
                            };
                            runner_started = true;
                            runner_command_tx = Some(command_tx);
                            drop(mutation_lease);
                            if let Ok(snapshot) =
                                build_coding_session_state(&coding_store, updated)
                            {
                                let _ = send_coding_json(&mut socket_tx, &snapshot).await;
                            }
                        }
                        Err(error) => {
                            drop(mutation_lease);
                            let _ = send_coding_json(
                                &mut socket_tx,
                                &CodingWsOutMessage::CodingProtocolError {
                                    code: "coding_restart_failed".to_string(),
                                    message: error.to_string(),
                                },
                            )
                            .await;
                        }
                    }
                } else if inbound == CodingWsInMessage::RecoverCoding {
                    // F-16：人工恢复态的显式恢复通道——重走 admission CAS 回到
                    // Running（完整路由/快照/policy 重验），再复用 StartCoding
                    // 同款 spawn 路径重启 runner。失败 fail-visible
                    // （coding_recover_failed），不吞错误；状态门保证该分支只
                    // 在 AwaitingManualRecovery 下可达。
                    let recovered = coding_store.recover_attempt_from_manual_recovery(
                        &current_attempt.project_id,
                        &current_attempt.issue_id,
                        &current_attempt.id,
                    );
                    match recovered {
                        Ok(updated) => {
                            let Some(command_tx) = spawn_coding_runner(
                                state.clone(),
                                coding_store.clone(),
                                event_tx.clone(),
                                updated.clone(),
                            ) else {
                                drop(mutation_lease);
                                let _ = send_coding_json(
                                    &mut socket_tx,
                                    &CodingWsOutMessage::CodingProtocolError {
                                        code: "coding_runner_already_started".to_string(),
                                        message:
                                            "coding runner is already active for this attempt"
                                                .to_string(),
                                    },
                                )
                                .await;
                                continue;
                            };
                            runner_started = true;
                            runner_command_tx = Some(command_tx);
                            drop(mutation_lease);
                            if let Ok(snapshot) =
                                build_coding_session_state(&coding_store, updated)
                            {
                                let _ = send_coding_json(&mut socket_tx, &snapshot).await;
                            }
                        }
                        Err(error) => {
                            drop(mutation_lease);
                            let _ = send_coding_json(
                                &mut socket_tx,
                                &CodingWsOutMessage::CodingProtocolError {
                                    code: "coding_recover_failed".to_string(),
                                    message: error.to_string(),
                                },
                            )
                            .await;
                        }
                    }
                } else if inbound == CodingWsInMessage::FinalConfirm {
                    let engine = CodingWorkspaceEngine::new(
                        coding_store.clone(),
                        GitWorkspaceService::new(),
                        event_tx.clone(),
                    );
                    let updated = match engine
                        .handle_final_confirm(
                            &current_attempt.project_id,
                            &current_attempt.issue_id,
                            &current_attempt.id,
                        )
                        .await
                    {
                        Ok(updated) => updated,
                        Err(error) => {
                            drop(mutation_lease);
                            let _ = send_coding_json(
                                &mut socket_tx,
                                &CodingWsOutMessage::CodingProtocolError {
                                    code: "coding_final_confirm_failed".to_string(),
                                    message: error.to_string(),
                                },
                            )
                            .await;
                            continue;
                        }
                    };
                    drop(mutation_lease);
                    state
                        .coding_sockets
                        .wait_until_hub_drained(&attempt_key)
                        .await;
                    flush_queued_coding_events(&mut socket_tx, &mut event_rx).await;
                    if let Ok(snapshot) = build_coding_session_state(&coding_store, updated) {
                        let _ = send_coding_json(&mut socket_tx, &snapshot).await;
                    }
                } else if inbound == CodingWsInMessage::AbortAttempt {
                    let attempt_key = CodingAttemptRunKey::from_attempt(&current_attempt);
                    drop(mutation_lease);
                    state
                        .coding_sockets
                        .wait_until_hub_drained(&attempt_key)
                        .await;
                    let abort_result = abort_attempt_while_draining_events(
                        &state.coding_runs,
                        &attempt_key,
                        &mut event_rx,
                    )
                    .await;
                    tracing::debug!(
                        aborted_runners = abort_result.aborted_runners,
                        attempt_id = current_attempt.id.as_str(),
                        "coding runners stopped before durable websocket abort"
                    );
                    runner_command_tx = None;
                    runner_started = false;
                    for (index, event) in abort_result.events.iter().enumerate() {
                        if !send_coding_event(&mut socket_tx, event).await {
                            for remaining in &abort_result.events[index + 1..] {
                                fail_plan_amendment_socket_write(remaining);
                            }
                            break 'socket;
                        }
                    }
                    let mutation_lease = state.coding_runs.lock_attempt_mutation(&attempt_key).await;
                    let engine = CodingWorkspaceEngine::new(
                        coding_store.clone(),
                        GitWorkspaceService::new(),
                        event_tx.clone(),
                    );
                    let updated = match engine
                        .handle_abort(
                            &current_attempt.project_id,
                            &current_attempt.issue_id,
                            &current_attempt.id,
                        )
                        .await
                    {
                        Ok(updated) => updated,
                        Err(error) => {
                            drop(mutation_lease);
                            let _ = send_coding_json(
                                &mut socket_tx,
                                &CodingWsOutMessage::CodingProtocolError {
                                    code: "coding_abort_failed".to_string(),
                                    message: error.to_string(),
                                },
                            )
                            .await;
                            continue;
                        }
                    };
                    drop(mutation_lease);
                    state
                        .coding_sockets
                        .wait_until_hub_drained(&attempt_key)
                        .await;
                    flush_queued_coding_events(&mut socket_tx, &mut event_rx).await;
                    if let Ok(snapshot) = build_coding_session_state(&coding_store, updated) {
                        let _ = send_coding_json(&mut socket_tx, &snapshot).await;
                    }
                } else if inbound == CodingWsInMessage::RetryPush {
                    // runner 存活窗口内优先转发命令臂（覆盖 brief 窗口）；runner 已退出时
                    // 直接处理（仿 FinalConfirm 臂），复用 execute_review_request 的
                    // journal 幂等重入：Failed 重开重推，Pushed 幂等返回。
                    // `runner_command_tx` 仅在 AbortAttempt 时置 None；runner 正常走完
                    // （FinalConfirm/WaitingForHuman）退出后 receiver drop、channel 关闭，
                    // sender 仍为 Some。若只看 is_some 转发，send 会返回 Err 被丢弃且
                    // continue 跳过下方直接处理 → RetryPush 静默吞没。故用
                    // `!is_closed()` 把「已关闭 sender」也归入直接处理路径。
                    if let Some(command_tx) = runner_command_tx
                        .as_ref()
                        .filter(|tx| !tx.is_closed())
                    {
                        let _ = command_tx.send(CodingRunnerCommand::RetryPush).await;
                        drop(mutation_lease);
                        continue;
                    }
                    let engine = CodingWorkspaceEngine::new(
                        coding_store.clone(),
                        GitWorkspaceService::new(),
                        event_tx.clone(),
                    );
                    if let Err(error) = engine
                        .execute_review_request(
                            &current_attempt,
                            "origin",
                            "feat: implement work item",
                        )
                        .await
                    {
                        drop(mutation_lease);
                        let _ = send_coding_json(
                            &mut socket_tx,
                            &CodingWsOutMessage::CodingProtocolError {
                                code: "coding_retry_push_failed".to_string(),
                                message: error.to_string(),
                            },
                        )
                        .await;
                        continue;
                    }
                    let updated = match coding_store.get_attempt(
                        &current_attempt.project_id,
                        &current_attempt.issue_id,
                        &current_attempt.id,
                    ) {
                        Ok(updated) => updated,
                        Err(error) => {
                            drop(mutation_lease);
                            let _ = send_coding_json(
                                &mut socket_tx,
                                &CodingWsOutMessage::CodingProtocolError {
                                    code: "coding_retry_push_failed".to_string(),
                                    message: error.to_string(),
                                },
                            )
                            .await;
                            continue;
                        }
                    };
                    drop(mutation_lease);
                    state
                        .coding_sockets
                        .wait_until_hub_drained(&attempt_key)
                        .await;
                    flush_queued_coding_events(&mut socket_tx, &mut event_rx).await;
                    if let Ok(snapshot) = build_coding_session_state(&coding_store, updated) {
                        let _ = send_coding_json(&mut socket_tx, &snapshot).await;
                    }
                } else if let CodingWsInMessage::GateResponse {
                    gate_id,
                    action_id,
                    extra_context,
                } = inbound
                {
                    let engine = CodingWorkspaceEngine::new(
                        coding_store.clone(),
                        GitWorkspaceService::new(),
                        event_tx.clone(),
                    );
                    let updated = match engine
                        .handle_blocked_gate_response(
                            &current_attempt.project_id,
                            &current_attempt.issue_id,
                            &current_attempt.id,
                            &gate_id,
                            &action_id,
                            extra_context,
                        )
                        .await
                    {
                        Ok(updated) => updated,
                        Err(error) => {
                            drop(mutation_lease);
                            let _ = send_coding_json(
                                &mut socket_tx,
                                &CodingWsOutMessage::CodingProtocolError {
                                    code: "coding_gate_response_failed".to_string(),
                                    message: error.to_string(),
                                },
                            )
                            .await;
                            continue;
                        }
                    };
                    drop(mutation_lease);
                    state
                        .coding_sockets
                        .wait_until_hub_drained(&attempt_key)
                        .await;
                    flush_queued_coding_events(&mut socket_tx, &mut event_rx).await;
                    if let Ok(snapshot) = build_coding_session_state(&coding_store, updated) {
                        let _ = send_coding_json(&mut socket_tx, &snapshot).await;
                    }
                    if should_resume_runner_after_gate_response(&action_id, &current_attempt)
                        && let Ok(updated) = coding_store.get_attempt(
                            &current_attempt.project_id,
                            &current_attempt.issue_id,
                            &current_attempt.id,
                        ) && updated.status == CodingAttemptStatus::Running
                        && let Some(command_tx) = spawn_coding_runner(
                            state.clone(),
                            coding_store.clone(),
                            event_tx.clone(),
                            updated,
                        )
                    {
                        runner_started = true;
                        runner_command_tx = Some(command_tx);
                    }
                } else if let CodingWsInMessage::ProviderSelect { role, provider } = inbound {
                    if let Some(command_tx) = runner_command_tx.as_ref() {
                        let open_gates = coding_store
                            .list_open_stage_gates(
                                &current_attempt.project_id,
                                &current_attempt.issue_id,
                                &current_attempt.id,
                            )
                            .unwrap_or_default();
                        if !open_gates.is_empty() {
                            let _ = command_tx
                                .send(CodingRunnerCommand::ProviderSelect { role, provider })
                                .await;
                            drop(mutation_lease);
                            continue;
                        }
                    }
                    if provider_selection_targets_current_running_stage(&current_attempt, &role) {
                        drop(mutation_lease);
                        let _ = send_coding_json(
                            &mut socket_tx,
                            &CodingWsOutMessage::CodingProtocolError {
                                code: "coding_provider_role_locked".to_string(),
                                message: "provider for the current running stage cannot be changed".to_string(),
                            },
                        )
                        .await;
                        continue;
                    }
                    let (updated, changed_role, changed_provider) = match update_provider_selection(
                        &coding_store,
                        &current_attempt,
                        &role,
                        provider,
                    ) {
                        Ok(updated) => updated,
                        Err(error) => {
                            drop(mutation_lease);
                            let _ = send_coding_json(
                                &mut socket_tx,
                                &CodingWsOutMessage::CodingProtocolError {
                                    code: "coding_provider_select_failed".to_string(),
                                    message: error.to_string(),
                                },
                            )
                            .await;
                            continue;
                        }
                    };
                    drop(mutation_lease);
                    let _ = send_coding_json(
                        &mut socket_tx,
                        &CodingWsOutMessage::CodingProviderConfigUpdated {
                            role: changed_role,
                            provider: changed_provider,
                        },
                    )
                    .await;
                    if let Ok(snapshot) = build_coding_session_state(&coding_store, updated) {
                        let _ = send_coding_json(&mut socket_tx, &snapshot).await;
                    }
                } else if let CodingWsInMessage::PermissionModeSelect {
                    role,
                    permission_mode,
                } = inbound
                {
                    let (changed_role, current_provider) = match update_provider_permission_mode(
                        &coding_store,
                        &current_attempt,
                        &role,
                        permission_mode,
                    ) {
                        Ok(updated) => updated,
                        Err(error) => {
                            drop(mutation_lease);
                            let _ = send_coding_json(
                                &mut socket_tx,
                                &CodingWsOutMessage::CodingProtocolError {
                                    code: "coding_permission_mode_select_failed".to_string(),
                                    message: error.to_string(),
                                },
                            )
                            .await;
                            continue;
                        }
                    };
                    drop(mutation_lease);
                    let _ = send_coding_json(
                        &mut socket_tx,
                        &CodingWsOutMessage::CodingProviderConfigUpdated {
                            role: changed_role,
                            provider: current_provider,
                        },
                    )
                    .await;
                    if let Ok(snapshot) =
                        build_coding_session_state(&coding_store, current_attempt.clone())
                    {
                        let _ = send_coding_json(&mut socket_tx, &snapshot).await;
                    }
                } else if let CodingWsInMessage::MaxAutoReworkSelect { max_auto_rework } = inbound
                {
                    let updated = match coding_store.update_attempt_max_auto_rework(
                        &current_attempt.project_id,
                        &current_attempt.issue_id,
                        &current_attempt.id,
                        max_auto_rework,
                    ) {
                        Ok(updated) => updated,
                        Err(error) => {
                            drop(mutation_lease);
                            let code = if error.to_string().contains("invalid_max_auto_rework") {
                                "invalid_max_auto_rework"
                            } else {
                                "coding_max_auto_rework_select_failed"
                            };
                            let _ = send_coding_json(
                                &mut socket_tx,
                                &CodingWsOutMessage::CodingProtocolError {
                                    code: code.to_string(),
                                    message: error.to_string(),
                                },
                            )
                            .await;
                            continue;
                        }
                    };
                    drop(mutation_lease);
                    if let Ok(snapshot) = build_coding_session_state(&coding_store, updated) {
                        let _ = send_coding_json(&mut socket_tx, &snapshot).await;
                    }
                } else if let CodingWsInMessage::StageGateConfirm { stage } = inbound {
                    if let Some(command_tx) = runner_command_tx.as_ref() {
                        let open_gates = coding_store
                            .list_open_stage_gates(
                                &current_attempt.project_id,
                                &current_attempt.issue_id,
                                &current_attempt.id,
                            )
                            .unwrap_or_default();
                        if !open_gates.is_empty() {
                            let _ = command_tx
                                .send(CodingRunnerCommand::StageGateConfirm {
                                    stage: stage.clone(),
                                })
                                .await;
                            drop(mutation_lease);
                            continue;
                        }
                    }
                    let confirmation =
                        confirm_open_stage_gate(&coding_store, &current_attempt, &stage);
                    drop(mutation_lease);
                    match confirmation {
                        Ok(Some(_gate)) => {
                            if let Ok(snapshot) =
                                build_coding_session_state(&coding_store, current_attempt)
                            {
                                let _ = send_coding_json(&mut socket_tx, &snapshot).await;
                            }
                        }
                        Ok(None) => {
                            let _ = send_coding_json(
                                &mut socket_tx,
                                &CodingWsOutMessage::CodingProtocolError {
                                    code: "coding_stage_gate_not_found".to_string(),
                                    message: "open stage gate was not found".to_string(),
                                },
                            )
                            .await;
                        }
                        Err(error) => {
                            let _ = send_coding_json(
                                &mut socket_tx,
                                &CodingWsOutMessage::CodingProtocolError {
                                    code: "coding_stage_gate_confirm_failed".to_string(),
                                    message: error.to_string(),
                                },
                            )
                            .await;
                        }
                    }
                } else if let CodingWsInMessage::PermissionResponse {
                    id,
                    approved,
                    reason,
                } = inbound
                {
                    if let Some(command_tx) =
                        interactive_runner_sender(&state, &attempt_key, runner_command_tx.as_ref())
                    {
                        let _ = command_tx
                            .send(CodingRunnerCommand::PermissionResponse {
                                id,
                                approved,
                                reason,
                            })
                            .await;
                    }
                    drop(mutation_lease);
                } else if let CodingWsInMessage::ChoiceResponse {
                    id,
                    selected_option_ids,
                    free_text,
                    ..
                } = inbound
                {
                    if let Some(command_tx) =
                        interactive_runner_sender(&state, &attempt_key, runner_command_tx.as_ref())
                    {
                        // P0 1.3：legacy WS 单题入口——T9 接线完整
                        // answers/command_id claim；此处保持原单题等价行为。
                        let _ = command_tx
                            .send(CodingRunnerCommand::ChoiceResponse {
                                id,
                                selected_option_ids,
                                free_text,
                                answers: Vec::new(),
                                receipt: None,
                            })
                            .await;
                        drop(mutation_lease);
                    } else {
                        drop(mutation_lease);
                        let _ = send_coding_json(
                            &mut socket_tx,
                            &CodingWsOutMessage::CodingProtocolError {
                                code: "coding_choice_runner_not_active".to_string(),
                                message: format!(
                                    "ChoiceResponse id={id} cannot be delivered because no coding runner is active"
                                ),
                            },
                        )
                        .await;
                    }
                } else if let CodingWsInMessage::ContextNote { content } = inbound {
                    let note = match coding_store.create_context_note(&current_attempt, content)
                    {
                        Ok(note) => note,
                        Err(error) => {
                            drop(mutation_lease);
                            let _ = send_coding_json(
                                &mut socket_tx,
                                &CodingWsOutMessage::CodingProtocolError {
                                    code: "coding_context_note_failed".to_string(),
                                    message: error.to_string(),
                                },
                            )
                            .await;
                            continue;
                        }
                    };
                    let entry = match context_note_chat_entry(&coding_store, &current_attempt, note)
                    {
                        Ok(entry) => entry,
                        Err(error) => {
                            drop(mutation_lease);
                            let _ = send_coding_json(
                                &mut socket_tx,
                                &CodingWsOutMessage::CodingProtocolError {
                                    code: "coding_context_note_echo_failed".to_string(),
                                    message: error.to_string(),
                                },
                            )
                            .await;
                            continue;
                        }
                    };
                    let _ = coding_store.save_chat_entry(&current_attempt, &entry);
                    drop(mutation_lease);
                    if !send_coding_json(
                        &mut socket_tx,
                        &CodingWsOutMessage::CodingChatEntryCreated { entry },
                    )
                    .await
                    {
                        break;
                    }
                }
                    }
                    Message::Ping(bytes) => match socket_tx.send(Message::Pong(bytes)).await {
                        Ok(()) => {}
                        Err(_) => break,
                    },
                    Message::Close(_) => break,
                    _ => {}
                }
            }
        }
    }
    state.coding_sockets.remove(&attempt_key, socket_token);
}

/// F-43：把该 attempt 当前未决 choice 逐帧补发给本次连接。
///
/// 读取面与快照同源（`build_coding_session_state` 读同一 choice-gate 目录），
/// 故此处读取失败只可能是「快照已失败/已给出空投影」的同一次故障，不再额外打断
/// 连接；仅写失败（连接已断）返回 false 交调用方收尾。
async fn send_pending_choice_frames<S>(
    socket: &mut S,
    coding_store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
) -> bool
where
    S: Sink<Message> + Unpin,
{
    let Ok(frames) = pending_choice_frames(coding_store, attempt) else {
        return true;
    };
    for frame in &frames {
        if !send_coding_json(socket, frame).await {
            return false;
        }
    }
    true
}

/// F-43：交互应答（choice/permission）的投递通道。
///
/// 本 socket 持有 runner 句柄（StartCoding/RecoverCoding 由本连接启动）时逐字
/// 沿用既有通道，行为零变化；无句柄的连接（页面刷新/新开 tab）回落到注册表里该
/// attempt 的 runner 命令通道——runner 仍在等这份应答，否则作答被拒
/// （`coding_choice_runner_not_active`）或被静默丢弃，「卡在了、点了没反应」仍是
/// 死锁（与 `abort_attempt` 同源的 attempt 级路由）。
fn interactive_runner_sender(
    state: &WebAppState,
    attempt_key: &CodingAttemptRunKey,
    local: Option<&mpsc::Sender<CodingRunnerCommand>>,
) -> Option<mpsc::Sender<CodingRunnerCommand>> {
    local
        .cloned()
        .or_else(|| state.coding_runs.command_sender(attempt_key))
}

pub fn is_coding_ws_message_allowed(
    status: &CodingAttemptStatus,
    stage: &CodingExecutionStage,
    message: &CodingWsInMessage,
) -> bool {
    if matches!(
        message,
        CodingWsInMessage::CodingHello { .. } | CodingWsInMessage::CodingPing
    ) {
        return true;
    }
    if matches!(
        status,
        CodingAttemptStatus::Completed | CodingAttemptStatus::Failed | CodingAttemptStatus::Aborted
    ) {
        // F-44：`Aborted`/`Failed` 是「可重新开始」的终态——只放行显式重开动作
        // RestartCoding（重走 admission CAS 回 Running 并重启 runner）；已完成
        // 的 attempt 不提供重开。StartCoding 等其余消息维持 F-14 fail-closed
        // 拒绝（终态不得被隐式唤醒，F-43 终态 group 拒绝面零回归）。
        return matches!(
            status,
            CodingAttemptStatus::Aborted | CodingAttemptStatus::Failed
        ) && matches!(message, CodingWsInMessage::RestartCoding);
    }
    if matches!(message, CodingWsInMessage::ContextNote { .. }) && status.is_active() {
        return true;
    }
    if matches!(message, CodingWsInMessage::StageGateConfirm { .. }) && status.is_active() {
        return true;
    }
    if matches!(message, CodingWsInMessage::ProviderSelect { .. }) && status.is_active() {
        return true;
    }
    if matches!(message, CodingWsInMessage::PermissionModeSelect { .. }) && status.is_active() {
        return true;
    }
    if matches!(message, CodingWsInMessage::GateResponse { .. })
        && *status == CodingAttemptStatus::WaitingForHuman
    {
        return true;
    }
    if *status == CodingAttemptStatus::Blocked {
        return matches!(
            message,
            CodingWsInMessage::GateResponse { .. } | CodingWsInMessage::AbortAttempt
        );
    }
    if *status == CodingAttemptStatus::AwaitingManualRecovery {
        // F-16：AbortAttempt（终态出口）之外仅放行显式恢复动作 RecoverCoding
        // （重走 admission CAS 回 Running + 重启 runner）；F-14 fail-closed
        // 白名单的其余收紧面不动。
        return matches!(
            message,
            CodingWsInMessage::AbortAttempt | CodingWsInMessage::RecoverCoding
        );
    }
    match stage {
        CodingExecutionStage::PrepareContext => matches!(
            message,
            CodingWsInMessage::ContextNote { .. }
                | CodingWsInMessage::StartCoding
                | CodingWsInMessage::ProviderSelect { .. }
                | CodingWsInMessage::PermissionModeSelect { .. }
                | CodingWsInMessage::MaxAutoReworkSelect { .. }
                | CodingWsInMessage::AbortAttempt
        ),
        CodingExecutionStage::WorktreePrepare => matches!(message, CodingWsInMessage::AbortAttempt),
        CodingExecutionStage::ReviewRequest => {
            matches!(
                message,
                CodingWsInMessage::StartCoding
                    | CodingWsInMessage::RetryPush
                    | CodingWsInMessage::AbortAttempt
            )
        }
        CodingExecutionStage::Coding
        | CodingExecutionStage::CodeReview
        | CodingExecutionStage::InternalPrReview => matches!(
            message,
            CodingWsInMessage::ContextNote { .. }
                | CodingWsInMessage::PermissionResponse { .. }
                | CodingWsInMessage::ChoiceResponse { .. }
                | CodingWsInMessage::AbortAttempt
        ),
        CodingExecutionStage::FinalConfirm => matches!(
            message,
            CodingWsInMessage::FinalConfirm
                | CodingWsInMessage::GateResponse { .. }
                | CodingWsInMessage::RetryPush
                | CodingWsInMessage::AbortAttempt
        ),
    }
}
