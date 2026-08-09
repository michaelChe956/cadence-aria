use std::path::PathBuf;

use crate::product::logical_codebase::LogicalRepositoryId;
use crate::product::models::{
    IssueWorkItemDependencyEdge, IssueWorkItemPlanOptions, IssueWorkItemPlanStatus, ProviderName,
    RepositoryProfileConfidence, VerificationCommand, VerificationFallbackPolicy,
    VerificationManualCheck, VerificationScope, WorkItemContextBudget, WorkItemKind,
    WorkItemPlanStatus, WorkItemSplitFinding, WorkspaceType,
};

/// 聚合代码库 Story 视野范围。由 Story 生成/修订入口在逻辑代码库分支构造：
/// `logical_codebase_ref` 取 manifest.logical_codebase_id；`effective_member_ids` 来自
/// `PlanningContextSnapshot.effective_member_ids`（权威）；`involved_repository_ids` 来自 AI
/// 输出且必须 ⊆ effective_member_ids；`focus_repository_id` 可空，若给出必须 ∈ involved。
///
/// AI 未明确涉及仓库（空 involved）或涉及不在有效集合的仓库 → blocker，不塞 primary
/// （REQ-PLN-07）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AggregateStorySpecScope {
    pub logical_codebase_ref: uuid::Uuid,
    pub effective_member_ids: Vec<LogicalRepositoryId>,
    /// AI 明确声明的涉及仓库；空表示未确定 → blocker（不回落 primary）。
    pub involved_repository_ids: Vec<LogicalRepositoryId>,
    /// 迁移期 focus/primary 投影；可空，若给出必须 ∈ involved_repository_ids。
    pub focus_repository_id: Option<LogicalRepositoryId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateStorySpecInput {
    pub project_id: String,
    pub issue_id: String,
    pub repository_id: String,
    pub title: String,
    /// 聚合代码库视野。`None` 表示传统单仓 issue，走原 `repository_id` 单值路径；
    /// `Some` 表示逻辑代码库分支，以 `involved_repository_ids` 等聚合字段为权威，
    /// 并按 effective_member_ids 校验。
    pub aggregate_codebase: Option<AggregateStorySpecScope>,
}

/// 聚合代码库 Design 视野范围。由 Design 生成/修订入口在逻辑代码库分支构造：
/// `logical_codebase_ref` 取 manifest.logical_codebase_id；`effective_member_ids` 来自
/// `PlanningContextSnapshot.effective_member_ids`（权威）；`involved_repository_ids` 来自 AI
/// 输出且必须 ⊆ effective_member_ids；`change_order` 为 AI 显式给出的改动顺序图（执行顺序，
/// 非服务调用图，REQ-TGT-04），缺失不强制 blocker。
///
/// AI 未明确涉及仓库（空 involved）或涉及不在有效集合的仓库 → blocker；Design 不再读取
/// `issue.repo_id` 填充任何字段（REQ-PLN-08）。`change_order` 若给出，其全部 id 必须 ∈
/// involved_repository_ids 且不得重复（执行顺序图不得重复顶点）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AggregateDesignSpecScope {
    pub logical_codebase_ref: uuid::Uuid,
    pub effective_member_ids: Vec<LogicalRepositoryId>,
    /// AI 明确声明的涉及仓库；空表示未确定 → blocker（不回落 issue.repo_id）。
    pub involved_repository_ids: Vec<LogicalRepositoryId>,
    /// AI 显式给出的改动顺序图（执行顺序）。可空；若给出则全部 id 必须 ∈ involved 且不重复。
    pub change_order: Vec<LogicalRepositoryId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateDesignSpecInput {
    pub project_id: String,
    pub issue_id: String,
    pub story_spec_ids: Vec<String>,
    pub title: String,
    /// 聚合代码库视野。`None` 表示传统单仓 issue；`Some` 进入逻辑代码库分支以聚合字段为权威，
    /// 并按 effective_member_ids 校验。Design 不再读取 issue.repo_id 填充任何字段。
    pub aggregate_codebase: Option<AggregateDesignSpecScope>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateWorkItemInput {
    pub id: Option<String>,
    pub project_id: String,
    pub issue_id: String,
    pub repository_id: String,
    pub story_spec_ids: Vec<String>,
    pub design_spec_ids: Vec<String>,
    pub title: String,
    pub work_item_set_id: Option<String>,
    pub source_work_item_plan_id: Option<String>,
    pub source_outline_id: Option<String>,
    pub source_draft_id: Option<String>,
    pub planned_implementation_context: Option<String>,
    pub kind: WorkItemKind,
    pub sequence_hint: Option<u32>,
    pub depends_on: Vec<String>,
    pub exclusive_write_scopes: Vec<String>,
    pub forbidden_write_scopes: Vec<String>,
    pub context_budget: WorkItemContextBudget,
    pub verification_plan_ref: Option<String>,
    pub require_execution_plan_confirm: bool,
    pub plan_status: WorkItemPlanStatus,
}

impl Default for CreateWorkItemInput {
    fn default() -> Self {
        Self {
            id: None,
            project_id: String::new(),
            issue_id: String::new(),
            repository_id: String::new(),
            story_spec_ids: Vec::new(),
            design_spec_ids: Vec::new(),
            title: String::new(),
            work_item_set_id: None,
            source_work_item_plan_id: None,
            source_outline_id: None,
            source_draft_id: None,
            planned_implementation_context: None,
            kind: WorkItemKind::default(),
            sequence_hint: None,
            depends_on: Vec::new(),
            exclusive_write_scopes: Vec::new(),
            forbidden_write_scopes: Vec::new(),
            context_budget: WorkItemContextBudget::default(),
            verification_plan_ref: None,
            require_execution_plan_confirm: false,
            plan_status: WorkItemPlanStatus::NotStarted,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateIssueWorkItemPlanInput {
    pub id: Option<String>,
    pub project_id: String,
    pub issue_id: String,
    pub source_story_spec_ids: Vec<String>,
    pub source_design_spec_ids: Vec<String>,
    pub options: IssueWorkItemPlanOptions,
    pub status: IssueWorkItemPlanStatus,
    pub work_item_ids: Vec<String>,
    pub repository_profile_ref: Option<String>,
    pub verification_plan_ids: Vec<String>,
    pub dependency_graph: Vec<IssueWorkItemDependencyEdge>,
    pub created_from_provider_run: Option<String>,
    pub validator_findings: Vec<crate::product::models::WorkItemSplitFinding>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueWorkItemPlanUpdate {
    pub work_item_ids: Vec<String>,
    pub verification_plan_ids: Vec<String>,
    pub repository_profile_ref: Option<String>,
    pub dependency_graph: Vec<IssueWorkItemDependencyEdge>,
    pub created_from_provider_run: Option<String>,
    pub validator_findings: Vec<WorkItemSplitFinding>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkItemPlanCandidateSnapshot {
    pub plan_id: String,
    pub work_item_ids: Vec<String>,
    pub verification_plan_ids: Vec<String>,
    pub repository_profile_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateRepositoryProfileInput {
    pub id: Option<String>,
    pub project_id: String,
    pub issue_id: String,
    pub repository_id: String,
    pub provider_run_ref: Option<String>,
    pub languages: Vec<String>,
    pub frameworks: Vec<String>,
    pub package_managers: Vec<String>,
    pub test_frameworks: Vec<String>,
    pub build_systems: Vec<String>,
    pub verification_capabilities: Vec<String>,
    pub detected_layers: Vec<String>,
    pub split_recommendation: String,
    pub confidence: RepositoryProfileConfidence,
    pub uncertainties: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateVerificationPlanInput {
    pub id: Option<String>,
    pub project_id: String,
    pub issue_id: String,
    pub work_item_id: String,
    pub repository_profile_ref: Option<String>,
    pub provider_run_ref: Option<String>,
    pub scope: VerificationScope,
    pub commands: Vec<VerificationCommand>,
    pub manual_checks: Vec<VerificationManualCheck>,
    pub required_gates: Vec<String>,
    pub risk_notes: Vec<String>,
    pub confidence: RepositoryProfileConfidence,
    pub fallback_policy: VerificationFallbackPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppendSpecVersionInput {
    pub project_id: String,
    pub issue_id: String,
    pub entity_id: String,
    pub markdown: String,
    pub provider_run_refs: Vec<String>,
    pub review_refs: Vec<String>,
    pub confirmed_by: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppendProviderReviewRoundInput {
    pub project_id: String,
    pub issue_id: String,
    pub session_id: String,
    pub round_index: u32,
    pub author_provider: ProviderName,
    pub reviewer_provider: ProviderName,
    pub review_result: String,
    pub revision_result: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateWorkspaceSessionInput {
    pub project_id: String,
    pub issue_id: String,
    pub entity_id: String,
    pub workspace_type: WorkspaceType,
    pub author_provider: ProviderName,
    pub reviewer_provider: ProviderName,
    pub review_rounds: u32,
    pub superpowers_enabled: bool,
    pub openspec_enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateProjectProviderDefaultsInput {
    pub project_id: String,
    pub author_provider: ProviderName,
    pub reviewer_provider: ProviderName,
    pub review_rounds: u32,
    pub superpowers_enabled: bool,
    pub openspec_enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpsertIssueSharedWorktreeInput {
    pub project_id: String,
    pub issue_id: String,
    pub repository_id: String,
    pub branch_name: String,
    pub worktree_path: PathBuf,
    pub base_branch: String,
}
