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
    TimelineNode, TimelineNodeStatus, TimelineNodeType, ValidatorFindingDto,
    VerificationCommandDto, VerificationManualCheckDto, VerificationPlanDto,
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

const SUMMARY_PREVIEW_CHARS: usize = 2048;
const CODEX_RESUME_STALL_ERROR_MARKER: &str = "Codex resume stalled before provider progress";

fn preview(value: &str) -> String {
    value.chars().take(SUMMARY_PREVIEW_CHARS).collect()
}

fn is_codex_resume_stall_failure(message: &str) -> bool {
    message.contains(CODEX_RESUME_STALL_ERROR_MARKER)
}

fn serialized_string<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_else(|| "unknown".to_string())
}

fn work_item_plan_outline_topological_order(
    outline: &WorkItemPlanOutline,
) -> Result<Vec<String>, String> {
    let outline_ids: Vec<String> = outline
        .work_item_outlines
        .iter()
        .map(|item| item.outline_id.clone())
        .collect();
    let known_ids: HashSet<String> = outline_ids.iter().cloned().collect();
    let mut indegree: HashMap<String, usize> = outline_ids
        .iter()
        .map(|outline_id| (outline_id.clone(), 0))
        .collect();
    let mut outgoing: HashMap<String, Vec<String>> = HashMap::new();

    for edge in &outline.dependency_graph {
        if !known_ids.contains(&edge.from_outline_id) {
            return Err(format!(
                "dependency edge references missing from_outline_id `{}`",
                edge.from_outline_id
            ));
        }
        if !known_ids.contains(&edge.to_outline_id) {
            return Err(format!(
                "dependency edge references missing to_outline_id `{}`",
                edge.to_outline_id
            ));
        }
        *indegree.entry(edge.to_outline_id.clone()).or_default() += 1;
        outgoing
            .entry(edge.from_outline_id.clone())
            .or_default()
            .push(edge.to_outline_id.clone());
    }

    let mut queue: VecDeque<String> = outline_ids
        .iter()
        .filter(|outline_id| indegree.get(*outline_id).copied().unwrap_or_default() == 0)
        .cloned()
        .collect();
    let mut order = Vec::with_capacity(outline_ids.len());

    while let Some(outline_id) = queue.pop_front() {
        order.push(outline_id.clone());
        if let Some(next_ids) = outgoing.get(&outline_id) {
            for next_id in next_ids {
                let Some(count) = indegree.get_mut(next_id) else {
                    continue;
                };
                *count -= 1;
                if *count == 0 {
                    queue.push_back(next_id.clone());
                }
            }
        }
    }

    if order.len() != outline_ids.len() {
        return Err("dependency graph contains a cycle".to_string());
    }

    Ok(order)
}

fn current_work_item_batch(
    index: &WorkItemPlanDraftActiveIndex,
) -> Result<&WorkItemBatchRecord, String> {
    index
        .batches
        .iter()
        .rev()
        .find(|batch| {
            batch.generation_round_id == index.current_generation_round_id
                && batch.mode == WorkItemGenerationMode::Batch
                && batch.status == WorkItemBatchStatus::Generating
        })
        .or_else(|| {
            index.batches.iter().rev().find(|batch| {
                batch.generation_round_id == index.current_generation_round_id
                    && batch.mode == WorkItemGenerationMode::Batch
            })
        })
        .ok_or_else(|| "current work item batch record is missing".to_string())
}

fn work_item_plan_findings_summary(prefix: &str, findings: &[WorkItemSplitFinding]) -> String {
    let errors = findings
        .iter()
        .filter(|finding| finding.severity == WorkItemSplitFindingSeverity::Error)
        .count();
    let warnings = findings
        .iter()
        .filter(|finding| finding.severity == WorkItemSplitFindingSeverity::Warning)
        .count();
    format!("{prefix}（errors: {errors}, warnings: {warnings}）")
}

fn work_item_draft_status_label(status: &WorkItemDraftStatus) -> &'static str {
    match status {
        WorkItemDraftStatus::Draft => "draft",
        WorkItemDraftStatus::Accepted => "accepted",
        WorkItemDraftStatus::Superseded => "superseded",
        WorkItemDraftStatus::ValidationFailed => "validation_failed",
    }
}

fn next_compile_id() -> String {
    format!("compile_{}", chrono::Utc::now().format("%Y%m%d%H%M%S%3f"))
}

fn compile_work_item_id(compile_id: &str, index: usize) -> String {
    format!("work_item_{compile_id}_{:03}", index + 1)
}

fn compile_verification_plan_id(compile_id: &str, index: usize) -> String {
    format!("verification_plan_{compile_id}_{:03}", index + 1)
}

fn parse_compile_verification_scope(value: Option<&str>) -> VerificationScope {
    match value.unwrap_or_default() {
        "unit" => VerificationScope::Unit,
        "integration" => VerificationScope::Integration,
        "e2e" => VerificationScope::E2e,
        "build" => VerificationScope::Build,
        "lint" => VerificationScope::Lint,
        "manual" => VerificationScope::Manual,
        _ => VerificationScope::Custom,
    }
}

fn parse_compile_confidence(value: Option<&str>) -> RepositoryProfileConfidence {
    match value.unwrap_or("high") {
        "low" => RepositoryProfileConfidence::Low,
        "medium" => RepositoryProfileConfidence::Medium,
        _ => RepositoryProfileConfidence::High,
    }
}

fn parse_compile_fallback_policy(value: Option<&str>) -> VerificationFallbackPolicy {
    match value.unwrap_or("manual_gate") {
        "repair_provider_output" => VerificationFallbackPolicy::RepairProviderOutput,
        _ => VerificationFallbackPolicy::ManualGate,
    }
}

fn parse_compile_safety(value: Option<&str>) -> VerificationCommandSafety {
    match value.unwrap_or("approved") {
        "needs_manual_review" => VerificationCommandSafety::NeedsManualReview,
        _ => VerificationCommandSafety::Approved,
    }
}

fn json_string_array(value: Option<&serde_json::Value>) -> Vec<String> {
    value
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn parse_compile_verification_plan(
    value: &serde_json::Value,
    id: String,
    project_id: String,
    issue_id: String,
    work_item_id: String,
    now: String,
) -> VerificationPlan {
    let commands = value
        .get("commands")
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .enumerate()
                .map(|(index, command)| VerificationCommand {
                    id: command
                        .get("id")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_string)
                        .unwrap_or_else(|| format!("cmd_{:03}", index + 1)),
                    label: command
                        .get("label")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("验证命令")
                        .to_string(),
                    command: command
                        .get("command")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    cwd: command
                        .get("cwd")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    purpose: command
                        .get("purpose")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    required: command
                        .get("required")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false),
                    timeout_seconds: command
                        .get("timeout_seconds")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(120),
                    source: VerificationCommandSource::Provider,
                    safety: parse_compile_safety(
                        command.get("safety").and_then(serde_json::Value::as_str),
                    ),
                })
                .collect()
        })
        .unwrap_or_default();
    let manual_checks = value
        .get("manual_checks")
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .enumerate()
                .map(|(index, check)| VerificationManualCheck {
                    id: check
                        .get("id")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_string)
                        .unwrap_or_else(|| format!("manual_{:03}", index + 1)),
                    label: check
                        .get("label")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("人工检查")
                        .to_string(),
                    instructions: check
                        .get("instructions")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    required: check
                        .get("required")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false),
                })
                .collect()
        })
        .unwrap_or_default();

    VerificationPlan {
        id,
        project_id,
        issue_id,
        work_item_id,
        repository_profile_ref: None,
        provider_run_ref: None,
        scope: parse_compile_verification_scope(
            value.get("scope").and_then(serde_json::Value::as_str),
        ),
        commands,
        manual_checks,
        required_gates: json_string_array(value.get("required_gates")),
        risk_notes: json_string_array(value.get("risk_notes")),
        confidence: parse_compile_confidence(
            value.get("confidence").and_then(serde_json::Value::as_str),
        ),
        fallback_policy: parse_compile_fallback_policy(
            value
                .get("fallback_policy")
                .and_then(serde_json::Value::as_str),
        ),
        created_at: now.clone(),
        updated_at: now,
    }
}

fn work_item_split_findings_to_dto(findings: &[WorkItemSplitFinding]) -> Vec<ValidatorFindingDto> {
    findings
        .iter()
        .map(|finding| ValidatorFindingDto {
            severity: finding.severity.as_str().to_string(),
            code: finding.code.clone(),
            message: finding.message.clone(),
            work_item_ids: finding.work_item_ids.clone(),
        })
        .collect()
}

fn work_item_plan_context_blockers_to_dto(
    blockers: &[WorkItemPlanContextBlocker],
) -> Vec<WorkItemPlanContextBlockerDto> {
    blockers
        .iter()
        .map(|blocker| WorkItemPlanContextBlockerDto {
            code: blocker.code.clone(),
            message: blocker.message.clone(),
            needed_context: blocker.needed_context.clone(),
        })
        .collect()
}

fn build_work_item_plan_candidate_dto(
    lifecycle: &LifecycleStore,
    project_id: &str,
    issue_id: &str,
    plan_id: &str,
) -> Result<WorkItemPlanCandidateDto, ProductStoreError> {
    let plan = lifecycle.get_issue_work_item_plan(project_id, issue_id, plan_id)?;
    let work_items = lifecycle.list_work_items(project_id, issue_id)?;
    let plan_work_item_ids: HashSet<String> = plan.work_item_ids.iter().cloned().collect();
    let plan_work_items: Vec<&LifecycleWorkItemRecord> = work_items
        .iter()
        .filter(|wi| plan_work_item_ids.contains(&wi.id))
        .collect();

    let found_ids: HashSet<&String> = plan_work_items.iter().map(|wi| &wi.id).collect();
    if let Some(missing_id) = plan_work_item_ids
        .iter()
        .find(|id| !found_ids.contains(*id))
    {
        return Err(ProductStoreError::NotFound {
            kind: "work_item",
            id: missing_id.clone(),
        });
    }

    let verification_plans: Vec<VerificationPlanDto> = plan
        .verification_plan_ids
        .iter()
        .map(|vp_id| {
            lifecycle
                .get_verification_plan(project_id, issue_id, vp_id)
                .map(|vp| VerificationPlanDto {
                    plan_ref: vp.id,
                    scope: vp.scope.as_str().to_string(),
                    commands: vp
                        .commands
                        .iter()
                        .map(|cmd| VerificationCommandDto {
                            label: cmd.label.clone(),
                            command: cmd.command.clone(),
                            cwd: cmd.cwd.clone(),
                            purpose: cmd.purpose.clone(),
                            required: cmd.required,
                            timeout_seconds: cmd.timeout_seconds,
                            safety: cmd.safety.as_str().to_string(),
                        })
                        .collect(),
                    manual_checks: vp
                        .manual_checks
                        .iter()
                        .map(|check| VerificationManualCheckDto {
                            label: check.label.clone(),
                            instructions: check.instructions.clone(),
                            required: check.required,
                        })
                        .collect(),
                    required_gates: vp.required_gates,
                    risk_notes: vp.risk_notes,
                    confidence: vp.confidence.as_str().to_string(),
                    fallback_policy: vp.fallback_policy.as_str().to_string(),
                })
        })
        .collect::<Result<Vec<_>, _>>()?;

    let repository_profile = plan
        .repository_profile_ref
        .as_ref()
        .map(|rid| lifecycle.get_repository_profile(project_id, issue_id, rid))
        .transpose()?
        .map(|rp| RepositoryProfileDto {
            profile_id: rp.id,
            repository_id: rp.repository_id,
            languages: rp.languages,
            frameworks: rp.frameworks,
            package_managers: rp.package_managers,
            test_frameworks: rp.test_frameworks,
            build_systems: rp.build_systems,
            detected_layers: rp.detected_layers,
            split_recommendation: rp.split_recommendation,
            confidence: rp.confidence.as_str().to_string(),
        });

    let work_item_dtos: Vec<WorkItemCandidateDto> = plan_work_items
        .iter()
        .map(|wi| WorkItemCandidateDto {
            id: wi.id.clone(),
            kind: wi.kind.as_str().to_string(),
            title: wi.title.clone(),
            depends_on: wi.depends_on.clone(),
            exclusive_write_scopes: wi.exclusive_write_scopes.clone(),
            verification_plan_ref: wi.verification_plan_ref.clone(),
            meta: WorkItemCandidateMetaDto {
                reverted: false,
                revert_feedback: None,
            },
        })
        .collect();

    Ok(WorkItemPlanCandidateDto {
        plan: WorkItemPlanDto {
            id: plan.id,
            status: plan.status.as_str().to_string(),
            options: WorkItemSplitOptionsDto {
                include_integration_tests: plan.options.include_integration_tests,
                include_e2e_tests: plan.options.include_e2e_tests,
                force_frontend_backend_split: plan.options.force_frontend_backend_split,
                require_execution_plan_confirm: plan.options.require_execution_plan_confirm,
            },
            dependency_graph: plan
                .dependency_graph
                .iter()
                .map(|e| WorkItemDependencyEdgeDto {
                    from_work_item_id: e.from_work_item_id.clone(),
                    to_work_item_id: e.to_work_item_id.clone(),
                })
                .collect(),
        },
        work_items: work_item_dtos,
        verification_plans,
        repository_profile,
        validator_findings: work_item_split_findings_to_dto(&plan.validator_findings),
    })
}

fn build_node_detail_summary(detail: &NodeDetail) -> NodeDetailSummary {
    let prompt = detail.prompt.as_deref().unwrap_or("");
    let stream = if !detail.streaming_content.is_empty() {
        detail.streaming_content.as_str()
    } else {
        detail
            .messages
            .last()
            .and_then(|message| message.get("content"))
            .and_then(|content| content.as_str())
            .unwrap_or("")
    };
    let has_large_event_output = detail.execution_events.iter().any(|event| {
        event
            .get("output")
            .and_then(|output| output.as_str())
            .is_some_and(|output| output.chars().count() > SUMMARY_PREVIEW_CHARS)
    });

    NodeDetailSummary {
        node_id: detail.node_id.clone(),
        node_type: serialized_string(&detail.node_type),
        status: serialized_string(&detail.status),
        agent_role: detail.agent_role.as_ref().map(serialized_string),
        provider_name: detail
            .provider
            .as_ref()
            .map(|provider| provider.name.clone()),
        prompt_size: prompt.len(),
        prompt_preview: detail.prompt.as_ref().map(|prompt| preview(prompt)),
        stream_size: stream.len(),
        stream_preview: (!stream.is_empty()).then(|| preview(stream)),
        execution_event_count: detail.execution_events.len(),
        has_large_outputs: prompt.chars().count() > SUMMARY_PREVIEW_CHARS
            || stream.chars().count() > SUMMARY_PREVIEW_CHARS
            || has_large_event_output,
        artifact_ref: detail
            .artifact_ref
            .as_ref()
            .map(|artifact| format!("{}/v{}", artifact.artifact_id, artifact.version)),
        started_at: detail.started_at.clone(),
        ended_at: detail.ended_at.clone(),
    }
}

fn build_session_state_node_detail(mut detail: NodeDetail) -> NodeDetail {
    detail.prompt = None;
    detail.messages.clear();
    if detail.streaming_content.chars().count() > SUMMARY_PREVIEW_CHARS {
        detail.streaming_content = preview(&detail.streaming_content);
    }
    detail.execution_events.clear();
    detail.permission_events.clear();
    detail
}

fn build_artifact_version_summary(version: &ArtifactVersion) -> ArtifactVersionSummary {
    let (markdown_size, markdown_preview) = match &version.payload {
        ArtifactPayload::Markdown { markdown, .. } => (markdown.len(), preview(markdown)),
        ArtifactPayload::WorkItemPlanCandidate { candidate } => {
            // For the candidate variant, `markdown_size`/`markdown_preview` reuse the
            // summary schema fields for compatibility: the size is the JSON length of
            // the candidate and the preview is the title of the first work item (or the
            // plan id as a fallback). In a future iteration we may rename these fields.
            let size = serde_json::to_string(candidate).map_or(0, |s| s.len());
            let preview_text = candidate
                .work_items
                .first()
                .map(|item| item.title.as_str())
                .unwrap_or(candidate.plan.id.as_str())
                .to_string();
            (size, preview(&preview_text))
        }
        ArtifactPayload::WorkItemPlanOutlineCandidate { outline_candidate } => {
            let size = serde_json::to_string(outline_candidate).map_or(0, |s| s.len());
            let preview_text = outline_candidate.outline.strategy_summary.clone();
            (size, preview(&preview_text))
        }
        ArtifactPayload::WorkItemPlanContextBlocker { context_blocker } => {
            let size = serde_json::to_string(context_blocker).map_or(0, |s| s.len());
            let preview_text = context_blocker.exploration_summary.clone();
            (size, preview(&preview_text))
        }
        ArtifactPayload::WorkItemDraftCandidate { draft_candidate } => {
            let size = serde_json::to_string(draft_candidate).map_or(0, |s| s.len());
            let preview_text = format!(
                "{}: {}",
                draft_candidate.draft_record.outline_id,
                draft_candidate.draft_record.candidate.title
            );
            (size, preview(&preview_text))
        }
        ArtifactPayload::WorkItemBatchState { batch_state } => {
            let size = serde_json::to_string(batch_state).map_or(0, |s| s.len());
            let preview_text = format!(
                "{}: {:?} ({} drafts)",
                batch_state.batch_id,
                batch_state.batch_status,
                batch_state.draft_records.len()
            );
            (size, preview(&preview_text))
        }
        ArtifactPayload::WorkItemPlanCompileReport { compile_report } => {
            let size = serde_json::to_string(compile_report).map_or(0, |s| s.len());
            let preview_text = format!(
                "{}: {:?} ({} work items)",
                compile_report.compile_id,
                compile_report.status,
                compile_report.work_item_ids.len()
            );
            (size, preview(&preview_text))
        }
    };
    ArtifactVersionSummary {
        version: version.version,
        generated_by: version.generated_by.clone(),
        reviewed_by: version.reviewed_by.clone(),
        review_verdict: version.review_verdict.clone(),
        confirmed_by: version.confirmed_by.clone(),
        is_current: version.is_current,
        created_at: version.created_at.clone(),
        source_node_id: version.source_node_id.clone(),
        markdown_size,
        markdown_preview,
    }
}

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

struct WorkItemPlanCompileProjectionContext<'a> {
    outline_order: &'a [String],
    outline_to_work_item_id: &'a BTreeMap<String, String>,
    outline_to_verification_plan_id: &'a BTreeMap<String, String>,
    repository_id: &'a str,
    now: &'a str,
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
    work_item_plan_author_retry_count: u32,
    work_item_plan_revision_retry_count: u32,
    work_item_batch_retry_counts: HashMap<String, u32>,
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

struct RevisionResumeFallbackContext {
    provider: Arc<dyn StreamingProviderAdapter>,
    attempted: bool,
}

struct ProviderSessionDriveInput {
    session: Result<ProviderSession, crate::cross_cutting::provider_adapter::ProviderAdapterError>,
    command_rx: mpsc::Receiver<ProviderCommand>,
    node_id: Option<String>,
    agent: Option<ProviderName>,
    role: ProviderConversationRole,
    artifact_retry: Option<ArtifactRetryContext>,
    revision_resume_fallback: Option<RevisionResumeFallbackContext>,
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

struct StructuredOutputDisplayFilter {
    pending: String,
    inside_structured_output: bool,
}

impl StructuredOutputDisplayFilter {
    fn new() -> Self {
        Self {
            pending: String::new(),
            inside_structured_output: false,
        }
    }

    fn push(&mut self, chunk: &str) -> String {
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

    fn finish(&mut self) -> String {
        if self.inside_structured_output {
            self.pending.clear();
            String::new()
        } else {
            std::mem::take(&mut self.pending)
        }
    }
}

fn longest_suffix_prefix_len(value: &str, pattern: &str) -> usize {
    let max_len = value.len().min(pattern.len().saturating_sub(1));
    (1..=max_len)
        .rev()
        .find(|len| value.ends_with(&pattern[..*len]))
        .unwrap_or(0)
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
            work_item_plan_author_retry_count: 0,
            work_item_plan_revision_retry_count: 0,
            work_item_batch_retry_counts: HashMap::new(),
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
                .map(|version| version.payload.clone());
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
        let pending_author_choice =
            recover_pending_author_choice(&session, active_node_id.as_deref(), &timeline_nodes);
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
            pending_author_choice,
            active_run_id: None,
            stream_buffers: HashMap::new(),
            work_item_plan_author_retry_count: 0,
            work_item_plan_revision_retry_count: 0,
            work_item_batch_retry_counts: HashMap::new(),
        }
    }

    pub fn session(&self) -> &WorkspaceSession {
        &self.session
    }

    pub fn pending_author_choice_request_message(&self) -> Option<WsOutMessage> {
        let pending = self.pending_author_choice.as_ref()?;
        Some(WsOutMessage::ChoiceRequest {
            id: pending.id.clone(),
            prompt: pending.prompt.clone(),
            options: pending
                .options
                .iter()
                .map(|option| ChoiceOption {
                    id: option.id.clone(),
                    label: option.label.clone(),
                    description: option.description.clone(),
                })
                .collect(),
            allow_multiple: false,
            allow_free_text: true,
            source: ChoiceRequestSource::TextFallback.as_str().to_string(),
        })
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

    pub fn active_timeline_node_id(&self) -> Option<String> {
        self.active_node_id.clone()
    }

    pub(crate) fn active_node_type(&self) -> Option<TimelineNodeType> {
        let active_node_id = self.active_node_id.as_deref()?;
        self.timeline_nodes
            .iter()
            .find(|node| node.node_id == active_node_id)
            .map(|node| node.node_type.clone())
    }

    fn current_work_item_plan_outline_candidate(
        &self,
    ) -> Result<&WorkItemPlanOutlineCandidateDto, String> {
        match self.session.artifact.as_ref() {
            Some(ArtifactPayload::WorkItemPlanOutlineCandidate { outline_candidate }) => {
                Ok(outline_candidate)
            }
            _ => Err("current WorkItemPlan Outline artifact is unavailable".to_string()),
        }
    }

    fn latest_work_item_plan_outline_candidate(
        &self,
    ) -> Result<WorkItemPlanOutlineCandidateDto, String> {
        if let Some(ArtifactPayload::WorkItemPlanOutlineCandidate { outline_candidate }) =
            self.session.artifact.as_ref()
        {
            return Ok(outline_candidate.as_ref().clone());
        }

        self.artifact_versions
            .iter()
            .rev()
            .find_map(|version| match &version.payload {
                ArtifactPayload::WorkItemPlanOutlineCandidate { outline_candidate } => {
                    Some(outline_candidate.as_ref().clone())
                }
                _ => None,
            })
            .ok_or_else(|| "latest WorkItemPlan Outline artifact is unavailable".to_string())
    }

    fn current_work_item_draft_candidate_payload(
        &self,
    ) -> Result<WorkItemDraftCandidatePayload, String> {
        match self.session.artifact.as_ref() {
            Some(ArtifactPayload::WorkItemDraftCandidate { draft_candidate }) => {
                Ok(draft_candidate.as_ref().clone())
            }
            _ => Err("current WorkItemDraft artifact is unavailable".to_string()),
        }
    }

    fn current_work_item_plan_outline_ids(&self) -> Vec<String> {
        self.latest_work_item_plan_outline_candidate()
            .map(|candidate| {
                candidate
                    .outline
                    .work_item_outlines
                    .iter()
                    .map(|outline| outline.outline_id.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    fn work_item_plan_store(&self) -> Result<WorkItemPlanStore, String> {
        let lifecycle = self
            .lifecycle_store
            .as_ref()
            .ok_or_else(|| "lifecycle_store unavailable".to_string())?;
        Ok(WorkItemPlanStore::new(lifecycle.app_paths()))
    }

    fn save_confirmed_work_item_plan_outline_index(&self) -> Result<String, String> {
        self.current_work_item_plan_outline_candidate()?;
        let store = self.work_item_plan_store()?;
        let project_id = self.session.project_id.clone();
        let issue_id = self.session.issue_id.clone();
        let plan_id = self.session.entity_id.clone();
        let current = store
            .load_active_index(&project_id, &issue_id, &plan_id)
            .map_err(|error| format!("load work item plan active index failed: {error}"))?;
        let generation_round_id = current
            .as_ref()
            .map(next_generation_round_id)
            .unwrap_or_else(|| "round_001".to_string());
        let now = chrono::Utc::now().to_rfc3339();
        let index = WorkItemPlanDraftActiveIndex {
            project_id,
            issue_id,
            plan_id,
            current_generation_round_id: generation_round_id.clone(),
            outline_state: "confirmed".to_string(),
            active_outline_id: None,
            outline_to_current_draft_id: BTreeMap::new(),
            draft_statuses: BTreeMap::new(),
            batches: Vec::new(),
            updated_at: now,
        };
        store
            .save_active_index(&index)
            .map_err(|error| format!("save work item plan active index failed: {error}"))?;
        Ok(generation_round_id)
    }

    fn mark_work_item_plan_outline_revising(&self) -> Result<(), String> {
        let store = self.work_item_plan_store()?;
        let project_id = self.session.project_id.clone();
        let issue_id = self.session.issue_id.clone();
        let plan_id = self.session.entity_id.clone();
        let mut index = store
            .load_active_index(&project_id, &issue_id, &plan_id)
            .map_err(|error| format!("load work item plan active index failed: {error}"))?
            .unwrap_or_else(|| WorkItemPlanDraftActiveIndex {
                project_id,
                issue_id,
                plan_id,
                current_generation_round_id: "round_001".to_string(),
                outline_state: "revising".to_string(),
                active_outline_id: None,
                outline_to_current_draft_id: BTreeMap::new(),
                draft_statuses: BTreeMap::new(),
                batches: Vec::new(),
                updated_at: chrono::Utc::now().to_rfc3339(),
            });
        let now = chrono::Utc::now().to_rfc3339();
        self.supersede_current_generation_drafts_for_outline_revision(&store, &mut index, &now)?;
        index.outline_state = "revising".to_string();
        index.active_outline_id = None;
        index.updated_at = now;
        store
            .save_active_index(&index)
            .map_err(|error| format!("save work item plan active index failed: {error}"))
    }

    fn supersede_current_generation_drafts_for_outline_revision(
        &self,
        store: &WorkItemPlanStore,
        index: &mut WorkItemPlanDraftActiveIndex,
        now: &str,
    ) -> Result<(), String> {
        let draft_ids: Vec<String> = index
            .draft_statuses
            .iter()
            .filter_map(|(draft_id, status)| {
                if status == &WorkItemDraftStatus::Superseded {
                    None
                } else {
                    Some(draft_id.clone())
                }
            })
            .collect();

        for draft_id in draft_ids {
            let mut record = store
                .get_draft_record(
                    &index.project_id,
                    &index.issue_id,
                    &index.plan_id,
                    &index.current_generation_round_id,
                    &draft_id,
                )
                .map_err(|error| format!("load draft for outline revision failed: {error}"))?;
            mark_draft_record_superseded(
                &mut record,
                None,
                WorkItemDraftSupersedeReason::OutlineRevised,
                now,
            );
            store.put_draft_record(&record).map_err(|error| {
                format!("save superseded outline revision draft failed: {error}")
            })?;
            index
                .draft_statuses
                .insert(draft_id, WorkItemDraftStatus::Superseded);
        }

        index.outline_to_current_draft_id.clear();
        Ok(())
    }

    fn set_active_work_item_plan_outline(&self, outline_id: &str) -> Result<(), String> {
        let store = self.work_item_plan_store()?;
        let project_id = self.session.project_id.clone();
        let issue_id = self.session.issue_id.clone();
        let plan_id = self.session.entity_id.clone();
        let mut index = store
            .load_active_index(&project_id, &issue_id, &plan_id)
            .map_err(|error| format!("load work item plan active index failed: {error}"))?
            .ok_or_else(|| "work item plan active index missing".to_string())?;
        index.active_outline_id = Some(outline_id.to_string());
        index.updated_at = chrono::Utc::now().to_rfc3339();
        store
            .save_active_index(&index)
            .map_err(|error| format!("save work item plan active index failed: {error}"))
    }

    async fn update_work_item_plan_outline_generation_metadata(
        &mut self,
        generation_round_id: Option<String>,
        selected_mode: Option<WorkItemGenerationModeDto>,
    ) -> Result<(), String> {
        let Some(ArtifactPayload::WorkItemPlanOutlineCandidate { outline_candidate }) =
            self.session.artifact.clone()
        else {
            return Err("current WorkItemPlan Outline artifact is unavailable".to_string());
        };
        let mut outline_candidate = *outline_candidate;
        if generation_round_id.is_some() {
            outline_candidate.current_generation_round_id = generation_round_id;
        }
        outline_candidate.selected_generation_mode = selected_mode;
        self.update_artifact(ArtifactPayload::WorkItemPlanOutlineCandidate {
            outline_candidate: Box::new(outline_candidate),
        })
        .await;
        Ok(())
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

    pub async fn begin_work_item_plan_author_run(&mut self) -> String {
        self.create_timeline_node(TimelineNodeDraft {
            node_type: TimelineNodeType::AuthorRun,
            agent: Some(self.session.author_provider.clone()),
            stage: WorkspaceStage::Running,
            round: None,
            title: "Work Item Plan 生成".to_string(),
            summary: None,
            status: TimelineNodeStatus::Active,
        })
        .await
    }

    pub async fn begin_work_item_plan_outline_run(&mut self) -> String {
        self.create_timeline_node(TimelineNodeDraft {
            node_type: TimelineNodeType::WorkItemPlanOutlineRun,
            agent: Some(self.session.author_provider.clone()),
            stage: WorkspaceStage::Running,
            round: None,
            title: "WorkItemPlan Outline 生成".to_string(),
            summary: None,
            status: TimelineNodeStatus::Active,
        })
        .await
    }

    pub async fn begin_work_item_plan_outline_review_run(&mut self) -> String {
        self.transition_stage(WorkspaceStage::CrossReview).await;
        let round = self.next_review_round();
        let reviewer = self
            .session
            .reviewer_provider
            .clone()
            .unwrap_or(ProviderName::Codex);
        self.create_timeline_node(TimelineNodeDraft {
            node_type: TimelineNodeType::WorkItemPlanOutlineReview,
            agent: Some(reviewer),
            stage: WorkspaceStage::CrossReview,
            round: Some(round),
            title: format!("WorkItemPlan Outline Review Round {round}"),
            summary: None,
            status: TimelineNodeStatus::Active,
        })
        .await
    }

    async fn begin_work_item_draft_review_run(&mut self, outline_id: &str) -> String {
        self.transition_stage(WorkspaceStage::CrossReview).await;
        let round = self.next_review_round();
        let reviewer = self
            .session
            .reviewer_provider
            .clone()
            .unwrap_or(ProviderName::Codex);
        self.create_timeline_node(TimelineNodeDraft {
            node_type: TimelineNodeType::WorkItemDraftReview,
            agent: Some(reviewer),
            stage: WorkspaceStage::CrossReview,
            round: Some(round),
            title: format!("Work Item Draft Review Round {round}"),
            summary: Some(format!("审核 outline `{outline_id}` 的 Work Item Draft")),
            status: TimelineNodeStatus::Active,
        })
        .await
    }

    async fn begin_work_item_batch_review_run(&mut self) -> String {
        self.transition_stage(WorkspaceStage::CrossReview).await;
        let round = self.next_review_round();
        let reviewer = self
            .session
            .reviewer_provider
            .clone()
            .unwrap_or(ProviderName::Codex);
        self.create_timeline_node(TimelineNodeDraft {
            node_type: TimelineNodeType::WorkItemBatchReview,
            agent: Some(reviewer),
            stage: WorkspaceStage::CrossReview,
            round: Some(round),
            title: format!("Work Item Batch Review Round {round}"),
            summary: Some("审核整组 Work Item Draft".to_string()),
            status: TimelineNodeStatus::Active,
        })
        .await
    }

    pub async fn select_work_item_generation_mode(
        &mut self,
        mode: WorkItemGenerationModeDto,
    ) -> Result<(), String> {
        if self.session.stage != WorkspaceStage::AuthorConfirm
            || self.active_node_type() != Some(TimelineNodeType::WorkItemGenerationMode)
        {
            return Err(
                "select_work_item_generation_mode requires active work_item_generation_mode node"
                    .to_string(),
            );
        }

        self.update_work_item_plan_outline_generation_metadata(None, Some(mode.clone()))
            .await?;
        self.pending_revision_context = None;
        match mode {
            WorkItemGenerationModeDto::Serial => {
                self.complete_active_node(Some("已选择逐项生成 Work Item".to_string()))
                    .await;
                self.start_serial_work_item_draft_run().await;
            }
            WorkItemGenerationModeDto::Batch => {
                self.create_current_work_item_batch_record()?;
                self.complete_active_node(Some("已选择自动生成全部 Work Item".to_string()))
                    .await;
                self.transition_stage(WorkspaceStage::Running).await;
                let _ = self
                    .create_timeline_node(TimelineNodeDraft {
                        node_type: TimelineNodeType::WorkItemBatchRun,
                        agent: Some(self.session.author_provider.clone()),
                        stage: WorkspaceStage::Running,
                        round: None,
                        title: "Work Item Batch 生成".to_string(),
                        summary: Some("WP5 占位节点，Batch 实际生成由后续 WP 接入".to_string()),
                        status: TimelineNodeStatus::Active,
                    })
                    .await;
            }
        }
        Ok(())
    }

    fn create_current_work_item_batch_record(&self) -> Result<WorkItemBatchRecord, String> {
        let store = self.work_item_plan_store()?;
        let project_id = self.session.project_id.clone();
        let issue_id = self.session.issue_id.clone();
        let plan_id = self.session.entity_id.clone();
        let mut index = store
            .load_active_index(&project_id, &issue_id, &plan_id)
            .map_err(|error| format!("load work item plan active index failed: {error}"))?
            .ok_or_else(|| "work item plan active index is missing".to_string())?;
        let now = chrono::Utc::now().to_rfc3339();
        let batch = WorkItemBatchRecord {
            batch_id: next_batch_id(&index, &now),
            generation_round_id: index.current_generation_round_id.clone(),
            mode: WorkItemGenerationMode::Batch,
            item_draft_ids: Vec::new(),
            status: WorkItemBatchStatus::Generating,
            validation_failed_ids: Vec::new(),
            created_at: now.clone(),
        };
        let outline_candidate = self.current_work_item_plan_outline_candidate()?;
        let first_outline_id =
            work_item_plan_outline_topological_order(&outline_candidate.outline)?
                .into_iter()
                .next()
                .ok_or_else(|| "WorkItemPlan Outline has no work item outlines".to_string())?;
        index.active_outline_id = Some(first_outline_id);
        index.batches.push(batch.clone());
        index.updated_at = now;
        store
            .save_active_index(&index)
            .map_err(|error| format!("save work item plan active index failed: {error}"))?;
        Ok(batch)
    }

    async fn start_serial_work_item_draft_run(&mut self) {
        let first_outline_id =
            match self
                .current_work_item_plan_outline_candidate()
                .and_then(|outline_candidate| {
                    work_item_plan_outline_topological_order(&outline_candidate.outline).and_then(
                        |order| {
                            order.into_iter().next().ok_or_else(|| {
                                "WorkItemPlan Outline has no work item outlines".to_string()
                            })
                        },
                    )
                }) {
                Ok(outline_id) => outline_id,
                Err(message) => {
                    self.enter_human_confirm(Some(format!(
                        "无法开始逐项生成 Work Item：{message}"
                    )))
                    .await;
                    return;
                }
            };

        if let Err(message) = self.set_active_work_item_plan_outline(&first_outline_id) {
            let _ = self.event_tx.send(EngineEvent::Error { message }).await;
            self.enter_human_confirm(Some("保存当前 Work Item 游标失败".to_string()))
                .await;
            return;
        }

        self.create_serial_work_item_draft_run_node(&first_outline_id)
            .await;
    }

    async fn start_serial_work_item_draft_run_for(
        &mut self,
        outline_id: &str,
    ) -> Result<(), String> {
        self.set_active_work_item_plan_outline(outline_id)?;
        self.create_serial_work_item_draft_run_node(outline_id)
            .await;
        Ok(())
    }

    async fn create_serial_work_item_draft_run_node(&mut self, outline_id: &str) {
        self.transition_stage(WorkspaceStage::Running).await;
        let _ = self
            .create_timeline_node(TimelineNodeDraft {
                node_type: TimelineNodeType::WorkItemDraftRun,
                agent: Some(self.session.author_provider.clone()),
                stage: WorkspaceStage::Running,
                round: None,
                title: "Work Item Draft 生成".to_string(),
                summary: Some(format!(
                    "准备生成 outline `{outline_id}` 的 Work Item Draft"
                )),
                status: TimelineNodeStatus::Active,
            })
            .await;
    }

    pub fn build_current_work_item_draft_streaming_input(
        &mut self,
        feedback: Option<&str>,
    ) -> Result<StreamingProviderInput, String> {
        let effective_feedback = match feedback {
            Some(value) => {
                self.pending_revision_context = None;
                Some(value.to_string())
            }
            None => self.pending_revision_context.take(),
        };
        self.build_current_work_item_draft_streaming_input_with_feedback(
            effective_feedback.as_deref(),
        )
    }

    fn build_current_work_item_draft_streaming_input_with_feedback(
        &self,
        feedback: Option<&str>,
    ) -> Result<StreamingProviderInput, String> {
        let store = self.work_item_plan_store()?;
        let index = store
            .load_active_index(
                &self.session.project_id,
                &self.session.issue_id,
                &self.session.entity_id,
            )
            .map_err(|error| format!("load work item plan active index failed: {error}"))?
            .ok_or_else(|| "work item plan active index missing".to_string())?;
        let active_outline_id = index
            .active_outline_id
            .clone()
            .ok_or_else(|| "active work item outline missing".to_string())?;
        let outline_candidate = self.latest_work_item_plan_outline_candidate()?;
        let accepted_drafts = self.accepted_work_item_plan_draft_records(&store, &index)?;
        let invocation = build_work_item_draft_invocation(
            &outline_candidate.outline,
            &active_outline_id,
            WorkItemGenerationMode::Serial,
            &accepted_drafts,
            feedback,
        )
        .map_err(|error| error.message)?;
        let working_dir = self
            .session
            .repository_path
            .clone()
            .or_else(|| std::env::current_dir().ok())
            .ok_or_else(|| "working directory unavailable".to_string())?;
        Ok(self.build_work_item_plan_streaming_input(
            provider_type_for_name(&self.session.author_provider),
            invocation.prompt,
            working_dir.to_string_lossy().to_string(),
        ))
    }

    pub fn build_current_work_item_batch_draft_streaming_input(
        &self,
    ) -> Result<StreamingProviderInput, String> {
        if self.active_node_type() != Some(TimelineNodeType::WorkItemBatchRun) {
            return Err("batch draft input requires active work_item_batch_run node".to_string());
        }
        let store = self.work_item_plan_store()?;
        let index = store
            .load_active_index(
                &self.session.project_id,
                &self.session.issue_id,
                &self.session.entity_id,
            )
            .map_err(|error| format!("load work item plan active index failed: {error}"))?
            .ok_or_else(|| "work item plan active index missing".to_string())?;
        let active_outline_id = index
            .active_outline_id
            .clone()
            .ok_or_else(|| "active batch work item outline missing".to_string())?;
        let batch = current_work_item_batch(&index)?;
        let outline_candidate = self.latest_work_item_plan_outline_candidate()?;
        let batch_drafts =
            self.batch_work_item_plan_draft_records(&store, &index, &batch.batch_id)?;
        let invocation = build_work_item_draft_invocation(
            &outline_candidate.outline,
            &active_outline_id,
            WorkItemGenerationMode::Batch,
            &batch_drafts,
            None,
        )
        .map_err(|error| error.message)?;
        let working_dir = self
            .session
            .repository_path
            .clone()
            .or_else(|| std::env::current_dir().ok())
            .ok_or_else(|| "working directory unavailable".to_string())?;
        Ok(self.build_work_item_plan_streaming_input(
            provider_type_for_name(&self.session.author_provider),
            invocation.prompt,
            working_dir.to_string_lossy().to_string(),
        ))
    }

    fn accepted_work_item_plan_draft_records(
        &self,
        store: &WorkItemPlanStore,
        index: &WorkItemPlanDraftActiveIndex,
    ) -> Result<Vec<WorkItemDraftRecord>, String> {
        let mut records = Vec::new();
        for draft_id in index.outline_to_current_draft_id.values() {
            if index.draft_statuses.get(draft_id) != Some(&WorkItemDraftStatus::Accepted) {
                continue;
            }
            let record = store
                .get_draft_record(
                    &index.project_id,
                    &index.issue_id,
                    &index.plan_id,
                    &index.current_generation_round_id,
                    draft_id,
                )
                .map_err(|error| format!("load accepted draft record failed: {error}"))?;
            records.push(record);
        }
        Ok(records)
    }

    fn batch_work_item_plan_draft_records(
        &self,
        store: &WorkItemPlanStore,
        index: &WorkItemPlanDraftActiveIndex,
        batch_id: &str,
    ) -> Result<Vec<WorkItemDraftRecord>, String> {
        let batch = index
            .batches
            .iter()
            .find(|batch| batch.batch_id == batch_id)
            .ok_or_else(|| format!("batch `{batch_id}` not found"))?;
        let mut records = Vec::new();
        for draft_id in &batch.item_draft_ids {
            let record = store
                .get_draft_record(
                    &index.project_id,
                    &index.issue_id,
                    &index.plan_id,
                    &index.current_generation_round_id,
                    draft_id,
                )
                .map_err(|error| format!("load batch draft record failed: {error}"))?;
            records.push(record);
        }
        Ok(records)
    }

    fn current_work_item_batch_state_payload(
        &self,
        store: &WorkItemPlanStore,
        index: &WorkItemPlanDraftActiveIndex,
        batch_id: &str,
    ) -> Result<WorkItemBatchStatePayload, String> {
        let batch = index
            .batches
            .iter()
            .find(|batch| batch.batch_id == batch_id)
            .ok_or_else(|| format!("batch `{batch_id}` not found"))?;
        let outline_candidate = self.latest_work_item_plan_outline_candidate()?;
        let queue = work_item_plan_outline_topological_order(&outline_candidate.outline)?;
        let mut draft_records = Vec::new();
        for draft_id in batch
            .item_draft_ids
            .iter()
            .chain(batch.validation_failed_ids.iter())
        {
            draft_records.push(
                store
                    .get_draft_record(
                        &index.project_id,
                        &index.issue_id,
                        &index.plan_id,
                        &index.current_generation_round_id,
                        draft_id,
                    )
                    .map_err(|error| format!("load batch state draft failed: {error}"))?,
            );
        }
        let failure_summary = draft_records
            .iter()
            .filter(|record| record.status == WorkItemDraftStatus::ValidationFailed)
            .map(|record| WorkItemBatchFailureSummaryDto {
                draft_id: record.draft_id.clone(),
                outline_id: record.outline_id.clone(),
                status: work_item_draft_status_label(&record.status).to_string(),
            })
            .collect();

        Ok(WorkItemBatchStatePayload {
            batch_id: batch.batch_id.clone(),
            generation_round_id: batch.generation_round_id.clone(),
            queue,
            draft_records,
            batch_status: batch.status.clone(),
            failure_summary,
        })
    }

    async fn enter_work_item_plan_compile(&mut self) {
        self.transition_stage(WorkspaceStage::Running).await;
        self.create_timeline_node(TimelineNodeDraft {
            node_type: TimelineNodeType::WorkItemPlanCompile,
            agent: None,
            stage: WorkspaceStage::Running,
            round: None,
            title: "WorkItemPlan Final Compile".to_string(),
            summary: Some("编译已确认 Draft 并写入真实 Work Item".to_string()),
            status: TimelineNodeStatus::Active,
        })
        .await;

        match self.run_work_item_plan_compile().await {
            Ok(report) => {
                let work_item_count = report.work_item_ids.len();
                self.update_artifact(ArtifactPayload::WorkItemPlanCompileReport {
                    compile_report: Box::new(report),
                })
                .await;
                self.complete_active_node(Some(format!(
                    "Final Compile 完成，已创建 {work_item_count} 个 Work Item"
                )))
                .await;
                self.enter_human_confirm(Some(format!(
                    "Final Compile 完成，已创建 {work_item_count} 个 Work Item，等待最终确认"
                )))
                .await;
            }
            Err(message) => {
                self.complete_active_node(Some(format!("Final Compile 失败：{message}")))
                    .await;
                if self.mark_latest_compile_transaction_recovery_required(&message) {
                    self.enter_work_item_plan_compile_recovery(Some(format!(
                        "Final Compile 需要恢复：{message}"
                    )))
                    .await;
                } else if self.is_current_work_item_plan_batch_mode() {
                    self.enter_work_item_batch_confirm(Some(format!(
                        "Final Compile strict validator 失败：{message}"
                    )))
                    .await;
                } else {
                    self.enter_human_confirm(Some(format!("Final Compile 失败：{message}")))
                        .await;
                }
            }
        }
    }

    async fn enter_work_item_plan_compile_recovery(&mut self, summary: Option<String>) {
        self.transition_stage(WorkspaceStage::HumanConfirm).await;
        let _ = self
            .create_timeline_node(TimelineNodeDraft {
                node_type: TimelineNodeType::WorkItemPlanCompileRecovery,
                agent: None,
                stage: WorkspaceStage::HumanConfirm,
                round: None,
                title: "WorkItemPlan Compile Recovery".to_string(),
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

    fn mark_latest_compile_transaction_recovery_required(&self, message: &str) -> bool {
        let Ok(store) = self.work_item_plan_store() else {
            return false;
        };
        let Ok(Some(mut tx)) = store
            .list_compile_transactions(
                &self.session.project_id,
                &self.session.issue_id,
                &self.session.entity_id,
            )
            .map(|transactions| {
                transactions
                    .into_iter()
                    .filter(|tx| {
                        matches!(
                            tx.status,
                            WorkItemPlanCompileStatus::Preparing
                                | WorkItemPlanCompileStatus::Validating
                                | WorkItemPlanCompileStatus::Committing
                                | WorkItemPlanCompileStatus::RecoveryRequired
                        )
                    })
                    .max_by(|left, right| left.created_at.cmp(&right.created_at))
            })
        else {
            return false;
        };
        tx.status = WorkItemPlanCompileStatus::RecoveryRequired;
        tx.failure_reason = Some(message.to_string());
        tx.updated_at = chrono::Utc::now().to_rfc3339();
        store.put_compile_transaction(&tx).is_ok()
    }

    async fn run_work_item_plan_compile(
        &mut self,
    ) -> Result<WorkItemPlanCompileReportPayload, String> {
        let lifecycle = self
            .lifecycle_store
            .clone()
            .ok_or_else(|| "lifecycle_store unavailable".to_string())?;
        let store = self.work_item_plan_store()?;
        let project_id = self.session.project_id.clone();
        let issue_id = self.session.issue_id.clone();
        let plan_id = self.session.entity_id.clone();
        let previous_plan = lifecycle
            .get_issue_work_item_plan(&project_id, &issue_id, &plan_id)
            .map_err(|error| format!("load issue work item plan failed: {error}"))?;
        let index = store
            .load_active_index(&project_id, &issue_id, &plan_id)
            .map_err(|error| format!("load work item plan active index failed: {error}"))?
            .ok_or_else(|| "work item plan active index missing".to_string())?;
        let outline_candidate = self.latest_work_item_plan_outline_candidate()?;
        let outline_order = work_item_plan_outline_topological_order(&outline_candidate.outline)?;
        let draft_records =
            self.accepted_active_draft_records_for_compile(&store, &index, &outline_order)?;
        let active_draft_ids: Vec<String> = draft_records
            .iter()
            .map(|record| record.draft_id.clone())
            .collect();
        let compile_id = next_compile_id();
        let now = chrono::Utc::now().to_rfc3339();
        let outline_to_work_item_id: BTreeMap<String, String> = outline_order
            .iter()
            .enumerate()
            .map(|(index, outline_id)| {
                (outline_id.clone(), compile_work_item_id(&compile_id, index))
            })
            .collect();
        let outline_to_verification_plan_id: BTreeMap<String, String> = outline_order
            .iter()
            .enumerate()
            .map(|(index, outline_id)| {
                (
                    outline_id.clone(),
                    compile_verification_plan_id(&compile_id, index),
                )
            })
            .collect();
        let mut tx = WorkItemPlanCompileTransaction {
            compile_id: compile_id.clone(),
            project_id: project_id.clone(),
            issue_id: issue_id.clone(),
            plan_id: plan_id.clone(),
            generation_round_id: index.current_generation_round_id.clone(),
            outline_version_ref: outline_candidate.outline.id.clone(),
            active_draft_ids,
            status: WorkItemPlanCompileStatus::Preparing,
            plan_commit_state: WorkItemPlanCommitState::NotStarted,
            step_cursor: "preparing".to_string(),
            outline_to_work_item_id: BTreeMap::new(),
            outline_to_verification_plan_id: BTreeMap::new(),
            created_work_item_ids: Vec::new(),
            created_verification_plan_ids: Vec::new(),
            child_session_ids: Vec::new(),
            validator_findings: Vec::new(),
            abort_requested_at: None,
            failure_reason: None,
            previous_plan_snapshot: previous_plan.clone(),
            created_at: now.clone(),
            updated_at: now.clone(),
            committed_at: None,
        };
        store
            .put_compile_transaction(&tx)
            .map_err(|error| format!("save compile transaction failed: {error}"))?;

        let repository_id = self.work_item_plan_repository_id(&lifecycle, &previous_plan)?;
        let (mut compiled_plan, work_items, verification_plans) = self
            .project_work_item_plan_drafts_for_compile(
                &previous_plan,
                &draft_records,
                WorkItemPlanCompileProjectionContext {
                    outline_order: &outline_order,
                    outline_to_work_item_id: &outline_to_work_item_id,
                    outline_to_verification_plan_id: &outline_to_verification_plan_id,
                    repository_id: &repository_id,
                    now: &now,
                },
            )?;
        tx.status = WorkItemPlanCompileStatus::Validating;
        tx.step_cursor = "validating".to_string();
        tx.outline_to_work_item_id = outline_to_work_item_id;
        tx.outline_to_verification_plan_id = outline_to_verification_plan_id;
        tx.updated_at = chrono::Utc::now().to_rfc3339();
        store
            .put_compile_transaction(&tx)
            .map_err(|error| format!("save validating compile transaction failed: {error}"))?;

        let report = WorkItemSplitValidator::validate(
            &compiled_plan,
            &work_items,
            None,
            &verification_plans,
        );
        tx.validator_findings = report.findings.clone();
        if report.has_errors() {
            tx.status = WorkItemPlanCompileStatus::Failed;
            tx.failure_reason = Some(work_item_plan_findings_summary(
                "Final Compile strict validator failed",
                &report.findings,
            ));
            tx.updated_at = chrono::Utc::now().to_rfc3339();
            store
                .put_compile_transaction(&tx)
                .map_err(|error| format!("save failed compile transaction failed: {error}"))?;
            return Err(work_item_plan_findings_summary(
                "Final Compile strict validator failed",
                &report.findings,
            ));
        }
        compiled_plan.validator_findings = report.findings.clone();

        tx.status = WorkItemPlanCompileStatus::Committing;
        tx.step_cursor = "committing".to_string();
        tx.updated_at = chrono::Utc::now().to_rfc3339();
        store
            .put_compile_transaction(&tx)
            .map_err(|error| format!("save committing compile transaction failed: {error}"))?;

        for (work_item, verification_plan) in work_items.iter().zip(verification_plans.iter()) {
            if !tx.created_work_item_ids.contains(&work_item.id) {
                lifecycle
                    .create_work_item(CreateWorkItemInput {
                        id: Some(work_item.id.clone()),
                        project_id: work_item.project_id.clone(),
                        issue_id: work_item.issue_id.clone(),
                        repository_id: work_item.repository_id.clone(),
                        story_spec_ids: work_item.story_spec_ids.clone(),
                        design_spec_ids: work_item.design_spec_ids.clone(),
                        title: work_item.title.clone(),
                        work_item_set_id: work_item.work_item_set_id.clone(),
                        kind: work_item.kind.clone(),
                        sequence_hint: work_item.sequence_hint,
                        depends_on: work_item.depends_on.clone(),
                        exclusive_write_scopes: work_item.exclusive_write_scopes.clone(),
                        forbidden_write_scopes: work_item.forbidden_write_scopes.clone(),
                        context_budget: work_item.context_budget.clone(),
                        required_handoff_from: work_item.required_handoff_from.clone(),
                        verification_plan_ref: work_item.verification_plan_ref.clone(),
                        require_execution_plan_confirm: work_item.require_execution_plan_confirm,
                        plan_status: WorkItemPlanStatus::Confirmed,
                    })
                    .map_err(|error| format!("create work item failed: {error}"))?;
                tx.created_work_item_ids.push(work_item.id.clone());
            }
            if !tx
                .created_verification_plan_ids
                .contains(&verification_plan.id)
            {
                lifecycle
                    .create_verification_plan(CreateVerificationPlanInput {
                        id: Some(verification_plan.id.clone()),
                        project_id: verification_plan.project_id.clone(),
                        issue_id: verification_plan.issue_id.clone(),
                        work_item_id: verification_plan.work_item_id.clone(),
                        repository_profile_ref: verification_plan.repository_profile_ref.clone(),
                        provider_run_ref: verification_plan.provider_run_ref.clone(),
                        scope: verification_plan.scope.clone(),
                        commands: verification_plan.commands.clone(),
                        manual_checks: verification_plan.manual_checks.clone(),
                        required_gates: verification_plan.required_gates.clone(),
                        risk_notes: verification_plan.risk_notes.clone(),
                        confidence: verification_plan.confidence.clone(),
                        fallback_policy: verification_plan.fallback_policy.clone(),
                    })
                    .map_err(|error| format!("create verification plan failed: {error}"))?;
                tx.created_verification_plan_ids
                    .push(verification_plan.id.clone());
            }
            let child_session = lifecycle
                .create_workspace_session(CreateWorkspaceSessionInput {
                    project_id: project_id.clone(),
                    issue_id: issue_id.clone(),
                    entity_id: work_item.id.clone(),
                    workspace_type: WorkspaceType::WorkItem,
                    author_provider: self.session.author_provider.clone(),
                    reviewer_provider: self
                        .session
                        .reviewer_provider
                        .clone()
                        .unwrap_or(ProviderName::Codex),
                    review_rounds: self.session.review_rounds,
                    superpowers_enabled: self.session.superpowers_enabled,
                    openspec_enabled: self.session.openspec_enabled,
                })
                .map_err(|error| format!("create child work item workspace failed: {error}"))?;
            tx.child_session_ids.push(child_session.id);
            tx.updated_at = chrono::Utc::now().to_rfc3339();
            store
                .put_compile_transaction(&tx)
                .map_err(|error| format!("save compile step cursor failed: {error}"))?;
        }

        tx.plan_commit_state = WorkItemPlanCommitState::Committed;
        tx.committed_at = Some(chrono::Utc::now().to_rfc3339());
        tx.step_cursor = "plan_commit_marker_written".to_string();
        tx.updated_at = chrono::Utc::now().to_rfc3339();
        store
            .put_compile_transaction(&tx)
            .map_err(|error| format!("save compile committed marker failed: {error}"))?;

        lifecycle
            .commit_issue_work_item_plan(
                &project_id,
                &issue_id,
                &plan_id,
                IssueWorkItemPlanUpdate {
                    work_item_ids: compiled_plan.work_item_ids.clone(),
                    verification_plan_ids: compiled_plan.verification_plan_ids.clone(),
                    repository_profile_ref: None,
                    dependency_graph: compiled_plan.dependency_graph.clone(),
                    created_from_provider_run: compiled_plan.created_from_provider_run.clone(),
                    validator_findings: compiled_plan.validator_findings.clone(),
                },
            )
            .map_err(|error| format!("commit issue work item plan failed: {error}"))?;

        tx.status = WorkItemPlanCompileStatus::Committed;
        tx.step_cursor = "committed".to_string();
        tx.updated_at = chrono::Utc::now().to_rfc3339();
        store
            .put_compile_transaction(&tx)
            .map_err(|error| format!("save committed compile transaction failed: {error}"))?;

        Ok(WorkItemPlanCompileReportPayload {
            compile_id,
            generation_round_id: index.current_generation_round_id,
            status: WorkItemPlanCompileStatus::Committed,
            plan_commit_state: WorkItemPlanCommitState::Committed,
            work_item_ids: compiled_plan.work_item_ids,
            verification_plan_ids: compiled_plan.verification_plan_ids,
            child_session_ids: tx.child_session_ids,
            validator_findings: work_item_split_findings_to_dto(&tx.validator_findings),
        })
    }

    pub async fn handle_work_item_plan_compile_recovery_action(
        &mut self,
        action: WorkItemPlanCompileRecoveryActionDto,
        reason: Option<String>,
    ) -> Result<WorkItemPlanCompileRecoveryOutcome, String> {
        if self.session.workspace_type != WorkspaceType::WorkItemPlan
            || self.active_node_type() != Some(TimelineNodeType::WorkItemPlanCompileRecovery)
        {
            return Err(
                "work_item_plan_compile_recovery_action requires active work_item_plan_compile_recovery node"
                    .to_string(),
            );
        }

        let lifecycle = self
            .lifecycle_store
            .clone()
            .ok_or_else(|| "lifecycle_store unavailable".to_string())?;
        let store = self.work_item_plan_store()?;
        let mut tx = self.latest_work_item_plan_recovery_transaction(&store)?;

        match action {
            WorkItemPlanCompileRecoveryActionDto::AbortAndRollback => {
                if tx.plan_commit_state == WorkItemPlanCommitState::Committed {
                    return Err(
                        "abort_and_rollback is not allowed when plan_commit_state=committed"
                            .to_string(),
                    );
                }

                for verification_plan_id in tx.created_verification_plan_ids.clone() {
                    lifecycle
                        .delete_verification_plan(
                            &tx.project_id,
                            &tx.issue_id,
                            &verification_plan_id,
                        )
                        .map_err(|error| {
                            format!("delete verification plan during rollback failed: {error}")
                        })?;
                }
                for work_item_id in tx.created_work_item_ids.clone() {
                    lifecycle
                        .delete_work_item(&tx.project_id, &tx.issue_id, &work_item_id)
                        .map_err(|error| {
                            format!("delete work item during rollback failed: {error}")
                        })?;
                }
                lifecycle
                    .restore_issue_work_item_plan_snapshot(
                        &tx.project_id,
                        &tx.issue_id,
                        &tx.plan_id,
                        &tx.previous_plan_snapshot,
                    )
                    .map_err(|error| format!("restore previous WorkItemPlan failed: {error}"))?;

                tx.status = WorkItemPlanCompileStatus::Failed;
                tx.created_work_item_ids.clear();
                tx.created_verification_plan_ids.clear();
                tx.child_session_ids.clear();
                tx.failure_reason = Some(
                    reason
                        .unwrap_or_else(|| "compile recovery aborted and rolled back".to_string()),
                );
                tx.step_cursor = "rolled_back".to_string();
                tx.updated_at = chrono::Utc::now().to_rfc3339();
                store.put_compile_transaction(&tx).map_err(|error| {
                    format!("save rolled back compile transaction failed: {error}")
                })?;

                self.complete_active_node(Some(
                    "已放弃本次 Final Compile 并恢复旧 Plan".to_string(),
                ))
                .await;
                self.enter_human_confirm(Some(
                    "Final Compile 已回滚，等待人工确认下一步".to_string(),
                ))
                .await;
                Ok(WorkItemPlanCompileRecoveryOutcome::HumanConfirm)
            }
            WorkItemPlanCompileRecoveryActionDto::Continue => {
                if tx.plan_commit_state == WorkItemPlanCommitState::Committed {
                    self.commit_recovered_work_item_plan_after_marker(&lifecycle, &tx)?;
                    tx.status = WorkItemPlanCompileStatus::Committed;
                    tx.failure_reason = reason.or(tx.failure_reason);
                    tx.step_cursor = "committed".to_string();
                    tx.updated_at = chrono::Utc::now().to_rfc3339();
                    store.put_compile_transaction(&tx).map_err(|error| {
                        format!("save continued compile transaction failed: {error}")
                    })?;
                    self.complete_active_node(Some(
                        "Final Compile 已从 committed marker 恢复".to_string(),
                    ))
                    .await;
                    self.enter_human_confirm(Some(
                        "Final Compile 已提交，等待最终确认".to_string(),
                    ))
                    .await;
                    return Ok(WorkItemPlanCompileRecoveryOutcome::HumanConfirm);
                }

                self.complete_active_node(Some("继续 Final Compile".to_string()))
                    .await;
                self.enter_work_item_plan_compile().await;
                Ok(WorkItemPlanCompileRecoveryOutcome::Continue)
            }
            WorkItemPlanCompileRecoveryActionDto::HumanTriage => {
                tx.failure_reason = reason.or(tx.failure_reason);
                tx.updated_at = chrono::Utc::now().to_rfc3339();
                store.put_compile_transaction(&tx).map_err(|error| {
                    format!("save human triage compile transaction failed: {error}")
                })?;
                self.complete_active_node(Some("Final Compile 转人工处理".to_string()))
                    .await;
                self.enter_human_confirm(Some("Final Compile 需要人工整理".to_string()))
                    .await;
                Ok(WorkItemPlanCompileRecoveryOutcome::HumanConfirm)
            }
        }
    }

    fn latest_work_item_plan_recovery_transaction(
        &self,
        store: &WorkItemPlanStore,
    ) -> Result<WorkItemPlanCompileTransaction, String> {
        store
            .list_compile_transactions(
                &self.session.project_id,
                &self.session.issue_id,
                &self.session.entity_id,
            )
            .map_err(|error| format!("list compile transactions failed: {error}"))?
            .into_iter()
            .filter(|tx| tx.status == WorkItemPlanCompileStatus::RecoveryRequired)
            .max_by(|left, right| left.created_at.cmp(&right.created_at))
            .ok_or_else(|| "work item plan compile recovery transaction is missing".to_string())
    }

    fn commit_recovered_work_item_plan_after_marker(
        &self,
        lifecycle: &LifecycleStore,
        tx: &WorkItemPlanCompileTransaction,
    ) -> Result<(), String> {
        let work_items = lifecycle
            .list_work_items(&tx.project_id, &tx.issue_id)
            .map_err(|error| format!("list work items during compile recovery failed: {error}"))?;
        let created_work_item_ids: HashSet<&str> = tx
            .created_work_item_ids
            .iter()
            .map(String::as_str)
            .collect();
        let work_items_by_id: HashMap<&str, &LifecycleWorkItemRecord> = work_items
            .iter()
            .filter(|item| created_work_item_ids.contains(item.id.as_str()))
            .map(|item| (item.id.as_str(), item))
            .collect();
        for work_item_id in &tx.created_work_item_ids {
            if !work_items_by_id.contains_key(work_item_id.as_str()) {
                return Err(format!(
                    "created work item `{work_item_id}` missing during compile recovery"
                ));
            }
        }

        let dependency_graph = tx
            .created_work_item_ids
            .iter()
            .filter_map(|work_item_id| work_items_by_id.get(work_item_id.as_str()).copied())
            .flat_map(|work_item| {
                work_item
                    .depends_on
                    .iter()
                    .cloned()
                    .map(|from_work_item_id| IssueWorkItemDependencyEdge {
                        from_work_item_id,
                        to_work_item_id: work_item.id.clone(),
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();

        lifecycle
            .commit_issue_work_item_plan(
                &tx.project_id,
                &tx.issue_id,
                &tx.plan_id,
                IssueWorkItemPlanUpdate {
                    work_item_ids: tx.created_work_item_ids.clone(),
                    verification_plan_ids: tx.created_verification_plan_ids.clone(),
                    repository_profile_ref: None,
                    dependency_graph,
                    created_from_provider_run: tx
                        .previous_plan_snapshot
                        .created_from_provider_run
                        .clone(),
                    validator_findings: tx.validator_findings.clone(),
                },
            )
            .map_err(|error| format!("commit recovered WorkItemPlan failed: {error}"))?;
        Ok(())
    }

    fn accepted_active_draft_records_for_compile(
        &self,
        store: &WorkItemPlanStore,
        index: &WorkItemPlanDraftActiveIndex,
        outline_order: &[String],
    ) -> Result<Vec<WorkItemDraftRecord>, String> {
        let mut records = Vec::with_capacity(outline_order.len());
        for outline_id in outline_order {
            let draft_id = index
                .outline_to_current_draft_id
                .get(outline_id)
                .ok_or_else(|| format!("outline `{outline_id}` has no active draft"))?;
            if index.draft_statuses.get(draft_id) != Some(&WorkItemDraftStatus::Accepted) {
                return Err(format!("draft `{draft_id}` is not accepted"));
            }
            let record = store
                .get_draft_record(
                    &index.project_id,
                    &index.issue_id,
                    &index.plan_id,
                    &index.current_generation_round_id,
                    draft_id,
                )
                .map_err(|error| format!("load active draft `{draft_id}` failed: {error}"))?;
            if !record.active || record.status != WorkItemDraftStatus::Accepted {
                return Err(format!(
                    "draft `{draft_id}` is not an accepted active draft"
                ));
            }
            if record.superseded_by_draft_id.is_some()
                || record.supersede_reason.is_some()
                || record.superseded_at.is_some()
            {
                return Err(format!("draft `{draft_id}` has been superseded"));
            }
            records.push(record);
        }
        Ok(records)
    }

    fn is_current_work_item_plan_batch_mode(&self) -> bool {
        let Ok(store) = self.work_item_plan_store() else {
            return false;
        };
        store
            .load_active_index(
                &self.session.project_id,
                &self.session.issue_id,
                &self.session.entity_id,
            )
            .ok()
            .flatten()
            .map(|index| {
                index.batches.iter().any(|batch| {
                    batch.generation_round_id == index.current_generation_round_id
                        && batch.mode == WorkItemGenerationMode::Batch
                })
            })
            .unwrap_or(false)
    }

    fn work_item_plan_repository_id(
        &self,
        lifecycle: &LifecycleStore,
        plan: &IssueWorkItemPlan,
    ) -> Result<String, String> {
        let story_specs = lifecycle
            .list_story_specs(&plan.project_id, &plan.issue_id)
            .map_err(|error| format!("list story specs failed: {error}"))?;
        for story_id in &plan.source_story_spec_ids {
            if let Some(story) = story_specs.iter().find(|story| &story.id == story_id) {
                return Ok(story.repository_id.clone());
            }
        }
        Err("cannot resolve repository_id for WorkItemPlan compile".to_string())
    }

    fn project_work_item_plan_drafts_for_compile(
        &self,
        previous_plan: &IssueWorkItemPlan,
        draft_records: &[WorkItemDraftRecord],
        context: WorkItemPlanCompileProjectionContext<'_>,
    ) -> Result<
        (
            IssueWorkItemPlan,
            Vec<LifecycleWorkItemRecord>,
            Vec<VerificationPlan>,
        ),
        String,
    > {
        let outline_order = context.outline_order;
        let outline_to_work_item_id = context.outline_to_work_item_id;
        let outline_to_verification_plan_id = context.outline_to_verification_plan_id;
        let repository_id = context.repository_id;
        let now = context.now;
        let draft_by_outline: HashMap<&str, &WorkItemDraftRecord> = draft_records
            .iter()
            .map(|record| (record.outline_id.as_str(), record))
            .collect();
        let mut work_items = Vec::with_capacity(outline_order.len());
        let mut verification_plans = Vec::with_capacity(outline_order.len());
        for (index, outline_id) in outline_order.iter().enumerate() {
            let record = draft_by_outline
                .get(outline_id.as_str())
                .ok_or_else(|| format!("accepted draft for outline `{outline_id}` missing"))?;
            let candidate = &record.candidate;
            let work_item_id = outline_to_work_item_id
                .get(outline_id)
                .cloned()
                .ok_or_else(|| format!("work item id for outline `{outline_id}` missing"))?;
            let verification_plan_id = outline_to_verification_plan_id
                .get(outline_id)
                .cloned()
                .ok_or_else(|| {
                    format!("verification plan id for outline `{outline_id}` missing")
                })?;
            let depends_on = candidate
                .depends_on_outline_ids
                .iter()
                .map(|dependency_outline_id| {
                    outline_to_work_item_id
                        .get(dependency_outline_id)
                        .cloned()
                        .ok_or_else(|| {
                            format!(
                                "dependency outline `{dependency_outline_id}` for `{outline_id}` missing"
                            )
                        })
                })
                .collect::<Result<Vec<_>, _>>()?;
            let required_handoff_from = candidate
                .required_handoff_from_outline_ids
                .iter()
                .map(|dependency_outline_id| {
                    outline_to_work_item_id
                        .get(dependency_outline_id)
                        .cloned()
                        .ok_or_else(|| {
                            format!(
                                "handoff outline `{dependency_outline_id}` for `{outline_id}` missing"
                            )
                        })
                })
                .collect::<Result<Vec<_>, _>>()?;
            work_items.push(LifecycleWorkItemRecord {
                id: work_item_id.clone(),
                project_id: previous_plan.project_id.clone(),
                issue_id: previous_plan.issue_id.clone(),
                repository_id: repository_id.to_string(),
                story_spec_ids: previous_plan.source_story_spec_ids.clone(),
                design_spec_ids: previous_plan.source_design_spec_ids.clone(),
                title: candidate.title.clone(),
                plan_status: WorkItemPlanStatus::Confirmed,
                execution_status: crate::product::models::WorkItemStatus::Pending,
                worktree_path: None,
                work_item_set_id: Some(previous_plan.id.clone()),
                kind: candidate.kind.clone(),
                sequence_hint: Some((index + 1) as u32),
                depends_on,
                exclusive_write_scopes: candidate.exclusive_write_scopes.clone(),
                forbidden_write_scopes: candidate.forbidden_write_scopes.clone(),
                context_budget: crate::product::models::WorkItemContextBudget::default(),
                required_handoff_from,
                verification_plan_ref: Some(verification_plan_id.clone()),
                require_execution_plan_confirm: previous_plan
                    .options
                    .require_execution_plan_confirm,
                execution_plan_status:
                    crate::product::models::WorkItemExecutionPlanStatus::NotStarted,
                handoff_summary_ref: None,
                completion_commit: None,
                completion_diff_summary_ref: None,
                created_at: now.to_string(),
                updated_at: now.to_string(),
            });
            verification_plans.push(parse_compile_verification_plan(
                &candidate.verification_plan,
                verification_plan_id,
                previous_plan.project_id.clone(),
                previous_plan.issue_id.clone(),
                work_item_id,
                now.to_string(),
            ));
        }
        let work_item_ids: Vec<String> = outline_order
            .iter()
            .filter_map(|outline_id| outline_to_work_item_id.get(outline_id).cloned())
            .collect();
        let verification_plan_ids: Vec<String> = outline_order
            .iter()
            .filter_map(|outline_id| outline_to_verification_plan_id.get(outline_id).cloned())
            .collect();
        let dependency_graph = self
            .latest_work_item_plan_outline_candidate()?
            .outline
            .dependency_graph
            .iter()
            .map(|edge| {
                let from_work_item_id = outline_to_work_item_id
                    .get(&edge.from_outline_id)
                    .cloned()
                    .ok_or_else(|| {
                        format!("dependency from outline `{}` missing", edge.from_outline_id)
                    })?;
                let to_work_item_id = outline_to_work_item_id
                    .get(&edge.to_outline_id)
                    .cloned()
                    .ok_or_else(|| {
                        format!("dependency to outline `{}` missing", edge.to_outline_id)
                    })?;
                Ok(IssueWorkItemDependencyEdge {
                    from_work_item_id,
                    to_work_item_id,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let mut compiled_plan = previous_plan.clone();
        compiled_plan.status = crate::product::models::IssueWorkItemPlanStatus::Confirmed;
        compiled_plan.work_item_ids = work_item_ids;
        compiled_plan.verification_plan_ids = verification_plan_ids;
        compiled_plan.repository_profile_ref = None;
        compiled_plan.dependency_graph = dependency_graph;
        compiled_plan.validator_findings = Vec::new();
        compiled_plan.updated_at = now.to_string();
        Ok((compiled_plan, work_items, verification_plans))
    }

    pub async fn complete_work_item_draft_author(
        &mut self,
        candidate: WorkItemDraftCandidate,
    ) -> Result<(), String> {
        if self.active_node_type() != Some(TimelineNodeType::WorkItemDraftRun) {
            return Err("work item draft author completion requires active draft run".to_string());
        }

        let generated_from_node_id = self
            .active_node_id
            .clone()
            .ok_or_else(|| "active draft run node missing".to_string())?;
        let store = self.work_item_plan_store()?;
        let mut index = store
            .load_active_index(
                &self.session.project_id,
                &self.session.issue_id,
                &self.session.entity_id,
            )
            .map_err(|error| format!("load work item plan active index failed: {error}"))?
            .ok_or_else(|| "work item plan active index missing".to_string())?;
        let active_outline_id = index
            .active_outline_id
            .clone()
            .ok_or_else(|| "active work item outline missing".to_string())?;
        if candidate.outline_id != active_outline_id {
            return Err(format!(
                "draft outline_id {} does not match active outline {}",
                candidate.outline_id, active_outline_id
            ));
        }
        let previous_draft_record = match index
            .outline_to_current_draft_id
            .get(&active_outline_id)
            .cloned()
        {
            Some(previous_draft_id) => Some(
                store
                    .get_draft_record(
                        &index.project_id,
                        &index.issue_id,
                        &index.plan_id,
                        &index.current_generation_round_id,
                        &previous_draft_id,
                    )
                    .map_err(|error| format!("load previous draft record failed: {error}"))?,
            ),
            None => None,
        };

        let outline_candidate = self.latest_work_item_plan_outline_candidate()?;
        let current_outline = outline_candidate
            .outline
            .work_item_outlines
            .iter()
            .find(|item| item.outline_id == active_outline_id)
            .cloned()
            .ok_or_else(|| format!("active outline {active_outline_id} not found"))?;
        let accepted_drafts = self.accepted_work_item_plan_draft_records(&store, &index)?;
        let accepted_candidates: Vec<WorkItemDraftCandidate> = accepted_drafts
            .iter()
            .map(|record| record.candidate.clone())
            .collect();
        let report = WorkItemDraftLocalValidator::validate(
            &candidate,
            &accepted_candidates,
            &current_outline,
        );
        let status = if report.has_errors() {
            WorkItemDraftStatus::ValidationFailed
        } else {
            WorkItemDraftStatus::Draft
        };
        let draft_id = next_draft_id(&index);
        let now = chrono::Utc::now().to_rfc3339();
        let record = WorkItemDraftRecord {
            project_id: self.session.project_id.clone(),
            issue_id: self.session.issue_id.clone(),
            plan_id: self.session.entity_id.clone(),
            draft_id: draft_id.clone(),
            outline_id: active_outline_id.clone(),
            generation_round_id: index.current_generation_round_id.clone(),
            batch_id: None,
            attempt_index: previous_draft_record
                .as_ref()
                .map(|record| record.attempt_index + 1)
                .unwrap_or(1),
            outline_version_ref: outline_candidate.outline.id.clone(),
            generation_mode: WorkItemGenerationMode::Serial,
            candidate,
            status: status.clone(),
            active: true,
            superseded_by_draft_id: None,
            supersede_reason: None,
            copied_from_draft_id: None,
            review_node_id: None,
            review_verdict_ref: None,
            generated_from_node_id,
            accepted_at: None,
            superseded_at: None,
            created_at: now.clone(),
            updated_at: now.clone(),
        };

        store
            .put_draft_record(&record)
            .map_err(|error| format!("save work item draft record failed: {error}"))?;
        if let Some(mut previous_record) = previous_draft_record
            && previous_record.draft_id != draft_id
        {
            mark_draft_record_superseded(
                &mut previous_record,
                Some(draft_id.clone()),
                WorkItemDraftSupersedeReason::DirectRewrite,
                &now,
            );
            store
                .put_draft_record(&previous_record)
                .map_err(|error| format!("save superseded draft record failed: {error}"))?;
        }
        mark_draft_active(&mut index, &active_outline_id, &draft_id, status.clone());
        index.active_outline_id = Some(active_outline_id);
        index.updated_at = now;
        store
            .save_active_index(&index)
            .map_err(|error| format!("save work item plan active index failed: {error}"))?;

        let validator_findings = work_item_split_findings_to_dto(&report.findings);
        let can_accept = !report.has_errors();
        self.update_artifact(ArtifactPayload::WorkItemDraftCandidate {
            draft_candidate: Box::new(WorkItemDraftCandidatePayload {
                draft_record: record,
                validator_findings,
                can_accept,
            }),
        })
        .await;
        self.complete_active_node(Some(if can_accept {
            "Work Item Draft 生成完成，等待确认".to_string()
        } else {
            "Work Item Draft 局部校验失败，等待重写或暂停".to_string()
        }))
        .await;
        self.enter_work_item_draft_confirm(Some("请确认当前 Work Item Draft".to_string()))
            .await;
        Ok(())
    }

    pub async fn complete_work_item_batch_draft_author(
        &mut self,
        candidate: WorkItemDraftCandidate,
    ) -> Result<(), String> {
        if self.active_node_type() != Some(TimelineNodeType::WorkItemBatchRun) {
            return Err("batch draft author completion requires active batch run".to_string());
        }

        let generated_from_node_id = self
            .active_node_id
            .clone()
            .ok_or_else(|| "active batch run node missing".to_string())?;
        let store = self.work_item_plan_store()?;
        let mut index = store
            .load_active_index(
                &self.session.project_id,
                &self.session.issue_id,
                &self.session.entity_id,
            )
            .map_err(|error| format!("load work item plan active index failed: {error}"))?
            .ok_or_else(|| "work item plan active index missing".to_string())?;
        let active_outline_id = index
            .active_outline_id
            .clone()
            .ok_or_else(|| "active batch work item outline missing".to_string())?;
        if candidate.outline_id != active_outline_id {
            return Err(format!(
                "draft outline_id {} does not match active outline {}",
                candidate.outline_id, active_outline_id
            ));
        }
        let outline_candidate = self.latest_work_item_plan_outline_candidate()?;
        let current_outline = outline_candidate
            .outline
            .work_item_outlines
            .iter()
            .find(|item| item.outline_id == active_outline_id)
            .cloned()
            .ok_or_else(|| format!("active outline {active_outline_id} not found"))?;
        let batch_id = current_work_item_batch(&index)?.batch_id.clone();
        let batch_drafts = self.batch_work_item_plan_draft_records(&store, &index, &batch_id)?;
        let batch_candidates: Vec<WorkItemDraftCandidate> = batch_drafts
            .iter()
            .map(|record| record.candidate.clone())
            .collect();
        let report =
            WorkItemDraftLocalValidator::validate(&candidate, &batch_candidates, &current_outline);
        if report.has_errors() {
            let retry_count = self
                .work_item_batch_retry_counts
                .entry(active_outline_id.clone())
                .or_default();
            if *retry_count == 0 {
                *retry_count += 1;
                return Ok(());
            }
        } else {
            self.work_item_batch_retry_counts.remove(&active_outline_id);
        }
        let status = if report.has_errors() {
            WorkItemDraftStatus::ValidationFailed
        } else {
            WorkItemDraftStatus::Draft
        };
        let draft_id = next_draft_id(&index);
        let now = chrono::Utc::now().to_rfc3339();
        let record = WorkItemDraftRecord {
            project_id: self.session.project_id.clone(),
            issue_id: self.session.issue_id.clone(),
            plan_id: self.session.entity_id.clone(),
            draft_id: draft_id.clone(),
            outline_id: active_outline_id.clone(),
            generation_round_id: index.current_generation_round_id.clone(),
            batch_id: Some(batch_id.clone()),
            attempt_index: 1,
            outline_version_ref: outline_candidate.outline.id.clone(),
            generation_mode: WorkItemGenerationMode::Batch,
            candidate,
            status: status.clone(),
            active: true,
            superseded_by_draft_id: None,
            supersede_reason: None,
            copied_from_draft_id: None,
            review_node_id: None,
            review_verdict_ref: None,
            generated_from_node_id,
            accepted_at: None,
            superseded_at: None,
            created_at: now.clone(),
            updated_at: now.clone(),
        };

        store
            .put_draft_record(&record)
            .map_err(|error| format!("save batch work item draft record failed: {error}"))?;
        mark_draft_active(&mut index, &active_outline_id, &draft_id, status.clone());
        let outline_order = work_item_plan_outline_topological_order(&outline_candidate.outline)?;
        let current_pos = outline_order
            .iter()
            .position(|id| id == &active_outline_id)
            .ok_or_else(|| format!("outline {active_outline_id} not found in order"))?;
        let next_outline_id = outline_order.get(current_pos + 1).cloned();
        {
            let batch = index
                .batches
                .iter_mut()
                .find(|batch| batch.batch_id == batch_id)
                .ok_or_else(|| format!("batch `{batch_id}` not found"))?;
            match status {
                WorkItemDraftStatus::ValidationFailed => {
                    batch.validation_failed_ids.push(draft_id.clone());
                }
                _ => {
                    batch.item_draft_ids.push(draft_id.clone());
                }
            }
            if next_outline_id.is_none() {
                batch.status = WorkItemBatchStatus::Completed;
            }
        }
        index.active_outline_id = next_outline_id;
        index.updated_at = now;
        store
            .save_active_index(&index)
            .map_err(|error| format!("save work item plan active index failed: {error}"))?;

        let validator_findings = work_item_split_findings_to_dto(&report.findings);
        self.update_artifact(ArtifactPayload::WorkItemDraftCandidate {
            draft_candidate: Box::new(WorkItemDraftCandidatePayload {
                draft_record: record,
                validator_findings,
                can_accept: !report.has_errors(),
            }),
        })
        .await;
        if index.active_outline_id.is_none() {
            let batch_state =
                self.current_work_item_batch_state_payload(&store, &index, &batch_id)?;
            self.update_artifact(ArtifactPayload::WorkItemBatchState {
                batch_state: Box::new(batch_state),
            })
            .await;
            self.complete_active_node(Some("Work Item Batch 生成完成，等待整组确认".to_string()))
                .await;
            self.enter_work_item_batch_confirm(Some("请确认整组 Work Item Draft".to_string()))
                .await;
        }
        Ok(())
    }

    pub async fn handle_work_item_batch_decision(
        &mut self,
        decision: WorkItemBatchDecisionDto,
        feedback: Option<String>,
        first_affected_outline_id: Option<String>,
    ) -> Result<WorkItemBatchDecisionOutcome, String> {
        if self.session.stage != WorkspaceStage::AuthorConfirm
            || self.active_node_type() != Some(TimelineNodeType::WorkItemBatchConfirm)
        {
            return Err(
                "work_item_batch_decision requires active work_item_batch_confirm node".to_string(),
            );
        }

        match decision {
            WorkItemBatchDecisionDto::AcceptAll => self.accept_current_work_item_batch().await,
            WorkItemBatchDecisionDto::Pause => {
                self.complete_active_node(Some("Work Item Batch 已暂停".to_string()))
                    .await;
                self.enter_human_confirm(Some("Work Item Batch 已暂停，等待人工处理".to_string()))
                    .await;
                Ok(WorkItemBatchDecisionOutcome::HumanConfirm)
            }
            WorkItemBatchDecisionDto::RewriteBatch => self.rewrite_current_work_item_batch().await,
            WorkItemBatchDecisionDto::DowngradeToSerial => {
                self.downgrade_current_work_item_batch_to_serial(
                    first_affected_outline_id,
                    feedback,
                )
                .await
            }
        }
    }

    async fn accept_current_work_item_batch(
        &mut self,
    ) -> Result<WorkItemBatchDecisionOutcome, String> {
        let store = self.work_item_plan_store()?;
        let mut index = store
            .load_active_index(
                &self.session.project_id,
                &self.session.issue_id,
                &self.session.entity_id,
            )
            .map_err(|error| format!("load work item plan active index failed: {error}"))?
            .ok_or_else(|| "work item plan active index missing".to_string())?;
        let batch_id = current_work_item_batch(&index)?.batch_id.clone();
        let batch_pos = index
            .batches
            .iter()
            .position(|batch| batch.batch_id == batch_id)
            .ok_or_else(|| format!("batch `{batch_id}` not found"))?;
        if !index.batches[batch_pos].validation_failed_ids.is_empty() {
            return Err("accept_all requires no validation_failed drafts".to_string());
        }

        let now = chrono::Utc::now().to_rfc3339();
        let draft_ids = index.batches[batch_pos].item_draft_ids.clone();
        for draft_id in &draft_ids {
            let mut record = store
                .get_draft_record(
                    &index.project_id,
                    &index.issue_id,
                    &index.plan_id,
                    &index.current_generation_round_id,
                    draft_id,
                )
                .map_err(|error| format!("load batch draft record failed: {error}"))?;
            if record.status == WorkItemDraftStatus::ValidationFailed {
                return Err(format!(
                    "draft `{}` has validation errors and cannot be accepted",
                    record.draft_id
                ));
            }
            record.status = WorkItemDraftStatus::Accepted;
            record.accepted_at = Some(now.clone());
            record.updated_at = now.clone();
            store
                .put_draft_record(&record)
                .map_err(|error| format!("save accepted batch draft failed: {error}"))?;
            index
                .draft_statuses
                .insert(draft_id.clone(), WorkItemDraftStatus::Accepted);
        }
        let review_enabled =
            self.session.review_rounds > 0 && self.session.reviewer_provider.is_some();
        index.batches[batch_pos].status = if review_enabled {
            WorkItemBatchStatus::ReviewPending
        } else {
            WorkItemBatchStatus::ReviewDone
        };
        index.updated_at = now;
        store
            .save_active_index(&index)
            .map_err(|error| format!("save work item plan active index failed: {error}"))?;

        self.complete_active_node(Some("Work Item Batch 已接受".to_string()))
            .await;
        if review_enabled {
            self.begin_work_item_batch_review_run().await;
            Ok(WorkItemBatchDecisionOutcome::StartReview)
        } else {
            self.enter_work_item_plan_compile().await;
            Ok(WorkItemBatchDecisionOutcome::HumanConfirm)
        }
    }

    pub async fn handle_work_item_draft_decision(
        &mut self,
        outline_id: String,
        decision: WorkItemDraftDecisionDto,
        feedback: Option<String>,
    ) -> Result<WorkItemDraftDecisionOutcome, String> {
        if self.session.stage != WorkspaceStage::AuthorConfirm
            || self.active_node_type() != Some(TimelineNodeType::WorkItemDraftConfirm)
        {
            return Err(
                "work_item_draft_decision requires active work_item_draft_confirm node".to_string(),
            );
        }

        match decision {
            WorkItemDraftDecisionDto::Accept => {
                self.accept_current_work_item_draft(outline_id).await
            }
            WorkItemDraftDecisionDto::Rewrite => {
                self.pending_revision_context = feedback;
                self.complete_active_node(Some("用户要求重写当前 Work Item Draft".to_string()))
                    .await;
                self.start_serial_work_item_draft_run_for(&outline_id)
                    .await?;
                Ok(WorkItemDraftDecisionOutcome::StartDraftRun)
            }
            WorkItemDraftDecisionDto::Pause => {
                self.complete_active_node(Some("用户暂停逐项 Work Item 生成".to_string()))
                    .await;
                self.enter_human_confirm(Some("逐项 Work Item 生成已暂停".to_string()))
                    .await;
                Ok(WorkItemDraftDecisionOutcome::HumanConfirm)
            }
        }
    }

    async fn accept_current_work_item_draft(
        &mut self,
        outline_id: String,
    ) -> Result<WorkItemDraftDecisionOutcome, String> {
        let Some(ArtifactPayload::WorkItemDraftCandidate { draft_candidate }) =
            self.session.artifact.clone()
        else {
            return Err("current artifact is not a WorkItemDraftCandidate".to_string());
        };
        if draft_candidate.draft_record.outline_id != outline_id {
            return Err(format!(
                "draft decision outline_id {} does not match current draft {}",
                outline_id, draft_candidate.draft_record.outline_id
            ));
        }
        if !draft_candidate.can_accept
            || draft_candidate.draft_record.status == WorkItemDraftStatus::ValidationFailed
        {
            return Err("current work item draft has local validation errors".to_string());
        }

        let store = self.work_item_plan_store()?;
        let mut record = draft_candidate.draft_record.clone();
        let now = chrono::Utc::now().to_rfc3339();
        record.status = WorkItemDraftStatus::Accepted;
        record.accepted_at = Some(now.clone());
        record.updated_at = now.clone();
        store
            .put_draft_record(&record)
            .map_err(|error| format!("save accepted work item draft failed: {error}"))?;

        let mut index = store
            .load_active_index(
                &self.session.project_id,
                &self.session.issue_id,
                &self.session.entity_id,
            )
            .map_err(|error| format!("load work item plan active index failed: {error}"))?
            .ok_or_else(|| "work item plan active index missing".to_string())?;
        index
            .draft_statuses
            .insert(record.draft_id.clone(), WorkItemDraftStatus::Accepted);
        let review_enabled =
            self.session.review_rounds > 0 && self.session.reviewer_provider.is_some();

        self.update_artifact(ArtifactPayload::WorkItemDraftCandidate {
            draft_candidate: Box::new(WorkItemDraftCandidatePayload {
                draft_record: record.clone(),
                validator_findings: draft_candidate.validator_findings.clone(),
                can_accept: true,
            }),
        })
        .await;
        self.complete_active_node(Some("Work Item Draft 已接受".to_string()))
            .await;

        if review_enabled {
            index.active_outline_id = Some(outline_id.clone());
            index.updated_at = now;
            store
                .save_active_index(&index)
                .map_err(|error| format!("save work item plan active index failed: {error}"))?;
            self.begin_work_item_draft_review_run(&outline_id).await;
            return Ok(WorkItemDraftDecisionOutcome::StartReview);
        }

        let outline_order = {
            let outline_candidate = self.latest_work_item_plan_outline_candidate()?;
            work_item_plan_outline_topological_order(&outline_candidate.outline)?
        };
        let current_pos = outline_order
            .iter()
            .position(|id| id == &outline_id)
            .ok_or_else(|| format!("outline {outline_id} not found in order"))?;
        let next_outline_id = outline_order.get(current_pos + 1).cloned();

        if let Some(next_outline_id) = next_outline_id {
            index.active_outline_id = Some(next_outline_id.clone());
            index.updated_at = now;
            store
                .save_active_index(&index)
                .map_err(|error| format!("save work item plan active index failed: {error}"))?;
            self.create_serial_work_item_draft_run_node(&next_outline_id)
                .await;
            Ok(WorkItemDraftDecisionOutcome::StartDraftRun)
        } else {
            index.active_outline_id = None;
            index.updated_at = now;
            store
                .save_active_index(&index)
                .map_err(|error| format!("save work item plan active index failed: {error}"))?;
            self.enter_work_item_plan_compile().await;
            Ok(WorkItemDraftDecisionOutcome::HumanConfirm)
        }
    }

    pub async fn request_work_item_plan_outline_revision(
        &mut self,
        feedback: Option<String>,
    ) -> Result<(), String> {
        if self.session.stage != WorkspaceStage::AuthorConfirm
            || self.active_node_type() != Some(TimelineNodeType::WorkItemGenerationMode)
        {
            return Err(
                "request_outline_revision requires active work_item_generation_mode node"
                    .to_string(),
            );
        }
        self.pending_revision_context = feedback;
        self.mark_work_item_plan_outline_revising()?;
        self.complete_active_node(Some("已返回 WorkItemPlan Outline 返修".to_string()))
            .await;
        self.transition_stage(WorkspaceStage::Running).await;
        self.begin_work_item_plan_outline_run().await;
        Ok(())
    }

    async fn rewrite_current_work_item_batch(
        &mut self,
    ) -> Result<WorkItemBatchDecisionOutcome, String> {
        let store = self.work_item_plan_store()?;
        let mut index = store
            .load_active_index(
                &self.session.project_id,
                &self.session.issue_id,
                &self.session.entity_id,
            )
            .map_err(|error| format!("load work item plan active index failed: {error}"))?
            .ok_or_else(|| "work item plan active index missing".to_string())?;
        let current_batch = current_work_item_batch(&index)?.clone();
        let now = chrono::Utc::now().to_rfc3339();
        let old_draft_ids: Vec<String> = current_batch
            .item_draft_ids
            .iter()
            .chain(current_batch.validation_failed_ids.iter())
            .cloned()
            .collect();
        for draft_id in &old_draft_ids {
            let mut record = store
                .get_draft_record(
                    &index.project_id,
                    &index.issue_id,
                    &index.plan_id,
                    &index.current_generation_round_id,
                    draft_id,
                )
                .map_err(|error| format!("load batch draft record failed: {error}"))?;
            mark_draft_record_superseded(
                &mut record,
                None,
                WorkItemDraftSupersedeReason::DirectRewrite,
                &now,
            );
            store
                .put_draft_record(&record)
                .map_err(|error| format!("save superseded batch draft failed: {error}"))?;
            index
                .draft_statuses
                .insert(draft_id.clone(), WorkItemDraftStatus::Superseded);
            if index
                .outline_to_current_draft_id
                .get(&record.outline_id)
                .is_some_and(|current_draft_id| current_draft_id == draft_id)
            {
                index.outline_to_current_draft_id.remove(&record.outline_id);
            }
        }

        let outline_candidate = self.latest_work_item_plan_outline_candidate()?;
        let first_outline_id =
            work_item_plan_outline_topological_order(&outline_candidate.outline)?
                .into_iter()
                .next()
                .ok_or_else(|| "WorkItemPlan Outline has no work item outlines".to_string())?;
        let new_batch = WorkItemBatchRecord {
            batch_id: next_batch_id(&index, &now),
            generation_round_id: index.current_generation_round_id.clone(),
            mode: WorkItemGenerationMode::Batch,
            item_draft_ids: Vec::new(),
            status: WorkItemBatchStatus::Generating,
            validation_failed_ids: Vec::new(),
            created_at: now.clone(),
        };
        index.active_outline_id = Some(first_outline_id);
        index.batches.push(new_batch);
        index.updated_at = now;
        store
            .save_active_index(&index)
            .map_err(|error| format!("save work item plan active index failed: {error}"))?;

        self.complete_active_node(Some("Work Item Batch 已请求整组重写".to_string()))
            .await;
        self.transition_stage(WorkspaceStage::Running).await;
        let _ = self
            .create_timeline_node(TimelineNodeDraft {
                node_type: TimelineNodeType::WorkItemBatchRun,
                agent: Some(self.session.author_provider.clone()),
                stage: WorkspaceStage::Running,
                round: None,
                title: "Work Item Batch 生成".to_string(),
                summary: Some("正在整组重写 Work Item Draft".to_string()),
                status: TimelineNodeStatus::Active,
            })
            .await;
        Ok(WorkItemBatchDecisionOutcome::StartBatchRun)
    }

    async fn downgrade_current_work_item_batch_to_serial(
        &mut self,
        first_affected_outline_id: Option<String>,
        feedback: Option<String>,
    ) -> Result<WorkItemBatchDecisionOutcome, String> {
        let store = self.work_item_plan_store()?;
        let mut index = store
            .load_active_index(
                &self.session.project_id,
                &self.session.issue_id,
                &self.session.entity_id,
            )
            .map_err(|error| format!("load work item plan active index failed: {error}"))?
            .ok_or_else(|| "work item plan active index missing".to_string())?;
        if !self.current_round_has_failed_compile(&store, &index)? {
            return Err(
                "downgrade_to_serial is not available before strict validation".to_string(),
            );
        }
        let outline_candidate = self.latest_work_item_plan_outline_candidate()?;
        let outline_order = work_item_plan_outline_topological_order(&outline_candidate.outline)?;
        let target_outline_id = first_affected_outline_id
            .or_else(|| outline_order.first().cloned())
            .ok_or_else(|| "WorkItemPlan Outline has no work item outlines".to_string())?;
        if !outline_order
            .iter()
            .any(|outline_id| outline_id == &target_outline_id)
        {
            return Err(format!(
                "first_affected_outline_id `{target_outline_id}` is not in current outline"
            ));
        }
        let target_pos = outline_order
            .iter()
            .position(|outline_id| outline_id == &target_outline_id)
            .ok_or_else(|| format!("outline {target_outline_id} not found in order"))?;
        let generated_from_node_id = self
            .active_node_id
            .clone()
            .unwrap_or_else(|| "work_item_batch_downgrade".to_string());
        let now = chrono::Utc::now().to_rfc3339();
        let mut accepted_copied_candidates = Vec::new();

        for outline_id in outline_order.iter().take(target_pos) {
            let source_draft_id = index
                .outline_to_current_draft_id
                .get(outline_id)
                .cloned()
                .ok_or_else(|| {
                    format!("cannot downgrade before `{target_outline_id}`: outline `{outline_id}` has no current draft")
                })?;
            let mut source = store
                .get_draft_record(
                    &index.project_id,
                    &index.issue_id,
                    &index.plan_id,
                    &index.current_generation_round_id,
                    &source_draft_id,
                )
                .map_err(|error| format!("load draft for downgrade failed: {error}"))?;
            let current_outline = outline_candidate
                .outline
                .work_item_outlines
                .iter()
                .find(|item| item.outline_id == *outline_id)
                .cloned()
                .ok_or_else(|| format!("outline `{outline_id}` not found"))?;
            let report = WorkItemDraftLocalValidator::validate(
                &source.candidate,
                &accepted_copied_candidates,
                &current_outline,
            );
            let mut copied =
                copy_draft_for_current_round(&index, &source, &generated_from_node_id, &now);
            copied.status = if report.has_errors() {
                WorkItemDraftStatus::ValidationFailed
            } else {
                WorkItemDraftStatus::Accepted
            };
            copied.accepted_at = if copied.status == WorkItemDraftStatus::Accepted {
                Some(now.clone())
            } else {
                None
            };

            store
                .put_draft_record(&copied)
                .map_err(|error| format!("save copied serial draft failed: {error}"))?;
            mark_draft_record_superseded(
                &mut source,
                Some(copied.draft_id.clone()),
                WorkItemDraftSupersedeReason::DirectRewrite,
                &now,
            );
            store
                .put_draft_record(&source)
                .map_err(|error| format!("save superseded batch draft failed: {error}"))?;
            mark_draft_active(
                &mut index,
                outline_id,
                &copied.draft_id,
                copied.status.clone(),
            );
            if copied.status == WorkItemDraftStatus::Accepted {
                accepted_copied_candidates.push(copied.candidate.clone());
            } else {
                return Err(format!(
                    "copied draft for outline `{outline_id}` failed local validation during downgrade"
                ));
            }
        }

        index.active_outline_id = Some(target_outline_id.clone());
        index.updated_at = now;
        store
            .save_active_index(&index)
            .map_err(|error| format!("save work item plan active index failed: {error}"))?;

        self.complete_active_node(Some(
            feedback
                .map(|feedback| format!("已降级为逐项生成：{feedback}"))
                .unwrap_or_else(|| "已降级为逐项生成".to_string()),
        ))
        .await;
        self.start_serial_work_item_draft_run_for(&target_outline_id)
            .await?;
        Ok(WorkItemBatchDecisionOutcome::StartDraftRun)
    }

    fn current_round_has_failed_compile(
        &self,
        store: &WorkItemPlanStore,
        index: &WorkItemPlanDraftActiveIndex,
    ) -> Result<bool, String> {
        let transactions = store
            .list_compile_transactions(&index.project_id, &index.issue_id, &index.plan_id)
            .map_err(|error| format!("list compile transactions failed: {error}"))?;
        Ok(transactions.iter().any(|tx| {
            tx.generation_round_id == index.current_generation_round_id
                && tx.status == WorkItemPlanCompileStatus::Failed
                && tx.plan_commit_state == WorkItemPlanCommitState::NotStarted
        }))
    }

    pub async fn begin_work_item_plan_auto_revision_run(&mut self, round: u32) -> String {
        self.transition_stage(WorkspaceStage::Revision).await;
        self.create_timeline_node(TimelineNodeDraft {
            node_type: TimelineNodeType::Revision,
            agent: Some(self.session.author_provider.clone()),
            stage: WorkspaceStage::Revision,
            round: Some(round),
            title: format!("Work Item Plan 自动返修 Round {round}"),
            summary: Some("根据 Work Item Plan 校验结果自动返修".to_string()),
            status: TimelineNodeStatus::Active,
        })
        .await
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

    pub async fn append_active_run_stream(
        &mut self,
        role: &str,
        content: impl Into<String>,
    ) -> Result<(), String> {
        let content = content.into();
        let node_id = self.active_node_id.clone();
        let persist_result = if let Some(node_id) = node_id.as_deref() {
            match self.buffer_stream_chunk(node_id, content.clone()).await {
                Ok(()) => self.flush_stream_buffer(node_id).await,
                Err(error) => Err(error),
            }
        } else {
            Ok(())
        };
        let _ = self
            .event_tx
            .send(EngineEvent::StreamChunk {
                role: role.to_string(),
                content,
                node_id,
            })
            .await;
        persist_result
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
        self.drive_provider_session(ProviderSessionDriveInput {
            session,
            command_rx,
            node_id: Some(generation_node_id),
            agent: Some(self.session.author_provider.clone()),
            role: ProviderConversationRole::Author,
            artifact_retry: Some(retry_context),
            revision_resume_fallback: None,
        })
        .await;
    }

    async fn drive_provider_session(&mut self, input: ProviderSessionDriveInput) {
        let ProviderSessionDriveInput {
            session,
            mut command_rx,
            node_id,
            agent,
            role,
            mut artifact_retry,
            mut revision_resume_fallback,
        } = input;
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
                            let retry_provider =
                                revision_resume_fallback.as_mut().and_then(|context| {
                                    if !context.attempted && is_codex_resume_stall_failure(&message)
                                    {
                                        context.attempted = true;
                                        Some(context.provider.clone())
                                    } else {
                                        None
                                    }
                                });
                            if let Some(provider) = retry_provider {
                                let retry_input = match self.build_revision_input_without_resume() {
                                    Ok(input) => input,
                                    Err(error) => {
                                        let _ = self
                                            .event_tx
                                            .send(EngineEvent::Error { message: error })
                                            .await;
                                        self.finish_failed_run().await;
                                        return;
                                    }
                                };
                                if let Some(context) = artifact_retry.as_mut() {
                                    context.input = retry_input.clone();
                                }
                                if let Some(node_id) = node_id.as_deref() {
                                    let _ = self
                                        .persist_prompt_snapshot(node_id, retry_input.prompt.clone())
                                        .await;
                                    self.emit_execution_event(
                                        provider_prompt_event(
                                            node_id,
                                            retry_input.prompt.clone(),
                                            "Codex resume 无事件，改用新 thread 的完整返修提示词",
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
                                                Some("Provider fresh retry 启动失败".to_string()),
                                            )
                                            .await;
                                        }
                                        self.finish_failed_run().await;
                                        return;
                                    }
                                }
                            }
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

    pub async fn drive_work_item_plan_provider_session_to_output(
        &mut self,
        session: Result<
            ProviderSession,
            crate::cross_cutting::provider_adapter::ProviderAdapterError,
        >,
        command_rx: &mut mpsc::Receiver<ProviderCommand>,
        node_id: String,
        agent: ProviderName,
    ) -> Result<String, String> {
        let mut session = match session {
            Ok(session) => session,
            Err(error) => {
                let message = error.details.clone();
                let _ = self
                    .event_tx
                    .send(EngineEvent::Error {
                        message: message.clone(),
                    })
                    .await;
                self.update_timeline_node(
                    &node_id,
                    TimelineNodeStatus::Failed,
                    Some("Provider 启动失败".to_string()),
                )
                .await;
                self.finish_failed_run().await;
                return Err(message);
            }
        };

        let cancel = self.cancel.clone();
        let mut full_content = String::new();
        let mut events_open = true;
        let mut commands_open = true;
        let mut tool_call_titles = BTreeMap::new();
        let mut tool_call_commands = BTreeMap::new();
        let mut display_filter = StructuredOutputDisplayFilter::new();

        while events_open {
            tokio::select! {
                _ = cancel.cancelled() => {
                    let display_content = display_filter.finish();
                    self.emit_work_item_plan_display_chunk(&node_id, display_content).await;
                    let _ = self.flush_stream_buffer(&node_id).await;
                    self.finish_aborted_run().await;
                    return Err("provider run aborted".to_string());
                }
                command = command_rx.recv(), if commands_open => {
                    match command {
                        Some(ProviderCommand::Abort) => {
                            let _ = session.commands.send(ProviderCommand::Abort).await;
                            cancel.cancel();
                            let display_content = display_filter.finish();
                            self.emit_work_item_plan_display_chunk(&node_id, display_content).await;
                            let _ = self.flush_stream_buffer(&node_id).await;
                            self.finish_aborted_run().await;
                            return Err("provider run aborted".to_string());
                        }
                        Some(ProviderCommand::PermissionResponse {
                            id,
                            approved,
                            reason,
                        }) => {
                            let _ = self
                                .persist_permission_response(
                                    &node_id,
                                    id.clone(),
                                    serde_json::json!({
                                        "approved": approved,
                                        "reason": reason.clone(),
                                    }),
                                )
                                .await;
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
                            if session.commands.send(ProviderCommand::ChoiceResponse {
                                id,
                                selected_option_ids,
                                free_text,
                            }).await.is_err() {
                                commands_open = false;
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
                            full_content.push_str(&content);
                            let display_content = display_filter.push(&content);
                            self.emit_work_item_plan_display_chunk(&node_id, display_content).await;
                        }
                        ProviderEvent::PermissionRequest(request) => {
                            let _ = self
                                .persist_permission_request(
                                    &node_id,
                                    request.id.clone(),
                                    serde_json::json!({
                                        "tool_name": request.tool_name.clone(),
                                        "description": request.description.clone(),
                                        "risk_level": risk_level_text(&request.risk_level),
                                    }),
                                )
                                .await;
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
                                    node_id: Some(node_id.clone()),
                                    agent: Some(agent.clone()),
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
                                    Some(node_id.clone()),
                                    Some(agent.clone()),
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
                                    Some(node_id.clone()),
                                    Some(agent.clone()),
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
                                    Some(node_id.clone()),
                                    Some(agent.clone()),
                                )
                                .await;
                        }
                        ProviderEvent::Completed {
                            full_output,
                            provider_session_id,
                        } => {
                            let display_content = display_filter.finish();
                            self.emit_work_item_plan_display_chunk(&node_id, display_content).await;
                            let _ = self.flush_stream_buffer(&node_id).await;
                            self
                                .record_provider_session(
                                    ProviderConversationRole::Author,
                                    agent,
                                    provider_session_id,
                                    Some(node_id),
                                )
                                .await;
                            return Ok(full_output);
                        }
                        ProviderEvent::Failed { message } => {
                            let display_content = display_filter.finish();
                            self.emit_work_item_plan_display_chunk(&node_id, display_content).await;
                            let _ = self.flush_stream_buffer(&node_id).await;
                            let _ = self
                                .event_tx
                                .send(EngineEvent::Error {
                                    message: message.clone(),
                                })
                                .await;
                            self.update_timeline_node(
                                &node_id,
                                TimelineNodeStatus::Failed,
                                Some("Provider 运行失败".to_string()),
                            )
                            .await;
                            self.finish_failed_run().await;
                            return Err(message);
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
                            self
                                .handle_permission_timeout(
                                    permission_id.clone(),
                                    Some(node_id.clone()),
                                )
                                .await;
                            return Err(format!("permission timeout: {permission_id}"));
                        }
                    }
                }
            }
        }

        let display_content = display_filter.finish();
        self.emit_work_item_plan_display_chunk(&node_id, display_content)
            .await;
        let _ = self.flush_stream_buffer(&node_id).await;
        if full_content.is_empty() {
            self.finish_empty_assistant_output().await;
            Err("provider completed without output".to_string())
        } else {
            Ok(full_content)
        }
    }

    async fn emit_work_item_plan_display_chunk(&mut self, node_id: &str, content: String) {
        if content.is_empty() {
            return;
        }
        let _ = self.buffer_stream_chunk(node_id, content.clone()).await;
        let _ = self
            .event_tx
            .send(EngineEvent::StreamChunk {
                role: "assistant".to_string(),
                content,
                node_id: Some(node_id.to_string()),
            })
            .await;
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
                    upsert_execution_event_json(&mut detail.execution_events, event_json);
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
        let revision_resume_fallback = if input.resume_provider_session_id.is_some()
            && self.session.author_provider == ProviderName::Codex
        {
            Some(RevisionResumeFallbackContext {
                provider: provider.clone(),
                attempted: false,
            })
        } else {
            None
        };
        let session = provider.start(input, self.cancel.clone()).await;
        self.drive_provider_session(ProviderSessionDriveInput {
            session,
            command_rx,
            node_id,
            agent: Some(author),
            role: ProviderConversationRole::Author,
            artifact_retry: Some(retry_context),
            revision_resume_fallback,
        })
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
        let active_node_type = self.active_node_type();
        let verdict = self.parse_review_verdict_for_active_node(&output);
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
                    "findings": verdict.findings.clone(),
                    "review_gate": verdict.review_gate.clone(),
                    "work_item_plan_review": verdict.work_item_plan_review.clone(),
                }),
            )
            .await;
        let _ = self
            .event_tx
            .send(review_complete_event_from_verdict(
                node_id.clone(),
                round,
                &verdict,
            ))
            .await;
        self.update_timeline_node(
            &node_id,
            TimelineNodeStatus::Completed,
            Some(verdict.summary.clone()),
        )
        .await;
        let artifact_verdict = match &verdict.review_gate {
            ReviewGate::RequiresRevision => ReviewVerdictType::Revise,
            ReviewGate::UserConfirmAllowed => match &verdict.verdict {
                ReviewVerdictType::Pass => ReviewVerdictType::Pass,
                ReviewVerdictType::Revise | ReviewVerdictType::NeedsHuman => {
                    ReviewVerdictType::NeedsHuman
                }
            },
            ReviewGate::UserTriageRequired => ReviewVerdictType::NeedsHuman,
        };
        self.mark_latest_artifact_reviewed(reviewer, Some(artifact_verdict));

        if active_node_type == Some(TimelineNodeType::WorkItemPlanOutlineReview) {
            self.route_work_item_plan_outline_review(verdict).await;
            return;
        }

        if active_node_type == Some(TimelineNodeType::WorkItemDraftReview) {
            self.route_work_item_draft_review(verdict).await;
            return;
        }

        if active_node_type == Some(TimelineNodeType::WorkItemBatchReview) {
            self.route_work_item_batch_review(verdict).await;
            return;
        }

        match &verdict.review_gate {
            ReviewGate::UserConfirmAllowed | ReviewGate::UserTriageRequired => {
                self.enter_human_confirm(Some(verdict.summary)).await;
            }
            ReviewGate::RequiresRevision => {
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

    async fn route_work_item_plan_outline_review(&mut self, verdict: ReviewVerdict) {
        let outline_verdict = verdict
            .work_item_plan_review
            .as_ref()
            .map(|review| review.verdict.clone());
        match outline_verdict.unwrap_or(match verdict.verdict {
            ReviewVerdictType::Pass => WorkItemPlanReviewVerdict::Pass,
            ReviewVerdictType::Revise => WorkItemPlanReviewVerdict::Revise,
            ReviewVerdictType::NeedsHuman => WorkItemPlanReviewVerdict::NeedsHuman,
        }) {
            WorkItemPlanReviewVerdict::Pass => {
                self.enter_work_item_generation_mode(Some(
                    "Outline review 通过，请选择 Work Item 生成模式".to_string(),
                ))
                .await;
            }
            WorkItemPlanReviewVerdict::Revise | WorkItemPlanReviewVerdict::PlanReopenRequired => {
                if let Err(message) = self.mark_work_item_plan_outline_revising() {
                    let _ = self.event_tx.send(EngineEvent::Error { message }).await;
                    self.enter_human_confirm(Some("Outline 返修状态保存失败".to_string()))
                        .await;
                    return;
                }
                self.pending_revision_context = Some(verdict.comments);
                self.transition_stage(WorkspaceStage::Running).await;
                self.begin_work_item_plan_outline_run().await;
            }
            WorkItemPlanReviewVerdict::NeedsHuman | WorkItemPlanReviewVerdict::ReviseBatch => {
                self.enter_human_confirm(Some(verdict.summary)).await;
            }
        }
    }

    async fn route_work_item_draft_review(&mut self, verdict: ReviewVerdict) {
        let draft_payload = match self.current_work_item_draft_candidate_payload() {
            Ok(payload) => payload,
            Err(message) => {
                let _ = self.event_tx.send(EngineEvent::Error { message }).await;
                self.enter_human_confirm(Some("Work Item Draft artifact 缺失".to_string()))
                    .await;
                return;
            }
        };
        let current_outline_id = draft_payload.draft_record.outline_id.clone();
        let review = verdict.work_item_plan_review.clone();
        let item_verdict = review
            .as_ref()
            .map(|review| review.verdict.clone())
            .unwrap_or(match verdict.verdict {
                ReviewVerdictType::Pass => WorkItemPlanReviewVerdict::Pass,
                ReviewVerdictType::Revise => WorkItemPlanReviewVerdict::Revise,
                ReviewVerdictType::NeedsHuman => WorkItemPlanReviewVerdict::NeedsHuman,
            });
        let target_outline_id = review
            .as_ref()
            .and_then(|review| review.target_outline_id.clone())
            .unwrap_or_else(|| current_outline_id.clone());

        match item_verdict {
            WorkItemPlanReviewVerdict::Pass => {
                if let Err(message) = self
                    .continue_after_work_item_draft_review_pass(&current_outline_id)
                    .await
                {
                    let _ = self.event_tx.send(EngineEvent::Error { message }).await;
                    self.enter_human_confirm(Some(
                        "继续生成下一个 Work Item Draft 失败".to_string(),
                    ))
                    .await;
                }
            }
            WorkItemPlanReviewVerdict::Revise => {
                if target_outline_id != current_outline_id {
                    self.enter_human_confirm(Some(
                        "Reviewer 要求修改非当前 Work Item，已转人工确认".to_string(),
                    ))
                    .await;
                    return;
                }
                self.pending_revision_context = Some(verdict.comments);
                if let Err(message) = self
                    .start_serial_work_item_draft_run_for(&current_outline_id)
                    .await
                {
                    let _ = self.event_tx.send(EngineEvent::Error { message }).await;
                    self.enter_human_confirm(Some("重写当前 Work Item Draft 失败".to_string()))
                        .await;
                }
            }
            WorkItemPlanReviewVerdict::PlanReopenRequired => {
                if let Err(message) = self.mark_work_item_plan_outline_revising() {
                    let _ = self.event_tx.send(EngineEvent::Error { message }).await;
                    self.enter_human_confirm(Some("Outline 返修状态保存失败".to_string()))
                        .await;
                    return;
                }
                self.enter_human_confirm(Some(
                    "Reviewer 要求重开 Outline，已暂停逐项生成".to_string(),
                ))
                .await;
            }
            WorkItemPlanReviewVerdict::NeedsHuman | WorkItemPlanReviewVerdict::ReviseBatch => {
                self.enter_human_confirm(Some(verdict.summary)).await;
            }
        }
    }

    async fn route_work_item_batch_review(&mut self, verdict: ReviewVerdict) {
        let review = verdict.work_item_plan_review.clone();
        let batch_verdict = review
            .as_ref()
            .map(|review| review.verdict.clone())
            .unwrap_or(match verdict.verdict {
                ReviewVerdictType::Pass => WorkItemPlanReviewVerdict::Pass,
                ReviewVerdictType::Revise => WorkItemPlanReviewVerdict::ReviseBatch,
                ReviewVerdictType::NeedsHuman => WorkItemPlanReviewVerdict::NeedsHuman,
            });

        match batch_verdict {
            WorkItemPlanReviewVerdict::Pass => {
                if let Err(message) = self.mark_current_work_item_batch_review_done() {
                    let _ = self.event_tx.send(EngineEvent::Error { message }).await;
                    self.enter_human_confirm(Some("Batch review 状态保存失败".to_string()))
                        .await;
                    return;
                }
                self.enter_work_item_plan_compile().await;
            }
            WorkItemPlanReviewVerdict::ReviseBatch => {
                self.enter_work_item_batch_confirm(Some(verdict.summary))
                    .await;
            }
            WorkItemPlanReviewVerdict::PlanReopenRequired => {
                if let Err(message) = self.mark_work_item_plan_outline_revising() {
                    let _ = self.event_tx.send(EngineEvent::Error { message }).await;
                    self.enter_human_confirm(Some("Outline 返修状态保存失败".to_string()))
                        .await;
                    return;
                }
                self.enter_human_confirm(Some(
                    "Reviewer 要求重开 Outline，已暂停自动生成流程".to_string(),
                ))
                .await;
            }
            WorkItemPlanReviewVerdict::NeedsHuman | WorkItemPlanReviewVerdict::Revise => {
                self.enter_human_confirm(Some(verdict.summary)).await;
            }
        }
    }

    fn mark_current_work_item_batch_review_done(&self) -> Result<(), String> {
        let store = self.work_item_plan_store()?;
        let mut index = store
            .load_active_index(
                &self.session.project_id,
                &self.session.issue_id,
                &self.session.entity_id,
            )
            .map_err(|error| format!("load work item plan active index failed: {error}"))?
            .ok_or_else(|| "work item plan active index missing".to_string())?;
        let batch_id = current_work_item_batch(&index)?.batch_id.clone();
        let batch = index
            .batches
            .iter_mut()
            .find(|batch| batch.batch_id == batch_id)
            .ok_or_else(|| format!("batch `{batch_id}` not found"))?;
        batch.status = WorkItemBatchStatus::ReviewDone;
        index.updated_at = chrono::Utc::now().to_rfc3339();
        store
            .save_active_index(&index)
            .map_err(|error| format!("save work item plan active index failed: {error}"))?;
        Ok(())
    }

    async fn continue_after_work_item_draft_review_pass(
        &mut self,
        outline_id: &str,
    ) -> Result<(), String> {
        let store = self.work_item_plan_store()?;
        let mut index = store
            .load_active_index(
                &self.session.project_id,
                &self.session.issue_id,
                &self.session.entity_id,
            )
            .map_err(|error| format!("load work item plan active index failed: {error}"))?
            .ok_or_else(|| "work item plan active index missing".to_string())?;
        let outline_candidate = self.latest_work_item_plan_outline_candidate()?;
        let outline_order = work_item_plan_outline_topological_order(&outline_candidate.outline)?;
        let current_pos = outline_order
            .iter()
            .position(|id| id == outline_id)
            .ok_or_else(|| format!("outline {outline_id} not found in order"))?;
        let now = chrono::Utc::now().to_rfc3339();

        if let Some(next_outline_id) = outline_order.get(current_pos + 1).cloned() {
            index.active_outline_id = Some(next_outline_id.clone());
            index.updated_at = now;
            store
                .save_active_index(&index)
                .map_err(|error| format!("save work item plan active index failed: {error}"))?;
            self.create_serial_work_item_draft_run_node(&next_outline_id)
                .await;
        } else {
            index.active_outline_id = None;
            index.updated_at = now;
            store
                .save_active_index(&index)
                .map_err(|error| format!("save work item plan active index failed: {error}"))?;
            self.enter_work_item_plan_compile().await;
        }
        Ok(())
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

        if self.session.workspace_type == WorkspaceType::WorkItemPlan {
            match self.active_node_type() {
                Some(TimelineNodeType::WorkItemPlanOutlineConfirm) => {
                    return self.handle_work_item_plan_outline_decision(decision).await;
                }
                Some(TimelineNodeType::WorkItemGenerationMode) => {
                    return Err(
                        "author_decision is not valid on work_item_generation_mode node"
                            .to_string(),
                    );
                }
                _ => {}
            }
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

    async fn handle_work_item_plan_outline_decision(
        &mut self,
        decision: AuthorDecision,
    ) -> Result<AuthorDecisionOutcome, String> {
        match decision {
            AuthorDecision::Accept => {
                let generation_round_id = self.save_confirmed_work_item_plan_outline_index()?;
                self.update_work_item_plan_outline_generation_metadata(
                    Some(generation_round_id.clone()),
                    None,
                )
                .await?;
                self.mark_latest_artifact_confirmed(Some("human".to_string()));
                let review_enabled =
                    self.session.review_rounds > 0 && self.session.reviewer_provider.is_some();
                let summary = format!(
                    "WorkItemPlan Outline 已确认，generation_round_id={generation_round_id}"
                );
                self.complete_active_node(Some(summary)).await;
                if review_enabled {
                    self.begin_work_item_plan_outline_review_run().await;
                    Ok(AuthorDecisionOutcome::StartReview)
                } else {
                    self.enter_work_item_generation_mode(Some(
                        "请选择 Work Item 生成模式".to_string(),
                    ))
                    .await;
                    Ok(AuthorDecisionOutcome::HumanConfirm)
                }
            }
            AuthorDecision::Reject => {
                self.mark_latest_artifact_rejected();
                self.complete_active_node(Some("用户要求重写 WorkItemPlan Outline".to_string()))
                    .await;
                self.mark_work_item_plan_outline_revising()?;
                self.transition_stage(WorkspaceStage::Running).await;
                self.begin_work_item_plan_outline_run().await;
                Ok(AuthorDecisionOutcome::HumanConfirm)
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

        if self.session.workspace_type == WorkspaceType::WorkItemPlan
            && self.active_node_type() == Some(TimelineNodeType::WorkItemPlanContextBlocker)
        {
            return self
                .handle_work_item_plan_context_blocker_decision(decision, payload)
                .await;
        }

        match decision {
            HumanConfirmDecision::Confirm => match self.handle_confirm().await? {
                WorkspaceConfirmOutcome::None => Ok(ReviewDecisionOutcome::HumanConfirm),
                WorkspaceConfirmOutcome::WorkItemPlan { child_sessions } => {
                    Ok(ReviewDecisionOutcome::ConfirmedWithChildSessions { child_sessions })
                }
            },
            HumanConfirmDecision::RequestChange => {
                let context = human_confirm_payload_description(payload);
                if self.latest_review_verdict.is_none() {
                    self.latest_review_verdict = Some(ReviewVerdict {
                        verdict: ReviewVerdictType::Revise,
                        comments: context
                            .clone()
                            .unwrap_or_else(|| "人工请求修改".to_string()),
                        summary: "人工请求修改".to_string(),
                        findings: Vec::new(),
                        review_gate: ReviewGate::RequiresRevision,
                        work_item_plan_review: None,
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

    async fn handle_work_item_plan_context_blocker_decision(
        &mut self,
        decision: HumanConfirmDecision,
        payload: Option<serde_json::Value>,
    ) -> Result<ReviewDecisionOutcome, String> {
        match decision {
            HumanConfirmDecision::Confirm => Err(
                "work item plan context blocker cannot be confirmed; provide context or terminate"
                    .to_string(),
            ),
            HumanConfirmDecision::RequestChange => {
                let resolution = human_confirm_payload_description(payload).ok_or_else(|| {
                    "work item plan context blocker requires non-empty context".to_string()
                })?;
                self.append_work_item_plan_context_blocker_resolution(resolution)
                    .await?;
                if let Some(store) = &self.lifecycle_store {
                    let _ = store.update_workspace_session_status(
                        &self.session.session_id,
                        WorkspaceSessionStatus::Open,
                    );
                }
                self.transition_stage(WorkspaceStage::Running).await;
                Ok(ReviewDecisionOutcome::StartWorkItemPlanOutline)
            }
            HumanConfirmDecision::Terminate => {
                self.complete_active_node(Some("已终止 WorkItemPlan Outline 生成".to_string()))
                    .await;
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
                        title: "WorkItemPlan Outline 生成已终止".to_string(),
                        summary: Some("用户终止上下文补充流程".to_string()),
                        status: TimelineNodeStatus::Completed,
                    })
                    .await;
                Ok(ReviewDecisionOutcome::HumanConfirm)
            }
        }
    }

    async fn append_work_item_plan_context_blocker_resolution(
        &mut self,
        resolution: String,
    ) -> Result<(), String> {
        let lifecycle = self
            .lifecycle_store
            .clone()
            .ok_or_else(|| "lifecycle_store unavailable".to_string())?;
        let blocker_node_id = self
            .active_node_id
            .clone()
            .ok_or_else(|| "context blocker node unavailable".to_string())?;
        let now = chrono::Utc::now().to_rfc3339();

        self.complete_active_node(Some("已记录上下文补充".to_string()))
            .await;
        let resolution_node = self
            .append_completed_timeline_event(
                TimelineNodeType::ContextNote,
                WorkspaceStage::HumanConfirm,
                "WorkItemPlan 上下文补充".to_string(),
                Some(resolution.clone()),
                TimelineNodeStatus::Completed,
                true,
            )
            .await;
        let resolution_node_id = resolution_node.node_id.clone();
        let artifact_ref = self
            .update_artifact(ArtifactPayload::Markdown {
                markdown: format_context_blocker_resolution_markdown(&resolution),
                diff: None,
            })
            .await;

        let store = WorkItemPlanStore::new(lifecycle.app_paths());
        let project_id = self.session.project_id.clone();
        let issue_id = self.session.issue_id.clone();
        let plan_id = self.session.entity_id.clone();
        let mut index = store
            .load_outline_context_index(&project_id, &issue_id, &plan_id)
            .map_err(|error| format!("load outline context index failed: {error}"))?
            .unwrap_or_else(|| OutlineContextIndex {
                project_id: project_id.clone(),
                issue_id: issue_id.clone(),
                plan_id: plan_id.clone(),
                generation_round_id: "outline_stage".to_string(),
                blocker_resolutions: Vec::new(),
                design_context_gaps: Vec::new(),
                design_context_capabilities: empty_design_context_capabilities(),
                updated_at: now.clone(),
            });

        index
            .blocker_resolutions
            .push(OutlineContextBlockerResolution {
                blocker_node_id: blocker_node_id.clone(),
                resolution_node_id: resolution_node_id.clone(),
                resolution_artifact_ref: format!(
                    "{}/v{}",
                    artifact_ref.artifact_id, artifact_ref.version
                ),
                estimated_tokens: estimate_context_resolution_tokens(&resolution),
                created_at: now.clone(),
                summary: Some(resolution.clone()),
                merged_count: None,
            });
        index.updated_at = now;
        store
            .save_outline_context_index(&index)
            .map_err(|error| format!("save outline context index failed: {error}"))?;
        Ok(())
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
        self.update_artifact(ArtifactPayload::Markdown {
            markdown: artifact_markdown.clone(),
            diff: None,
        })
        .await;

        let message_index = self.session.messages.len() as u32;
        let artifact_snapshot = self.session.artifact.as_ref();
        let checkpoint = self.checkpoint_store.create_checkpoint(
            &self.session.session_id,
            message_index,
            artifact_snapshot,
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
            timeout_secs: DEFAULT_PROVIDER_TIMEOUT_SECS,
        })
    }

    pub fn build_work_item_plan_streaming_input(
        &self,
        provider_type: ProviderType,
        prompt: String,
        worktree_path: String,
    ) -> StreamingProviderInput {
        StreamingProviderInput {
            provider_type,
            role: AdapterRole::WorkItemSplitter,
            prompt,
            working_dir: PathBuf::from(worktree_path),
            workspace_session_id: Some(self.session.session_id.clone()),
            resume_provider_session_id: None,
            permission_mode: ProviderPermissionMode::Supervised,
            env_vars: BTreeMap::new(),
            timeout_secs: DEFAULT_PROVIDER_TIMEOUT_SECS,
        }
    }

    fn build_review_input(&self) -> Result<StreamingProviderInput, String> {
        if matches!(self.session.workspace_type, WorkspaceType::WorkItemPlan) {
            return self.build_work_item_plan_review_input();
        }

        let working_dir = match &self.session.repository_path {
            Some(path) => path.clone(),
            None => std::env::current_dir()
                .map_err(|error| format!("working directory error: {error}"))?,
        };

        let artifact = self
            .session
            .artifact
            .clone()
            .map(|payload| payload.into_markdown().unwrap_or_default())
            .unwrap_or_default();
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
        prompt.push_str("\n当前已提取 Artifact Markdown（daemon 已剥离外层 artifact fence）:\n\n");
        prompt.push_str(&artifact);
        prompt.push_str(
            "\n\n审核边界说明：当前 Artifact 是 daemon 从 author 原始输出中提取后的 markdown，外层 artifact fence 已被剥离是正常状态。\
             不要因为当前 Artifact 未包含外层 artifact fence 判定返修；只审核 markdown 内部一级标题、必需 heading、稳定 ID、追踪关系、内容完整性和设计质量。\
             如果 markdown 正文内部的代码块未闭合或内容结构不合规，仍可按实际问题要求返修。\n",
        );
        let nonce = structured_output_nonce();
        prompt.push_str(&reviewer_output_contract(
            &nonce,
            r#"{"verdict":"pass|revise|needs_human","summary":"一句话摘要","findings":[{"severity":"blocking|must_fix|strong_recommend_fix|suggestion|minor|optional","message":"问题描述","evidence":"当前产物中的具体证据","impact":"为什么影响或不影响下一阶段","required_action":"需要作者执行的最小动作"}]}"#,
            "\n\n请输出审核意见；可以先输出简短可读说明，最终 JSON 必须放在 nonce sentinel block 中，不得使用 Markdown code fence：\n\
             - 只有影响下一阶段可用性的 finding 才能标记为 `blocking`、`must_fix` 或 `strong_recommend_fix`。\n\
             - 风格、措辞、文档美化、未来扩展、非必要补充只能标记为 `suggestion`、`minor` 或 `optional`。\n\
             - 没有强返修 finding 时，必须允许用户确认当前版本，不要为了普通建议使用强返修。\n\
             - 如果输出 `verdict=revise`，必须给出至少一个结构化 finding；否则系统会进入人工裁决而不是自动返修。\n\
             - 第二轮及后续 review 只复核上一轮强返修项是否关闭；除非 revision 新引入真正阻塞问题，不得重新发散普通建议。\n\
             - `pass`：产物可进入最终人工确认。\n\
             - `revise`：仅当存在 blocking/must_fix/strong_recommend_fix finding。\n\
             - `needs_human`：没有明确可自动返修内容，需要用户做产品/范围判断。\n",
        ));

        Ok(StreamingProviderInput {
            provider_type: provider_type_for_name(&provider),
            role: AdapterRole::Reviewer,
            prompt,
            working_dir,
            workspace_session_id: Some(self.session.session_id.clone()),
            resume_provider_session_id: None,
            permission_mode: ProviderPermissionMode::Supervised,
            env_vars: BTreeMap::new(),
            timeout_secs: DEFAULT_PROVIDER_TIMEOUT_SECS,
        })
    }

    fn build_work_item_plan_review_input(&self) -> Result<StreamingProviderInput, String> {
        if self.active_node_type() == Some(TimelineNodeType::WorkItemBatchReview) {
            return self.build_work_item_batch_review_input();
        }

        if self.active_node_type() == Some(TimelineNodeType::WorkItemDraftReview) {
            let draft_candidate = self.current_work_item_draft_candidate_payload()?;
            return self.build_work_item_draft_review_input(&draft_candidate);
        }

        if let Some(ArtifactPayload::WorkItemPlanOutlineCandidate { outline_candidate }) =
            self.session.artifact.as_ref()
        {
            return self.build_work_item_plan_outline_review_input(outline_candidate);
        }

        let lifecycle = self
            .lifecycle_store
            .as_ref()
            .ok_or_else(|| "lifecycle_store unavailable for work_item_plan review".to_string())?;
        let candidate = build_work_item_plan_candidate_dto(
            lifecycle,
            &self.session.project_id,
            &self.session.issue_id,
            &self.session.entity_id,
        )
        .map_err(|error| format!("build work_item_plan candidate dto failed: {error}"))?;

        let working_dir = match &self.session.repository_path {
            Some(path) => path.clone(),
            None => std::env::current_dir()
                .map_err(|error| format!("working directory error: {error}"))?,
        };
        let provider = self
            .session
            .reviewer_provider
            .clone()
            .unwrap_or(ProviderName::Codex);

        let mut prompt = String::new();
        prompt
            .push_str("请作为 reviewer 审核当前 WorkItemPlan 候选（整组 WorkItem 拆分计划）。\n\n");
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

        prompt.push_str("\n## 待审核候选\n\n");
        prompt.push_str(&format!(
            "### Plan\n- id: {}\n- status: {}\n",
            candidate.plan.id, candidate.plan.status
        ));
        prompt.push_str(&format!(
            "- options: include_integration_tests={}, include_e2e_tests={}, force_frontend_backend_split={}, require_execution_plan_confirm={}\n",
            candidate.plan.options.include_integration_tests,
            candidate.plan.options.include_e2e_tests,
            candidate.plan.options.force_frontend_backend_split,
            candidate.plan.options.require_execution_plan_confirm,
        ));

        prompt.push_str("\n### WorkItems\n");
        for wi in &candidate.work_items {
            prompt.push_str(&format!(
                "\n- id: {}\n  kind: {}\n  title: {}\n  depends_on: [{}]\n  exclusive_write_scopes: [{}]\n  verification_plan_ref: {}\n",
                wi.id,
                wi.kind,
                wi.title,
                wi.depends_on.join(", "),
                wi.exclusive_write_scopes.join(", "),
                wi.verification_plan_ref.as_deref().unwrap_or("(none)"),
            ));
        }

        prompt.push_str("\n### dependency_graph\n");
        if candidate.plan.dependency_graph.is_empty() {
            prompt.push_str("(empty)\n");
        } else {
            for edge in &candidate.plan.dependency_graph {
                prompt.push_str(&format!(
                    "- {} -> {}\n",
                    edge.from_work_item_id, edge.to_work_item_id
                ));
            }
        }

        prompt.push_str("\n### validator_findings\n");
        if candidate.validator_findings.is_empty() {
            prompt.push_str("(none)\n");
        } else {
            for finding in &candidate.validator_findings {
                prompt.push_str(&format!(
                    "- [{}] {}: {} (work_items: [{}])\n",
                    finding.severity,
                    finding.code,
                    finding.message,
                    finding.work_item_ids.join(", "),
                ));
            }
        }

        prompt.push_str("\n### Repository Profile (trimmed)\n");
        if let Some(rp) = &candidate.repository_profile {
            prompt.push_str(&format!(
                "- confidence: {}\n- detected_layers: [{}]\n",
                rp.confidence,
                rp.detected_layers.join(", "),
            ));
        } else {
            prompt.push_str("(none)\n");
        }

        prompt.push_str("\n### Verification Plans (summary)\n");
        if candidate.verification_plans.is_empty() {
            prompt.push_str("(none)\n");
        } else {
            for vp in &candidate.verification_plans {
                prompt.push_str(&format!(
                    "- plan_ref: {} | scope: {} | commands: {} | manual_checks: {}\n",
                    vp.plan_ref,
                    vp.scope,
                    vp.commands.len(),
                    vp.manual_checks.len(),
                ));
            }
        }

        prompt.push_str(
            "\n\n审核边界说明：本候选是 WorkItemPlan 整组拆分计划，请从以下维度评估：\
             1) 拆分粒度合理性（是否过粗或过细）；\
             2) 依赖完整性（DAG 是否无环、depends_on 指向存在的 work_item）；\
             3) 写入范围互斥（exclusive_write_scopes 之间无重叠）；\
             4) 跨端拆分恰当性（前端/后端/全栈划分是否合理）；\
             5) 验证计划覆盖度（每个 work_item 的 verification_plan_ref 是否存在、scope 是否匹配）。\
             不要因为 verification_plans 摘要未展开 commands 判定返修；只审核上述五个维度。\n",
        );
        let nonce = structured_output_nonce();
        prompt.push_str(&reviewer_output_contract(
            &nonce,
            r#"{"verdict":"pass|revise|needs_human","summary":"一句话摘要","findings":[{"severity":"blocking|must_fix|strong_recommend_fix|suggestion|minor|optional","message":"问题描述","evidence":"当前产物中的具体证据","impact":"为什么影响或不影响下一阶段","required_action":"需要作者执行的最小动作"}]}"#,
            "\n\n请输出审核意见；可以先输出简短可读说明，最终 JSON 必须放在 nonce sentinel block 中，不得使用 Markdown code fence：\n\
             - 只有影响下一阶段可用性的 finding 才能标记为 `blocking`、`must_fix` 或 `strong_recommend_fix`。\n\
             - 风格、措辞、文档美化、未来扩展、非必要补充只能标记为 `suggestion`、`minor` 或 `optional`。\n\
             - 没有强返修 finding 时，必须允许用户确认当前版本，不要为了普通建议使用强返修。\n\
             - 如果输出 `verdict=revise`，必须给出至少一个结构化 finding；否则系统会进入人工裁决而不是自动返修。\n\
             - 第二轮及后续 review 只复核上一轮强返修项是否关闭；除非 revision 新引入真正阻塞问题，不得重新发散普通建议。\n\
             - `pass`：产物可进入最终人工确认。\n\
             - `revise`：仅当存在 blocking/must_fix/strong_recommend_fix finding。\n\
             - `needs_human`：没有明确可自动返修内容，需要用户做产品/范围判断。\n",
        ));

        Ok(StreamingProviderInput {
            provider_type: provider_type_for_name(&provider),
            role: AdapterRole::Reviewer,
            prompt,
            working_dir,
            workspace_session_id: Some(self.session.session_id.clone()),
            resume_provider_session_id: None,
            permission_mode: ProviderPermissionMode::Supervised,
            env_vars: BTreeMap::new(),
            timeout_secs: DEFAULT_PROVIDER_TIMEOUT_SECS,
        })
    }

    fn build_work_item_plan_outline_review_input(
        &self,
        outline_candidate: &WorkItemPlanOutlineCandidateDto,
    ) -> Result<StreamingProviderInput, String> {
        let working_dir = match &self.session.repository_path {
            Some(path) => path.clone(),
            None => std::env::current_dir()
                .map_err(|error| format!("working directory error: {error}"))?,
        };
        let provider = self
            .session
            .reviewer_provider
            .clone()
            .unwrap_or(ProviderName::Codex);
        let generation_round_id = self
            .work_item_plan_store()
            .ok()
            .and_then(|store| {
                store
                    .load_active_index(
                        &self.session.project_id,
                        &self.session.issue_id,
                        &self.session.entity_id,
                    )
                    .ok()
                    .flatten()
            })
            .map(|index| index.current_generation_round_id)
            .unwrap_or_else(|| "generation_round_unknown".to_string());

        let outline = &outline_candidate.outline;
        let mut prompt = String::new();
        prompt.push_str("请作为 reviewer 审核当前 WorkItemPlan Outline。\n\n");
        prompt.push_str("审核对象只是 Outline 阶段的拆分方案，不是完整 Work Item，不得要求完整 verification plan、required_gates 或 repository_profile。\n\n");
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

        prompt.push_str("\n## Design context gaps\n");
        if outline_candidate.design_context_gaps.is_empty() {
            prompt.push_str("(none)\n");
        } else {
            for gap in &outline_candidate.design_context_gaps {
                prompt.push_str(&format!("- {gap}\n"));
            }
        }

        prompt.push_str("\n## Validator findings\n");
        if outline_candidate.validator_findings.is_empty() {
            prompt.push_str("(none)\n");
        } else {
            for finding in &outline_candidate.validator_findings {
                prompt.push_str(&format!(
                    "- [{}] {}: {}\n",
                    finding.severity, finding.code, finding.message
                ));
            }
        }

        prompt.push_str("\n## Outline\n");
        prompt.push_str(&format!(
            "- id: {}\n- strategy_summary: {}\n- handoff_strategy: {}\n",
            outline.id, outline.strategy_summary, outline.handoff_strategy
        ));
        prompt.push_str("\n### Work item outlines\n");
        for item in &outline.work_item_outlines {
            prompt.push_str(&format!(
                "\n- outline_id: {}\n  title: {}\n  kind: {:?}\n  goal: {}\n  scope: [{}]\n  depends_on: [{}]\n  exclusive_write_scopes: [{}]\n  forbidden_write_scopes: [{}]\n  verification_intent: [{}]\n  handoff_notes: {}\n",
                item.outline_id,
                item.title,
                item.kind,
                item.goal,
                item.scope.join(", "),
                item.depends_on.join(", "),
                item.exclusive_write_scopes.join(", "),
                item.forbidden_write_scopes.join(", "),
                item.verification_intent.join(", "),
                item.handoff_notes,
            ));
        }
        prompt.push_str("\n### Dependency graph\n");
        if outline.dependency_graph.is_empty() {
            prompt.push_str("(empty)\n");
        } else {
            for edge in &outline.dependency_graph {
                prompt.push_str(&format!(
                    "- {} -> {}\n",
                    edge.from_outline_id, edge.to_outline_id
                ));
            }
        }
        prompt.push_str("\n### Risks\n");
        for risk in &outline.risks {
            prompt.push_str(&format!("- {risk}\n"));
        }

        prompt.push_str(
            "\n\n审核边界说明：请只检查拆分策略、覆盖 Story/Design、outline 粒度、依赖图、写入边界、上下文缺口补齐假设与 handoff 策略。\
             不要要求 author 在 Outline 阶段输出完整 Work Item 正文、完整 verification plan、required_gates 或 repository_profile。\
             如果问题会影响拆分边界，返回 `revise`；如果需要用户做产品/范围判断，返回 `needs_human`。\n",
        );
        let nonce = structured_output_nonce();
        let schema = format!(
            r#"{{"verdict":"pass|revise|needs_human","review_scope":"outline","generation_round_id":"{}","summary":"一句话摘要","affects_items":[{{"target_outline_id":"outline id"}}],"findings":[{{"severity":"blocking|must_fix|strong_recommend_fix|suggestion|minor|optional","message":"问题描述","evidence":"Outline 中的具体证据","impact":"为什么影响或不影响 Draft 生成","required_action":"需要 Outline author 执行的最小动作"}}]}}"#,
            generation_round_id
        );
        prompt.push_str(&reviewer_output_contract(
            &nonce,
            &schema,
            "\n\n请输出审核意见；可以先输出简短可读说明，最终 JSON 必须放在 nonce sentinel block 中，不得使用 Markdown code fence：\n\
             - `pass`：Outline 可进入生成模式选择。\n\
             - `revise`：Outline 需要返修，且必须给出至少一个 blocking/must_fix/strong_recommend_fix finding。\n\
             - `needs_human`：需要用户做产品/范围判断。\n\
             - `affects_items.target_outline_id` 只能引用当前 Outline 中存在的 outline_id。\n",
        ));

        Ok(StreamingProviderInput {
            provider_type: provider_type_for_name(&provider),
            role: AdapterRole::Reviewer,
            prompt,
            working_dir,
            workspace_session_id: Some(self.session.session_id.clone()),
            resume_provider_session_id: None,
            permission_mode: ProviderPermissionMode::Supervised,
            env_vars: BTreeMap::new(),
            timeout_secs: DEFAULT_PROVIDER_TIMEOUT_SECS,
        })
    }

    fn build_work_item_batch_review_input(&self) -> Result<StreamingProviderInput, String> {
        let store = self.work_item_plan_store()?;
        let index = store
            .load_active_index(
                &self.session.project_id,
                &self.session.issue_id,
                &self.session.entity_id,
            )
            .map_err(|error| format!("load work item plan active index failed: {error}"))?
            .ok_or_else(|| "work item plan active index missing".to_string())?;
        let batch = current_work_item_batch(&index)?;
        let draft_records =
            self.batch_work_item_plan_draft_records(&store, &index, &batch.batch_id)?;
        let draft_json =
            serde_json::to_string_pretty(&draft_records).unwrap_or_else(|_| "[]".to_string());
        let outline_ids = self.current_work_item_plan_outline_ids();
        let provider = self
            .session
            .reviewer_provider
            .clone()
            .unwrap_or(ProviderName::Codex);
        let nonce = structured_output_nonce();
        let mut prompt = String::new();
        prompt
            .push_str("请作为 reviewer 审核 WorkItemPlan 自动模式生成的整组 Work Item Draft。\n\n");
        prompt.push_str(&format!(
            "generation_round_id: {}\nbatch_id: {}\n\n",
            batch.generation_round_id, batch.batch_id
        ));
        prompt.push_str("[batch_draft_records]\n");
        prompt.push_str(&draft_json);
        prompt.push_str("\n\n");
        prompt.push_str(&reviewer_output_contract(
            &nonce,
            r#"{"verdict":"pass|revise_batch|needs_human|plan_reopen_required","review_scope":"batch","generation_round_id":"round id","summary":"一句话摘要","affects_items":[{"target_outline_id":"outline id"}],"findings":[{"severity":"blocking|must_fix|strong_recommend_fix|suggestion|minor|optional","message":"问题描述","evidence":"整组 draft 或依赖上下文中的具体证据","impact":"为什么影响或不影响 final compile","required_action":"需要 batch author 执行的最小动作"}]}"#,
            "\n\n审核规则：自动模式只能整组通过、整组返修或要求重开 Outline；不得要求单项重写。最终 JSON 必须放在 nonce sentinel block 中。\n",
        ));
        prompt.push_str(&format!(
            "\n[valid_outline_ids]\n{}\n",
            outline_ids.join("\n")
        ));
        let working_dir = self
            .session
            .repository_path
            .clone()
            .or_else(|| std::env::current_dir().ok())
            .ok_or_else(|| "working directory unavailable".to_string())?;
        Ok(StreamingProviderInput {
            provider_type: provider_type_for_name(&provider),
            role: AdapterRole::Reviewer,
            prompt,
            working_dir,
            workspace_session_id: Some(self.session.session_id.clone()),
            resume_provider_session_id: None,
            permission_mode: ProviderPermissionMode::Supervised,
            env_vars: BTreeMap::new(),
            timeout_secs: DEFAULT_PROVIDER_TIMEOUT_SECS,
        })
    }

    fn build_work_item_draft_review_input(
        &self,
        draft_candidate: &WorkItemDraftCandidatePayload,
    ) -> Result<StreamingProviderInput, String> {
        let working_dir = match &self.session.repository_path {
            Some(path) => path.clone(),
            None => std::env::current_dir()
                .map_err(|error| format!("working directory error: {error}"))?,
        };
        let provider = self
            .session
            .reviewer_provider
            .clone()
            .unwrap_or(ProviderName::Codex);
        let outline_candidate = self.latest_work_item_plan_outline_candidate()?;
        let current_outline = outline_candidate
            .outline
            .work_item_outlines
            .iter()
            .find(|outline| outline.outline_id == draft_candidate.draft_record.outline_id)
            .ok_or_else(|| {
                format!(
                    "outline {} not found for draft review",
                    draft_candidate.draft_record.outline_id
                )
            })?;
        let store = self.work_item_plan_store()?;
        let index = store
            .load_active_index(
                &self.session.project_id,
                &self.session.issue_id,
                &self.session.entity_id,
            )
            .map_err(|error| format!("load work item plan active index failed: {error}"))?
            .ok_or_else(|| "work item plan active index missing".to_string())?;
        let accepted_drafts = self.accepted_work_item_plan_draft_records(&store, &index)?;

        let mut prompt = String::new();
        prompt.push_str("请作为 reviewer 审核当前单个 Work Item Draft。\n\n");
        prompt.push_str("审核边界：只能审核当前 draft 是否符合对应 outline 以及是否正确消费已接受依赖。若需要修改当前 item，返回 `revise`；若需要修改前序 item 或拆分边界，必须返回 `plan_reopen_required`；不得用 `revise` 修改非当前 item。\n\n");
        prompt.push_str(&format!(
            "generation_round_id: {}\ndraft_id: {}\ntarget_outline_id: {}\n\n",
            draft_candidate.draft_record.generation_round_id,
            draft_candidate.draft_record.draft_id,
            draft_candidate.draft_record.outline_id
        ));
        prompt.push_str("## Current outline\n");
        prompt.push_str(
            &serde_json::to_string_pretty(current_outline)
                .map_err(|error| format!("serialize current outline failed: {error}"))?,
        );
        prompt.push_str("\n\n## Current draft\n");
        prompt.push_str(
            &serde_json::to_string_pretty(&draft_candidate.draft_record.candidate)
                .map_err(|error| format!("serialize current draft failed: {error}"))?,
        );
        prompt.push_str("\n\n## Local validator findings\n");
        if draft_candidate.validator_findings.is_empty() {
            prompt.push_str("(none)\n");
        } else {
            for finding in &draft_candidate.validator_findings {
                prompt.push_str(&format!(
                    "- [{}] {}: {}\n",
                    finding.severity, finding.code, finding.message
                ));
            }
        }
        prompt.push_str("\n## Accepted previous drafts\n");
        if accepted_drafts.is_empty() {
            prompt.push_str("(none)\n");
        } else {
            for record in &accepted_drafts {
                prompt.push_str(&format!(
                    "- outline_id: {}\n  draft_id: {}\n  title: {}\n  handoff_summary: {}\n  exclusive_write_scopes: [{}]\n",
                    record.outline_id,
                    record.draft_id,
                    record.candidate.title,
                    record.candidate.handoff_summary,
                    record.candidate.exclusive_write_scopes.join(", ")
                ));
            }
        }

        let nonce = structured_output_nonce();
        let schema = format!(
            r#"{{"verdict":"pass|revise|needs_human|plan_reopen_required","review_scope":"item","target_outline_id":"{}","generation_round_id":"{}","draft_id":"{}","summary":"一句话摘要","affects_items":[{{"target_outline_id":"{}"}}],"findings":[{{"severity":"blocking|must_fix|strong_recommend_fix|suggestion|minor|optional","message":"问题描述","evidence":"当前 draft 或依赖上下文中的具体证据","impact":"为什么影响或不影响后续生成","required_action":"需要当前 item author 执行的最小动作"}}]}}"#,
            draft_candidate.draft_record.outline_id,
            draft_candidate.draft_record.generation_round_id,
            draft_candidate.draft_record.draft_id,
            draft_candidate.draft_record.outline_id
        );
        prompt.push_str(&reviewer_output_contract(
            &nonce,
            &schema,
            "\n\n请输出审核意见；可以先输出简短可读说明，最终 JSON 必须放在 nonce sentinel block 中，不得使用 Markdown code fence：\n\
             - `pass`：当前 draft 可进入下一项。\n\
             - `revise`：只允许重写当前 target_outline_id 对应的 draft。\n\
             - `plan_reopen_required`：需要修改前序 item、拆分边界或 Outline 依赖。\n\
             - `needs_human`：需要用户做范围或产品判断。\n",
        ));

        Ok(StreamingProviderInput {
            provider_type: provider_type_for_name(&provider),
            role: AdapterRole::Reviewer,
            prompt,
            working_dir,
            workspace_session_id: Some(self.session.session_id.clone()),
            resume_provider_session_id: None,
            permission_mode: ProviderPermissionMode::Supervised,
            env_vars: BTreeMap::new(),
            timeout_secs: DEFAULT_PROVIDER_TIMEOUT_SECS,
        })
    }

    fn build_revision_input(&self) -> Result<StreamingProviderInput, String> {
        self.build_revision_input_with_resume(true)
    }

    fn build_revision_input_without_resume(&self) -> Result<StreamingProviderInput, String> {
        self.build_revision_input_with_resume(false)
    }

    fn build_revision_input_with_resume(
        &self,
        allow_resume: bool,
    ) -> Result<StreamingProviderInput, String> {
        let working_dir = match &self.session.repository_path {
            Some(path) => path.clone(),
            None => std::env::current_dir()
                .map_err(|error| format!("working directory error: {error}"))?,
        };

        let artifact = self
            .session
            .artifact
            .clone()
            .map(|payload| payload.into_markdown().unwrap_or_default())
            .unwrap_or_default();
        let provider = self.session.author_provider.clone();
        let resume_provider_session_id = if allow_resume {
            self.provider_resume_session_id(ProviderConversationRole::Author, &provider)
        } else {
            None
        };
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
            timeout_secs: DEFAULT_PROVIDER_TIMEOUT_SECS,
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
        self.append_author_artifact_output_contract(&mut prompt, false);
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
        self.append_author_artifact_output_contract(&mut prompt, true);
        prompt.push_str("\n\n请根据以上审核意见修改产物，输出完整更新后的 artifact markdown。\n");
        prompt
    }

    fn append_author_artifact_output_contract(
        &self,
        prompt: &mut String,
        mentions_prior_artifact: bool,
    ) {
        prompt.push_str("\n\n输出格式契约：");
        if mentions_prior_artifact {
            prompt.push_str(
                "上一版 Artifact 是 daemon 已提取的 markdown，外层 artifact fence 已被剥离；不要把上一版 Artifact 的裸 markdown 形态当作原始返回格式样例。",
            );
        } else {
            prompt.push_str(
                "当前 provider 会话中的既有 artifact 是 daemon 已提取的 markdown，外层 artifact fence 可能已被剥离；不要把裸 markdown 形态当作原始返回格式样例。",
            );
        }
        prompt.push_str("原始返回必须使用完整 artifact fenced block，fence 内第一行必须是 ");
        prompt.push_str(workspace_type_title(&self.session.workspace_type));
        prompt.push_str(
            " 一级标题。正文内部包含 ``` 代码块时，外层使用四反引号 ````artifact ... ````，避免和内部代码块冲突。\
             过程说明必须放在 artifact fence 外，最终候选产物必须放在 artifact fence 内。",
        );
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

        self.session.artifact = target.artifact_snapshot.clone();
        if let Some(store) = &self.lifecycle_store {
            let _ = store.truncate_workspace_session_messages(
                &self.session.session_id,
                keep_count,
                workspace_status_for_stage(&self.session.stage),
            );
        }

        Ok(())
    }

    pub async fn handle_confirm(&mut self) -> Result<WorkspaceConfirmOutcome, String> {
        match self.session.stage {
            WorkspaceStage::HumanConfirm => {
                self.complete_active_node(Some("已确认通过".to_string()))
                    .await;
                self.mark_latest_artifact_confirmed(Some("human".to_string()));
                match self.session.workspace_type {
                    WorkspaceType::WorkItemPlan => {
                        let (plan, new_sessions) = self.confirm_work_item_plan().await?;
                        self.transition_stage(WorkspaceStage::Completed).await;
                        let _ = self
                            .create_timeline_node(TimelineNodeDraft {
                                node_type: TimelineNodeType::Completed,
                                agent: None,
                                stage: WorkspaceStage::Completed,
                                round: None,
                                title: "WorkItemPlan 已确认".to_string(),
                                summary: Some(format!(
                                    "plan {} confirmed，已建立 {} 个子 WorkItem session",
                                    plan.id,
                                    new_sessions.len()
                                )),
                                status: TimelineNodeStatus::Completed,
                            })
                            .await;

                        return Ok(WorkspaceConfirmOutcome::WorkItemPlan {
                            child_sessions: new_sessions,
                        });
                    }
                    _ => {
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
                                WorkspaceType::WorkItemPlan => Ok(()),
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
                }
            }
            WorkspaceStage::Running => {
                self.transition_stage(WorkspaceStage::CrossReview).await;
            }
            _ => {}
        }
        Ok(WorkspaceConfirmOutcome::None)
    }

    /// WorkItemPlan 确认：plan/work_items Draft -> Confirmed，并幂等创建子 WorkItem session。
    async fn confirm_work_item_plan(
        &mut self,
    ) -> Result<(IssueWorkItemPlan, Vec<WorkspaceSessionRecord>), String> {
        let lifecycle = self
            .lifecycle_store
            .clone()
            .ok_or("lifecycle_store unavailable")?;
        let project_id = self.session.project_id.clone();
        let issue_id = self.session.issue_id.clone();
        let plan_id = self.session.entity_id.clone();

        let current_plan = lifecycle
            .get_issue_work_item_plan(&project_id, &issue_id, &plan_id)
            .map_err(|e| format!("load plan failed: {e}"))?;
        let plan = match current_plan.status {
            crate::product::models::IssueWorkItemPlanStatus::Draft => {
                lifecycle
                    .confirm_issue_work_item_plan(&project_id, &issue_id, &plan_id)
                    .map_err(|e| format!("confirm plan failed: {e}"))?
                    .0
            }
            crate::product::models::IssueWorkItemPlanStatus::Confirmed => current_plan,
            crate::product::models::IssueWorkItemPlanStatus::ChangeRequested => {
                return Err("cannot confirm a change_requested WorkItemPlan".to_string());
            }
        };

        let _created_sessions = lifecycle
            .ensure_work_item_sessions_for_plan(
                &project_id,
                &issue_id,
                &plan_id,
                self.session.author_provider.clone(),
                self.session.reviewer_provider.clone(),
                self.session.review_rounds,
                self.session.superpowers_enabled,
                self.session.openspec_enabled,
            )
            .map_err(|e| format!("ensure child sessions failed: {e}"))?;
        let plan_work_item_ids: HashSet<String> = plan.work_item_ids.iter().cloned().collect();
        let child_sessions = lifecycle
            .list_workspace_sessions(&project_id, &issue_id)
            .map_err(|e| format!("list child sessions failed: {e}"))?
            .into_iter()
            .filter(|session| {
                session.workspace_type == WorkspaceType::WorkItem
                    && plan_work_item_ids.contains(&session.entity_id)
            })
            .collect::<Vec<_>>();

        if let Some(store) = &self.lifecycle_store {
            let _ = store.update_workspace_session_status(
                &self.session.session_id,
                WorkspaceSessionStatus::Confirmed,
            );
        }

        Ok((plan, child_sessions))
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

    pub async fn update_artifact(&mut self, payload: ArtifactPayload) -> ArtifactRef {
        self.session.artifact = Some(payload.clone());
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
            payload: payload.clone(),
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
        let artifact_ref = ArtifactRef {
            artifact_id: format!("artifact_version_{version:03}"),
            version,
        };
        let _ = self
            .persist_artifact_ref(&source_node_id, artifact_ref.clone())
            .await;
        let _ = self
            .event_tx
            .send(EngineEvent::ArtifactUpdate {
                version,
                payload: payload.clone(),
            })
            .await;
        artifact_ref
    }

    pub async fn complete_work_item_plan_author(
        &mut self,
        output: WorkItemSplitProviderOutput,
    ) -> Result<WorkItemPlanAuthorOutcome, String> {
        let lifecycle = self
            .lifecycle_store
            .clone()
            .ok_or("lifecycle_store unavailable")?;
        let project_id = self.session.project_id.clone();
        let issue_id = self.session.issue_id.clone();
        let plan_id = self.session.entity_id.clone();

        let report = WorkItemSplitValidator::validate(
            &output.plan,
            &output.work_items,
            Some(&output.repository_profile),
            &output.verification_plans,
        );
        let findings = report.findings.clone();

        if report.has_errors() {
            self.work_item_plan_author_retry_count += 1;
            if self.work_item_plan_author_retry_count >= 3 {
                if let Err(error) = lifecycle.replace_issue_work_item_plan_candidate(
                    &project_id,
                    &issue_id,
                    &plan_id,
                    &output,
                    findings.clone(),
                ) {
                    tracing::warn!(%error, "persist final validate findings before HumanConfirm failed");
                }
                self.complete_active_node(Some(work_item_plan_findings_summary(
                    "WorkItemPlan 校验失败，转人工确认",
                    &findings,
                )))
                .await;
                self.enter_human_confirm_for_work_item_plan_author_failure(&findings)
                    .await;
                return Ok(WorkItemPlanAuthorOutcome::HumanConfirm {
                    reason: "validate 连续 3 次失败".to_string(),
                });
            }

            lifecycle
                .replace_issue_work_item_plan_candidate(
                    &project_id,
                    &issue_id,
                    &plan_id,
                    &output,
                    findings.clone(),
                )
                .map_err(|e| format!("replace candidate failed: {e}"))?;
            self.complete_active_node(Some(work_item_plan_findings_summary(
                "WorkItemPlan 校验失败，准备自动返修",
                &findings,
            )))
            .await;
            return Ok(WorkItemPlanAuthorOutcome::AutoRevision { findings });
        }

        // 所有同步落盘与 DTO 组装完成后，再进入异步事件发送；保持锁内同步操作连续，
        // 避免在 await 点之间穿插同步 IO。
        lifecycle
            .replace_issue_work_item_plan_candidate(
                &project_id,
                &issue_id,
                &plan_id,
                &output,
                findings.clone(),
            )
            .map_err(|e| format!("replace candidate failed: {e}"))?;

        let candidate =
            build_work_item_plan_candidate_dto(&lifecycle, &project_id, &issue_id, &plan_id)
                .map_err(|e| format!("build candidate dto failed: {e}"))?;
        self.update_artifact(ArtifactPayload::WorkItemPlanCandidate {
            candidate: Box::new(candidate),
        })
        .await;

        self.complete_active_node(Some("WorkItemPlan provider 输出完成".to_string()))
            .await;
        self.enter_author_confirm(Some("WorkItemPlan 候选已生成，等待确认".to_string()))
            .await;

        self.work_item_plan_author_retry_count = 0;
        Ok(WorkItemPlanAuthorOutcome::AuthorConfirm)
    }

    pub async fn complete_work_item_plan_outline_author(
        &mut self,
        output: OutlineAuthorOutput,
    ) -> Result<WorkItemPlanAuthorOutcome, String> {
        let design_context_gaps = self.current_work_item_plan_design_context_gaps();
        if !output.context_blockers.is_empty() {
            let payload = ArtifactPayload::WorkItemPlanContextBlocker {
                context_blocker: Box::new(WorkItemPlanContextBlockerPayload {
                    context_blockers: work_item_plan_context_blockers_to_dto(
                        &output.context_blockers,
                    ),
                    design_context_gaps: design_context_gaps.clone(),
                    exploration_summary: "Outline author 需要补充上下文后才能继续".to_string(),
                    allowed_actions: vec!["provide_context".to_string(), "abort".to_string()],
                }),
            };
            self.update_artifact(payload).await;
            self.complete_active_node(Some(
                "WorkItemPlan Outline author 请求补充上下文".to_string(),
            ))
            .await;
            self.enter_work_item_plan_context_blocker(Some(
                "请补充 WorkItemPlan Outline 所需上下文".to_string(),
            ))
            .await;
            return Ok(WorkItemPlanAuthorOutcome::HumanConfirm {
                reason: "context_blockers".to_string(),
            });
        }

        let outline = output
            .outline
            .ok_or_else(|| "WorkItemPlan Outline output missing outline".to_string())?;
        let report = WorkItemPlanOutlineValidator::validate(&outline);
        let findings = report.findings.clone();

        if report.has_errors() {
            self.work_item_plan_author_retry_count += 1;
            self.complete_active_node(Some(work_item_plan_findings_summary(
                "WorkItemPlan Outline 校验失败",
                &findings,
            )))
            .await;

            if self.work_item_plan_author_retry_count >= 2 {
                let payload = ArtifactPayload::WorkItemPlanContextBlocker {
                    context_blocker: Box::new(WorkItemPlanContextBlockerPayload {
                        context_blockers: Vec::new(),
                        design_context_gaps: design_context_gaps.clone(),
                        exploration_summary: work_item_plan_findings_summary(
                            "Outline 自动重跑后仍校验失败",
                            &findings,
                        ),
                        allowed_actions: vec!["provide_context".to_string(), "abort".to_string()],
                    }),
                };
                self.update_artifact(payload).await;
                self.enter_work_item_plan_context_blocker(Some(
                    "Outline 校验失败，请补充上下文或终止".to_string(),
                ))
                .await;
                return Ok(WorkItemPlanAuthorOutcome::HumanConfirm {
                    reason: "outline_validation_failed".to_string(),
                });
            }

            return Ok(WorkItemPlanAuthorOutcome::AutoRevision { findings });
        }

        self.update_artifact(ArtifactPayload::WorkItemPlanOutlineCandidate {
            outline_candidate: Box::new(WorkItemPlanOutlineCandidateDto {
                outline,
                design_context_gaps,
                validator_findings: work_item_split_findings_to_dto(&findings),
                context_blockers: Vec::new(),
                current_generation_round_id: None,
                selected_generation_mode: None,
            }),
        })
        .await;
        self.complete_active_node(Some("WorkItemPlan Outline provider 输出完成".to_string()))
            .await;
        self.enter_work_item_plan_outline_confirm(Some(
            "WorkItemPlan Outline 已生成，等待确认".to_string(),
        ))
        .await;
        self.work_item_plan_author_retry_count = 0;
        Ok(WorkItemPlanAuthorOutcome::AuthorConfirm)
    }

    fn current_work_item_plan_design_context_gaps(&self) -> Vec<String> {
        let Some(lifecycle) = &self.lifecycle_store else {
            return Vec::new();
        };
        let store = WorkItemPlanStore::new(lifecycle.app_paths());
        store
            .load_outline_context_index(
                &self.session.project_id,
                &self.session.issue_id,
                &self.session.entity_id,
            )
            .ok()
            .flatten()
            .map(|index| index.design_context_gaps)
            .unwrap_or_default()
    }

    /// WorkItemPlan Revision 完成：validate → replace Draft candidate → 组装 DTO →
    /// `update_artifact(WorkItemPlanCandidate)`（新 version）→ 回 AuthorConfirm。
    ///
    /// 校验逻辑与 `complete_work_item_plan_author` 保持一致：出现 errors 时进入
    /// AutoRevision/HumanConfirm，避免非法候选直接暴露给用户。
    pub async fn complete_work_item_plan_revision(
        &mut self,
        output: WorkItemSplitProviderOutput,
    ) -> Result<WorkItemPlanAuthorOutcome, String> {
        let lifecycle = self
            .lifecycle_store
            .clone()
            .ok_or("lifecycle_store unavailable")?;
        let project_id = self.session.project_id.clone();
        let issue_id = self.session.issue_id.clone();
        let plan_id = self.session.entity_id.clone();

        let report = WorkItemSplitValidator::validate(
            &output.plan,
            &output.work_items,
            Some(&output.repository_profile),
            &output.verification_plans,
        );
        let findings = report.findings.clone();

        if report.has_errors() {
            self.work_item_plan_revision_retry_count += 1;
            if self.work_item_plan_revision_retry_count >= 3 {
                if let Err(error) = lifecycle.replace_issue_work_item_plan_candidate(
                    &project_id,
                    &issue_id,
                    &plan_id,
                    &output,
                    findings.clone(),
                ) {
                    tracing::warn!(%error, "persist final validate findings before HumanConfirm failed");
                }
                self.complete_active_node(Some(work_item_plan_findings_summary(
                    "WorkItemPlan 返修校验失败，转人工确认",
                    &findings,
                )))
                .await;
                self.enter_human_confirm_for_work_item_plan_author_failure(&findings)
                    .await;
                return Ok(WorkItemPlanAuthorOutcome::HumanConfirm {
                    reason: "revision validate 连续 3 次失败".to_string(),
                });
            }

            lifecycle
                .replace_issue_work_item_plan_candidate(
                    &project_id,
                    &issue_id,
                    &plan_id,
                    &output,
                    findings.clone(),
                )
                .map_err(|e| format!("replace candidate failed: {e}"))?;
            self.complete_active_node(Some(work_item_plan_findings_summary(
                "WorkItemPlan 返修校验失败，准备自动返修",
                &findings,
            )))
            .await;
            return Ok(WorkItemPlanAuthorOutcome::AutoRevision { findings });
        }

        lifecycle
            .replace_issue_work_item_plan_candidate(
                &project_id,
                &issue_id,
                &plan_id,
                &output,
                findings.clone(),
            )
            .map_err(|e| format!("replace candidate failed: {e}"))?;

        let candidate =
            build_work_item_plan_candidate_dto(&lifecycle, &project_id, &issue_id, &plan_id)
                .map_err(|e| format!("build candidate dto failed: {e}"))?;
        self.update_artifact(ArtifactPayload::WorkItemPlanCandidate {
            candidate: Box::new(candidate),
        })
        .await;

        self.complete_active_node(Some("WorkItemPlan 返修 provider 输出完成".to_string()))
            .await;
        self.enter_author_confirm(Some("WorkItemPlan 候选已重做，等待确认".to_string()))
            .await;
        self.work_item_plan_revision_retry_count = 0;
        Ok(WorkItemPlanAuthorOutcome::AuthorConfirm)
    }

    /// AuthorConfirm 阶段用户主动请求 revision：进入 Revision 阶段并记录反馈。
    pub async fn request_work_item_plan_revision(
        &mut self,
        feedback: Option<String>,
    ) -> Result<ReviewDecisionOutcome, String> {
        if self.session.stage != WorkspaceStage::AuthorConfirm {
            return Err(
                "request_revision is only available during author_confirm stage".to_string(),
            );
        }
        if self.active_node_type() == Some(TimelineNodeType::WorkItemPlanOutlineConfirm) {
            self.pending_revision_context = feedback;
            self.work_item_plan_revision_retry_count = 0;
            self.mark_latest_artifact_rejected();
            self.complete_active_node(Some("已请求重写 WorkItemPlan Outline".to_string()))
                .await;
            if let Some(store) = &self.lifecycle_store {
                let _ = store.update_workspace_session_status(
                    &self.session.session_id,
                    WorkspaceSessionStatus::Open,
                );
            }
            self.transition_stage(WorkspaceStage::Running).await;
            self.work_item_plan_author_retry_count = 0;
            return Ok(ReviewDecisionOutcome::StartWorkItemPlanOutline);
        }
        self.pending_revision_context = feedback;
        self.work_item_plan_revision_retry_count = 0;
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
        self.work_item_plan_author_retry_count = 0;
        Ok(ReviewDecisionOutcome::StartRevision)
    }

    /// 组装 review / AutoRevision 触发 WorkItemPlan revision 时使用的整体反馈文本。
    pub fn work_item_plan_revision_feedback(&self) -> Option<String> {
        let mut parts = Vec::new();
        if let Some(verdict) = &self.latest_review_verdict {
            if !verdict.comments.is_empty() {
                parts.push(format!("Reviewer 审核意见:\n{}", verdict.comments));
            }
            if !verdict.summary.is_empty() {
                parts.push(format!("摘要: {}", verdict.summary));
            }
            for finding in &verdict.findings {
                parts.push(format!(
                    "[{}] {}",
                    serialized_string(&finding.severity),
                    finding.message
                ));
            }
        }
        if let Some(context) = &self.pending_revision_context {
            parts.push(format!("用户补充信息:\n{}", context));
        }
        if parts.is_empty() {
            None
        } else {
            Some(parts.join("\n\n"))
        }
    }
}

/// 从当前 session artifact 与 lifecycle 构建 WorkItemPlan revision 的输入三元组。
///
/// - `retained`: candidate 中未标记 revert 的项，从 lifecycle 取完整记录。
/// - `redo_specs`: candidate 中标记 revert 的项（old_id + 反馈）。
/// - `request`: 从当前 Draft plan 与 session 配置组装，并注入 `feedback` 作为
///   `revision_feedback`。
pub(crate) fn build_work_item_plan_revision_input(
    engine: &WorkspaceEngine,
    lifecycle: &LifecycleStore,
    feedback: Option<&str>,
) -> Result<
    (
        Vec<LifecycleWorkItemRecord>,
        Vec<RedoSpec>,
        GenerateWorkItemsRequest,
    ),
    String,
> {
    let session = engine.session();
    let plan = lifecycle
        .get_issue_work_item_plan(&session.project_id, &session.issue_id, &session.entity_id)
        .map_err(|e| format!("load plan failed: {e}"))?;
    let candidate = match &session.artifact {
        Some(ArtifactPayload::WorkItemPlanCandidate { candidate }) => candidate,
        _ => return Err("current artifact is not a WorkItemPlanCandidate".to_string()),
    };

    let all_work_items = lifecycle
        .list_work_items(&session.project_id, &session.issue_id)
        .map_err(|e| format!("list work items failed: {e}"))?;
    let by_id: HashMap<String, LifecycleWorkItemRecord> = all_work_items
        .into_iter()
        .map(|wi| (wi.id.clone(), wi))
        .collect();

    let mut retained = Vec::new();
    let mut redo_specs = Vec::new();
    for wi in &candidate.work_items {
        if wi.meta.reverted {
            let item_feedback = match (&wi.meta.revert_feedback, feedback) {
                (Some(rev), Some(overall)) => format!("{}\n\n整体反馈: {}", rev, overall),
                (Some(rev), None) => rev.clone(),
                (None, Some(overall)) => overall.to_string(),
                (None, None) => "请重做".to_string(),
            };
            redo_specs.push(RedoSpec {
                old_id: wi.id.clone(),
                feedback: item_feedback,
            });
        } else {
            let record = by_id
                .get(&wi.id)
                .ok_or_else(|| format!("retained work item {} not found", wi.id))?;
            retained.push(record.clone());
        }
    }

    let provider_name_string = |name: &ProviderName| -> Result<String, String> {
        serde_json::to_value(name)
            .map_err(|e| format!("serialize provider name failed: {e}"))
            .and_then(|v| {
                v.as_str()
                    .map(ToString::to_string)
                    .ok_or_else(|| format!("provider name is not a string: {v}"))
            })
    };

    let request = GenerateWorkItemsRequest {
        title: plan.id.clone(),
        story_spec_ids: plan.source_story_spec_ids.clone(),
        design_spec_ids: plan.source_design_spec_ids.clone(),
        include_integration_tests: Some(plan.options.include_integration_tests),
        include_e2e_tests: Some(plan.options.include_e2e_tests),
        force_frontend_backend_split: Some(plan.options.force_frontend_backend_split),
        require_execution_plan_confirm: Some(plan.options.require_execution_plan_confirm),
        author_provider: Some(provider_name_string(&session.author_provider)?),
        reviewer_provider: session
            .reviewer_provider
            .as_ref()
            .map(provider_name_string)
            .transpose()?,
        review_rounds: Some(session.review_rounds),
        superpowers_enabled: Some(session.superpowers_enabled),
        openspec_enabled: Some(session.openspec_enabled),
        revision_feedback: feedback.map(ToString::to_string),
    };

    Ok((retained, redo_specs, request))
}

impl WorkspaceEngine {
    /// AuthorConfirm 阶段标记/取消标记单个 WorkItem 的 revert。
    ///
    /// **不产生新 artifact_version**：改 `session.artifact` 与当前 is_current
    /// `ArtifactVersion.payload` 的 candidate meta，再推同 version 的 `EngineEvent::ArtifactUpdate`。
    pub async fn apply_revert_mark(
        &mut self,
        work_item_id: &str,
        feedback: Option<String>,
        clear: bool,
    ) -> Result<(), String> {
        let payload = self
            .session
            .artifact
            .clone()
            .ok_or("no artifact to mark revert on")?;
        let mut candidate = match payload {
            ArtifactPayload::WorkItemPlanCandidate { candidate } => candidate,
            _ => return Err("artifact is not a WorkItemPlanCandidate".into()),
        };
        let wi = candidate
            .work_items
            .iter_mut()
            .find(|w| w.id == work_item_id)
            .ok_or_else(|| format!("work_item {} not in candidate", work_item_id))?;
        if clear {
            wi.meta.reverted = false;
            wi.meta.revert_feedback = None;
        } else {
            wi.meta.reverted = true;
            wi.meta.revert_feedback = feedback;
        }

        // 更新 session.artifact + 当前 ArtifactVersion.payload（不 push artifact_versions，version 不变）
        let current_version = self
            .artifact_versions
            .iter()
            .rev()
            .find(|v| v.is_current)
            .map(|v| v.version)
            .ok_or_else(|| "no current artifact version to apply revert mark on".to_string())?;
        let payload = ArtifactPayload::WorkItemPlanCandidate {
            candidate: candidate.clone(),
        };
        self.session.artifact = Some(payload.clone());
        if let Some(version) = self
            .artifact_versions
            .iter_mut()
            .rev()
            .find(|v| v.is_current)
        {
            version.payload = payload.clone();
            self.persist_artifact_versions();
        }

        // 推同 version 的 ArtifactUpdate（前端据此刷新 candidate 展示）
        let _ = self
            .event_tx
            .send(EngineEvent::ArtifactUpdate {
                version: current_version,
                payload,
            })
            .await;
        Ok(())
    }

    async fn enter_human_confirm_for_work_item_plan_author_failure(
        &mut self,
        _findings: &[WorkItemSplitFinding],
    ) {
        self.transition_stage(WorkspaceStage::HumanConfirm).await;
        if let Some(store) = &self.lifecycle_store {
            let _ = store.update_workspace_session_status(
                &self.session.session_id,
                WorkspaceSessionStatus::WaitingForHuman,
            );
        }
        let _ = self
            .create_timeline_node(TimelineNodeDraft {
                node_type: TimelineNodeType::HumanConfirm,
                agent: None,
                stage: WorkspaceStage::HumanConfirm,
                round: None,
                title: "WorkItemPlan validate 连续失败".to_string(),
                summary: Some("author 多次重生仍 validate 失败，需人工介入".to_string()),
                status: TimelineNodeStatus::Active,
            })
            .await;
    }

    async fn enter_work_item_plan_context_blocker(&mut self, summary: Option<String>) {
        self.transition_stage(WorkspaceStage::HumanConfirm).await;
        if let Some(store) = &self.lifecycle_store {
            let _ = store.update_workspace_session_status(
                &self.session.session_id,
                WorkspaceSessionStatus::WaitingForHuman,
            );
        }
        let _ = self
            .create_timeline_node(TimelineNodeDraft {
                node_type: TimelineNodeType::WorkItemPlanContextBlocker,
                agent: None,
                stage: WorkspaceStage::HumanConfirm,
                round: None,
                title: "WorkItemPlan 上下文补充".to_string(),
                summary,
                status: TimelineNodeStatus::Active,
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

    pub async fn finish_active_run_with_failed_node(&mut self, message: impl Into<String>) {
        let message = message.into();
        if let Some(node_id) = self.active_node_id.clone() {
            self.update_timeline_node(&node_id, TimelineNodeStatus::Failed, Some(message))
                .await;
        }
        self.finish_failed_run().await;
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

        let mut timeline_node_details = HashMap::new();
        let mut timeline_node_summaries = HashMap::new();
        if let Some(store) = self.lifecycle_store.as_ref()
            && let Ok(ids) = store.list_node_detail_ids(&self.session.session_id)
        {
            let timeline_node_ids = self
                .timeline_nodes
                .iter()
                .map(|node| node.node_id.as_str())
                .collect::<HashSet<_>>();
            for id in ids {
                let Ok(detail) = store.load_node_detail(&self.session.session_id, &id) else {
                    continue;
                };
                timeline_node_summaries.insert(id.clone(), build_node_detail_summary(&detail));
                if self.session.workspace_type == WorkspaceType::WorkItemPlan
                    && timeline_node_ids.contains(id.as_str())
                {
                    timeline_node_details.insert(id, build_session_state_node_detail(detail));
                }
            }
        }
        let artifact_version_summaries = self
            .artifact_versions
            .iter()
            .map(build_artifact_version_summary)
            .collect();

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
            artifact_versions: Vec::new(),
            artifact_version_summaries,
            timeline_node_details,
            timeline_node_summaries,
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

    async fn enter_work_item_plan_outline_confirm(&mut self, summary: Option<String>) {
        self.transition_stage(WorkspaceStage::AuthorConfirm).await;
        let _ = self
            .create_timeline_node(TimelineNodeDraft {
                node_type: TimelineNodeType::WorkItemPlanOutlineConfirm,
                agent: None,
                stage: WorkspaceStage::AuthorConfirm,
                round: None,
                title: "WorkItemPlan Outline 确认".to_string(),
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

    async fn enter_work_item_generation_mode(&mut self, summary: Option<String>) {
        self.transition_stage(WorkspaceStage::AuthorConfirm).await;
        let _ = self
            .create_timeline_node(TimelineNodeDraft {
                node_type: TimelineNodeType::WorkItemGenerationMode,
                agent: None,
                stage: WorkspaceStage::AuthorConfirm,
                round: None,
                title: "Work Item 生成模式选择".to_string(),
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

    async fn enter_work_item_draft_confirm(&mut self, summary: Option<String>) {
        self.transition_stage(WorkspaceStage::AuthorConfirm).await;
        let _ = self
            .create_timeline_node(TimelineNodeDraft {
                node_type: TimelineNodeType::WorkItemDraftConfirm,
                agent: None,
                stage: WorkspaceStage::AuthorConfirm,
                round: None,
                title: "Work Item Draft 确认".to_string(),
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

    async fn enter_work_item_batch_confirm(&mut self, summary: Option<String>) {
        self.transition_stage(WorkspaceStage::AuthorConfirm).await;
        let _ = self
            .create_timeline_node(TimelineNodeDraft {
                node_type: TimelineNodeType::WorkItemBatchConfirm,
                agent: None,
                stage: WorkspaceStage::AuthorConfirm,
                round: None,
                title: "Work Item Batch 确认".to_string(),
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
                TimelineNodeType::AuthorRun
                | TimelineNodeType::WorkItemPlanOutlineRun
                | TimelineNodeType::WorkItemDraftRun
                | TimelineNodeType::WorkItemBatchRun => Some(AgentRole::Author),
                TimelineNodeType::ReviewerRun
                | TimelineNodeType::WorkItemPlanOutlineReview
                | TimelineNodeType::WorkItemDraftReview
                | TimelineNodeType::WorkItemBatchReview => Some(AgentRole::Reviewer),
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
            .filter(|node| {
                matches!(
                    node.node_type,
                    TimelineNodeType::ReviewerRun | TimelineNodeType::WorkItemPlanOutlineReview
                )
            })
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
        Self::parse_review_verdict_for_workspace(output, &WorkspaceType::Story)
    }

    fn parse_review_verdict_for_active_node(&self, output: &str) -> ReviewVerdict {
        if self.session.workspace_type == WorkspaceType::WorkItemPlan
            && self.active_node_type() == Some(TimelineNodeType::WorkItemPlanOutlineReview)
        {
            let valid_outline_ids = self.current_work_item_plan_outline_ids();
            let trimmed = output.trim();
            let parsed = extract_structured_json(trimmed).and_then(|(comments, json)| {
                parse_work_item_plan_review_json(
                    &json,
                    &comments,
                    &valid_outline_ids,
                    WorkItemPlanReviewScope::Outline,
                )
                .or_else(|| parse_review_json(&json, &comments))
            });
            return parsed.unwrap_or_else(|| ReviewVerdict {
                verdict: ReviewVerdictType::NeedsHuman,
                comments: output.to_string(),
                summary: "需要人工确认".to_string(),
                findings: Vec::new(),
                review_gate: ReviewGate::UserTriageRequired,
                work_item_plan_review: None,
            });
        }

        if self.session.workspace_type == WorkspaceType::WorkItemPlan
            && self.active_node_type() == Some(TimelineNodeType::WorkItemDraftReview)
        {
            let valid_outline_ids = self.current_work_item_plan_outline_ids();
            let trimmed = output.trim();
            let parsed = extract_structured_json(trimmed).and_then(|(comments, json)| {
                parse_work_item_plan_review_json(
                    &json,
                    &comments,
                    &valid_outline_ids,
                    WorkItemPlanReviewScope::Item,
                )
                .or_else(|| parse_review_json(&json, &comments))
            });
            return parsed.unwrap_or_else(|| ReviewVerdict {
                verdict: ReviewVerdictType::NeedsHuman,
                comments: output.to_string(),
                summary: "需要人工确认".to_string(),
                findings: Vec::new(),
                review_gate: ReviewGate::UserTriageRequired,
                work_item_plan_review: None,
            });
        }

        if self.session.workspace_type == WorkspaceType::WorkItemPlan
            && self.active_node_type() == Some(TimelineNodeType::WorkItemBatchReview)
        {
            let valid_outline_ids = self.current_work_item_plan_outline_ids();
            let trimmed = output.trim();
            let parsed = extract_structured_json(trimmed).and_then(|(comments, json)| {
                parse_work_item_plan_review_json(
                    &json,
                    &comments,
                    &valid_outline_ids,
                    WorkItemPlanReviewScope::Batch,
                )
                .or_else(|| parse_review_json(&json, &comments))
            });
            return parsed.unwrap_or_else(|| ReviewVerdict {
                verdict: ReviewVerdictType::NeedsHuman,
                comments: output.to_string(),
                summary: "需要人工确认".to_string(),
                findings: Vec::new(),
                review_gate: ReviewGate::UserTriageRequired,
                work_item_plan_review: None,
            });
        }

        Self::parse_review_verdict_for_workspace(output, &self.session.workspace_type)
    }

    fn parse_review_verdict_for_workspace(
        output: &str,
        workspace_type: &WorkspaceType,
    ) -> ReviewVerdict {
        let trimmed = output.trim();
        let parsed = extract_structured_json(trimmed).and_then(|(comments, json)| {
            if *workspace_type == WorkspaceType::WorkItemPlan {
                parse_work_item_plan_review_json(
                    &json,
                    &comments,
                    &[],
                    WorkItemPlanReviewScope::Batch,
                )
                .or_else(|| parse_review_json(&json, &comments))
            } else {
                parse_review_json(&json, &comments)
            }
        });

        parsed.unwrap_or_else(|| ReviewVerdict {
            verdict: ReviewVerdictType::NeedsHuman,
            comments: output.to_string(),
            summary: "需要人工确认".to_string(),
            findings: Vec::new(),
            review_gate: ReviewGate::UserTriageRequired,
            work_item_plan_review: None,
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

fn recover_pending_author_choice(
    session: &WorkspaceSession,
    active_node_id: Option<&str>,
    timeline_nodes: &[TimelineNode],
) -> Option<PendingAuthorChoice> {
    let active_node_id = active_node_id?;
    let active_node = timeline_nodes
        .iter()
        .find(|node| node.node_id == active_node_id)?;
    if active_node.node_type != TimelineNodeType::AuthorRun
        || active_node.status != TimelineNodeStatus::Paused
        || !active_node
            .summary
            .as_deref()
            .is_some_and(|summary| summary.contains("等待用户选择"))
    {
        return None;
    }

    let assistant_message = session
        .messages
        .iter()
        .rev()
        .find(|message| message.role == "assistant")?;
    let (prompt, options) =
        detect_author_choice_request(&assistant_message.content, &session.workspace_type)?;
    Some(PendingAuthorChoice {
        id: format!("author_choice_{}", assistant_message.id),
        prompt,
        options,
        source_node_id: Some(active_node_id.to_string()),
    })
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

fn review_complete_event_from_verdict(
    node_id: String,
    round: u32,
    verdict: &ReviewVerdict,
) -> EngineEvent {
    EngineEvent::ReviewComplete {
        node_id,
        round,
        verdict: verdict.verdict.clone(),
        comments: verdict.comments.clone(),
        summary: verdict.summary.clone(),
        findings: verdict.findings.clone(),
        review_gate: verdict.review_gate.clone(),
        work_item_plan_review: verdict.work_item_plan_review.clone(),
    }
}

const STRUCTURED_OUTPUT_START_PREFIX: &str = "<ARIA_STRUCTURED_OUTPUT";
const STRUCTURED_OUTPUT_END_PREFIX: &str = "</ARIA_STRUCTURED_OUTPUT";

fn extract_structured_json(output: &str) -> Option<(String, String)> {
    extract_nonce_sentinel_json(output).or_else(|| extract_markdown_fence_json(output))
}

fn extract_nonce_sentinel_json(output: &str) -> Option<(String, String)> {
    let mut search_end = output.len();
    while let Some(start) = output[..search_end].rfind(STRUCTURED_OUTPUT_START_PREFIX) {
        let after_start_prefix = &output[start + STRUCTURED_OUTPUT_START_PREFIX.len()..];
        let Some((Some(start_nonce), start_tag_len)) =
            parse_structured_output_tag(after_start_prefix)
        else {
            search_end = start;
            continue;
        };
        let json_start = start + STRUCTURED_OUTPUT_START_PREFIX.len() + start_tag_len;
        let after_start = &output[json_start..];
        let Some(end) = after_start.find(STRUCTURED_OUTPUT_END_PREFIX) else {
            search_end = start;
            continue;
        };
        let after_end_prefix = &after_start[end + STRUCTURED_OUTPUT_END_PREFIX.len()..];
        let Some((end_nonce, _end_tag_len)) = parse_structured_output_tag(after_end_prefix) else {
            search_end = start;
            continue;
        };
        if end_nonce.as_deref() != Some(start_nonce.as_str()) {
            search_end = start;
            continue;
        }
        return Some((
            output[..start].to_string(),
            after_start[..end].trim().to_string(),
        ));
    }
    None
}

fn parse_structured_output_tag(after_prefix: &str) -> Option<(Option<String>, usize)> {
    let end_offset = after_prefix.find('>')?;
    let attrs = after_prefix[..end_offset].trim();
    let nonce = parse_structured_output_nonce(attrs)?;
    Some((nonce, end_offset + 1))
}

fn parse_structured_output_nonce(attrs: &str) -> Option<Option<String>> {
    if attrs.is_empty() {
        return Some(None);
    }
    let nonce = attrs
        .strip_prefix("nonce=\"")
        .and_then(|value| value.strip_suffix('"'))?;
    if nonce.len() != 8 || !nonce.chars().all(|ch| ch.is_ascii_alphanumeric()) {
        return None;
    }
    Some(Some(nonce.to_string()))
}

fn extract_markdown_fence_json(output: &str) -> Option<(String, String)> {
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

fn parse_review_json(json: &str, comments: &str) -> Option<ReviewVerdict> {
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    let parsed_verdict = match value.get("verdict")?.as_str()? {
        "pass" => ReviewVerdictType::Pass,
        "revise" => ReviewVerdictType::Revise,
        "needs_human" => ReviewVerdictType::NeedsHuman,
        _ => return None,
    };
    let summary = value
        .get("summary")
        .and_then(|value| value.as_str())
        .unwrap_or(match parsed_verdict {
            ReviewVerdictType::Pass => "审核通过",
            ReviewVerdictType::Revise => "需要返修",
            ReviewVerdictType::NeedsHuman => "需要人工确认",
        })
        .to_string();
    let parsed_findings = parse_review_findings(value.get("findings"));
    let review_gate = review_gate_for(&parsed_verdict, &parsed_findings);
    let verdict = match review_gate {
        ReviewGate::RequiresRevision => ReviewVerdictType::Revise,
        ReviewGate::UserConfirmAllowed => match parsed_verdict {
            ReviewVerdictType::Pass => ReviewVerdictType::Pass,
            ReviewVerdictType::Revise | ReviewVerdictType::NeedsHuman => {
                ReviewVerdictType::NeedsHuman
            }
        },
        ReviewGate::UserTriageRequired => ReviewVerdictType::NeedsHuman,
    };
    Some(ReviewVerdict {
        verdict,
        comments: comments.trim().to_string(),
        summary,
        findings: parsed_findings.findings,
        review_gate,
        work_item_plan_review: None,
    })
}

fn parse_work_item_plan_review_json(
    json: &str,
    comments: &str,
    valid_outline_ids: &[String],
    scope: WorkItemPlanReviewScope,
) -> Option<ReviewVerdict> {
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    let parsed_verdict = parse_work_item_plan_review_verdict(value.get("verdict")?.as_str()?);
    let summary = value
        .get("summary")
        .and_then(|value| value.as_str())
        .unwrap_or(match parsed_verdict {
            WorkItemPlanReviewVerdict::Pass => "审核通过",
            WorkItemPlanReviewVerdict::Revise => "需要返修当前 Work Item",
            WorkItemPlanReviewVerdict::ReviseBatch => "需要重写当前 Batch",
            WorkItemPlanReviewVerdict::NeedsHuman => "需要人工确认",
            WorkItemPlanReviewVerdict::PlanReopenRequired => "需要重开 Outline",
        })
        .to_string();
    let target_outline_id = value
        .get("target_outline_id")
        .and_then(|value| value.as_str())
        .map(ToString::to_string);
    if target_outline_id
        .as_ref()
        .is_some_and(|id| !valid_outline_ids.iter().any(|valid| valid == id))
    {
        return Some(work_item_plan_review_invalid_reference(comments));
    }

    let (affects_items, warnings, total_affects, invalid_affects) =
        parse_work_item_plan_review_affects_items(value.get("affects_items"), valid_outline_ids);
    if total_affects > 0 && invalid_affects * 2 > total_affects {
        return Some(work_item_plan_review_invalid_reference(comments));
    }

    let parsed_findings = parse_review_findings(value.get("findings"));
    let generation_round_id = value
        .get("generation_round_id")
        .and_then(|value| value.as_str())
        .unwrap_or("generation_round_unknown")
        .to_string();
    let draft_id = value
        .get("draft_id")
        .and_then(|value| value.as_str())
        .map(ToString::to_string);
    let batch_id = value
        .get("batch_id")
        .and_then(|value| value.as_str())
        .map(ToString::to_string);
    let (generic_verdict, review_gate, review_action, gates) =
        work_item_plan_review_routing(&parsed_verdict, &scope);
    let extension = WorkItemPlanReviewComplete {
        verdict: parsed_verdict,
        review_scope: scope,
        target_outline_id,
        generation_round_id,
        draft_id,
        batch_id,
        review_action,
        gates,
        affects_items,
        warnings,
    };

    Some(ReviewVerdict {
        verdict: generic_verdict,
        comments: comments.trim().to_string(),
        summary,
        findings: parsed_findings.findings,
        review_gate,
        work_item_plan_review: Some(extension),
    })
}

fn parse_work_item_plan_review_verdict(value: &str) -> WorkItemPlanReviewVerdict {
    match value {
        "pass" => WorkItemPlanReviewVerdict::Pass,
        "revise" => WorkItemPlanReviewVerdict::Revise,
        "revise_batch" => WorkItemPlanReviewVerdict::ReviseBatch,
        "needs_human" => WorkItemPlanReviewVerdict::NeedsHuman,
        "plan_reopen_required" => WorkItemPlanReviewVerdict::PlanReopenRequired,
        _ => WorkItemPlanReviewVerdict::NeedsHuman,
    }
}

fn work_item_plan_review_routing(
    verdict: &WorkItemPlanReviewVerdict,
    scope: &WorkItemPlanReviewScope,
) -> (
    ReviewVerdictType,
    ReviewGate,
    WorkItemPlanReviewAction,
    Vec<WorkItemPlanReviewGate>,
) {
    match verdict {
        WorkItemPlanReviewVerdict::Pass => (
            ReviewVerdictType::Pass,
            ReviewGate::UserConfirmAllowed,
            WorkItemPlanReviewAction::Continue,
            Vec::new(),
        ),
        WorkItemPlanReviewVerdict::Revise => {
            if scope == &WorkItemPlanReviewScope::Outline {
                (
                    ReviewVerdictType::Revise,
                    ReviewGate::RequiresRevision,
                    WorkItemPlanReviewAction::ReviseOutline,
                    vec![WorkItemPlanReviewGate::RequiresPlanReopen],
                )
            } else {
                (
                    ReviewVerdictType::Revise,
                    ReviewGate::RequiresRevision,
                    WorkItemPlanReviewAction::ReviseCurrentItem,
                    vec![WorkItemPlanReviewGate::RequiresCurrentItemRevision],
                )
            }
        }
        WorkItemPlanReviewVerdict::ReviseBatch => (
            ReviewVerdictType::NeedsHuman,
            ReviewGate::UserTriageRequired,
            WorkItemPlanReviewAction::ReviseBatch,
            vec![WorkItemPlanReviewGate::RequiresBatchRevision],
        ),
        WorkItemPlanReviewVerdict::NeedsHuman => (
            ReviewVerdictType::NeedsHuman,
            ReviewGate::UserTriageRequired,
            WorkItemPlanReviewAction::HumanTriage,
            Vec::new(),
        ),
        WorkItemPlanReviewVerdict::PlanReopenRequired => (
            ReviewVerdictType::NeedsHuman,
            ReviewGate::UserTriageRequired,
            WorkItemPlanReviewAction::ReviseOutline,
            vec![WorkItemPlanReviewGate::RequiresPlanReopen],
        ),
    }
}

fn parse_work_item_plan_review_affects_items(
    value: Option<&serde_json::Value>,
    valid_outline_ids: &[String],
) -> (
    Vec<WorkItemPlanReviewAffectedItem>,
    Vec<String>,
    usize,
    usize,
) {
    let Some(items) = value.and_then(|value| value.as_array()) else {
        return (Vec::new(), Vec::new(), 0, 0);
    };

    let mut valid_items = Vec::new();
    let mut warnings = Vec::new();
    let mut invalid_count = 0;
    for item in items {
        let outline_index = item
            .get("outline_index")
            .and_then(|value| value.as_u64())
            .and_then(|value| u32::try_from(value).ok());
        let target_outline_id = item
            .get("target_outline_id")
            .and_then(|value| value.as_str())
            .map(ToString::to_string);
        let index_valid = outline_index.is_none_or(|index| {
            usize::try_from(index)
                .ok()
                .is_some_and(|index| index < valid_outline_ids.len())
        });
        let target_valid = target_outline_id
            .as_ref()
            .is_some_and(|id| valid_outline_ids.iter().any(|valid| valid == id));
        let valid = index_valid && (target_outline_id.is_none() || target_valid);

        if valid && (outline_index.is_some() || target_outline_id.is_some()) {
            valid_items.push(WorkItemPlanReviewAffectedItem {
                outline_index,
                target_outline_id,
            });
        } else {
            invalid_count += 1;
            warnings.push(format!(
                "invalid_reference: target_outline_id={} not found",
                target_outline_id.as_deref().unwrap_or("<missing>")
            ));
        }
    }

    (valid_items, warnings, items.len(), invalid_count)
}

fn work_item_plan_review_invalid_reference(comments: &str) -> ReviewVerdict {
    ReviewVerdict {
        verdict: ReviewVerdictType::NeedsHuman,
        comments: comments.trim().to_string(),
        summary: "WorkItemPlan reviewer 引用无效，需要人工确认".to_string(),
        findings: Vec::new(),
        review_gate: ReviewGate::UserTriageRequired,
        work_item_plan_review: None,
    }
}

struct ParsedReviewFindings {
    findings: Vec<ReviewFinding>,
    malformed: bool,
}

fn parse_review_findings(value: Option<&serde_json::Value>) -> ParsedReviewFindings {
    let Some(value) = value else {
        return ParsedReviewFindings {
            findings: Vec::new(),
            malformed: false,
        };
    };
    let Some(items) = value.as_array() else {
        return ParsedReviewFindings {
            findings: Vec::new(),
            malformed: true,
        };
    };

    let mut findings = Vec::new();
    let mut malformed = false;
    for item in items {
        let Some(severity) = item
            .get("severity")
            .and_then(|value| value.as_str())
            .and_then(parse_review_finding_severity)
        else {
            malformed = true;
            continue;
        };
        let Some(message) = item.get("message").and_then(|value| value.as_str()) else {
            malformed = true;
            continue;
        };

        findings.push(ReviewFinding {
            severity,
            message: message.to_string(),
            evidence: item
                .get("evidence")
                .and_then(|value| value.as_str())
                .unwrap_or("")
                .to_string(),
            impact: item
                .get("impact")
                .and_then(|value| value.as_str())
                .unwrap_or("")
                .to_string(),
            required_action: item
                .get("required_action")
                .and_then(|value| value.as_str())
                .unwrap_or("")
                .to_string(),
        });
    }

    ParsedReviewFindings {
        findings,
        malformed,
    }
}

fn parse_review_finding_severity(value: &str) -> Option<ReviewFindingSeverity> {
    match value {
        "blocking" => Some(ReviewFindingSeverity::Blocking),
        "must_fix" => Some(ReviewFindingSeverity::MustFix),
        "strong_recommend_fix" => Some(ReviewFindingSeverity::StrongRecommendFix),
        "suggestion" => Some(ReviewFindingSeverity::Suggestion),
        "minor" => Some(ReviewFindingSeverity::Minor),
        "optional" => Some(ReviewFindingSeverity::Optional),
        _ => None,
    }
}

fn review_gate_for(
    verdict: &ReviewVerdictType,
    parsed_findings: &ParsedReviewFindings,
) -> ReviewGate {
    if parsed_findings.findings.iter().any(|finding| {
        matches!(
            finding.severity,
            ReviewFindingSeverity::Blocking
                | ReviewFindingSeverity::MustFix
                | ReviewFindingSeverity::StrongRecommendFix
        )
    }) {
        return ReviewGate::RequiresRevision;
    }
    if parsed_findings.malformed {
        return ReviewGate::UserTriageRequired;
    }

    match verdict {
        ReviewVerdictType::Pass => ReviewGate::UserConfirmAllowed,
        ReviewVerdictType::NeedsHuman => ReviewGate::UserTriageRequired,
        ReviewVerdictType::Revise if parsed_findings.findings.is_empty() => {
            ReviewGate::UserTriageRequired
        }
        ReviewVerdictType::Revise => ReviewGate::UserConfirmAllowed,
    }
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

fn empty_design_context_capabilities() -> DesignContextCapabilities {
    DesignContextCapabilities {
        has_architecture: false,
        has_module_breakdown: false,
        has_tech_stack: false,
        has_test_strategy: false,
        has_key_paths: false,
    }
}

fn estimate_context_resolution_tokens(value: &str) -> u32 {
    ((value.chars().count() as u32).saturating_add(3) / 4).max(1)
}

fn format_context_blocker_resolution_markdown(resolution: &str) -> String {
    format!(
        "# WorkItemPlan 上下文补充\n\n## 用户补充\n\n{resolution}\n",
        resolution = resolution.trim()
    )
}

fn workspace_type_title(workspace_type: &WorkspaceType) -> &'static str {
    match workspace_type {
        WorkspaceType::Story => "Story Spec",
        WorkspaceType::Design => "Design Spec",
        WorkspaceType::WorkItem => "Work Item",
        WorkspaceType::WorkItemPlan => "Work Item Plan",
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

    if let Some(choice) = detect_explicit_choice_request(content) {
        return Some(choice);
    }

    detect_recommendation_choice_request(content)
}

fn detect_explicit_choice_request(content: &str) -> Option<(String, Vec<ChoiceOptionData>)> {
    let lines = content.lines().collect::<Vec<_>>();
    let mut candidates = Vec::new();
    let mut index = 0;

    while index < lines.len() {
        let trimmed = lines[index].trim();
        let Some(first_option) = parse_choice_option_line(trimmed) else {
            index += 1;
            continue;
        };

        let option_start = index;
        let mut options = vec![first_option];
        index += 1;
        while index < lines.len() {
            let trimmed = lines[index].trim();
            if trimmed.is_empty() {
                break;
            }
            let Some(option) = parse_choice_option_line(trimmed) else {
                break;
            };
            options.push(option);
            index += 1;
        }

        if options.len() >= 2 {
            candidates.push((choice_prompt_before_options(&lines, option_start), options));
        }
    }

    candidates.into_iter().last()
}

fn choice_prompt_before_options(lines: &[&str], option_start: usize) -> String {
    let Some((prompt_start, prompt_lines)) = previous_non_empty_block(lines, option_start) else {
        return default_choice_prompt();
    };

    let mut prompt_parts = Vec::new();
    if let Some((_, heading_lines)) = previous_non_empty_block(lines, prompt_start)
        && heading_lines.len() == 1
        && looks_like_choice_question_heading(&heading_lines[0])
    {
        prompt_parts.extend(heading_lines);
    }
    prompt_parts.extend(prompt_lines);

    let prompt = prompt_parts.join("\n");
    let trimmed = prompt.trim();
    if trimmed.is_empty() {
        default_choice_prompt()
    } else {
        trimmed.to_string()
    }
}

fn previous_non_empty_block(lines: &[&str], before: usize) -> Option<(usize, Vec<String>)> {
    if before == 0 {
        return None;
    }

    let mut index = before;
    loop {
        index -= 1;
        if !lines[index].trim().is_empty() {
            break;
        }
        if index == 0 {
            return None;
        }
    }

    let end = index + 1;
    while index > 0 && !lines[index - 1].trim().is_empty() {
        index -= 1;
    }

    Some((
        index,
        lines[index..end]
            .iter()
            .map(|line| line.trim().to_string())
            .collect(),
    ))
}

fn looks_like_choice_question_heading(line: &str) -> bool {
    let normalized = line.trim().trim_matches('*').trim_matches('_').trim();
    (normalized.starts_with("问题") || normalized.starts_with("Question"))
        && (normalized.contains('：') || normalized.contains(':'))
}

fn default_choice_prompt() -> String {
    "请选择下一步处理方式。".to_string()
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
        WorkspaceType::WorkItem | WorkspaceType::WorkItemPlan => false,
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

fn structured_output_nonce() -> String {
    uuid::Uuid::new_v4()
        .simple()
        .to_string()
        .chars()
        .take(8)
        .collect()
}

fn reviewer_output_contract(nonce: &str, schema: &str, intro: &str) -> String {
    format!(
        "{intro}\
         <ARIA_STRUCTURED_OUTPUT nonce=\"{nonce}\">\n\
         {schema}\n\
         </ARIA_STRUCTURED_OUTPUT nonce=\"{nonce}\">\n"
    )
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

fn upsert_execution_event_json(events: &mut Vec<serde_json::Value>, event: serde_json::Value) {
    let Some(event_id) = event
        .get("event_id")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
    else {
        events.push(event);
        return;
    };

    if let Some(existing) = events.iter_mut().find(|existing| {
        existing.get("event_id").and_then(serde_json::Value::as_str) == Some(event_id.as_str())
    }) {
        *existing = event;
        return;
    }

    events.push(event);
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

fn latest_artifact_from_messages(messages: &[WorkspaceMessageRecord]) -> Option<ArtifactPayload> {
    messages
        .iter()
        .rev()
        .find(|message| matches!(message.role.as_str(), "assistant" | "provider"))
        .map(|message| ArtifactPayload::Markdown {
            markdown: extract_artifact_content(&message.content),
            diff: None,
        })
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
    use crate::product::lifecycle_store::{
        CreateDesignSpecInput, CreateIssueWorkItemPlanInput, CreateRepositoryProfileInput,
        CreateStorySpecInput, CreateVerificationPlanInput, CreateWorkItemInput,
        CreateWorkspaceSessionInput, IssueWorkItemPlanUpdate,
    };
    use crate::product::models::{
        AgentRole, ArtifactRef, IssueWorkItemDependencyEdge, IssueWorkItemPlan,
        IssueWorkItemPlanOptions, IssueWorkItemPlanStatus, NodeDetail, PermissionEvent,
        ProviderSnapshot, RepositoryProfileConfidence, VerificationCommand,
        VerificationCommandSafety, VerificationCommandSource, VerificationFallbackPolicy,
        VerificationManualCheck, VerificationScope, WorkItemContextBudget, WorkItemKind,
        WorkItemOutline, WorkItemOutlineDependencyEdge, WorkItemPlanOutline, WorkItemPlanStatus,
        WorkItemSplitFinding, WorkItemSplitFindingSeverity, WorkspaceMessageRecord,
    };
    use crate::protocol::contracts::{AdapterInput, ProviderType};
    use crate::web::workspace_ws_types::{
        ArtifactPayload, AuthorDecision, ProviderConfigSnapshot, ReviewFindingSeverity, ReviewGate,
        ReviewVerdictType, TimelineNode, TimelineNodeStatus, TimelineNodeType,
        WorkItemCandidateDto, WorkItemCandidateMetaDto, WorkItemPlanCandidateDto, WorkItemPlanDto,
        WorkItemPlanReviewAction, WorkItemPlanReviewGate, WorkItemPlanReviewScope,
        WorkItemPlanReviewVerdict, WorkItemSplitOptionsDto, WorkspaceStage as WsWorkspaceStage,
    };
    use std::sync::Mutex;
    use tempfile::TempDir;

    fn setup() -> (TempDir, Arc<CheckpointStore>) {
        let tmp = TempDir::new().unwrap();
        let store = Arc::new(CheckpointStore::new(tmp.path().to_path_buf()));
        (tmp, store)
    }

    fn artifact_payload(markdown: &str) -> ArtifactPayload {
        ArtifactPayload::Markdown {
            markdown: markdown.to_string(),
            diff: None,
        }
    }

    fn test_work_item_plan_outline(
        dependency_graph: Vec<WorkItemOutlineDependencyEdge>,
    ) -> WorkItemPlanOutline {
        WorkItemPlanOutline {
            id: "outline_001".to_string(),
            project_id: "project_001".to_string(),
            issue_id: "issue_001".to_string(),
            source_story_spec_ids: vec!["story_001".to_string()],
            source_design_spec_ids: vec!["design_001".to_string()],
            strategy_summary: "test strategy".to_string(),
            work_item_outlines: vec![
                WorkItemOutline {
                    outline_id: "outline_a".to_string(),
                    title: "A".to_string(),
                    kind: WorkItemKind::Backend,
                    goal: "A".to_string(),
                    scope: vec!["src/a.rs".to_string()],
                    non_goals: Vec::new(),
                    source_story_spec_ids: vec!["story_001".to_string()],
                    source_design_spec_ids: vec!["design_001".to_string()],
                    exclusive_write_scopes: vec!["src/a.rs".to_string()],
                    forbidden_write_scopes: Vec::new(),
                    depends_on: Vec::new(),
                    verification_intent: vec!["cargo test --locked --lib a".to_string()],
                    handoff_notes: "handoff A".to_string(),
                },
                WorkItemOutline {
                    outline_id: "outline_b".to_string(),
                    title: "B".to_string(),
                    kind: WorkItemKind::Frontend,
                    goal: "B".to_string(),
                    scope: vec!["web/b.ts".to_string()],
                    non_goals: Vec::new(),
                    source_story_spec_ids: vec!["story_001".to_string()],
                    source_design_spec_ids: vec!["design_001".to_string()],
                    exclusive_write_scopes: vec!["web/b.ts".to_string()],
                    forbidden_write_scopes: Vec::new(),
                    depends_on: Vec::new(),
                    verification_intent: vec!["pnpm -C web test".to_string()],
                    handoff_notes: "handoff B".to_string(),
                },
                WorkItemOutline {
                    outline_id: "outline_c".to_string(),
                    title: "C".to_string(),
                    kind: WorkItemKind::Integration,
                    goal: "C".to_string(),
                    scope: vec!["tests/c.rs".to_string()],
                    non_goals: Vec::new(),
                    source_story_spec_ids: vec!["story_001".to_string()],
                    source_design_spec_ids: vec!["design_001".to_string()],
                    exclusive_write_scopes: vec!["tests/c.rs".to_string()],
                    forbidden_write_scopes: Vec::new(),
                    depends_on: Vec::new(),
                    verification_intent: vec!["cargo test --locked --test c".to_string()],
                    handoff_notes: "handoff C".to_string(),
                },
            ],
            dependency_graph,
            risks: Vec::new(),
            handoff_strategy: "handoff".to_string(),
            status: "draft".to_string(),
        }
    }

    #[test]
    fn work_item_plan_outline_topological_order_keeps_original_order_for_ready_items() {
        let outline = test_work_item_plan_outline(vec![
            WorkItemOutlineDependencyEdge {
                from_outline_id: "outline_a".to_string(),
                to_outline_id: "outline_c".to_string(),
            },
            WorkItemOutlineDependencyEdge {
                from_outline_id: "outline_b".to_string(),
                to_outline_id: "outline_c".to_string(),
            },
        ]);

        let order = work_item_plan_outline_topological_order(&outline).expect("topological order");

        assert_eq!(
            order,
            vec![
                "outline_a".to_string(),
                "outline_b".to_string(),
                "outline_c".to_string()
            ]
        );
    }

    #[test]
    fn work_item_plan_outline_topological_order_rejects_cycles() {
        let outline = test_work_item_plan_outline(vec![
            WorkItemOutlineDependencyEdge {
                from_outline_id: "outline_a".to_string(),
                to_outline_id: "outline_b".to_string(),
            },
            WorkItemOutlineDependencyEdge {
                from_outline_id: "outline_b".to_string(),
                to_outline_id: "outline_a".to_string(),
            },
        ]);

        let error = work_item_plan_outline_topological_order(&outline).expect_err("cycle rejected");

        assert!(error.contains("cycle"));
    }

    #[test]
    fn build_artifact_version_summary_derives_size_for_markdown_and_candidate() {
        let markdown_version = ArtifactVersion {
            version: 1,
            payload: ArtifactPayload::Markdown {
                markdown: "hello".to_string(),
                diff: None,
            },
            generated_by: ProviderName::ClaudeCode,
            reviewed_by: None,
            review_verdict: None,
            confirmed_by: None,
            is_current: true,
            created_at: "2026-06-01T00:00:00Z".to_string(),
            source_node_id: "node_001".to_string(),
        };
        let summary = build_artifact_version_summary(&markdown_version);
        assert_eq!(summary.markdown_size, 5);
        assert_eq!(summary.markdown_preview, "hello");

        let candidate = WorkItemPlanCandidateDto {
            plan: WorkItemPlanDto {
                id: "plan_001".to_string(),
                status: "draft".to_string(),
                options: WorkItemSplitOptionsDto {
                    include_integration_tests: false,
                    include_e2e_tests: false,
                    force_frontend_backend_split: false,
                    require_execution_plan_confirm: false,
                },
                dependency_graph: vec![],
            },
            work_items: vec![WorkItemCandidateDto {
                id: "wi_001".to_string(),
                kind: "backend".to_string(),
                title: "first work item".to_string(),
                depends_on: vec![],
                exclusive_write_scopes: vec![],
                verification_plan_ref: None,
                meta: WorkItemCandidateMetaDto {
                    reverted: false,
                    revert_feedback: None,
                },
            }],
            verification_plans: vec![],
            repository_profile: None,
            validator_findings: vec![],
        };
        let candidate_version = ArtifactVersion {
            version: 2,
            payload: ArtifactPayload::WorkItemPlanCandidate {
                candidate: Box::new(candidate.clone()),
            },
            generated_by: ProviderName::Codex,
            reviewed_by: None,
            review_verdict: None,
            confirmed_by: None,
            is_current: false,
            created_at: "2026-06-01T00:00:01Z".to_string(),
            source_node_id: "node_002".to_string(),
        };
        let summary = build_artifact_version_summary(&candidate_version);
        assert_eq!(
            summary.markdown_size,
            serde_json::to_string(&candidate).unwrap().len()
        );
        assert!(
            summary.markdown_preview.contains("first work item")
                || summary.markdown_preview.contains("plan_001"),
            "candidate preview should contain title or plan id: {}",
            summary.markdown_preview
        );
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
            .drive_provider_session(ProviderSessionDriveInput {
                session: Ok(text_choice_provider_session(
                    "climb_stairs(n) 对 n <= 0 应该如何处理？\nA. 返回 0\nB. 抛出 ValueError\n",
                )),
                command_rx: empty_provider_commands(),
                node_id: Some("timeline_node_author".to_string()),
                agent: Some(ProviderName::ClaudeCode),
                role: ProviderConversationRole::Author,
                artifact_retry: None,
                revision_resume_fallback: None,
            })
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

    #[tokio::test]
    async fn persistent_engine_recovers_pending_text_fallback_choice_after_restart() {
        let (_tmp, checkpoint_store) = setup();
        let app_root = tempfile::tempdir().expect("app root");
        let app_paths = ProductAppPaths::new(app_root.path().join(".aria"));
        let lifecycle_store = LifecycleStore::new(app_paths.clone());
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
            .expect("workspace session");
        lifecycle_store
            .replace_workspace_messages(
                &session_record.id,
                vec![
                    WorkspaceMessageRecord {
                        role: "system".to_string(),
                        content: "context".to_string(),
                        created_at: "2026-06-05T00:00:00Z".to_string(),
                    },
                    WorkspaceMessageRecord {
                        role: "user".to_string(),
                        content: "开始生成".to_string(),
                        created_at: "2026-06-05T00:00:01Z".to_string(),
                    },
                    WorkspaceMessageRecord {
                        role: "assistant".to_string(),
                        content: "我先说明一下当前判断。\n\n首次启动检测到缺失 Claude Code/Codex 时，Aria 应采用哪种安装策略？\n\n1. `确认后安装`\n2. `自动静默安装`\n3. `只检查不安装`".to_string(),
                        created_at: "2026-06-05T00:00:02Z".to_string(),
                    },
                ],
            )
            .expect("replace messages");
        let provider_config_snapshot = ProviderConfigSnapshot {
            author: ProviderName::Codex,
            reviewer: Some(ProviderName::ClaudeCode),
            review_rounds: 1,
        };
        lifecycle_store
            .save_timeline_nodes(
                &session_record.id,
                &[TimelineNode {
                    node_id: "timeline_node_002".to_string(),
                    node_type: TimelineNodeType::AuthorRun,
                    agent: Some(ProviderName::Codex),
                    stage: WsWorkspaceStage::Running,
                    round: None,
                    status: TimelineNodeStatus::Paused,
                    title: "Story Spec 生成".to_string(),
                    summary: Some("等待用户选择".to_string()),
                    started_at: "2026-06-05T00:00:02Z".to_string(),
                    completed_at: None,
                    duration_ms: None,
                    artifact_ref: None,
                    provider_config_snapshot,
                }],
            )
            .expect("replace timeline nodes");

        let session = WorkspaceSession::from_record(
            lifecycle_store
                .get_workspace_session(&session_record.id)
                .expect("reload session"),
        );
        let (tx, _rx) = mpsc::channel(8);
        let mut engine =
            WorkspaceEngine::new_persistent(checkpoint_store, lifecycle_store, tx, session);

        let prompt = engine
            .take_pending_author_choice_prompt("author_choice_msg_003", vec!["1".to_string()], None)
            .await
            .expect("pending choice should be recovered from persisted assistant text");

        assert!(prompt.contains("用户回答了 author 的确认问题"));
        assert!(prompt.contains("首次启动检测到缺失 Claude Code/Codex"));
        assert!(prompt.contains("1. `确认后安装`"));
        assert!(!prompt.contains("我先说明一下当前判断"));
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
        session.artifact = Some(artifact_payload(
            "# Story Spec\n\n## 功能需求\n- [REQ-001] Draft.\n",
        ));
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
    fn workspace_provider_inputs_use_three_hour_timeout() {
        let (event_tx, _event_rx) = mpsc::channel(8);
        let mut session = make_session("sess_workspace_timeout");
        session.artifact = Some(artifact_payload(
            "# Story Spec\n\n## 功能需求\n- [REQ-001] Draft.\n",
        ));
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
            findings: Vec::new(),
            review_gate: ReviewGate::RequiresRevision,
            work_item_plan_review: None,
        });

        assert_eq!(
            engine
                .build_streaming_input("开始生成", AuthorPromptMode::FullConversation)
                .expect("author input")
                .timeout_secs,
            10_800
        );
        assert_eq!(
            engine
                .build_review_input()
                .expect("review input")
                .timeout_secs,
            10_800
        );
        assert_eq!(
            engine
                .build_revision_input()
                .expect("revision input")
                .timeout_secs,
            10_800
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
        session.artifact = Some(artifact_payload(
            "# Current Story Spec\n\n## 功能需求\n- [REQ-001] 当前稿。\n\n## 成功标准\n- [AC-001] 当前稿覆盖 n=10 -> 89。\n",
        ));
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

    #[test]
    fn review_input_marks_design_artifact_as_extracted_markdown_without_outer_fence() {
        let (event_tx, _event_rx) = mpsc::channel(8);
        let mut session = make_session("sess_design_review_prompt_extracted_artifact");
        session.workspace_type = WorkspaceType::Design;
        session.artifact = Some(artifact_payload(
            "# 底层依赖安装任务 Design Spec\n\n\
             ## 设计范围\n\n\
             - [DEC-001] 覆盖依赖安装任务。\n\n\
             ## API 契约\n\n\
             ```json\n\
             {\"task_id\":\"install_001\"}\n\
             ```\n\n\
             ## 风险\n\n\
             - 无。\n",
        ));
        let checkpoint_tmp = TempDir::new().unwrap();
        let engine = WorkspaceEngine::new(
            Arc::new(CheckpointStore::new(checkpoint_tmp.path().to_path_buf())),
            event_tx,
            session,
        );

        let input = engine.build_review_input().expect("review input");

        assert!(
            input
                .prompt
                .contains("当前已提取 Artifact Markdown（daemon 已剥离外层 artifact fence）"),
            "review prompt should label stored artifact as extracted markdown: {}",
            input.prompt
        );
        assert!(input.prompt.contains("# 底层依赖安装任务 Design Spec"));
        assert!(
            input
                .prompt
                .contains("不要因为当前 Artifact 未包含外层 artifact fence 判定返修"),
            "reviewer should not reject extracted artifact for missing outer fence: {}",
            input.prompt
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
            .drive_provider_session(ProviderSessionDriveInput {
                session: Ok(ProviderSession {
                    events: provider_event_rx,
                    commands: provider_command_tx,
                }),
                command_rx: empty_provider_commands(),
                node_id: Some(node_id.clone()),
                agent: Some(ProviderName::ClaudeCode),
                role: ProviderConversationRole::Author,
                artifact_retry: None,
                revision_resume_fallback: None,
            })
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

        assert_eq!(verdict.verdict, ReviewVerdictType::NeedsHuman);
        assert_eq!(verdict.review_gate, ReviewGate::UserTriageRequired);
        assert_eq!(verdict.summary, "补充异常路径");
        assert_eq!(verdict.comments.trim(), "整体可用，但需要补充异常路径。");
    }

    #[test]
    fn reviewer_prompt_requires_nonce_sentinel() {
        let (event_tx, _event_rx) = mpsc::channel(8);
        let mut session = make_session("sess_reviewer_nonce_prompt");
        session.artifact = Some(artifact_payload(
            "# Story Spec\n\n## 功能需求\n- [REQ-001] Draft.",
        ));
        session.reviewer_provider = Some(ProviderName::Codex);
        let checkpoint_tmp = TempDir::new().unwrap();
        let engine = WorkspaceEngine::new(
            Arc::new(CheckpointStore::new(checkpoint_tmp.path().to_path_buf())),
            event_tx,
            session,
        );

        let input = engine.build_review_input().expect("review input");

        assert!(input.prompt.contains("<ARIA_STRUCTURED_OUTPUT nonce=\""));
        assert!(input.prompt.contains("</ARIA_STRUCTURED_OUTPUT nonce=\""));
        assert!(input.prompt.contains("不得使用 Markdown code fence"));
        assert!(!input.prompt.contains("```json"));
    }

    #[test]
    fn extract_structured_json_prefers_last_matching_nonce_block() {
        let output = "第一次输出\n\
            <ARIA_STRUCTURED_OUTPUT nonce=\"old00001\">{\"verdict\":\"needs_human\",\"summary\":\"old\"}</ARIA_STRUCTURED_OUTPUT nonce=\"old00001\">\n\
            最终输出\n\
            <ARIA_STRUCTURED_OUTPUT nonce=\"new00002\">{\"verdict\":\"pass\",\"summary\":\"new\"}</ARIA_STRUCTURED_OUTPUT nonce=\"new00002\">";

        let (comments, json) = extract_structured_json(output).expect("structured json");

        assert!(comments.contains("最终输出"));
        assert!(json.contains("\"summary\":\"new\""));
    }

    #[test]
    fn extract_structured_json_ignores_nonce_mismatch() {
        let output = "review text\n\
            <ARIA_STRUCTURED_OUTPUT nonce=\"a1b2c3d4\">{\"verdict\":\"pass\",\"summary\":\"ok\"}</ARIA_STRUCTURED_OUTPUT nonce=\"deadbeef\">";

        assert!(extract_structured_json(output).is_none());
    }

    #[test]
    fn extract_structured_json_falls_back_to_markdown_fence() {
        let output = "review text\n\n```json\n{\"verdict\":\"pass\",\"summary\":\"ok\"}\n```";

        let (comments, json) = extract_structured_json(output).expect("markdown fallback json");

        assert_eq!(comments.trim(), "review text");
        assert!(json.contains("\"summary\":\"ok\""));
    }

    #[test]
    fn extract_structured_json_treats_non_nonce_sentinel_as_text() {
        let output =
            "review text\n<ARIA_STRUCTURED_OUTPUT>{\"verdict\":\"pass\"}</ARIA_STRUCTURED_OUTPUT>";

        assert!(extract_structured_json(output).is_none());
    }

    #[test]
    fn parse_review_verdict_does_not_upgrade_actionable_comments_without_strong_findings() {
        let output = "**审核结论**\n\n\
            不建议通过。当前 Story Spec 覆盖主方向，但安装任务 API 设计存在实现级歧义。\n\n\
            **主要问题**\n\n\
            - **High**：进度接口无法区分并发安装、重试安装、页面刷新后重连到哪一次任务。\n\n\
            ```json\n\
            {\"verdict\":\"needs_human\",\"summary\":\"安装任务 API 设计需修正。\"}\n\
            ```";

        let verdict = WorkspaceEngine::parse_review_verdict(output);

        assert_eq!(verdict.verdict, ReviewVerdictType::NeedsHuman);
        assert_eq!(verdict.review_gate, ReviewGate::UserTriageRequired);
        assert_eq!(verdict.summary, "安装任务 API 设计需修正。");
        assert!(verdict.comments.contains("不建议通过"));
    }

    #[test]
    fn parse_review_verdict_defaults_to_needs_human_when_contract_missing() {
        let output = "我无法确定是否通过，请人工确认。";

        let verdict = WorkspaceEngine::parse_review_verdict(output);

        assert_eq!(verdict.verdict, ReviewVerdictType::NeedsHuman);
        assert_eq!(verdict.review_gate, ReviewGate::UserTriageRequired);
        assert_eq!(verdict.summary, "需要人工确认");
        assert_eq!(verdict.comments, output);
    }

    #[test]
    fn parse_review_verdict_classifies_optional_findings_as_user_confirm_allowed() {
        let output = r#"整体可用，建议补充措辞。

```json
{
  "verdict": "revise",
  "summary": "有非阻塞建议",
  "findings": [
    {
      "severity": "suggestion",
      "message": "建议补充边界说明",
      "evidence": "验收标准已经覆盖主路径",
      "impact": "不影响下一阶段执行",
      "required_action": "可在后续优化中补充"
    }
  ]
}
```"#;

        let verdict = WorkspaceEngine::parse_review_verdict(output);

        assert_eq!(verdict.verdict, ReviewVerdictType::NeedsHuman);
        assert_eq!(verdict.review_gate, ReviewGate::UserConfirmAllowed);
        assert_eq!(verdict.findings.len(), 1);
        assert_eq!(
            verdict.findings[0].severity,
            ReviewFindingSeverity::Suggestion
        );
    }

    #[test]
    fn parse_review_verdict_classifies_strong_findings_as_requires_revision() {
        let output = r#"缺少 Work Item 可执行验证命令。

```json
{
  "verdict": "revise",
  "summary": "必须补充验证命令",
  "findings": [
    {
      "severity": "must_fix",
      "message": "Work Item 没有验证命令",
      "evidence": "Artifact 未出现验证命令段落",
      "impact": "Coding Workspace 无法执行验收",
      "required_action": "补充明确验证命令"
    }
  ]
}
```"#;

        let verdict = WorkspaceEngine::parse_review_verdict(output);

        assert_eq!(verdict.verdict, ReviewVerdictType::Revise);
        assert_eq!(verdict.review_gate, ReviewGate::RequiresRevision);
        assert_eq!(verdict.findings[0].severity, ReviewFindingSeverity::MustFix);
    }

    #[test]
    fn parse_review_verdict_revise_without_findings_requires_user_triage() {
        let output = r#"建议修改一些描述。

```json
{"verdict":"revise","summary":"建议修改描述"}
```"#;

        let verdict = WorkspaceEngine::parse_review_verdict(output);

        assert_eq!(verdict.verdict, ReviewVerdictType::NeedsHuman);
        assert_eq!(verdict.review_gate, ReviewGate::UserTriageRequired);
        assert!(verdict.findings.is_empty());
    }

    #[test]
    fn parse_review_verdict_malformed_findings_require_user_triage() {
        let output = r#"建议返修，但 findings 结构不合规。

```json
{
  "verdict": "revise",
  "summary": "返修意图不结构化",
  "findings": [
    {
      "severity": "must_fix",
      "message": 42
    }
  ]
}
```"#;

        let verdict = WorkspaceEngine::parse_review_verdict(output);

        assert_eq!(verdict.verdict, ReviewVerdictType::NeedsHuman);
        assert_eq!(verdict.review_gate, ReviewGate::UserTriageRequired);
        assert!(verdict.findings.is_empty());
    }

    #[test]
    fn work_item_plan_review_revise_batch_maps_to_needs_human_generic_verdict_with_extension() {
        let json = r#"{
            "verdict": "revise_batch",
            "summary": "整组需要重写",
            "generation_round_id": "round_0001",
            "batch_id": "batch_0001"
        }"#;

        let verdict = parse_work_item_plan_review_json(
            json,
            "batch review comments",
            &["outline_api".to_string()],
            WorkItemPlanReviewScope::Batch,
        )
        .expect("work item plan review");

        assert_eq!(verdict.verdict, ReviewVerdictType::NeedsHuman);
        assert_eq!(verdict.review_gate, ReviewGate::UserTriageRequired);
        let review = verdict
            .work_item_plan_review
            .expect("work item plan extension");
        assert_eq!(review.verdict, WorkItemPlanReviewVerdict::ReviseBatch);
        assert_eq!(review.review_action, WorkItemPlanReviewAction::ReviseBatch);
        assert_eq!(
            review.gates,
            vec![WorkItemPlanReviewGate::RequiresBatchRevision]
        );
    }

    #[test]
    fn work_item_plan_review_invalid_target_outline_id_downgrades_to_needs_human() {
        let json = r#"{
            "verdict": "plan_reopen_required",
            "summary": "outline 不可局部修复",
            "target_outline_id": "outline_missing",
            "generation_round_id": "round_0001",
            "draft_id": "draft_0001"
        }"#;

        let verdict = parse_work_item_plan_review_json(
            json,
            "raw comments",
            &["outline_api".to_string()],
            WorkItemPlanReviewScope::Item,
        )
        .expect("work item plan review");

        assert_eq!(verdict.verdict, ReviewVerdictType::NeedsHuman);
        assert_eq!(verdict.review_gate, ReviewGate::UserTriageRequired);
        assert!(verdict.work_item_plan_review.is_none());
        assert!(verdict.summary.contains("引用无效"));
    }

    #[test]
    fn work_item_plan_review_drops_invalid_affects_items_below_threshold() {
        let json = r#"{
            "verdict": "needs_human",
            "summary": "部分 item 需要人工判断",
            "generation_round_id": "round_0001",
            "affects_items": [
                { "target_outline_id": "outline_api" },
                { "target_outline_id": "outline_missing" }
            ]
        }"#;

        let verdict = parse_work_item_plan_review_json(
            json,
            "",
            &["outline_api".to_string(), "outline_ui".to_string()],
            WorkItemPlanReviewScope::Batch,
        )
        .expect("work item plan review");

        let review = verdict
            .work_item_plan_review
            .expect("work item plan extension");
        assert_eq!(review.affects_items.len(), 1);
        assert_eq!(
            review.affects_items[0].target_outline_id.as_deref(),
            Some("outline_api")
        );
        assert!(
            review
                .warnings
                .iter()
                .any(|warning| warning.contains("outline_missing"))
        );
    }

    #[test]
    fn work_item_plan_review_invalid_affects_items_over_half_downgrades() {
        let json = r#"{
            "verdict": "needs_human",
            "summary": "引用大量不存在 item",
            "generation_round_id": "round_0001",
            "affects_items": [
                { "target_outline_id": "outline_api" },
                { "target_outline_id": "outline_missing_1" },
                { "target_outline_id": "outline_missing_2" }
            ]
        }"#;

        let verdict = parse_work_item_plan_review_json(
            json,
            "raw comments",
            &["outline_api".to_string(), "outline_ui".to_string()],
            WorkItemPlanReviewScope::Batch,
        )
        .expect("work item plan review");

        assert_eq!(verdict.verdict, ReviewVerdictType::NeedsHuman);
        assert_eq!(verdict.review_gate, ReviewGate::UserTriageRequired);
        assert!(verdict.work_item_plan_review.is_none());
        assert!(verdict.summary.contains("引用无效"));
    }

    #[test]
    fn review_complete_event_preserves_work_item_plan_extension() {
        let extension = WorkItemPlanReviewComplete {
            verdict: WorkItemPlanReviewVerdict::PlanReopenRequired,
            review_scope: WorkItemPlanReviewScope::Item,
            target_outline_id: Some("outline_api".to_string()),
            generation_round_id: "round_0001".to_string(),
            draft_id: Some("draft_0001".to_string()),
            batch_id: None,
            review_action: WorkItemPlanReviewAction::ReviseOutline,
            gates: vec![WorkItemPlanReviewGate::RequiresPlanReopen],
            affects_items: Vec::new(),
            warnings: Vec::new(),
        };
        let verdict = ReviewVerdict {
            verdict: ReviewVerdictType::NeedsHuman,
            comments: "需要重开 outline".to_string(),
            summary: "需要重开 Outline".to_string(),
            findings: Vec::new(),
            review_gate: ReviewGate::UserTriageRequired,
            work_item_plan_review: Some(extension.clone()),
        };

        let event = review_complete_event_from_verdict("node_review_001".to_string(), 2, &verdict);

        match event {
            EngineEvent::ReviewComplete {
                work_item_plan_review: Some(actual),
                ..
            } => assert_eq!(actual, extension),
            _ => panic!("expected review extension"),
        }
    }

    #[tokio::test]
    async fn optional_review_findings_enter_human_confirm_for_all_workspace_types() {
        for workspace_type in [
            WorkspaceType::Story,
            WorkspaceType::Design,
            WorkspaceType::WorkItem,
        ] {
            let (_tmp, store) = setup();
            let (tx, _) = mpsc::channel(64);
            let mut session = make_session(&format!("sess_optional_review_{workspace_type:?}"));
            session.workspace_type = workspace_type.clone();
            session.review_rounds = 2;
            session.artifact = Some(artifact_payload("# Artifact\n\n可用版本"));
            let mut engine = WorkspaceEngine::new(store, tx, session);
            engine.start_review_or_skip().await;

            engine
                .drive_review_session(
                    Arc::new(ReviewVerdictStreamingProvider {
                        output: r#"建议补充说明。

```json
{
  "verdict": "revise",
  "summary": "仅有可选建议",
  "findings": [
    {
      "severity": "optional",
      "message": "可补充说明",
      "evidence": "当前主路径完整",
      "impact": "不影响下一阶段执行",
      "required_action": "可后续优化"
    }
  ]
}
```"#,
                        provider_type: Arc::new(Mutex::new(None)),
                        prompt: Arc::new(Mutex::new(None)),
                    }),
                    empty_provider_commands(),
                )
                .await;

            assert_eq!(engine.session().stage, WorkspaceStage::HumanConfirm);
            assert!(
                engine
                    .timeline_nodes
                    .iter()
                    .any(|node| node.node_type == TimelineNodeType::HumanConfirm),
                "{workspace_type:?} should create human_confirm node"
            );
            assert!(
                !engine
                    .timeline_nodes
                    .iter()
                    .any(|node| node.node_type == TimelineNodeType::ReviewDecision),
                "{workspace_type:?} should not block optional review findings"
            );
        }
    }

    #[tokio::test]
    async fn strong_review_findings_enter_review_decision_for_all_workspace_types() {
        for workspace_type in [
            WorkspaceType::Story,
            WorkspaceType::Design,
            WorkspaceType::WorkItem,
        ] {
            let (_tmp, store) = setup();
            let (tx, _) = mpsc::channel(64);
            let mut session = make_session(&format!("sess_strong_review_{workspace_type:?}"));
            session.workspace_type = workspace_type.clone();
            session.review_rounds = 2;
            session.artifact = Some(artifact_payload("# Artifact\n\n缺少验收标准"));
            let mut engine = WorkspaceEngine::new(store, tx, session);
            engine.start_review_or_skip().await;

            engine
                .drive_review_session(
                    Arc::new(ReviewVerdictStreamingProvider {
                        output: r#"必须补充验收标准。

```json
{
  "verdict": "revise",
  "summary": "必须补充验收标准",
  "findings": [
    {
      "severity": "strong_recommend_fix",
      "message": "验收标准不足",
      "evidence": "Artifact 未列出可测试验收值",
      "impact": "下一阶段无法判断实现是否完成",
      "required_action": "补充明确验收标准"
    }
  ]
}
```"#,
                        provider_type: Arc::new(Mutex::new(None)),
                        prompt: Arc::new(Mutex::new(None)),
                    }),
                    empty_provider_commands(),
                )
                .await;

            assert_eq!(engine.session().stage, WorkspaceStage::ReviewDecision);
            assert!(
                engine
                    .timeline_nodes
                    .iter()
                    .any(|node| node.node_type == TimelineNodeType::ReviewDecision),
                "{workspace_type:?} should require revision for strong findings"
            );
        }
    }

    #[tokio::test]
    async fn revise_without_findings_enters_user_triage_for_all_workspace_types() {
        for workspace_type in [
            WorkspaceType::Story,
            WorkspaceType::Design,
            WorkspaceType::WorkItem,
        ] {
            let (_tmp, store) = setup();
            let (tx, _) = mpsc::channel(64);
            let mut session = make_session(&format!("sess_triage_review_{workspace_type:?}"));
            session.workspace_type = workspace_type.clone();
            session.review_rounds = 2;
            session.artifact = Some(artifact_payload("# Artifact\n\n需要人工裁决的版本"));
            let mut engine = WorkspaceEngine::new(store, tx, session);
            engine.start_review_or_skip().await;

            engine
                .drive_review_session(
                    Arc::new(ReviewVerdictStreamingProvider {
                        output: r#"Reviewer 明确要求返修，但未输出结构化 finding。

```json
{
  "verdict": "revise",
  "summary": "返修意图需要人工判断"
}
```"#,
                        provider_type: Arc::new(Mutex::new(None)),
                        prompt: Arc::new(Mutex::new(None)),
                    }),
                    empty_provider_commands(),
                )
                .await;

            assert_eq!(engine.session().stage, WorkspaceStage::HumanConfirm);
            assert_eq!(
                engine
                    .latest_review_verdict
                    .as_ref()
                    .expect("latest review verdict")
                    .review_gate,
                ReviewGate::UserTriageRequired
            );
            assert!(
                engine
                    .timeline_nodes
                    .iter()
                    .any(|node| node.node_type == TimelineNodeType::HumanConfirm),
                "{workspace_type:?} should create human_confirm node for user triage"
            );
            assert!(
                !engine
                    .timeline_nodes
                    .iter()
                    .any(|node| node.node_type == TimelineNodeType::ReviewDecision),
                "{workspace_type:?} should not auto-revise unstructured review intent"
            );
        }
    }

    #[tokio::test]
    async fn malformed_findings_enter_user_triage_for_all_workspace_types() {
        for workspace_type in [
            WorkspaceType::Story,
            WorkspaceType::Design,
            WorkspaceType::WorkItem,
        ] {
            let (_tmp, store) = setup();
            let (tx, _) = mpsc::channel(64);
            let mut session = make_session(&format!("sess_malformed_review_{workspace_type:?}"));
            session.workspace_type = workspace_type.clone();
            session.review_rounds = 2;
            session.artifact = Some(artifact_payload("# Artifact\n\n需要人工裁决的版本"));
            let mut engine = WorkspaceEngine::new(store, tx, session);
            engine.start_review_or_skip().await;

            engine
                .drive_review_session(
                    Arc::new(ReviewVerdictStreamingProvider {
                        output: r#"Reviewer 明确要求返修，但 findings 结构错误。

```json
{
  "verdict": "revise",
  "summary": "findings 无法可靠解析",
  "findings": [{"severity": "must_fix", "message": 42}]
}
```"#,
                        provider_type: Arc::new(Mutex::new(None)),
                        prompt: Arc::new(Mutex::new(None)),
                    }),
                    empty_provider_commands(),
                )
                .await;

            assert_eq!(engine.session().stage, WorkspaceStage::HumanConfirm);
            assert_eq!(
                engine
                    .latest_review_verdict
                    .as_ref()
                    .expect("latest review verdict")
                    .review_gate,
                ReviewGate::UserTriageRequired
            );
        }
    }

    #[test]
    fn review_prompt_limits_revise_to_strong_findings() {
        let (_tmp, store) = setup();
        let (tx, _) = mpsc::channel(8);
        let mut session = make_session("sess_review_prompt_gate");
        session.artifact = Some(artifact_payload("# Story Spec\n\n可用版本"));
        let engine = WorkspaceEngine::new(store, tx, session);

        let input = engine.build_review_input().expect("review input");

        assert!(
            input
                .prompt
                .contains("blocking|must_fix|strong_recommend_fix")
        );
        assert!(input.prompt.contains("suggestion|minor|optional"));
        assert!(
            input
                .prompt
                .contains("没有强返修 finding 时，必须允许用户确认当前版本")
        );
        assert!(
            !input
                .prompt
                .contains("High/Medium 问题、建议改动或可执行返修项，必须使用 `revise`")
        );
        assert!(
            input
                .prompt
                .contains("如果输出 `verdict=revise`，必须给出至少一个结构化 finding")
        );
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

    #[test]
    fn detect_author_choice_request_uses_nearest_question_for_codex_numbered_options() {
        let output = "我会先读取本仓库规则和必须使用的技能说明，然后根据未决点用结构化提问确认范围，再产出候选 Story Spec。\
            规则侧已经明确：这次最终只输出候选 Markdown，不落盘、不改 OpenSpec。\
            结构化提问工具当前不可用，我先用文本方式提问：\n\n\
            首次启动检测到缺失 Claude Code/Codex 时，Aria 应采用哪种安装策略？\n\n\
            1. `确认后安装`：弹窗展示将执行的 npm 安装命令，用户点击安装后才执行。\n\
            2. `自动静默安装`：检测缺失后直接运行 npm 安装。\n\
            3. `只检查不安装`：只展示缺失与命令，由用户自行安装。\n\n\
            我建议选 `确认后安装`，因为它满足“自动检查与自动安装”。";

        let (prompt, options) = detect_author_choice_request(output, &WorkspaceType::Story)
            .expect("Codex numbered text question should become a choice request");

        assert_eq!(
            prompt,
            "首次启动检测到缺失 Claude Code/Codex 时，Aria 应采用哪种安装策略？"
        );
        assert!(!prompt.contains("我会先读取本仓库规则"));
        assert!(!prompt.contains("结构化提问工具当前不可用"));
        assert_eq!(options.len(), 3);
        assert_eq!(options[0].id, "1");
        assert!(options[0].label.contains("确认后安装"));
        assert_eq!(options[1].id, "2");
        assert_eq!(options[2].id, "3");
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
                verdict,
                summary,
                findings,
                review_gate,
                ..
            } = event
            {
                assert_eq!(verdict, ReviewVerdictType::Pass);
                assert_eq!(summary, "可以确认");
                assert!(findings.is_empty());
                assert_eq!(review_gate, ReviewGate::UserConfirmAllowed);
                saw_review_complete = true;
            }
        }
        assert!(saw_review_complete);
    }

    #[tokio::test]
    async fn drive_review_session_strong_revise_pauses_for_decision() {
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
                    output: r#"需要补充失败路径。

```json
{
  "verdict": "revise",
  "summary": "补充失败路径",
  "findings": [
    {
      "severity": "must_fix",
      "message": "缺少失败路径",
      "evidence": "Artifact 未覆盖失败路径",
      "impact": "下一阶段无法验收异常流程",
      "required_action": "补充失败路径说明"
    }
  ]
}
```"#,
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
    async fn single_review_round_strong_revise_still_pauses_for_decision() {
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
                    output: r#"需要移除非规范正文。

```json
{
  "verdict": "revise",
  "summary": "需要返修",
  "findings": [
    {
      "severity": "strong_recommend_fix",
      "message": "存在非规范正文",
      "evidence": "Artifact 包含不符合模板的正文",
      "impact": "会影响下一阶段投影和审核",
      "required_action": "移除非规范正文"
    }
  ]
}
```"#,
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
    async fn review_decision_continue_after_strong_revise_runs_revision() {
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
                .as_ref()
                .is_some_and(|artifact| artifact.markdown_or_empty().contains("# Story Spec"))
        );
        engine
            .handle_author_decision(AuthorDecision::Accept)
            .await
            .unwrap();
        assert_eq!(engine.session().stage, WorkspaceStage::CrossReview);

        engine
            .drive_review_session(
                Arc::new(ReviewVerdictStreamingProvider {
                    output: r#"需要补充失败路径。

```json
{
  "verdict": "revise",
  "summary": "补充失败路径",
  "findings": [
    {
      "severity": "must_fix",
      "message": "缺少失败路径",
      "evidence": "Artifact 未覆盖登录错误码",
      "impact": "下一阶段无法实现和验收失败路径",
      "required_action": "补充登录错误码失败路径"
    }
  ]
}
```"#,
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
            engine
                .session()
                .artifact
                .as_ref()
                .map(|payload| payload.markdown_or_empty()),
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
                findings: Vec::new(),
                review_gate: ReviewGate::RequiresRevision,
                work_item_plan_review: None,
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
        session.artifact = Some(artifact_payload(
            "# Story Spec\n\n## 功能需求\n- [REQ-001] 初版。\n\n## 成功标准\n- [AC-001] 初版可验收。\n",
        ));
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
            findings: Vec::new(),
            review_gate: ReviewGate::RequiresRevision,
            work_item_plan_review: None,
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
        session.artifact = Some(artifact_payload(
            "# Story Spec\n\n## 功能需求\n- [REQ-001] 初版。\n\n## 成功标准\n- [AC-001] 初版可验收。\n",
        ));
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
            content: session
                .artifact
                .clone()
                .unwrap()
                .into_markdown()
                .expect("artifact"),
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
            findings: Vec::new(),
            review_gate: ReviewGate::RequiresRevision,
            work_item_plan_review: None,
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
    async fn revision_codex_resume_stall_retries_fresh_full_prompt_for_all_workspace_types() {
        for (workspace_type, artifact, output) in [
            (
                WorkspaceType::Story,
                "# Story Spec\n\n## 功能需求\n- [REQ-001] 初版。\n\n## 成功标准\n- [AC-001] 初版可验收。\n",
                "# Story Spec\n\n## 功能需求\n- [REQ-001] fresh 返修版本。\n\n## 成功标准\n- [AC-001] fresh 返修可验收。\n",
            ),
            (
                WorkspaceType::Design,
                "# Design Spec\n\n## 设计决策\n- [DEC-001] 初版。\n\n## API 契约\n- [API-001] 初版接口。\n",
                "# Design Spec\n\n## 设计决策\n- [DEC-001] fresh 返修版本。\n\n## API 契约\n- [API-001] fresh 返修接口。\n",
            ),
            (
                WorkspaceType::WorkItem,
                "# Work Item\n\n## 目标\n- 初版任务。\n\n## 验证命令\n- cargo test --locked\n",
                "# Work Item\n\n## 目标\n- fresh 返修任务。\n\n## 验证命令\n- cargo test --locked\n",
            ),
        ] {
            let (_tmp, store) = setup();
            let (tx, _) = mpsc::channel(64);
            let mut session =
                make_session(&format!("sess_revision_resume_stall_{workspace_type:?}"));
            session.workspace_type = workspace_type.clone();
            session.stage = WorkspaceStage::ReviewDecision;
            session.artifact = Some(artifact_payload(artifact));
            session.author_provider = ProviderName::Codex;
            session.messages.push(SessionMessage {
                id: "msg_001".to_string(),
                role: "assistant".to_string(),
                content: artifact.to_string(),
                checkpoint_id: None,
                created_at: chrono::Utc::now().to_rfc3339(),
            });
            session
                .provider_conversations
                .push(ProviderConversationRef {
                    role: ProviderConversationRole::Author,
                    provider: ProviderName::Codex,
                    provider_session_id: "codex-stale-ephemeral-thread".to_string(),
                    updated_at: chrono::Utc::now().to_rfc3339(),
                    last_node_id: Some("timeline_node_002".to_string()),
                });
            let mut engine = WorkspaceEngine::new(store, tx, session);
            engine.latest_review_verdict = Some(ReviewVerdict {
                verdict: ReviewVerdictType::Revise,
                comments: "需要补充失败路径。".to_string(),
                summary: "补充失败路径".to_string(),
                findings: Vec::new(),
                review_gate: ReviewGate::RequiresRevision,
                work_item_plan_review: None,
            });
            engine
                .handle_review_decision(
                    "continue_with_context".to_string(),
                    Some("补充旧 session 不可恢复时的处理。".to_string()),
                )
                .await
                .expect("review decision should enter revision");

            let inputs = Arc::new(Mutex::new(Vec::new()));
            engine
                .drive_revision_session(
                    Arc::new(RevisionResumeStallThenSuccessProvider {
                        inputs: inputs.clone(),
                        calls: Arc::new(Mutex::new(0)),
                        output,
                    }),
                    empty_provider_commands(),
                )
                .await;

            let inputs = inputs.lock().unwrap().clone();
            assert_eq!(inputs.len(), 2, "{workspace_type:?} should retry once");
            assert_eq!(
                inputs[0].resume_provider_session_id.as_deref(),
                Some("codex-stale-ephemeral-thread")
            );
            assert!(
                !inputs[0].prompt.contains("上一版 Artifact"),
                "{workspace_type:?} first resume attempt should use delta prompt"
            );
            assert_eq!(inputs[1].resume_provider_session_id, None);
            assert!(
                inputs[1].prompt.contains("上一版 Artifact"),
                "{workspace_type:?} fresh retry should use full prompt"
            );
            assert!(
                inputs[1].prompt.contains(artifact.trim()),
                "{workspace_type:?} fresh retry should include prior artifact"
            );
            assert_eq!(
                engine
                    .session()
                    .artifact
                    .as_ref()
                    .map(|payload| payload.markdown_or_empty()),
                Some(output.trim())
            );
            assert_eq!(
                engine
                    .provider_resume_session_id(
                        ProviderConversationRole::Author,
                        &ProviderName::Codex,
                    )
                    .as_deref(),
                Some("codex-fresh-thread")
            );
        }
    }

    #[test]
    fn revision_input_reminds_design_author_to_return_artifact_fenced_block() {
        let (event_tx, _event_rx) = mpsc::channel(8);
        let mut session = make_session("sess_design_revision_prompt_fence_contract");
        session.workspace_type = WorkspaceType::Design;
        session.stage = WorkspaceStage::Revision;
        session.artifact = Some(artifact_payload(
            "# 底层依赖安装任务 Design Spec\n\n\
             ## 设计范围\n\n\
             - 覆盖依赖安装任务。\n\n\
             ## API 契约\n\n\
             ```json\n\
             {\"task_id\":\"install_001\"}\n\
             ```\n",
        ));
        let checkpoint_tmp = TempDir::new().unwrap();
        let mut engine = WorkspaceEngine::new(
            Arc::new(CheckpointStore::new(checkpoint_tmp.path().to_path_buf())),
            event_tx,
            session,
        );
        engine.latest_review_verdict = Some(ReviewVerdict {
            verdict: ReviewVerdictType::Revise,
            comments: "需要补齐追踪关系。".to_string(),
            summary: "补齐追踪关系".to_string(),
            findings: Vec::new(),
            review_gate: ReviewGate::RequiresRevision,
            work_item_plan_review: None,
        });

        let input = engine.build_revision_input().expect("revision input");

        assert!(
            input
                .prompt
                .contains("原始返回必须使用完整 artifact fenced block"),
            "revision author prompt should require the raw artifact fence: {}",
            input.prompt
        );
        assert!(
            input
                .prompt
                .contains("正文内部包含 ``` 代码块时，外层使用四反引号 ````artifact"),
            "revision author prompt should explain four-backtick outer fence: {}",
            input.prompt
        );
        assert!(
            input
                .prompt
                .contains("上一版 Artifact 是 daemon 已提取的 markdown"),
            "revision author prompt should explain why prior artifact has no fence: {}",
            input.prompt
        );
    }

    #[tokio::test]
    async fn revision_delta_prompt_includes_legacy_context_note() {
        let (_tmp, store) = setup();
        let (tx, _) = mpsc::channel(64);
        let mut session = make_session("sess_revision_delta_legacy_context_note");
        session.stage = WorkspaceStage::Revision;
        session.artifact = Some(artifact_payload(
            "# Story Spec\n\n## 功能需求\n- [REQ-001] 初版。\n\n## 成功标准\n- [AC-001] 初版可验收。\n",
        ));
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
            findings: Vec::new(),
            review_gate: ReviewGate::RequiresRevision,
            work_item_plan_review: None,
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

    struct RevisionResumeStallThenSuccessProvider {
        inputs: Arc<Mutex<Vec<StreamingProviderInput>>>,
        calls: Arc<Mutex<u32>>,
        output: &'static str,
    }

    #[async_trait::async_trait]
    impl StreamingProviderAdapter for RevisionResumeStallThenSuccessProvider {
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
            let output = self.output.to_string();
            tokio::spawn(async move {
                if call_no == 1 {
                    let _ = event_tx
                        .send(ProviderEvent::Failed {
                            message:
                                "Codex resume stalled before provider progress for thread codex-stale-ephemeral-thread"
                                    .to_string(),
                        })
                        .await;
                } else {
                    let _ = event_tx
                        .send(ProviderEvent::Completed {
                            full_output: output,
                            provider_session_id: Some("codex-fresh-thread".to_string()),
                        })
                        .await;
                }
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

        engine.handle_confirm().await.unwrap();
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

        engine.handle_confirm().await.unwrap();

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
                entity_id: "issue_work_item_plan_0001".to_string(),
                workspace_type: WorkspaceType::WorkItemPlan,
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
        let huge_prompt = "P".repeat(3000);
        let huge_stream = "S".repeat(3000);
        let huge_output = "O".repeat(3000);
        let artifact_markdown = format!("# Artifact\n\n{}", "M".repeat(3000));
        engine.artifact_versions.push(ArtifactVersion {
            version: 2,
            payload: ArtifactPayload::Markdown {
                markdown: artifact_markdown.clone(),
                diff: None,
            },
            generated_by: ProviderName::ClaudeCode,
            reviewed_by: Some(ProviderName::Codex),
            review_verdict: Some(ReviewVerdictType::Pass),
            confirmed_by: Some("user".to_string()),
            is_current: true,
            created_at: "2026-05-20T14:35:00Z".to_string(),
            source_node_id: "node-1".to_string(),
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
            prompt: Some(huge_prompt.clone()),
            messages: vec![],
            streaming_content: huge_stream.clone(),
            execution_events: vec![serde_json::json!({"output": huge_output})],
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
        let serialized = serde_json::to_string(&state).unwrap();
        match state {
            WsOutMessage::SessionState {
                timeline_node_details,
                timeline_node_summaries,
                artifact_versions,
                artifact_version_summaries,
                active_run_id,
                ..
            } => {
                assert!(artifact_versions.is_empty());

                let inline_detail = timeline_node_details
                    .get("node-1")
                    .expect("inline node detail");
                assert_eq!(inline_detail.node_id, "node-1");
                assert_eq!(inline_detail.prompt, None);
                assert!(inline_detail.messages.is_empty());
                assert!(inline_detail.streaming_content.chars().count() <= SUMMARY_PREVIEW_CHARS);
                assert_ne!(inline_detail.streaming_content, huge_stream);
                assert!(inline_detail.execution_events.is_empty());
                assert!(inline_detail.permission_events.is_empty());
                assert_eq!(inline_detail.artifact_ref.as_ref().unwrap().version, 2);

                let summary = timeline_node_summaries.get("node-1").expect("node summary");
                assert_eq!(summary.node_id, "node-1");
                assert_eq!(summary.prompt_size, huge_prompt.len());
                assert!(summary.prompt_preview.as_ref().unwrap().chars().count() <= 2048);
                assert_ne!(
                    summary.prompt_preview.as_deref(),
                    Some(huge_prompt.as_str())
                );
                assert_eq!(summary.stream_size, huge_stream.len());
                assert!(summary.stream_preview.as_ref().unwrap().chars().count() <= 2048);
                assert_ne!(
                    summary.stream_preview.as_deref(),
                    Some(huge_stream.as_str())
                );
                assert_eq!(summary.execution_event_count, 1);
                assert_eq!(summary.artifact_ref.as_deref(), Some("artifact-1/v2"));
                assert!(summary.has_large_outputs);

                let artifact_summary = artifact_version_summaries
                    .iter()
                    .find(|summary| summary.version == 2)
                    .expect("artifact summary");
                assert_eq!(artifact_summary.markdown_size, artifact_markdown.len());
                assert!(artifact_summary.markdown_preview.chars().count() <= 2048);
                assert_ne!(artifact_summary.markdown_preview, artifact_markdown);
                assert_eq!(active_run_id.as_deref(), Some("run-1"));
            }
            _ => panic!("expected SessionState"),
        }
        assert!(!serialized.contains(&huge_prompt));
        assert!(!serialized.contains(&huge_stream));
        assert!(!serialized.contains(&artifact_markdown));
    }

    #[tokio::test]
    async fn build_session_state_keeps_story_details_out_of_inline_payload() {
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
            node_id: "node-story".to_string(),
            node_type: TimelineNodeType::AuthorRun,
            agent: Some(ProviderName::ClaudeCode),
            stage: WsWorkspaceStage::Completed,
            round: None,
            status: TimelineNodeStatus::Completed,
            title: "Story 生成".to_string(),
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
        lifecycle_store
            .save_node_detail(
                &session_id,
                "node-story",
                &NodeDetail {
                    node_id: "node-story".to_string(),
                    session_id: session_id.clone(),
                    node_type: TimelineNodeType::AuthorRun,
                    status: TimelineNodeStatus::Completed,
                    agent_role: Some(AgentRole::Author),
                    provider: None,
                    prompt: None,
                    messages: vec![],
                    streaming_content: "Story provider stream".to_string(),
                    execution_events: vec![],
                    permission_events: vec![],
                    verdict: None,
                    artifact_ref: None,
                    is_revision: false,
                    base_artifact_ref: None,
                    started_at: "2026-05-20T14:30:00Z".to_string(),
                    ended_at: Some("2026-05-20T14:35:00Z".to_string()),
                },
            )
            .unwrap();

        match engine.build_session_state() {
            WsOutMessage::SessionState {
                timeline_node_details,
                timeline_node_summaries,
                ..
            } => {
                assert!(timeline_node_details.is_empty());
                assert!(
                    timeline_node_summaries
                        .get("node-story")
                        .and_then(|summary| summary.stream_preview.as_deref())
                        .is_some_and(|stream| stream.contains("Story provider stream"))
                );
            }
            _ => panic!("expected SessionState"),
        }
    }

    #[tokio::test]
    async fn append_active_run_stream_sends_event_when_detail_persist_fails() {
        let (tmp, checkpoint_store) = setup();
        let lifecycle_store = LifecycleStore::new(ProductAppPaths::new(tmp.path().join(".aria")));
        let (tx, mut rx) = mpsc::channel(64);
        let session_record = lifecycle_store
            .create_workspace_session(CreateWorkspaceSessionInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                entity_id: "issue_work_item_plan_0001".to_string(),
                workspace_type: WorkspaceType::WorkItemPlan,
                author_provider: ProviderName::ClaudeCode,
                reviewer_provider: ProviderName::Codex,
                review_rounds: 2,
                superpowers_enabled: true,
                openspec_enabled: true,
            })
            .unwrap();
        let session = WorkspaceSession::from_record(session_record);
        let mut engine =
            WorkspaceEngine::new_persistent(checkpoint_store, lifecycle_store, tx, session);
        engine.active_node_id = Some("missing-node".to_string());

        let result = engine
            .append_active_run_stream("assistant", "正在生成 Work Item Plan：准备上下文\n")
            .await;

        assert!(result.is_err());
        let event = tokio::time::timeout(Duration::from_secs(1), rx.recv())
            .await
            .expect("stream chunk event timeout")
            .expect("stream chunk event");
        match event {
            EngineEvent::StreamChunk {
                role,
                content,
                node_id,
            } => {
                assert_eq!(role, "assistant");
                assert!(content.contains("正在生成 Work Item Plan"));
                assert_eq!(node_id.as_deref(), Some("missing-node"));
            }
            _ => panic!("expected StreamChunk"),
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
            findings: Vec::new(),
            review_gate: ReviewGate::UserConfirmAllowed,
            work_item_plan_review: None,
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
                .markdown()
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
        assert!(engine.session().artifact.as_ref().is_some_and(|artifact| {
            let artifact = artifact.markdown_or_empty();
            artifact.contains("## 功能需求") && artifact.contains("## 成功标准")
        }));

        let mut saw_artifact = false;
        let mut saw_author_confirm = false;
        while let Ok(event) = rx.try_recv() {
            match event {
                EngineEvent::ArtifactUpdate { payload, .. }
                    if payload.markdown_or_empty().contains("## 功能需求")
                        && payload.markdown_or_empty().contains("## 成功标准") =>
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
        assert!(engine.session().artifact.as_ref().is_some_and(|artifact| {
            artifact
                .markdown_or_empty()
                .contains("# Streamed Story Spec")
        }));
        assert!(
            drain_engine_events(&mut rx).iter().any(|event| matches!(
                event,
                EngineEvent::ArtifactUpdate { payload, .. }
                    if payload.markdown_or_empty().contains("# Streamed Story Spec")
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
        assert!(engine.session().artifact.as_ref().is_some_and(|artifact| {
            let artifact = artifact.markdown_or_empty();
            artifact.contains("## 设计决策") && artifact.contains("## 公共组件")
        }));
        assert!(
            drain_engine_events(&mut rx).iter().any(|event| matches!(
                event,
                EngineEvent::ArtifactUpdate { payload, .. }
                    if payload.markdown_or_empty().contains("# Retried Design Spec")
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
            .drive_provider_session(ProviderSessionDriveInput {
                session: Ok(tool_event_provider_session("# Draft")),
                command_rx: empty_provider_commands(),
                node_id: Some(node_id.clone()),
                agent: Some(ProviderName::ClaudeCode),
                role: ProviderConversationRole::Author,
                artifact_retry: None,
                revision_resume_fallback: None,
            })
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
        let tool_events = detail
            .execution_events
            .iter()
            .filter(|event| event["event_id"] == "tool_0001")
            .collect::<Vec<_>>();
        assert_eq!(
            tool_events.len(),
            1,
            "same provider execution event id should be persisted once, got {detail:?}"
        );
        assert!(
            tool_events[0]["status"] == "completed"
                && tool_events[0]["output"] == "updated stairs.py",
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
    async fn finish_active_run_with_failed_node_marks_outline_node_failed() {
        let (_tmp, lifecycle_store, mut engine) = persistent_test_engine();
        let node_id = engine.begin_work_item_plan_outline_run().await;
        engine.mark_active_run_started("outline-run");

        engine
            .finish_active_run_with_failed_node("Outline structured output parse failed")
            .await;

        let node = engine
            .timeline_nodes
            .iter()
            .find(|node| node.node_id == node_id)
            .expect("outline node");
        assert_eq!(node.status, TimelineNodeStatus::Failed);
        assert_eq!(
            node.summary.as_deref(),
            Some("Outline structured output parse failed")
        );
        assert_eq!(engine.active_run_id(), None);
        assert_eq!(engine.current_stage(), WorkspaceStage::PrepareContext);

        let detail = lifecycle_store
            .load_node_detail(&engine.session().session_id, &node_id)
            .expect("node detail");
        assert_eq!(detail.status, TimelineNodeStatus::Failed);
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

    #[tokio::test]
    async fn complete_work_item_plan_author_pushes_candidate_and_enters_author_confirm() {
        use crate::product::lifecycle_store::{
            CreateDesignSpecInput, CreateIssueWorkItemPlanInput, CreateStorySpecInput,
        };
        use crate::product::models::{
            IssueWorkItemDependencyEdge, IssueWorkItemPlanOptions, IssueWorkItemPlanStatus,
            LifecycleWorkItemRecord, RepositoryProfile, RepositoryProfileConfidence,
            VerificationCommand, VerificationCommandSafety, VerificationCommandSource,
            VerificationFallbackPolicy, VerificationManualCheck, VerificationPlan,
            VerificationScope, WorkItemContextBudget, WorkItemExecutionPlanStatus, WorkItemKind,
            WorkItemPlanStatus, WorkItemStatus,
        };
        use crate::product::work_item_split_engine::WorkItemSplitProviderOutput;

        let (_tmp, checkpoint_store) = setup();
        let app_root = tempfile::tempdir().expect("app root");
        let lifecycle_store =
            LifecycleStore::new(ProductAppPaths::new(app_root.path().join(".aria")));
        let project_id = "project_0001";
        let issue_id = "issue_0001";
        let repository_id = "repo_0001";

        let story = lifecycle_store
            .create_story_spec(CreateStorySpecInput {
                project_id: project_id.to_string(),
                issue_id: issue_id.to_string(),
                repository_id: repository_id.to_string(),
                title: "Story".to_string(),
            })
            .unwrap();
        let design = lifecycle_store
            .create_design_spec(CreateDesignSpecInput {
                project_id: project_id.to_string(),
                issue_id: issue_id.to_string(),
                story_spec_ids: vec![story.id.clone()],
                title: "Design".to_string(),
            })
            .unwrap();

        let plan = lifecycle_store
            .create_issue_work_item_plan(CreateIssueWorkItemPlanInput {
                id: Some("issue_work_item_plan_0001".to_string()),
                project_id: project_id.to_string(),
                issue_id: issue_id.to_string(),
                source_story_spec_ids: vec![story.id.clone()],
                source_design_spec_ids: vec![design.id.clone()],
                options: IssueWorkItemPlanOptions {
                    include_integration_tests: false,
                    include_e2e_tests: false,
                    force_frontend_backend_split: false,
                    require_execution_plan_confirm: false,
                },
                status: IssueWorkItemPlanStatus::Draft,
                work_item_ids: vec![],
                repository_profile_ref: None,
                verification_plan_ids: vec![],
                dependency_graph: vec![],
                created_from_provider_run: None,
                validator_findings: vec![],
            })
            .unwrap();

        let session_record = lifecycle_store
            .create_workspace_session(CreateWorkspaceSessionInput {
                project_id: project_id.to_string(),
                issue_id: issue_id.to_string(),
                entity_id: plan.id.clone(),
                workspace_type: WorkspaceType::WorkItemPlan,
                author_provider: ProviderName::Codex,
                reviewer_provider: ProviderName::ClaudeCode,
                review_rounds: 0,
                superpowers_enabled: false,
                openspec_enabled: false,
            })
            .unwrap();

        let session = WorkspaceSession::from_record(session_record);
        let (tx, _rx) = mpsc::channel(64);
        let mut engine =
            WorkspaceEngine::new_persistent(checkpoint_store, lifecycle_store.clone(), tx, session);

        let now = chrono::Utc::now().to_rfc3339();
        let repository_profile = RepositoryProfile {
            id: "repo_profile_0001".to_string(),
            project_id: project_id.to_string(),
            issue_id: issue_id.to_string(),
            repository_id: repository_id.to_string(),
            provider_run_ref: None,
            languages: vec!["rust".to_string()],
            frameworks: vec![],
            package_managers: vec!["cargo".to_string()],
            test_frameworks: vec![],
            build_systems: vec!["cargo".to_string()],
            verification_capabilities: vec![],
            detected_layers: vec!["backend".to_string()],
            split_recommendation: "backend_only".to_string(),
            confidence: RepositoryProfileConfidence::High,
            uncertainties: vec![],
            created_at: now.clone(),
            updated_at: now.clone(),
        };

        let work_items = vec![
            LifecycleWorkItemRecord {
                id: "wi_001".to_string(),
                project_id: project_id.to_string(),
                issue_id: issue_id.to_string(),
                repository_id: repository_id.to_string(),
                story_spec_ids: vec![story.id.clone()],
                design_spec_ids: vec![design.id.clone()],
                title: "Backend work item".to_string(),
                plan_status: WorkItemPlanStatus::Draft,
                execution_status: WorkItemStatus::Pending,
                worktree_path: None,
                work_item_set_id: None,
                kind: WorkItemKind::Backend,
                sequence_hint: None,
                depends_on: vec![],
                exclusive_write_scopes: vec!["src/backend.rs".to_string()],
                forbidden_write_scopes: vec![],
                context_budget: WorkItemContextBudget::default(),
                required_handoff_from: vec![],
                verification_plan_ref: Some("vp_001".to_string()),
                require_execution_plan_confirm: false,
                execution_plan_status: WorkItemExecutionPlanStatus::NotStarted,
                handoff_summary_ref: None,
                completion_commit: None,
                completion_diff_summary_ref: None,
                created_at: now.clone(),
                updated_at: now.clone(),
            },
            LifecycleWorkItemRecord {
                id: "wi_002".to_string(),
                project_id: project_id.to_string(),
                issue_id: issue_id.to_string(),
                repository_id: repository_id.to_string(),
                story_spec_ids: vec![story.id.clone()],
                design_spec_ids: vec![design.id.clone()],
                title: "Frontend work item".to_string(),
                plan_status: WorkItemPlanStatus::Draft,
                execution_status: WorkItemStatus::Pending,
                worktree_path: None,
                work_item_set_id: None,
                kind: WorkItemKind::Frontend,
                sequence_hint: None,
                depends_on: vec!["wi_001".to_string()],
                exclusive_write_scopes: vec!["src/frontend.rs".to_string()],
                forbidden_write_scopes: vec![],
                context_budget: WorkItemContextBudget::default(),
                required_handoff_from: vec![],
                verification_plan_ref: Some("vp_002".to_string()),
                require_execution_plan_confirm: false,
                execution_plan_status: WorkItemExecutionPlanStatus::NotStarted,
                handoff_summary_ref: None,
                completion_commit: None,
                completion_diff_summary_ref: None,
                created_at: now.clone(),
                updated_at: now.clone(),
            },
        ];

        let verification_plans = vec![
            VerificationPlan {
                id: "vp_001".to_string(),
                project_id: project_id.to_string(),
                issue_id: issue_id.to_string(),
                work_item_id: "wi_001".to_string(),
                repository_profile_ref: Some("repo_profile_0001".to_string()),
                provider_run_ref: None,
                scope: VerificationScope::Unit,
                commands: vec![VerificationCommand {
                    id: "cmd_001".to_string(),
                    label: "cargo test".to_string(),
                    command: "cargo test".to_string(),
                    cwd: "".to_string(),
                    purpose: "unit tests".to_string(),
                    required: true,
                    timeout_seconds: 120,
                    source: VerificationCommandSource::Provider,
                    safety: VerificationCommandSafety::Approved,
                }],
                manual_checks: vec![VerificationManualCheck {
                    id: "check_001".to_string(),
                    label: "manual".to_string(),
                    instructions: "check".to_string(),
                    required: false,
                }],
                required_gates: vec!["cmd_001".to_string()],
                risk_notes: vec![],
                confidence: RepositoryProfileConfidence::High,
                fallback_policy: VerificationFallbackPolicy::ManualGate,
                created_at: now.clone(),
                updated_at: now.clone(),
            },
            VerificationPlan {
                id: "vp_002".to_string(),
                project_id: project_id.to_string(),
                issue_id: issue_id.to_string(),
                work_item_id: "wi_002".to_string(),
                repository_profile_ref: Some("repo_profile_0001".to_string()),
                provider_run_ref: None,
                scope: VerificationScope::Unit,
                commands: vec![VerificationCommand {
                    id: "cmd_002".to_string(),
                    label: "cargo test".to_string(),
                    command: "cargo test".to_string(),
                    cwd: "".to_string(),
                    purpose: "unit tests".to_string(),
                    required: true,
                    timeout_seconds: 120,
                    source: VerificationCommandSource::Provider,
                    safety: VerificationCommandSafety::Approved,
                }],
                manual_checks: vec![],
                required_gates: vec![],
                risk_notes: vec![],
                confidence: RepositoryProfileConfidence::High,
                fallback_policy: VerificationFallbackPolicy::ManualGate,
                created_at: now.clone(),
                updated_at: now.clone(),
            },
        ];

        let plan_record = IssueWorkItemPlan {
            id: plan.id.clone(),
            project_id: project_id.to_string(),
            issue_id: issue_id.to_string(),
            source_story_spec_ids: vec![story.id.clone()],
            source_design_spec_ids: vec![design.id.clone()],
            options: IssueWorkItemPlanOptions {
                include_integration_tests: false,
                include_e2e_tests: false,
                force_frontend_backend_split: false,
                require_execution_plan_confirm: false,
            },
            status: IssueWorkItemPlanStatus::Draft,
            work_item_ids: vec!["wi_001".to_string(), "wi_002".to_string()],
            repository_profile_ref: Some("repo_profile_0001".to_string()),
            verification_plan_ids: vec!["vp_001".to_string(), "vp_002".to_string()],
            dependency_graph: vec![IssueWorkItemDependencyEdge {
                from_work_item_id: "wi_001".to_string(),
                to_work_item_id: "wi_002".to_string(),
            }],
            created_from_provider_run: None,
            validator_findings: vec![],
            review_summary: None,
            created_at: now.clone(),
            updated_at: now.clone(),
        };

        let output = WorkItemSplitProviderOutput {
            repository_profile,
            plan: plan_record,
            work_items,
            verification_plans,
        };

        let outcome = engine
            .complete_work_item_plan_author(output)
            .await
            .expect("author completion should succeed");
        assert!(matches!(outcome, WorkItemPlanAuthorOutcome::AuthorConfirm));

        let artifact = engine
            .session()
            .artifact
            .as_ref()
            .expect("artifact should be set");
        assert!(matches!(
            artifact,
            ArtifactPayload::WorkItemPlanCandidate { .. }
        ));
        assert_eq!(engine.session().stage, WorkspaceStage::AuthorConfirm);

        if let ArtifactPayload::WorkItemPlanCandidate { candidate } = artifact {
            assert_eq!(candidate.work_items.len(), 2);
            assert_eq!(candidate.plan.id, plan.id);
        }
    }

    #[tokio::test]
    async fn complete_work_item_plan_author_errors_trigger_auto_revision_then_human_confirm() {
        use crate::product::lifecycle_store::{
            CreateDesignSpecInput, CreateIssueWorkItemPlanInput, CreateStorySpecInput,
        };
        use crate::product::models::{
            IssueWorkItemPlanOptions, IssueWorkItemPlanStatus, LifecycleWorkItemRecord,
            RepositoryProfile, RepositoryProfileConfidence, VerificationCommand,
            VerificationCommandSafety, VerificationCommandSource, VerificationFallbackPolicy,
            VerificationManualCheck, VerificationPlan, VerificationScope, WorkItemContextBudget,
            WorkItemExecutionPlanStatus, WorkItemKind, WorkItemPlanStatus,
            WorkItemSplitFindingSeverity, WorkItemStatus,
        };
        use crate::product::work_item_split_engine::WorkItemSplitProviderOutput;

        let (_tmp, checkpoint_store) = setup();
        let app_root = tempfile::tempdir().expect("app root");
        let lifecycle_store =
            LifecycleStore::new(ProductAppPaths::new(app_root.path().join(".aria")));
        let project_id = "project_0001";
        let issue_id = "issue_0001";
        let repository_id = "repo_0001";

        let story = lifecycle_store
            .create_story_spec(CreateStorySpecInput {
                project_id: project_id.to_string(),
                issue_id: issue_id.to_string(),
                repository_id: repository_id.to_string(),
                title: "Story".to_string(),
            })
            .unwrap();
        let design = lifecycle_store
            .create_design_spec(CreateDesignSpecInput {
                project_id: project_id.to_string(),
                issue_id: issue_id.to_string(),
                story_spec_ids: vec![story.id.clone()],
                title: "Design".to_string(),
            })
            .unwrap();

        let plan = lifecycle_store
            .create_issue_work_item_plan(CreateIssueWorkItemPlanInput {
                id: Some("issue_work_item_plan_0001".to_string()),
                project_id: project_id.to_string(),
                issue_id: issue_id.to_string(),
                source_story_spec_ids: vec![story.id.clone()],
                source_design_spec_ids: vec![design.id.clone()],
                options: IssueWorkItemPlanOptions {
                    include_integration_tests: false,
                    include_e2e_tests: false,
                    force_frontend_backend_split: false,
                    require_execution_plan_confirm: false,
                },
                status: IssueWorkItemPlanStatus::Draft,
                work_item_ids: vec![],
                repository_profile_ref: None,
                verification_plan_ids: vec![],
                dependency_graph: vec![],
                created_from_provider_run: None,
                validator_findings: vec![],
            })
            .unwrap();

        let session_record = lifecycle_store
            .create_workspace_session(CreateWorkspaceSessionInput {
                project_id: project_id.to_string(),
                issue_id: issue_id.to_string(),
                entity_id: plan.id.clone(),
                workspace_type: WorkspaceType::WorkItemPlan,
                author_provider: ProviderName::Codex,
                reviewer_provider: ProviderName::ClaudeCode,
                review_rounds: 0,
                superpowers_enabled: false,
                openspec_enabled: false,
            })
            .unwrap();

        let session = WorkspaceSession::from_record(session_record);
        let (tx, _rx) = mpsc::channel(64);
        let mut engine =
            WorkspaceEngine::new_persistent(checkpoint_store, lifecycle_store.clone(), tx, session);

        fn make_error_output(story_id: &str, design_id: &str) -> WorkItemSplitProviderOutput {
            let now = chrono::Utc::now().to_rfc3339();
            let repository_profile = RepositoryProfile {
                id: "repo_profile_0001".to_string(),
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                repository_id: "repo_0001".to_string(),
                provider_run_ref: None,
                languages: vec!["rust".to_string()],
                frameworks: vec![],
                package_managers: vec!["cargo".to_string()],
                test_frameworks: vec![],
                build_systems: vec!["cargo".to_string()],
                verification_capabilities: vec![],
                detected_layers: vec!["backend".to_string()],
                split_recommendation: "backend_only".to_string(),
                confidence: RepositoryProfileConfidence::High,
                uncertainties: vec![],
                created_at: now.clone(),
                updated_at: now.clone(),
            };

            let work_items = vec![LifecycleWorkItemRecord {
                id: "wi_err_001".to_string(),
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                repository_id: "repo_0001".to_string(),
                story_spec_ids: vec![story_id.to_string()],
                design_spec_ids: vec![design_id.to_string()],
                title: "Error work item".to_string(),
                plan_status: WorkItemPlanStatus::Draft,
                execution_status: WorkItemStatus::Pending,
                worktree_path: None,
                work_item_set_id: None,
                kind: WorkItemKind::Backend,
                sequence_hint: None,
                depends_on: vec![],
                exclusive_write_scopes: vec![], // empty -> validator error
                forbidden_write_scopes: vec![],
                context_budget: WorkItemContextBudget::default(),
                required_handoff_from: vec![],
                verification_plan_ref: Some("vp_err_001".to_string()),
                require_execution_plan_confirm: false,
                execution_plan_status: WorkItemExecutionPlanStatus::NotStarted,
                handoff_summary_ref: None,
                completion_commit: None,
                completion_diff_summary_ref: None,
                created_at: now.clone(),
                updated_at: now.clone(),
            }];

            let verification_plans = vec![VerificationPlan {
                id: "vp_err_001".to_string(),
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                work_item_id: "wi_err_001".to_string(),
                repository_profile_ref: Some("repo_profile_0001".to_string()),
                provider_run_ref: None,
                scope: VerificationScope::Unit,
                commands: vec![VerificationCommand {
                    id: "cmd_001".to_string(),
                    label: "cargo test".to_string(),
                    command: "cargo test".to_string(),
                    cwd: "".to_string(),
                    purpose: "unit tests".to_string(),
                    required: true,
                    timeout_seconds: 120,
                    source: VerificationCommandSource::Provider,
                    safety: VerificationCommandSafety::Approved,
                }],
                manual_checks: vec![VerificationManualCheck {
                    id: "check_001".to_string(),
                    label: "manual".to_string(),
                    instructions: "check".to_string(),
                    required: false,
                }],
                required_gates: vec!["cmd_001".to_string()],
                risk_notes: vec![],
                confidence: RepositoryProfileConfidence::High,
                fallback_policy: VerificationFallbackPolicy::ManualGate,
                created_at: now.clone(),
                updated_at: now.clone(),
            }];

            let plan_record = crate::product::models::IssueWorkItemPlan {
                id: "issue_work_item_plan_0001".to_string(),
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                source_story_spec_ids: vec![story_id.to_string()],
                source_design_spec_ids: vec![design_id.to_string()],
                options: IssueWorkItemPlanOptions {
                    include_integration_tests: false,
                    include_e2e_tests: false,
                    force_frontend_backend_split: false,
                    require_execution_plan_confirm: false,
                },
                status: IssueWorkItemPlanStatus::Draft,
                work_item_ids: vec!["wi_err_001".to_string()],
                repository_profile_ref: Some("repo_profile_0001".to_string()),
                verification_plan_ids: vec!["vp_err_001".to_string()],
                dependency_graph: vec![],
                created_from_provider_run: None,
                validator_findings: vec![],
                review_summary: None,
                created_at: now.clone(),
                updated_at: now.clone(),
            };

            WorkItemSplitProviderOutput {
                repository_profile,
                plan: plan_record,
                work_items,
                verification_plans,
            }
        }

        // 第一次：validate 报错 -> AutoRevision
        let outcome = engine
            .complete_work_item_plan_author(make_error_output(&story.id, &design.id))
            .await
            .expect("first call should succeed");
        assert!(
            matches!(outcome, WorkItemPlanAuthorOutcome::AutoRevision { .. }),
            "first error should trigger auto revision, got {outcome:?}"
        );
        assert_eq!(engine.work_item_plan_author_retry_count, 1);

        // 第二次：validate 仍报错 -> AutoRevision
        let outcome = engine
            .complete_work_item_plan_author(make_error_output(&story.id, &design.id))
            .await
            .expect("second call should succeed");
        assert!(
            matches!(outcome, WorkItemPlanAuthorOutcome::AutoRevision { .. }),
            "second error should trigger auto revision, got {outcome:?}"
        );
        assert_eq!(engine.work_item_plan_author_retry_count, 2);

        // 第三次：超过阈值 -> HumanConfirm
        let outcome = engine
            .complete_work_item_plan_author(make_error_output(&story.id, &design.id))
            .await
            .expect("third call should succeed");
        assert!(
            matches!(outcome, WorkItemPlanAuthorOutcome::HumanConfirm { .. }),
            "third error should escalate to human confirm, got {outcome:?}"
        );
        assert_eq!(engine.session().stage, WorkspaceStage::HumanConfirm);

        // 验证 persisted plan 记录了 error finding
        let persisted = lifecycle_store
            .get_issue_work_item_plan(project_id, issue_id, &plan.id)
            .unwrap();
        assert!(
            persisted
                .validator_findings
                .iter()
                .any(|f| f.severity == WorkItemSplitFindingSeverity::Error)
        );
    }

    #[test]
    fn build_work_item_plan_review_input_includes_trimmed_candidate_fields() {
        let (_tmp, _checkpoint_store, _lifecycle, _plan_id, engine) =
            make_work_item_plan_engine_with_draft_candidate("sess_wip_review_prompt");

        let input = engine
            .build_work_item_plan_review_input()
            .expect("review input");

        assert_eq!(input.role, AdapterRole::Reviewer);
        assert!(
            input.prompt.contains("Work Item Plan"),
            "prompt 应含 workspace 类型标题"
        );
        assert!(input.prompt.contains("work_item_0001"));
        assert!(input.prompt.contains("work_item_0002"));
        assert!(input.prompt.contains("depends_on"));
        assert!(input.prompt.contains("exclusive_write_scopes"));
        assert!(input.prompt.contains("verification_plan_ref"));
        assert!(input.prompt.contains("dependency_graph"));
        assert!(
            input.prompt.contains("high"),
            "prompt 应含 repository_profile confidence"
        );
        assert!(input.prompt.contains("backend"));
        assert!(
            !input.prompt.contains("frameworks"),
            "prompt 不应含 repository_profile 的 frameworks 字段"
        );
        assert!(
            input
                .prompt
                .contains("\"verdict\":\"pass|revise|needs_human\"")
        );
        assert!(input.prompt.contains("\"summary\""));
        assert!(input.prompt.contains("\"findings\""));
    }

    #[test]
    fn build_review_input_routes_work_item_plan_to_dedicated_helper() {
        let (_tmp, _checkpoint_store, _lifecycle, _plan_id, engine) =
            make_work_item_plan_engine_with_draft_candidate("sess_wip_review_route");

        let input = engine.build_review_input().expect("review input");

        assert_eq!(input.role, AdapterRole::Reviewer);
        assert!(input.prompt.contains("work_item_0001"));
        assert!(
            !input.prompt.contains("当前已提取 Artifact Markdown"),
            "WorkItemPlan 分支不应走 Story/Design 的 artifact markdown 提示"
        );
    }

    #[test]
    fn build_work_item_plan_review_input_returns_error_when_lifecycle_store_missing() {
        let (_tmp, _checkpoint_store, _lifecycle, _plan_id, mut engine) =
            make_work_item_plan_engine_with_draft_candidate("sess_wip_review_no_lifecycle");
        engine.lifecycle_store = None;

        let result = engine.build_work_item_plan_review_input();

        assert!(result.is_err());
        let error = result.unwrap_err();
        assert!(
            error.contains("lifecycle_store unavailable"),
            "错误信息应提示 lifecycle_store 不可用，实际为: {error}"
        );
    }

    #[tokio::test]
    async fn begin_work_item_plan_author_run_creates_standard_author_node() {
        let (_tmp, _checkpoint_store, _lifecycle, _plan_id, mut engine) =
            make_work_item_plan_engine_with_draft_candidate("sess_wip_author_stream_node");

        let node_id = engine.begin_work_item_plan_author_run().await;
        let node = engine
            .timeline_nodes
            .iter()
            .find(|node| node.node_id == node_id)
            .expect("author node");

        assert_eq!(node.node_type, TimelineNodeType::AuthorRun);
        assert_eq!(node.stage, WsWorkspaceStage::Running);
        assert_eq!(node.agent, Some(ProviderName::ClaudeCode));
        assert_eq!(node.status, TimelineNodeStatus::Active);
        assert_eq!(node.title, "Work Item Plan 生成");
        assert_eq!(
            engine.active_timeline_node_id().as_deref(),
            Some(node_id.as_str())
        );
    }

    #[tokio::test]
    async fn drive_work_item_plan_provider_session_returns_output_and_persists_stream() {
        let (_tmp, _checkpoint_store, lifecycle, _plan_id, mut engine) =
            make_work_item_plan_engine_with_draft_candidate("sess_wip_stream_collector");
        engine.session.session_id = lifecycle
            .list_workspace_sessions("project_0001", "issue_0001")
            .expect("workspace sessions")
            .into_iter()
            .find(|session| session.workspace_type == WorkspaceType::WorkItemPlan)
            .expect("work item plan session")
            .id;
        let node_id = engine.begin_work_item_plan_author_run().await;
        let (provider_event_tx, provider_event_rx) = mpsc::channel(8);
        let (provider_command_tx, _provider_command_rx) = mpsc::channel(8);
        provider_event_tx
            .send(ProviderEvent::TextDelta {
                content: "Fake Work Item Plan streaming draft\n".to_string(),
            })
            .await
            .expect("send text delta");
        provider_event_tx
            .send(ProviderEvent::Completed {
                full_output: "Final structured output".to_string(),
                provider_session_id: Some("provider-work-item-plan-author-1".to_string()),
            })
            .await
            .expect("send completed");
        drop(provider_event_tx);
        let mut command_rx = empty_provider_commands();

        let output = engine
            .drive_work_item_plan_provider_session_to_output(
                Ok(ProviderSession {
                    events: provider_event_rx,
                    commands: provider_command_tx,
                }),
                &mut command_rx,
                node_id.clone(),
                ProviderName::ClaudeCode,
            )
            .await
            .expect("collector output");

        assert_eq!(output, "Final structured output");
        let detail = lifecycle
            .load_node_detail(engine.session().session_id.as_str(), &node_id)
            .expect("node detail");
        assert!(
            detail
                .streaming_content
                .contains("Fake Work Item Plan streaming draft")
        );
        assert!(
            engine
                .session()
                .provider_conversations
                .iter()
                .any(|conversation| {
                    conversation.role == ProviderConversationRole::Author
                        && conversation.provider == ProviderName::ClaudeCode
                        && conversation.provider_session_id == "provider-work-item-plan-author-1"
                        && conversation.last_node_id.as_deref() == Some(node_id.as_str())
                })
        );
    }

    #[tokio::test]
    async fn drive_work_item_plan_provider_session_hides_structured_output_from_stream() {
        let (_tmp, _checkpoint_store, lifecycle, _plan_id, mut engine) =
            make_work_item_plan_engine_with_draft_candidate("sess_wip_stream_filter");
        engine.session.session_id = lifecycle
            .list_workspace_sessions("project_0001", "issue_0001")
            .expect("workspace sessions")
            .into_iter()
            .find(|session| session.workspace_type == WorkspaceType::WorkItemPlan)
            .expect("work item plan session")
            .id;
        let node_id = engine.begin_work_item_plan_author_run().await;
        let (provider_event_tx, provider_event_rx) = mpsc::channel(8);
        let (provider_command_tx, _provider_command_rx) = mpsc::channel(8);
        let full_output = "Readable Work Item Plan draft\n<ARIA_STRUCTURED_OUTPUT>{\"work_items\":[]}</ARIA_STRUCTURED_OUTPUT>".to_string();
        provider_event_tx
            .send(ProviderEvent::TextDelta {
                content: "Readable Work Item Plan draft\n<ARIA_STRUCTURED".to_string(),
            })
            .await
            .expect("send text delta");
        provider_event_tx
            .send(ProviderEvent::TextDelta {
                content: "_OUTPUT>{\"work_items\":[]}</ARIA_STRUCTURED_OUTPUT>".to_string(),
            })
            .await
            .expect("send structured delta");
        provider_event_tx
            .send(ProviderEvent::Completed {
                full_output: full_output.clone(),
                provider_session_id: None,
            })
            .await
            .expect("send completed");
        drop(provider_event_tx);
        let mut command_rx = empty_provider_commands();

        let output = engine
            .drive_work_item_plan_provider_session_to_output(
                Ok(ProviderSession {
                    events: provider_event_rx,
                    commands: provider_command_tx,
                }),
                &mut command_rx,
                node_id.clone(),
                ProviderName::ClaudeCode,
            )
            .await
            .expect("collector output");

        assert_eq!(output, full_output);
        let detail = lifecycle
            .load_node_detail(engine.session().session_id.as_str(), &node_id)
            .expect("node detail");
        assert!(
            detail
                .streaming_content
                .contains("Readable Work Item Plan draft")
        );
        assert!(!detail.streaming_content.contains("ARIA_STRUCTURED_OUTPUT"));
        assert!(!detail.streaming_content.contains("\"work_items\""));
    }

    #[test]
    fn build_work_item_plan_streaming_input_uses_splitter_role() {
        let (_tmp, _checkpoint_store, _lifecycle, _plan_id, engine) =
            make_work_item_plan_engine_with_draft_candidate("sess_wip_splitter_input");

        let input = engine.build_work_item_plan_streaming_input(
            ProviderType::Fake,
            "split prompt".to_string(),
            "/tmp/worktree".to_string(),
        );

        assert_eq!(input.provider_type, ProviderType::Fake);
        assert_eq!(input.role, AdapterRole::WorkItemSplitter);
        assert_eq!(input.prompt, "split prompt");
        assert_eq!(input.working_dir, PathBuf::from("/tmp/worktree"));
        assert_eq!(
            input.workspace_session_id.as_deref(),
            Some(engine.session().session_id.as_str())
        );
        assert_eq!(input.resume_provider_session_id, None);
        assert_eq!(input.permission_mode, ProviderPermissionMode::Supervised);
        assert_eq!(input.timeout_secs, DEFAULT_PROVIDER_TIMEOUT_SECS);
    }

    fn make_work_item_plan_engine_with_draft_candidate(
        session_id: &str,
    ) -> (
        TempDir,
        Arc<CheckpointStore>,
        LifecycleStore,
        String,
        WorkspaceEngine,
    ) {
        let tmp = TempDir::new().unwrap();
        let checkpoint_store = Arc::new(CheckpointStore::new(tmp.path().to_path_buf()));
        let app_paths = ProductAppPaths::new(tmp.path().join(".aria"));
        let lifecycle = LifecycleStore::new(app_paths);

        let project_id = "project_0001";
        let issue_id = "issue_0001";
        let repository_id = "repo_0001";

        let story = lifecycle
            .create_story_spec(CreateStorySpecInput {
                project_id: project_id.to_string(),
                issue_id: issue_id.to_string(),
                repository_id: repository_id.to_string(),
                title: "Story".to_string(),
            })
            .unwrap();
        let design = lifecycle
            .create_design_spec(CreateDesignSpecInput {
                project_id: project_id.to_string(),
                issue_id: issue_id.to_string(),
                story_spec_ids: vec![story.id.clone()],
                title: "Design".to_string(),
            })
            .unwrap();

        let plan = lifecycle
            .create_issue_work_item_plan(CreateIssueWorkItemPlanInput {
                id: Some("issue_work_item_plan_0001".to_string()),
                project_id: project_id.to_string(),
                issue_id: issue_id.to_string(),
                source_story_spec_ids: vec![story.id.clone()],
                source_design_spec_ids: vec![design.id.clone()],
                options: IssueWorkItemPlanOptions {
                    include_integration_tests: false,
                    include_e2e_tests: false,
                    force_frontend_backend_split: false,
                    require_execution_plan_confirm: false,
                },
                status: IssueWorkItemPlanStatus::Draft,
                work_item_ids: vec![],
                repository_profile_ref: None,
                verification_plan_ids: vec![],
                dependency_graph: vec![],
                created_from_provider_run: None,
                validator_findings: vec![],
            })
            .unwrap();

        let profile = lifecycle
            .create_repository_profile(CreateRepositoryProfileInput {
                id: Some("repo_profile_0001".to_string()),
                project_id: project_id.to_string(),
                issue_id: issue_id.to_string(),
                repository_id: repository_id.to_string(),
                provider_run_ref: None,
                languages: vec!["rust".to_string()],
                frameworks: vec!["axum".to_string()],
                package_managers: vec!["cargo".to_string()],
                test_frameworks: vec![],
                build_systems: vec!["cargo".to_string()],
                verification_capabilities: vec![],
                detected_layers: vec!["backend".to_string(), "frontend".to_string()],
                split_recommendation: "frontend_backend".to_string(),
                confidence: RepositoryProfileConfidence::High,
                uncertainties: vec![],
            })
            .unwrap();

        let work_item_1 = lifecycle
            .create_work_item(CreateWorkItemInput {
                id: Some("work_item_0001".to_string()),
                project_id: project_id.to_string(),
                issue_id: issue_id.to_string(),
                repository_id: repository_id.to_string(),
                story_spec_ids: vec![story.id.clone()],
                design_spec_ids: vec![design.id.clone()],
                title: "Backend work item".to_string(),
                work_item_set_id: None,
                kind: WorkItemKind::Backend,
                sequence_hint: None,
                depends_on: vec![],
                exclusive_write_scopes: vec!["src/backend.rs".to_string()],
                forbidden_write_scopes: vec![],
                context_budget: WorkItemContextBudget::default(),
                required_handoff_from: vec![],
                verification_plan_ref: Some("vp_0001".to_string()),
                require_execution_plan_confirm: false,
                plan_status: WorkItemPlanStatus::Draft,
            })
            .unwrap();
        let work_item_2 = lifecycle
            .create_work_item(CreateWorkItemInput {
                id: Some("work_item_0002".to_string()),
                project_id: project_id.to_string(),
                issue_id: issue_id.to_string(),
                repository_id: repository_id.to_string(),
                story_spec_ids: vec![story.id.clone()],
                design_spec_ids: vec![design.id.clone()],
                title: "Frontend work item".to_string(),
                work_item_set_id: None,
                kind: WorkItemKind::Frontend,
                sequence_hint: None,
                depends_on: vec!["work_item_0001".to_string()],
                exclusive_write_scopes: vec!["src/frontend.rs".to_string()],
                forbidden_write_scopes: vec![],
                context_budget: WorkItemContextBudget::default(),
                required_handoff_from: vec![],
                verification_plan_ref: Some("vp_0002".to_string()),
                require_execution_plan_confirm: false,
                plan_status: WorkItemPlanStatus::Draft,
            })
            .unwrap();

        let vp_1 = lifecycle
            .create_verification_plan(CreateVerificationPlanInput {
                id: Some("vp_0001".to_string()),
                project_id: project_id.to_string(),
                issue_id: issue_id.to_string(),
                work_item_id: work_item_1.id.clone(),
                repository_profile_ref: Some(profile.id.clone()),
                provider_run_ref: None,
                scope: VerificationScope::Unit,
                commands: vec![VerificationCommand {
                    id: "cmd_001".to_string(),
                    label: "cargo test".to_string(),
                    command: "cargo test".to_string(),
                    cwd: "".to_string(),
                    purpose: "unit tests".to_string(),
                    required: true,
                    timeout_seconds: 120,
                    source: VerificationCommandSource::Provider,
                    safety: VerificationCommandSafety::Approved,
                }],
                manual_checks: vec![VerificationManualCheck {
                    id: "check_001".to_string(),
                    label: "manual".to_string(),
                    instructions: "check".to_string(),
                    required: false,
                }],
                required_gates: vec!["cmd_001".to_string()],
                risk_notes: vec![],
                confidence: RepositoryProfileConfidence::High,
                fallback_policy: VerificationFallbackPolicy::ManualGate,
            })
            .unwrap();
        let vp_2 = lifecycle
            .create_verification_plan(CreateVerificationPlanInput {
                id: Some("vp_0002".to_string()),
                project_id: project_id.to_string(),
                issue_id: issue_id.to_string(),
                work_item_id: work_item_2.id.clone(),
                repository_profile_ref: Some(profile.id.clone()),
                provider_run_ref: None,
                scope: VerificationScope::Unit,
                commands: vec![VerificationCommand {
                    id: "cmd_002".to_string(),
                    label: "cargo test".to_string(),
                    command: "cargo test".to_string(),
                    cwd: "".to_string(),
                    purpose: "unit tests".to_string(),
                    required: true,
                    timeout_seconds: 120,
                    source: VerificationCommandSource::Provider,
                    safety: VerificationCommandSafety::Approved,
                }],
                manual_checks: vec![],
                required_gates: vec![],
                risk_notes: vec![],
                confidence: RepositoryProfileConfidence::High,
                fallback_policy: VerificationFallbackPolicy::ManualGate,
            })
            .unwrap();

        lifecycle
            .update_issue_work_item_plan(
                project_id,
                issue_id,
                &plan.id,
                IssueWorkItemPlanUpdate {
                    work_item_ids: vec![work_item_1.id.clone(), work_item_2.id.clone()],
                    verification_plan_ids: vec![vp_1.id.clone(), vp_2.id.clone()],
                    repository_profile_ref: Some(profile.id.clone()),
                    dependency_graph: vec![IssueWorkItemDependencyEdge {
                        from_work_item_id: work_item_1.id.clone(),
                        to_work_item_id: work_item_2.id.clone(),
                    }],
                    created_from_provider_run: None,
                    validator_findings: vec![WorkItemSplitFinding {
                        severity: WorkItemSplitFindingSeverity::Warning,
                        code: "W001".to_string(),
                        message: "scope overlap risk".to_string(),
                        work_item_ids: vec![work_item_1.id.clone()],
                    }],
                },
            )
            .unwrap();

        let session_record = lifecycle
            .create_workspace_session(CreateWorkspaceSessionInput {
                project_id: project_id.to_string(),
                issue_id: issue_id.to_string(),
                entity_id: plan.id.clone(),
                workspace_type: WorkspaceType::WorkItemPlan,
                author_provider: ProviderName::ClaudeCode,
                reviewer_provider: ProviderName::Codex,
                review_rounds: 1,
                superpowers_enabled: false,
                openspec_enabled: false,
            })
            .unwrap();

        let session = WorkspaceSession::from_record(session_record);
        let (event_tx, _event_rx) = mpsc::channel(64);
        let mut engine = WorkspaceEngine::new_persistent(
            checkpoint_store.clone(),
            lifecycle.clone(),
            event_tx,
            session,
        );
        engine.session.session_id = session_id.to_string();
        engine.session.stage = WorkspaceStage::AuthorConfirm;
        engine.session.reviewer_provider = Some(ProviderName::Codex);

        (tmp, checkpoint_store, lifecycle, plan.id, engine)
    }
}
