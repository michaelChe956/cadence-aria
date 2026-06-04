use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::cross_cutting::streaming_provider::{
    ChoiceOptionData, ChoiceRequestSource, ProviderCommand, ProviderEvent, ProviderExecutionEvent,
    ProviderExecutionEventKind, ProviderExecutionEventStatus, ProviderPermissionMode,
    ProviderSession, ProviderStatus, ProviderToolCall, ProviderToolResult, RiskLevel,
    StreamingProviderAdapter, StreamingProviderInput,
};
use crate::product::artifact_extraction::extract_artifact_content;
use crate::product::checkpoint_store::CheckpointStore;
use crate::product::json_store::ProductStoreError;
use crate::product::lifecycle_store::{AppendSpecVersionInput, LifecycleStore};
use crate::product::models::{
    AgentRole, ArtifactRef, LifecycleConfirmationStatus, NodeDetail, PermissionEvent,
    ProviderConversationRef, ProviderConversationRole, ProviderName, ProviderSnapshot,
    WorkItemPlanStatus, WorkspaceMessageRecord, WorkspaceSessionRecord, WorkspaceSessionStatus,
    WorkspaceType,
};
use crate::protocol::contracts::{AdapterRole, ProviderType};
use crate::web::workspace_ws_types::{
    ArtifactVersion, AuthorDecision, HumanConfirmDecision, ProviderConfigSnapshot, ReviewVerdict,
    ReviewVerdictType, TimelineNode, TimelineNodeStatus, TimelineNodeType,
    WorkspaceStage as WsWorkspaceStage, WsCheckpointDto, WsMessageDto, WsOutMessage,
    WsProviderConfig,
};

const WORKSPACE_PROVIDER_TIMEOUT_SECS: u64 = 3600;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceStage {
    PrepareContext,
    Running,
    AuthorConfirm,
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
            Self::AuthorConfirm => "author_confirm",
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
            "author_confirm" => Some(Self::AuthorConfirm),
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
    pub superpowers_enabled: bool,
    pub openspec_enabled: bool,
    pub provider_conversations: Vec<ProviderConversationRef>,
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
            superpowers_enabled: record.superpowers_enabled,
            openspec_enabled: record.openspec_enabled,
            provider_conversations: record.provider_conversations,
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

fn provider_conversation_session_id(
    conversations: &[ProviderConversationRef],
    role: &ProviderConversationRole,
    provider: &ProviderName,
) -> Option<String> {
    conversations
        .iter()
        .find(|conversation| &conversation.role == role && &conversation.provider == provider)
        .map(|conversation| conversation.provider_session_id.clone())
        .filter(|id| !id.trim().is_empty())
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
    ChoiceRequest {
        id: String,
        prompt: String,
        options: Vec<ChoiceOptionData>,
        allow_multiple: bool,
        allow_free_text: bool,
        source: ChoiceRequestSource,
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
    ProtocolError {
        code: String,
        message: String,
        context: Option<serde_json::Value>,
    },
    PermissionTimeout {
        permission_id: String,
        node_id: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReviewDecisionOutcome {
    StartRevision,
    HumanConfirm,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthorDecisionOutcome {
    StartReview,
    HumanConfirm,
    PrepareContext,
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
    pending_author_choice: Option<PendingAuthorChoice>,
    active_run_id: Option<String>,
    stream_buffers: HashMap<String, PendingStreamBuffer>,
}

#[derive(Debug, Clone)]
struct PendingAuthorChoice {
    id: String,
    prompt: String,
    options: Vec<ChoiceOptionData>,
    source_node_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AuthorPromptMode {
    FullConversation,
    DeltaOnly,
}

impl AuthorPromptMode {
    fn prompt_event_detail(self) -> &'static str {
        match self {
            Self::FullConversation => "发送给 Workspace provider 的完整提示词",
            Self::DeltaOnly => "发送给 Workspace provider 的追加提示词",
        }
    }
}

struct ArtifactRetryContext {
    provider: Arc<dyn StreamingProviderAdapter>,
    input: StreamingProviderInput,
    attempted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PendingAuthorChoiceError {
    NotFound { id: String },
    IdMismatch { expected: String, actual: String },
    OptionUnmatched { id: String },
}

impl PendingAuthorChoiceError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotFound { .. } => "PENDING_AUTHOR_CHOICE_NOT_FOUND",
            Self::IdMismatch { .. } => "CHOICE_ID_UNMATCHED",
            Self::OptionUnmatched { .. } => "CHOICE_OPTION_UNMATCHED",
        }
    }

    pub fn message(&self) -> String {
        match self {
            Self::NotFound { id } => {
                format!("choice_response id={id} has no pending author choice")
            }
            Self::IdMismatch { expected, actual } => {
                format!("choice_response id={actual} does not match pending choice id={expected}")
            }
            Self::OptionUnmatched { id } => format!("selected option id={id} is not available"),
        }
    }
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
            pending_author_choice: None,
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
        if !persisted_artifact_versions.is_empty() {
            session.artifact = persisted_artifact_versions
                .iter()
                .rev()
                .find(|version| version.is_current)
                .map(|version| version.markdown.clone());
        }
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
            pending_author_choice: None,
            active_run_id: None,
            stream_buffers: HashMap::new(),
        }
    }

    pub fn session(&self) -> &WorkspaceSession {
        &self.session
    }

    fn provider_resume_session_id(
        &self,
        role: ProviderConversationRole,
        provider: &ProviderName,
    ) -> Option<String> {
        provider_conversation_session_id(&self.session.provider_conversations, &role, provider)
            .or_else(|| {
                self.lifecycle_store.as_ref().and_then(|store| {
                    store
                        .get_workspace_session(&self.session.session_id)
                        .ok()
                        .and_then(|session| {
                            provider_conversation_session_id(
                                &session.provider_conversations,
                                &role,
                                provider,
                            )
                        })
                })
            })
    }

    async fn record_provider_session(
        &mut self,
        role: ProviderConversationRole,
        provider: ProviderName,
        provider_session_id: Option<String>,
        node_id: Option<String>,
    ) {
        let Some(provider_session_id) = provider_session_id
            .map(|id| id.trim().to_string())
            .filter(|id| !id.is_empty())
        else {
            return;
        };
        let now = chrono::Utc::now().to_rfc3339();
        if let Some(existing) = self
            .session
            .provider_conversations
            .iter_mut()
            .find(|conversation| conversation.role == role && conversation.provider == provider)
        {
            existing.provider_session_id = provider_session_id;
            existing.updated_at = now;
            existing.last_node_id = node_id;
        } else {
            self.session
                .provider_conversations
                .push(ProviderConversationRef {
                    role,
                    provider,
                    provider_session_id,
                    updated_at: now,
                    last_node_id: node_id,
                });
        }
        if let Some(store) = &self.lifecycle_store {
            let _ = store.replace_workspace_provider_conversations(
                &self.session.session_id,
                self.session.provider_conversations.clone(),
            );
        }
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

    pub async fn take_pending_author_choice_prompt(
        &mut self,
        id: &str,
        selected_option_ids: Vec<String>,
        free_text: Option<String>,
    ) -> Result<String, PendingAuthorChoiceError> {
        let Some(pending) = self.pending_author_choice.as_ref() else {
            return Err(PendingAuthorChoiceError::NotFound { id: id.to_string() });
        };
        if pending.id != id {
            return Err(PendingAuthorChoiceError::IdMismatch {
                expected: pending.id.clone(),
                actual: id.to_string(),
            });
        }

        let mut selected_labels = Vec::new();
        for selected_id in &selected_option_ids {
            let Some(option) = pending
                .options
                .iter()
                .find(|option| option.id == *selected_id)
            else {
                return Err(PendingAuthorChoiceError::OptionUnmatched {
                    id: selected_id.clone(),
                });
            };
            selected_labels.push(option.label.clone());
        }

        let free_text = free_text.and_then(|text| {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        });

        let pending = self
            .pending_author_choice
            .take()
            .expect("pending author choice present");
        if let Some(node_id) = pending.source_node_id.as_deref() {
            self.update_timeline_node(
                node_id,
                TimelineNodeStatus::Completed,
                Some("已收到用户选择".to_string()),
            )
            .await;
        }

        let mut prompt = String::new();
        prompt.push_str("用户回答了 author 的确认问题：\n");
        prompt.push_str(&format!("问题：{}\n", pending.prompt));
        if !selected_labels.is_empty() {
            prompt.push_str("选择：\n");
            for label in selected_labels {
                prompt.push_str(&format!("- {label}\n"));
            }
        }
        if let Some(free_text) = free_text {
            prompt.push_str(&format!("补充：{free_text}\n"));
        }
        prompt.push_str(
            "\n请基于该回答继续生成完整候选产物；如果仍有必须由用户确认的问题，请继续发起选择请求，不要进入 reviewer。",
        );
        Ok(prompt)
    }

    pub async fn append_context_note(&mut self, content: String) -> Result<TimelineNode, String> {
        let msg_id = format!("msg_{:03}", self.session.messages.len() + 1);
        let now = chrono::Utc::now().to_rfc3339();
        self.session.messages.push(SessionMessage {
            id: msg_id,
            role: "user".to_string(),
            content: content.clone(),
            checkpoint_id: None,
            created_at: now,
        });
        if let Some(store) = &self.lifecycle_store {
            store
                .append_workspace_message(
                    &self.session.session_id,
                    "user".to_string(),
                    content.clone(),
                )
                .map_err(|error| format!("persist context note failed: {error}"))?;
        }
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

    pub async fn recover_stale_active_run_after_disconnect(&mut self) {
        if !matches!(
            self.session.stage,
            WorkspaceStage::Running | WorkspaceStage::CrossReview | WorkspaceStage::Revision
        ) {
            return;
        }

        let already_recorded = self
            .timeline_nodes
            .last()
            .is_some_and(|node| node.node_type == TimelineNodeType::AbortedByDisconnect);
        if !already_recorded {
            let run_id = self
                .active_run_id
                .clone()
                .unwrap_or_else(|| "stale-connection".to_string());
            let _ = self.append_aborted_by_disconnect(run_id).await;
        }
        self.transition_to_prepare_context_after_disconnect().await;
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

    pub async fn persist_permission_timeout(
        &mut self,
        node_id: &str,
        request_id: String,
    ) -> Result<(), String> {
        self.persist_permission_response(
            node_id,
            request_id,
            serde_json::json!({ "status": "timeout" }),
        )
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
        self.handle_author_message_with_prompt_mode(
            content,
            provider,
            command_rx,
            AuthorPromptMode::FullConversation,
        )
        .await;
    }

    pub async fn handle_author_choice_followup_message(
        &mut self,
        content: String,
        provider: Arc<dyn StreamingProviderAdapter>,
        command_rx: mpsc::Receiver<ProviderCommand>,
    ) {
        self.handle_author_message_with_prompt_mode(
            content,
            provider,
            command_rx,
            AuthorPromptMode::DeltaOnly,
        )
        .await;
    }

    async fn handle_author_message_with_prompt_mode(
        &mut self,
        content: String,
        provider: Arc<dyn StreamingProviderAdapter>,
        command_rx: mpsc::Receiver<ProviderCommand>,
        prompt_mode: AuthorPromptMode,
    ) {
        let content = normalize_generation_prompt(content, &self.session.workspace_type);
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

        let input = match self.build_streaming_input(&content, prompt_mode) {
            Ok(input) => input,
            Err(message) => {
                let _ = self.event_tx.send(EngineEvent::Error { message }).await;
                self.finish_failed_run().await;
                return;
            }
        };
        let _ = self
            .persist_prompt_snapshot(&generation_node_id, input.prompt.clone())
            .await;
        self.emit_execution_event(
            provider_prompt_event(
                &generation_node_id,
                input.prompt.clone(),
                prompt_mode.prompt_event_detail(),
            ),
            Some(generation_node_id.clone()),
            Some(self.session.author_provider.clone()),
        )
        .await;

        let retry_context = ArtifactRetryContext {
            provider: provider.clone(),
            input: input.clone(),
            attempted: false,
        };
        let session = provider.start(input, self.cancel.clone()).await;
        self.drive_provider_session(
            session,
            command_rx,
            Some(generation_node_id),
            Some(self.session.author_provider.clone()),
            ProviderConversationRole::Author,
            Some(retry_context),
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
        role: ProviderConversationRole,
        mut artifact_retry: Option<ArtifactRetryContext>,
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
        let mut tool_call_titles = BTreeMap::new();
        let mut tool_call_commands = BTreeMap::new();

        while events_open {
            tokio::select! {
                _ = cancel.cancelled() => {
                    if let Some(node_id) = node_id.as_deref() {
                        let _ = self.flush_stream_buffer(node_id).await;
                    }
                    self.finish_aborted_run().await;
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
                            self.finish_aborted_run().await;
                            return;
                        }
                        Some(ProviderCommand::PermissionResponse {
                            id,
                            approved,
                            reason,
                        }) => {
                            tracing::info!(permission_id = %id, "engine forwarding permission response");
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
                        Some(ProviderCommand::ChoiceResponse {
                            id,
                            selected_option_ids,
                            free_text,
                        }) => {
                            tracing::info!(choice_id = %id, "engine forwarding choice response");
                            let choice_id = id.clone();
                            eprintln!(
                                "[aria-choice-diag] engine forwarding author choice_response id={} selected={:?} free_text_present={}",
                                choice_id,
                                selected_option_ids,
                                free_text.as_ref().is_some_and(|text| !text.trim().is_empty())
                            );
                            if session.commands.send(ProviderCommand::ChoiceResponse {
                                id,
                                selected_option_ids,
                                free_text,
                            }).await.is_err() {
                                eprintln!(
                                    "[aria-choice-diag] engine failed to forward author choice_response id={} to provider session",
                                    choice_id
                                );
                                commands_open = false;
                            } else {
                                eprintln!(
                                    "[aria-choice-diag] engine forwarded author choice_response id={} to provider session",
                                    choice_id
                                );
                            }
                        }
                        Some(ProviderCommand::ToolResult(_)) => {}
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
                        ProviderEvent::ChoiceRequest(request) => {
                            let _ = self
                                .event_tx
                                .send(EngineEvent::ChoiceRequest {
                                    id: request.id,
                                    prompt: request.prompt,
                                    options: request.options,
                                    allow_multiple: request.allow_multiple,
                                    allow_free_text: request.allow_free_text,
                                    source: request.source,
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
                            self.emit_execution_event(event, node_id.clone(), agent.clone()).await;
                        }
                        ProviderEvent::ToolCall(call) => {
                            tool_call_titles.insert(call.id.clone(), call.tool_name.clone());
                            if let Some(command) = extract_tool_command(&call.input) {
                                tool_call_commands.insert(call.id.clone(), command);
                            }
                            self
                                .emit_execution_event(
                                    execution_event_from_tool_call(call),
                                    node_id.clone(),
                                    agent.clone(),
                                )
                                .await;
                        }
                        ProviderEvent::ToolResult(result) => {
                            let title = tool_call_titles
                                .get(&result.tool_use_id)
                                .cloned()
                                .unwrap_or_else(|| "Tool result".to_string());
                            let command = tool_call_commands.get(&result.tool_use_id).cloned();
                            self
                                .emit_execution_event(
                                    execution_event_from_tool_result(result, title, command),
                                    node_id.clone(),
                                    agent.clone(),
                                )
                                .await;
                        }
                        ProviderEvent::Completed {
                            full_output,
                            provider_session_id,
                        } => {
                            if let Some(node_id) = node_id.as_deref() {
                                let _ = self.flush_stream_buffer(node_id).await;
                            }
                            let completed_provider_session_id = provider_session_id.clone();
                            if let Some(provider) = agent.clone() {
                                self.record_provider_session(
                                    role.clone(),
                                    provider,
                                    provider_session_id,
                                    node_id.clone(),
                                )
                                .await;
                            }
                            let completed_output = if self.workspace_requires_artifact_gate()
                                && !content_has_complete_workspace_artifact(
                                    &extract_artifact_content(&full_output),
                                    &self.session.workspace_type,
                                )
                                && content_has_complete_workspace_artifact(
                                    &extract_artifact_content(&full_content),
                                    &self.session.workspace_type,
                                ) {
                                full_content.clone()
                            } else {
                                full_output
                            };

                            let retry_start = if self
                                .should_retry_missing_workspace_artifact(&completed_output)
                            {
                                if let Some(context) = artifact_retry.as_mut() {
                                    if context.attempted {
                                        None
                                    } else {
                                        context.attempted = true;
                                        let retry_input = self.build_artifact_retry_input(
                                            &context.input,
                                            &completed_output,
                                            completed_provider_session_id.clone(),
                                        );
                                        context.input = retry_input.clone();
                                        Some((context.provider.clone(), retry_input))
                                    }
                                } else {
                                    None
                                }
                            } else {
                                None
                            };

                            if let Some((provider, retry_input)) = retry_start {
                                if let Some(node_id) = node_id.as_deref() {
                                    self.emit_execution_event(
                                        provider_prompt_event(
                                            node_id,
                                            retry_input.prompt.clone(),
                                            "自动续写缺失 artifact 的提示词",
                                        ),
                                        Some(node_id.to_string()),
                                        agent.clone(),
                                    )
                                    .await;
                                }
                                match provider.start(retry_input, self.cancel.clone()).await {
                                    Ok(next_session) => {
                                        session = next_session;
                                        full_content.clear();
                                        tool_call_titles.clear();
                                        tool_call_commands.clear();
                                        continue;
                                    }
                                    Err(error) => {
                                        let _ = self
                                            .event_tx
                                            .send(EngineEvent::Error {
                                                message: error.details.clone(),
                                            })
                                            .await;
                                        if let Some(node_id) = node_id.as_deref() {
                                            self.update_timeline_node(
                                                node_id,
                                                TimelineNodeStatus::Failed,
                                                Some("Provider 自动续写启动失败".to_string()),
                                            )
                                            .await;
                                        }
                                        self.finish_failed_run().await;
                                        return;
                                    }
                                }
                            }

                            let artifact_retry_attempted =
                                artifact_retry.as_ref().is_some_and(|context| context.attempted);
                            self.complete_assistant_message(
                                assistant_msg_id,
                                completed_output,
                                artifact_retry_attempted,
                            )
                                .await;
                            return;
                        }
                        ProviderEvent::Failed { message } => {
                            let _ = self
                                .event_tx
                                .send(EngineEvent::Error { message })
                                .await;
                            if let Some(node_id) = node_id.as_deref() {
                                let _ = self.flush_stream_buffer(node_id).await;
                                self.update_timeline_node(
                                    node_id,
                                    TimelineNodeStatus::Failed,
                                    Some("Provider 运行失败".to_string()),
                                )
                                .await;
                            }
                            self.finish_failed_run().await;
                            return;
                        }
                        ProviderEvent::ProtocolError {
                            code,
                            message,
                            context,
                        } => {
                            let _ = self
                                .event_tx
                                .send(EngineEvent::ProtocolError {
                                    code,
                                    message,
                                    context,
                                })
                                .await;
                        }
                        ProviderEvent::PermissionTimeout { permission_id } => {
                            self.handle_permission_timeout(permission_id, node_id.clone())
                                .await;
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
            self.finish_aborted_run().await;
            return;
        }

        if full_content.is_empty() {
            if let Some(node_id) = node_id.as_deref() {
                let _ = self.flush_stream_buffer(node_id).await;
            }
            self.finish_empty_assistant_output().await;
        } else {
            if let Some(node_id) = node_id.as_deref() {
                let _ = self.flush_stream_buffer(node_id).await;
            }
            self.complete_assistant_message(assistant_msg_id, full_content, false)
                .await;
        }
    }

    async fn emit_execution_event(
        &mut self,
        event: ProviderExecutionEvent,
        node_id: Option<String>,
        agent: Option<ProviderName>,
    ) {
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
                node_id,
                agent,
            })
            .await;
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
        if let Some(node_id) = self.active_node_id.clone() {
            let _ = self
                .persist_prompt_snapshot(&node_id, input.prompt.clone())
                .await;
            self.emit_execution_event(
                provider_prompt_event(
                    &node_id,
                    input.prompt.clone(),
                    "发送给 Workspace provider 的完整提示词",
                ),
                Some(node_id),
                Some(reviewer.clone()),
            )
            .await;
        }
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
        if let Some(node_id) = node_id.clone() {
            let _ = self
                .persist_prompt_snapshot(&node_id, input.prompt.clone())
                .await;
            self.emit_execution_event(
                provider_prompt_event(
                    &node_id,
                    input.prompt.clone(),
                    "发送给 Workspace provider 的完整提示词",
                ),
                Some(node_id),
                Some(author.clone()),
            )
            .await;
        }
        let retry_context = ArtifactRetryContext {
            provider: provider.clone(),
            input: input.clone(),
            attempted: false,
        };
        let session = provider.start(input, self.cancel.clone()).await;
        self.drive_provider_session(
            session,
            command_rx,
            node_id,
            Some(author),
            ProviderConversationRole::Author,
            Some(retry_context),
        )
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
        let mut tool_call_titles = BTreeMap::new();
        let mut tool_call_commands = BTreeMap::new();

        while events_open {
            tokio::select! {
                _ = cancel.cancelled() => {
                    if let Some(node_id) = node_id.as_deref() {
                        let _ = self.flush_stream_buffer(node_id).await;
                    }
                    self.finish_aborted_run().await;
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
                            self.finish_aborted_run().await;
                            return;
                        }
                        Some(ProviderCommand::PermissionResponse {
                            id,
                            approved,
                            reason,
                        }) => {
                            tracing::info!(permission_id = %id, "engine forwarding permission response");
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
                        Some(ProviderCommand::ChoiceResponse {
                            id,
                            selected_option_ids,
                            free_text,
                        }) => {
                            tracing::info!(choice_id = %id, "engine forwarding choice response");
                            let choice_id = id.clone();
                            eprintln!(
                                "[aria-choice-diag] engine forwarding reviewer choice_response id={} selected={:?} free_text_present={}",
                                choice_id,
                                selected_option_ids,
                                free_text.as_ref().is_some_and(|text| !text.trim().is_empty())
                            );
                            if session
                                .commands
                                .send(ProviderCommand::ChoiceResponse {
                                    id,
                                    selected_option_ids,
                                    free_text,
                                })
                                .await
                                .is_err()
                            {
                                eprintln!(
                                    "[aria-choice-diag] engine failed to forward reviewer choice_response id={} to provider session",
                                    choice_id
                                );
                                commands_open = false;
                            } else {
                                eprintln!(
                                    "[aria-choice-diag] engine forwarded reviewer choice_response id={} to provider session",
                                    choice_id
                                );
                            }
                        }
                        Some(ProviderCommand::ToolResult(_)) => {}
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
                        ProviderEvent::ChoiceRequest(request) => {
                            let _ = self
                                .event_tx
                                .send(EngineEvent::ChoiceRequest {
                                    id: request.id,
                                    prompt: request.prompt,
                                    options: request.options,
                                    allow_multiple: request.allow_multiple,
                                    allow_free_text: request.allow_free_text,
                                    source: request.source,
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
                            self
                                .emit_execution_event(
                                    event,
                                    node_id.clone(),
                                    Some(reviewer.clone()),
                                )
                                .await;
                        }
                        ProviderEvent::ToolCall(call) => {
                            tool_call_titles.insert(call.id.clone(), call.tool_name.clone());
                            if let Some(command) = extract_tool_command(&call.input) {
                                tool_call_commands.insert(call.id.clone(), command);
                            }
                            self
                                .emit_execution_event(
                                    execution_event_from_tool_call(call),
                                    node_id.clone(),
                                    Some(reviewer.clone()),
                                )
                                .await;
                        }
                        ProviderEvent::ToolResult(result) => {
                            let title = tool_call_titles
                                .get(&result.tool_use_id)
                                .cloned()
                                .unwrap_or_else(|| "Tool result".to_string());
                            let command = tool_call_commands.get(&result.tool_use_id).cloned();
                            self
                                .emit_execution_event(
                                    execution_event_from_tool_result(result, title, command),
                                    node_id.clone(),
                                    Some(reviewer.clone()),
                                )
                                .await;
                        }
                        ProviderEvent::Completed {
                            full_output,
                            provider_session_id,
                        } => {
                            if let Some(node_id) = node_id.as_deref() {
                                let _ = self.flush_stream_buffer(node_id).await;
                            }
                            self.record_provider_session(
                                ProviderConversationRole::Reviewer,
                                reviewer.clone(),
                                provider_session_id,
                                node_id.clone(),
                            )
                            .await;
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
                                self.update_timeline_node(
                                    node_id,
                                    TimelineNodeStatus::Failed,
                                    Some("Provider 运行失败".to_string()),
                                )
                                .await;
                            }
                            self.finish_failed_run().await;
                            return;
                        }
                        ProviderEvent::ProtocolError {
                            code,
                            message,
                            context,
                        } => {
                            let _ = self
                                .event_tx
                                .send(EngineEvent::ProtocolError {
                                    code,
                                    message,
                                    context,
                                })
                                .await;
                        }
                        ProviderEvent::PermissionTimeout { permission_id } => {
                            self.handle_permission_timeout(permission_id, node_id.clone())
                                .await;
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
            self.finish_aborted_run().await;
        } else if full_content.is_empty() {
            if let Some(node_id) = node_id.as_deref() {
                let _ = self.flush_stream_buffer(node_id).await;
            }
            self.finish_empty_assistant_output().await;
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

    pub async fn handle_author_decision(
        &mut self,
        decision: AuthorDecision,
    ) -> Result<AuthorDecisionOutcome, String> {
        if self.session.stage != WorkspaceStage::AuthorConfirm {
            return Err(
                "author decision is only available during author_confirm stage".to_string(),
            );
        }

        match decision {
            AuthorDecision::Accept => {
                let review_enabled =
                    self.session.review_rounds > 0 && self.session.reviewer_provider.is_some();
                self.complete_active_node(Some("已进入 Review".to_string()))
                    .await;
                self.start_review_or_skip().await;
                if review_enabled && self.session.stage == WorkspaceStage::CrossReview {
                    Ok(AuthorDecisionOutcome::StartReview)
                } else {
                    Ok(AuthorDecisionOutcome::HumanConfirm)
                }
            }
            AuthorDecision::Reject => {
                self.complete_active_node(Some("用户要求重新编写".to_string()))
                    .await;
                self.session.artifact = None;
                self.mark_latest_artifact_rejected();
                if let Some(store) = &self.lifecycle_store {
                    let _ = store.update_workspace_session_status(
                        &self.session.session_id,
                        WorkspaceSessionStatus::Open,
                    );
                }
                self.transition_stage(WorkspaceStage::PrepareContext).await;
                let _ = self
                    .create_timeline_node(TimelineNodeDraft {
                        node_type: TimelineNodeType::PrepareContext,
                        agent: None,
                        stage: WorkspaceStage::PrepareContext,
                        round: None,
                        title: "准备上下文".to_string(),
                        summary: Some("等待重新补充上下文".to_string()),
                        status: TimelineNodeStatus::Active,
                    })
                    .await;
                Ok(AuthorDecisionOutcome::PrepareContext)
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
                if decision == "continue_with_context" && normalized_context.is_none() {
                    return Err(
                        "continue_with_context requires non-empty extra_context".to_string()
                    );
                }
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

    fn should_retry_missing_workspace_artifact(&self, full_content: &str) -> bool {
        if !self.workspace_requires_artifact_gate() || full_content.trim().is_empty() {
            return false;
        }

        let artifact_markdown = extract_artifact_content(full_content);
        !content_has_complete_workspace_artifact(&artifact_markdown, &self.session.workspace_type)
            && detect_author_choice_request(full_content, &self.session.workspace_type).is_none()
    }

    fn build_artifact_retry_input(
        &self,
        base_input: &StreamingProviderInput,
        previous_output: &str,
        provider_session_id: Option<String>,
    ) -> StreamingProviderInput {
        let mut input = base_input.clone();
        input.prompt = build_artifact_retry_prompt(&self.session.workspace_type, previous_output);
        if let Some(provider_session_id) = provider_session_id
            .map(|id| id.trim().to_string())
            .filter(|id| !id.is_empty())
        {
            input.resume_provider_session_id = Some(provider_session_id);
        }
        input
    }

    async fn complete_assistant_message(
        &mut self,
        assistant_msg_id: String,
        full_content: String,
        artifact_retry_attempted: bool,
    ) {
        if self.cancel.is_cancelled() {
            self.finish_aborted_run().await;
            return;
        }

        if full_content.is_empty() {
            self.finish_empty_assistant_output().await;
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
        }

        if let Some(choice) =
            detect_author_choice_request(&full_content, &self.session.workspace_type).map(
                |(prompt, options)| PendingAuthorChoice {
                    id: format!("author_choice_{}", assistant_msg_id),
                    prompt,
                    options,
                    source_node_id: self.active_node_id.clone(),
                },
            )
        {
            if let Some(node_id) = choice.source_node_id.as_deref() {
                self.update_timeline_node(
                    node_id,
                    TimelineNodeStatus::Paused,
                    Some("等待用户选择".to_string()),
                )
                .await;
            }
            self.pending_author_choice = Some(choice.clone());
            let _ = self
                .event_tx
                .send(EngineEvent::ChoiceRequest {
                    id: choice.id,
                    prompt: choice.prompt,
                    options: choice.options,
                    allow_multiple: false,
                    allow_free_text: true,
                    source: ChoiceRequestSource::TextFallback,
                })
                .await;
            return;
        }

        self.pending_author_choice = None;
        let artifact_markdown = extract_artifact_content(&full_content);
        if self.workspace_requires_artifact_gate()
            && !content_has_complete_workspace_artifact(
                &artifact_markdown,
                &self.session.workspace_type,
            )
        {
            if artifact_retry_attempted {
                self.finish_invalid_workspace_artifact_after_retry().await;
            } else {
                self.finish_invalid_workspace_artifact().await;
            }
            return;
        }
        if let Some(store) = &self.lifecycle_store
            && matches!(
                self.session.workspace_type,
                WorkspaceType::Story | WorkspaceType::Design
            )
        {
            let _ = store.append_version(AppendSpecVersionInput {
                project_id: self.session.project_id.clone(),
                issue_id: self.session.issue_id.clone(),
                entity_id: self.session.entity_id.clone(),
                markdown: artifact_markdown.clone(),
                provider_run_refs: Vec::new(),
                review_refs: Vec::new(),
                confirmed_by: None,
            });
        }
        self.update_artifact(artifact_markdown).await;

        let message_index = self.session.messages.len() as u32;
        let artifact_snapshot = self.session.artifact.clone().unwrap_or_default();
        let checkpoint = self.checkpoint_store.create_checkpoint(
            &self.session.session_id,
            message_index,
            &artifact_snapshot,
            WorkspaceStage::AuthorConfirm.as_str(),
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
        self.enter_author_confirm(Some("等待用户确认 author 结果".to_string()))
            .await;
    }

    fn build_streaming_input(
        &self,
        user_content: &str,
        prompt_mode: AuthorPromptMode,
    ) -> Result<StreamingProviderInput, String> {
        let working_dir = match &self.session.repository_path {
            Some(path) => path.clone(),
            None => std::env::current_dir()
                .map_err(|error| format!("working directory error: {error}"))?,
        };
        let provider = self.session.author_provider.clone();
        let resume_provider_session_id =
            self.provider_resume_session_id(ProviderConversationRole::Author, &provider);

        Ok(StreamingProviderInput {
            provider_type: provider_type_for_name(&provider),
            role: AdapterRole::Orchestrator,
            prompt: match prompt_mode {
                AuthorPromptMode::FullConversation => self.build_prompt(user_content),
                AuthorPromptMode::DeltaOnly => user_content.to_string(),
            },
            working_dir,
            workspace_session_id: Some(self.session.session_id.clone()),
            resume_provider_session_id,
            permission_mode: ProviderPermissionMode::Supervised,
            env_vars: BTreeMap::new(),
            timeout_secs: WORKSPACE_PROVIDER_TIMEOUT_SECS,
        })
    }

    fn build_review_input(&self) -> Result<StreamingProviderInput, String> {
        let working_dir = match &self.session.repository_path {
            Some(path) => path.clone(),
            None => std::env::current_dir()
                .map_err(|error| format!("working directory error: {error}"))?,
        };

        let artifact = self.session.artifact.clone().unwrap_or_default();
        let provider = self
            .session
            .reviewer_provider
            .clone()
            .unwrap_or(ProviderName::Codex);
        let mut prompt = String::new();
        prompt.push_str("请作为 reviewer 审核当前 Workspace 产物。\n\n");
        prompt.push_str(&format!(
            "Workspace 类型: {}\n",
            workspace_type_title(&self.session.workspace_type)
        ));
        prompt.push_str("会话上下文:\n");
        for msg in &self.session.messages {
            if matches!(msg.role.as_str(), "assistant" | "provider") {
                continue;
            }
            prompt.push_str(&format!("[{}]: {}\n", msg.role, msg.content));
        }
        self.append_missing_context_notes_to_prompt(&mut prompt);
        prompt.push_str("\n当前 Artifact:\n\n");
        prompt.push_str(&artifact);
        prompt.push_str(
            "\n\n请输出审核意见，并在末尾附加 JSON 代码块：\n\
             ```json\n\
             {\"verdict\":\"pass|revise|needs_human\",\"summary\":\"一句话摘要\"}\n\
             ```\n",
        );

        Ok(StreamingProviderInput {
            provider_type: provider_type_for_name(&provider),
            role: AdapterRole::Reviewer,
            prompt,
            working_dir,
            workspace_session_id: Some(self.session.session_id.clone()),
            resume_provider_session_id: None,
            permission_mode: ProviderPermissionMode::Supervised,
            env_vars: BTreeMap::new(),
            timeout_secs: WORKSPACE_PROVIDER_TIMEOUT_SECS,
        })
    }

    fn build_revision_input(&self) -> Result<StreamingProviderInput, String> {
        let working_dir = match &self.session.repository_path {
            Some(path) => path.clone(),
            None => std::env::current_dir()
                .map_err(|error| format!("working directory error: {error}"))?,
        };

        let artifact = self.session.artifact.clone().unwrap_or_default();
        let provider = self.session.author_provider.clone();
        let resume_provider_session_id =
            self.provider_resume_session_id(ProviderConversationRole::Author, &provider);
        let review = self
            .latest_review_verdict
            .as_ref()
            .ok_or_else(|| "review verdict is unavailable for revision".to_string())?;
        let prompt = if resume_provider_session_id.is_some() {
            self.build_revision_delta_prompt(review)
        } else {
            self.build_revision_full_prompt(&artifact, review)
        };

        Ok(StreamingProviderInput {
            provider_type: provider_type_for_name(&provider),
            role: AdapterRole::Orchestrator,
            prompt,
            working_dir,
            workspace_session_id: Some(self.session.session_id.clone()),
            resume_provider_session_id,
            permission_mode: ProviderPermissionMode::Supervised,
            env_vars: BTreeMap::new(),
            timeout_secs: WORKSPACE_PROVIDER_TIMEOUT_SECS,
        })
    }

    fn build_revision_delta_prompt(&self, review: &ReviewVerdict) -> String {
        let mut prompt = String::new();
        prompt.push_str("请作为 author 继续返修当前 Workspace 产物。\n\n");
        prompt.push_str(&format!(
            "Workspace 类型: {}\n",
            workspace_type_title(&self.session.workspace_type)
        ));
        prompt.push_str("这是对当前 provider 会话的增量返修指令。不要重新调研完整上下文，不要只解释；请基于本会话已有上下文、上一版 artifact 和以下 reviewer 意见，直接输出完整更新后的 artifact markdown。\n");
        self.append_missing_context_notes_to_prompt(&mut prompt);
        prompt.push_str("\nReviewer 审核意见:\n\n");
        prompt.push_str(&review.comments);
        prompt.push_str("\n\nReviewer 摘要:\n");
        prompt.push_str(&review.summary);
        if let Some(context) = &self.pending_revision_context {
            prompt.push_str("\n\n用户补充信息优先级高于 Reviewer 审核意见；如二者冲突，以用户补充信息为准，并在更新后的 artifact 中体现用户补充要求。\n用户补充信息:\n");
            prompt.push_str(context);
        }
        prompt.push_str("\n\n请根据以上审核意见修改产物，输出完整更新后的 artifact markdown。\n");
        prompt
    }

    fn build_revision_full_prompt(&self, artifact: &str, review: &ReviewVerdict) -> String {
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
        self.append_missing_context_notes_to_prompt(&mut prompt);
        prompt.push_str("\n上一版 Artifact:\n\n");
        prompt.push_str(artifact);
        prompt.push_str("\n\nReviewer 审核意见:\n\n");
        prompt.push_str(&review.comments);
        prompt.push_str("\n\nReviewer 摘要:\n");
        prompt.push_str(&review.summary);
        if let Some(context) = &self.pending_revision_context {
            prompt.push_str("\n\n用户补充信息优先级高于 Reviewer 审核意见；如二者冲突，以用户补充信息为准，并在更新后的 artifact 中体现用户补充要求。\n用户补充信息:\n");
            prompt.push_str(context);
        }
        prompt.push_str("\n\n请根据以上审核意见修改产物，输出完整更新后的 artifact markdown。\n");
        prompt
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
        for version in &mut self.artifact_versions {
            version.is_current = false;
        }
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
            is_current: true,
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
        self.active_run_id = None;
        self.transition_stage(WorkspaceStage::PrepareContext).await;
    }

    async fn finish_empty_assistant_output(&mut self) {
        let _ = self
            .event_tx
            .send(EngineEvent::Error {
                message: "Provider completed without assistant output".to_string(),
            })
            .await;
        if let Some(node_id) = self.active_node_id.clone() {
            self.update_timeline_node(
                &node_id,
                TimelineNodeStatus::Failed,
                Some("Provider 未返回助手内容".to_string()),
            )
            .await;
        }
        self.finish_failed_run().await;
    }

    async fn finish_invalid_workspace_artifact(&mut self) {
        let artifact_name = workspace_type_title(&self.session.workspace_type);
        let message = format!("Provider 未返回有效的 {artifact_name} artifact");
        let _ = self
            .event_tx
            .send(EngineEvent::Error {
                message: message.clone(),
            })
            .await;
        if let Some(node_id) = self.active_node_id.clone() {
            self.update_timeline_node(&node_id, TimelineNodeStatus::Failed, Some(message))
                .await;
        }
        self.finish_failed_run().await;
    }

    async fn finish_invalid_workspace_artifact_after_retry(&mut self) {
        let artifact_name = workspace_type_title(&self.session.workspace_type);
        let message = format!("自动续写后仍未返回有效的 {artifact_name} artifact");
        let _ = self
            .event_tx
            .send(EngineEvent::Error {
                message: message.clone(),
            })
            .await;
        if let Some(node_id) = self.active_node_id.clone() {
            self.update_timeline_node(&node_id, TimelineNodeStatus::Failed, Some(message))
                .await;
        }
        self.finish_failed_run().await;
    }

    fn workspace_requires_artifact_gate(&self) -> bool {
        matches!(
            self.session.workspace_type,
            WorkspaceType::Story | WorkspaceType::Design
        )
    }

    async fn finish_aborted_run(&mut self) {
        if let Some(node_id) = self.active_node_id.clone() {
            self.update_timeline_node(
                &node_id,
                TimelineNodeStatus::Failed,
                Some("运行已中止".to_string()),
            )
            .await;
        }
        self.finish_failed_run().await;
    }

    async fn handle_permission_timeout(&mut self, permission_id: String, node_id: Option<String>) {
        tracing::warn!(permission_id = %permission_id, "permission timed out; aborting active run");
        if let Some(node_id) = node_id.as_deref() {
            let _ = self
                .persist_permission_timeout(node_id, permission_id.clone())
                .await;
            let _ = self.flush_stream_buffer(node_id).await;
            self.update_timeline_node(
                node_id,
                TimelineNodeStatus::Failed,
                Some("权限请求超时，运行已中止".to_string()),
            )
            .await;
        }
        self.active_run_id = None;
        self.cancel.cancel();
        let _ = self
            .event_tx
            .send(EngineEvent::PermissionTimeout {
                permission_id,
                node_id,
            })
            .await;
        self.finish_failed_run().await;
    }

    fn build_prompt(&self, user_content: &str) -> String {
        let mut prompt = String::new();
        let last_current_user_message_index =
            self.session.messages.len().checked_sub(1).filter(|index| {
                let message = &self.session.messages[*index];
                message.role == "user" && message.content == user_content
            });
        for (index, msg) in self.session.messages.iter().enumerate() {
            if Some(index) == last_current_user_message_index {
                continue;
            }
            prompt.push_str(&format!("[{}]: {}\n", msg.role, msg.content));
        }

        for note in self.missing_context_note_summaries() {
            prompt.push_str(&format!("[user]: {note}\n"));
        }

        if let Some(index) = last_current_user_message_index {
            let msg = &self.session.messages[index];
            prompt.push_str(&format!("[{}]: {}\n", msg.role, msg.content));
        } else {
            prompt.push_str(&format!("[user]: {user_content}\n"));
        }
        prompt
    }

    fn missing_context_note_summaries(&self) -> Vec<String> {
        let known_message_contents = self
            .session
            .messages
            .iter()
            .map(|message| message.content.trim().to_string())
            .collect::<Vec<_>>();

        self.timeline_nodes
            .iter()
            .filter_map(|node| {
                if node.node_type != TimelineNodeType::ContextNote {
                    return None;
                }
                let note = node.summary.as_deref()?.trim();
                (!note.is_empty()
                    && !known_message_contents
                        .iter()
                        .any(|content| content.as_str() == note))
                .then(|| note.to_string())
            })
            .collect()
    }

    fn append_missing_context_notes_to_prompt(&self, prompt: &mut String) {
        let notes = self.missing_context_note_summaries();
        if notes.is_empty() {
            return;
        }

        prompt.push_str("\n准备阶段用户补充上下文:\n");
        for note in notes {
            prompt.push_str(&format!("- {note}\n"));
        }
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
            superpowers_enabled: self.session.superpowers_enabled,
            openspec_enabled: self.session.openspec_enabled,
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

    async fn enter_author_confirm(&mut self, summary: Option<String>) {
        self.transition_stage(WorkspaceStage::AuthorConfirm).await;
        let _ = self
            .create_timeline_node(TimelineNodeDraft {
                node_type: TimelineNodeType::AuthorConfirm,
                agent: None,
                stage: WorkspaceStage::AuthorConfirm,
                round: None,
                title: "Author 结果确认".to_string(),
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
            prompt: None,
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
        let Some(store) = &self.lifecycle_store else {
            return Ok(());
        };

        let Some(node) = self
            .timeline_nodes
            .iter()
            .find(|node| node.node_id == node_id)
            .cloned()
            .or_else(|| {
                store
                    .load_timeline_nodes(&self.session.session_id)
                    .ok()?
                    .into_iter()
                    .find(|node| node.node_id == node_id)
            })
        else {
            return Err(format!("timeline node not found: {node_id}"));
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

    async fn persist_prompt_snapshot(
        &mut self,
        node_id: &str,
        prompt: String,
    ) -> Result<(), String> {
        self.update_node_detail(node_id, |detail| {
            detail.prompt = Some(prompt);
        })
        .await
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

    fn mark_latest_artifact_rejected(&mut self) {
        if let Some(version) = self.artifact_versions.last_mut() {
            version.is_current = false;
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
        .map(|node| node.node_id.clone())
}

fn workspace_stage_from_ws_stage(stage: &WsWorkspaceStage) -> WorkspaceStage {
    match stage {
        WsWorkspaceStage::PrepareContext => WorkspaceStage::PrepareContext,
        WsWorkspaceStage::Running => WorkspaceStage::Running,
        WsWorkspaceStage::AuthorConfirm => WorkspaceStage::AuthorConfirm,
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

fn detect_author_choice_request(
    content: &str,
    workspace_type: &WorkspaceType,
) -> Option<(String, Vec<ChoiceOptionData>)> {
    if !matches!(workspace_type, WorkspaceType::Story | WorkspaceType::Design) {
        return None;
    }
    if content_has_complete_workspace_artifact(content, workspace_type) {
        return None;
    }
    if !looks_like_user_question(content) {
        return None;
    }

    let mut options = Vec::new();
    let mut prompt_lines = Vec::new();
    let mut seen_first_option = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(option) = parse_choice_option_line(trimmed) {
            seen_first_option = true;
            options.push(option);
            continue;
        }
        if !seen_first_option {
            prompt_lines.push(trimmed.to_string());
        }
    }

    if options.len() < 2 {
        return detect_recommendation_choice_request(content);
    }

    let prompt = prompt_lines.join("\n");
    let prompt = if prompt.trim().is_empty() {
        "请选择下一步处理方式。".to_string()
    } else {
        prompt.trim().to_string()
    };
    Some((prompt, options))
}

fn detect_recommendation_choice_request(content: &str) -> Option<(String, Vec<ChoiceOptionData>)> {
    let mut prompt_lines = Vec::new();
    let mut option_texts = Vec::new();
    let mut seen_option_line = false;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        if let Some(text) = strip_choice_prefix(trimmed, &["推荐选项：", "推荐选项:"]) {
            seen_option_line = true;
            push_choice_text(&mut option_texts, text);
            continue;
        }

        if let Some(text) = strip_choice_prefix(
            trimmed,
            &["其他可选：", "其他可选:", "其他选项：", "其他选项:"],
        ) {
            seen_option_line = true;
            for choice_text in split_inline_choices(text) {
                push_choice_text(&mut option_texts, choice_text);
            }
            continue;
        }

        if !seen_option_line {
            prompt_lines.push(trimmed.to_string());
        }
    }

    if option_texts.len() < 2 {
        return None;
    }

    let options = option_texts
        .into_iter()
        .enumerate()
        .map(|(idx, text)| {
            let id = ((b'A' + idx as u8) as char).to_string();
            ChoiceOptionData {
                id: id.clone(),
                label: format!("{id}. {text}"),
                description: None,
            }
        })
        .collect();
    let prompt = prompt_lines.join("\n");
    let prompt = if prompt.trim().is_empty() {
        "请选择下一步处理方式。".to_string()
    } else {
        prompt.trim().to_string()
    };
    Some((prompt, options))
}

fn strip_choice_prefix<'a>(line: &'a str, prefixes: &[&str]) -> Option<&'a str> {
    prefixes
        .iter()
        .find_map(|prefix| line.strip_prefix(prefix))
        .map(str::trim)
        .filter(|text| !text.is_empty())
}

fn split_inline_choices(text: &str) -> Vec<&str> {
    text.split(['；', ';'])
        .flat_map(|part| part.split(" 或 "))
        .flat_map(|part| part.split("或"))
        .map(|part| part.trim())
        .filter(|part| !part.is_empty())
        .collect()
}

fn push_choice_text(option_texts: &mut Vec<String>, text: &str) {
    let normalized = text
        .trim()
        .trim_start_matches("或")
        .trim()
        .trim_end_matches(['。', '.', '；', ';'])
        .trim();
    if !normalized.is_empty() {
        option_texts.push(normalized.to_string());
    }
}

fn looks_like_user_question(content: &str) -> bool {
    content.contains('?')
        || content.contains('？')
        || content.contains("需要确认")
        || content.contains("需要先确认")
        || content.contains("请选择")
        || content.contains("如何处理")
}

fn content_has_complete_workspace_artifact(content: &str, workspace_type: &WorkspaceType) -> bool {
    match workspace_type {
        WorkspaceType::Story => content.contains("## 功能需求") && content.contains("## 成功标准"),
        WorkspaceType::Design => design_artifact_has_required_headings(content),
        WorkspaceType::WorkItem => false,
    }
}

fn design_artifact_has_required_headings(content: &str) -> bool {
    let headings = workspace_artifact_headings(content).collect::<Vec<_>>();
    let has_decisions = headings
        .iter()
        .any(|heading| heading_matches(heading, &["设计决策", "Design Decisions"]));
    let has_structure = headings.iter().any(|heading| {
        heading_matches(
            heading,
            &[
                "公共组件",
                "Shared Components",
                "shared_components",
                "API 契约",
                "API Contract",
                "api_entries",
                "数据模型",
                "数据实体",
                "Data Entities",
                "data_entities",
            ],
        ) || heading_contains_component_api_data_bucket(heading)
    });

    has_decisions && has_structure
}

fn workspace_artifact_headings(content: &str) -> impl Iterator<Item = String> + '_ {
    content.lines().filter_map(normalize_workspace_heading_line)
}

fn normalize_workspace_heading_line(line: &str) -> Option<String> {
    let trimmed = line.trim_start();
    let heading_level = trimmed.chars().take_while(|ch| *ch == '#').count();
    if !(1..=6).contains(&heading_level) {
        return None;
    }

    let heading_text = trimmed.get(heading_level..)?.trim();
    if heading_text.is_empty() {
        return None;
    }

    Some(strip_heading_number_prefix(heading_text).trim().to_string())
}

fn strip_heading_number_prefix(text: &str) -> &str {
    let trimmed = text.trim_start();
    let Some(split_index) = trimmed
        .char_indices()
        .find(|(_, ch)| ch.is_whitespace())
        .map(|(index, _)| index)
    else {
        return trimmed;
    };

    let token = &trimmed[..split_index];
    if is_heading_number_token(token) {
        trimmed[split_index..].trim_start()
    } else {
        trimmed
    }
}

fn is_heading_number_token(token: &str) -> bool {
    if !token
        .chars()
        .any(|ch| matches!(ch, '.' | '、' | ')' | '）'))
    {
        return false;
    }

    let number = token.trim_end_matches(['.', '、', ')', '）']);
    !number.is_empty()
        && number
            .split('.')
            .all(|part| !part.is_empty() && part.chars().all(|ch| ch.is_ascii_digit()))
}

fn heading_matches(heading: &str, candidates: &[&str]) -> bool {
    candidates
        .iter()
        .any(|candidate| heading.eq_ignore_ascii_case(candidate))
}

fn heading_contains_component_api_data_bucket(heading: &str) -> bool {
    heading.contains("组件") && heading.contains("API") && heading.contains("数据模型")
}

fn parse_choice_option_line(line: &str) -> Option<ChoiceOptionData> {
    let line = normalize_choice_option_line(line);
    let mut chars = line.char_indices();
    let (_, first) = chars.next()?;
    if !first.is_ascii_alphanumeric() {
        return None;
    }
    let (delimiter_index, delimiter) = chars.next()?;
    if !matches!(delimiter, '.' | '、' | ')' | '）' | '．') {
        return None;
    }

    let label_start = delimiter_index + delimiter.len_utf8();
    let raw_label = line
        .get(label_start..)?
        .trim()
        .trim_start_matches('*')
        .trim_start_matches('_')
        .trim();
    if raw_label.is_empty() {
        return None;
    }

    let id = first.to_string().to_ascii_uppercase();
    Some(ChoiceOptionData {
        id: id.clone(),
        label: format!("{id}. {raw_label}"),
        description: None,
    })
}

fn normalize_choice_option_line(line: &str) -> String {
    let mut candidate = line.trim();
    if let Some(rest) = strip_markdown_list_marker(candidate) {
        candidate = rest;
    }
    candidate = candidate.trim_start();
    if let Some(rest) = candidate.strip_prefix("**") {
        candidate = rest;
    } else if let Some(rest) = candidate.strip_prefix("__") {
        candidate = rest;
    }
    candidate.trim_start().to_string()
}

fn strip_markdown_list_marker(line: &str) -> Option<&str> {
    let mut chars = line.char_indices();
    let (_, marker) = chars.next()?;
    if !matches!(marker, '-' | '*' | '+') {
        return None;
    }
    let (space_index, space) = chars.next()?;
    space
        .is_whitespace()
        .then(|| line[space_index + space.len_utf8()..].trim_start())
}

fn normalize_generation_prompt(content: String, workspace_type: &WorkspaceType) -> String {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        format!(
            "Workspace 类型: {}\n开始生成 {}",
            workspace_type_title(workspace_type),
            workspace_type_title(workspace_type)
        )
    } else {
        trimmed.to_string()
    }
}

fn build_artifact_retry_prompt(workspace_type: &WorkspaceType, previous_output: &str) -> String {
    let artifact_name = workspace_type_title(workspace_type);
    let mut prompt = format!(
        "上一轮已结束，但没有输出完整 artifact。\n\
         不要继续调研，不要只解释。\n\
         请基于已有上下文和刚才读取的文件，立即输出完整 ```artifact``` {artifact_name}。\n"
    );
    let previous_output = previous_output.trim();
    if !previous_output.is_empty() {
        prompt.push_str("\n上一轮可见输出:\n");
        prompt.push_str(previous_output);
        prompt.push('\n');
    }
    prompt
}

fn ws_stage(stage: &WorkspaceStage) -> WsWorkspaceStage {
    match stage {
        WorkspaceStage::PrepareContext => WsWorkspaceStage::PrepareContext,
        WorkspaceStage::Running => WsWorkspaceStage::Running,
        WorkspaceStage::AuthorConfirm => WsWorkspaceStage::AuthorConfirm,
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

fn provider_prompt_event(
    node_id: &str,
    prompt: String,
    detail: &'static str,
) -> ProviderExecutionEvent {
    ProviderExecutionEvent {
        event_id: format!("{node_id}_prompt"),
        kind: ProviderExecutionEventKind::Output,
        status: ProviderExecutionEventStatus::Started,
        title: "Provider Prompt".to_string(),
        detail: Some(detail.to_string()),
        command: None,
        cwd: None,
        output: Some(prompt),
        exit_code: None,
    }
}

fn execution_event_from_tool_call(call: ProviderToolCall) -> ProviderExecutionEvent {
    ProviderExecutionEvent {
        event_id: call.id,
        kind: ProviderExecutionEventKind::Command,
        status: ProviderExecutionEventStatus::Started,
        title: call.tool_name,
        detail: Some(format_tool_call_input(&call.input)),
        command: extract_tool_command(&call.input),
        cwd: None,
        output: None,
        exit_code: None,
    }
}

fn execution_event_from_tool_result(
    result: ProviderToolResult,
    title: String,
    command: Option<String>,
) -> ProviderExecutionEvent {
    ProviderExecutionEvent {
        event_id: result.tool_use_id,
        kind: ProviderExecutionEventKind::Command,
        status: if result.is_error {
            ProviderExecutionEventStatus::Failed
        } else {
            ProviderExecutionEventStatus::Completed
        },
        title,
        detail: None,
        command,
        cwd: None,
        output: Some(result.output),
        exit_code: if result.is_error { Some(1) } else { Some(0) },
    }
}

fn format_tool_call_input(input: &serde_json::Value) -> String {
    serde_json::to_string_pretty(input).unwrap_or_else(|_| input.to_string())
}

fn extract_tool_command(input: &serde_json::Value) -> Option<String> {
    let command = input.get("command").or_else(|| input.get("cmd"))?;
    if let Some(command) = command.as_str() {
        return Some(command.to_string());
    }
    command.as_array().and_then(|parts| {
        parts
            .iter()
            .map(serde_json::Value::as_str)
            .collect::<Option<Vec<_>>>()
            .map(|parts| parts.join(" "))
            .filter(|command| !command.trim().is_empty())
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
        WorkspaceStage::AuthorConfirm | WorkspaceStage::HumanConfirm => {
            WorkspaceSessionStatus::WaitingForHuman
        }
        WorkspaceStage::Completed => WorkspaceSessionStatus::Confirmed,
    }
}

fn latest_artifact_from_messages(messages: &[WorkspaceMessageRecord]) -> Option<String> {
    messages
        .iter()
        .rev()
        .find(|message| matches!(message.role.as_str(), "assistant" | "provider"))
        .map(|message| extract_artifact_content(&message.content))
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
    use crate::web::workspace_ws_types::{
        AuthorDecision, ReviewVerdictType, TimelineNodeStatus, TimelineNodeType,
    };
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
            superpowers_enabled: true,
            openspec_enabled: true,
            provider_conversations: Vec::new(),
            repository_path: None,
        }
    }

    fn empty_provider_commands() -> mpsc::Receiver<ProviderCommand> {
        let (_tx, rx) = mpsc::channel(8);
        rx
    }

    #[derive(Default)]
    struct SessionRecordingProvider {
        inputs: Arc<Mutex<Vec<StreamingProviderInput>>>,
        calls: Arc<Mutex<u32>>,
    }

    struct ImmediateOutputRecordingProvider {
        inputs: Arc<Mutex<Vec<StreamingProviderInput>>>,
        output: String,
    }

    #[async_trait::async_trait]
    impl StreamingProviderAdapter for ImmediateOutputRecordingProvider {
        async fn start(
            &self,
            input: StreamingProviderInput,
            _cancel: CancellationToken,
        ) -> Result<ProviderSession, ProviderAdapterError> {
            self.inputs.lock().unwrap().push(input);
            let output = self.output.clone();
            let (event_tx, event_rx) = mpsc::channel(8);
            let (command_tx, _command_rx) = mpsc::channel(8);
            tokio::spawn(async move {
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
                "run_streaming is not used by this test provider",
                0,
            ))
        }
    }

    #[async_trait::async_trait]
    impl StreamingProviderAdapter for SessionRecordingProvider {
        async fn start(
            &self,
            input: StreamingProviderInput,
            _cancel: CancellationToken,
        ) -> Result<ProviderSession, ProviderAdapterError> {
            self.inputs.lock().unwrap().push(input);
            let mut calls = self.calls.lock().unwrap();
            *calls += 1;
            let call_no = *calls;
            drop(calls);

            let (event_tx, event_rx) = mpsc::channel(8);
            let (command_tx, _command_rx) = mpsc::channel(8);
            tokio::spawn(async move {
                let output = if call_no == 1 {
                    "climb_stairs(n) 对 n <= 0 应该如何处理？\nA. 返回 0\nB. 抛出 ValueError\n"
                } else {
                    "# Story Spec\n\n## 功能需求\n- 对 n <= 0 返回 0。\n\n## 成功标准\n- n <= 0 时返回 0。\n"
                };
                let _ = event_tx
                    .send(ProviderEvent::Completed {
                        full_output: output.to_string(),
                        provider_session_id: Some("provider-author-session-1".to_string()),
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
                "run_streaming is not used by this test provider",
                0,
            ))
        }
    }

    #[tokio::test]
    async fn author_choice_followup_resumes_author_provider_session() {
        let (event_tx, _event_rx) = mpsc::channel(32);
        let mut session = make_session("sess_resume_author");
        session.workspace_type = WorkspaceType::Story;
        session.author_provider = ProviderName::Codex;
        session.reviewer_provider = None;
        let checkpoint_tmp = TempDir::new().unwrap();
        let mut engine = WorkspaceEngine::new(
            Arc::new(CheckpointStore::new(checkpoint_tmp.path().to_path_buf())),
            event_tx,
            session,
        );
        let provider = Arc::new(SessionRecordingProvider::default());

        let (_command_tx, command_rx) = mpsc::channel(8);
        engine
            .handle_user_message(
                "开始生成 Story Spec".to_string(),
                provider.clone(),
                command_rx,
            )
            .await;

        let prompt = engine
            .take_pending_author_choice_prompt("author_choice_msg_002", vec!["A".to_string()], None)
            .await
            .expect("pending author choice prompt");

        let (_command_tx2, command_rx2) = mpsc::channel(8);
        engine
            .handle_author_choice_followup_message(prompt.clone(), provider.clone(), command_rx2)
            .await;

        let inputs = provider.inputs.lock().unwrap();
        assert_eq!(inputs.len(), 2);
        assert_eq!(inputs[0].resume_provider_session_id, None);
        assert_eq!(
            inputs[1].resume_provider_session_id.as_deref(),
            Some("provider-author-session-1")
        );
        assert_eq!(inputs[1].prompt, prompt);
        assert!(
            inputs[1]
                .prompt
                .starts_with("用户回答了 author 的确认问题：")
        );
        assert!(!inputs[1].prompt.contains("[system]:"));
        assert!(!inputs[1].prompt.contains("[assistant]:"));
    }

    #[tokio::test]
    async fn claude_code_text_choice_output_uses_text_fallback_as_recovery_path() {
        let (_tmp, store) = setup();
        let (tx, mut rx) = mpsc::channel(64);
        let mut session = make_session("sess_claude_text_choice_fallback");
        session.author_provider = ProviderName::ClaudeCode;
        session.reviewer_provider = None;
        let mut engine = WorkspaceEngine::new(store, tx, session);

        engine
            .drive_provider_session(
                Ok(text_choice_provider_session(
                    "climb_stairs(n) 对 n <= 0 应该如何处理？\nA. 返回 0\nB. 抛出 ValueError\n",
                )),
                empty_provider_commands(),
                Some("timeline_node_author".to_string()),
                Some(ProviderName::ClaudeCode),
                ProviderConversationRole::Author,
                None,
            )
            .await;

        let events = drain_engine_events(&mut rx);
        assert!(
            events.iter().any(|event| {
                matches!(
                    event,
                    EngineEvent::ChoiceRequest {
                        prompt,
                        source,
                        ..
                    } if prompt.contains("n <= 0")
                        && *source == ChoiceRequestSource::TextFallback
                )
            }),
            "Claude Code 文本选择题应该作为兜底进入 text_fallback choice_request"
        );
        assert!(
            !events.iter().any(|event| {
                matches!(event, EngineEvent::ProtocolError { code, .. }
                    if code == "CLAUDE_CODE_STRUCTURED_QUESTION_REQUIRED")
            }),
            "Claude Code 可解析文本选择题不应该再被结构化提问 protocol error 拦截"
        );
        let prompt = engine
            .take_pending_author_choice_prompt("author_choice_msg_001", vec!["A".to_string()], None)
            .await
            .expect("pending Claude Code text fallback choice prompt");
        assert!(prompt.contains("用户回答了 author 的确认问题"));
        assert!(prompt.contains("A. 返回 0"));
    }

    #[test]
    fn provider_resume_session_id_is_isolated_by_role_and_provider() {
        let (event_tx, _event_rx) = mpsc::channel(8);
        let mut session = make_session("sess_role_isolation");
        session.author_provider = ProviderName::ClaudeCode;
        session.reviewer_provider = Some(ProviderName::ClaudeCode);
        session.provider_conversations = vec![ProviderConversationRef {
            role: ProviderConversationRole::Author,
            provider: ProviderName::ClaudeCode,
            provider_session_id: "author-session".to_string(),
            updated_at: "2026-06-01T00:00:00Z".to_string(),
            last_node_id: Some("node-author".to_string()),
        }];
        let checkpoint_tmp = TempDir::new().unwrap();
        let engine = WorkspaceEngine::new(
            Arc::new(CheckpointStore::new(checkpoint_tmp.path().to_path_buf())),
            event_tx,
            session,
        );

        assert_eq!(
            engine.provider_resume_session_id(
                ProviderConversationRole::Author,
                &ProviderName::ClaudeCode
            ),
            Some("author-session".to_string())
        );
        assert_eq!(
            engine.provider_resume_session_id(
                ProviderConversationRole::Reviewer,
                &ProviderName::ClaudeCode
            ),
            None
        );
        assert_eq!(
            engine
                .provider_resume_session_id(ProviderConversationRole::Author, &ProviderName::Codex),
            None
        );
    }

    #[test]
    fn design_artifact_gate_accepts_numbered_canonical_headings() {
        let content = r#"# Provider 依赖自检 Design Spec

## 1. 设计范围

本设计覆盖 provider 依赖自检与安装。

## 2. 设计决策

- [DEC-001] 新建 ProviderCatalog。

## 3. 组件 / API / 数据模型

- [CMP-001] ProviderCatalog。

## 4. 风险

无。
"#;

        assert!(content_has_complete_workspace_artifact(
            content,
            &WorkspaceType::Design
        ));
    }

    #[test]
    fn design_artifact_gate_rejects_legacy_key_decision_heading() {
        let content = r#"# Provider 依赖自检 Design Spec

## 1. 设计范围

本设计覆盖 provider 依赖自检与安装。

## 关键决策

- [DEC-001] 新建 ProviderCatalog。

## 组件 / API / 数据模型

- [CMP-001] ProviderCatalog。
"#;

        assert!(!content_has_complete_workspace_artifact(
            content,
            &WorkspaceType::Design
        ));
    }

    #[test]
    fn review_input_does_not_resume_prior_reviewer_provider_session() {
        let (event_tx, _event_rx) = mpsc::channel(8);
        let mut session = make_session("sess_review_no_resume");
        session.reviewer_provider = Some(ProviderName::Codex);
        session.artifact = Some("# Story Spec\n\n## 功能需求\n- [REQ-001] Draft.\n".to_string());
        session.provider_conversations = vec![ProviderConversationRef {
            role: ProviderConversationRole::Reviewer,
            provider: ProviderName::Codex,
            provider_session_id: "codex-review-thread-1".to_string(),
            updated_at: "2026-06-01T00:00:00Z".to_string(),
            last_node_id: Some("timeline_node_003".to_string()),
        }];
        let checkpoint_tmp = TempDir::new().unwrap();
        let engine = WorkspaceEngine::new(
            Arc::new(CheckpointStore::new(checkpoint_tmp.path().to_path_buf())),
            event_tx,
            session,
        );

        let input = engine.build_review_input().expect("review input");

        assert_eq!(input.resume_provider_session_id, None);
        assert!(input.prompt.contains("当前 Artifact"));
    }

    #[test]
    fn workspace_provider_inputs_use_one_hour_timeout() {
        let (event_tx, _event_rx) = mpsc::channel(8);
        let mut session = make_session("sess_workspace_timeout");
        session.artifact = Some("# Story Spec\n\n## 功能需求\n- [REQ-001] Draft.\n".to_string());
        let checkpoint_tmp = TempDir::new().unwrap();
        let mut engine = WorkspaceEngine::new(
            Arc::new(CheckpointStore::new(checkpoint_tmp.path().to_path_buf())),
            event_tx,
            session,
        );
        engine.latest_review_verdict = Some(ReviewVerdict {
            verdict: ReviewVerdictType::Revise,
            comments: "补充验收标准".to_string(),
            summary: "需要返修".to_string(),
        });

        assert_eq!(
            engine
                .build_streaming_input("开始生成", AuthorPromptMode::FullConversation)
                .expect("author input")
                .timeout_secs,
            3600
        );
        assert_eq!(
            engine
                .build_review_input()
                .expect("review input")
                .timeout_secs,
            3600
        );
        assert_eq!(
            engine
                .build_revision_input()
                .expect("revision input")
                .timeout_secs,
            3600
        );
    }

    #[test]
    fn review_input_keeps_current_artifact_and_context_without_old_assistant_artifacts() {
        let (event_tx, _event_rx) = mpsc::channel(8);
        let mut session = make_session("sess_review_prompt_dedupe");
        session.messages = vec![
            SessionMessage {
                id: "msg_001".to_string(),
                role: "system".to_string(),
                content: "系统上下文：真实 issue 描述。".to_string(),
                checkpoint_id: None,
                created_at: "2026-06-01T00:00:00Z".to_string(),
            },
            SessionMessage {
                id: "msg_002".to_string(),
                role: "user".to_string(),
                content: "用户补充：必须覆盖 n=10 -> 89。".to_string(),
                checkpoint_id: None,
                created_at: "2026-06-01T00:00:01Z".to_string(),
            },
            SessionMessage {
                id: "msg_003".to_string(),
                role: "assistant".to_string(),
                content: "# Old Story Spec\n\n## 功能需求\n- [REQ-OLD] 旧稿。\n\n## 成功标准\n- [AC-OLD] 旧验收。\n".to_string(),
                checkpoint_id: None,
                created_at: "2026-06-01T00:00:02Z".to_string(),
            },
        ];
        session.artifact = Some(
            "# Current Story Spec\n\n## 功能需求\n- [REQ-001] 当前稿。\n\n## 成功标准\n- [AC-001] 当前稿覆盖 n=10 -> 89。\n"
                .to_string(),
        );
        let checkpoint_tmp = TempDir::new().unwrap();
        let engine = WorkspaceEngine::new(
            Arc::new(CheckpointStore::new(checkpoint_tmp.path().to_path_buf())),
            event_tx,
            session,
        );

        let input = engine.build_review_input().expect("review input");

        assert!(input.prompt.contains("系统上下文：真实 issue 描述。"));
        assert!(input.prompt.contains("用户补充：必须覆盖 n=10 -> 89。"));
        assert_eq!(input.prompt.matches("# Current Story Spec").count(), 1);
        assert!(
            !input.prompt.contains("# Old Story Spec"),
            "review prompt should not include historical assistant artifact bodies: {}",
            input.prompt
        );
        assert!(
            input
                .prompt
                .contains("{\"verdict\":\"pass|revise|needs_human\"")
        );
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
    async fn permission_timeout_marks_node_detail_and_returns_to_prepare_context() {
        let (tmp, checkpoint_store) = setup();
        let lifecycle_store = LifecycleStore::new(ProductAppPaths::new(tmp.path().join(".aria")));
        let (engine_tx, mut engine_rx) = mpsc::channel(64);
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
        let mut engine = WorkspaceEngine::new_persistent(
            checkpoint_store,
            lifecycle_store.clone(),
            engine_tx,
            session,
        );
        let node_id = create_author_run_node(&mut engine).await;
        engine.mark_active_run_started("run-1");
        engine
            .persist_permission_request(
                &node_id,
                "permission_1".to_string(),
                serde_json::json!({"tool_name": "shell", "description": "cargo test"}),
            )
            .await
            .unwrap();

        let (provider_event_tx, provider_event_rx) = mpsc::channel(8);
        let (provider_command_tx, _provider_command_rx) = mpsc::channel(8);
        provider_event_tx
            .send(ProviderEvent::PermissionTimeout {
                permission_id: "permission_1".to_string(),
            })
            .await
            .unwrap();
        drop(provider_event_tx);

        engine
            .drive_provider_session(
                Ok(ProviderSession {
                    events: provider_event_rx,
                    commands: provider_command_tx,
                }),
                empty_provider_commands(),
                Some(node_id.clone()),
                Some(ProviderName::ClaudeCode),
                ProviderConversationRole::Author,
                None,
            )
            .await;

        let detail = lifecycle_store
            .load_node_detail(&engine.session().session_id, &node_id)
            .unwrap();
        assert_eq!(
            detail.permission_events[0]
                .response
                .as_ref()
                .and_then(|value| value.get("status"))
                .and_then(|value| value.as_str()),
            Some("timeout")
        );
        assert_eq!(detail.status, TimelineNodeStatus::Failed);
        assert_eq!(engine.current_stage(), WorkspaceStage::PrepareContext);
        assert_eq!(engine.active_run_id(), None);

        let mut saw_timeout_event = false;
        while let Ok(event) = engine_rx.try_recv() {
            if let EngineEvent::PermissionTimeout {
                permission_id,
                node_id: event_node_id,
            } = event
            {
                saw_timeout_event = permission_id == "permission_1"
                    && event_node_id.as_deref() == Some(node_id.as_str());
            }
        }
        assert!(saw_timeout_event);
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

        engine
            .handle_author_decision(AuthorDecision::Accept)
            .await
            .unwrap();

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
    async fn empty_start_generation_records_default_prompt_for_audit() {
        let (_tmp, lifecycle_store, mut engine) = persistent_test_engine();

        engine
            .handle_user_message(
                String::new(),
                Arc::new(FakeStreamingProvider),
                empty_provider_commands(),
            )
            .await;

        let user_message = engine
            .session()
            .messages
            .iter()
            .find(|message| message.role == "user")
            .expect("user prompt message");
        assert!(!user_message.content.trim().is_empty());
        assert!(user_message.content.contains("Story Spec"));

        let author_node = engine
            .timeline_nodes
            .iter()
            .find(|node| node.node_type == TimelineNodeType::AuthorRun)
            .expect("author run node");
        let detail = lifecycle_store
            .load_node_detail(&engine.session().session_id, &author_node.node_id)
            .expect("author run detail");
        let prompt = detail.prompt.as_ref().expect("prompt snapshot");
        assert!(prompt.contains("Workspace 类型: Story Spec"));
        assert!(prompt.contains(&user_message.content));
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

        engine
            .handle_author_decision(AuthorDecision::Accept)
            .await
            .unwrap();

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

    #[test]
    fn detect_author_choice_request_accepts_markdown_bold_bulleted_options() {
        let output = "感谢提供项目上下文。\n\n\
            在生成 Story Spec 之前，我有几个问题需要确认：\n\n\
            **问题 1：弹窗触发时机**\n\n\
            根据 Issue 描述，弹窗是在\"启动 aria 后\"触发。请问这里的\"启动 aria\"具体指什么时机？\n\n\
            - **A)** 用户运行 `aria` 命令启动 daemon 时（Rust 后端启动时）\n\
            - **B)** 用户打开 Web 工作台页面时（前端首次加载时）\n\
            - **C)** 两者都需要（后端启动时检测，前端展示弹窗）\n";

        let (prompt, options) = detect_author_choice_request(output, &WorkspaceType::Story)
            .expect("markdown bold bulleted options should become a choice request");

        assert!(prompt.contains("弹窗触发时机"));
        assert_eq!(options.len(), 3);
        assert_eq!(options[0].id, "A");
        assert!(options[0].label.contains("用户运行 `aria`"));
        assert_eq!(options[1].id, "B");
        assert_eq!(options[2].id, "C");
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
        engine
            .handle_author_decision(AuthorDecision::Accept)
            .await
            .unwrap();
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
    async fn single_review_round_revise_still_pauses_for_decision() {
        let (_tmp, store) = setup();
        let (tx, _) = mpsc::channel(64);
        let mut session = make_session("sess_single_review_revise");
        session.review_rounds = 1;
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
                        "需要移除非规范正文。\n\n```json\n{\"verdict\":\"revise\",\"summary\":\"需要返修\"}\n```",
                    provider_type: Arc::new(Mutex::new(None)),
                    prompt: Arc::new(Mutex::new(None)),
                }),
                empty_provider_commands(),
            )
            .await;

        assert_eq!(engine.session().stage, WorkspaceStage::ReviewDecision);
        let active_node = engine
            .timeline_nodes
            .iter()
            .find(|node| Some(&node.node_id) == engine.active_node_id.as_ref())
            .expect("active review decision node");
        assert_eq!(active_node.node_type, TimelineNodeType::ReviewDecision);
        assert_eq!(active_node.status, TimelineNodeStatus::Paused);
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
            .handle_author_decision(AuthorDecision::Accept)
            .await
            .unwrap();
        assert_eq!(engine.session().stage, WorkspaceStage::CrossReview);

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
        let revised_artifact = "# Story Spec\n\n\
            ## 功能需求\n\
            - [REQ-001] 补充失败路径后的版本。\n\n\
            ## 成功标准\n\
            - [AC-001] 覆盖失败路径。\n";
        engine
            .drive_revision_session(
                Arc::new(ReviewVerdictStreamingProvider {
                    output: revised_artifact,
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
        assert!(prompt.contains("用户补充信息优先级高于 Reviewer 审核意见"));
        assert!(prompt.contains("如二者冲突，以用户补充信息为准"));
        assert!(prompt.contains("请根据以上审核意见修改产物"));
        assert_eq!(
            engine.session().artifact.as_deref(),
            Some(revised_artifact.trim())
        );
        engine
            .handle_author_decision(AuthorDecision::Accept)
            .await
            .unwrap();
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
    async fn review_decision_with_context_requires_non_empty_context_for_all_workspace_types() {
        for workspace_type in [
            WorkspaceType::Story,
            WorkspaceType::Design,
            WorkspaceType::WorkItem,
        ] {
            let (_tmp, store) = setup();
            let (tx, _) = mpsc::channel(64);
            let mut session = make_session("sess_review_context_required");
            session.workspace_type = workspace_type.clone();
            session.stage = WorkspaceStage::ReviewDecision;
            let mut engine = WorkspaceEngine::new(store, tx, session);
            engine.latest_review_verdict = Some(ReviewVerdict {
                verdict: ReviewVerdictType::Revise,
                comments: "需要补充上下文后再返修。".to_string(),
                summary: "补充上下文".to_string(),
            });

            let result = engine
                .handle_review_decision(
                    "continue_with_context".to_string(),
                    Some("   ".to_string()),
                )
                .await;

            assert_eq!(
                result,
                Err("continue_with_context requires non-empty extra_context".to_string())
            );
            assert_eq!(engine.session().stage, WorkspaceStage::ReviewDecision);
            assert!(
                !engine
                    .timeline_nodes
                    .iter()
                    .any(|node| node.node_type == TimelineNodeType::Revision),
                "{workspace_type:?} should not create revision node without extra context"
            );
        }
    }

    #[tokio::test]
    async fn revision_input_uses_persisted_codex_author_session_when_engine_session_is_stale() {
        let (tmp, checkpoint_store) = setup();
        let lifecycle_store = LifecycleStore::new(ProductAppPaths::new(tmp.path().join(".aria")));
        let (tx, _) = mpsc::channel(64);
        let session_record = lifecycle_store
            .create_workspace_session(CreateWorkspaceSessionInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                entity_id: "story_spec_0001".to_string(),
                workspace_type: WorkspaceType::Story,
                author_provider: ProviderName::Codex,
                reviewer_provider: ProviderName::ClaudeCode,
                review_rounds: 1,
                superpowers_enabled: true,
                openspec_enabled: true,
            })
            .unwrap();
        lifecycle_store
            .replace_workspace_provider_conversations(
                &session_record.id,
                vec![ProviderConversationRef {
                    role: ProviderConversationRole::Author,
                    provider: ProviderName::Codex,
                    provider_session_id: "codex-author-session-1".to_string(),
                    updated_at: chrono::Utc::now().to_rfc3339(),
                    last_node_id: Some("timeline_node_002".to_string()),
                }],
            )
            .unwrap();

        let mut session = WorkspaceSession::from_record(session_record);
        session.stage = WorkspaceStage::Revision;
        session.artifact = Some(
            "# Story Spec\n\n## 功能需求\n- [REQ-001] 初版。\n\n## 成功标准\n- [AC-001] 初版可验收。\n"
                .to_string(),
        );
        session.messages.push(SessionMessage {
            id: "msg_001".to_string(),
            role: "system".to_string(),
            content: "很长的系统上下文，返修续接时不应重复发送。".to_string(),
            checkpoint_id: None,
            created_at: chrono::Utc::now().to_rfc3339(),
        });
        let mut engine =
            WorkspaceEngine::new_persistent(checkpoint_store, lifecycle_store, tx, session);
        engine.latest_review_verdict = Some(ReviewVerdict {
            verdict: ReviewVerdictType::Revise,
            comments: "需要补充 reviewer 指出的 API 字段。".to_string(),
            summary: "补 API 字段".to_string(),
        });

        let input = engine.build_revision_input().expect("revision input");

        assert_eq!(
            input.resume_provider_session_id.as_deref(),
            Some("codex-author-session-1")
        );
        assert!(input.prompt.contains("需要补充 reviewer 指出的 API 字段。"));
        assert!(input.prompt.contains("输出完整更新后的 artifact markdown"));
        assert!(!input.prompt.contains("会话上下文:"));
        assert!(!input.prompt.contains("[system]:"));
        assert!(!input.prompt.contains("上一版 Artifact"));
        assert!(!input.prompt.contains("# Story Spec"));
    }

    #[tokio::test]
    async fn revision_with_existing_author_provider_session_uses_delta_prompt() {
        let (_tmp, store) = setup();
        let (tx, _) = mpsc::channel(64);
        let mut session = make_session("sess_revision_delta_prompt");
        session.stage = WorkspaceStage::Revision;
        session.artifact = Some(
            "# Story Spec\n\n## 功能需求\n- [REQ-001] 初版。\n\n## 成功标准\n- [AC-001] 初版可验收。\n"
                .to_string(),
        );
        session.messages.push(SessionMessage {
            id: "msg_001".to_string(),
            role: "system".to_string(),
            content: "很长的系统上下文，返修续接时不应重复发送。".to_string(),
            checkpoint_id: None,
            created_at: chrono::Utc::now().to_rfc3339(),
        });
        session.messages.push(SessionMessage {
            id: "msg_002".to_string(),
            role: "assistant".to_string(),
            content: session.artifact.clone().expect("artifact"),
            checkpoint_id: None,
            created_at: chrono::Utc::now().to_rfc3339(),
        });
        session
            .provider_conversations
            .push(ProviderConversationRef {
                role: ProviderConversationRole::Author,
                provider: ProviderName::ClaudeCode,
                provider_session_id: "provider-author-session-1".to_string(),
                updated_at: chrono::Utc::now().to_rfc3339(),
                last_node_id: Some("timeline_node_002".to_string()),
            });
        let mut engine = WorkspaceEngine::new(store, tx, session);
        engine.latest_review_verdict = Some(ReviewVerdict {
            verdict: ReviewVerdictType::Revise,
            comments: "需要补充失败路径。".to_string(),
            summary: "补充失败路径".to_string(),
        });
        engine.pending_revision_context = Some("补充登录错误码".to_string());
        let captured_input = Arc::new(Mutex::new(None));

        engine
            .drive_revision_session(
                Arc::new(RevisionInputRecordingProvider {
                    input: captured_input.clone(),
                    output: "# Story Spec\n\n## 功能需求\n- [REQ-001] 补充失败路径。\n\n## 成功标准\n- [AC-001] 覆盖失败路径。\n",
                }),
                empty_provider_commands(),
            )
            .await;

        let input = captured_input
            .lock()
            .unwrap()
            .clone()
            .expect("revision provider input");
        assert_eq!(
            input.resume_provider_session_id.as_deref(),
            Some("provider-author-session-1")
        );
        assert!(input.prompt.contains("需要补充失败路径。"));
        assert!(input.prompt.contains("补充登录错误码"));
        assert!(
            input
                .prompt
                .contains("用户补充信息优先级高于 Reviewer 审核意见")
        );
        assert!(input.prompt.contains("如二者冲突，以用户补充信息为准"));
        assert!(input.prompt.contains("输出完整更新后的 artifact markdown"));
        assert!(!input.prompt.contains("会话上下文:"));
        assert!(!input.prompt.contains("[system]:"));
        assert!(!input.prompt.contains("上一版 Artifact"));
        assert!(!input.prompt.contains("# Story Spec"));
    }

    #[tokio::test]
    async fn revision_delta_prompt_includes_legacy_context_note() {
        let (_tmp, store) = setup();
        let (tx, _) = mpsc::channel(64);
        let mut session = make_session("sess_revision_delta_legacy_context_note");
        session.stage = WorkspaceStage::Revision;
        session.artifact = Some(
            "# Story Spec\n\n## 功能需求\n- [REQ-001] 初版。\n\n## 成功标准\n- [AC-001] 初版可验收。\n"
                .to_string(),
        );
        session
            .provider_conversations
            .push(ProviderConversationRef {
                role: ProviderConversationRole::Author,
                provider: ProviderName::ClaudeCode,
                provider_session_id: "provider-author-session-1".to_string(),
                updated_at: chrono::Utc::now().to_rfc3339(),
                last_node_id: Some("timeline_node_002".to_string()),
            });
        let mut engine = WorkspaceEngine::new(store, tx, session);
        engine.latest_review_verdict = Some(ReviewVerdict {
            verdict: ReviewVerdictType::Revise,
            comments: "需要补充验收值。".to_string(),
            summary: "补充验收值".to_string(),
        });
        engine
            .append_completed_timeline_event(
                TimelineNodeType::ContextNote,
                WorkspaceStage::PrepareContext,
                "上下文补充".to_string(),
                Some("旧现场补充：必须覆盖 n=10 -> 89。".to_string()),
                TimelineNodeStatus::Completed,
                false,
            )
            .await;

        let input = engine.build_revision_input().expect("revision input");

        assert_eq!(
            input.resume_provider_session_id.as_deref(),
            Some("provider-author-session-1")
        );
        assert!(
            input.prompt.contains("旧现场补充：必须覆盖 n=10 -> 89。"),
            "revision author prompt should include legacy context note, got: {}",
            input.prompt
        );
    }

    struct RevisionInputRecordingProvider {
        input: Arc<Mutex<Option<StreamingProviderInput>>>,
        output: &'static str,
    }

    #[async_trait::async_trait]
    impl StreamingProviderAdapter for RevisionInputRecordingProvider {
        async fn start(
            &self,
            input: StreamingProviderInput,
            _cancel: CancellationToken,
        ) -> Result<ProviderSession, ProviderAdapterError> {
            *self.input.lock().unwrap() = Some(input);
            let (event_tx, event_rx) = mpsc::channel(8);
            let (command_tx, _command_rx) = mpsc::channel(8);
            let output = self.output.to_string();
            tokio::spawn(async move {
                let _ = event_tx
                    .send(ProviderEvent::Completed {
                        full_output: output,
                        provider_session_id: Some("provider-author-session-1".to_string()),
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
        engine
            .handle_author_decision(AuthorDecision::Accept)
            .await
            .unwrap();
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
    async fn persistent_engine_keeps_open_stage_after_failed_running_node() {
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
                review_rounds: 1,
                superpowers_enabled: true,
                openspec_enabled: true,
            })
            .unwrap();
        let session_id = session_record.id.clone();
        let provider_config_snapshot = ProviderConfigSnapshot {
            author: session_record.author_provider.clone(),
            reviewer: Some(session_record.reviewer_provider.clone()),
            review_rounds: session_record.review_rounds,
        };
        lifecycle_store
            .save_timeline_nodes(
                &session_id,
                &[
                    TimelineNode {
                        node_id: "timeline_node_001".to_string(),
                        node_type: TimelineNodeType::StartGeneration,
                        agent: None,
                        stage: WsWorkspaceStage::PrepareContext,
                        round: None,
                        status: TimelineNodeStatus::Completed,
                        title: "开始生成".to_string(),
                        summary: None,
                        started_at: "2026-06-01T14:12:29Z".to_string(),
                        completed_at: Some("2026-06-01T14:12:29Z".to_string()),
                        duration_ms: Some(0),
                        artifact_ref: None,
                        provider_config_snapshot: provider_config_snapshot.clone(),
                    },
                    TimelineNode {
                        node_id: "timeline_node_002".to_string(),
                        node_type: TimelineNodeType::AuthorRun,
                        agent: Some(ProviderName::ClaudeCode),
                        stage: WsWorkspaceStage::Running,
                        round: None,
                        status: TimelineNodeStatus::Failed,
                        title: "Story Spec 生成".to_string(),
                        summary: Some("运行已中止".to_string()),
                        started_at: "2026-06-01T14:12:29Z".to_string(),
                        completed_at: Some("2026-06-01T14:12:36Z".to_string()),
                        duration_ms: None,
                        artifact_ref: None,
                        provider_config_snapshot,
                    },
                ],
            )
            .unwrap();

        let session = WorkspaceSession::from_record(
            lifecycle_store
                .get_workspace_session(&session_id)
                .expect("workspace session"),
        );
        let engine =
            WorkspaceEngine::new_persistent(checkpoint_store, lifecycle_store, tx, session);

        assert_eq!(engine.current_stage(), WorkspaceStage::PrepareContext);
        match engine.build_session_state() {
            WsOutMessage::SessionState { stage, .. } => {
                assert_eq!(stage, "prepare_context");
            }
            other => panic!("expected session_state, got {other:?}"),
        }
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
            prompt: Some("Workspace 类型: Story Spec".to_string()),
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
    async fn context_notes_are_included_in_author_prompt_for_all_workspace_types() {
        for (workspace_type, output) in [
            (
                WorkspaceType::Story,
                "# Story Spec\n\n## 功能需求\n- [REQ-001] 记录用户补充上下文。\n\n## 成功标准\n- [AC-001] author prompt 包含补充上下文。\n",
            ),
            (
                WorkspaceType::Design,
                "# Design Spec\n\n## 设计决策\n- [DEC-001] 使用用户补充上下文。\n\n## API 契约\n- 无新增 API。\n",
            ),
            (
                WorkspaceType::WorkItem,
                "# Work Item\n\n## 目标\n- 使用用户补充上下文。\n\n## 验证命令\n- cargo test --locked\n",
            ),
        ] {
            let (_tmp, store) = setup();
            let (tx, _) = mpsc::channel(64);
            let mut session = make_session("sess_context_note_prompt");
            session.workspace_type = workspace_type.clone();
            session.reviewer_provider = None;
            let mut engine = WorkspaceEngine::new(store, tx, session);
            let inputs = Arc::new(Mutex::new(Vec::new()));
            let provider = Arc::new(ImmediateOutputRecordingProvider {
                inputs: inputs.clone(),
                output: output.to_string(),
            });

            engine
                .append_context_note("用户补充：必须覆盖 n=10 -> 89。".to_string())
                .await
                .unwrap();
            engine
                .handle_user_message("开始生成".to_string(), provider, empty_provider_commands())
                .await;

            let inputs = inputs.lock().unwrap();
            let prompt = &inputs
                .first()
                .expect("author provider should receive input")
                .prompt;
            assert!(
                prompt.contains("用户补充：必须覆盖 n=10 -> 89。"),
                "{workspace_type:?} author prompt should include prepare context note, got: {prompt}"
            );
            assert!(
                prompt.contains("开始生成"),
                "{workspace_type:?} author prompt should include generation request, got: {prompt}"
            );
        }
    }

    #[tokio::test]
    async fn legacy_context_note_timeline_nodes_are_included_in_author_prompt() {
        let (_tmp, store) = setup();
        let (tx, _) = mpsc::channel(64);
        let mut session = make_session("sess_legacy_context_note_prompt");
        session.reviewer_provider = None;
        let mut engine = WorkspaceEngine::new(store, tx, session);
        let inputs = Arc::new(Mutex::new(Vec::new()));
        let provider = Arc::new(ImmediateOutputRecordingProvider {
            inputs: inputs.clone(),
            output: "# Story Spec\n\n## 功能需求\n- [REQ-001] 记录旧补充上下文。\n\n## 成功标准\n- [AC-001] author prompt 包含旧补充上下文。\n".to_string(),
        });

        engine
            .append_completed_timeline_event(
                TimelineNodeType::ContextNote,
                WorkspaceStage::PrepareContext,
                "上下文补充".to_string(),
                Some("旧现场补充：Story Spec 必须使用 n=10 -> 89。".to_string()),
                TimelineNodeStatus::Completed,
                false,
            )
            .await;
        engine
            .handle_user_message("开始生成".to_string(), provider, empty_provider_commands())
            .await;

        let inputs = inputs.lock().unwrap();
        let prompt = &inputs
            .first()
            .expect("author provider should receive input")
            .prompt;
        assert!(
            prompt.contains("旧现场补充：Story Spec 必须使用 n=10 -> 89。"),
            "author prompt should include legacy timeline-only context note, got: {prompt}"
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

    #[tokio::test]
    async fn author_completion_enters_author_confirm_for_all_workspace_types() {
        for (workspace_type, output) in [
            (
                WorkspaceType::Story,
                "# Story Spec\n\n## 功能需求\n- [REQ-001] 生成候选草稿。\n\n## 成功标准\n- [AC-001] 候选草稿可进入人工处理。\n",
            ),
            (
                WorkspaceType::Design,
                "# Design Spec\n\n## 设计决策\n- [DEC-001] 生成候选设计。\n\n## 公共组件\n- [CMP-001] 无新增组件。\n",
            ),
            (
                WorkspaceType::WorkItem,
                "# Work Item\n\n## 目标\n- 生成候选实施计划。\n\n## 验证命令\n- cargo test --locked\n",
            ),
        ] {
            let (_tmp, store) = setup();
            let (tx, _) = mpsc::channel(64);
            let mut session = make_session("sess_author_confirm");
            session.workspace_type = workspace_type.clone();
            session.reviewer_provider = Some(ProviderName::Codex);
            session.review_rounds = 1;
            let mut engine = WorkspaceEngine::new(store, tx, session);

            engine
                .handle_user_message(
                    "开始生成".to_string(),
                    Arc::new(ImmediateOutputRecordingProvider {
                        inputs: Arc::new(Mutex::new(Vec::new())),
                        output: output.to_string(),
                    }),
                    empty_provider_commands(),
                )
                .await;

            assert_eq!(
                engine.session().stage,
                WorkspaceStage::AuthorConfirm,
                "{workspace_type:?} should pause after author output"
            );
            assert!(
                engine
                    .timeline_nodes
                    .iter()
                    .any(|node| node.node_type == TimelineNodeType::AuthorConfirm
                        && node.status == TimelineNodeStatus::Active),
                "{workspace_type:?} should create an active author_confirm node"
            );
            assert!(
                !engine
                    .timeline_nodes
                    .iter()
                    .any(|node| node.node_type == TimelineNodeType::ReviewerRun),
                "{workspace_type:?} should not start reviewer before user accepts author output"
            );
            assert!(
                engine.session().artifact.is_some(),
                "{workspace_type:?} author output should remain visible while waiting for decision"
            );
        }
    }

    #[tokio::test]
    async fn author_decision_accept_starts_review_or_final_confirmation() {
        let (_tmp, store) = setup();
        let (tx, _) = mpsc::channel(64);
        let mut session = make_session("sess_author_accept_review");
        session.reviewer_provider = Some(ProviderName::Codex);
        session.review_rounds = 1;
        let mut engine = WorkspaceEngine::new(store, tx, session);
        engine
            .handle_user_message(
                "开始生成".to_string(),
                Arc::new(ImmediateOutputRecordingProvider {
                    inputs: Arc::new(Mutex::new(Vec::new())),
                    output: "# Story Spec\n\n## 功能需求\n- [REQ-001] 候选。\n\n## 成功标准\n- [AC-001] 可审核。\n".to_string(),
                }),
                empty_provider_commands(),
            )
            .await;

        engine
            .handle_author_decision(AuthorDecision::Accept)
            .await
            .unwrap();

        assert_eq!(engine.session().stage, WorkspaceStage::CrossReview);
        assert!(engine.timeline_nodes.iter().any(|node| {
            node.node_type == TimelineNodeType::ReviewerRun
                && node.status == TimelineNodeStatus::Active
        }));

        let (_tmp, store) = setup();
        let (tx, _) = mpsc::channel(64);
        let mut session = make_session("sess_author_accept_no_review");
        session.reviewer_provider = None;
        session.review_rounds = 0;
        let mut engine = WorkspaceEngine::new(store, tx, session);
        engine
            .handle_user_message(
                "开始生成".to_string(),
                Arc::new(ImmediateOutputRecordingProvider {
                    inputs: Arc::new(Mutex::new(Vec::new())),
                    output: "# Story Spec\n\n## 功能需求\n- [REQ-001] 候选。\n\n## 成功标准\n- [AC-001] 可确认。\n".to_string(),
                }),
                empty_provider_commands(),
            )
            .await;

        engine
            .handle_author_decision(AuthorDecision::Accept)
            .await
            .unwrap();

        assert_eq!(engine.session().stage, WorkspaceStage::HumanConfirm);
        assert!(engine.timeline_nodes.iter().any(|node| {
            node.node_type == TimelineNodeType::HumanConfirm
                && node.status == TimelineNodeStatus::Active
        }));
    }

    #[tokio::test]
    async fn author_decision_reject_returns_to_prepare_without_losing_history() {
        let (_tmp, store) = setup();
        let (tx, _) = mpsc::channel(64);
        let mut session = make_session("sess_author_reject");
        session.reviewer_provider = Some(ProviderName::Codex);
        session.review_rounds = 1;
        let mut engine = WorkspaceEngine::new(store, tx, session);
        engine
            .handle_user_message(
                "开始生成".to_string(),
                Arc::new(ImmediateOutputRecordingProvider {
                    inputs: Arc::new(Mutex::new(Vec::new())),
                    output: "# Story Spec\n\n## 功能需求\n- [REQ-001] 不满意的候选。\n\n## 成功标准\n- [AC-001] 需要重新写。\n".to_string(),
                }),
                empty_provider_commands(),
            )
            .await;

        engine
            .handle_author_decision(AuthorDecision::Reject)
            .await
            .unwrap();

        assert_eq!(engine.session().stage, WorkspaceStage::PrepareContext);
        assert_eq!(engine.session().artifact, None);
        assert!(
            engine
                .session()
                .messages
                .iter()
                .any(|message| message.role == "assistant"
                    && message.content.contains("不满意的候选")),
            "rejected author output should remain in message history"
        );
        assert_eq!(engine.artifact_versions.len(), 1);
        assert!(
            engine.artifact_versions[0]
                .markdown
                .contains("不满意的候选")
        );
        assert!(
            !engine.artifact_versions[0].is_current,
            "rejected artifact version should remain historical but not active"
        );
        assert!(
            engine.timeline_nodes.iter().any(|node| {
                node.node_type == TimelineNodeType::AuthorConfirm
                    && node.status == TimelineNodeStatus::Completed
                    && node.summary.as_deref() == Some("用户要求重新编写")
            }),
            "author_confirm node should record the rejection decision"
        );
    }

    #[tokio::test]
    async fn rejected_author_artifact_is_not_restored_after_reconnect() {
        let (tmp, lifecycle_store, mut engine) = persistent_test_engine();
        engine
            .handle_user_message(
                "开始生成".to_string(),
                Arc::new(ImmediateOutputRecordingProvider {
                    inputs: Arc::new(Mutex::new(Vec::new())),
                    output: "# Story Spec\n\n## 功能需求\n- [REQ-001] 被拒绝候选。\n\n## 成功标准\n- [AC-001] 不应恢复为当前稿。\n".to_string(),
                }),
                empty_provider_commands(),
            )
            .await;

        engine
            .handle_author_decision(AuthorDecision::Reject)
            .await
            .unwrap();

        let session_record = lifecycle_store
            .get_workspace_session(&engine.session().session_id)
            .unwrap();
        let reloaded = WorkspaceEngine::new_persistent(
            Arc::new(CheckpointStore::new(tmp.path().to_path_buf())),
            lifecycle_store,
            mpsc::channel(64).0,
            WorkspaceSession::from_record(session_record),
        );

        assert_eq!(reloaded.session().stage, WorkspaceStage::PrepareContext);
        assert_eq!(reloaded.session().artifact, None);
        match reloaded.build_session_state() {
            WsOutMessage::SessionState { artifact, .. } => assert_eq!(artifact, None),
            other => panic!("expected SessionState, got {other:?}"),
        }
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
                let output = "# Story Spec\n\n\
                    ## 功能需求\n\
                    - [REQ-001] 生成候选草稿。\n\n\
                    ## 成功标准\n\
                    - [AC-001] 候选草稿可进入审核。\n";
                let _ = event_tx
                    .send(ProviderEvent::TextDelta {
                        content: output.to_string(),
                    })
                    .await;
                let _ = event_tx
                    .send(ProviderEvent::Completed {
                        full_output: output.to_string(),
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
        assert_eq!(engine.session().stage, WorkspaceStage::AuthorConfirm);
        assert!(
            engine
                .session()
                .artifact
                .as_deref()
                .is_some_and(
                    |artifact| artifact.contains("## 功能需求") && artifact.contains("## 成功标准")
                )
        );

        let mut saw_artifact = false;
        let mut saw_author_confirm = false;
        while let Ok(event) = rx.try_recv() {
            match event {
                EngineEvent::ArtifactUpdate { markdown, .. }
                    if markdown.contains("## 功能需求") && markdown.contains("## 成功标准") =>
                {
                    saw_artifact = true;
                }
                EngineEvent::StageChange { stage } if stage == "author_confirm" => {
                    saw_author_confirm = true;
                }
                _ => {}
            }
        }
        assert!(
            saw_artifact,
            "provider completion should update the artifact pane"
        );
        assert!(
            saw_author_confirm,
            "provider completion should wait for author confirmation"
        );
    }

    #[tokio::test]
    async fn handle_user_message_uses_streamed_artifact_when_completed_output_is_summary() {
        let (_tmp, store) = setup();
        let (tx, mut rx) = mpsc::channel(64);
        let session = make_session("sess_streamed_artifact_summary");
        let mut engine = WorkspaceEngine::new(store, tx, session);

        engine
            .handle_user_message(
                "start".to_string(),
                Arc::new(StreamedArtifactSummaryProvider),
                empty_provider_commands(),
            )
            .await;

        engine
            .handle_author_decision(AuthorDecision::Accept)
            .await
            .unwrap();

        assert_eq!(engine.session().stage, WorkspaceStage::CrossReview);
        assert!(
            engine
                .session()
                .artifact
                .as_deref()
                .is_some_and(|artifact| artifact.contains("# Streamed Story Spec"))
        );
        assert!(
            drain_engine_events(&mut rx).iter().any(|event| matches!(
                event,
                EngineEvent::ArtifactUpdate { markdown, .. }
                    if markdown.contains("# Streamed Story Spec")
            )),
            "streamed artifact should be published even when Completed.full_output is only a summary"
        );
    }

    #[tokio::test]
    async fn handle_user_message_retries_once_when_design_author_completes_without_artifact() {
        let (_tmp, store) = setup();
        let (tx, mut rx) = mpsc::channel(64);
        let mut session = make_session("sess_design_artifact_retry");
        session.workspace_type = WorkspaceType::Design;
        session.entity_id = "design_spec_0001".to_string();
        session.reviewer_provider = None;
        session.review_rounds = 0;
        let mut engine = WorkspaceEngine::new(store, tx, session);
        let provider = Arc::new(DesignArtifactRetryProvider::default());

        engine
            .handle_user_message(
                "start".to_string(),
                provider.clone(),
                empty_provider_commands(),
            )
            .await;

        assert_eq!(*provider.calls.lock().unwrap(), 2);
        let inputs = provider.inputs.lock().unwrap();
        assert_eq!(inputs.len(), 2);
        assert!(
            inputs[1].prompt.contains("上一轮已结束")
                && inputs[1].prompt.contains("没有输出完整 artifact")
                && inputs[1]
                    .prompt
                    .contains("立即输出完整 ```artifact``` Design Spec"),
            "retry prompt should force a complete Design Spec artifact, got: {}",
            inputs[1].prompt
        );
        assert_eq!(
            inputs[1].resume_provider_session_id.as_deref(),
            Some("design-retry-session-1")
        );
        drop(inputs);

        assert_eq!(engine.session().stage, WorkspaceStage::AuthorConfirm);
        assert!(
            engine
                .session()
                .artifact
                .as_deref()
                .is_some_and(
                    |artifact| artifact.contains("## 设计决策") && artifact.contains("## 公共组件")
                )
        );
        assert!(
            drain_engine_events(&mut rx).iter().any(|event| matches!(
                event,
                EngineEvent::ArtifactUpdate { markdown, .. }
                    if markdown.contains("# Retried Design Spec")
            )),
            "retry artifact should be published"
        );
    }

    struct StreamedArtifactSummaryProvider;

    #[async_trait::async_trait]
    impl StreamingProviderAdapter for StreamedArtifactSummaryProvider {
        async fn start(
            &self,
            _input: StreamingProviderInput,
            _cancel: CancellationToken,
        ) -> Result<ProviderSession, ProviderAdapterError> {
            let (event_tx, event_rx) = mpsc::channel(8);
            let (command_tx, _command_rx) = mpsc::channel(8);
            tokio::spawn(async move {
                let streamed = "```artifact\n# Streamed Story Spec\n\n\
                    ## 功能需求\n\
                    - [REQ-001] 使用流式正文中的候选产物。\n\n\
                    ## 成功标准\n\
                    - [AC-001] Completed 摘要不含 artifact 时仍能进入审核。\n\
                    ```";
                let _ = event_tx
                    .send(ProviderEvent::TextDelta {
                        content: streamed.to_string(),
                    })
                    .await;
                let _ = event_tx
                    .send(ProviderEvent::Completed {
                        full_output: "Story Spec 候选已输出。等待 daemon 处理。".to_string(),
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

    #[derive(Default)]
    struct DesignArtifactRetryProvider {
        inputs: Arc<Mutex<Vec<StreamingProviderInput>>>,
        calls: Arc<Mutex<u32>>,
    }

    #[async_trait::async_trait]
    impl StreamingProviderAdapter for DesignArtifactRetryProvider {
        async fn start(
            &self,
            input: StreamingProviderInput,
            _cancel: CancellationToken,
        ) -> Result<ProviderSession, ProviderAdapterError> {
            self.inputs.lock().unwrap().push(input);
            let mut calls = self.calls.lock().unwrap();
            *calls += 1;
            let call_no = *calls;
            drop(calls);

            let (event_tx, event_rx) = mpsc::channel(8);
            let (command_tx, _command_rx) = mpsc::channel(8);
            tokio::spawn(async move {
                if call_no == 1 {
                    let output = "我先核对 reviewer 指出的几处代码锚点。\n";
                    let _ = event_tx
                        .send(ProviderEvent::TextDelta {
                            content: output.to_string(),
                        })
                        .await;
                    let _ = event_tx
                        .send(ProviderEvent::Completed {
                            full_output: output.to_string(),
                            provider_session_id: Some("design-retry-session-1".to_string()),
                        })
                        .await;
                    return;
                }

                let output = "```artifact\n# Retried Design Spec\n\n\
                    ## 设计决策\n\
                    - [DEC-001] 返修时直接输出完整设计产物。\n\n\
                    ## 公共组件\n\
                    - [CMP-001] ProviderDependencyDialog。\n\
                    ```";
                let _ = event_tx
                    .send(ProviderEvent::TextDelta {
                        content: output.to_string(),
                    })
                    .await;
                let _ = event_tx
                    .send(ProviderEvent::Completed {
                        full_output: output.to_string(),
                        provider_session_id: Some("design-retry-session-2".to_string()),
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

    fn tool_event_provider_session(full_output: &str) -> ProviderSession {
        let (event_tx, event_rx) = mpsc::channel(8);
        let (command_tx, _command_rx) = mpsc::channel(8);
        event_tx
            .try_send(ProviderEvent::ToolCall(
                crate::cross_cutting::streaming_provider::ProviderToolCall {
                    id: "tool_0001".to_string(),
                    tool_name: "edit_file".to_string(),
                    input: serde_json::json!({
                        "command": "apply_patch",
                        "path": "stairs.py"
                    }),
                },
            ))
            .expect("send tool call");
        event_tx
            .try_send(ProviderEvent::ToolResult(
                crate::cross_cutting::streaming_provider::ProviderToolResult {
                    tool_use_id: "tool_0001".to_string(),
                    output: "updated stairs.py".to_string(),
                    is_error: false,
                },
            ))
            .expect("send tool result");
        event_tx
            .try_send(ProviderEvent::Completed {
                full_output: full_output.to_string(),
                provider_session_id: None,
            })
            .expect("send completed");
        ProviderSession {
            events: event_rx,
            commands: command_tx,
        }
    }

    fn text_choice_provider_session(full_output: &str) -> ProviderSession {
        let (event_tx, event_rx) = mpsc::channel(8);
        let (command_tx, _command_rx) = mpsc::channel(8);
        event_tx
            .try_send(ProviderEvent::Completed {
                full_output: full_output.to_string(),
                provider_session_id: Some("provider-author-session-1".to_string()),
            })
            .expect("send completed");
        ProviderSession {
            events: event_rx,
            commands: command_tx,
        }
    }

    fn drain_engine_events(rx: &mut mpsc::Receiver<EngineEvent>) -> Vec<EngineEvent> {
        let mut events = Vec::new();
        while let Ok(event) = rx.try_recv() {
            events.push(event);
        }
        events
    }

    fn assert_tool_call_and_result_events(
        events: &[EngineEvent],
        expected_node_id: Option<&str>,
        expected_agent: ProviderName,
    ) {
        assert!(
            events.iter().any(|event| {
                matches!(
                    event,
                    EngineEvent::ExecutionEvent { event, node_id, agent }
                        if event.event_id == "tool_0001"
                            && event.kind == ProviderExecutionEventKind::Command
                            && event.status == ProviderExecutionEventStatus::Started
                            && event.title == "edit_file"
                            && event
                                .detail
                                .as_deref()
                                .is_some_and(|detail| detail.contains("stairs.py"))
                            && node_id.as_deref() == expected_node_id
                            && agent.as_ref() == Some(&expected_agent)
                )
            }),
            "expected visible tool call event, got {} engine events",
            events.len()
        );
        assert!(
            events.iter().any(|event| {
                matches!(
                    event,
                    EngineEvent::ExecutionEvent { event, node_id, agent }
                        if event.event_id == "tool_0001"
                            && event.kind == ProviderExecutionEventKind::Command
                            && event.status == ProviderExecutionEventStatus::Completed
                            && event.title == "edit_file"
                            && event.command.as_deref() == Some("apply_patch")
                            && event.output.as_deref() == Some("updated stairs.py")
                            && event.exit_code == Some(0)
                            && node_id.as_deref() == expected_node_id
                            && agent.as_ref() == Some(&expected_agent)
                )
            }),
            "expected visible tool result event, got {} engine events",
            events.len()
        );
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
                if event.event_id != "command_cmd_001" {
                    continue;
                }
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
    async fn handle_user_message_emits_provider_prompt_event() {
        let (_tmp, store) = setup();
        let (tx, mut rx) = mpsc::channel(64);
        let session = make_session("sess_007_prompt");
        let mut engine = WorkspaceEngine::new(store, tx, session);

        engine
            .handle_user_message(
                "start".to_string(),
                Arc::new(ExecutionEventStreamingProvider),
                empty_provider_commands(),
            )
            .await;

        let events = drain_engine_events(&mut rx);
        assert!(
            events.iter().any(|event| {
                matches!(
                    event,
                    EngineEvent::ExecutionEvent { event, node_id, agent }
                        if event.title == "Provider Prompt"
                            && event.kind == ProviderExecutionEventKind::Output
                            && event.output.as_deref().is_some_and(|output| output.contains("[user]: start"))
                            && node_id.as_deref().is_some_and(|id| id.starts_with("timeline_node_"))
                    && agent.as_ref() == Some(&ProviderName::ClaudeCode)
                )
            }),
            "expected provider prompt event"
        );
    }

    #[tokio::test]
    async fn provider_session_forwards_tool_call_and_result_events() {
        let (tmp, checkpoint_store) = setup();
        let lifecycle_store = LifecycleStore::new(ProductAppPaths::new(tmp.path().join(".aria")));
        let (tx, mut rx) = mpsc::channel(64);
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
        let mut engine =
            WorkspaceEngine::new_persistent(checkpoint_store, lifecycle_store.clone(), tx, session);
        let node_id = create_author_run_node(&mut engine).await;

        engine
            .drive_provider_session(
                Ok(tool_event_provider_session("# Draft")),
                empty_provider_commands(),
                Some(node_id.clone()),
                Some(ProviderName::ClaudeCode),
                ProviderConversationRole::Author,
                None,
            )
            .await;

        let events = drain_engine_events(&mut rx);
        assert_tool_call_and_result_events(
            &events,
            Some(node_id.as_str()),
            ProviderName::ClaudeCode,
        );

        let detail = lifecycle_store
            .load_node_detail(&engine.session().session_id, &node_id)
            .unwrap();
        assert!(
            detail
                .execution_events
                .iter()
                .any(|event| event["event_id"] == "tool_0001"
                    && event["status"] == "completed"
                    && event["output"] == "updated stairs.py"),
            "tool result should be persisted to node detail, got {detail:?}"
        );
    }

    #[tokio::test]
    async fn reviewer_provider_session_forwards_tool_call_and_result_events() {
        let (_tmp, store) = setup();
        let (tx, mut rx) = mpsc::channel(64);
        let session = make_session("sess_007_review_tools");
        let mut engine = WorkspaceEngine::new(store, tx, session);
        let node_id = create_reviewer_run_node(&mut engine).await;

        engine
            .drive_reviewer_provider_session(
                Ok(tool_event_provider_session(
                    r#"{"verdict":"pass","summary":"审核通过"}"#,
                )),
                empty_provider_commands(),
                ProviderName::Codex,
            )
            .await;

        let events = drain_engine_events(&mut rx);
        assert_tool_call_and_result_events(&events, Some(node_id.as_str()), ProviderName::Codex);
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

        engine
            .handle_author_decision(AuthorDecision::Accept)
            .await
            .unwrap();

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

    struct EmptyCompletedStreamingProvider;

    #[async_trait::async_trait]
    impl StreamingProviderAdapter for EmptyCompletedStreamingProvider {
        async fn start(
            &self,
            _input: StreamingProviderInput,
            _cancel: CancellationToken,
        ) -> Result<ProviderSession, ProviderAdapterError> {
            let (event_tx, event_rx) = mpsc::channel(8);
            let (command_tx, _command_rx) = mpsc::channel(8);
            tokio::spawn(async move {
                let _ = event_tx
                    .send(ProviderEvent::Completed {
                        full_output: String::new(),
                        provider_session_id: Some("empty-session".to_string()),
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

    struct InvalidArtifactStreamingProvider;

    #[async_trait::async_trait]
    impl StreamingProviderAdapter for InvalidArtifactStreamingProvider {
        async fn start(
            &self,
            _input: StreamingProviderInput,
            _cancel: CancellationToken,
        ) -> Result<ProviderSession, ProviderAdapterError> {
            let (event_tx, event_rx) = mpsc::channel(8);
            let (command_tx, _command_rx) = mpsc::channel(8);
            tokio::spawn(async move {
                let _ = event_tx
                    .send(ProviderEvent::Completed {
                        full_output: "我还需要继续分析，目前没有生成 Story Spec。".to_string(),
                        provider_session_id: Some("invalid-artifact-session".to_string()),
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
    async fn handle_user_message_rejects_non_artifact_author_output_without_review() {
        let (_tmp, store) = setup();
        let (tx, mut rx) = mpsc::channel(64);
        let session = make_session("sess_invalid_artifact");
        let mut engine = WorkspaceEngine::new(store, tx, session);

        engine
            .handle_user_message(
                "start".to_string(),
                Arc::new(InvalidArtifactStreamingProvider),
                empty_provider_commands(),
            )
            .await;

        assert_eq!(engine.session().stage, WorkspaceStage::PrepareContext);
        assert_eq!(engine.session().artifact, None);

        let events = drain_engine_events(&mut rx);
        assert!(
            events.iter().any(|event| matches!(
                event,
                EngineEvent::Error { message }
                    if message.contains("未返回有效的 Story Spec artifact")
            )),
            "invalid author output should emit an explicit artifact error"
        );
        assert!(
            !events.iter().any(|event| matches!(
                event,
                EngineEvent::StageChange { stage } if stage == "cross_review"
            )),
            "invalid author output must not start reviewer"
        );
    }

    #[tokio::test]
    async fn handle_user_message_empty_provider_output_marks_author_node_failed() {
        let (_tmp, store) = setup();
        let (tx, mut rx) = mpsc::channel(64);
        let session = make_session("sess_empty_output");
        let mut engine = WorkspaceEngine::new(store, tx, session);

        engine
            .handle_user_message(
                "start".to_string(),
                Arc::new(EmptyCompletedStreamingProvider),
                empty_provider_commands(),
            )
            .await;

        let author_node = engine
            .timeline_nodes
            .iter()
            .find(|node| node.node_type == TimelineNodeType::AuthorRun)
            .expect("author node");
        assert_eq!(author_node.status, TimelineNodeStatus::Failed);
        assert_eq!(
            author_node.summary.as_deref(),
            Some("Provider 未返回助手内容")
        );
        assert_eq!(engine.session().stage, WorkspaceStage::PrepareContext);
        assert_eq!(engine.session().messages.len(), 1);

        let mut saw_error = false;
        while let Ok(event) = rx.try_recv() {
            if let EngineEvent::Error { message } = event {
                saw_error = message == "Provider completed without assistant output";
            }
        }
        assert!(saw_error);
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
