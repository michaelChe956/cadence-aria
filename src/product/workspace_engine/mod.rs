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
    ChoiceAnswerData, ChoiceOptionData, ChoiceQuestionData, ChoiceRequestData, ChoiceRequestSource,
    ProviderCommand, ProviderCompletion, ProviderEvent, ProviderExecutionEvent,
    ProviderExecutionEventKind, ProviderExecutionEventStatus, ProviderPermissionMode,
    ProviderSession, ProviderStatus, ProviderToolCall, ProviderToolResult, RiskLevel,
    StreamingProviderAdapter, StreamingProviderInput,
};
use crate::cross_cutting::structured_output::{StructuredOutputError, StructuredOutputState};
use crate::product::artifact_extraction::extract_artifact_content;
use crate::product::checkpoint_store::CheckpointStore;
use crate::product::json_store::ProductStoreError;
use crate::product::lifecycle_store::{
    AppendSpecVersionInput, CreateWorkspaceSessionInput, IssueWorkItemPlanUpdate, LifecycleStore,
};
use crate::product::models::{
    AgentRole, ArtifactRef, DesignContextCapabilities, IssueWorkItemDependencyEdge,
    IssueWorkItemPlan, LifecycleConfirmationStatus, LifecycleWorkItemRecord, NodeDetail,
    OutlineContextBlockerResolution, OutlineContextIndex, PermissionEvent,
    PlanRepairSessionSnapshotDto, PlanRepairSessionStage, ProviderConversationRef,
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
use crate::product::work_item_revision_store::WorkItemRevisionStore;
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
    ChoiceQuestion, HumanConfirmDecision, NodeDetailSummary, ProviderConfigSnapshot,
    RecoverableInterruptedOperation, RecoverableInterruptedRun, RepositoryProfileDto,
    ReviewFinding, ReviewFindingSeverity, ReviewGate, ReviewVerdict, ReviewVerdictType,
    StructuredOutputDiagnostic, TimelineNode, TimelineNodeRetry, TimelineNodeRetryError,
    TimelineNodeStatus, TimelineNodeType, ValidatorFindingDto, VerificationCommandDto,
    VerificationManualCheckDto, VerificationPlanDto, WorkItemBatchDecisionDto,
    WorkItemBatchFailureSummaryDto, WorkItemBatchStatePayload, WorkItemCandidateDto,
    WorkItemCandidateMetaDto, WorkItemDependencyEdgeDto, WorkItemDraftCandidatePayload,
    WorkItemDraftDecisionDto, WorkItemGenerationModeDto, WorkItemPlanCandidateDto,
    WorkItemPlanCompileRecoveryActionDto, WorkItemPlanCompileReportPayload,
    WorkItemPlanContextBlockerDto, WorkItemPlanContextBlockerPayload, WorkItemPlanDto,
    WorkItemPlanOutlineCandidateDto, WorkItemPlanReviewAction, WorkItemPlanReviewAffectedItem,
    WorkItemPlanReviewComplete, WorkItemPlanReviewGate, WorkItemPlanReviewScope,
    WorkItemPlanReviewVerdict, WorkItemSplitOptionsDto, WorkspaceStage as WsWorkspaceStage,
    WsCheckpointDto, WsMessageDto, WsOutMessage, WsProviderConfig,
};

mod artifact_constraints;
mod author_confirm;
mod compile;
mod compile_parse;
mod controls;
mod decisions;
mod draft_batch;
mod human_presentation;
mod interrupted_run_recovery;
mod lifecycle;
mod lifecycle_recovery;
mod linked_workspace_amendment;
mod mappings;
mod parsers;
mod plan_outline;
mod plan_projection;
mod plan_repair;
mod plan_repair_artifacts;
mod plan_repair_recovery;
mod plan_repair_review;
mod plan_repair_transaction;
mod plan_repair_validation;
mod prompts;
mod provider_drive;
mod review;
mod session_state;
mod types;

#[cfg(test)]
mod tests;

pub use human_presentation::{
    HumanPresentationScope, SaveHumanPresentationRevision, save_human_presentation_revision,
};
pub use interrupted_run_recovery::{InterruptedRunRecoveryError, InterruptedRunRecoveryOutcome};
pub use linked_workspace_amendment::restore_linked_workspace_snapshot;
pub use plan_projection::{
    CompiledWorkItemRevision, InitialPlanCompileOutcome, WorkspaceEngineError,
    compile_plan_projection_bundle, compile_work_item_revision, plan_projection_input,
    publish_initial_plan_revision,
};
pub use types::{
    ArtifactUpdateEvent, AuthorDecisionOutcome, EngineEvent, LinkedWorkspaceAmendmentTarget,
    LinkedWorkspaceSessionSnapshot, PendingAuthorChoiceError, ReviewDecisionOutcome,
    SessionMessage, WorkItemBatchDecisionOutcome, WorkItemDraftDecisionOutcome,
    WorkItemPlanAuthorOutcome, WorkItemPlanCompileRecoveryOutcome, WorkspaceConfirmOutcome,
    WorkspaceEngine, WorkspaceSession, WorkspaceStage,
};

pub(crate) use artifact_constraints::*;
#[cfg(test)]
pub(crate) use compile::WorkItemPlanCompileFinalizerCheckpoint;
pub(crate) use compile_parse::*;
pub(crate) use lifecycle_recovery::*;
pub(crate) use mappings::*;
pub(crate) use parsers::*;
pub(crate) use plan_outline::*;
pub(crate) use plan_repair::{
    amendment_id_for, canonical_plan_repair_parent_session, linked_child_session,
};
pub(crate) use plan_repair_recovery::*;
pub(crate) use plan_repair_transaction::*;
pub(crate) use plan_repair_validation::*;
pub(crate) use prompts::*;
#[cfg(test)]
pub(crate) use review::{ReviewCompletionError, fallback_review_verdict};
pub(crate) use session_state::*;
pub(crate) use types::{
    ArtifactRetryContext, AuthorPromptMode, OutlineRevisionCrashPoint,
    OutlineRevisionPersistencePolicy, PendingAuthorChoice, PlanRepairCrashPoint,
    ProviderSessionDriveInput, ReviewProviderRunResult, RevisionResumeFallbackContext,
    StructuredOutputDisplayFilter, TimelineNodeDraft, WorkItemPlanCompileProjectionContext,
    WorkItemPlanOutlineRevisionSource,
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
