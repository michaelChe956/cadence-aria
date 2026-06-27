use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path as AxumPath, State};
use axum::response::IntoResponse;
use futures_util::stream::SplitSink;
use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;

use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_attempt_store::CodingAttemptStore;
use crate::product::coding_models::{CodingAttemptStatus, CodingExecutionStage};
use crate::product::coding_workspace_engine::CodingWorkspaceEngine;
use crate::product::coding_workspace_runner::CodingRunnerCommand;
use crate::product::git_workspace_service::GitWorkspaceService;
use crate::web::state::WebAppState;

use super::{
    CodingWsInMessage, CodingWsOutMessage, build_coding_session_state, confirm_open_stage_gate,
    context_note_chat_entry, provider_selection_targets_current_running_stage,
    should_resume_runner_after_gate_response, spawn_coding_runner, update_provider_permission_mode,
    update_provider_selection,
};

pub(crate) type CodingWsSender = SplitSink<WebSocket, Message>;

pub async fn coding_ws(
    ws: WebSocketUpgrade,
    AxumPath(attempt_id): AxumPath<String>,
    State(state): State<WebAppState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_coding_socket(socket, attempt_id, state))
        .into_response()
}

async fn handle_coding_socket(socket: WebSocket, attempt_id: String, state: WebAppState) {
    let (mut socket_tx, mut socket_rx) = socket.split();
    let app_paths = ProductAppPaths::new(state.workspace_root.join(".aria"));
    let coding_store = CodingAttemptStore::new(app_paths);
    let attempt = match coding_store.get_attempt_by_id(&attempt_id) {
        Ok(attempt) => attempt,
        Err(error) => {
            let _ = send_coding_json(
                &mut socket_tx,
                &CodingWsOutMessage::CodingProtocolError {
                    code: "coding_attempt_not_found".to_string(),
                    message: format!("coding attempt not found: {error}"),
                },
            )
            .await;
            return;
        }
    };
    if let Ok(snapshot) = build_coding_session_state(&coding_store, attempt)
        && !send_coding_json(&mut socket_tx, &snapshot).await
    {
        return;
    }

    let (event_tx, mut event_rx) = mpsc::channel(1024);
    let mut runner_started = false;
    let mut runner_command_tx: Option<mpsc::Sender<CodingRunnerCommand>> = None;
    loop {
        tokio::select! {
            event = event_rx.recv() => {
                let Some(event) = event else {
                    continue;
                };
                if !send_coding_json(&mut socket_tx, &event).await {
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
                if inbound == CodingWsInMessage::CodingPing {
                    if !send_coding_json(&mut socket_tx, &CodingWsOutMessage::CodingPong).await {
                        break;
                    }
                    continue;
                }
                let Ok(current_attempt) = coding_store.get_attempt_by_id(&attempt_id) else {
                    break;
                };
                if !is_coding_ws_message_allowed(
                    &current_attempt.status,
                    &current_attempt.stage,
                    &inbound,
                ) {
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
                if inbound == CodingWsInMessage::StartCoding {
                    if runner_started {
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
                    runner_started = true;
                    runner_command_tx = Some(spawn_coding_runner(
                        state.clone(),
                        coding_store.clone(),
                        event_tx.clone(),
                        current_attempt.clone(),
                    ));
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
                    while let Ok(event) = event_rx.try_recv() {
                        if !send_coding_json(&mut socket_tx, &event).await {
                            break;
                        }
                    }
                    if let Ok(snapshot) = build_coding_session_state(&coding_store, updated) {
                        let _ = send_coding_json(&mut socket_tx, &snapshot).await;
                    }
                } else if inbound == CodingWsInMessage::AbortAttempt {
                    if let Some(command_tx) = runner_command_tx.as_ref() {
                        let open_gates = coding_store
                            .list_open_stage_gates(
                                &current_attempt.project_id,
                                &current_attempt.issue_id,
                                &current_attempt.id,
                            )
                            .unwrap_or_default();
                        if !open_gates.is_empty() {
                            let _ = command_tx.send(CodingRunnerCommand::AbortAttempt).await;
                            continue;
                        }
                        let _ = command_tx.send(CodingRunnerCommand::AbortAttempt).await;
                    }
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
                    while let Ok(event) = event_rx.try_recv() {
                        if !send_coding_json(&mut socket_tx, &event).await {
                            break;
                        }
                    }
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
                    while let Ok(event) = event_rx.try_recv() {
                        if !send_coding_json(&mut socket_tx, &event).await {
                            break;
                        }
                    }
                    if let Ok(snapshot) = build_coding_session_state(&coding_store, updated) {
                        let _ = send_coding_json(&mut socket_tx, &snapshot).await;
                    }
                    if should_resume_runner_after_gate_response(&action_id, &current_attempt) {
                        runner_started = true;
                        if let Ok(updated) = coding_store.get_attempt(
                            &current_attempt.project_id,
                            &current_attempt.issue_id,
                            &current_attempt.id,
                        ) && updated.status == CodingAttemptStatus::Running
                        {
                            runner_command_tx = Some(spawn_coding_runner(
                                state.clone(),
                                coding_store.clone(),
                                event_tx.clone(),
                                updated,
                            ));
                        }
                    }
                } else if let CodingWsInMessage::ContinueRework { extra_context } = inbound {
                    let engine = CodingWorkspaceEngine::new(
                        coding_store.clone(),
                        GitWorkspaceService::new(),
                        event_tx.clone(),
                    );
                    let updated = match engine.continue_rework_after_limit(
                        &current_attempt.project_id,
                        &current_attempt.issue_id,
                        &current_attempt.id,
                        extra_context,
                    ) {
                        Ok(updated) => updated,
                        Err(error) => {
                            let _ = send_coding_json(
                                &mut socket_tx,
                                &CodingWsOutMessage::CodingProtocolError {
                                    code: "coding_continue_rework_failed".to_string(),
                                    message: error.to_string(),
                                },
                            )
                            .await;
                            continue;
                        }
                    };
                    if let Ok(snapshot) =
                        build_coding_session_state(&coding_store, updated.clone())
                    {
                        let _ = send_coding_json(&mut socket_tx, &snapshot).await;
                    }
                    if updated.status == CodingAttemptStatus::Running {
                        runner_started = true;
                        runner_command_tx = Some(spawn_coding_runner(
                            state.clone(),
                            coding_store.clone(),
                            event_tx.clone(),
                            updated,
                        ));
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
                            continue;
                        }
                    }
                    if provider_selection_targets_current_running_stage(&current_attempt, &role) {
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
                            continue;
                        }
                    }
                    match confirm_open_stage_gate(&coding_store, &current_attempt, &stage) {
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
                    } else {
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
                    let note = match coding_store.create_context_note(&current_attempt.id, content)
                    {
                        Ok(note) => note,
                        Err(error) => {
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
                    let _ = coding_store.save_chat_entry(&entry);
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
}

pub(crate) async fn send_coding_json(
    socket: &mut CodingWsSender,
    message: &CodingWsOutMessage,
) -> bool {
    match serde_json::to_string(message) {
        Ok(json) => socket.send(Message::Text(json.into())).await.is_ok(),
        Err(_) => false,
    }
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
    if matches!(message, CodingWsInMessage::ContinueRework { .. }) {
        return *status == CodingAttemptStatus::WaitingForHuman
            && *stage == CodingExecutionStage::Rework;
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
    match stage {
        CodingExecutionStage::PrepareContext => matches!(
            message,
            CodingWsInMessage::ContextNote { .. }
                | CodingWsInMessage::StartCoding
                | CodingWsInMessage::ProviderSelect { .. }
                | CodingWsInMessage::PermissionModeSelect { .. }
                | CodingWsInMessage::AbortAttempt
        ),
        CodingExecutionStage::WorktreePrepare | CodingExecutionStage::ReviewRequest => {
            matches!(message, CodingWsInMessage::AbortAttempt)
        }
        CodingExecutionStage::Coding
        | CodingExecutionStage::Testing
        | CodingExecutionStage::Rework
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
                | CodingWsInMessage::AbortAttempt
        ),
    }
}
