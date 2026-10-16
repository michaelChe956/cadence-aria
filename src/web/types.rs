use serde::{Deserialize, Serialize};
use sha2::Digest;

use serde_json::Value;
use std::path::PathBuf;

use crate::product::coding_models::{
    CodeReviewReport, CodingChoiceGate, CodingGateRequired, CodingRoleRunSnapshot,
    CodingTimelineNode, GroupFinalReadinessSnapshot, GroupReviewArtifactRef, InternalPrReview,
    ReviewRequest, WorkItemExecutionPlan,
};
use crate::product::logical_codebase::LogicalRepositoryId;
use crate::product::work_item_plan_policy::RunPolicy;
use crate::web::coding_final_confirm_info::CodingFinalConfirmInfoDto;
use crate::web::plan_confirmed_info::PlanConfirmedInfoDto;
use crate::web::workspace_ws_types::ProviderConfigSnapshot;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CreateTaskRequest {
    pub request_text: String,
    pub change_id: String,
    pub policy_preset: String,
    pub provider_mode: String,
    pub timeout_secs: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CreateTaskResponse {
    pub task_id: String,
    pub session_id: String,
    pub change_id: String,
    pub phase: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PendingProviderStepDto {
    pub node_id: String,
    pub provider_type: String,
    pub runtime_role: String,
    pub adapter_role: String,
    pub prompt: String,
    pub input_summary: Value,
    pub canonical_input_refs: Vec<String>,
    pub context_files: Vec<String>,
    pub output_schema: String,
    pub allowed_write_scope: Vec<String>,
    pub forbidden_actions: Vec<String>,
    pub verification_commands: Vec<String>,
    pub checkpoint_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum AdvanceTaskResponse {
    Advanced {
        projection_version: u64,
    },
    PausedForApproval {
        pending_step: Box<PendingProviderStepDto>,
    },
    Completed {
        projection_version: u64,
    },
}

impl AdvanceTaskResponse {
    pub fn expect_pending_step(self) -> Option<PendingProviderStepDto> {
        match self {
            AdvanceTaskResponse::PausedForApproval { pending_step } => Some(*pending_step),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ConfirmTaskRequest {
    pub checkpoint_id: String,
    pub prompt: String,
    pub policy_override: Option<String>,
    pub provider_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ConfirmTaskResponse {
    pub status: String,
    pub node_id: String,
    pub turn_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RollbackPreviewRequest {
    pub checkpoint_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RollbackPreviewResponse {
    pub checkpoint_id: String,
    pub git_head: Option<String>,
    pub dirty: bool,
    pub turns_to_drop: usize,
    pub node_runs_to_drop: usize,
    pub provider_runs_to_drop: usize,
    pub artifacts_to_drop: usize,
    pub files_may_change: Vec<String>,
}

impl From<crate::interactive::checkpoint::RollbackPreview> for RollbackPreviewResponse {
    fn from(preview: crate::interactive::checkpoint::RollbackPreview) -> Self {
        Self {
            checkpoint_id: preview.checkpoint_id,
            git_head: preview.git_head,
            dirty: preview.dirty,
            turns_to_drop: preview.turns_to_drop,
            node_runs_to_drop: preview.node_runs_to_drop,
            provider_runs_to_drop: preview.provider_runs_to_drop,
            artifacts_to_drop: preview.artifacts_to_drop,
            files_may_change: preview.files_may_change,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RollbackRequest {
    pub checkpoint_id: String,
    pub force_when_dirty: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct IssueRollbackPreviewRequest {
    pub execution_record_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct IssueRollbackRequest {
    pub execution_record_id: String,
    pub force_when_dirty: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RollbackResponse {
    pub status: String,
    pub checkpoint_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WebEvent {
    pub cursor: u64,
    pub event_type: String,
    pub task_id: Option<String>,
    pub payload: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct TaskListResponse {
    pub tasks: Vec<TaskListItem>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct TaskListItem {
    pub task_id: String,
    pub change_id: Option<String>,
    pub phase: Option<String>,
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ArtifactContentResponse {
    pub artifact_ref: String,
    pub artifact_kind: String,
    pub producer_node: Option<String>,
    pub path: String,
    pub content_type: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct FileContentResponse {
    pub path: String,
    pub content_type: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct FileDiffResponse {
    pub base_checkpoint: String,
    pub path: String,
    pub diff: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CodingAttemptDiffResponse {
    pub attempt_id: String,
    pub base_branch: String,
    pub worktree_path: PathBuf,
    pub diff: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ProviderInputContentResponse {
    pub input_ref: String,
    pub content_type: String,
    pub content: String,
    pub redaction_applied: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ProviderInputPrepared {
    pub node_id: String,
    pub input_ref: String,
    pub input_summary: Value,
    pub redaction_applied: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ProviderOutputChunk {
    pub node_id: String,
    pub provider_run_id: String,
    pub stream: String,
    pub text: String,
    pub structured_output: Option<Value>,
    pub manual_gate: Option<String>,
    pub retry_attempt: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct StopTaskResponse {
    pub status: String,
    pub task_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkspaceListResponse {
    pub workspaces: Vec<WorkspaceDto>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkspaceDto {
    pub workspace_id: String,
    pub name: String,
    pub path: String,
    pub default_policy_preset: String,
    pub default_provider_mode: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CreateWorkspaceRequest {
    pub name: String,
    pub path: String,
    pub default_policy_preset: Option<String>,
    pub default_provider_mode: Option<String>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ProjectDto {
    pub project_id: String,
    pub name: String,
    pub description: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub last_opened_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ProjectListResponse {
    pub projects: Vec<ProjectDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CreateProjectRequest {
    pub name: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RepositoryDto {
    pub repository_id: String,
    pub project_id: String,
    pub name: String,
    pub path: String,
    pub repo_hash: String,
    pub runtime_root: String,
    pub default_policy_preset: String,
    pub default_provider_mode: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RepositoryListResponse {
    pub repositories: Vec<RepositoryDto>,
}

/// 仓库本地分支列表（REQ-PIB-01）：仅 refs/heads（不隐式 fetch）；
/// default_branch 为服务端默认链 main→master→null（UI 不自行推导）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RepositoryBranchListResponse {
    pub branches: Vec<String>,
    pub default_branch: Option<String>,
}

/// Task R2（v1.3 §4）：统一 codebases 混合列表条目。单仓条目是 repos.json 的呈现层
/// 投影（数据不动），逻辑条目来自 LogicalCodebaseStore。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CodebaseSummaryDto {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub repository_id: Option<String>,
    pub logical_codebase_id: Option<String>,
    pub member_count: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CodebaseListResponse {
    pub codebases: Vec<CodebaseSummaryDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CreateLogicalCodebaseRequest {
    pub name: String,
    pub aggregate_root: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct LogicalCodebaseDto {
    pub id: String,
    pub name: String,
    pub aggregate_root: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct LogicalCodebaseMemberDto {
    pub logical_repository_id: String,
    pub physical_repository_id: String,
    pub alias: String,
    pub status: String,
    pub ordinal: u32,
    pub created_at: String,
    pub updated_at: String,
}

/// LC 详情：record + 成员列表 + 初始化/索引状态汇总（manifest 待首批登记创建 →
/// manifest_present=false 即未初始化；active_aggregate_index_id 汇总索引状态）。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct LogicalCodebaseDetailDto {
    pub id: String,
    pub name: String,
    pub aggregate_root: String,
    pub created_at: String,
    pub manifest_present: bool,
    pub membership_revision: Option<u64>,
    pub active_aggregate_index_id: Option<String>,
    pub member_count: usize,
    pub members: Vec<LogicalCodebaseMemberDto>,
}

mod repository_initialization;
pub use repository_initialization::{
    RepositoryInitializationOperationDto, RepositoryInitializationResultDto,
    RepositoryInitializationStepDto, RepositoryRegistrationInitializationDto,
    ResumeRepositoryInitializationRequest,
};

mod aggregate_initialization;
pub use aggregate_initialization::{
    AggregateCancellationDto, AggregateInitializationOperationDto, AggregateInitializationStepDto,
    AggregateMemberProjectionDto, CancelAggregateInitializationRequest,
    CreateAggregateInitializationRequest,
};

mod logical_codebase_registration;
pub use logical_codebase_registration::{
    RegistrationBatchDto, RegistrationBatchItemDto, RegistrationPreflightItemDto,
    RegistrationPreflightRequest, RegistrationPreflightResponse, RegistrationSubmitRequest,
};

mod logical_codebase_bootstrap;
pub use logical_codebase_bootstrap::{
    BootstrapActionRequestDto, BootstrapActionResultDto, BootstrapCheckpointDto,
    BootstrapFailureDto, BootstrapPolicyReferenceDto, BootstrapStepProjectionDto,
    LogicalCodebaseBootstrapNoticeDto, LogicalCodebaseBootstrapProjectionDto,
};

mod identity_repair;
pub use identity_repair::{
    IdentityJournalDiagnosticDto, IdentityMappingSubmissionDto, IdentityRepairActionRequestDto,
    RepositoryIdentityMappingDto,
};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CreateRepositoryRequest {
    pub name: String,
    pub path: String,
    pub default_policy_preset: Option<String>,
    pub default_provider_mode: Option<String>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ProductIssueDto {
    pub issue_id: String,
    pub project_id: String,
    pub repo_id: Option<String>,
    /// issue 基准分支（REQ-PIB-01）：创建时锁定；null=存量记录（默认链解析）。
    pub base_branch: Option<String>,
    pub workspace_id: Option<String>,
    pub task_id: Option<String>,
    pub session_id: Option<String>,
    pub title: String,
    pub description: Option<String>,
    pub change_id: String,
    pub phase: String,
    pub status: String,
    pub active_binding_id: Option<String>,
    pub artifacts: Vec<ProductIssueArtifactDto>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ProductIssueArtifactDto {
    pub artifact_ref: String,
    pub artifact_kind: String,
    pub producer_node: Option<String>,
    pub path: String,
    pub summary: String,
    pub stage: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ProductIssueListResponse {
    pub issues: Vec<ProductIssueDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CreateProductIssueRequest {
    pub title: String,
    pub description: Option<String>,
    pub change_id: Option<String>,
    pub repository_id: Option<String>,
    /// 基准分支（REQ-PIB-01）：单仓 issue 创建时可选显式指定；缺省按默认链
    /// main→master 解析，皆无且未指定即 422。逻辑代码库 issue 忽略本字段。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_branch: Option<String>,
    /// v1.3：逻辑代码库归属；Some 时创建逻辑 issue（repository_id 须为其 active primary）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logical_codebase_id: Option<String>,
    /// REQ-MRE-01：LC 成员勾选集（logical_repository_id 复数）。非空 → selection 以
    /// Explicit 策略落盘（include=focus=勾选集，授权上界=勾选集原样）；缺省/空 →
    /// 维持存量 AllMembers（存量兼容）。仅逻辑代码库 issue 消费本字段。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub focus_repository_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct IssueLifecycleResponse {
    pub issue: ProductIssueDto,
    pub story_specs: Vec<StorySpecDto>,
    pub design_specs: Vec<DesignSpecDto>,
    pub work_item_plans: Vec<IssueWorkItemPlanDetailDto>,
    pub work_items: Vec<LifecycleWorkItemDto>,
    /// REQ-TGT-05：WorkItem 按 `target_repository_id` 分组的聚合视图（向后兼容，
    /// 保留扁平 `work_items` 不变）。
    pub work_item_repository_groups: Vec<WorkItemRepositoryGroupDto>,
    pub workspace_sessions: Vec<WorkspaceSessionSummaryDto>,
    pub coding_attempts: Vec<CodingAttemptDto>,
    /// Issue 级交付状态聚合（`all_pushed` | `partial` | `none`）。向后兼容：
    /// 旧客户端/旧响应不携带该字段时反序列化为空聚合。
    #[serde(default)]
    pub delivery_summary: IssueDeliverySummaryDto,
    /// P1（REQ-WIGA-07）：durable publication/compile + Confirmed 派生的只读
    /// 确认信息（0 或 1 条）。additive：旧客户端反序列化缺省为空。
    #[serde(default)]
    pub plan_confirmed_info: Vec<PlanConfirmedInfoDto>,
    /// P2（REQ-WIGA-07/R5）：enrolled 已认领 group attempt 的 FinalConfirm
    /// 等待/已确认只读信息。additive：旧客户端反序列化缺省为空。
    #[serde(default)]
    pub coding_final_confirm_info: Vec<CodingFinalConfirmInfoDto>,
    /// P3（REQ-WIGA-07）：issue 级有界近期完成目录（默认空；仅在请求携带
    /// recent_since 时计算）。additive：旧客户端反序列化缺省为空。
    #[serde(default)]
    pub recent_completion_info: Vec<crate::web::recent_completion_info::RecentCompletionInfoDto>,
    /// C1（enrollment-recovery-surface Task 9）：durable 恢复等待项投影
    /// （孤儿候选/lease 三态/Failed advance/intent 停等/换代历史）。
    /// additive：旧客户端反序列化缺省为空。
    #[serde(default)]
    pub c1_waiting_items: Vec<crate::web::plan_confirmed_info::C1WaitingItemDto>,
    /// P4-c（REQ-CRO-12 / 契约 A1）：durable 事实派生的 coding 待处理事项
    /// 投影（无 watched/recent 过滤，全局 cockpit 消费）。additive：旧
    /// 客户端反序列化缺省为空。
    #[serde(default)]
    pub coding_attention_items: Vec<crate::web::coding_attention_info::CodingAttentionItemDto>,
}

#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct DeliveryEntryDto {
    pub repository_name: String,
    pub work_item_id: String,
    /// `None` 表示该 Work Item 没有任何 attempt。
    pub attempt_status: Option<String>,
    pub branch_name: Option<String>,
    pub commit_sha: Option<String>,
    /// `None` 表示无 ReviewRequest。
    pub push_status: Option<String>,
    pub push_error: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct IssueDeliverySummaryDto {
    pub project_id: String,
    pub issue_id: String,
    pub entries: Vec<DeliveryEntryDto>,
    /// "all_pushed" | "partial" | "none"
    pub overall: String,
}

/// REQ-MTG-04（WP3）：plan 级 group 聚合只读投影的单 target 条目（serde
/// snake_case）。`None` 字段表示该 target 无对应事实（无 attempt 字段值/
/// 无 ReviewRequest）。
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PlanTargetEntryDto {
    pub target_repository_id: String,
    pub repository_name: String,
    pub attempt_id: Option<String>,
    pub attempt_status: Option<String>,
    pub stage: Option<String>,
    pub branch_name: Option<String>,
    pub head_commit: Option<String>,
    pub push_status: Option<String>,
    pub review_request_id: Option<String>,
    /// 失败/阻塞原因（只呈现不判定）：manual_recovery_reason → push_error →
    /// 失败态 status 文本。
    pub blocked_reason: Option<String>,
}

/// REQ-MTG-04（WP3）：plan 级三值聚合终态投影。`overall` 取值
/// `"all_delivered" | "partial" | "not_started"`——不复用 issue 级 `"none"`
/// （plan 级未启=无 target-attempt 或全部未达 provider 启动，k3 §2.3 口径差异）。
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PlanGroupProjectionDto {
    pub plan_id: String,
    pub entries: Vec<PlanTargetEntryDto>,
    pub overall: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ArtifactVersionDto {
    pub version: u32,
    pub markdown: String,
    pub generated_by: String,
    pub reviewed_by: Option<String>,
    pub review_verdict: Option<String>,
    pub confirmed_by: Option<String>,
    pub created_at: String,
    pub source_node_id: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct StorySpecDto {
    pub story_spec_id: String,
    pub issue_id: String,
    pub repository_id: String,
    pub title: String,
    pub current_version: Option<u32>,
    pub current_markdown_preview: Option<String>,
    pub confirmation_status: String,
    pub artifact_versions: Vec<ArtifactVersionDto>,
    /// F-26b（additive）：workspace timeline reviewer 节点证据投影
    /// （"running"/"completed"），零证据缺省 None 兼容旧响应。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_status: Option<String>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct DesignSpecDto {
    pub design_spec_id: String,
    pub issue_id: String,
    pub story_spec_ids: Vec<String>,
    pub title: String,
    pub current_version: Option<u32>,
    pub current_markdown_preview: Option<String>,
    pub confirmation_status: String,
    pub artifact_versions: Vec<ArtifactVersionDto>,
    /// F-26b（additive）：workspace timeline reviewer 节点证据投影
    /// （"running"/"completed"），零证据缺省 None 兼容旧响应。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_status: Option<String>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct LifecycleWorkItemDto {
    pub work_item_id: String,
    pub issue_id: String,
    pub repository_id: String,
    pub story_spec_ids: Vec<String>,
    pub design_spec_ids: Vec<String>,
    pub title: String,
    pub plan_status: String,
    pub execution_status: String,
    pub latest_attempt: Option<CodingAttemptDto>,
    pub artifact_versions: Vec<ArtifactVersionDto>,
    pub work_item_set_id: Option<String>,
    pub source_work_item_plan_id: Option<String>,
    pub source_outline_id: Option<String>,
    pub source_draft_id: Option<String>,
    pub planned_implementation_context: Option<String>,
    pub kind: String,
    pub sequence_hint: Option<u32>,
    pub depends_on: Vec<String>,
    pub exclusive_write_scopes: Vec<String>,
    pub forbidden_write_scopes: Vec<String>,
    pub context_budget: WorkItemContextBudgetDto,
    pub verification_plan_ref: Option<String>,
    pub require_execution_plan_confirm: bool,
    pub execution_plan_status: String,
    pub completion_commit: Option<String>,
    pub completion_diff_summary_ref: Option<String>,
}

/// REQ-TGT-05：单个仓库分组。`target_repository_id == None` 表示遗留/未指定仓库，
/// `compatibility_projection` 置 true。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkItemRepositoryGroupDto {
    pub target_repository_id: Option<LogicalRepositoryId>,
    pub alias: String,
    pub status: String,
    pub compatibility_projection: bool,
    pub items: Vec<LifecycleWorkItemDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkItemContextBudgetDto {
    pub target_context_k: String,
    pub max_summary_chars: usize,
    pub max_code_context_chars: usize,
    pub max_context_file_refs: usize,
    pub max_traceability_refs: usize,
}

impl Default for WorkItemContextBudgetDto {
    fn default() -> Self {
        Self {
            target_context_k: "30-50".to_string(),
            max_summary_chars: 20_000,
            max_code_context_chars: 30_000,
            max_context_file_refs: 80,
            max_traceability_refs: 40,
        }
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CodingAttemptDto {
    pub attempt_id: String,
    pub project_id: String,
    pub issue_id: String,
    pub work_item_id: String,
    pub attempt_scope: String,
    pub work_item_group_id: Option<String>,
    pub current_work_item_id: Option<String>,
    pub active_unit_id: Option<String>,
    pub attempt_no: u32,
    pub status: String,
    pub stage: String,
    pub manual_recovery_reason: Option<String>,
    pub branch_name: String,
    pub base_branch: String,
    pub worktree_path: Option<String>,
    pub rework_count: u32,
    pub head_commit: Option<String>,
    pub push_status: Option<String>,
    pub review_request_url: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    /// 终态回填时刻（Completed/Failed/Aborted 转换处落盘）。非终态与
    /// 缺失该字段的旧记录（历史只读，不回写）投影为 None。
    pub completed_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CodingExecutionUnitDto {
    pub unit_id: String,
    pub logical_work_item_id: String,
    pub work_item_revision_id: String,
    pub dependency_logical_work_item_ids: Vec<String>,
    pub order_index: u32,
    pub status: String,
    pub summary: Option<String>,
    pub latest_handoff_revision_id: Option<String>,
    pub completion_commit: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkItemCodingProgressDto {
    pub logical_work_item_id: String,
    pub unit_id: String,
    pub status: String,
    pub stage: Option<String>,
    pub current_commit: Option<String>,
    pub final_commit: Option<String>,
    pub code_review: Option<CodeReviewReport>,
    pub handoff_revision_id: Option<String>,
    pub failure_or_blocked_reason: Option<String>,
    pub plan_revision_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct GroupCodingProgressDto {
    pub total: usize,
    pub pending: usize,
    pub active: usize,
    pub completed: usize,
    pub failed_or_blocked: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RequestExecutionPlanChangeRequest {
    #[serde(default)]
    pub note: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct GroupReviewArtifactProjection {
    pub shard_reports: Vec<GroupReviewArtifactRef>,
    pub reduction_reports: Vec<GroupReviewArtifactRef>,
}
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CodingAttemptSnapshotResponse {
    pub attempt: CodingAttemptDto,
    pub attempt_scope: String,
    pub work_item_group_id: Option<String>,
    pub current_work_item_id: Option<String>,
    pub active_unit_id: Option<String>,
    #[serde(default)]
    pub units: Vec<CodingExecutionUnitDto>,
    pub provider_config_snapshot: ProviderConfigSnapshot,
    pub timeline_nodes: Vec<CodingTimelineNode>,
    pub active_node_id: Option<String>,
    pub code_review_reports: Vec<CodeReviewReport>,
    pub review_request: Option<ReviewRequest>,
    pub internal_pr_review: Option<InternalPrReview>,
    #[serde(default)]
    pub group_coding_progress: Option<Vec<WorkItemCodingProgressDto>>,
    #[serde(default)]
    pub group_progress: Option<GroupCodingProgressDto>,
    #[serde(default)]
    pub group_review_artifacts: Option<GroupReviewArtifactProjection>,
    #[serde(default)]
    pub group_final_readiness: Option<GroupFinalReadinessSnapshot>,
    pub pending_gates: Vec<CodingGateRequired>,
    pub pending_choices: Vec<CodingChoiceGate>,
    #[serde(default)]
    pub role_runs: Vec<CodingRoleRunSnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub work_item_execution_plan: Option<WorkItemExecutionPlan>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct GenerateStorySpecsRequest {
    pub title: String,
    /// Optional logical-codebase scope emitted by a provider/gateway. Omission
    /// preserves the existing draft behavior where involved targets are not yet
    /// determined.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub involved_repository_ids: Option<Vec<LogicalRepositoryId>>,
    pub author_provider: Option<String>,
    pub reviewer_provider: Option<String>,
    pub review_rounds: Option<u32>,
    pub superpowers_enabled: Option<bool>,
    pub openspec_enabled: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct GenerateStorySpecsResponse {
    pub story_specs: Vec<StorySpecDto>,
    pub workspace_session: WorkspaceSessionDto,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct GenerateDesignSpecsRequest {
    pub title: String,
    pub story_spec_ids: Vec<String>,
    /// Optional logical-codebase targets emitted by a provider/gateway.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub involved_repository_ids: Option<Vec<LogicalRepositoryId>>,
    /// Optional execution order for the supplied logical-codebase targets.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change_order: Option<Vec<LogicalRepositoryId>>,
    pub author_provider: Option<String>,
    pub reviewer_provider: Option<String>,
    pub review_rounds: Option<u32>,
    pub superpowers_enabled: Option<bool>,
    pub openspec_enabled: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct GenerateDesignSpecsResponse {
    pub design_specs: Vec<DesignSpecDto>,
    pub workspace_session: WorkspaceSessionDto,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct GenerateWorkItemsRequest {
    pub title: String,
    pub story_spec_ids: Vec<String>,
    pub design_spec_ids: Vec<String>,
    pub include_integration_tests: Option<bool>,
    pub include_e2e_tests: Option<bool>,
    pub force_frontend_backend_split: Option<bool>,
    pub require_execution_plan_confirm: Option<bool>,
    pub author_provider: Option<String>,
    pub reviewer_provider: Option<String>,
    pub review_rounds: Option<u32>,
    pub superpowers_enabled: Option<bool>,
    pub openspec_enabled: Option<bool>,
    /// 重生时注入上一次 validate findings，让 provider 针对问题返修。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision_feedback: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkItemSplitOptions {
    pub include_integration_tests: bool,
    pub include_e2e_tests: bool,
    pub force_frontend_backend_split: bool,
    pub require_execution_plan_confirm: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkItemSplitFinding {
    pub finding_id: String,
    pub level: String,
    pub message: String,
    pub affected_scopes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PrepareWorkItemPlanRequest {
    pub title: String,
    pub story_spec_ids: Vec<String>,
    pub design_spec_ids: Vec<String>,
    pub author_provider: Option<String>,
    pub reviewer_provider: Option<String>,
    pub review_rounds: Option<u32>,
    pub superpowers_enabled: Option<bool>,
    pub openspec_enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_policy: Option<RunPolicy>,
    pub include_integration_tests: Option<bool>,
    pub include_e2e_tests: Option<bool>,
    pub force_frontend_backend_split: Option<bool>,
    pub require_execution_plan_confirm: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnrollmentPutRequest {
    pub expected_revision: Option<u64>,
    pub command: crate::product::models::automation::EnrollmentWriteCommand,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnrollmentBindRequest {
    pub expected_revision: u64,
    pub plan_id: String,
    pub session_id: String,
}
/// 批 2 T3（REQ-WIGA-09，块 3 §4）：统一「终止自动化」REST 请求体。
/// `confirmed=true` 表示统一二次确认已接受；未带确认零副作用（返回
/// `automation_plan_termination_required`＋身份 details）。`expected_binding`
/// 为提示性绑定互证（与 enrollment 当前代全等，不符 → 409，零副作用）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutomationEnrollmentTerminateRequest {
    pub command_id: String,
    pub expected_enrollment_id: String,
    pub expected_policy_revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_binding: Option<crate::product::models::automation::EnrollmentBindingIdentity>,
    #[serde(default)]
    pub confirmed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// 批 2 T3：统一终止逐 attempt 收口结果（透传引擎侧专用收口状态）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutomationEnrollmentTerminateAttemptResult {
    pub attempt_id: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// 批 2 T3：统一终止响应。`outcome=terminated|cleanup_required`
/// （cleanup_required 仍不可继续，重试只重做收口，同 command 重放）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutomationEnrollmentTerminateResponse {
    pub command_id: String,
    pub state: crate::product::models::automation::OperationState,
    pub enrollment_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_id: Option<String>,
    pub terminated_at: String,
    pub outcome: String,
    pub results: Vec<AutomationEnrollmentTerminateAttemptResult>,
}

/// 批 2 T3（块 3 §4）：advance/coding_start 停点的第三种显式继续请求
/// （不是 plan 对话继续也不是 coder 门，不携带说明文本）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutomationContinueChainRequest {
    pub command_id: String,
    pub expected_binding: crate::product::models::automation::EnrollmentBindingIdentity,
    pub expected_stop_key: String,
    #[serde(default)]
    pub confirm_unknown_side_effect: bool,
}

/// 批 2 T3：continue-chain 结果。`resolved=true` 表示原停点事实已解除并
/// 已发 autopilot 唤醒（auto 保留）；NeedsHuman/失败保留原事实。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutomationContinueChainResponse {
    pub command_id: String,
    pub state: crate::product::models::automation::OperationState,
    pub stop_key: String,
    pub stage: String,
    pub resolved: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PrepareWorkItemPlanResponse {
    pub work_item_plan: IssueWorkItemPlanDetailDto,
    pub workspace_session: WorkspaceSessionDto,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct IssueWorkItemPlanDetailDto {
    pub id: String,
    pub issue_id: String,
    pub project_id: String,
    pub status: String,
    pub source_story_spec_ids: Vec<String>,
    pub source_design_spec_ids: Vec<String>,
    pub work_item_ids: Vec<String>,
    pub verification_plan_ids: Vec<String>,
    pub dependency_graph: Vec<IssueWorkItemPlanDependencyEdgeDto>,
    /// REQ-MTG-04（WP3，additive）：plan 级 group 聚合只读投影——旧响应/
    /// 旧客户端缺省 None 兼容（serde default）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_projection: Option<PlanGroupProjectionDto>,
    /// F-26b（additive）：plan 会话 workspace timeline reviewer 节点证据投影
    /// （"running"/"completed"），零证据缺省 None 兼容旧响应。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_status: Option<String>,
    pub repository_profile_ref: Option<String>,
    pub options: WorkItemSplitOptions,
    pub validator_findings: Vec<WorkItemSplitFinding>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct IssueWorkItemPlanDependencyEdgeDto {
    pub from_work_item_id: String,
    pub to_work_item_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkspaceSessionMessageRequest {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkspaceSessionRunNextRequest {
    pub user_prompt: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkspaceSessionConfirmRequest {
    pub confirmed_by: String,
    /// F-31 纠正轮：用户是否选择「确认并送审」。缺省 false=直接定稿（F-31 v36 前的
    /// 既有行为）；true=进入 Review（begin_review_after_author_confirm 路径）。review
    /// 未启用的会话携带 true 时端点如实 4xx，不得静默按定稿。
    #[serde(default)]
    pub with_review: bool,
}

/// P0 1.3（REQ-WIGA-05）Task 10：无 driver 的人工门命令族。`expected_gate_id`
/// 与服务端当前 active gate/timeline node 比对；`command_id` 为幂等/审计键
/// （与 WS HumanGateFeedback/AbandonHumanGate 同族）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HumanActionRequest {
    Approve {
        command_id: String,
        expected_gate_id: String,
    },
    Abandon {
        command_id: String,
        expected_gate_id: String,
    },
    Feedback {
        command_id: String,
        expected_gate_id: String,
        feedback: String,
    },
    CompileRecovery {
        command_id: String,
        expected_gate_id: String,
        action: crate::web::workspace_ws_types::WorkItemPlanCompileRecoveryActionDto,
        reason: Option<String>,
    },
    /// C1 Task 4（REQ-C1-GATE-01/02）：孤儿候选门恢复/重建动作（唯一
    /// additive variant，不新增第二 candidate recovery 路由）。
    CandidateRecovery {
        command_id: String,
        expected_gate_id: String,
        action: CandidateRecoveryActionDto,
    },
}

/// C1 Task 4：恢复动作 wire 枚举（`recover`＝恢复原门；`rebuild`＝从权威
/// source/IR/report 重建呈现面）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateRecoveryActionDto {
    Recover,
    Rebuild,
}

impl From<CandidateRecoveryActionDto>
    for crate::product::work_item_plan_policy::CandidateRecoveryAction
{
    fn from(value: CandidateRecoveryActionDto) -> Self {
        match value {
            CandidateRecoveryActionDto::Recover => Self::Recover,
            CandidateRecoveryActionDto::Rebuild => Self::Rebuild,
        }
    }
}

/// C1 Task 4（REQ-C1-GATE-02）：候选门恢复结果（`CandidateRecovery` 变体
/// 的专用响应体）。`state` 复用统一 `OperationState`（accepted/replayed/
/// needs_human/rejected）；`complete=false` 时 `missing` 列出缺失事实。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CandidateRecoveryResult {
    pub command_id: String,
    pub gate_id: String,
    pub state: crate::product::models::OperationState,
    pub complete: bool,
    pub missing: Vec<String>,
    pub completed_steps: Vec<String>,
}

/// 人工命令回执状态（snake_case wire）：Accepted=已受理（含幂等重放与
/// AlreadyClosed）；Busy=门/轮次占用（409）；Rejected=引擎语义拒绝（422，
/// 细节经 ApiError code/message 上抛）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HumanActionState {
    Accepted,
    Busy,
    Rejected,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct HumanActionStatus {
    pub command_id: String,
    pub state: HumanActionState,
    pub gate_id: String,
}

/// P2 GAP-E/G（Task 0.1）：人工显式重驱失败 SingleCandidate reviewer 运行。
/// `expected_phase` 必须与 durable 当前相位一致（当前仅受理 `failed`）；
/// `command_id` 与路径中的 `failed_node_id` 组成幂等/审计键。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RetryFailedScRunRequest {
    pub command_id: String,
    pub expected_phase: crate::product::models::SingleCandidatePhase,
}

/// 受理回执：accepted=首次受理；replayed=同键重放读原结果；
/// needs_human=派发副作用不明，人工分诊（不自动重试）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RetryFailedScRunStatus {
    pub command_id: String,
    pub failed_node_id: String,
    pub state: String,
}

/// 批 2 P4（REQ-WIGA-10）：plan 阶段失败继续请求。`expected_binding` 复用
/// durable 绑定身份（与 enrollment 当前代全等才可继续）；`expected_stop_key`
/// 取服务端当前停点（失败节点/人工门/生成检查点派生键），防止把旧说明
/// 送给新停点；`context` 为对话窗口补充说明（provider 失败继续可空、
/// 评审超限继续必须非空）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutomationContinuePlanRequest {
    pub command_id: String,
    pub expected_binding: crate::product::models::automation::EnrollmentBindingIdentity,
    pub expected_stop_key: String,
    #[serde(default)]
    pub context: Option<String>,
}

impl AutomationContinuePlanRequest {
    /// 稳定 payload 摘要（命令账本按此判同 command 异 payload，fail-closed）。
    pub fn payload_digest(&self) -> String {
        let payload =
            serde_json::to_string(self).expect("continue-plan request payload is serializable");
        format!("sha256:{:x}", sha2::Sha256::digest(payload.as_bytes()))
    }
}

/// 批 2 P4（REQ-WIGA-10）：继续受理回执。accepted=首次受理并推进；
/// replayed=同键同负载重放读首次结果（不重复写 note/派发）；
/// needs_human=派发副作用不明或收尾写入失败，人工分诊（不自动重试）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct AutomationContinuePlanStatus {
    pub command_id: String,
    pub state: String,
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<String>,
}

/// 批 2 P4（REQ-WIGA-10）：只读 control 管理投影。按 durable
/// session/门/生成检查点及 N0 持久事实判定；初版 allowed_actions 只含
/// continue_plan 与既有门动作，不含 takeover；GET 不写任何事实。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct AutomationControlStatus {
    pub managed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binding: Option<crate::product::models::automation::EnrollmentBindingIdentity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminated_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_key: Option<String>,
    pub stage: String,
    pub allowed_actions: Vec<String>,
    pub context_required: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkspaceMessageDto {
    pub role: String,
    pub content: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkspaceSessionSummaryDto {
    pub workspace_session_id: String,
    pub issue_id: String,
    pub entity_id: String,
    pub workspace_type: String,
    pub status: String,
    pub author_provider: String,
    pub reviewer_provider: String,
    pub review_rounds: u32,
    pub superpowers_enabled: bool,
    pub openspec_enabled: bool,
    /// P0 1.2（REQ-WIGA-08）：同源 durable automation 归属投影——不复制进
    /// 持久 JSON，由 HTTP 出口按 enrollment 事实计算注入；读取失败在
    /// handler 层显式报错，不伪 client。
    pub automation: crate::product::models::automation::AutomationOwnership,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkspaceSessionDto {
    pub workspace_session_id: String,
    pub issue_id: String,
    pub entity_id: String,
    pub workspace_type: String,
    pub status: String,
    pub author_provider: String,
    pub reviewer_provider: String,
    pub review_rounds: u32,
    pub superpowers_enabled: bool,
    pub openspec_enabled: bool,
    pub automation: crate::product::models::automation::AutomationOwnership,
    pub messages: Vec<WorkspaceMessageDto>,
}

/// 显式人工接管结果。`workspace_session` 是新建的交互式子 session；其余字段保留
/// 与不可变 stopped 父 session 及审计事件的持久关联。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkspaceSessionTakeoverDto {
    #[serde(flatten)]
    pub workspace_session: WorkspaceSessionDto,
    pub parent_session_id: String,
    pub takeover_event_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct IssueListResponse {
    pub issues: Vec<IssueDto>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct IssueDto {
    pub issue_id: String,
    pub title: String,
    pub description: Option<String>,
    pub status: String,
    pub workspace_id: Option<String>,
    pub task_id: Option<String>,
    pub session_id: Option<String>,
    pub change_id: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CreateIssueRequest {
    pub title: String,
    pub description: Option<String>,
    pub change_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ResolveGateRequest {
    pub comment: Option<String>,
    pub requested_change: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ResolveGateResponse {
    pub issue_id: String,
    pub gate_id: String,
    pub node_id: String,
    pub decision: String,
    pub next_node: Option<String>,
}

/// N3（REQ-CS-01）：多仓 confirmed plan 的一键显式 Coding 启动请求——
/// `command_id` 为稳定幂等键，`expected_plan_revision_id` 与 GET 聚合返回的
/// bound revision 互证（漂移即拒）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PlanCodingStartRequest {
    pub command_id: String,
    pub expected_plan_revision_id: String,
}

/// N3（REQ-CS-06）：plan 聚合中的 target 仓投影。`lease_state` 沿现 repo
/// worktree lock 事实（`absent|held|contested`），不造 provider_running 猜测。
///
/// S3（Q5 逐仓 PR 面板数据源 A）：九个 additive 字段承载逐仓交付事实，
/// 类型均 `string | null`——`None` = 无对应事实（成员 attempt 已删/attempt
/// 面字段缺失、无 ReviewRequest、无人工恢复记录）。`stage`/`push_status`
/// 为 snake_case 文本（`coding_execution_stage_text`/`push_status_text`
/// 值域，`push_status` 含 `skipped_not_configured`）；`repository_name`
/// 解析自 checkout 路径末段（`resolve_plan_target_repository_name` 同款，
/// 回落 id 字符串）；`manual_recovery_reason` 为开放值集原样透传（如
/// `coding_runner_failed_while_running`）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PlanCodingTargetDto {
    pub target_repository_id: String,
    pub attempt_id: String,
    pub attempt_status: crate::product::coding_models::CodingAttemptStatus,
    pub worktree_path: Option<PathBuf>,
    pub lease_state: String,
    pub repository_name: Option<String>,
    pub branch_name: Option<String>,
    pub base_branch: Option<String>,
    pub stage: Option<String>,
    pub head_commit: Option<String>,
    pub push_status: Option<String>,
    pub push_error: Option<String>,
    pub review_request_id: Option<String>,
    pub manual_recovery_reason: Option<String>,
}

/// N3（REQ-CS-06）：plan 聚合中的 WI 进度投影。`execution_state` 值域
/// `initialized|running|waiting_dependency|waiting_slot|paused|completed|invalidated`
///（invalidated=run 授权失效，完成事实仍投影 completed），provider 事实
///（claim 相位 ≥ ProviderMayHaveStarted）与初始化 unit Running 区分；
/// S3（Q3 投影真实化，additive）：`provider_started` 需 attempt.status==
/// Running 才投影 `running`；attempt 级屏幕优先于 unit 级等待投影——
/// `awaiting_manual_recovery`（AMR 现场 unit 仍 Running，不改写底层事实）、
/// `waiting_human`（Blocked/WaitingForHuman）、`failed`、`aborted`。
/// `waiting_reason` 编排等待子集值域
/// `dependency_not_ready|execution_slot_occupied|plan_coding_continuation_not_enabled`；
/// `awaiting_manual_recovery` 时为 `manual_recovery_reason` 开放值集透传。
/// Slice B（仪表盘跨仓依赖投影）：另带全量直接依赖（权威绑定，跨桶不失真）、
/// 全图拓扑序 `order_index` 与桶内 `summary`，前端据此重建 plan 全图拓扑/依赖链。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PlanCodingUnitProgressDto {
    pub unit_id: String,
    pub logical_work_item_id: String,
    pub target_repository_id: String,
    pub attempt_id: String,
    pub unit_status: crate::product::coding_models::CodingExecutionUnitStatus,
    pub execution_state: String,
    pub unsatisfied_dependency_ids: Vec<String>,
    pub waiting_reason: Option<String>,
    pub dependency_logical_work_item_ids: Vec<String>,
    pub order_index: usize,
    pub summary: Option<String>,
}

/// N3（REQ-CS-06）：按 bound target attempts 聚合的 plan 进度（GET/POST 共用
/// 读函数产出）。primary 仅表示打开入口，不代表整 plan 执行或完成。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PlanCodingProgressDto {
    pub plan_id: String,
    pub bound_plan_revision_id: String,
    pub command_id: Option<String>,
    pub mode: Option<String>,
    pub phase: Option<String>,
    pub primary_attempt_id: Option<String>,
    pub selected_unit_id: Option<String>,
    pub selected_attempt_id: Option<String>,
    pub target_attempts: Vec<PlanCodingTargetDto>,
    pub units: Vec<PlanCodingUnitProgressDto>,
    /// P1-a（REQ-MTG-11 / 契约 A2）：plan 级聚合最终确认就绪投影。additive：
    /// 旧客户端反序列化缺省为 not_ready 空投影；GET 只看持久事实，不检查
    /// git（owner/范围/git 仍由逐项 `handle_final_confirm` 提交时校验）。
    #[serde(default)]
    pub final_confirm: PlanCodingFinalConfirmDto,
    /// P4-c（REQ-CRO-12 / 契约 A1）：本 plan 同源的待处理事项（choice/
    /// triage/blocked/stalled/manual_recovery/plan_final_confirm；多 target
    /// 聚合抑制逐仓 final_confirm）。additive：旧客户端缺省为空。
    #[serde(default)]
    pub attention_items: Vec<crate::web::coding_attention_info::CodingAttentionItemDto>,
    pub plan_complete: bool,
    /// H4（设计 C 节，H5 冻结形状对齐）：workspace_lifecycle 事实——
    /// generation 单调、现存/缺失成员计数与状态；数据来自 run/advance/
    /// journal 交叉核验，不从 attempt_status 猜物理存在性。
    pub workspace_lifecycle: PlanWorkspaceLifecycleDto,
}

/// H4（设计 C 节）：plan Coding 工作区生命周期事实投影（GET/DELETE 共用）。
/// `state` 值域 `clean|complete|incomplete|clearing`——clean=无执行内容、
/// complete=成员齐全、incomplete=成员缺失（部分删除后）、clearing=清理进行中。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PlanWorkspaceLifecycleDto {
    pub generation: u64,
    pub existing_member_count: usize,
    pub missing_member_count: usize,
    pub state: String,
}

/// P1-a（REQ-MTG-11 / 契约 A2）：未齐备的冻结 target（GET `blocking` 条目；
/// POST not_ready 回执的失败项同构）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PlanCodingFinalConfirmBlockingDto {
    pub attempt_id: String,
    pub reason_code: String,
    pub message: String,
}

/// P1-a（REQ-MTG-11 / 契约 A2）：plan 级聚合最终确认投影——
/// `state` 值域 `not_ready|ready|partial|done`：Confirmed 视为已确认；
/// 全 Completed → done；部分 Completed 且剩余全就绪 → partial；零确认且
/// 全就绪 → ready；其余（含 blocking 非空/plan 未执行完成）→ not_ready。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PlanCodingFinalConfirmDto {
    pub state: String,
    pub ready_attempt_ids: Vec<String>,
    pub confirmed_attempt_ids: Vec<String>,
    pub blocking: Vec<PlanCodingFinalConfirmBlockingDto>,
}

impl Default for PlanCodingFinalConfirmDto {
    fn default() -> Self {
        Self {
            state: "not_ready".to_string(),
            ready_attempt_ids: Vec::new(),
            confirmed_attempt_ids: Vec::new(),
            blocking: Vec::new(),
        }
    }
}

/// P1-a（契约 A2）：POST `.../coding-final-confirm` 回执逐仓结果。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PlanCodingFinalConfirmResultItemDto {
    pub attempt_id: String,
    pub target_repository_id: String,
    /// 值域 `confirmed|already_completed|failed|not_attempted`。
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// P1-a（契约 A2）：POST `.../coding-final-confirm` 回执——
/// `outcome` 值域 `all_confirmed|partially_confirmed|not_ready`；partial 只
/// 表示本次未全成，成功数按 `results` 计算。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PlanCodingFinalConfirmResultDto {
    pub plan_id: String,
    pub outcome: String,
    pub results: Vec<PlanCodingFinalConfirmResultItemDto>,
}

/// H4（设计 C 节，H5 冻结名对齐）：DELETE …/coding-workspaces 成功回执——
/// `operation_state: accepted|replayed`（replayed=同 command 幂等重放，不扫
/// 下一代刚创建的 attempts）；`generation` 为操作完成后的当前轮次；
/// `deleted/remaining_attempt_ids` 为本次操作权威范围结果（服务端枚举，
/// 不收前端 attempt ID 列表）；`workspace_lifecycle` 为操作后事实投影。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PlanCodingWorkspaceDeletionResultDto {
    pub plan_id: String,
    pub command_id: String,
    pub operation_state: String,
    pub generation: u64,
    pub deleted_attempt_ids: Vec<String>,
    pub remaining_attempt_ids: Vec<String>,
    pub workspace_lifecycle: PlanWorkspaceLifecycleDto,
}

/// H4（设计 C 节）：组删 DELETE 的 query 参数——稳定 `command_id`（同动作
/// 重试复用）、`expected_generation`（GET workspace_lifecycle.generation 互证，
/// 未知时省略）、`confirmed=true`（仅确认弹窗同意后显式携带；query 布尔不是
/// 安全授权，服务端分类才是权威）。
#[derive(Debug, Default, serde::Deserialize)]
pub struct PlanCodingWorkspaceDeletionQuery {
    pub command_id: String,
    pub expected_generation: Option<u64>,
    pub confirmed: Option<bool>,
}

/// N3（REQ-CS-01/02）：一键启动结果——`operation_state` 沿现
/// `accepted|replayed|needs_human|rejected` 语义；`start_outcome` 为冻结的
/// 首启真实回执。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PlanCodingStartResultDto {
    pub plan_id: String,
    pub bound_plan_revision_id: String,
    pub command_id: String,
    pub operation_state: crate::product::models::automation::OperationState,
    pub primary_attempt_id: Option<String>,
    pub target_attempts: Vec<PlanCodingTargetDto>,
    pub selected_unit_id: Option<String>,
    pub selected_attempt_id: Option<String>,
    pub start_outcome: Option<crate::product::coding_plan_run::CodingPlanStartOutcome>,
    pub waiting_units: Vec<PlanCodingUnitProgressDto>,
    pub progress: PlanCodingProgressDto,
}
