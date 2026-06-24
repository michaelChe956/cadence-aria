use super::*;

pub const TESTING_RESULT_REVIEW_REASON_CODE: &str = "testing_result_review_required";

pub(crate) const REWORK_CONTEXT_NOTE_CHAR_LIMIT: usize = 10_000;

#[derive(Debug, Error)]
pub enum CodingWorkspaceEngineError {
    #[error(transparent)]
    Store(#[from] ProductStoreError),
    #[error(transparent)]
    Git(#[from] GitWorkspaceError),
    #[error(transparent)]
    TestExecutor(#[from] TestExecutorError),
    #[error(transparent)]
    TesterAgent(#[from] crate::product::tester_agent_loop::TesterAgentError),
    #[error(transparent)]
    ProviderAdapter(#[from] ProviderAdapterError),
    #[error("coding_provider_stream_failed: {0}")]
    ProviderStream(String),
    #[error("coding_aborted")]
    Aborted,
    #[error("coding_rework_limit_exceeded: {0}")]
    ReworkLimitExceeded(String),
    #[error("coding_review_request_missing: {0}")]
    MissingReviewRequest(String),
    #[error("coding_attempt_missing_worktree: {0}")]
    MissingWorktree(String),
    #[error("coding_attempt_not_ready_for_final_confirm: {0}")]
    FinalConfirmNotReady(String),
    #[error("{0}")]
    NoReviewableChanges(String),
    #[error("shared_worktree_dirty_manual_gate: {0}")]
    SharedWorktreeDirtyManualGate(String),
    #[error("work_item_execution_plan_not_confirmed: {0}")]
    ExecutionPlanNotConfirmed(String),
    #[error("completion_commit_missing: {0}")]
    CompletionCommitMissing(String),
    #[error("work_item_handoff_missing: {0}")]
    WorkItemHandoffMissing(String),
    #[error("verification_gate_result_missing: {0}")]
    VerificationGateResultMissing(String),
    #[error("verification_gate_failed: {0}")]
    VerificationGateFailed(String),
    #[error("work_item_diff_scope_violation: {0}")]
    WorkItemDiffScopeViolation(String),
}

#[derive(Debug, Clone)]
pub struct CompletionGateReport;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CodingExecutionContext {
    pub work_item_markdown: Option<String>,
    pub verification_commands: Vec<String>,
}

pub(crate) struct CodingProviderStreamRun<'a> {
    pub(crate) attempt: &'a CodingExecutionAttempt,
    pub(crate) node_id: &'a str,
    pub(crate) role_run: Option<&'a CodingRoleRun>,
    pub(crate) provider: &'a dyn StreamingProviderAdapter,
    pub(crate) legacy_input: &'a AdapterInput,
    pub(crate) input: StreamingProviderInput,
    pub(crate) provider_name: &'a ProviderName,
    pub(crate) provider_role: CodingProviderRole,
    pub(crate) command_rx: &'a mut mpsc::Receiver<CodingRunnerCommand>,
    pub(crate) allow_legacy_stream_fallback: bool,
    pub(crate) timeout: Option<Duration>,
    pub(crate) timeout_reason_code: Option<&'static str>,
}

pub(crate) struct BlockedTestingGateContext<'a> {
    pub(crate) reason_code: String,
    pub(crate) description: String,
    pub(crate) raw_provider_output_ref: Option<String>,
    pub(crate) role_run: Option<&'a CodingRoleRun>,
}

pub(crate) fn run_timeout_sleep(
    timeout: Option<Duration>,
) -> Pin<Box<dyn Future<Output = ()> + Send>> {
    match timeout {
        Some(duration) => Box::pin(tokio::time::sleep(duration)),
        None => Box::pin(std::future::pending()),
    }
}

pub(crate) fn provider_conversation_role_for_coding_role(
    role: &CodingProviderRole,
) -> ProviderConversationRole {
    match role {
        CodingProviderRole::Coder => ProviderConversationRole::Coder,
        CodingProviderRole::Tester => ProviderConversationRole::Tester,
        CodingProviderRole::Analyst => ProviderConversationRole::Analyst,
        CodingProviderRole::CodeReviewer => ProviderConversationRole::CodeReviewer,
        CodingProviderRole::InternalReviewer => ProviderConversationRole::InternalReviewer,
    }
}

pub(crate) fn should_resume_provider_conversation(role: &CodingProviderRole) -> bool {
    matches!(role, CodingProviderRole::Coder)
}

pub(crate) fn coding_provider_permission_mode(
    mode: CodingProviderPermissionMode,
) -> ProviderPermissionMode {
    match mode {
        CodingProviderPermissionMode::Auto => ProviderPermissionMode::Auto,
        CodingProviderPermissionMode::Supervised => ProviderPermissionMode::Supervised,
    }
}

pub(crate) fn role_permission_mode_for_attempt(
    store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
    role: CodingProviderRole,
) -> Result<ProviderPermissionMode, CodingWorkspaceEngineError> {
    let snapshot = store.get_role_provider_config_snapshot(
        &attempt.project_id,
        &attempt.issue_id,
        &attempt.id,
    )?;
    Ok(coding_provider_permission_mode(
        snapshot.permission_mode_for_role(&role),
    ))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CodingPromptMode {
    FullConversation,
    DeltaOnly,
}

impl CodingPromptMode {
    pub(crate) fn event_detail(self) -> &'static str {
        match self {
            Self::FullConversation => "发送给 Coding provider 的完整提示词",
            Self::DeltaOnly => "发送给 Coding provider 的增量提示词",
        }
    }
}

#[derive(Clone)]
pub struct CodingWorkspaceEngine {
    pub(crate) store: CodingAttemptStore,
    pub(crate) _git_service: GitWorkspaceService,
    pub(crate) provider: Option<Arc<dyn ProviderAdapter + Send + Sync>>,
    pub(crate) event_tx: mpsc::Sender<CodingWsOutMessage>,
}

impl std::fmt::Debug for CodingWorkspaceEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CodingWorkspaceEngine")
            .field("store", &self.store)
            .field("event_tx", &self.event_tx)
            .finish_non_exhaustive()
    }
}
