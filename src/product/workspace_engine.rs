use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::cross_cutting::streaming_provider::{
    ProviderCommand, ProviderEvent, ProviderExecutionEvent, ProviderExecutionEventKind,
    ProviderExecutionEventStatus, ProviderPermissionMode, ProviderSession, ProviderStatus,
    RiskLevel, StreamingProviderAdapter, StreamingProviderInput,
};
use crate::product::checkpoint_store::CheckpointStore;
use crate::product::json_store::ProductStoreError;
use crate::product::lifecycle_store::{AppendSpecVersionInput, LifecycleStore};
use crate::product::models::{
    AgentRole, ArtifactRef, LifecycleConfirmationStatus, NodeDetail, PermissionEvent, ProviderName,
    ProviderSnapshot, WorkItemPlanStatus, WorkspaceMessageRecord, WorkspaceSessionRecord,
    WorkspaceSessionStatus, WorkspaceType,
};
use crate::protocol::contracts::{AdapterRole, ProviderType};
use crate::web::workspace_ws_types::{
    ArtifactVersion, HumanConfirmDecision, ProviderConfigSnapshot, ReviewVerdict,
    ReviewVerdictType, TimelineNode, TimelineNodeStatus, TimelineNodeType,
    WorkspaceStage as WsWorkspaceStage, WsCheckpointDto, WsMessageDto, WsOutMessage,
    WsProviderConfig,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceStage {
    PrepareContext,
    Running,
    CrossReview,
    ReviewDecision,
    Revision,
    HumanConfirm,
    Completed,
}

impl WorkspaceStage {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::PrepareContext => "prepare_context",
            Self::Running => "running",
            Self::CrossReview => "cross_review",
            Self::ReviewDecision => "review_decision",
            Self::Revision => "revision",
            Self::HumanConfirm => "human_confirm",
            Self::Completed => "completed",
        }
    }

    pub fn from_stage_name(s: &str) -> Option<Self> {
        match s {
            "prepare_context" => Some(Self::PrepareContext),
            "running" => Some(Self::Running),
            "cross_review" => Some(Self::CrossReview),
            "review_decision" => Some(Self::ReviewDecision),
            "revision" => Some(Self::Revision),
            "human_confirm" => Some(Self::HumanConfirm),
            "completed" => Some(Self::Completed),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SessionMessage {
    pub id: String,
    pub role: String,
    pub content: String,
    pub checkpoint_id: Option<String>,
    pub created_at: String,
}

pub struct WorkspaceSession {
    pub session_id: String,
    pub project_id: String,
    pub issue_id: String,
    pub entity_id: String,
    pub workspace_type: WorkspaceType,
    pub stage: WorkspaceStage,
    pub messages: Vec<SessionMessage>,
    pub artifact: Option<String>,
    pub author_provider: ProviderName,
    pub reviewer_provider: Option<ProviderName>,
    pub review_rounds: u32,
    pub repository_path: Option<PathBuf>,
}

impl WorkspaceSession {
    pub fn from_record(record: WorkspaceSessionRecord) -> Self {
        let artifact = latest_artifact_from_messages(&record.messages);
        Self {
            session_id: record.id,
            project_id: record.project_id,
            issue_id: record.issue_id,
            entity_id: record.entity_id,
            workspace_type: record.workspace_type,
            stage: workspace_stage_for_status(&record.status),
            messages: record
                .messages
                .into_iter()
                .enumerate()
                .map(|(idx, message)| SessionMessage {
                    id: format!("msg_{:03}", idx + 1),
                    role: message.role,
                    content: message.content,
                    checkpoint_id: None,
                    created_at: message.created_at,
                })
                .collect(),
            artifact,
            author_provider: record.author_provider,
            reviewer_provider: Some(record.reviewer_provider),
            review_rounds: record.review_rounds,
            repository_path: None,
        }
    }

    pub fn restore_checkpoint_ids(
        &mut self,
        checkpoints: &[crate::product::checkpoint_store::Checkpoint],
    ) {
        for checkpoint in checkpoints {
            let Some(message_index) = checkpoint.message_index.checked_sub(1) else {
                continue;
            };
            if let Some(message) = self.messages.get_mut(message_index as usize)
                && message.role != "user"
            {
                message.checkpoint_id = Some(checkpoint.id.clone());
            }
        }
    }
}

pub enum EngineEvent {
    StreamChunk {
        role: String,
        content: String,
        node_id: Option<String>,
    },
    MessageComplete {
        message_id: String,
        checkpoint_id: String,
        node_id: Option<String>,
    },
    StageChange {
        stage: String,
    },
    ArtifactUpdate {
        version: u32,
        markdown: String,
    },
    PermissionRequest {
        id: String,
        tool_name: String,
        description: String,
        risk_level: RiskLevel,
    },
    ProviderStatus {
        status: ProviderStatus,
    },
    ExecutionEvent {
        event: ProviderExecutionEvent,
        node_id: Option<String>,
        agent: Option<ProviderName>,
    },
    TimelineNodeCreated {
        node: TimelineNode,
    },
    TimelineNodeUpdated {
        node_id: String,
        status: TimelineNodeStatus,
        summary: Option<String>,
        completed_at: Option<String>,
    },
    ReviewComplete {
        node_id: String,
        round: u32,
        verdict: ReviewVerdictType,
        comments: String,
        summary: String,
    },
    ReviewDecisionRequired {
        node_id: String,
        round: u32,
        options: Vec<String>,
    },
    Error {
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReviewDecisionOutcome {
    StartRevision,
    HumanConfirm,
}

pub struct WorkspaceEngine {
    checkpoint_store: Arc<CheckpointStore>,
    lifecycle_store: Option<LifecycleStore>,
    event_tx: mpsc::Sender<EngineEvent>,
    session: WorkspaceSession,
    cancel: CancellationToken,
    timeline_nodes: Vec<TimelineNode>,
    active_node_id: Option<String>,
    artifact_versions: Vec<ArtifactVersion>,
    latest_review_verdict: Option<ReviewVerdict>,
    pending_revision_context: Option<String>,
    active_run_id: Option<String>,
    stream_buffers: HashMap<String, PendingStreamBuffer>,
}

struct PendingStreamBuffer {
    content: String,
    last_flush_at: Instant,
}

impl Default for PendingStreamBuffer {
    fn default() -> Self {
        Self {
            content: String::new(),
            last_flush_at: Instant::now(),
        }
    }
}

struct TimelineNodeDraft {
    node_type: TimelineNodeType,
    agent: Option<ProviderName>,
    stage: WorkspaceStage,
    round: Option<u32>,
    title: String,
    summary: Option<String>,
    status: TimelineNodeStatus,
}

impl WorkspaceEngine {
    pub fn new(
        checkpoint_store: Arc<CheckpointStore>,
        event_tx: mpsc::Sender<EngineEvent>,
        session: WorkspaceSession,
    ) -> Self {
        let (timeline_nodes, active_node_id) = initial_timeline(&session);
        Self {
            checkpoint_store,
            lifecycle_store: None,
            event_tx,
            session,
            cancel: CancellationToken::new(),
            timeline_nodes,
            active_node_id,
            artifact_versions: Vec::new(),
            latest_review_verdict: None,
            pending_revision_context: None,
            active_run_id: None,
            stream_buffers: HashMap::new(),
        }
    }

    pub fn new_persistent(
        checkpoint_store: Arc<CheckpointStore>,
        lifecycle_store: LifecycleStore,
        event_tx: mpsc::Sender<EngineEvent>,
        mut session: WorkspaceSession,
    ) -> Self {
        let persisted_timeline_nodes = lifecycle_store
            .load_timeline_nodes(&session.session_id)
            .unwrap_or_default();
        let persisted_artifact_versions = lifecycle_store
            .list_artifact_versions(&session.session_id)
            .unwrap_or_default();
        let (timeline_nodes, active_node_id) = if persisted_timeline_nodes.is_empty() {
            initial_timeline(&session)
        } else {
            let active_node_id = active_timeline_node_id(&persisted_timeline_nodes);
            if let Some(stage) = active_node_id
                .as_ref()
                .and_then(|node_id| {
                    persisted_timeline_nodes
                        .iter()
                        .find(|node| &node.node_id == node_id)
                })
                .map(|node| workspace_stage_from_ws_stage(&node.stage))
            {
                session.stage = stage;
            }
            (persisted_timeline_nodes, active_node_id)
        };
        let latest_review_verdict = latest_review_verdict_from_messages(&session.messages);
        Self {
            checkpoint_store,
            lifecycle_store: Some(lifecycle_store),
            event_tx,
            session,
            cancel: CancellationToken::new(),
            timeline_nodes,
            active_node_id,
            artifact_versions: persisted_artifact_versions,
            latest_review_verdict,
            pending_revision_context: None,
            active_run_id: None,
            stream_buffers: HashMap::new(),
        }
    }

    pub fn session(&self) -> &WorkspaceSession {
        &self.session
    }

    pub fn current_stage(&self) -> WorkspaceStage {
        self.session.stage.clone()
    }

    pub fn cancel_token(&self) -> CancellationToken {
        self.cancel.clone()
    }

    pub fn start_new_run_token(&mut self) -> CancellationToken {
        self.cancel = CancellationToken::new();
        self.cancel.clone()
    }

    pub fn use_run_token(&mut self, cancel: CancellationToken) {
        self.cancel = cancel;
    }

    pub fn mark_active_run_started(&mut self, run_id: impl Into<String>) {
        self.active_run_id = Some(run_id.into());
    }

    pub fn mark_active_run_finished(&mut self, run_id: &str) {
        if self.active_run_id.as_deref() == Some(run_id) {
            self.active_run_id = None;
        }
    }

    pub fn active_run_id(&self) -> Option<&str> {
        self.active_run_id.as_deref()
    }

    pub async fn append_context_note(&mut self, content: String) -> Result<TimelineNode, String> {
        Ok(self
            .append_completed_timeline_event(
                TimelineNodeType::ContextNote,
                WorkspaceStage::PrepareContext,
                "上下文补充".to_string(),
                Some(content),
                TimelineNodeStatus::Completed,
                false,
            )
            .await)
    }

    pub async fn start_generation(
        &mut self,
        provider_config: ProviderConfigSnapshot,
        reviewer_enabled: bool,
    ) -> Result<(TimelineNode, WsOutMessage), String> {
        let mut locked_snapshot = provider_config;
        if !reviewer_enabled {
            locked_snapshot.reviewer = None;
            locked_snapshot.review_rounds = 0;
        }

        self.session.author_provider = locked_snapshot.author.clone();
        self.session.reviewer_provider = locked_snapshot.reviewer.clone();
        self.session.review_rounds = locked_snapshot.review_rounds;

        if let Some(store) = &self.lifecycle_store {
            let reviewer_provider = locked_snapshot
                .reviewer
                .clone()
                .unwrap_or_else(|| locked_snapshot.author.clone());
            store
                .update_workspace_session_providers(
                    &self.session.session_id,
                    locked_snapshot.author.clone(),
                    reviewer_provider,
                )
                .map_err(|error| format!("persist provider lock failed: {error}"))?;
            store
                .update_workspace_session_status(
                    &self.session.session_id,
                    WorkspaceSessionStatus::Running,
                )
                .map_err(|error| format!("persist workspace status failed: {error}"))?;
        }

        self.complete_active_node(Some("上下文已确认".to_string()))
            .await;
        let node = self
            .append_completed_timeline_event(
                TimelineNodeType::StartGeneration,
                WorkspaceStage::PrepareContext,
                "开始生成".to_string(),
                None,
                TimelineNodeStatus::Completed,
                true,
            )
            .await;
        self.transition_stage(WorkspaceStage::Running).await;

        let locked = WsOutMessage::ProviderLocked {
            snapshot: locked_snapshot,
            locked_at: chrono::Utc::now().to_rfc3339(),
        };
        Ok((node, locked))
    }

    pub async fn append_aborted_by_disconnect(
        &mut self,
        last_active_run_id: String,
    ) -> Result<TimelineNode, String> {
        if let Some(node_id) = self.active_node_id.clone() {
            self.update_timeline_node(
                &node_id,
                TimelineNodeStatus::Failed,
                Some("连接断开，运行已中止".to_string()),
            )
            .await;
        }
        self.active_run_id = None;
        Ok(self
            .append_completed_timeline_event(
                TimelineNodeType::AbortedByDisconnect,
                WorkspaceStage::PrepareContext,
                "运行因断开中止".to_string(),
                Some(format!("last_active_run_id: {last_active_run_id}")),
                TimelineNodeStatus::Failed,
                true,
            )
            .await)
    }

    pub async fn transition_to_prepare_context_after_disconnect(&mut self) {
        self.active_run_id = None;
        if let Some(store) = &self.lifecycle_store {
            let _ = store.update_workspace_session_status(
                &self.session.session_id,
                WorkspaceSessionStatus::Open,
            );
        }
        self.transition_stage(WorkspaceStage::PrepareContext).await;
    }

    pub async fn buffer_stream_chunk(
        &mut self,
        node_id: &str,
        content: String,
    ) -> Result<(), String> {
        let should_flush = {
            let buffer = self.stream_buffers.entry(node_id.to_string()).or_default();
            buffer.content.push_str(&content);
            buffer.content.len() >= 4096
                || buffer.last_flush_at.elapsed() >= Duration::from_millis(200)
        };

        if should_flush {
            self.flush_stream_buffer(node_id).await?;
        }
        Ok(())
    }

    pub async fn flush_stream_buffer(&mut self, node_id: &str) -> Result<(), String> {
        let Some(buffer) = self.stream_buffers.remove(node_id) else {
            return Ok(());
        };
        if buffer.content.is_empty() {
            return Ok(());
        }

        self.update_node_detail(node_id, |detail| {
            detail.streaming_content.push_str(&buffer.content);
        })
        .await
    }

    pub async fn persist_permission_request(
        &mut self,
        node_id: &str,
        request_id: String,
        request: serde_json::Value,
    ) -> Result<(), String> {
        self.update_node_detail(node_id, |detail| {
            if let Some(event) = detail
                .permission_events
                .iter_mut()
                .find(|event| event.request_id == request_id)
            {
                event.request = request;
                return;
            }

            detail.permission_events.push(PermissionEvent {
                request_id,
                request,
                response: None,
                ts: chrono::Utc::now().to_rfc3339(),
            });
        })
        .await
    }

    pub async fn persist_permission_response(
        &mut self,
        node_id: &str,
        request_id: String,
        response: serde_json::Value,
    ) -> Result<(), String> {
        self.update_node_detail(node_id, |detail| {
            if let Some(event) = detail
                .permission_events
                .iter_mut()
                .find(|event| event.request_id == request_id)
            {
                event.response = Some(response);
            }
        })
        .await
    }

    pub async fn persist_review_verdict(
        &mut self,
        node_id: &str,
        verdict: serde_json::Value,
    ) -> Result<(), String> {
        self.update_node_detail(node_id, |detail| {
            detail.verdict = Some(verdict);
        })
        .await
    }

    pub async fn persist_artifact_ref(
        &mut self,
        node_id: &str,
        artifact_ref: ArtifactRef,
    ) -> Result<(), String> {
        self.update_node_detail(node_id, |detail| {
            detail.artifact_ref = Some(artifact_ref);
        })
        .await
    }

    pub async fn handle_user_message(
        &mut self,
        content: String,
        provider: Arc<dyn StreamingProviderAdapter>,
        command_rx: mpsc::Receiver<ProviderCommand>,
    ) {
        let msg_id = format!("msg_{:03}", self.session.messages.len() + 1);
        let now = chrono::Utc::now().to_rfc3339();

        let user_msg = SessionMessage {
            id: msg_id.clone(),
            role: "user".to_string(),
            content: content.clone(),
            checkpoint_id: None,
            created_at: now.clone(),
        };
        self.session.messages.push(user_msg);
        if let Some(store) = &self.lifecycle_store {
            let _ = store.append_workspace_message(
                &self.session.session_id,
                "user".to_string(),
                content.clone(),
            );
            let _ = store.update_workspace_session_status(
                &self.session.session_id,
                WorkspaceSessionStatus::Running,
            );
        }

        if self.session.stage != WorkspaceStage::Running {
            self.complete_active_node(Some("上下文已确认".to_string()))
                .await;
            self.transition_stage(WorkspaceStage::Running).await;
        }

        let generation_node_id = self
            .create_timeline_node(TimelineNodeDraft {
                node_type: TimelineNodeType::AuthorRun,
                agent: Some(self.session.author_provider.clone()),
                stage: WorkspaceStage::Running,
                round: None,
                title: format!(
                    "{} 生成",
                    workspace_type_title(&self.session.workspace_type)
                ),
                summary: None,
                status: TimelineNodeStatus::Active,
            })
            .await;

        let input = match self.build_streaming_input(&content) {
            Ok(input) => input,
            Err(message) => {
                let _ = self.event_tx.send(EngineEvent::Error { message }).await;
                self.finish_failed_run().await;
                return;
            }
        };

        let session = provider.start(input, self.cancel.clone()).await;
        self.drive_provider_session(
            session,
            command_rx,
            Some(generation_node_id),
            Some(self.session.author_provider.clone()),
        )
        .await;
    }

    async fn drive_provider_session(
        &mut self,
        session: Result<
            ProviderSession,
            crate::cross_cutting::provider_adapter::ProviderAdapterError,
        >,
        mut command_rx: mpsc::Receiver<ProviderCommand>,
        node_id: Option<String>,
        agent: Option<ProviderName>,
    ) {
        let mut session = match session {
            Ok(session) => session,
            Err(error) => {
                let _ = self
                    .event_tx
                    .send(EngineEvent::Error {
                        message: error.details.clone(),
                    })
                    .await;
                self.finish_failed_run().await;
                return;
            }
        };

        let assistant_msg_id = format!("msg_{:03}", self.session.messages.len() + 1);
        let mut full_content = String::new();
        let cancel = self.cancel.clone();
        let mut events_open = true;
        let mut commands_open = true;

        while events_open {
            tokio::select! {
                _ = cancel.cancelled() => {
                    if let Some(node_id) = node_id.as_deref() {
                        let _ = self.flush_stream_buffer(node_id).await;
                    }
                    self.finish_failed_run().await;
                    return;
                }
                command = command_rx.recv(), if commands_open => {
                    match command {
                        Some(ProviderCommand::Abort) => {
                            let _ = session.commands.send(ProviderCommand::Abort).await;
                            cancel.cancel();
                            if let Some(node_id) = node_id.as_deref() {
                                let _ = self.flush_stream_buffer(node_id).await;
                            }
                            self.finish_failed_run().await;
                            return;
                        }
                        Some(ProviderCommand::PermissionResponse {
                            id,
                            approved,
                            reason,
                        }) => {
                            if let Some(node_id) = node_id.as_deref() {
                                let _ = self
                                    .persist_permission_response(
                                        node_id,
                                        id.clone(),
                                        serde_json::json!({
                                            "approved": approved,
                                            "reason": reason.clone(),
                                        }),
                                    )
                                    .await;
                            }
                            if session.commands.send(ProviderCommand::PermissionResponse {
                                id,
                                approved,
                                reason,
                            }).await.is_err() {
                                commands_open = false;
                            }
                        }
                        None => commands_open = false,
                    }
                }
                event = session.events.recv() => {
                    let Some(event) = event else {
                        events_open = false;
                        continue;
                    };

                    match event {
                        ProviderEvent::TextDelta { content } => {
                            if let Some(node_id) = node_id.as_deref() {
                                let _ = self.buffer_stream_chunk(node_id, content.clone()).await;
                            }
                            full_content.push_str(&content);
                            let _ = self
                                .event_tx
                                .send(EngineEvent::StreamChunk {
                                    role: "assistant".to_string(),
                                    content,
                                    node_id: node_id.clone(),
                                })
                                .await;
                        }
                        ProviderEvent::PermissionRequest(request) => {
                            if let Some(node_id) = node_id.as_deref() {
                                let _ = self
                                    .persist_permission_request(
                                        node_id,
                                        request.id.clone(),
                                        serde_json::json!({
                                            "tool_name": request.tool_name.clone(),
                                            "description": request.description.clone(),
                                            "risk_level": risk_level_text(&request.risk_level),
                                        }),
                                    )
                                    .await;
                            }
                            let _ = self
                                .event_tx
                                .send(EngineEvent::ExecutionEvent {
                                    event: ProviderExecutionEvent {
                                        event_id: format!("permission_{}", request.id),
                                        kind: ProviderExecutionEventKind::Command,
                                        status: ProviderExecutionEventStatus::WaitingApproval,
                                        title: "Waiting for permission".to_string(),
                                        detail: Some(request.description.clone()),
                                        command: Some(request.tool_name.clone()),
                                        cwd: self
                                            .session
                                            .repository_path
                                            .as_ref()
                                            .map(|path| path.display().to_string()),
                                        output: None,
                                        exit_code: None,
                                    },
                                    node_id: node_id.clone(),
                                    agent: agent.clone(),
                                })
                                .await;
                            let _ = self
                                .event_tx
                                .send(EngineEvent::PermissionRequest {
                                    id: request.id,
                                    tool_name: request.tool_name,
                                    description: request.description,
                                    risk_level: request.risk_level,
                                })
                                .await;
                        }
                        ProviderEvent::StatusChanged(status) => {
                            let _ = self
                                .event_tx
                                .send(EngineEvent::ProviderStatus { status })
                                .await;
                        }
                        ProviderEvent::Execution(event) => {
                            if let Some(node_id) = node_id.as_deref() {
                                let event_json = execution_event_json(&event);
                                let _ = self
                                    .update_node_detail(node_id, |detail| {
                                        detail.execution_events.push(event_json);
                                    })
                                    .await;
                            }
                            let _ = self
                                .event_tx
                                .send(EngineEvent::ExecutionEvent {
                                    event,
                                    node_id: node_id.clone(),
                                    agent: agent.clone(),
                                })
                                .await;
                        }
                        ProviderEvent::Completed { full_output, .. } => {
                            if let Some(node_id) = node_id.as_deref() {
                                let _ = self.flush_stream_buffer(node_id).await;
                            }
                            self.complete_assistant_message(assistant_msg_id, full_output).await;
                            return;
                        }
                        ProviderEvent::Failed { message } => {
                            let _ = self
                                .event_tx
                                .send(EngineEvent::Error { message })
                                .await;
                            if let Some(node_id) = node_id.as_deref() {
                                let _ = self.flush_stream_buffer(node_id).await;
                            }
                            self.finish_failed_run().await;
                            return;
                        }
                    }
                }
            }
        }

        if cancel.is_cancelled() {
            if let Some(node_id) = node_id.as_deref() {
                let _ = self.flush_stream_buffer(node_id).await;
            }
            self.finish_failed_run().await;
            return;
        }

        if full_content.is_empty() {
            if let Some(node_id) = node_id.as_deref() {
                let _ = self.flush_stream_buffer(node_id).await;
            }
            self.finish_failed_run().await;
        } else {
            if let Some(node_id) = node_id.as_deref() {
                let _ = self.flush_stream_buffer(node_id).await;
            }
            self.complete_assistant_message(assistant_msg_id, full_content)
                .await;
        }
    }

    pub async fn drive_review_session(
        &mut self,
        provider: Arc<dyn StreamingProviderAdapter>,
        command_rx: mpsc::Receiver<ProviderCommand>,
    ) {
        let reviewer = self
            .session
            .reviewer_provider
            .clone()
            .unwrap_or(ProviderName::Codex);
        let input = match self.build_review_input() {
            Ok(input) => input,
            Err(message) => {
                let _ = self.event_tx.send(EngineEvent::Error { message }).await;
                self.finish_failed_run().await;
                return;
            }
        };
        let session = provider.start(input, self.cancel.clone()).await;
        self.drive_reviewer_provider_session(session, command_rx, reviewer)
            .await;
    }

    pub async fn drive_revision_session(
        &mut self,
        provider: Arc<dyn StreamingProviderAdapter>,
        command_rx: mpsc::Receiver<ProviderCommand>,
    ) {
        let author = self.session.author_provider.clone();
        let node_id = self.active_node_id.clone();
        let input = match self.build_revision_input() {
            Ok(input) => input,
            Err(message) => {
                let _ = self.event_tx.send(EngineEvent::Error { message }).await;
                self.finish_failed_run().await;
                return;
            }
        };
        let session = provider.start(input, self.cancel.clone()).await;
        self.drive_provider_session(session, command_rx, node_id, Some(author))
            .await;
    }

    async fn drive_reviewer_provider_session(
        &mut self,
        session: Result<
            ProviderSession,
            crate::cross_cutting::provider_adapter::ProviderAdapterError,
        >,
        mut command_rx: mpsc::Receiver<ProviderCommand>,
        reviewer: ProviderName,
    ) {
        let mut session = match session {
            Ok(session) => session,
            Err(error) => {
                let _ = self
                    .event_tx
                    .send(EngineEvent::Error {
                        message: error.details.clone(),
                    })
                    .await;
                self.finish_failed_run().await;
                return;
            }
        };

        let node_id = self.active_node_id.clone();
        let mut full_content = String::new();
        let cancel = self.cancel.clone();
        let mut events_open = true;
        let mut commands_open = true;

        while events_open {
            tokio::select! {
                _ = cancel.cancelled() => {
                    if let Some(node_id) = node_id.as_deref() {
                        let _ = self.flush_stream_buffer(node_id).await;
                    }
                    self.finish_failed_run().await;
                    return;
                }
                command = command_rx.recv(), if commands_open => {
                    match command {
                        Some(ProviderCommand::Abort) => {
                            let _ = session.commands.send(ProviderCommand::Abort).await;
                            cancel.cancel();
                            if let Some(node_id) = node_id.as_deref() {
                                let _ = self.flush_stream_buffer(node_id).await;
                            }
                            self.finish_failed_run().await;
                            return;
                        }
                        Some(ProviderCommand::PermissionResponse {
                            id,
                            approved,
                            reason,
                        }) => {
                            if let Some(node_id) = node_id.as_deref() {
                                let _ = self
                                    .persist_permission_response(
                                        node_id,
                                        id.clone(),
                                        serde_json::json!({
                                            "approved": approved,
                                            "reason": reason.clone(),
                                        }),
                                    )
                                    .await;
                            }
                            if session
                                .commands
                                .send(ProviderCommand::PermissionResponse {
                                    id,
                                    approved,
                                    reason,
                                })
                                .await
                                .is_err()
                            {
                                commands_open = false;
                            }
                        }
                        None => commands_open = false,
                    }
                }
                event = session.events.recv() => {
                    let Some(event) = event else {
                        events_open = false;
                        continue;
                    };

                    match event {
                        ProviderEvent::TextDelta { content } => {
                            if let Some(node_id) = node_id.as_deref() {
                                let _ = self.buffer_stream_chunk(node_id, content.clone()).await;
                            }
                            full_content.push_str(&content);
                            let _ = self
                                .event_tx
                                .send(EngineEvent::StreamChunk {
                                    role: "reviewer".to_string(),
                                    content,
                                    node_id: node_id.clone(),
                                })
                                .await;
                        }
                        ProviderEvent::PermissionRequest(request) => {
                            if let Some(node_id) = node_id.as_deref() {
                                let _ = self
                                    .persist_permission_request(
                                        node_id,
                                        request.id.clone(),
                                        serde_json::json!({
                                            "tool_name": request.tool_name.clone(),
                                            "description": request.description.clone(),
                                            "risk_level": risk_level_text(&request.risk_level),
                                        }),
                                    )
                                    .await;
                            }
                            let _ = self
                                .event_tx
                                .send(EngineEvent::ExecutionEvent {
                                    event: ProviderExecutionEvent {
                                        event_id: format!("permission_{}", request.id),
                                        kind: ProviderExecutionEventKind::Command,
                                        status: ProviderExecutionEventStatus::WaitingApproval,
                                        title: "Waiting for permission".to_string(),
                                        detail: Some(request.description.clone()),
                                        command: Some(request.tool_name.clone()),
                                        cwd: self
                                            .session
                                            .repository_path
                                            .as_ref()
                                            .map(|path| path.display().to_string()),
                                        output: None,
                                        exit_code: None,
                                    },
                                    node_id: node_id.clone(),
                                    agent: Some(reviewer.clone()),
                                })
                                .await;
                            let _ = self
                                .event_tx
                                .send(EngineEvent::PermissionRequest {
                                    id: request.id,
                                    tool_name: request.tool_name,
                                    description: request.description,
                                    risk_level: request.risk_level,
                                })
                                .await;
                        }
                        ProviderEvent::StatusChanged(status) => {
                            let _ = self
                                .event_tx
                                .send(EngineEvent::ProviderStatus { status })
                                .await;
                        }
                        ProviderEvent::Execution(event) => {
                            if let Some(node_id) = node_id.as_deref() {
                                let event_json = execution_event_json(&event);
                                let _ = self
                                    .update_node_detail(node_id, |detail| {
                                        detail.execution_events.push(event_json);
                                    })
                                    .await;
                            }
                            let _ = self
                                .event_tx
                                .send(EngineEvent::ExecutionEvent {
                                    event,
                                    node_id: node_id.clone(),
                                    agent: Some(reviewer.clone()),
                                })
                                .await;
                        }
                        ProviderEvent::Completed { full_output, .. } => {
                            if let Some(node_id) = node_id.as_deref() {
                                let _ = self.flush_stream_buffer(node_id).await;
                            }
                            self.complete_review(full_output).await;
                            return;
                        }
                        ProviderEvent::Failed { message } => {
                            let _ = self
                                .event_tx
                                .send(EngineEvent::Error { message })
                                .await;
                            if let Some(node_id) = node_id.as_deref() {
                                let _ = self.flush_stream_buffer(node_id).await;
                            }
                            self.finish_failed_run().await;
                            return;
                        }
                    }
                }
            }
        }

        if cancel.is_cancelled() || full_content.is_empty() {
            if let Some(node_id) = node_id.as_deref() {
                let _ = self.flush_stream_buffer(node_id).await;
            }
            self.finish_failed_run().await;
        } else {
            if let Some(node_id) = node_id.as_deref() {
                let _ = self.flush_stream_buffer(node_id).await;
            }
            self.complete_review(full_content).await;
        }
    }

    async fn complete_review(&mut self, output: String) {
        let node_id = self
            .active_node_id
            .clone()
            .unwrap_or_else(|| "review_unknown".to_string());
        let round = self.active_review_round().unwrap_or(1);
        let verdict = Self::parse_review_verdict(&output);
        self.record_review_message(output);
        self.latest_review_verdict = Some(verdict.clone());
        let reviewer = self
            .active_node_agent()
            .or_else(|| self.session.reviewer_provider.clone());
        let _ = self
            .persist_review_verdict(
                &node_id,
                serde_json::json!({
                    "verdict": verdict.verdict.clone(),
                    "comments": verdict.comments.clone(),
                    "summary": verdict.summary.clone(),
                }),
            )
            .await;
        let _ = self
            .event_tx
            .send(EngineEvent::ReviewComplete {
                node_id: node_id.clone(),
                round,
                verdict: verdict.verdict.clone(),
                comments: verdict.comments.clone(),
                summary: verdict.summary.clone(),
            })
            .await;
        self.update_timeline_node(
            &node_id,
            TimelineNodeStatus::Completed,
            Some(verdict.summary.clone()),
        )
        .await;
        self.mark_latest_artifact_reviewed(reviewer, Some(verdict.verdict.clone()));

        if round >= self.session.review_rounds {
            self.enter_human_confirm(Some(verdict.summary)).await;
            return;
        }

        match verdict.verdict {
            ReviewVerdictType::Pass | ReviewVerdictType::NeedsHuman => {
                self.enter_human_confirm(Some(verdict.summary)).await;
            }
            ReviewVerdictType::Revise => {
                self.transition_stage(WorkspaceStage::ReviewDecision).await;
                let decision_node_id = self
                    .create_timeline_node(TimelineNodeDraft {
                        node_type: TimelineNodeType::ReviewDecision,
                        agent: None,
                        stage: WorkspaceStage::ReviewDecision,
                        round: Some(round),
                        title: format!("Review Decision Round {round}"),
                        summary: Some(verdict.summary),
                        status: TimelineNodeStatus::Paused,
                    })
                    .await;
                let _ = self
                    .event_tx
                    .send(EngineEvent::ReviewDecisionRequired {
                        node_id: decision_node_id,
                        round,
                        options: vec![
                            "continue".to_string(),
                            "continue_with_context".to_string(),
                            "human_intervene".to_string(),
                        ],
                    })
                    .await;
            }
        }
    }

    pub async fn handle_review_decision(
        &mut self,
        decision: String,
        extra_context: Option<String>,
    ) -> Result<ReviewDecisionOutcome, String> {
        if self.session.stage != WorkspaceStage::ReviewDecision {
            return Err(
                "review decision is only available during review_decision stage".to_string(),
            );
        }

        let round = self.active_review_round().unwrap_or(1);
        match decision.as_str() {
            "continue" | "continue_with_context" => {
                let normalized_context = if decision == "continue_with_context" {
                    extra_context.and_then(|context| {
                        let trimmed = context.trim().to_string();
                        if trimmed.is_empty() {
                            None
                        } else {
                            Some(trimmed)
                        }
                    })
                } else {
                    None
                };
                self.pending_revision_context = normalized_context;
                self.complete_active_node(Some("已选择返修".to_string()))
                    .await;
                self.transition_stage(WorkspaceStage::Revision).await;
                let _ = self
                    .create_timeline_node(TimelineNodeDraft {
                        node_type: TimelineNodeType::Revision,
                        agent: Some(self.session.author_provider.clone()),
                        stage: WorkspaceStage::Revision,
                        round: Some(round),
                        title: format!("返修 Round {round}"),
                        summary: Some("根据 review 意见返修".to_string()),
                        status: TimelineNodeStatus::Active,
                    })
                    .await;
                Ok(ReviewDecisionOutcome::StartRevision)
            }
            "human_intervene" => {
                self.complete_active_node(Some("转人工介入".to_string()))
                    .await;
                let summary = self
                    .latest_review_verdict
                    .as_ref()
                    .map(|verdict| verdict.summary.clone())
                    .or_else(|| Some("等待人工介入".to_string()));
                self.enter_human_confirm(summary).await;
                Ok(ReviewDecisionOutcome::HumanConfirm)
            }
            _ => Err(format!("unknown review decision: {decision}")),
        }
    }

    pub async fn handle_human_confirm(
        &mut self,
        decision: HumanConfirmDecision,
        payload: Option<serde_json::Value>,
    ) -> Result<ReviewDecisionOutcome, String> {
        if self.session.stage != WorkspaceStage::HumanConfirm {
            return Err("human confirm is only available during human_confirm stage".to_string());
        }

        match decision {
            HumanConfirmDecision::Confirm => {
                self.handle_confirm().await;
                Ok(ReviewDecisionOutcome::HumanConfirm)
            }
            HumanConfirmDecision::RequestChange => {
                let context = human_confirm_payload_description(payload);
                if self.latest_review_verdict.is_none() {
                    self.latest_review_verdict = Some(ReviewVerdict {
                        verdict: ReviewVerdictType::Revise,
                        comments: context
                            .clone()
                            .unwrap_or_else(|| "人工请求修改".to_string()),
                        summary: "人工请求修改".to_string(),
                    });
                }
                self.pending_revision_context = context;
                self.complete_active_node(Some("已请求修改".to_string()))
                    .await;
                self.transition_stage(WorkspaceStage::Revision).await;
                let round = (self
                    .timeline_nodes
                    .iter()
                    .filter(|node| node.node_type == TimelineNodeType::ReviewerRun)
                    .count() as u32)
                    .max(1);
                let _ = self
                    .create_timeline_node(TimelineNodeDraft {
                        node_type: TimelineNodeType::Revision,
                        agent: Some(self.session.author_provider.clone()),
                        stage: WorkspaceStage::Revision,
                        round: Some(round),
                        title: format!("返修 Round {round}"),
                        summary: Some("根据人工反馈返修".to_string()),
                        status: TimelineNodeStatus::Active,
                    })
                    .await;
                Ok(ReviewDecisionOutcome::StartRevision)
            }
            HumanConfirmDecision::Terminate => {
                self.complete_active_node(Some("已终止".to_string())).await;
                if let Some(store) = &self.lifecycle_store {
                    let _ = store.update_workspace_session_status(
                        &self.session.session_id,
                        WorkspaceSessionStatus::Terminated,
                    );
                }
                self.transition_stage(WorkspaceStage::Completed).await;
                let _ = self
                    .create_timeline_node(TimelineNodeDraft {
                        node_type: TimelineNodeType::Completed,
                        agent: None,
                        stage: WorkspaceStage::Completed,
                        round: None,
                        title: "流程终止".to_string(),
                        summary: Some("已终止".to_string()),
                        status: TimelineNodeStatus::Completed,
                    })
                    .await;
                Ok(ReviewDecisionOutcome::HumanConfirm)
            }
        }
    }

    async fn complete_assistant_message(&mut self, assistant_msg_id: String, full_content: String) {
        if self.cancel.is_cancelled() {
            self.finish_failed_run().await;
            return;
        }

        if full_content.is_empty() {
            self.finish_failed_run().await;
            return;
        }

        let assistant_msg = SessionMessage {
            id: assistant_msg_id.clone(),
            role: "assistant".to_string(),
            content: full_content.clone(),
            checkpoint_id: None,
            created_at: chrono::Utc::now().to_rfc3339(),
        };
        self.session.messages.push(assistant_msg);
        if let Some(store) = &self.lifecycle_store {
            let _ = store.append_workspace_message(
                &self.session.session_id,
                "assistant".to_string(),
                full_content.clone(),
            );
            if matches!(
                self.session.workspace_type,
                WorkspaceType::Story | WorkspaceType::Design
            ) {
                let _ = store.append_version(AppendSpecVersionInput {
                    project_id: self.session.project_id.clone(),
                    issue_id: self.session.issue_id.clone(),
                    entity_id: self.session.entity_id.clone(),
                    markdown: full_content.clone(),
                    provider_run_refs: Vec::new(),
                    review_refs: Vec::new(),
                    confirmed_by: None,
                });
            }
        }
        let artifact_markdown = self
            .session
            .messages
            .last()
            .map(|message| message.content.clone())
            .unwrap_or_default();
        self.update_artifact(artifact_markdown.clone()).await;

        let message_index = self.session.messages.len() as u32;
        let artifact_snapshot = self.session.artifact.clone().unwrap_or_default();
        let checkpoint = self.checkpoint_store.create_checkpoint(
            &self.session.session_id,
            message_index,
            &artifact_snapshot,
            WorkspaceStage::HumanConfirm.as_str(),
        );

        let checkpoint_id = match checkpoint {
            Ok(cp) => {
                if let Some(last) = self.session.messages.last_mut() {
                    last.checkpoint_id = Some(cp.id.clone());
                }
                cp.id
            }
            Err(e) => {
                let _ = self
                    .event_tx
                    .send(EngineEvent::Error {
                        message: format!("checkpoint error: {e}"),
                    })
                    .await;
                return;
            }
        };

        let node_id = self.active_node_id.clone();
        let _ = self
            .event_tx
            .send(EngineEvent::MessageComplete {
                message_id: assistant_msg_id,
                checkpoint_id,
                node_id,
            })
            .await;
        self.complete_active_node(Some("生成完成".to_string()))
            .await;
        self.start_review_or_skip().await;
    }

    fn build_streaming_input(&self, user_content: &str) -> Result<StreamingProviderInput, String> {
        let working_dir = match &self.session.repository_path {
            Some(path) => path.clone(),
            None => std::env::current_dir()
                .map_err(|error| format!("working directory error: {error}"))?,
        };

        Ok(StreamingProviderInput {
            provider_type: provider_type_for_name(&self.session.author_provider),
            role: AdapterRole::Orchestrator,
            prompt: self.build_prompt(user_content),
            working_dir,
            session_id: None,
            permission_mode: ProviderPermissionMode::Supervised,
            env_vars: BTreeMap::new(),
            timeout_secs: 300,
        })
    }

    fn build_review_input(&self) -> Result<StreamingProviderInput, String> {
        let working_dir = match &self.session.repository_path {
            Some(path) => path.clone(),
            None => std::env::current_dir()
                .map_err(|error| format!("working directory error: {error}"))?,
        };

        let artifact = self.session.artifact.clone().unwrap_or_default();
        let mut prompt = String::new();
        prompt.push_str("请作为 reviewer 审核当前 Workspace 产物。\n\n");
        prompt.push_str(&format!(
            "Workspace 类型: {}\n",
            workspace_type_title(&self.session.workspace_type)
        ));
        prompt.push_str("会话上下文:\n");
        for msg in &self.session.messages {
            prompt.push_str(&format!("[{}]: {}\n", msg.role, msg.content));
        }
        prompt.push_str("\n当前 Artifact:\n\n");
        prompt.push_str(&artifact);
        prompt.push_str(
            "\n\n请输出审核意见，并在末尾附加 JSON 代码块：\n\
             ```json\n\
             {\"verdict\":\"pass|revise|needs_human\",\"summary\":\"一句话摘要\"}\n\
             ```\n",
        );

        Ok(StreamingProviderInput {
            provider_type: provider_type_for_name(
                &self
                    .session
                    .reviewer_provider
                    .clone()
                    .unwrap_or(ProviderName::Codex),
            ),
            role: AdapterRole::Reviewer,
            prompt,
            working_dir,
            session_id: None,
            permission_mode: ProviderPermissionMode::Supervised,
            env_vars: BTreeMap::new(),
            timeout_secs: 300,
        })
    }

    fn build_revision_input(&self) -> Result<StreamingProviderInput, String> {
        let working_dir = match &self.session.repository_path {
            Some(path) => path.clone(),
            None => std::env::current_dir()
                .map_err(|error| format!("working directory error: {error}"))?,
        };

        let artifact = self.session.artifact.clone().unwrap_or_default();
        let review = self
            .latest_review_verdict
            .as_ref()
            .ok_or_else(|| "review verdict is unavailable for revision".to_string())?;
        let mut prompt = String::new();
        prompt.push_str("请作为 author 返修当前 Workspace 产物。\n\n");
        prompt.push_str(&format!(
            "Workspace 类型: {}\n",
            workspace_type_title(&self.session.workspace_type)
        ));
        prompt.push_str("会话上下文:\n");
        for msg in &self.session.messages {
            prompt.push_str(&format!("[{}]: {}\n", msg.role, msg.content));
        }
        prompt.push_str("\n上一版 Artifact:\n\n");
        prompt.push_str(&artifact);
        prompt.push_str("\n\nReviewer 审核意见:\n\n");
        prompt.push_str(&review.comments);
        prompt.push_str("\n\nReviewer 摘要:\n");
        prompt.push_str(&review.summary);
        if let Some(context) = &self.pending_revision_context {
            prompt.push_str("\n\n用户补充信息:\n");
            prompt.push_str(context);
        }
        prompt.push_str("\n\n请根据以上审核意见修改产物，输出完整更新后的 artifact markdown。\n");

        Ok(StreamingProviderInput {
            provider_type: provider_type_for_name(&self.session.author_provider),
            role: AdapterRole::Orchestrator,
            prompt,
            working_dir,
            session_id: None,
            permission_mode: ProviderPermissionMode::Supervised,
            env_vars: BTreeMap::new(),
            timeout_secs: 300,
        })
    }

    pub async fn handle_rollback(&mut self, checkpoint_id: &str) -> Result<(), String> {
        let target = self
            .checkpoint_store
            .rollback_to(&self.session.session_id, checkpoint_id)
            .map_err(|e| format!("rollback failed: {e}"))?;

        let keep_count = target.message_index as usize;
        self.session.messages.truncate(keep_count);

        if let Some(stage) = WorkspaceStage::from_stage_name(&target.stage)
            && self.session.stage != stage
        {
            self.transition_stage(stage).await;
        }

        if !target.artifact_snapshot.is_empty() {
            self.session.artifact = Some(target.artifact_snapshot);
        } else {
            self.session.artifact = None;
        }
        if let Some(store) = &self.lifecycle_store {
            let _ = store.truncate_workspace_session_messages(
                &self.session.session_id,
                keep_count,
                workspace_status_for_stage(&self.session.stage),
            );
        }

        Ok(())
    }

    pub async fn handle_confirm(&mut self) {
        match self.session.stage {
            WorkspaceStage::HumanConfirm => {
                self.complete_active_node(Some("已确认通过".to_string()))
                    .await;
                self.mark_latest_artifact_confirmed(Some("human".to_string()));
                if let Some(store) = &self.lifecycle_store {
                    let _ = store.update_workspace_session_status(
                        &self.session.session_id,
                        WorkspaceSessionStatus::Confirmed,
                    );
                    let _ = match self.session.workspace_type {
                        WorkspaceType::Story | WorkspaceType::Design => store
                            .update_spec_confirmation_status(
                                &self.session.project_id,
                                &self.session.issue_id,
                                &self.session.entity_id,
                                LifecycleConfirmationStatus::Confirmed,
                            )
                            .map(|_| ()),
                        WorkspaceType::WorkItem => store
                            .update_work_item_plan_status(
                                &self.session.project_id,
                                &self.session.issue_id,
                                &self.session.entity_id,
                                WorkItemPlanStatus::Confirmed,
                            )
                            .map(|_| ()),
                    };
                }
                self.transition_stage(WorkspaceStage::Completed).await;
                let _ = self
                    .create_timeline_node(TimelineNodeDraft {
                        node_type: TimelineNodeType::Completed,
                        agent: None,
                        stage: WorkspaceStage::Completed,
                        round: None,
                        title: "流程完成".to_string(),
                        summary: Some("已确认通过".to_string()),
                        status: TimelineNodeStatus::Completed,
                    })
                    .await;
            }
            WorkspaceStage::Running => {
                self.transition_stage(WorkspaceStage::CrossReview).await;
            }
            _ => {}
        }
    }

    pub fn handle_abort(&mut self) {
        self.cancel.cancel();
    }

    pub fn set_provider(&mut self, role: &str, provider: ProviderName) -> Result<(), String> {
        if self.session.stage != WorkspaceStage::PrepareContext {
            return Err("provider selection is locked after generation starts".to_string());
        }

        match role {
            "author" => {
                self.session.author_provider = provider;
                Ok(())
            }
            "reviewer" => {
                self.session.reviewer_provider = Some(provider);
                Ok(())
            }
            _ => Err(format!("unknown provider role: {role}")),
        }?;

        if let Some(store) = &self.lifecycle_store {
            let reviewer_provider = self
                .session
                .reviewer_provider
                .clone()
                .unwrap_or(ProviderName::Codex);
            store
                .update_workspace_session_providers(
                    &self.session.session_id,
                    self.session.author_provider.clone(),
                    reviewer_provider,
                )
                .map_err(|error| format!("persist provider selection failed: {error}"))?;
        }

        Ok(())
    }

    pub async fn update_artifact(&mut self, markdown: String) {
        self.session.artifact = Some(markdown.clone());
        let version = self.artifact_versions.len() as u32 + 1;
        let source_node_id = self
            .active_node_id
            .clone()
            .unwrap_or_else(|| "timeline_node_unknown".to_string());
        self.artifact_versions.push(ArtifactVersion {
            version,
            markdown,
            generated_by: self.session.author_provider.clone(),
            reviewed_by: None,
            review_verdict: None,
            confirmed_by: None,
            created_at: chrono::Utc::now().to_rfc3339(),
            source_node_id,
        });
        self.persist_artifact_versions();
        let source_node_id = self
            .artifact_versions
            .last()
            .map(|version| version.source_node_id.clone())
            .unwrap_or_else(|| "timeline_node_unknown".to_string());
        let _ = self
            .persist_artifact_ref(
                &source_node_id,
                ArtifactRef {
                    artifact_id: format!("artifact_version_{version:03}"),
                    version,
                },
            )
            .await;
        let _ = self
            .event_tx
            .send(EngineEvent::ArtifactUpdate {
                version,
                markdown: self.session.artifact.clone().unwrap_or_default(),
            })
            .await;
    }

    async fn transition_stage(&mut self, new_stage: WorkspaceStage) {
        self.session.stage = new_stage;
        let _ = self
            .event_tx
            .send(EngineEvent::StageChange {
                stage: self.session.stage.as_str().to_string(),
            })
            .await;
    }

    async fn finish_failed_run(&mut self) {
        if let Some(store) = &self.lifecycle_store {
            let _ = store.update_workspace_session_status(
                &self.session.session_id,
                WorkspaceSessionStatus::Open,
            );
        }
        self.transition_stage(WorkspaceStage::PrepareContext).await;
    }

    fn build_prompt(&self, user_content: &str) -> String {
        let mut prompt = String::new();
        for msg in &self.session.messages {
            prompt.push_str(&format!("[{}]: {}\n", msg.role, msg.content));
        }
        if self
            .session
            .messages
            .last()
            .is_none_or(|message| message.role != "user" || message.content != user_content)
        {
            prompt.push_str(&format!("[user]: {user_content}\n"));
        }
        prompt
    }

    pub fn build_session_state(&self) -> WsOutMessage {
        let messages: Vec<WsMessageDto> = self
            .session
            .messages
            .iter()
            .map(|m| WsMessageDto {
                id: m.id.clone(),
                role: m.role.clone(),
                content: m.content.clone(),
                checkpoint_id: m.checkpoint_id.clone(),
                created_at: m.created_at.clone(),
            })
            .collect();

        let checkpoints: Vec<WsCheckpointDto> = self
            .checkpoint_store
            .list_checkpoints(&self.session.session_id)
            .unwrap_or_default()
            .into_iter()
            .map(|cp| WsCheckpointDto {
                id: cp.id,
                message_index: cp.message_index,
                stage: cp.stage,
                created_at: cp.created_at,
            })
            .collect();

        let timeline_node_details = self
            .lifecycle_store
            .as_ref()
            .and_then(|store| {
                let ids = store.list_node_detail_ids(&self.session.session_id).ok()?;
                Some(
                    ids.into_iter()
                        .filter_map(|id| {
                            store
                                .load_node_detail(&self.session.session_id, &id)
                                .ok()
                                .map(|detail| (id, detail))
                        })
                        .collect::<HashMap<_, _>>(),
                )
            })
            .unwrap_or_default();

        WsOutMessage::SessionState {
            session_id: self.session.session_id.clone(),
            workspace_type: self.session.workspace_type.clone(),
            stage: self.session.stage.as_str().to_string(),
            messages,
            checkpoints,
            artifact: self.session.artifact.clone(),
            providers: WsProviderConfig {
                author: self.session.author_provider.clone(),
                reviewer: self.session.reviewer_provider.clone(),
            },
            timeline_nodes: self.timeline_nodes.clone(),
            active_node_id: self.active_node_id.clone(),
            artifact_versions: self.artifact_versions.clone(),
            timeline_node_details,
            active_run_id: self.active_run_id.clone(),
        }
    }

    async fn start_review_or_skip(&mut self) {
        if self.session.review_rounds == 0 || self.session.reviewer_provider.is_none() {
            self.enter_human_confirm(Some("未启用交叉审核，等待人工确认".to_string()))
                .await;
            return;
        }

        self.transition_stage(WorkspaceStage::CrossReview).await;
        let round = self.next_review_round();
        let reviewer = self
            .session
            .reviewer_provider
            .clone()
            .unwrap_or(ProviderName::Codex);
        let review_node_id = self
            .create_timeline_node(TimelineNodeDraft {
                node_type: TimelineNodeType::ReviewerRun,
                agent: Some(reviewer.clone()),
                stage: WorkspaceStage::CrossReview,
                round: Some(round),
                title: format!("Review Round {round}"),
                summary: None,
                status: TimelineNodeStatus::Active,
            })
            .await;

        if reviewer == ProviderName::Fake {
            self.update_timeline_node(
                &review_node_id,
                TimelineNodeStatus::Skipped,
                Some("未执行真实 review（Fake 快速路径）".to_string()),
            )
            .await;
            self.mark_latest_artifact_reviewed(Some(ProviderName::Fake), None);
            self.enter_human_confirm(Some("等待人工确认".to_string()))
                .await;
        }
    }

    async fn enter_human_confirm(&mut self, summary: Option<String>) {
        self.transition_stage(WorkspaceStage::HumanConfirm).await;
        let _ = self
            .create_timeline_node(TimelineNodeDraft {
                node_type: TimelineNodeType::HumanConfirm,
                agent: None,
                stage: WorkspaceStage::HumanConfirm,
                round: None,
                title: "人工确认".to_string(),
                summary,
                status: TimelineNodeStatus::Active,
            })
            .await;
        if let Some(store) = &self.lifecycle_store {
            let _ = store.update_workspace_session_status(
                &self.session.session_id,
                WorkspaceSessionStatus::WaitingForHuman,
            );
        }
    }

    async fn create_timeline_node(&mut self, draft: TimelineNodeDraft) -> String {
        let node_id = format!("timeline_node_{:03}", self.timeline_nodes.len() + 1);
        let node = TimelineNode {
            node_id: node_id.clone(),
            node_type: draft.node_type,
            agent: draft.agent,
            stage: ws_stage(&draft.stage),
            round: draft.round,
            status: draft.status,
            title: draft.title,
            summary: draft.summary,
            started_at: chrono::Utc::now().to_rfc3339(),
            completed_at: None,
            duration_ms: None,
            artifact_ref: self
                .session
                .artifact
                .as_ref()
                .map(|_| "artifact_current".to_string()),
            provider_config_snapshot: self.provider_config_snapshot(),
        };
        self.timeline_nodes.push(node.clone());
        self.active_node_id = Some(node_id.clone());
        self.persist_timeline_nodes();
        let _ = self
            .event_tx
            .send(EngineEvent::TimelineNodeCreated { node })
            .await;
        node_id
    }

    async fn append_completed_timeline_event(
        &mut self,
        node_type: TimelineNodeType,
        stage: WorkspaceStage,
        title: String,
        summary: Option<String>,
        status: TimelineNodeStatus,
        make_active: bool,
    ) -> TimelineNode {
        let now = chrono::Utc::now().to_rfc3339();
        let node_id = format!("timeline_node_{:03}", self.timeline_nodes.len() + 1);
        let node = TimelineNode {
            node_id: node_id.clone(),
            node_type,
            agent: None,
            stage: ws_stage(&stage),
            round: None,
            status,
            title,
            summary,
            started_at: now.clone(),
            completed_at: Some(now),
            duration_ms: Some(0),
            artifact_ref: self
                .session
                .artifact
                .as_ref()
                .map(|_| "artifact_current".to_string()),
            provider_config_snapshot: self.provider_config_snapshot(),
        };
        self.timeline_nodes.push(node.clone());
        if make_active {
            self.active_node_id = Some(node_id);
        }
        self.persist_timeline_nodes();
        let _ = self
            .event_tx
            .send(EngineEvent::TimelineNodeCreated { node: node.clone() })
            .await;
        node
    }

    fn empty_node_detail_for(&self, node: &TimelineNode) -> NodeDetail {
        NodeDetail {
            node_id: node.node_id.clone(),
            session_id: self.session.session_id.clone(),
            node_type: node.node_type.clone(),
            status: node.status.clone(),
            agent_role: match node.node_type {
                TimelineNodeType::AuthorRun => Some(AgentRole::Author),
                TimelineNodeType::ReviewerRun => Some(AgentRole::Reviewer),
                _ => None,
            },
            provider: node.agent.as_ref().map(|provider| ProviderSnapshot {
                name: provider_name_text(provider).to_string(),
                model: provider_name_text(provider).to_string(),
            }),
            messages: Vec::new(),
            streaming_content: String::new(),
            execution_events: Vec::new(),
            permission_events: Vec::new(),
            verdict: None,
            artifact_ref: None,
            is_revision: node.node_type == TimelineNodeType::AuthorRun
                && node.stage == WsWorkspaceStage::Revision,
            base_artifact_ref: None,
            started_at: node.started_at.clone(),
            ended_at: node.completed_at.clone(),
        }
    }

    async fn update_node_detail<F>(&mut self, node_id: &str, update: F) -> Result<(), String>
    where
        F: FnOnce(&mut NodeDetail),
    {
        let Some(node) = self
            .timeline_nodes
            .iter()
            .find(|node| node.node_id == node_id)
            .cloned()
        else {
            return Err(format!("timeline node not found: {node_id}"));
        };

        let Some(store) = &self.lifecycle_store else {
            return Ok(());
        };

        let mut detail = match store.load_node_detail(&self.session.session_id, node_id) {
            Ok(detail) => detail,
            Err(ProductStoreError::NotFound { .. }) => self.empty_node_detail_for(&node),
            Err(error) => return Err(format!("load node detail failed: {error}")),
        };
        update(&mut detail);
        store
            .save_node_detail(&self.session.session_id, node_id, &detail)
            .map_err(|error| format!("save node detail failed: {error}"))?;
        Ok(())
    }

    async fn complete_active_node(&mut self, summary: Option<String>) {
        let Some(node_id) = self.active_node_id.clone() else {
            return;
        };
        self.update_timeline_node(&node_id, TimelineNodeStatus::Completed, summary)
            .await;
    }

    async fn update_timeline_node(
        &mut self,
        node_id: &str,
        status: TimelineNodeStatus,
        summary: Option<String>,
    ) {
        let completed_at = if matches!(
            status,
            TimelineNodeStatus::Completed
                | TimelineNodeStatus::Failed
                | TimelineNodeStatus::Skipped
        ) {
            Some(chrono::Utc::now().to_rfc3339())
        } else {
            None
        };

        if let Some(node) = self
            .timeline_nodes
            .iter_mut()
            .find(|node| node.node_id == node_id)
        {
            node.status = status.clone();
            if summary.is_some() {
                node.summary = summary.clone();
            }
            if completed_at.is_some() {
                node.completed_at = completed_at.clone();
            }
        }
        self.persist_timeline_nodes();
        let detail_status = status.clone();
        let detail_completed_at = completed_at.clone();
        let _ = self
            .update_node_detail(node_id, |detail| {
                detail.status = detail_status;
                if detail_completed_at.is_some() {
                    detail.ended_at = detail_completed_at;
                }
            })
            .await;

        let _ = self
            .event_tx
            .send(EngineEvent::TimelineNodeUpdated {
                node_id: node_id.to_string(),
                status,
                summary,
                completed_at,
            })
            .await;
    }

    fn provider_config_snapshot(&self) -> ProviderConfigSnapshot {
        ProviderConfigSnapshot {
            author: self.session.author_provider.clone(),
            reviewer: self.session.reviewer_provider.clone(),
            review_rounds: self.session.review_rounds,
        }
    }

    fn next_review_round(&self) -> u32 {
        self.timeline_nodes
            .iter()
            .filter(|node| node.node_type == TimelineNodeType::ReviewerRun)
            .count() as u32
            + 1
    }

    fn active_review_round(&self) -> Option<u32> {
        let active_node_id = self.active_node_id.as_ref()?;
        self.timeline_nodes
            .iter()
            .find(|node| &node.node_id == active_node_id)
            .and_then(|node| node.round)
    }

    fn active_node_agent(&self) -> Option<ProviderName> {
        let active_node_id = self.active_node_id.as_ref()?;
        self.timeline_nodes
            .iter()
            .find(|node| &node.node_id == active_node_id)
            .and_then(|node| node.agent.clone())
    }

    fn record_review_message(&mut self, content: String) {
        let msg_id = format!("msg_{:03}", self.session.messages.len() + 1);
        let now = chrono::Utc::now().to_rfc3339();
        self.session.messages.push(SessionMessage {
            id: msg_id,
            role: "reviewer".to_string(),
            content: content.clone(),
            checkpoint_id: None,
            created_at: now,
        });
        if let Some(store) = &self.lifecycle_store {
            let _ = store.append_workspace_message(
                &self.session.session_id,
                "reviewer".to_string(),
                content,
            );
        }
    }

    fn mark_latest_artifact_reviewed(
        &mut self,
        reviewed_by: Option<ProviderName>,
        review_verdict: Option<ReviewVerdictType>,
    ) {
        if let Some(version) = self.artifact_versions.last_mut() {
            version.reviewed_by = reviewed_by;
            version.review_verdict = review_verdict;
            self.persist_artifact_versions();
        }
    }

    fn mark_latest_artifact_confirmed(&mut self, confirmed_by: Option<String>) {
        if let Some(version) = self.artifact_versions.last_mut() {
            version.confirmed_by = confirmed_by;
            self.persist_artifact_versions();
        }
    }

    fn persist_timeline_nodes(&self) {
        if let Some(store) = &self.lifecycle_store {
            let _ = store.save_timeline_nodes(&self.session.session_id, &self.timeline_nodes);
        }
    }

    fn persist_artifact_versions(&self) {
        if let Some(store) = &self.lifecycle_store {
            let _ = store.save_artifact_versions(&self.session.session_id, &self.artifact_versions);
        }
    }

    fn parse_review_verdict(output: &str) -> ReviewVerdict {
        let trimmed = output.trim();
        let parsed = extract_tail_json(trimmed).and_then(|(comments, json)| {
            parse_review_json(&json).map(|(verdict, summary)| ReviewVerdict {
                verdict,
                comments: comments.trim().to_string(),
                summary,
            })
        });

        parsed.unwrap_or_else(|| ReviewVerdict {
            verdict: ReviewVerdictType::NeedsHuman,
            comments: output.to_string(),
            summary: "需要人工确认".to_string(),
        })
    }
}

fn initial_timeline(session: &WorkspaceSession) -> (Vec<TimelineNode>, Option<String>) {
    if !session.messages.is_empty() {
        return (Vec::new(), None);
    }

    let node = TimelineNode {
        node_id: "timeline_node_001".to_string(),
        node_type: TimelineNodeType::PrepareContext,
        agent: None,
        stage: ws_stage(&WorkspaceStage::PrepareContext),
        round: None,
        status: TimelineNodeStatus::Active,
        title: "准备上下文".to_string(),
        summary: Some("等待补充上下文".to_string()),
        started_at: chrono::Utc::now().to_rfc3339(),
        completed_at: None,
        duration_ms: None,
        artifact_ref: None,
        provider_config_snapshot: ProviderConfigSnapshot {
            author: session.author_provider.clone(),
            reviewer: session.reviewer_provider.clone(),
            review_rounds: session.review_rounds,
        },
    };
    let active_node_id = Some(node.node_id.clone());
    (vec![node], active_node_id)
}

fn active_timeline_node_id(nodes: &[TimelineNode]) -> Option<String> {
    if let Some(node) = nodes.last()
        && node.node_type == TimelineNodeType::Completed
        && node.status == TimelineNodeStatus::Completed
    {
        return Some(node.node_id.clone());
    }

    nodes
        .iter()
        .rev()
        .find(|node| {
            matches!(
                node.status,
                TimelineNodeStatus::Active | TimelineNodeStatus::Paused
            )
        })
        .or_else(|| nodes.last())
        .map(|node| node.node_id.clone())
}

fn workspace_stage_from_ws_stage(stage: &WsWorkspaceStage) -> WorkspaceStage {
    match stage {
        WsWorkspaceStage::PrepareContext => WorkspaceStage::PrepareContext,
        WsWorkspaceStage::Running => WorkspaceStage::Running,
        WsWorkspaceStage::CrossReview => WorkspaceStage::CrossReview,
        WsWorkspaceStage::ReviewDecision => WorkspaceStage::ReviewDecision,
        WsWorkspaceStage::Revision => WorkspaceStage::Revision,
        WsWorkspaceStage::HumanConfirm => WorkspaceStage::HumanConfirm,
        WsWorkspaceStage::Completed => WorkspaceStage::Completed,
    }
}

fn latest_review_verdict_from_messages(messages: &[SessionMessage]) -> Option<ReviewVerdict> {
    messages
        .iter()
        .rev()
        .find(|message| message.role == "reviewer")
        .map(|message| WorkspaceEngine::parse_review_verdict(&message.content))
}

fn extract_tail_json(output: &str) -> Option<(String, String)> {
    if output.starts_with('{') && output.ends_with('}') {
        return Some((String::new(), output.to_string()));
    }

    let end = output.rfind("```")?;
    let before_end = &output[..end];
    let start = before_end.rfind("```")?;
    let comments = output[..start].to_string();
    let mut json = before_end[start + 3..].trim().to_string();
    if let Some(stripped) = json.strip_prefix("json") {
        json = stripped.trim().to_string();
    }
    Some((comments, json))
}

fn parse_review_json(json: &str) -> Option<(ReviewVerdictType, String)> {
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    let verdict = match value.get("verdict")?.as_str()? {
        "pass" => ReviewVerdictType::Pass,
        "revise" => ReviewVerdictType::Revise,
        "needs_human" => ReviewVerdictType::NeedsHuman,
        _ => return None,
    };
    let summary = value
        .get("summary")
        .and_then(|value| value.as_str())
        .unwrap_or(match verdict {
            ReviewVerdictType::Pass => "审核通过",
            ReviewVerdictType::Revise => "需要返修",
            ReviewVerdictType::NeedsHuman => "需要人工确认",
        })
        .to_string();
    Some((verdict, summary))
}

fn human_confirm_payload_description(payload: Option<serde_json::Value>) -> Option<String> {
    let payload = payload?;
    let description = payload.as_str().map(ToString::to_string).or_else(|| {
        payload
            .get("description")
            .and_then(|value| value.as_str())
            .map(ToString::to_string)
    })?;
    let trimmed = description.trim().to_string();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

fn workspace_type_title(workspace_type: &WorkspaceType) -> &'static str {
    match workspace_type {
        WorkspaceType::Story => "Story Spec",
        WorkspaceType::Design => "Design Spec",
        WorkspaceType::WorkItem => "Work Item",
    }
}

fn ws_stage(stage: &WorkspaceStage) -> WsWorkspaceStage {
    match stage {
        WorkspaceStage::PrepareContext => WsWorkspaceStage::PrepareContext,
        WorkspaceStage::Running => WsWorkspaceStage::Running,
        WorkspaceStage::CrossReview => WsWorkspaceStage::CrossReview,
        WorkspaceStage::ReviewDecision => WsWorkspaceStage::ReviewDecision,
        WorkspaceStage::Revision => WsWorkspaceStage::Revision,
        WorkspaceStage::HumanConfirm => WsWorkspaceStage::HumanConfirm,
        WorkspaceStage::Completed => WsWorkspaceStage::Completed,
    }
}

fn provider_type_for_name(provider: &ProviderName) -> ProviderType {
    match provider {
        ProviderName::ClaudeCode => ProviderType::ClaudeCode,
        ProviderName::Codex => ProviderType::Codex,
        ProviderName::Fake => ProviderType::Fake,
    }
}

fn provider_name_text(provider: &ProviderName) -> &'static str {
    match provider {
        ProviderName::ClaudeCode => "claude_code",
        ProviderName::Codex => "codex",
        ProviderName::Fake => "fake",
    }
}

fn risk_level_text(risk_level: &RiskLevel) -> &'static str {
    match risk_level {
        RiskLevel::Low => "low",
        RiskLevel::Medium => "medium",
        RiskLevel::High => "high",
    }
}

fn execution_event_json(event: &ProviderExecutionEvent) -> serde_json::Value {
    serde_json::json!({
        "event_id": event.event_id,
        "kind": execution_event_kind_text(&event.kind),
        "status": execution_event_status_text(&event.status),
        "title": event.title,
        "detail": event.detail,
        "command": event.command,
        "cwd": event.cwd,
        "output": event.output,
        "exit_code": event.exit_code,
    })
}

fn execution_event_kind_text(kind: &ProviderExecutionEventKind) -> &'static str {
    match kind {
        ProviderExecutionEventKind::Provider => "provider",
        ProviderExecutionEventKind::Turn => "turn",
        ProviderExecutionEventKind::Command => "command",
        ProviderExecutionEventKind::Output => "output",
        ProviderExecutionEventKind::Artifact => "artifact",
    }
}

fn execution_event_status_text(status: &ProviderExecutionEventStatus) -> &'static str {
    match status {
        ProviderExecutionEventStatus::Started => "started",
        ProviderExecutionEventStatus::Running => "running",
        ProviderExecutionEventStatus::WaitingApproval => "waiting_approval",
        ProviderExecutionEventStatus::Completed => "completed",
        ProviderExecutionEventStatus::Failed => "failed",
        ProviderExecutionEventStatus::Aborted => "aborted",
    }
}

fn workspace_stage_for_status(status: &WorkspaceSessionStatus) -> WorkspaceStage {
    match status {
        WorkspaceSessionStatus::Open => WorkspaceStage::PrepareContext,
        WorkspaceSessionStatus::Running => WorkspaceStage::Running,
        WorkspaceSessionStatus::WaitingForHuman | WorkspaceSessionStatus::ChangeRequested => {
            WorkspaceStage::HumanConfirm
        }
        WorkspaceSessionStatus::Confirmed => WorkspaceStage::Completed,
        WorkspaceSessionStatus::BlockedProviderUnavailable | WorkspaceSessionStatus::Terminated => {
            WorkspaceStage::Completed
        }
    }
}

fn workspace_status_for_stage(stage: &WorkspaceStage) -> WorkspaceSessionStatus {
    match stage {
        WorkspaceStage::PrepareContext => WorkspaceSessionStatus::Open,
        WorkspaceStage::Running
        | WorkspaceStage::CrossReview
        | WorkspaceStage::ReviewDecision
        | WorkspaceStage::Revision => WorkspaceSessionStatus::Running,
        WorkspaceStage::HumanConfirm => WorkspaceSessionStatus::WaitingForHuman,
        WorkspaceStage::Completed => WorkspaceSessionStatus::Confirmed,
    }
}

fn latest_artifact_from_messages(messages: &[WorkspaceMessageRecord]) -> Option<String> {
    messages
        .iter()
        .rev()
        .find(|message| matches!(message.role.as_str(), "assistant" | "provider"))
        .map(|message| message.content.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cross_cutting::provider_adapter::ProviderAdapterError;
    use crate::cross_cutting::streaming_provider::{
        FakeStreamingProvider, ProviderExecutionEvent, ProviderExecutionEventKind,
        ProviderExecutionEventStatus, StreamChunk,
    };
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::lifecycle_store::CreateWorkspaceSessionInput;
    use crate::product::models::{
        AgentRole, ArtifactRef, NodeDetail, PermissionEvent, ProviderSnapshot,
    };
    use crate::protocol::contracts::{AdapterInput, ProviderType};
    use crate::web::workspace_ws_types::{ReviewVerdictType, TimelineNodeStatus, TimelineNodeType};
    use std::sync::Mutex;
    use tempfile::TempDir;

    fn setup() -> (TempDir, Arc<CheckpointStore>) {
        let tmp = TempDir::new().unwrap();
        let store = Arc::new(CheckpointStore::new(tmp.path().to_path_buf()));
        (tmp, store)
    }

    fn make_session(session_id: &str) -> WorkspaceSession {
        WorkspaceSession {
            session_id: session_id.to_string(),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: "story_spec_0001".to_string(),
            workspace_type: WorkspaceType::Story,
            stage: WorkspaceStage::PrepareContext,
            messages: Vec::new(),
            artifact: None,
            author_provider: ProviderName::ClaudeCode,
            reviewer_provider: Some(ProviderName::Codex),
            review_rounds: 2,
            repository_path: None,
        }
    }

    fn empty_provider_commands() -> mpsc::Receiver<ProviderCommand> {
        let (_tx, rx) = mpsc::channel(8);
        rx
    }

    fn persistent_test_engine() -> (TempDir, LifecycleStore, WorkspaceEngine) {
        let (tmp, checkpoint_store) = setup();
        let lifecycle_store = LifecycleStore::new(ProductAppPaths::new(tmp.path().join(".aria")));
        let (tx, _) = mpsc::channel(64);
        let session_record = lifecycle_store
            .create_workspace_session(CreateWorkspaceSessionInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                entity_id: "story_spec_0001".to_string(),
                workspace_type: WorkspaceType::Story,
                author_provider: ProviderName::ClaudeCode,
                reviewer_provider: ProviderName::Codex,
                review_rounds: 2,
                superpowers_enabled: true,
                openspec_enabled: true,
            })
            .unwrap();
        let session = WorkspaceSession::from_record(session_record);
        let engine =
            WorkspaceEngine::new_persistent(checkpoint_store, lifecycle_store.clone(), tx, session);
        (tmp, lifecycle_store, engine)
    }

    async fn create_author_run_node(engine: &mut WorkspaceEngine) -> String {
        engine
            .create_timeline_node(TimelineNodeDraft {
                node_type: TimelineNodeType::AuthorRun,
                agent: Some(ProviderName::ClaudeCode),
                stage: WorkspaceStage::Running,
                round: None,
                title: "Story 生成".to_string(),
                summary: None,
                status: TimelineNodeStatus::Active,
            })
            .await
    }

    async fn create_reviewer_run_node(engine: &mut WorkspaceEngine) -> String {
        engine
            .create_timeline_node(TimelineNodeDraft {
                node_type: TimelineNodeType::ReviewerRun,
                agent: Some(ProviderName::Codex),
                stage: WorkspaceStage::CrossReview,
                round: Some(1),
                title: "交叉审核 Round 1".to_string(),
                summary: None,
                status: TimelineNodeStatus::Active,
            })
            .await
    }

    #[tokio::test]
    async fn stream_chunk_flushes_after_4kb_or_node_end() {
        let (_tmp, lifecycle_store, mut engine) = persistent_test_engine();
        let node_id = create_author_run_node(&mut engine).await;

        engine
            .buffer_stream_chunk(&node_id, "hello ".to_string())
            .await
            .unwrap();
        engine
            .buffer_stream_chunk(&node_id, "world".to_string())
            .await
            .unwrap();
        assert!(
            lifecycle_store
                .load_node_detail(&engine.session().session_id, &node_id)
                .is_err(),
            "small chunks should stay buffered before explicit flush"
        );

        engine.flush_stream_buffer(&node_id).await.unwrap();

        let detail = lifecycle_store
            .load_node_detail(&engine.session().session_id, &node_id)
            .unwrap();
        assert_eq!(detail.streaming_content, "hello world");

        let large = "x".repeat(4096);
        engine
            .buffer_stream_chunk(&node_id, large.clone())
            .await
            .unwrap();
        let detail = lifecycle_store
            .load_node_detail(&engine.session().session_id, &node_id)
            .unwrap();
        assert!(detail.streaming_content.ends_with(&large));
    }

    #[tokio::test]
    async fn permission_request_and_response_are_persisted_to_node_detail() {
        let (_tmp, lifecycle_store, mut engine) = persistent_test_engine();
        let node_id = create_author_run_node(&mut engine).await;

        engine
            .persist_permission_request(
                &node_id,
                "permission_1".to_string(),
                serde_json::json!({"tool_name": "shell", "description": "cargo test"}),
            )
            .await
            .unwrap();
        engine
            .persist_permission_response(
                &node_id,
                "permission_1".to_string(),
                serde_json::json!({"approved": true, "reason": null}),
            )
            .await
            .unwrap();

        let detail = lifecycle_store
            .load_node_detail(&engine.session().session_id, &node_id)
            .unwrap();
        assert_eq!(detail.permission_events.len(), 1);
        assert_eq!(detail.permission_events[0].request_id, "permission_1");
        assert_eq!(
            detail.permission_events[0].response.as_ref().unwrap()["approved"],
            true
        );
    }

    #[tokio::test]
    async fn verdict_and_artifact_ref_are_persisted_to_node_detail() {
        let (_tmp, lifecycle_store, mut engine) = persistent_test_engine();
        let node_id = create_reviewer_run_node(&mut engine).await;

        engine
            .persist_review_verdict(
                &node_id,
                serde_json::json!({"verdict": "pass", "summary": "ok"}),
            )
            .await
            .unwrap();
        engine
            .persist_artifact_ref(
                &node_id,
                ArtifactRef {
                    artifact_id: "artifact_story_spec_0001".to_string(),
                    version: 2,
                },
            )
            .await
            .unwrap();

        let detail = lifecycle_store
            .load_node_detail(&engine.session().session_id, &node_id)
            .unwrap();
        assert_eq!(detail.verdict.as_ref().unwrap()["verdict"], "pass");
        assert_eq!(detail.artifact_ref.as_ref().unwrap().version, 2);
    }

    #[tokio::test]
    async fn handle_user_message_transitions_from_prepare_to_running() {
        let (_tmp, store) = setup();
        let (tx, mut rx) = mpsc::channel(64);
        let session = make_session("sess_001");
        let mut engine = WorkspaceEngine::new(store, tx, session);

        engine
            .handle_user_message(
                "hello world".to_string(),
                Arc::new(FakeStreamingProvider),
                empty_provider_commands(),
            )
            .await;

        let mut saw_running = false;
        while let Ok(event) = rx.try_recv() {
            if matches!(event, EngineEvent::StageChange { stage } if stage == "running") {
                saw_running = true;
            }
        }
        assert!(saw_running);
        assert_eq!(engine.session().stage, WorkspaceStage::CrossReview);
        assert_eq!(engine.session().messages.len(), 2); // user + assistant
        assert_eq!(engine.session().messages[0].role, "user");
        assert_eq!(engine.session().messages[1].role, "assistant");
        assert!(engine.session().messages[1].checkpoint_id.is_some());

        match engine.build_session_state() {
            WsOutMessage::SessionState {
                timeline_nodes,
                active_node_id,
                ..
            } => {
                assert!(
                    timeline_nodes.iter().any(|node| {
                        node.node_type == TimelineNodeType::AuthorRun
                            && node.status == TimelineNodeStatus::Completed
                    }),
                    "generation node should be completed"
                );
                let active_id = active_node_id.expect("active review node id");
                let active = timeline_nodes
                    .iter()
                    .find(|node| node.node_id == active_id)
                    .expect("active timeline node");
                assert_eq!(active.node_type, TimelineNodeType::ReviewerRun);
                assert_eq!(active.agent, Some(ProviderName::Codex));
                assert_eq!(active.status, TimelineNodeStatus::Active);
            }
            _ => panic!("expected SessionState"),
        }
    }

    #[tokio::test]
    async fn fake_reviewer_creates_skipped_review_node_and_enters_human_confirm() {
        let (_tmp, store) = setup();
        let (tx, _) = mpsc::channel(64);
        let mut session = make_session("sess_fake_review");
        session.reviewer_provider = Some(ProviderName::Fake);
        let mut engine = WorkspaceEngine::new(store, tx, session);

        engine
            .handle_user_message(
                "hello world".to_string(),
                Arc::new(FakeStreamingProvider),
                empty_provider_commands(),
            )
            .await;

        assert_eq!(engine.session().stage, WorkspaceStage::HumanConfirm);
        match engine.build_session_state() {
            WsOutMessage::SessionState { timeline_nodes, .. } => {
                assert!(timeline_nodes.iter().any(|node| {
                    node.node_type == TimelineNodeType::ReviewerRun
                        && node.status == TimelineNodeStatus::Skipped
                        && node.summary.as_deref() == Some("未执行真实 review（Fake 快速路径）")
                }));
            }
            _ => panic!("expected SessionState"),
        }
    }

    #[test]
    fn parse_review_verdict_reads_json_contract_from_tail_block() {
        let output = "整体可用，但需要补充异常路径。\n\n```json\n{\"verdict\":\"revise\",\"summary\":\"补充异常路径\"}\n```";

        let verdict = WorkspaceEngine::parse_review_verdict(output);

        assert_eq!(verdict.verdict, ReviewVerdictType::Revise);
        assert_eq!(verdict.summary, "补充异常路径");
        assert_eq!(verdict.comments.trim(), "整体可用，但需要补充异常路径。");
    }

    #[test]
    fn parse_review_verdict_defaults_to_needs_human_when_contract_missing() {
        let output = "我无法确定是否通过，请人工确认。";

        let verdict = WorkspaceEngine::parse_review_verdict(output);

        assert_eq!(verdict.verdict, ReviewVerdictType::NeedsHuman);
        assert_eq!(verdict.summary, "需要人工确认");
        assert_eq!(verdict.comments, output);
    }

    struct ReviewVerdictStreamingProvider {
        output: &'static str,
        provider_type: Arc<Mutex<Option<ProviderType>>>,
        prompt: Arc<Mutex<Option<String>>>,
    }

    #[async_trait::async_trait]
    impl StreamingProviderAdapter for ReviewVerdictStreamingProvider {
        async fn start(
            &self,
            input: StreamingProviderInput,
            _cancel: CancellationToken,
        ) -> Result<ProviderSession, ProviderAdapterError> {
            *self.provider_type.lock().unwrap() = Some(input.provider_type.clone());
            *self.prompt.lock().unwrap() = Some(input.prompt.clone());
            let (event_tx, event_rx) = mpsc::channel(8);
            let (command_tx, _command_rx) = mpsc::channel(8);
            let output = self.output.to_string();
            tokio::spawn(async move {
                let _ = event_tx
                    .send(ProviderEvent::TextDelta {
                        content: output.clone(),
                    })
                    .await;
                let _ = event_tx
                    .send(ProviderEvent::Completed {
                        full_output: output,
                        provider_session_id: None,
                    })
                    .await;
            });
            Ok(ProviderSession {
                events: event_rx,
                commands: command_tx,
            })
        }

        async fn run_streaming(
            &self,
            _input: &AdapterInput,
            _cancel: CancellationToken,
        ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
            Err(ProviderAdapterError::execution_failed(
                None,
                String::new(),
                "run_streaming is not used by WorkspaceEngine",
                0,
            ))
        }
    }

    #[tokio::test]
    async fn drive_review_session_pass_enters_human_confirm() {
        let (_tmp, store) = setup();
        let (tx, mut rx) = mpsc::channel(64);
        let session = make_session("sess_review_pass");
        let mut engine = WorkspaceEngine::new(store, tx, session);

        engine
            .handle_user_message(
                "start".to_string(),
                Arc::new(FakeStreamingProvider),
                empty_provider_commands(),
            )
            .await;
        assert_eq!(engine.session().stage, WorkspaceStage::CrossReview);

        let provider_type = Arc::new(Mutex::new(None));
        let prompt = Arc::new(Mutex::new(None));
        engine
            .drive_review_session(
                Arc::new(ReviewVerdictStreamingProvider {
                    output: "审核通过。\n\n```json\n{\"verdict\":\"pass\",\"summary\":\"可以确认\"}\n```",
                    provider_type: provider_type.clone(),
                    prompt: prompt.clone(),
                }),
                empty_provider_commands(),
            )
            .await;

        assert_eq!(*provider_type.lock().unwrap(), Some(ProviderType::Codex));
        assert!(
            prompt
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .contains("# Story Spec")
        );
        assert_eq!(engine.session().stage, WorkspaceStage::HumanConfirm);
        match engine.build_session_state() {
            WsOutMessage::SessionState { timeline_nodes, .. } => {
                assert!(timeline_nodes.iter().any(|node| {
                    node.node_type == TimelineNodeType::ReviewerRun
                        && node.status == TimelineNodeStatus::Completed
                        && node.summary.as_deref() == Some("可以确认")
                }));
            }
            _ => panic!("expected SessionState"),
        }

        let mut saw_review_complete = false;
        while let Ok(event) = rx.try_recv() {
            if let EngineEvent::ReviewComplete {
                verdict, summary, ..
            } = event
            {
                assert_eq!(verdict, ReviewVerdictType::Pass);
                assert_eq!(summary, "可以确认");
                saw_review_complete = true;
            }
        }
        assert!(saw_review_complete);
    }

    #[tokio::test]
    async fn drive_review_session_revise_pauses_for_decision() {
        let (_tmp, store) = setup();
        let (tx, _) = mpsc::channel(64);
        let session = make_session("sess_review_revise");
        let mut engine = WorkspaceEngine::new(store, tx, session);

        engine
            .handle_user_message(
                "start".to_string(),
                Arc::new(FakeStreamingProvider),
                empty_provider_commands(),
            )
            .await;

        engine
            .drive_review_session(
                Arc::new(ReviewVerdictStreamingProvider {
                    output:
                        "需要补充失败路径。\n\n```json\n{\"verdict\":\"revise\",\"summary\":\"补充失败路径\"}\n```",
                    provider_type: Arc::new(Mutex::new(None)),
                    prompt: Arc::new(Mutex::new(None)),
                }),
                empty_provider_commands(),
            )
            .await;

        assert_eq!(engine.session().stage, WorkspaceStage::ReviewDecision);
        match engine.build_session_state() {
            WsOutMessage::SessionState {
                timeline_nodes,
                active_node_id,
                ..
            } => {
                let active = timeline_nodes
                    .iter()
                    .find(|node| Some(&node.node_id) == active_node_id.as_ref())
                    .expect("active review decision node");
                assert_eq!(active.node_type, TimelineNodeType::ReviewDecision);
                assert_eq!(active.status, TimelineNodeStatus::Paused);
            }
            _ => panic!("expected SessionState"),
        }
    }

    #[tokio::test]
    async fn review_decision_continue_with_context_runs_revision_and_starts_next_review() {
        let (_tmp, store) = setup();
        let (tx, _) = mpsc::channel(64);
        let session = make_session("sess_review_revision");
        let mut engine = WorkspaceEngine::new(store, tx, session);

        engine
            .handle_user_message(
                "start".to_string(),
                Arc::new(FakeStreamingProvider),
                empty_provider_commands(),
            )
            .await;
        assert!(
            engine
                .session()
                .artifact
                .as_deref()
                .is_some_and(|artifact| artifact.contains("# Story Spec"))
        );

        engine
            .drive_review_session(
                Arc::new(ReviewVerdictStreamingProvider {
                    output:
                        "需要补充失败路径。\n\n```json\n{\"verdict\":\"revise\",\"summary\":\"补充失败路径\"}\n```",
                    provider_type: Arc::new(Mutex::new(None)),
                    prompt: Arc::new(Mutex::new(None)),
                }),
                empty_provider_commands(),
            )
            .await;
        assert_eq!(engine.session().stage, WorkspaceStage::ReviewDecision);

        engine
            .handle_review_decision(
                "continue_with_context".to_string(),
                Some("补充登录错误码".to_string()),
            )
            .await
            .expect("decision should be accepted");
        assert_eq!(engine.session().stage, WorkspaceStage::Revision);

        let revision_provider_type = Arc::new(Mutex::new(None));
        let revision_prompt = Arc::new(Mutex::new(None));
        engine
            .drive_revision_session(
                Arc::new(ReviewVerdictStreamingProvider {
                    output: "# Story Spec\n\n补充失败路径后的版本",
                    provider_type: revision_provider_type.clone(),
                    prompt: revision_prompt.clone(),
                }),
                empty_provider_commands(),
            )
            .await;

        assert_eq!(
            *revision_provider_type.lock().unwrap(),
            Some(ProviderType::ClaudeCode)
        );
        let prompt = revision_prompt
            .lock()
            .unwrap()
            .clone()
            .expect("revision prompt");
        assert!(prompt.contains("# Story Spec"));
        assert!(prompt.contains("需要补充失败路径"));
        assert!(prompt.contains("补充登录错误码"));
        assert!(prompt.contains("请根据以上审核意见修改产物"));
        assert_eq!(
            engine.session().artifact.as_deref(),
            Some("# Story Spec\n\n补充失败路径后的版本")
        );
        assert_eq!(engine.session().stage, WorkspaceStage::CrossReview);
        match engine.build_session_state() {
            WsOutMessage::SessionState {
                timeline_nodes,
                active_node_id,
                ..
            } => {
                assert!(timeline_nodes.iter().any(|node| {
                    node.node_type == TimelineNodeType::Revision
                        && node.status == TimelineNodeStatus::Completed
                        && node.agent == Some(ProviderName::ClaudeCode)
                }));
                let active = timeline_nodes
                    .iter()
                    .find(|node| Some(&node.node_id) == active_node_id.as_ref())
                    .expect("active review node");
                assert_eq!(active.node_type, TimelineNodeType::ReviewerRun);
                assert_eq!(active.round, Some(2));
            }
            _ => panic!("expected SessionState"),
        }
    }

    #[tokio::test]
    async fn handle_rollback_truncates_messages() {
        let (_tmp, store) = setup();
        let (tx, _) = mpsc::channel(64);
        let session = make_session("sess_002");
        let mut engine = WorkspaceEngine::new(store, tx, session);

        engine
            .handle_user_message(
                "first".to_string(),
                Arc::new(FakeStreamingProvider),
                empty_provider_commands(),
            )
            .await;
        engine
            .handle_user_message(
                "second".to_string(),
                Arc::new(FakeStreamingProvider),
                empty_provider_commands(),
            )
            .await;

        assert_eq!(engine.session().messages.len(), 4);

        let cp_id = engine.session().messages[1].checkpoint_id.clone().unwrap();
        engine.handle_rollback(&cp_id).await.unwrap();

        assert_eq!(engine.session().messages.len(), 2);
    }

    #[tokio::test]
    async fn handle_confirm_transitions_stage() {
        let (_tmp, store) = setup();
        let (tx, _) = mpsc::channel(64);
        let mut session = make_session("sess_003");
        session.stage = WorkspaceStage::HumanConfirm;
        let mut engine = WorkspaceEngine::new(store, tx, session);

        engine.handle_confirm().await;
        assert_eq!(engine.session().stage, WorkspaceStage::Completed);
    }

    #[tokio::test]
    async fn handle_confirm_completes_human_confirm_node_before_completed_node() {
        let (_tmp, store) = setup();
        let (tx, _) = mpsc::channel(64);
        let mut session = make_session("sess_confirm_timeline");
        session.reviewer_provider = Some(ProviderName::Fake);
        let mut engine = WorkspaceEngine::new(store, tx, session);

        engine
            .handle_user_message(
                "start".to_string(),
                Arc::new(FakeStreamingProvider),
                empty_provider_commands(),
            )
            .await;
        assert_eq!(engine.session().stage, WorkspaceStage::HumanConfirm);

        engine.handle_confirm().await;

        match engine.build_session_state() {
            WsOutMessage::SessionState {
                timeline_nodes,
                active_node_id,
                stage,
                ..
            } => {
                assert_eq!(stage, "completed");
                assert!(timeline_nodes.iter().any(|node| {
                    node.node_type == TimelineNodeType::HumanConfirm
                        && node.status == TimelineNodeStatus::Completed
                }));
                let active = timeline_nodes
                    .iter()
                    .find(|node| Some(&node.node_id) == active_node_id.as_ref())
                    .expect("active completed node");
                assert_eq!(active.node_type, TimelineNodeType::Completed);
                assert_eq!(active.status, TimelineNodeStatus::Completed);
                assert_eq!(
                    active_timeline_node_id(&timeline_nodes).as_deref(),
                    active_node_id.as_deref()
                );
            }
            _ => panic!("expected SessionState"),
        }
    }

    #[test]
    fn active_timeline_node_id_prefers_terminal_completed_node_over_stale_active_node() {
        let session = make_session("sess_stale_timeline");
        let provider_config_snapshot = ProviderConfigSnapshot {
            author: session.author_provider.clone(),
            reviewer: session.reviewer_provider.clone(),
            review_rounds: session.review_rounds,
        };
        let stale_human_confirm = TimelineNode {
            node_id: "timeline_node_001".to_string(),
            node_type: TimelineNodeType::HumanConfirm,
            agent: None,
            stage: WsWorkspaceStage::HumanConfirm,
            round: None,
            status: TimelineNodeStatus::Active,
            title: "人工确认".to_string(),
            summary: Some("等待人工确认".to_string()),
            started_at: "2026-05-19T00:00:00Z".to_string(),
            completed_at: None,
            duration_ms: None,
            artifact_ref: Some("artifact_current".to_string()),
            provider_config_snapshot: provider_config_snapshot.clone(),
        };
        let completed = TimelineNode {
            node_id: "timeline_node_002".to_string(),
            node_type: TimelineNodeType::Completed,
            agent: None,
            stage: WsWorkspaceStage::Completed,
            round: None,
            status: TimelineNodeStatus::Completed,
            title: "流程完成".to_string(),
            summary: Some("已确认通过".to_string()),
            started_at: "2026-05-19T00:01:00Z".to_string(),
            completed_at: None,
            duration_ms: None,
            artifact_ref: Some("artifact_current".to_string()),
            provider_config_snapshot,
        };

        assert_eq!(
            active_timeline_node_id(&[stale_human_confirm, completed]).as_deref(),
            Some("timeline_node_002")
        );
    }

    #[tokio::test]
    async fn build_session_state_returns_correct_structure() {
        let (_tmp, store) = setup();
        let (tx, _) = mpsc::channel(64);
        let session = make_session("sess_004");
        let engine = WorkspaceEngine::new(store, tx, session);

        let state = engine.build_session_state();
        match state {
            WsOutMessage::SessionState {
                session_id, stage, ..
            } => {
                assert_eq!(session_id, "sess_004");
                assert_eq!(stage, "prepare_context");
            }
            _ => panic!("expected SessionState"),
        }
    }

    #[tokio::test]
    async fn build_session_state_includes_node_details_and_active_run_id() {
        let (tmp, checkpoint_store) = setup();
        let lifecycle_store = LifecycleStore::new(ProductAppPaths::new(tmp.path().join(".aria")));
        let (tx, _) = mpsc::channel(64);
        let session_record = lifecycle_store
            .create_workspace_session(CreateWorkspaceSessionInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                entity_id: "story_spec_0001".to_string(),
                workspace_type: WorkspaceType::Story,
                author_provider: ProviderName::ClaudeCode,
                reviewer_provider: ProviderName::Codex,
                review_rounds: 2,
                superpowers_enabled: true,
                openspec_enabled: true,
            })
            .unwrap();
        let session = WorkspaceSession::from_record(session_record);
        let session_id = session.session_id.clone();
        let mut engine =
            WorkspaceEngine::new_persistent(checkpoint_store, lifecycle_store.clone(), tx, session);
        engine.timeline_nodes.push(TimelineNode {
            node_id: "node-1".to_string(),
            node_type: TimelineNodeType::AuthorRun,
            agent: Some(ProviderName::ClaudeCode),
            stage: WsWorkspaceStage::Completed,
            round: None,
            status: TimelineNodeStatus::Completed,
            title: "生成".to_string(),
            summary: None,
            started_at: "2026-05-20T14:30:00Z".to_string(),
            completed_at: Some("2026-05-20T14:35:00Z".to_string()),
            duration_ms: Some(300000),
            artifact_ref: None,
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::ClaudeCode,
                reviewer: None,
                review_rounds: 0,
            },
        });
        let detail = NodeDetail {
            node_id: "node-1".to_string(),
            session_id: session_id.clone(),
            node_type: TimelineNodeType::AuthorRun,
            status: TimelineNodeStatus::Completed,
            agent_role: Some(AgentRole::Author),
            provider: Some(ProviderSnapshot {
                name: "claude_code".to_string(),
                model: "claude-opus-4-7".to_string(),
            }),
            messages: vec![],
            streaming_content: "生成内容".to_string(),
            execution_events: vec![],
            permission_events: vec![PermissionEvent {
                request_id: "perm-1".to_string(),
                request: serde_json::json!({"tool": "shell"}),
                response: Some(serde_json::json!({"approved": true})),
                ts: "2026-05-20T14:31:00Z".to_string(),
            }],
            verdict: None,
            artifact_ref: Some(ArtifactRef {
                artifact_id: "artifact-1".to_string(),
                version: 2,
            }),
            is_revision: false,
            base_artifact_ref: None,
            started_at: "2026-05-20T14:30:00Z".to_string(),
            ended_at: Some("2026-05-20T14:35:00Z".to_string()),
        };
        lifecycle_store
            .save_node_detail(&session_id, "node-1", &detail)
            .unwrap();
        engine.mark_active_run_started("run-1");

        let state = engine.build_session_state();
        match state {
            WsOutMessage::SessionState {
                timeline_node_details,
                active_run_id,
                ..
            } => {
                assert_eq!(
                    timeline_node_details
                        .get("node-1")
                        .map(|detail| detail.streaming_content.as_str()),
                    Some("生成内容")
                );
                assert_eq!(
                    timeline_node_details
                        .get("node-1")
                        .and_then(|detail| detail.artifact_ref.as_ref())
                        .map(|artifact| artifact.version),
                    Some(2)
                );
                assert_eq!(active_run_id.as_deref(), Some("run-1"));
            }
            _ => panic!("expected SessionState"),
        }
    }

    #[tokio::test]
    async fn append_context_note_creates_timeline_node() {
        let (_tmp, store) = setup();
        let (tx, _) = mpsc::channel(64);
        let session = make_session("sess_context_note");
        let mut engine = WorkspaceEngine::new(store, tx, session);

        let node = engine
            .append_context_note("补充上下文".to_string())
            .await
            .unwrap();

        assert_eq!(node.node_type, TimelineNodeType::ContextNote);
        assert_eq!(node.status, TimelineNodeStatus::Completed);
        assert_eq!(node.summary.as_deref(), Some("补充上下文"));
        assert!(
            engine
                .timeline_nodes
                .iter()
                .any(|candidate| candidate.node_id == node.node_id)
        );
    }

    #[tokio::test]
    async fn start_generation_locks_provider_and_creates_node() {
        let (_tmp, store) = setup();
        let (tx, _) = mpsc::channel(64);
        let session = make_session("sess_start_generation");
        let mut engine = WorkspaceEngine::new(store, tx, session);
        let snapshot = ProviderConfigSnapshot {
            author: ProviderName::Codex,
            reviewer: Some(ProviderName::ClaudeCode),
            review_rounds: 1,
        };

        let (node, locked) = engine
            .start_generation(snapshot.clone(), true)
            .await
            .unwrap();

        assert_eq!(node.node_type, TimelineNodeType::StartGeneration);
        assert_eq!(node.status, TimelineNodeStatus::Completed);
        assert_eq!(engine.session().stage, WorkspaceStage::Running);
        assert_eq!(engine.session().author_provider, ProviderName::Codex);
        assert_eq!(
            engine.session().reviewer_provider,
            Some(ProviderName::ClaudeCode)
        );
        assert_eq!(engine.session().review_rounds, 1);
        match locked {
            WsOutMessage::ProviderLocked {
                snapshot: locked_snapshot,
                locked_at,
            } => {
                assert_eq!(locked_snapshot, snapshot);
                assert!(!locked_at.is_empty());
            }
            _ => panic!("expected ProviderLocked"),
        }
    }

    #[tokio::test]
    async fn reviewer_disabled_enters_human_confirm_without_review_node() {
        let (_tmp, store) = setup();
        let (tx, _) = mpsc::channel(64);
        let mut session = make_session("sess_reviewer_disabled");
        session.stage = WorkspaceStage::Running;
        session.reviewer_provider = None;
        session.review_rounds = 0;
        let mut engine = WorkspaceEngine::new(store, tx, session);

        engine.start_review_or_skip().await;

        assert_eq!(engine.session().stage, WorkspaceStage::HumanConfirm);
        assert!(
            !engine
                .timeline_nodes
                .iter()
                .any(|node| node.node_type == TimelineNodeType::ReviewerRun)
        );
    }

    #[tokio::test]
    async fn append_aborted_by_disconnect_creates_node() {
        let (_tmp, store) = setup();
        let (tx, _) = mpsc::channel(64);
        let session = make_session("sess_disconnect_abort");
        let mut engine = WorkspaceEngine::new(store, tx, session);

        let node = engine
            .append_aborted_by_disconnect("run-1".to_string())
            .await
            .unwrap();

        assert_eq!(node.node_type, TimelineNodeType::AbortedByDisconnect);
        assert_eq!(node.status, TimelineNodeStatus::Failed);
        assert!(
            node.summary
                .as_deref()
                .is_some_and(|summary| summary.contains("run-1"))
        );
    }

    #[tokio::test]
    async fn handle_human_confirm_request_change_starts_revision() {
        let (_tmp, store) = setup();
        let (tx, _) = mpsc::channel(64);
        let session = make_session("sess_human_request_change");
        let mut engine = WorkspaceEngine::new(store, tx, session);
        engine.latest_review_verdict = Some(ReviewVerdict {
            verdict: ReviewVerdictType::NeedsHuman,
            comments: "需要人工判断".to_string(),
            summary: "等待人工确认".to_string(),
        });
        engine
            .enter_human_confirm(Some("等待人工确认".to_string()))
            .await;

        let outcome = engine
            .handle_human_confirm(
                HumanConfirmDecision::RequestChange,
                Some(serde_json::json!({"description": "补充边界条件"})),
            )
            .await
            .unwrap();

        assert_eq!(outcome, ReviewDecisionOutcome::StartRevision);
        assert_eq!(engine.session().stage, WorkspaceStage::Revision);
        assert!(engine.timeline_nodes.iter().any(|node| {
            node.node_type == TimelineNodeType::Revision
                && node.status == TimelineNodeStatus::Active
                && node.summary.as_deref() == Some("根据人工反馈返修")
        }));
    }

    #[tokio::test]
    async fn set_provider_updates_author_and_reviewer() {
        let (_tmp, store) = setup();
        let (tx, _) = mpsc::channel(64);
        let session = make_session("sess_005");
        let mut engine = WorkspaceEngine::new(store, tx, session);

        assert_eq!(engine.session().author_provider, ProviderName::ClaudeCode);
        assert_eq!(
            engine.session().reviewer_provider,
            Some(ProviderName::Codex)
        );

        engine.set_provider("author", ProviderName::Codex).unwrap();
        assert_eq!(engine.session().author_provider, ProviderName::Codex);

        engine
            .set_provider("reviewer", ProviderName::ClaudeCode)
            .unwrap();
        assert_eq!(
            engine.session().reviewer_provider,
            Some(ProviderName::ClaudeCode)
        );

        let err = engine.set_provider("unknown", ProviderName::Fake);
        assert!(err.is_err());
    }

    struct RecordingStreamingProvider {
        provider_type: Arc<Mutex<Option<ProviderType>>>,
    }

    #[async_trait::async_trait]
    impl StreamingProviderAdapter for RecordingStreamingProvider {
        async fn start(
            &self,
            input: StreamingProviderInput,
            _cancel: CancellationToken,
        ) -> Result<ProviderSession, ProviderAdapterError> {
            *self.provider_type.lock().unwrap() = Some(input.provider_type.clone());
            let (event_tx, event_rx) = mpsc::channel(8);
            let (command_tx, _command_rx) = mpsc::channel(8);
            tokio::spawn(async move {
                let _ = event_tx
                    .send(ProviderEvent::TextDelta {
                        content: "# Draft".to_string(),
                    })
                    .await;
                let _ = event_tx
                    .send(ProviderEvent::Completed {
                        full_output: "# Draft".to_string(),
                        provider_session_id: None,
                    })
                    .await;
            });
            Ok(ProviderSession {
                events: event_rx,
                commands: command_tx,
            })
        }

        async fn run_streaming(
            &self,
            _input: &AdapterInput,
            _cancel: CancellationToken,
        ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
            Err(ProviderAdapterError::execution_failed(
                None,
                String::new(),
                "run_streaming is not used by WorkspaceEngine",
                0,
            ))
        }
    }

    #[tokio::test]
    async fn handle_user_message_uses_author_provider_and_publishes_artifact_for_confirmation() {
        let (_tmp, store) = setup();
        let (tx, mut rx) = mpsc::channel(64);
        let mut session = make_session("sess_006");
        session.author_provider = ProviderName::Codex;
        let mut engine = WorkspaceEngine::new(store, tx, session);
        let provider_type = Arc::new(Mutex::new(None));
        let provider = RecordingStreamingProvider {
            provider_type: provider_type.clone(),
        };

        engine
            .handle_user_message(
                "start".to_string(),
                Arc::new(provider),
                empty_provider_commands(),
            )
            .await;

        assert_eq!(*provider_type.lock().unwrap(), Some(ProviderType::Codex));
        assert_eq!(engine.session().stage, WorkspaceStage::CrossReview);
        assert_eq!(engine.session().artifact.as_deref(), Some("# Draft"));

        let mut saw_artifact = false;
        let mut saw_cross_review = false;
        while let Ok(event) = rx.try_recv() {
            match event {
                EngineEvent::ArtifactUpdate { markdown, .. } if markdown == "# Draft" => {
                    saw_artifact = true;
                }
                EngineEvent::StageChange { stage } if stage == "cross_review" => {
                    saw_cross_review = true;
                }
                _ => {}
            }
        }
        assert!(
            saw_artifact,
            "provider completion should update the artifact pane"
        );
        assert!(
            saw_cross_review,
            "provider completion should start cross review"
        );
    }

    struct ExecutionEventStreamingProvider;

    #[async_trait::async_trait]
    impl StreamingProviderAdapter for ExecutionEventStreamingProvider {
        async fn start(
            &self,
            _input: StreamingProviderInput,
            _cancel: CancellationToken,
        ) -> Result<ProviderSession, ProviderAdapterError> {
            let (event_tx, event_rx) = mpsc::channel(8);
            let (command_tx, _command_rx) = mpsc::channel(8);
            tokio::spawn(async move {
                let _ = event_tx
                    .send(ProviderEvent::Execution(ProviderExecutionEvent {
                        event_id: "command_cmd_001".to_string(),
                        kind: ProviderExecutionEventKind::Command,
                        status: ProviderExecutionEventStatus::Completed,
                        title: "Command completed".to_string(),
                        detail: Some("exit code 0".to_string()),
                        command: Some("pwd".to_string()),
                        cwd: Some("/tmp/repo".to_string()),
                        output: Some("/tmp/repo\n".to_string()),
                        exit_code: Some(0),
                    }))
                    .await;
                let _ = event_tx
                    .send(ProviderEvent::Completed {
                        full_output: "# Draft".to_string(),
                        provider_session_id: None,
                    })
                    .await;
            });
            Ok(ProviderSession {
                events: event_rx,
                commands: command_tx,
            })
        }

        async fn run_streaming(
            &self,
            _input: &AdapterInput,
            _cancel: CancellationToken,
        ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
            Err(ProviderAdapterError::execution_failed(
                None,
                String::new(),
                "run_streaming is not used by WorkspaceEngine",
                0,
            ))
        }
    }

    #[tokio::test]
    async fn handle_user_message_forwards_provider_execution_events() {
        let (_tmp, store) = setup();
        let (tx, mut rx) = mpsc::channel(64);
        let session = make_session("sess_007_exec");
        let mut engine = WorkspaceEngine::new(store, tx, session);

        engine
            .handle_user_message(
                "start".to_string(),
                Arc::new(ExecutionEventStreamingProvider),
                empty_provider_commands(),
            )
            .await;

        let mut saw_execution_event = false;
        while let Ok(event) = rx.try_recv() {
            if let EngineEvent::ExecutionEvent { event, .. } = event {
                assert_eq!(event.event_id, "command_cmd_001");
                assert_eq!(event.kind, ProviderExecutionEventKind::Command);
                assert_eq!(event.status, ProviderExecutionEventStatus::Completed);
                assert_eq!(event.command.as_deref(), Some("pwd"));
                assert_eq!(event.output.as_deref(), Some("/tmp/repo\n"));
                saw_execution_event = true;
            }
        }

        assert!(
            saw_execution_event,
            "provider execution events should be forwarded to websocket layer"
        );
    }

    #[tokio::test]
    async fn handle_user_message_from_human_confirm_reenters_running_stage() {
        let (_tmp, store) = setup();
        let (tx, mut rx) = mpsc::channel(64);
        let mut session = make_session("sess_007");
        session.stage = WorkspaceStage::HumanConfirm;
        let mut engine = WorkspaceEngine::new(store, tx, session);
        engine
            .handle_user_message(
                "revise".to_string(),
                Arc::new(FakeStreamingProvider),
                empty_provider_commands(),
            )
            .await;

        let mut saw_running = false;
        while let Ok(event) = rx.try_recv() {
            if matches!(event, EngineEvent::StageChange { stage } if stage == "running") {
                saw_running = true;
            }
        }
        assert!(
            saw_running,
            "manual intervention should restart the run stage"
        );
        assert_eq!(engine.session().stage, WorkspaceStage::CrossReview);
    }

    struct ErrorStreamingProvider;

    #[async_trait::async_trait]
    impl StreamingProviderAdapter for ErrorStreamingProvider {
        async fn start(
            &self,
            _input: StreamingProviderInput,
            _cancel: CancellationToken,
        ) -> Result<ProviderSession, ProviderAdapterError> {
            let (event_tx, event_rx) = mpsc::channel(8);
            let (command_tx, _command_rx) = mpsc::channel(8);
            tokio::spawn(async move {
                let _ = event_tx
                    .send(ProviderEvent::Failed {
                        message: "provider unavailable".to_string(),
                    })
                    .await;
            });
            Ok(ProviderSession {
                events: event_rx,
                commands: command_tx,
            })
        }

        async fn run_streaming(
            &self,
            _input: &AdapterInput,
            _cancel: CancellationToken,
        ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
            Err(ProviderAdapterError::execution_failed(
                None,
                String::new(),
                "run_streaming is not used by WorkspaceEngine",
                0,
            ))
        }
    }

    #[tokio::test]
    async fn handle_user_message_provider_error_returns_to_prepare_context() {
        let (_tmp, store) = setup();
        let (tx, mut rx) = mpsc::channel(64);
        let session = make_session("sess_008");
        let mut engine = WorkspaceEngine::new(store, tx, session);

        engine
            .handle_user_message(
                "start".to_string(),
                Arc::new(ErrorStreamingProvider),
                empty_provider_commands(),
            )
            .await;

        let mut saw_error = false;
        let mut saw_prepare = false;
        while let Ok(event) = rx.try_recv() {
            match event {
                EngineEvent::Error { message } if message == "provider unavailable" => {
                    saw_error = true;
                }
                EngineEvent::StageChange { stage } if stage == "prepare_context" => {
                    saw_prepare = true;
                }
                _ => {}
            }
        }
        assert!(saw_error);
        assert!(saw_prepare);
        assert_eq!(engine.session().stage, WorkspaceStage::PrepareContext);
        assert_eq!(engine.session().messages.len(), 1);
    }
}
