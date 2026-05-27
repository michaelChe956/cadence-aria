use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path as AxumPath, State};
use axum::response::IntoResponse;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};

use crate::cross_cutting::streaming_provider::StreamingProviderAdapter;
use crate::product::app_paths::ProductAppPaths;
use crate::product::artifact_extraction::extract_artifact_content;
use crate::product::coding_attempt_store::CodingAttemptStore;
use crate::product::coding_models::{
    CodeReviewReport, CodingAttemptStatus, CodingExecutionAttempt, CodingExecutionStage,
    CodingGateRequired as CodingGateRequiredModel, CodingTimelineNode, CodingTimelineNodeStatus,
    InternalPrReview, PushStatus, ReviewRequest, ReviewVerdict, TestingOverallStatus,
    TestingReport,
};
use crate::product::coding_workspace_engine::{
    CodingExecutionContext, CodingWorkspaceEngine, CodingWorkspaceEngineError,
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
use crate::web::state::WebAppState;
use crate::web::workspace_ws_types::{ProviderConfigSnapshot, WsExecutionEvent};
use tokio::sync::mpsc;

pub async fn coding_ws(
    ws: WebSocketUpgrade,
    AxumPath(attempt_id): AxumPath<String>,
    State(state): State<WebAppState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_coding_socket(socket, attempt_id, state))
        .into_response()
}

async fn handle_coding_socket(mut socket: WebSocket, attempt_id: String, state: WebAppState) {
    let app_paths = ProductAppPaths::new(state.workspace_root.join(".aria"));
    let coding_store = CodingAttemptStore::new(app_paths);
    let attempt = match coding_store.get_attempt_by_id(&attempt_id) {
        Ok(attempt) => attempt,
        Err(error) => {
            let _ = send_coding_json(
                &mut socket,
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
        && !send_coding_json(&mut socket, &snapshot).await
    {
        return;
    }

    while let Some(message) = socket.next().await {
        let Ok(message) = message else {
            break;
        };
        match message {
            Message::Text(text) => {
                let Ok(inbound) = serde_json::from_str::<CodingWsInMessage>(&text) else {
                    let _ = send_coding_json(
                        &mut socket,
                        &CodingWsOutMessage::CodingProtocolError {
                            code: "invalid_coding_ws_message".to_string(),
                            message: "invalid coding websocket message".to_string(),
                        },
                    )
                    .await;
                    continue;
                };
                if inbound == CodingWsInMessage::CodingPing {
                    if !send_coding_json(&mut socket, &CodingWsOutMessage::CodingPong).await {
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
                        &mut socket,
                        &CodingWsOutMessage::CodingProtocolError {
                            code: "coding_message_not_allowed".to_string(),
                            message: "message is not allowed in current coding stage".to_string(),
                        },
                    )
                    .await;
                    continue;
                }
                if inbound == CodingWsInMessage::StartCoding {
                    let (event_tx, mut event_rx) = mpsc::channel(1024);
                    let engine = CodingWorkspaceEngine::new(
                        coding_store.clone(),
                        GitWorkspaceService::new(),
                        event_tx,
                    );
                    match execute_start_coding_flow(
                        &mut socket,
                        &state,
                        &coding_store,
                        &engine,
                        &mut event_rx,
                        &current_attempt,
                    )
                    .await
                    {
                        Ok(true) => {}
                        Ok(false) => {
                            break;
                        }
                        Err(error) => {
                            let _ = send_coding_json(
                                &mut socket,
                                &CodingWsOutMessage::CodingProtocolError {
                                    code: "coding_start_failed".to_string(),
                                    message: error.to_string(),
                                },
                            )
                            .await;
                            continue;
                        }
                    }
                } else if inbound == CodingWsInMessage::FinalConfirm {
                    let (event_tx, mut event_rx) = mpsc::channel(8);
                    let engine = CodingWorkspaceEngine::new(
                        coding_store.clone(),
                        GitWorkspaceService::new(),
                        event_tx,
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
                                &mut socket,
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
                        if !send_coding_json(&mut socket, &event).await {
                            break;
                        }
                    }
                    if let Ok(snapshot) = build_coding_session_state(&coding_store, updated) {
                        let _ = send_coding_json(&mut socket, &snapshot).await;
                    }
                } else if inbound == CodingWsInMessage::AbortAttempt {
                    let (event_tx, mut event_rx) = mpsc::channel(8);
                    let engine = CodingWorkspaceEngine::new(
                        coding_store.clone(),
                        GitWorkspaceService::new(),
                        event_tx,
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
                                &mut socket,
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
                        if !send_coding_json(&mut socket, &event).await {
                            break;
                        }
                    }
                    if let Ok(snapshot) = build_coding_session_state(&coding_store, updated) {
                        let _ = send_coding_json(&mut socket, &snapshot).await;
                    }
                }
            }
            Message::Ping(bytes) => match socket.send(Message::Pong(bytes)).await {
                Ok(()) => {}
                Err(_) => break,
            },
            Message::Close(_) => break,
            _ => {}
        }
    }
}

async fn execute_start_coding_flow(
    socket: &mut WebSocket,
    state: &WebAppState,
    coding_store: &CodingAttemptStore,
    engine: &CodingWorkspaceEngine,
    event_rx: &mut mpsc::Receiver<CodingWsOutMessage>,
    attempt: &CodingExecutionAttempt,
) -> Result<bool, CodingWorkspaceEngineError> {
    let app_paths = ProductAppPaths::new(state.workspace_root.join(".aria"));
    let repo_path = repository_path_for_attempt(&app_paths, attempt)?;
    let author_provider = provider_for(
        state,
        &attempt.provider_config_snapshot.author,
        "coding author provider",
    )?;
    let reviewer_name = attempt
        .provider_config_snapshot
        .reviewer
        .as_ref()
        .unwrap_or(&attempt.provider_config_snapshot.author);
    let reviewer_provider = provider_for(state, reviewer_name, "coding reviewer provider")?;
    let execution_context = coding_execution_context(&app_paths, attempt)?;

    let mut current = match run_engine_step(
        socket,
        event_rx,
        engine.start_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id),
    )
    .await
    {
        Some(result) => result?,
        None => return Ok(false),
    };
    current = match run_engine_step(
        socket,
        event_rx,
        engine.execute_worktree_prepare(&current, &repo_path),
    )
    .await
    {
        Some(result) => result?,
        None => return Ok(false),
    };
    current = match run_engine_step(
        socket,
        event_rx,
        engine.execute_coding(&current, author_provider.as_ref(), &execution_context),
    )
    .await
    {
        Some(result) => result?,
        None => return Ok(false),
    };

    let test_specs = test_specs_for_attempt(&current, &execution_context);
    let testing_report = match run_engine_step(
        socket,
        event_rx,
        engine.execute_testing(&current, &test_specs),
    )
    .await
    {
        Some(result) => result?,
        None => return Ok(false),
    };
    if testing_report.overall_status != TestingOverallStatus::Passed {
        return send_current_session_state(socket, coding_store, attempt).await;
    }

    let review_report = match run_engine_step(
        socket,
        event_rx,
        engine.execute_code_review(&current, reviewer_provider.as_ref()),
    )
    .await
    {
        Some(result) => result?,
        None => return Ok(false),
    };
    if review_report.verdict != ReviewVerdict::Approve {
        return send_current_session_state(socket, coding_store, attempt).await;
    }

    let review_request = match run_engine_step(
        socket,
        event_rx,
        engine.execute_review_request(&current, "origin", "feat: implement work item"),
    )
    .await
    {
        Some(result) => result?,
        None => return Ok(false),
    };
    if review_request.push_status != PushStatus::Pushed {
        return send_current_session_state(socket, coding_store, attempt).await;
    }

    let internal_review = match run_engine_step(
        socket,
        event_rx,
        engine.execute_internal_pr_review(&current, reviewer_provider.as_ref()),
    )
    .await
    {
        Some(result) => result?,
        None => return Ok(false),
    };
    if internal_review.verdict != ReviewVerdict::Approve {
        return send_current_session_state(socket, coding_store, attempt).await;
    }

    send_current_session_state(socket, coding_store, attempt).await
}

fn coding_execution_context(
    app_paths: &ProductAppPaths,
    attempt: &CodingExecutionAttempt,
) -> Result<CodingExecutionContext, CodingWorkspaceEngineError> {
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
            .or_else(|| latest_assistant_artifact_markdown(session)),
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

fn latest_assistant_artifact_markdown(session: &WorkspaceSessionRecord) -> Option<String> {
    session
        .messages
        .iter()
        .rev()
        .find(|message| matches!(message.role.as_str(), "assistant" | "provider"))
        .map(|message| extract_artifact_content(&message.content))
        .filter(|content| !content.trim().is_empty())
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

async fn run_engine_step<T, F>(
    socket: &mut WebSocket,
    event_rx: &mut mpsc::Receiver<CodingWsOutMessage>,
    step: F,
) -> Option<Result<T, CodingWorkspaceEngineError>>
where
    F: Future<Output = Result<T, CodingWorkspaceEngineError>>,
{
    tokio::pin!(step);
    loop {
        tokio::select! {
            result = &mut step => {
                while let Ok(event) = event_rx.try_recv() {
                    if !send_coding_json(socket, &event).await {
                        return None;
                    }
                }
                return Some(result);
            }
            event = event_rx.recv() => {
                let Some(event) = event else {
                    continue;
                };
                if !send_coding_json(socket, &event).await {
                    return None;
                }
            }
        }
    }
}

async fn send_current_session_state(
    socket: &mut WebSocket,
    coding_store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
) -> Result<bool, CodingWorkspaceEngineError> {
    let current = coding_store.get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
    let snapshot = build_coding_session_state(coding_store, current)?;
    Ok(send_coding_json(socket, &snapshot).await)
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

async fn send_coding_json(socket: &mut WebSocket, message: &CodingWsOutMessage) -> bool {
    match serde_json::to_string(message) {
        Ok(json) => socket.send(Message::Text(json.into())).await.is_ok(),
        Err(_) => false,
    }
}

fn build_coding_session_state(
    coding_store: &CodingAttemptStore,
    attempt: CodingExecutionAttempt,
) -> Result<CodingWsOutMessage, ProductStoreError> {
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
        provider_config_snapshot: attempt.provider_config_snapshot,
        timeline_nodes,
        active_node_id,
        testing_report: Box::new(testing_report),
        code_review_reports,
        review_request: Box::new(review_request),
        internal_pr_review: Box::new(internal_pr_review),
        pending_gates: Vec::new(),
    })
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
        provider_config_snapshot: ProviderConfigSnapshot,
        timeline_nodes: Vec<CodingTimelineNode>,
        active_node_id: Option<String>,
        testing_report: Box<Option<TestingReport>>,
        code_review_reports: Vec<CodeReviewReport>,
        review_request: Box<Option<ReviewRequest>>,
        internal_pr_review: Box<Option<InternalPrReview>>,
        pending_gates: Vec<CodingGateRequiredModel>,
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
    GateResponse {
        gate_id: String,
        action_id: String,
        extra_context: Option<String>,
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
                | CodingWsInMessage::AbortAttempt
        ),
        CodingExecutionStage::WorktreePrepare
        | CodingExecutionStage::Testing
        | CodingExecutionStage::CodeReview
        | CodingExecutionStage::ReviewRequest
        | CodingExecutionStage::InternalPrReview => {
            matches!(message, CodingWsInMessage::AbortAttempt)
        }
        CodingExecutionStage::Coding | CodingExecutionStage::Rework => matches!(
            message,
            CodingWsInMessage::ContextNote { .. }
                | CodingWsInMessage::PermissionResponse { .. }
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
