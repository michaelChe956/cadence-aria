import type { CodingAttempt, CodingAttemptStatus, PushStatus } from "./coding";
import type {
  LifecycleConfirmationStatus,
  ProductIssue,
  ProviderWorkspaceConfigInput,
  WorkItemContextBudget,
  WorkItemExecutionPlanStatus,
  WorkItemKind,
} from "./common";
import type {
  IssueWorkItemPlanDetailDto,
  WorkItemSplitFinding,
} from "./work-item-plan";
import type { ArtifactVersion, WorkspaceSession, WorkspaceSessionSummary } from "./workspace";

export type StorySpec = {
  story_spec_id: string;
  issue_id: string;
  repository_id: string;
  title: string;
  current_version: number | null;
  current_markdown_preview: string | null;
  confirmation_status: LifecycleConfirmationStatus;
  artifact_versions: ArtifactVersion[];
};

export type DesignSpec = {
  design_spec_id: string;
  issue_id: string;
  story_spec_ids: string[];
  title: string;
  current_version: number | null;
  current_markdown_preview: string | null;
  confirmation_status: LifecycleConfirmationStatus;
  artifact_versions: ArtifactVersion[];
};

export type LifecycleWorkItem = {
  work_item_id: string;
  issue_id: string;
  repository_id: string;
  story_spec_ids: string[];
  design_spec_ids: string[];
  title: string;
  plan_status: "not_started" | "draft" | "confirmed" | "change_requested";
  execution_status: "pending" | "planning" | "coding" | "completed" | "blocked";
  latest_attempt: CodingAttempt | null;
  artifact_versions: ArtifactVersion[];
  work_item_set_id: string | null;
  source_work_item_plan_id?: string | null;
  source_outline_id?: string | null;
  source_draft_id?: string | null;
  planned_implementation_context?: string | null;
  kind: WorkItemKind;
  sequence_hint: number | null;
  depends_on: string[];
  exclusive_write_scopes: string[];
  forbidden_write_scopes: string[];
  context_budget: WorkItemContextBudget;
  verification_plan_ref: string | null;
  require_execution_plan_confirm: boolean;
  execution_plan_status: WorkItemExecutionPlanStatus;
  completion_commit: string | null;
  completion_diff_summary_ref: string | null;
  validator_findings?: WorkItemSplitFinding[];
};

// REQ-TGT-05：后端按 target_repository_id 分组的 WorkItem 聚合视图 DTO（对应
// human_presentation.rs 的 WorkItemRepositoryGroup）。
// - target_repository_id 为 null 时表示遗留/未指定仓库（compatibility_projection = true）。
// - alias 为仓库展示名（member alias / 物理投影名）。
// - status 为仓库级聚合状态（blocked/pending/planning/coding/completed）。
export type WorkItemRepositoryGroup = {
  target_repository_id: string | null;
  alias: string;
  status: string;
  compatibility_projection: boolean;
  items: LifecycleWorkItem[];
};

// 后端 IssueLifecycleResponse.delivery_summary 的单个条目投影（serde snake_case）。
// attempt_status / push_status 为 null 表示对应 Work Item 尚无 attempt / ReviewRequest。
export type DeliveryEntryDto = {
  repository_name: string;
  work_item_id: string;
  attempt_status: CodingAttemptStatus | null;
  branch_name: string | null;
  commit_sha: string | null;
  push_status: PushStatus | null;
  push_error: string | null;
};

// Issue 级交付状态聚合（"all_pushed" | "partial" | "none"）。
export type IssueDeliverySummaryDto = {
  project_id: string;
  issue_id: string;
  entries: DeliveryEntryDto[];
  overall: "all_pushed" | "partial" | "none";
};

export type IssueLifecycleResponse = {
  issue: ProductIssue;
  story_specs: StorySpec[];
  design_specs: DesignSpec[];
  work_item_plans: IssueWorkItemPlanDetailDto[];
  work_items: LifecycleWorkItem[];
  // 向后兼容：后端始终返回该字段；旧响应缺失时前端按空数组（单仓扁平展示）处理。
  work_item_repository_groups: WorkItemRepositoryGroup[];
  workspace_sessions: WorkspaceSessionSummary[];
  coding_attempts: CodingAttempt[];
  // 向后兼容：后端始终返回该字段；旧响应缺失时前端不渲染交付状态面板。
  delivery_summary?: IssueDeliverySummaryDto;
};

export type GenerateStorySpecsRequest = ProviderWorkspaceConfigInput & {
  title: string;
};

export type GenerateStorySpecsResponse = {
  story_specs: StorySpec[];
  workspace_session: WorkspaceSession;
};

export type GenerateDesignSpecsRequest = ProviderWorkspaceConfigInput & {
  title: string;
  story_spec_ids: string[];
};

export type GenerateDesignSpecsResponse = {
  design_specs: DesignSpec[];
  workspace_session: WorkspaceSession;
};
