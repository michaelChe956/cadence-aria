use super::*;
use std::sync::Arc;

pub(crate) const REWORK_CONTEXT_NOTE_CHAR_LIMIT: usize = 10_000;

#[derive(Debug, Error)]
pub enum CodingWorkspaceEngineError {
    #[error(transparent)]
    Store(#[from] ProductStoreError),
    #[error(transparent)]
    Git(#[from] GitWorkspaceError),
    #[error(transparent)]
    ProviderAdapter(#[from] ProviderAdapterError),
    #[error("coding_provider_stream_failed: {0}")]
    ProviderStream(String),
    #[error("coding_provider_protocol_error: {0}")]
    ProviderProtocol(String),
    #[error("group_review_blocked: {reason_code}; gate_id={gate_id:?}")]
    GroupReviewBlocked {
        reason_code: String,
        gate_id: Option<String>,
    },
    #[error("group_review_shard_stale_audit")]
    GroupReviewShardStaleAudit,
    #[error("group_review_reduction_stale")]
    GroupReviewReductionStale,
    #[error("group_review_executor_transport: {0}")]
    GroupReviewExecutorTransport(String),
    #[error("group_review_executor_internal: {0}")]
    GroupReviewExecutorInternal(String),
    #[error("group_review_git_fact_error: {0}")]
    GroupReviewGitFact(String),
    #[error("group_review_material_error: {0}")]
    GroupReviewMaterial(String),
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
    /// 多仓路径首次访问仓维 worktree 前发现旧 `issue-shared-worktree.json`
    /// （迁移契约化 §4.2.6 断言 fail-closed）。稳定码 `legacy_shared_worktree_present`。
    #[error("legacy_shared_worktree_present: {0}")]
    LegacySharedWorktreePresent(String),
    #[error("work_item_execution_plan_not_confirmed: {0}")]
    ExecutionPlanNotConfirmed(String),
    #[error("completion_commit_missing: {0}")]
    CompletionCommitMissing(String),
    #[error("work_item_handoff_missing: {0}")]
    WorkItemHandoffMissing(String),
    #[error("verification_gate_failed: {0}")]
    VerificationGateFailed(String),
    #[error("work_item_diff_scope_violation: {0}")]
    WorkItemDiffScopeViolation(String),
    /// 交付前统一门（Task 15）：任一 provider run 的越界检测基线被破坏（他仓主
    /// checkout HEAD/工作区变更或基线缺失）时阻断 commit/push，稳定码经
    /// `StableCode::as_str()` 转字符串填充。
    #[error("cross_target_delivery_blocked: {0}")]
    CrossTargetDeliveryBlocked(String),
    /// C-4 证据查询脚本注入失败（worktree `.aria/bin/aria-evidence-query` 写入失败）。
    #[error("coding_evidence_script_injection_failed: {0}")]
    EvidenceScriptInjection(String),
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
    /// 组级审查拥有失败状态收口权时，stream 层只返回 transport 错误，不直接
    /// 修改 attempt、role run 或 timeline 状态。
    pub(crate) suppress_failure_side_effects: bool,
    /// Task 11:逻辑代码库真实 provider 启动的绑定 validated policy input。
    /// 非空时,`provider_stream` 经 `LogicalCodebaseProviderGateway::start_streaming`
    /// 启动,而非直接 `provider.start`;传统/非逻辑路径为 `None`,保留既有行为。
    pub(crate) validated_input:
        Option<crate::cross_cutting::session_launch::ValidatedStreamingProviderInput>,
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

pub(crate) fn coding_permission_mode_for_provider_type(
    provider_type: &ProviderType,
    configured_mode: CodingProviderPermissionMode,
) -> ProviderPermissionMode {
    permission_mode_for_provider_type(
        provider_type,
        coding_provider_permission_mode(configured_mode),
    )
}

pub(crate) fn coding_permission_mode_for_provider(
    provider: &ProviderName,
    configured_mode: CodingProviderPermissionMode,
) -> ProviderPermissionMode {
    coding_permission_mode_for_provider_type(&provider_type_for_name(provider), configured_mode)
}

pub(crate) fn normalize_coding_permission_mode_for_provider(
    provider: &ProviderName,
    configured_mode: CodingProviderPermissionMode,
) -> CodingProviderPermissionMode {
    match coding_permission_mode_for_provider(provider, configured_mode) {
        ProviderPermissionMode::Auto => CodingProviderPermissionMode::Auto,
        ProviderPermissionMode::Supervised => CodingProviderPermissionMode::Supervised,
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
    Ok(coding_permission_mode_for_provider(
        snapshot.provider_for_role(&role),
        snapshot.permission_mode_for_role(&role),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coding_permission_mode_for_provider_type_forces_pi_to_auto() {
        assert_eq!(
            coding_permission_mode_for_provider_type(
                &ProviderType::Pi,
                CodingProviderPermissionMode::Supervised,
            ),
            ProviderPermissionMode::Auto
        );
    }

    #[test]
    fn coding_permission_mode_for_provider_type_preserves_non_pi_mode() {
        assert_eq!(
            coding_permission_mode_for_provider_type(
                &ProviderType::ClaudeCode,
                CodingProviderPermissionMode::Supervised,
            ),
            ProviderPermissionMode::Supervised
        );
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CodingPromptMode {
    FullConversation,
    DeltaOnly,
}

#[derive(Clone)]
pub(crate) struct CancellableCodingEventSender {
    sender: mpsc::Sender<CodingWsOutMessage>,
    cancellation: CancellationToken,
}

impl CancellableCodingEventSender {
    pub(crate) fn new(
        sender: mpsc::Sender<CodingWsOutMessage>,
        cancellation: CancellationToken,
    ) -> Self {
        Self {
            sender,
            cancellation,
        }
    }

    pub(crate) async fn send(
        &self,
        event: CodingWsOutMessage,
    ) -> Result<(), mpsc::error::SendError<CodingWsOutMessage>> {
        let permit = tokio::select! {
            biased;
            _ = self.cancellation.cancelled() => {
                return Err(mpsc::error::SendError(event));
            }
            permit = self.sender.reserve() => permit,
        };
        match permit {
            Ok(permit) => {
                permit.send(event);
                Ok(())
            }
            Err(_) => Err(mpsc::error::SendError(event)),
        }
    }

    pub(crate) fn raw_sender(&self) -> &mpsc::Sender<CodingWsOutMessage> {
        &self.sender
    }
}

impl std::fmt::Debug for CancellableCodingEventSender {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CancellableCodingEventSender")
            .field("sender", &self.sender)
            .field("cancelled", &self.cancellation.is_cancelled())
            .finish()
    }
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
    pub(crate) event_tx: CancellableCodingEventSender,
    pub(crate) cancellation: CancellationToken,
    /// Task 11:逻辑代码库真实 provider 启动的唯一入口。非空时,provider stream
    /// 经 `start_streaming` 启动并留 audit;为 `None` 时(传统单仓/非逻辑 issue)
    /// 保留直接 `provider.start` 路径,防止本工作包扩大旧 API 行为。Web 接入 task
    /// 为逻辑代码库 issue 注入真实 gateway。
    pub(crate) logical_provider_gateway:
        Option<Arc<crate::product::logical_codebase::LogicalCodebaseProviderGateway>>,
}

impl CodingWorkspaceEngine {
    pub(crate) fn ensure_not_cancelled(&self) -> Result<(), CodingWorkspaceEngineError> {
        if self.cancellation.is_cancelled() {
            return Err(CodingWorkspaceEngineError::Aborted);
        }
        Ok(())
    }

    pub(crate) fn cancellation_token(&self) -> &CancellationToken {
        &self.cancellation
    }
}

impl std::fmt::Debug for CodingWorkspaceEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CodingWorkspaceEngine")
            .field("store", &self.store)
            .field("event_tx", &self.event_tx)
            .finish_non_exhaustive()
    }
}
