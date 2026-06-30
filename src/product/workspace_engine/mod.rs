use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::cross_cutting::provider_adapter::{
    DEFAULT_PROVIDER_TIMEOUT_SECS, STRUCTURED_OUTPUT_END, STRUCTURED_OUTPUT_START,
};
use crate::cross_cutting::streaming_provider::{
    ChoiceOptionData, ChoiceRequestSource, ProviderCommand, ProviderEvent, ProviderExecutionEvent,
    ProviderExecutionEventKind, ProviderExecutionEventStatus, ProviderPermissionMode,
    ProviderSession, ProviderStatus, ProviderToolCall, ProviderToolResult, RiskLevel,
    StreamingProviderAdapter, StreamingProviderInput,
};
use crate::product::artifact_extraction::extract_artifact_content;
use crate::product::checkpoint_store::CheckpointStore;
use crate::product::json_store::ProductStoreError;
use crate::product::lifecycle_store::{
    AppendSpecVersionInput, CreateVerificationPlanInput, CreateWorkItemInput,
    CreateWorkspaceSessionInput, IssueWorkItemPlanUpdate, LifecycleStore,
};
use crate::product::models::{
    AgentRole, ArtifactRef, DesignContextCapabilities, IssueWorkItemDependencyEdge,
    IssueWorkItemPlan, LifecycleConfirmationStatus, LifecycleWorkItemRecord, NodeDetail,
    OutlineContextBlockerResolution, OutlineContextIndex, PermissionEvent, ProviderConversationRef,
    ProviderConversationRole, ProviderName, ProviderSnapshot, RepositoryProfileConfidence,
    VerificationCommand, VerificationCommandSafety, VerificationCommandSource,
    VerificationFallbackPolicy, VerificationManualCheck, VerificationPlan, VerificationScope,
    WorkItemBatchRecord, WorkItemBatchStatus, WorkItemDraftCandidate, WorkItemDraftRecord,
    WorkItemDraftStatus, WorkItemDraftSupersedeReason, WorkItemGenerationMode,
    WorkItemPlanCommitState, WorkItemPlanCompileStatus, WorkItemPlanCompileTransaction,
    WorkItemPlanDraftActiveIndex, WorkItemPlanOutline, WorkItemPlanStatus, WorkItemSplitFinding,
    WorkItemSplitFindingSeverity, WorkspaceMessageRecord, WorkspaceSessionRecord,
    WorkspaceSessionStatus, WorkspaceType,
};
use crate::product::work_item_plan_store::{
    WorkItemPlanStore, copy_draft_for_current_round, mark_draft_active,
    mark_draft_record_superseded, next_batch_id, next_draft_id, next_generation_round_id,
};
use crate::product::work_item_split_engine::{
    OutlineAuthorOutput, RedoSpec, WorkItemPlanContextBlocker, WorkItemSplitProviderOutput,
    build_work_item_draft_invocation,
};
use crate::product::work_item_split_validator::{
    WorkItemDraftLocalValidator, WorkItemPlanOutlineValidator, WorkItemSplitValidator,
};
use crate::protocol::contracts::{AdapterRole, ProviderType};
use crate::web::types::GenerateWorkItemsRequest;
use crate::web::workspace_ws_types::{
    ArtifactPayload, ArtifactVersion, ArtifactVersionSummary, AuthorDecision, ChoiceOption,
    HumanConfirmDecision, NodeDetailSummary, ProviderConfigSnapshot, RepositoryProfileDto,
    ReviewFinding, ReviewFindingSeverity, ReviewGate, ReviewVerdict, ReviewVerdictType,
    TimelineNode, TimelineNodeRetry, TimelineNodeRetryError, TimelineNodeStatus, TimelineNodeType,
    ValidatorFindingDto, VerificationCommandDto, VerificationManualCheckDto, VerificationPlanDto,
    WorkItemBatchDecisionDto, WorkItemBatchFailureSummaryDto, WorkItemBatchStatePayload,
    WorkItemCandidateDto, WorkItemCandidateMetaDto, WorkItemDependencyEdgeDto,
    WorkItemDraftCandidatePayload, WorkItemDraftDecisionDto, WorkItemGenerationModeDto,
    WorkItemPlanCandidateDto, WorkItemPlanCompileRecoveryActionDto,
    WorkItemPlanCompileReportPayload, WorkItemPlanContextBlockerDto,
    WorkItemPlanContextBlockerPayload, WorkItemPlanDto, WorkItemPlanOutlineCandidateDto,
    WorkItemPlanReviewAction, WorkItemPlanReviewAffectedItem, WorkItemPlanReviewComplete,
    WorkItemPlanReviewGate, WorkItemPlanReviewScope, WorkItemPlanReviewVerdict,
    WorkItemSplitOptionsDto, WorkspaceStage as WsWorkspaceStage, WsCheckpointDto, WsMessageDto,
    WsOutMessage, WsProviderConfig,
};

mod artifact_constraints;
mod author_confirm;
mod compile;
mod controls;
mod decisions;
mod draft_batch;
mod lifecycle;
mod mappings;
mod parsers;
mod plan_outline;
mod prompts;
mod provider_drive;
mod review;
mod session_state;
mod types;

#[cfg(test)]
mod tests;

pub use types::{
    AuthorDecisionOutcome, EngineEvent, PendingAuthorChoiceError, ReviewDecisionOutcome,
    SessionMessage, WorkItemBatchDecisionOutcome, WorkItemDraftDecisionOutcome,
    WorkItemPlanAuthorOutcome, WorkItemPlanCompileRecoveryOutcome, WorkspaceConfirmOutcome,
    WorkspaceEngine, WorkspaceSession, WorkspaceStage,
};

pub(crate) use artifact_constraints::*;
pub(crate) use compile::*;
pub(crate) use mappings::*;
pub(crate) use parsers::*;
pub(crate) use plan_outline::*;
pub(crate) use prompts::*;
pub(crate) use session_state::*;
pub(crate) use types::{
    ArtifactRetryContext, AuthorPromptMode, PendingAuthorChoice, ProviderSessionDriveInput,
    RevisionResumeFallbackContext, StructuredOutputDisplayFilter, TimelineNodeDraft,
    WorkItemPlanCompileProjectionContext,
};

const SUMMARY_PREVIEW_CHARS: usize = 2048;
const CODEX_RESUME_STALL_ERROR_MARKER: &str = "Codex resume stalled before provider progress";

pub(crate) fn preview(value: &str) -> String {
    value.chars().take(SUMMARY_PREVIEW_CHARS).collect()
}

pub(crate) fn is_codex_resume_stall_failure(message: &str) -> bool {
    message.contains(CODEX_RESUME_STALL_ERROR_MARKER)
}

pub(crate) fn serialized_string<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_else(|| "unknown".to_string())
}
