use super::*;

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
    pub artifact: Option<ArtifactPayload>,
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
        payload: ArtifactPayload,
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
        findings: Vec<ReviewFinding>,
        review_gate: ReviewGate,
        work_item_plan_review: Option<WorkItemPlanReviewComplete>,
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
    StartWorkItemPlanOutline,
    StartWorkItemPlanOutlineRevision {
        feedback: Option<String>,
    },
    StartWorkItemDraft {
        feedback: Option<String>,
    },
    StartWorkItemBatch,
    HumanConfirm,
    ConfirmedWithChildSessions {
        child_sessions: Vec<WorkspaceSessionRecord>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceConfirmOutcome {
    WorkItemPlan {
        child_sessions: Vec<WorkspaceSessionRecord>,
    },
    None,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthorDecisionOutcome {
    StartReview,
    HumanConfirm,
    PrepareContext,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkItemPlanAuthorOutcome {
    /// validate 通过（warnings 随 candidate 推送），进 AuthorConfirm。
    AuthorConfirm,
    /// validate 有 errors，需 handler 层重新调 WorkItemSplitEngine::generate 重生。
    /// findings 作为 revision feedback 注入重生 prompt。
    AutoRevision { findings: Vec<WorkItemSplitFinding> },
    /// 连续重生超阈值（3 次）仍 has_errors，交用户决策。
    HumanConfirm { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkItemDraftDecisionOutcome {
    StartDraftRun,
    StartReview,
    HumanConfirm,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkItemBatchDecisionOutcome {
    StartBatchRun,
    StartDraftRun,
    StartReview,
    HumanConfirm,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkItemPlanCompileRecoveryOutcome {
    Continue,
    HumanConfirm,
}

pub(crate) struct WorkItemPlanCompileProjectionContext<'a> {
    pub(crate) outline_order: &'a [String],
    pub(crate) outline_to_work_item_id: &'a BTreeMap<String, String>,
    pub(crate) outline_to_verification_plan_id: &'a BTreeMap<String, String>,
    pub(crate) repository_id: &'a str,
    pub(crate) now: &'a str,
}

pub struct WorkspaceEngine {
    pub(crate) checkpoint_store: Arc<CheckpointStore>,
    pub(crate) lifecycle_store: Option<LifecycleStore>,
    pub(crate) event_tx: mpsc::Sender<EngineEvent>,
    pub(crate) session: WorkspaceSession,
    pub(crate) cancel: CancellationToken,
    pub(crate) timeline_nodes: Vec<TimelineNode>,
    pub(crate) active_node_id: Option<String>,
    pub(crate) artifact_versions: Vec<ArtifactVersion>,
    pub(crate) latest_review_verdict: Option<ReviewVerdict>,
    pub(crate) pending_revision_context: Option<String>,
    pub(crate) pending_author_choice: Option<PendingAuthorChoice>,
    pub(crate) active_run_id: Option<String>,
    pub(crate) stream_buffers: HashMap<String, PendingStreamBuffer>,
    pub(crate) work_item_plan_author_retry_count: u32,
    pub(crate) work_item_plan_revision_retry_count: u32,
    pub(crate) work_item_batch_retry_counts: HashMap<String, u32>,
}

#[derive(Debug, Clone)]
pub(crate) struct PendingAuthorChoice {
    pub(crate) id: String,
    pub(crate) prompt: String,
    pub(crate) options: Vec<ChoiceOptionData>,
    pub(crate) source_node_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AuthorPromptMode {
    FullConversation,
    DeltaOnly,
}

impl AuthorPromptMode {
    pub(crate) fn prompt_event_detail(self) -> &'static str {
        match self {
            Self::FullConversation => "发送给 Workspace provider 的完整提示词",
            Self::DeltaOnly => "发送给 Workspace provider 的追加提示词",
        }
    }
}

pub(crate) struct ArtifactRetryContext {
    pub(crate) provider: Arc<dyn StreamingProviderAdapter>,
    pub(crate) input: StreamingProviderInput,
    pub(crate) attempted: bool,
}

pub(crate) struct RevisionResumeFallbackContext {
    pub(crate) provider: Arc<dyn StreamingProviderAdapter>,
    pub(crate) attempted: bool,
}

pub(crate) struct ProviderSessionDriveInput {
    pub(crate) session:
        Result<ProviderSession, crate::cross_cutting::provider_adapter::ProviderAdapterError>,
    pub(crate) command_rx: mpsc::Receiver<ProviderCommand>,
    pub(crate) node_id: Option<String>,
    pub(crate) agent: Option<ProviderName>,
    pub(crate) role: ProviderConversationRole,
    pub(crate) artifact_retry: Option<ArtifactRetryContext>,
    pub(crate) revision_resume_fallback: Option<RevisionResumeFallbackContext>,
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

pub(crate) struct PendingStreamBuffer {
    pub(crate) content: String,
    pub(crate) last_flush_at: Instant,
}

impl Default for PendingStreamBuffer {
    fn default() -> Self {
        Self {
            content: String::new(),
            last_flush_at: Instant::now(),
        }
    }
}

pub(crate) struct StructuredOutputDisplayFilter {
    pub(crate) pending: String,
    pub(crate) inside_structured_output: bool,
}

impl StructuredOutputDisplayFilter {
    pub(crate) fn new() -> Self {
        Self {
            pending: String::new(),
            inside_structured_output: false,
        }
    }

    pub(crate) fn push(&mut self, chunk: &str) -> String {
        self.pending.push_str(chunk);
        let mut visible = String::new();

        loop {
            if self.inside_structured_output {
                if let Some(end_index) = self.pending.find(STRUCTURED_OUTPUT_END) {
                    let drain_end = end_index + STRUCTURED_OUTPUT_END.len();
                    self.pending.drain(..drain_end);
                    self.inside_structured_output = false;
                    continue;
                }

                let keep = longest_suffix_prefix_len(&self.pending, STRUCTURED_OUTPUT_END);
                if self.pending.len() > keep {
                    self.pending.drain(..self.pending.len() - keep);
                }
                break;
            }

            if let Some(start_index) = self.pending.find(STRUCTURED_OUTPUT_START) {
                visible.push_str(&self.pending[..start_index]);
                let drain_end = start_index + STRUCTURED_OUTPUT_START.len();
                self.pending.drain(..drain_end);
                self.inside_structured_output = true;
                continue;
            }

            let keep = longest_suffix_prefix_len(&self.pending, STRUCTURED_OUTPUT_START);
            if self.pending.len() > keep {
                let emit: String = self.pending.drain(..self.pending.len() - keep).collect();
                visible.push_str(&emit);
            }
            break;
        }

        visible
    }

    pub(crate) fn finish(&mut self) -> String {
        if self.inside_structured_output {
            self.pending.clear();
            String::new()
        } else {
            std::mem::take(&mut self.pending)
        }
    }
}

pub(crate) fn longest_suffix_prefix_len(value: &str, pattern: &str) -> usize {
    let max_len = value.len().min(pattern.len().saturating_sub(1));
    (1..=max_len)
        .rev()
        .find(|len| value.ends_with(&pattern[..*len]))
        .unwrap_or(0)
}

pub(crate) struct TimelineNodeDraft {
    pub(crate) node_type: TimelineNodeType,
    pub(crate) agent: Option<ProviderName>,
    pub(crate) stage: WorkspaceStage,
    pub(crate) round: Option<u32>,
    pub(crate) title: String,
    pub(crate) summary: Option<String>,
    pub(crate) status: TimelineNodeStatus,
}
