use std::collections::BTreeMap;
use std::sync::Arc;

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::cross_cutting::streaming_provider::{
    ProviderCommand, ProviderEvent, ProviderPermissionMode, ProviderSession, ProviderStatus,
    RiskLevel, StreamingProviderAdapter, StreamingProviderInput,
};
use crate::product::checkpoint_store::CheckpointStore;
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::models::{
    LifecycleConfirmationStatus, ProviderName, WorkItemPlanStatus, WorkspaceMessageRecord,
    WorkspaceSessionRecord, WorkspaceSessionStatus, WorkspaceType,
};
use crate::protocol::contracts::{AdapterRole, ProviderType};
use crate::web::workspace_ws_types::{
    WsCheckpointDto, WsMessageDto, WsOutMessage, WsProviderConfig,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceStage {
    PrepareContext,
    Running,
    CrossReview,
    HumanConfirm,
    Completed,
}

impl WorkspaceStage {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::PrepareContext => "prepare_context",
            Self::Running => "running",
            Self::CrossReview => "cross_review",
            Self::HumanConfirm => "human_confirm",
            Self::Completed => "completed",
        }
    }

    pub fn from_stage_name(s: &str) -> Option<Self> {
        match s {
            "prepare_context" => Some(Self::PrepareContext),
            "running" => Some(Self::Running),
            "cross_review" => Some(Self::CrossReview),
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
    },
    MessageComplete {
        message_id: String,
        checkpoint_id: String,
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
    Error {
        message: String,
    },
}

pub struct WorkspaceEngine {
    checkpoint_store: Arc<CheckpointStore>,
    lifecycle_store: Option<LifecycleStore>,
    event_tx: mpsc::Sender<EngineEvent>,
    session: WorkspaceSession,
    cancel: CancellationToken,
}

impl WorkspaceEngine {
    pub fn new(
        checkpoint_store: Arc<CheckpointStore>,
        event_tx: mpsc::Sender<EngineEvent>,
        session: WorkspaceSession,
    ) -> Self {
        Self {
            checkpoint_store,
            lifecycle_store: None,
            event_tx,
            session,
            cancel: CancellationToken::new(),
        }
    }

    pub fn new_persistent(
        checkpoint_store: Arc<CheckpointStore>,
        lifecycle_store: LifecycleStore,
        event_tx: mpsc::Sender<EngineEvent>,
        session: WorkspaceSession,
    ) -> Self {
        Self {
            checkpoint_store,
            lifecycle_store: Some(lifecycle_store),
            event_tx,
            session,
            cancel: CancellationToken::new(),
        }
    }

    pub fn session(&self) -> &WorkspaceSession {
        &self.session
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
            self.transition_stage(WorkspaceStage::Running).await;
        }

        let input = match self.build_streaming_input(&content) {
            Ok(input) => input,
            Err(message) => {
                let _ = self.event_tx.send(EngineEvent::Error { message }).await;
                self.finish_failed_run().await;
                return;
            }
        };

        let session = provider.start(input, self.cancel.clone()).await;
        self.drive_provider_session(session, command_rx).await;
    }

    async fn drive_provider_session(
        &mut self,
        session: Result<
            ProviderSession,
            crate::cross_cutting::provider_adapter::ProviderAdapterError,
        >,
        mut command_rx: mpsc::Receiver<ProviderCommand>,
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
                    self.finish_failed_run().await;
                    return;
                }
                command = command_rx.recv(), if commands_open => {
                    match command {
                        Some(ProviderCommand::Abort) => {
                            let _ = session.commands.send(ProviderCommand::Abort).await;
                            cancel.cancel();
                            self.finish_failed_run().await;
                            return;
                        }
                        Some(command) => {
                            if session.commands.send(command).await.is_err() {
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
                            full_content.push_str(&content);
                            let _ = self
                                .event_tx
                                .send(EngineEvent::StreamChunk {
                                    role: "assistant".to_string(),
                                    content,
                                })
                                .await;
                        }
                        ProviderEvent::PermissionRequest(request) => {
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
                        ProviderEvent::Completed { full_output, .. } => {
                            self.complete_assistant_message(assistant_msg_id, full_output).await;
                            return;
                        }
                        ProviderEvent::Failed { message } => {
                            let _ = self
                                .event_tx
                                .send(EngineEvent::Error { message })
                                .await;
                            self.finish_failed_run().await;
                            return;
                        }
                    }
                }
            }
        }

        if cancel.is_cancelled() {
            self.finish_failed_run().await;
            return;
        }

        if full_content.is_empty() {
            self.finish_failed_run().await;
        } else {
            self.complete_assistant_message(assistant_msg_id, full_content)
                .await;
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

        let _ = self
            .event_tx
            .send(EngineEvent::MessageComplete {
                message_id: assistant_msg_id,
                checkpoint_id,
            })
            .await;
        self.transition_stage(WorkspaceStage::CrossReview).await;
        self.transition_stage(WorkspaceStage::HumanConfirm).await;
        if let Some(store) = &self.lifecycle_store {
            let _ = store.update_workspace_session_status(
                &self.session.session_id,
                WorkspaceSessionStatus::WaitingForHuman,
            );
        }
    }

    fn build_streaming_input(&self, user_content: &str) -> Result<StreamingProviderInput, String> {
        let working_dir =
            std::env::current_dir().map_err(|error| format!("working directory error: {error}"))?;

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
        self.session.artifact = Some(markdown);
        let _ = self
            .event_tx
            .send(EngineEvent::ArtifactUpdate {
                version: 1,
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
        }
    }
}

fn provider_type_for_name(provider: &ProviderName) -> ProviderType {
    match provider {
        ProviderName::ClaudeCode => ProviderType::ClaudeCode,
        ProviderName::Codex => ProviderType::Codex,
        ProviderName::Fake => ProviderType::Fake,
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
        WorkspaceStage::Running | WorkspaceStage::CrossReview => WorkspaceSessionStatus::Running,
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
    use crate::cross_cutting::streaming_provider::{FakeStreamingProvider, StreamChunk};
    use crate::protocol::contracts::{AdapterInput, ProviderType};
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
        }
    }

    fn empty_provider_commands() -> mpsc::Receiver<ProviderCommand> {
        let (_tx, rx) = mpsc::channel(8);
        rx
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
        assert_eq!(engine.session().stage, WorkspaceStage::HumanConfirm);
        assert_eq!(engine.session().messages.len(), 2); // user + assistant
        assert_eq!(engine.session().messages[0].role, "user");
        assert_eq!(engine.session().messages[1].role, "assistant");
        assert!(engine.session().messages[1].checkpoint_id.is_some());
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
        assert_eq!(engine.session().stage, WorkspaceStage::HumanConfirm);
        assert_eq!(engine.session().artifact.as_deref(), Some("# Draft"));

        let mut saw_artifact = false;
        let mut saw_human_confirm = false;
        while let Ok(event) = rx.try_recv() {
            match event {
                EngineEvent::ArtifactUpdate { markdown, .. } if markdown == "# Draft" => {
                    saw_artifact = true;
                }
                EngineEvent::StageChange { stage } if stage == "human_confirm" => {
                    saw_human_confirm = true;
                }
                _ => {}
            }
        }
        assert!(
            saw_artifact,
            "provider completion should update the artifact pane"
        );
        assert!(
            saw_human_confirm,
            "provider completion should unlock human confirmation"
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
        assert_eq!(engine.session().stage, WorkspaceStage::HumanConfirm);
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
