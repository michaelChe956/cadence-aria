use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path as AxumPath, State};
use axum::response::IntoResponse;
use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;

use crate::product::advance_store::AdvanceStore;
use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_attempt_store::CodingAttemptStore;
use crate::product::coding_models::{
    CodingAdmissionKind, CodingAttemptStatus, CodingExecutionStage,
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
    provider_selection_targets_current_running_stage, should_resume_runner_after_gate_response,
    spawn_coding_runner, spawn_coding_runner_reserved, update_provider_permission_mode,
    update_provider_selection,
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
#[cfg(test)]
pub(crate) use preparation::{CodingRecoveryPreparationProbe, prepare_coding_message_with_probe};

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
    let (event_tx, event_rx) = mpsc::channel(1024);
    let socket_token = state
        .coding_sockets
        .register(&attempt_key, event_tx.clone());
    if let Ok(snapshot) = build_coding_session_state(&coding_store, attempt)
        && !send_coding_json(&mut socket_tx, &snapshot).await
    {
        state.coding_sockets.remove(&attempt_key, socket_token);
        return;
    }

    let mut event_rx = OutboundEventReceiver::new(event_rx);
    let mut runner_started = false;
    let mut runner_command_tx: Option<mpsc::Sender<CodingRunnerCommand>> = None;
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
                if inbound == CodingWsInMessage::StartCoding {
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
                    flush_queued_coding_events(&mut socket_tx, &mut event_rx).await;
                    if let Ok(snapshot) = build_coding_session_state(&coding_store, updated) {
                        let _ = send_coding_json(&mut socket_tx, &snapshot).await;
                    }
                } else if inbound == CodingWsInMessage::AbortAttempt {
                    let attempt_key = CodingAttemptRunKey::from_attempt(&current_attempt);
                    drop(mutation_lease);
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
                    if let Some(command_tx) = runner_command_tx.as_ref() {
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
                } = inbound
                {
                    if let Some(command_tx) = runner_command_tx.as_ref() {
                        let _ = command_tx
                            .send(CodingRunnerCommand::ChoiceResponse {
                                id,
                                selected_option_ids,
                                free_text,
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
        return false;
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
        return matches!(message, CodingWsInMessage::AbortAttempt);
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
