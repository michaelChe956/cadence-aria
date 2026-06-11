use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path as AxumPath, State};
use axum::response::IntoResponse;
use chrono::{Duration as ChronoDuration, Utc};
use futures_util::stream::SplitSink;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};

use crate::cross_cutting::streaming_provider::StreamingProviderAdapter;
use crate::product::app_paths::ProductAppPaths;
use crate::product::artifact_extraction::extract_artifact_content;
use crate::product::coding_attempt_store::CodingAttemptStore;
use crate::product::coding_models::{
    CodeReviewReport, CodingAgentRole, CodingAttemptStatus, CodingChatEntry, CodingContextNote,
    CodingEntryType, CodingExecutionAttempt, CodingExecutionStage, CodingGateAction,
    CodingGateActionType, CodingGateKind, CodingGateRequired as CodingGateRequiredModel,
    CodingProviderPermissionMode, CodingProviderRole, CodingRoleProviderConfigSnapshot,
    CodingStageGateState, CodingStageGateStatus, CodingTimelineNode, CodingTimelineNodeStatus,
    InternalPrReview, PushStatus, ReviewRequest, ReviewVerdict, TestingReport,
};
use crate::product::coding_workspace_engine::{
    CodingExecutionContext, CodingWorkspaceEngine, CodingWorkspaceEngineError,
};
use crate::product::coding_workspace_runner::{
    CodingRunnerCommand, apply_provider_selection_to_snapshots, coding_provider_role_for_stage,
    parse_coding_provider_role,
};
use crate::product::git_workspace_service::GitWorkspaceService;
use crate::product::json_store::ProductStoreError;
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::models::{
    ProviderName, WorkspaceSessionRecord, WorkspaceSessionStatus, WorkspaceType,
};
use crate::product::repository_store::RepositoryStore;
use crate::product::test_executor::{
    TestCommandSpec, discover_test_commands, planned_test_commands_from_markdown,
};
use crate::product::tester_agent_loop::TesterAgentOptions;
use crate::web::state::WebAppState;
use crate::web::workspace_ws_types::{
    ChoiceOption, ProviderConfigSnapshot, WsExecutionEvent, WsPermissionRiskLevel,
};
use tokio::sync::mpsc;
use tokio::time::{Duration, Instant};

type CodingWsSender = SplitSink<WebSocket, Message>;
const STAGE_GATE_COUNTDOWN_SECONDS: u64 = 5;

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
                    let runner_state = state.clone();
                    let runner_store = coding_store.clone();
                    let runner_attempt = current_attempt.clone();
                    let runner_event_tx = event_tx.clone();
                    let (command_tx, command_rx) = mpsc::channel(32);
                    runner_command_tx = Some(command_tx);
                    tokio::spawn(async move {
                    let engine = CodingWorkspaceEngine::new(
                            runner_store.clone(),
                        GitWorkspaceService::new(),
                            runner_event_tx.clone(),
                    );
                        if let Err(error) = execute_start_coding_flow(
                            &runner_state,
                            &runner_store,
                        &engine,
                            &runner_event_tx,
                            command_rx,
                            &runner_attempt,
                    )
                    .await
                    {
                            if matches!(error, CodingWorkspaceEngineError::Aborted) {
                                return;
                            }
                            let _ = runner_event_tx
                                .send(CodingWsOutMessage::CodingProtocolError {
                                    code: "coding_start_failed".to_string(),
                                    message: error.to_string(),
                                })
                                .await;
                        }
                    });
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

async fn execute_start_coding_flow(
    state: &WebAppState,
    coding_store: &CodingAttemptStore,
    engine: &CodingWorkspaceEngine,
    event_tx: &mpsc::Sender<CodingWsOutMessage>,
    mut command_rx: mpsc::Receiver<CodingRunnerCommand>,
    attempt: &CodingExecutionAttempt,
) -> Result<(), CodingWorkspaceEngineError> {
    let app_paths = ProductAppPaths::new(state.workspace_root.join(".aria"));
    let repo_path = repository_path_for_attempt(&app_paths, attempt)?;
    let execution_context = coding_execution_context(&app_paths, attempt)?;

    let mut current = engine
        .start_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .await?;
    if handle_pending_runner_commands(&mut command_rx, coding_store, engine, event_tx, &current)
        .await?
    {
        return Ok(());
    }
    current = engine
        .execute_worktree_prepare(&current, &repo_path)
        .await?;
    if handle_pending_runner_commands(&mut command_rx, coding_store, engine, event_tx, &current)
        .await?
    {
        return Ok(());
    }
    'pipeline: loop {
        {
            let Some(next) = await_stage_gate(
                &mut command_rx,
                coding_store,
                engine,
                event_tx,
                &current,
                CodingExecutionStage::Coding,
            )
            .await?
            else {
                return Ok(());
            };
            current = next;
            let author_provider_name = coding_store
                .get_role_provider_config_snapshot(
                    &current.project_id,
                    &current.issue_id,
                    &current.id,
                )?
                .coder;
            let author_provider =
                provider_for(state, &author_provider_name, "coding author provider")?;
            current = engine
                .execute_coding_with_commands(
                    &current,
                    author_provider.as_ref(),
                    &execution_context,
                    &mut command_rx,
                )
                .await?;
            if handle_pending_runner_commands(
                &mut command_rx,
                coding_store,
                engine,
                event_tx,
                &current,
            )
            .await?
            {
                return Ok(());
            }

            let Some(next) = await_stage_gate(
                &mut command_rx,
                coding_store,
                engine,
                event_tx,
                &current,
                CodingExecutionStage::Testing,
            )
            .await?
            else {
                return Ok(());
            };
            current = next;
            let test_specs = test_specs_for_attempt(&current, &execution_context);
            let tester_provider_name = coding_store
                .get_role_provider_config_snapshot(
                    &current.project_id,
                    &current.issue_id,
                    &current.id,
                )?
                .tester;
            let tester_provider =
                provider_for(state, &tester_provider_name, "coding tester provider")?;
            let testing_report = engine
                .execute_testing_with_provider_commands(
                    &current,
                    tester_provider.as_ref(),
                    &execution_context,
                    &test_specs,
                    TesterAgentOptions::default(),
                    &mut command_rx,
                )
                .await?;
            current =
                coding_store.get_attempt(&current.project_id, &current.issue_id, &current.id)?;
            if handle_pending_runner_commands(
                &mut command_rx,
                coding_store,
                engine,
                event_tx,
                &current,
            )
            .await?
            {
                return Ok(());
            }

            let Some(next) = await_stage_gate(
                &mut command_rx,
                coding_store,
                engine,
                event_tx,
                &current,
                CodingExecutionStage::Rework,
            )
            .await?
            else {
                return Ok(());
            };
            current = next;
            let analyst_provider_name = coding_store
                .get_role_provider_config_snapshot(
                    &current.project_id,
                    &current.issue_id,
                    &current.id,
                )?
                .analyst;
            let analyst_provider =
                provider_for(state, &analyst_provider_name, "coding analyst provider")?;
            let evidence = testing_rework_evidence(&testing_report);
            current = engine
                .execute_rework_with_commands(
                    &current,
                    &evidence,
                    analyst_provider.as_ref(),
                    &mut command_rx,
                )
                .await?;
            current =
                coding_store.get_attempt(&current.project_id, &current.issue_id, &current.id)?;
            if handle_pending_runner_commands(
                &mut command_rx,
                coding_store,
                engine,
                event_tx,
                &current,
            )
            .await?
            {
                return Ok(());
            }

            match current.stage {
                CodingExecutionStage::Coding => continue 'pipeline,
                CodingExecutionStage::CodeReview => {}
                _ => return emit_current_session_state(event_tx, coding_store, &current).await,
            }
        }

        {
            let Some(next) = await_stage_gate(
                &mut command_rx,
                coding_store,
                engine,
                event_tx,
                &current,
                CodingExecutionStage::CodeReview,
            )
            .await?
            else {
                return Ok(());
            };
            current = next;
            let reviewer_provider_name = coding_store
                .get_role_provider_config_snapshot(
                    &current.project_id,
                    &current.issue_id,
                    &current.id,
                )?
                .code_reviewer;
            let reviewer_provider =
                provider_for(state, &reviewer_provider_name, "coding reviewer provider")?;
            let review_report = engine
                .execute_code_review_with_commands(
                    &current,
                    reviewer_provider.as_ref(),
                    &mut command_rx,
                )
                .await?;
            current =
                coding_store.get_attempt(&current.project_id, &current.issue_id, &current.id)?;
            if handle_pending_runner_commands(
                &mut command_rx,
                coding_store,
                engine,
                event_tx,
                &current,
            )
            .await?
            {
                return Ok(());
            }
            if review_report.verdict == ReviewVerdict::Blocked {
                return emit_current_session_state(event_tx, coding_store, &current).await;
            }

            let Some(next) = await_stage_gate(
                &mut command_rx,
                coding_store,
                engine,
                event_tx,
                &current,
                CodingExecutionStage::Rework,
            )
            .await?
            else {
                return Ok(());
            };
            current = next;
            let analyst_provider_name = coding_store
                .get_role_provider_config_snapshot(
                    &current.project_id,
                    &current.issue_id,
                    &current.id,
                )?
                .analyst;
            let analyst_provider =
                provider_for(state, &analyst_provider_name, "coding analyst provider")?;
            let evidence = code_review_rework_evidence(&review_report);
            current = engine
                .execute_rework_with_commands(
                    &current,
                    &evidence,
                    analyst_provider.as_ref(),
                    &mut command_rx,
                )
                .await?;
            current =
                coding_store.get_attempt(&current.project_id, &current.issue_id, &current.id)?;
            if handle_pending_runner_commands(
                &mut command_rx,
                coding_store,
                engine,
                event_tx,
                &current,
            )
            .await?
            {
                return Ok(());
            }
            match current.stage {
                CodingExecutionStage::Coding => continue 'pipeline,
                CodingExecutionStage::ReviewRequest => {}
                _ => return emit_current_session_state(event_tx, coding_store, &current).await,
            }

            let review_request = engine
                .execute_review_request(&current, "origin", "feat: implement work item")
                .await?;
            current =
                coding_store.get_attempt(&current.project_id, &current.issue_id, &current.id)?;
            if handle_pending_runner_commands(
                &mut command_rx,
                coding_store,
                engine,
                event_tx,
                &current,
            )
            .await?
            {
                return Ok(());
            }
            if review_request.push_status != PushStatus::Pushed {
                return emit_current_session_state(event_tx, coding_store, &current).await;
            }

            let Some(next) = await_stage_gate(
                &mut command_rx,
                coding_store,
                engine,
                event_tx,
                &current,
                CodingExecutionStage::InternalPrReview,
            )
            .await?
            else {
                return Ok(());
            };
            current = next;
            let internal_reviewer_provider_name = coding_store
                .get_role_provider_config_snapshot(
                    &current.project_id,
                    &current.issue_id,
                    &current.id,
                )?
                .internal_reviewer;
            let internal_reviewer_provider = provider_for(
                state,
                &internal_reviewer_provider_name,
                "coding internal reviewer provider",
            )?;
            let internal_review = engine
                .execute_internal_pr_review_with_commands(
                    &current,
                    internal_reviewer_provider.as_ref(),
                    &mut command_rx,
                )
                .await?;
            current =
                coding_store.get_attempt(&current.project_id, &current.issue_id, &current.id)?;
            if handle_pending_runner_commands(
                &mut command_rx,
                coding_store,
                engine,
                event_tx,
                &current,
            )
            .await?
            {
                return Ok(());
            }
            if internal_review.verdict == ReviewVerdict::Blocked {
                return emit_current_session_state(event_tx, coding_store, &current).await;
            }

            let Some(next) = await_stage_gate(
                &mut command_rx,
                coding_store,
                engine,
                event_tx,
                &current,
                CodingExecutionStage::Rework,
            )
            .await?
            else {
                return Ok(());
            };
            current = next;
            let analyst_provider_name = coding_store
                .get_role_provider_config_snapshot(
                    &current.project_id,
                    &current.issue_id,
                    &current.id,
                )?
                .analyst;
            let analyst_provider =
                provider_for(state, &analyst_provider_name, "coding analyst provider")?;
            let evidence = internal_pr_review_rework_evidence(&internal_review);
            current = engine
                .execute_rework_with_commands(
                    &current,
                    &evidence,
                    analyst_provider.as_ref(),
                    &mut command_rx,
                )
                .await?;
            current =
                coding_store.get_attempt(&current.project_id, &current.issue_id, &current.id)?;
            if handle_pending_runner_commands(
                &mut command_rx,
                coding_store,
                engine,
                event_tx,
                &current,
            )
            .await?
            {
                return Ok(());
            }
            match current.stage {
                CodingExecutionStage::Coding => continue 'pipeline,
                CodingExecutionStage::FinalConfirm => {
                    return emit_current_session_state(event_tx, coding_store, &current).await;
                }
                _ => return emit_current_session_state(event_tx, coding_store, &current).await,
            }
        }
    }
}

async fn await_stage_gate(
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
        &current.id,
        stage.clone(),
        role,
        expires_at,
        provider_snapshot,
    )?;
    emit_stage_gate(event_tx, coding_store, &current, &gate).await?;

    loop {
        tokio::select! {
            _ = tokio::time::sleep_until(deadline) => {
                let _ = coding_store.update_stage_gate_status(
                    &current.project_id,
                    &current.issue_id,
                    &current.id,
                    &gate.gate_id,
                    CodingStageGateStatus::Expired,
                )?;
                let snapshot = build_coding_session_state(coding_store, current.clone())?;
                let _ = event_tx.send(snapshot).await;
                return Ok(Some(current));
            }
            command = command_rx.recv() => {
                let Some(command) = command else {
                    tokio::time::sleep_until(deadline).await;
                    let _ = coding_store.update_stage_gate_status(
                        &current.project_id,
                        &current.issue_id,
                        &current.id,
                        &gate.gate_id,
                        CodingStageGateStatus::Expired,
                    )?;
                    let snapshot = build_coding_session_state(coding_store, current.clone())?;
                    let _ = event_tx.send(snapshot).await;
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
                        let _ = event_tx.send(snapshot).await;
                        return Ok(Some(current));
                    }
                    CodingRunnerCommand::StageGateConfirm { .. } => {
                        let _ = event_tx
                            .send(CodingWsOutMessage::CodingProtocolError {
                                code: "coding_stage_gate_mismatch".to_string(),
                                message: "stage gate confirm did not match the open stage gate".to_string(),
                            })
                            .await;
                    }
                    CodingRunnerCommand::PermissionResponse { .. }
                    | CodingRunnerCommand::ChoiceResponse { .. } => {}
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
                                    let _ = event_tx
                                        .send(CodingWsOutMessage::CodingProtocolError {
                                            code: "coding_provider_select_failed".to_string(),
                                            message: error.to_string(),
                                        })
                                        .await;
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
                        let _ = event_tx
                            .send(CodingWsOutMessage::CodingProviderConfigUpdated {
                                role: changed_role,
                                provider: changed_provider,
                            })
                            .await;
                        emit_stage_gate(event_tx, coding_store, &current, &gate).await?;
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
                        emit_current_session_state(event_tx, coding_store, &updated).await?;
                        return Ok(None);
                    }
                }
            }
        }
    }
}

async fn emit_stage_gate(
    event_tx: &mpsc::Sender<CodingWsOutMessage>,
    coding_store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
    gate: &CodingStageGateState,
) -> Result<(), CodingWorkspaceEngineError> {
    let _ = event_tx
        .send(CodingWsOutMessage::CodingGateRequired {
            gate: stage_gate_required(gate.clone()),
        })
        .await;
    let snapshot = build_coding_session_state(coding_store, attempt.clone())?;
    let _ = event_tx.send(snapshot).await;
    Ok(())
}

fn stage_gate_expires_at() -> String {
    (Utc::now() + ChronoDuration::seconds(STAGE_GATE_COUNTDOWN_SECONDS as i64)).to_rfc3339()
}

async fn handle_pending_runner_commands(
    command_rx: &mut mpsc::Receiver<CodingRunnerCommand>,
    coding_store: &CodingAttemptStore,
    engine: &CodingWorkspaceEngine,
    event_tx: &mpsc::Sender<CodingWsOutMessage>,
    attempt: &CodingExecutionAttempt,
) -> Result<bool, CodingWorkspaceEngineError> {
    while let Ok(command) = command_rx.try_recv() {
        match command {
            CodingRunnerCommand::AbortAttempt => {
                let updated = engine
                    .handle_abort(&attempt.project_id, &attempt.issue_id, &attempt.id)
                    .await?;
                emit_current_session_state(event_tx, coding_store, &updated).await?;
                return Ok(true);
            }
            CodingRunnerCommand::ProviderSelect { role, provider } => {
                let (updated, changed_role, changed_provider) =
                    update_provider_selection(coding_store, attempt, &role, provider)?;
                let _ = event_tx
                    .send(CodingWsOutMessage::CodingProviderConfigUpdated {
                        role: changed_role,
                        provider: changed_provider,
                    })
                    .await;
                let _ = event_tx
                    .send(build_coding_session_state(coding_store, updated)?)
                    .await;
            }
            CodingRunnerCommand::StageGateConfirm { .. } => {}
            CodingRunnerCommand::PermissionResponse { .. }
            | CodingRunnerCommand::ChoiceResponse { .. } => {}
        }
    }
    Ok(false)
}

fn coding_execution_context(
    app_paths: &ProductAppPaths,
    attempt: &CodingExecutionAttempt,
) -> Result<CodingExecutionContext, ProductStoreError> {
    let lifecycle = LifecycleStore::new(app_paths.clone());
    let sessions = lifecycle.list_workspace_sessions(&attempt.project_id, &attempt.issue_id)?;
    let work_item_session = sessions
        .iter()
        .rev()
        .find(|session| {
            session.entity_id == attempt.work_item_id
                && session.workspace_type == WorkspaceType::WorkItem
                && session.status == WorkspaceSessionStatus::Confirmed
        })
        .or_else(|| {
            sessions.iter().rev().find(|session| {
                session.entity_id == attempt.work_item_id
                    && session.workspace_type == WorkspaceType::WorkItem
            })
        });
    let work_item_markdown = match work_item_session {
        Some(session) => lifecycle
            .list_artifact_versions(&session.id)?
            .into_iter()
            .last()
            .map(|version| version.markdown)
            .and_then(|markdown| select_work_item_markdown(Some(markdown), session))
            .or_else(|| select_work_item_markdown(None, session)),
        None => None,
    };
    let verification_commands = work_item_markdown
        .as_deref()
        .map(planned_test_commands_from_markdown)
        .unwrap_or_default()
        .into_iter()
        .map(|spec| spec.command.join(" "))
        .collect();

    Ok(CodingExecutionContext {
        work_item_markdown,
        verification_commands,
    })
}

fn update_provider_selection(
    coding_store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
    role: &str,
    provider: ProviderName,
) -> Result<(CodingExecutionAttempt, CodingProviderRole, ProviderName), ProductStoreError> {
    let mut snapshot = attempt.provider_config_snapshot.clone();
    let mut role_snapshot = coding_store.get_role_provider_config_snapshot(
        &attempt.project_id,
        &attempt.issue_id,
        &attempt.id,
    )?;
    let changed_provider = provider.clone();
    let changed_role =
        apply_provider_selection_to_snapshots(role, provider, &mut snapshot, &mut role_snapshot)
            .map_err(ProductStoreError::Io)?;
    let updated = coding_store.update_attempt_provider_config_snapshot(
        &attempt.project_id,
        &attempt.issue_id,
        &attempt.id,
        snapshot,
    )?;
    coding_store.update_role_provider_config_snapshot(
        &attempt.project_id,
        &attempt.issue_id,
        &attempt.id,
        role_snapshot,
    )?;
    Ok((updated, changed_role, changed_provider))
}

fn update_provider_permission_mode(
    coding_store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
    role: &str,
    permission_mode: CodingProviderPermissionMode,
) -> Result<(CodingProviderRole, ProviderName), ProductStoreError> {
    let parsed_role = parse_coding_provider_role(role).ok_or_else(|| {
        ProductStoreError::Io(format!("unknown coding role: {role}"))
    })?;
    let mut role_snapshot = coding_store.get_role_provider_config_snapshot(
        &attempt.project_id,
        &attempt.issue_id,
        &attempt.id,
    )?;
    let provider = role_snapshot.provider_for_role(&parsed_role).clone();
    role_snapshot.set_permission_mode_for_role(&parsed_role, permission_mode);
    coding_store.update_role_provider_config_snapshot(
        &attempt.project_id,
        &attempt.issue_id,
        &attempt.id,
        role_snapshot,
    )?;
    Ok((parsed_role, provider))
}

fn provider_selection_targets_current_running_stage(
    attempt: &CodingExecutionAttempt,
    role: &str,
) -> bool {
    if attempt.status != CodingAttemptStatus::Running {
        return false;
    }
    let Some(current_role) = coding_provider_role_for_stage(&attempt.stage) else {
        return false;
    };
    parse_coding_provider_role(role).as_ref() == Some(&current_role)
}

fn confirm_open_stage_gate(
    coding_store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
    stage: &CodingExecutionStage,
) -> Result<Option<CodingStageGateState>, ProductStoreError> {
    let Some(gate) = coding_store
        .list_open_stage_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)?
        .into_iter()
        .find(|gate| gate.stage == *stage)
    else {
        return Ok(None);
    };
    coding_store
        .update_stage_gate_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &gate.gate_id,
            CodingStageGateStatus::Confirmed,
        )
        .map(Some)
}

fn context_note_chat_entry(
    coding_store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
    note: CodingContextNote,
) -> Result<CodingChatEntry, ProductStoreError> {
    let timeline_nodes =
        coding_store.get_timeline_nodes(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
    Ok(CodingChatEntry {
        id: chat_entry_id_for_context_note(&note.id),
        attempt_id: attempt.id.clone(),
        node_id: active_coding_timeline_node_id(&timeline_nodes),
        role: CodingAgentRole::Author,
        entry_type: CodingEntryType::UserMessage,
        content: Some(note.content),
        metadata: Some(serde_json::json!({
            "context_note_id": note.id,
        })),
        created_at: note.created_at,
    })
}

fn chat_entry_id_for_context_note(note_id: &str) -> String {
    note_id.replacen("coding_context_note", "coding_chat_entry", 1)
}

fn latest_assistant_artifact_markdown(session: &WorkspaceSessionRecord) -> Option<String> {
    session
        .messages
        .iter()
        .rev()
        .find(|message| matches!(message.role.as_str(), "assistant" | "provider"))
        .map(|message| extract_artifact_content(&message.content))
        .filter(|content| !content.trim().is_empty())
}

fn select_work_item_markdown(
    version_markdown: Option<String>,
    session: &WorkspaceSessionRecord,
) -> Option<String> {
    match version_markdown {
        Some(markdown) if !planned_test_commands_from_markdown(&markdown).is_empty() => {
            Some(markdown)
        }
        Some(markdown) => latest_assistant_artifact_markdown(session).or(Some(markdown)),
        None => latest_assistant_artifact_markdown(session),
    }
}

fn test_specs_for_attempt(
    attempt: &CodingExecutionAttempt,
    context: &CodingExecutionContext,
) -> Vec<TestCommandSpec> {
    if let Some(markdown) = context.work_item_markdown.as_deref() {
        let planned = planned_test_commands_from_markdown(markdown);
        if !planned.is_empty() {
            return planned;
        }
    }
    attempt
        .worktree_path
        .as_ref()
        .map(discover_test_commands)
        .unwrap_or_default()
}

fn testing_rework_evidence(report: &TestingReport) -> String {
    serde_json::to_string_pretty(report).unwrap_or_else(|_| {
        format!(
            "TestingReport serialization failed; overall_status={:?}",
            report.overall_status
        )
    })
}

fn code_review_rework_evidence(report: &CodeReviewReport) -> String {
    serde_json::to_string_pretty(report).unwrap_or_else(|_| {
        format!(
            "CodeReviewReport serialization failed; verdict={:?}; summary={}",
            report.verdict, report.summary
        )
    })
}

fn internal_pr_review_rework_evidence(review: &InternalPrReview) -> String {
    serde_json::to_string_pretty(review).unwrap_or_else(|_| {
        format!(
            "InternalPrReview serialization failed; verdict={:?}; summary={}",
            review.verdict, review.summary
        )
    })
}

async fn emit_current_session_state(
    event_tx: &mpsc::Sender<CodingWsOutMessage>,
    coding_store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
) -> Result<(), CodingWorkspaceEngineError> {
    let current = coding_store.get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
    let snapshot = build_coding_session_state(coding_store, current)?;
    let _ = event_tx.send(snapshot).await;
    Ok(())
}

fn repository_path_for_attempt(
    app_paths: &ProductAppPaths,
    attempt: &CodingExecutionAttempt,
) -> Result<PathBuf, CodingWorkspaceEngineError> {
    let work_item = LifecycleStore::new(app_paths.clone())
        .list_work_items(&attempt.project_id, &attempt.issue_id)?
        .into_iter()
        .find(|work_item| work_item.id == attempt.work_item_id)
        .ok_or_else(|| ProductStoreError::NotFound {
            kind: "work_item",
            id: attempt.work_item_id.clone(),
        })?;
    RepositoryStore::new(app_paths.clone())
        .list(&attempt.project_id)?
        .into_iter()
        .find(|repository| repository.id == work_item.repository_id)
        .map(|repository| repository.path)
        .ok_or({
            CodingWorkspaceEngineError::Store(ProductStoreError::NotFound {
                kind: "repository",
                id: work_item.repository_id,
            })
        })
}

fn provider_for(
    state: &WebAppState,
    provider_name: &ProviderName,
    kind: &'static str,
) -> Result<Arc<dyn StreamingProviderAdapter>, CodingWorkspaceEngineError> {
    state.provider_registry.get(provider_name).ok_or_else(|| {
        CodingWorkspaceEngineError::Store(ProductStoreError::NotFound {
            kind,
            id: format!("{provider_name:?}"),
        })
    })
}

async fn send_coding_json(socket: &mut CodingWsSender, message: &CodingWsOutMessage) -> bool {
    match serde_json::to_string(message) {
        Ok(json) => socket.send(Message::Text(json.into())).await.is_ok(),
        Err(_) => false,
    }
}

fn build_coding_session_state(
    coding_store: &CodingAttemptStore,
    attempt: CodingExecutionAttempt,
) -> Result<CodingWsOutMessage, ProductStoreError> {
    let execution_context = coding_execution_context(&coding_store.paths(), &attempt)?;
    let timeline_nodes =
        coding_store.get_timeline_nodes(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
    let active_node_id = active_coding_timeline_node_id(&timeline_nodes);
    let testing_report = coding_store
        .list_testing_reports(&attempt.project_id, &attempt.issue_id, &attempt.id)?
        .into_iter()
        .last();
    let code_review_reports = coding_store.list_code_review_reports(
        &attempt.project_id,
        &attempt.issue_id,
        &attempt.id,
    )?;
    let review_request = coding_store
        .list_review_requests(&attempt.project_id, &attempt.issue_id, &attempt.id)?
        .into_iter()
        .last();
    let internal_pr_review = coding_store
        .list_internal_pr_reviews(&attempt.project_id, &attempt.issue_id, &attempt.id)?
        .into_iter()
        .last();
    let mut pending_gates: Vec<CodingGateRequiredModel> = coding_store
        .list_open_stage_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)?
        .into_iter()
        .map(stage_gate_required)
        .collect();
    pending_gates.extend(coding_store.list_open_blocked_gates(
        &attempt.project_id,
        &attempt.issue_id,
        &attempt.id,
    )?);
    let role_provider_config_snapshot = coding_store.get_role_provider_config_snapshot(
        &attempt.project_id,
        &attempt.issue_id,
        &attempt.id,
    )?;
    let chat_entries =
        coding_store.list_chat_entries(&attempt.project_id, &attempt.issue_id, &attempt.id)?;

    Ok(CodingWsOutMessage::CodingSessionState {
        attempt_id: attempt.id,
        status: attempt.status,
        stage: attempt.stage,
        branch_name: attempt.branch_name,
        base_branch: attempt.base_branch,
        worktree_path: attempt.worktree_path,
        rework_count: attempt.rework_count,
        max_auto_rework: attempt.max_auto_rework,
        head_commit: attempt.head_commit,
        pushed_remote: attempt.pushed_remote,
        role_provider_config_snapshot,
        provider_config_snapshot: attempt.provider_config_snapshot,
        chat_entries,
        timeline_nodes,
        active_node_id,
        testing_report: Box::new(testing_report),
        code_review_reports,
        review_request: Box::new(review_request),
        internal_pr_review: Box::new(internal_pr_review),
        pending_gates,
        work_item_markdown: execution_context.work_item_markdown,
        verification_commands: execution_context.verification_commands,
    })
}

fn stage_gate_required(gate: CodingStageGateState) -> CodingGateRequiredModel {
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
    }
}

fn active_coding_timeline_node_id(nodes: &[CodingTimelineNode]) -> Option<String> {
    nodes
        .iter()
        .rev()
        .find(|node| {
            matches!(
                node.status,
                CodingTimelineNodeStatus::Pending
                    | CodingTimelineNodeStatus::Running
                    | CodingTimelineNodeStatus::Blocked
            )
        })
        .map(|node| node.id.clone())
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CodingWsOutMessage {
    CodingSessionState {
        attempt_id: String,
        status: CodingAttemptStatus,
        stage: CodingExecutionStage,
        branch_name: String,
        base_branch: String,
        worktree_path: Option<PathBuf>,
        rework_count: u32,
        max_auto_rework: u32,
        head_commit: Option<String>,
        pushed_remote: Option<String>,
        role_provider_config_snapshot: CodingRoleProviderConfigSnapshot,
        provider_config_snapshot: ProviderConfigSnapshot,
        chat_entries: Vec<CodingChatEntry>,
        timeline_nodes: Vec<CodingTimelineNode>,
        active_node_id: Option<String>,
        testing_report: Box<Option<TestingReport>>,
        code_review_reports: Vec<CodeReviewReport>,
        review_request: Box<Option<ReviewRequest>>,
        internal_pr_review: Box<Option<InternalPrReview>>,
        pending_gates: Vec<CodingGateRequiredModel>,
        work_item_markdown: Option<String>,
        verification_commands: Vec<String>,
    },
    CodingStageChange {
        stage: CodingExecutionStage,
    },
    CodingTimelineNodeCreated {
        node: CodingTimelineNode,
    },
    CodingTimelineNodeUpdated {
        node_id: String,
        status: CodingTimelineNodeStatus,
        summary: Option<String>,
        completed_at: Option<String>,
    },
    CodingExecutionEvent {
        event: WsExecutionEvent,
    },
    CodingPermissionRequest {
        id: String,
        tool_name: String,
        description: String,
        risk_level: WsPermissionRiskLevel,
    },
    CodingChoiceRequest {
        id: String,
        prompt: String,
        options: Vec<ChoiceOption>,
        allow_multiple: bool,
        allow_free_text: bool,
    },
    CodingStreamChunk {
        content: String,
        node_id: Option<String>,
    },
    CodingMessageComplete {
        node_id: Option<String>,
    },
    TestingReportUpdate {
        report: Box<TestingReport>,
    },
    CodeReviewComplete {
        report: Box<CodeReviewReport>,
    },
    ReviewRequestUpdate {
        review_request: Box<ReviewRequest>,
    },
    InternalPrReviewComplete {
        review: Box<InternalPrReview>,
    },
    CodingGateRequired {
        gate: CodingGateRequiredModel,
    },
    CodingChatEntryCreated {
        entry: CodingChatEntry,
    },
    CodingProviderConfigUpdated {
        role: CodingProviderRole,
        provider: ProviderName,
    },
    CodingProtocolError {
        code: String,
        message: String,
    },
    CodingPong,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CodingWsInMessage {
    CodingHello {
        attempt_id: String,
        last_seen_node_id: Option<String>,
    },
    StartCoding,
    ContextNote {
        content: String,
    },
    PermissionResponse {
        id: String,
        approved: bool,
        reason: Option<String>,
    },
    ChoiceResponse {
        id: String,
        selected_option_ids: Vec<String>,
        free_text: Option<String>,
    },
    GateResponse {
        gate_id: String,
        action_id: String,
        extra_context: Option<String>,
    },
    ProviderSelect {
        role: String,
        provider: ProviderName,
    },
    PermissionModeSelect {
        role: String,
        permission_mode: CodingProviderPermissionMode,
    },
    StageGateConfirm {
        stage: CodingExecutionStage,
    },
    FinalConfirm,
    AbortAttempt,
    RequestManualPause,
    CodingPing,
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

#[cfg(test)]
mod tests {
    use crate::product::coding_models::{CodingAttemptStatus, CodingExecutionStage};
    use crate::product::models::{
        ProviderName, WorkspaceMessageRecord, WorkspaceSessionRecord, WorkspaceSessionStatus,
        WorkspaceType,
    };
    use crate::product::test_executor::planned_test_commands_from_markdown;

    use super::{CodingWsInMessage, is_coding_ws_message_allowed, select_work_item_markdown};

    #[test]
    fn falls_back_to_assistant_artifact_when_persisted_markdown_lacks_commands() {
        let session = WorkspaceSessionRecord {
            id: "workspace_session_0001".to_string(),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: "work_item_0001".to_string(),
            workspace_type: WorkspaceType::WorkItem,
            status: WorkspaceSessionStatus::Confirmed,
            author_provider: ProviderName::Codex,
            reviewer_provider: ProviderName::ClaudeCode,
            review_rounds: 1,
            superpowers_enabled: true,
            openspec_enabled: true,
            provider_conversations: Vec::new(),
            messages: vec![WorkspaceMessageRecord {
                role: "assistant".to_string(),
                content: "```artifact\n# Work Item\n\n## 验证命令\n\n```bash\nuv run python -m unittest discover -s tests -v\n```\n```"
                    .to_string(),
                created_at: "2026-05-28T00:00:00Z".to_string(),
            }],
            created_at: "2026-05-28T00:00:00Z".to_string(),
            updated_at: "2026-05-28T00:00:00Z".to_string(),
        };

        let selected = select_work_item_markdown(
            Some("# Work Item\n\n## 验证命令\n\n首选无第三方测试依赖命令：".to_string()),
            &session,
        )
        .expect("selected markdown");

        assert!(selected.contains("uv run python -m unittest discover -s tests -v"));
        assert_eq!(
            planned_test_commands_from_markdown(&selected)[0].command,
            vec![
                "uv", "run", "python", "-m", "unittest", "discover", "-s", "tests", "-v"
            ]
        );
    }

    #[test]
    fn blocked_attempt_allows_gate_response_messages() {
        assert!(is_coding_ws_message_allowed(
            &CodingAttemptStatus::Blocked,
            &CodingExecutionStage::Testing,
            &CodingWsInMessage::GateResponse {
                gate_id: "coding_blocked_gate_0001".to_string(),
                action_id: "retry_test_plan".to_string(),
                extra_context: None,
            },
        ));
        assert!(is_coding_ws_message_allowed(
            &CodingAttemptStatus::Blocked,
            &CodingExecutionStage::Testing,
            &CodingWsInMessage::AbortAttempt,
        ));
    }
}
