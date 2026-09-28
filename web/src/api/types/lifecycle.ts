import type { CodingAttempt, CodingAttemptStatus, PushStatus } from "./coding";
import type {
  LifecycleConfirmationStatus,
  ProductIssue,
  ProviderWorkspaceConfigInput,
  WorkItemContextBudget,
  WorkItemExecutionPlanStatus,
  WorkItemKind,
  WorkspaceProviderName,
} from "./common";
import type {
  IssueWorkItemPlanDetailDto,
  WorkItemSplitFinding,
} from "./work-item-plan";
import type { ArtifactVersion, WorkspaceSession, WorkspaceSessionSummary } from "./workspace";

export type WorkspaceReviewStatus = "running" | "completed";

export type StorySpec = {
  story_spec_id: string;
  issue_id: string;
  repository_id: string;
  title: string;
  current_version: number | null;
  current_markdown_preview: string | null;
  confirmation_status: LifecycleConfirmationStatus;
  artifact_versions: ArtifactVersion[];
  // F-26b（additive）：workspace timeline reviewer 节点证据投影；旧响应缺省。
  review_status?: WorkspaceReviewStatus | null;
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
  // F-26b（additive）：workspace timeline reviewer 节点证据投影；旧响应缺省。
  review_status?: WorkspaceReviewStatus | null;
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

// P1（REQ-WIGA-07）Task 8：durable publication/compile + Confirmed 派生的只读
// 确认信息。key 形如 plan_confirmed:{plan_id}:{compile_id}（稳定身份），
// occurred_at 取成功事务 committed_at（不随 updated_at 变化）。
export type PlanConfirmedInfoItem = {
  key: string;
  plan_id: string;
  session_id: string;
  occurred_at: string;
  title: string;
};

// P2（REQ-WIGA-07/R5）Task 8：enrolled 已认领 group attempt 的编码执行完成/
// 人工最终确认信息。key 形如 coding_final_confirm:{attempt_id}:{node_id}
// （稳定身份），occurred_at 取 FinalConfirm 节点首次 started_at（readiness
// 重写不漂移）。
export type CodingFinalConfirmInfoItem = {
  key: string;
  project_id: string;
  issue_id: string;
  plan_id: string;
  attempt_id: string;
  occurred_at: string;
  title: string;
  final_confirmed: boolean;
};

// P3（REQ-WIGA-07）：issue 级有界近期完成目录条目（判别联合；服务端
// snake_case，null 不可省略）。plan：session_id 有值、attempt_id/
// final_confirmed 为 null；coding：session_id 为 null、attempt_id/
// final_confirmed 有值。
export type RecentCompletionInfoItem =
  | {
      kind: "plan_confirmed";
      key: string;
      project_id: string;
      issue_id: string;
      plan_id: string;
      session_id: string;
      attempt_id: null;
      occurred_at: string;
      title: string;
      final_confirmed: null;
    }
  | {
      kind: "coding_final_confirm";
      key: string;
      project_id: string;
      issue_id: string;
      plan_id: string;
      session_id: null;
      attempt_id: string;
      occurred_at: string;
      title: string;
      final_confirmed: boolean;
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
  // P1（REQ-WIGA-07）：只读 plan 确认信息（0 或 1 条/issue）。additive：旧响应
  // 缺失时前端按无 info 处理。
  plan_confirmed_info?: PlanConfirmedInfoItem[];
  // P2（REQ-WIGA-07/R5）：enrolled 已认领 group attempt 的 FinalConfirm 等待/
  // 已确认只读信息。additive：旧响应缺失时前端按无 info 处理。
  coding_final_confirm_info?: CodingFinalConfirmInfoItem[];
  // P3（REQ-WIGA-07）：issue 级有界近期完成目录（仅在请求携带 recent_since
  // 时计算）。additive：旧响应缺失时前端按无近期补读处理（回落 watched
  // info 投影，绝不认作 K 外全量）。
  recent_completion_info?: RecentCompletionInfoItem[];
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

// P1 WIGA（对齐 Rust `src/product/models/automation.rs` + `automation_target.rs`
// 的 serde snake_case JSON）：Design 确认后的显式自动化 enrollment 面。
export type AutomationMode = "manual" | "automatic";

export type EnrollmentSpecRef = {
  id: string;
  version: number;
};

export type EnrollmentSource = {
  stories: EnrollmentSpecRef[];
  designs: EnrollmentSpecRef[];
};

export type EnrollmentPlanOptions = {
  include_integration_tests: boolean;
  include_e2e_tests: boolean;
  force_frontend_backend_split: boolean;
  require_execution_plan_confirm: boolean;
};

export type EnrollmentOptions = {
  author_provider: WorkspaceProviderName;
  reviewer_provider: WorkspaceProviderName;
  review_rounds: number;
  superpowers_enabled: boolean;
  openspec_enabled: boolean;
  plan_options: EnrollmentPlanOptions;
};

export type EnrollmentTarget =
  | { kind: "single_repository"; repository_id: string }
  | {
      kind: "logical_codebase";
      logical_codebase_id: string;
      logical_repository_id: string;
    };

export type EnrollmentBindingIdentity = {
  binding_version: number;
  enrollment_id: string;
  plan_id: string;
  session_id: string;
  source: EnrollmentSource;
  target: EnrollmentTarget;
  author_provider: WorkspaceProviderName;
  reviewer_provider: WorkspaceProviderName;
};

export type EnrollmentBindingHistory = {
  current: EnrollmentBindingIdentity;
  previous: EnrollmentBindingIdentity[];
};

export type AutomationEnrollmentOperationState =
  | "accepted"
  | "replayed"
  | "needs_human"
  | "rejected";

export type IssueAutomationEnrollment = {
  enrollment_id: string;
  selection_key: string;
  project_id: string;
  issue_id: string;
  enabled: boolean;
  policy_revision: number;
  source: EnrollmentSource;
  options: EnrollmentOptions;
  logical_repository_id: string;
  prepare_intent_id: string;
  plan_id: string | null;
  session_id: string | null;
  created_at: string;
  updated_at: string;
  /** C1：enable 显式声明时在场；旧投影缺字段按 off/Manual 解释。 */
  target?: EnrollmentTarget;
  binding_history?: EnrollmentBindingHistory;
};

export type AutomationEnrollmentEnableCommand = {
  type: "enable";
  selection_key: string;
  source: EnrollmentSource;
  options: EnrollmentOptions;
  logical_repository_id: string;
  /** C1：服务端 automation-target 投影原样回传，不从前端拼凑。 */
  target?: EnrollmentTarget;
};

export type AutomationEnrollmentDisableCommand = {
  type: "disable";
};

export type AutomationEnrollmentPutRequest = {
  expected_revision: number | null;
  command: AutomationEnrollmentEnableCommand | AutomationEnrollmentDisableCommand;
};

export type AutomationEnrollmentRebindRequest = {
  command_id: string;
  expected_policy_revision: number;
  expected_binding_version: number;
  binding: {
    plan_id: string;
    session_id: string;
    source: EnrollmentSource;
    target: EnrollmentTarget;
    author_provider: WorkspaceProviderName;
    reviewer_provider: WorkspaceProviderName;
  };
  reason: string;
};

export type AutomationEnrollmentRebindResult = {
  command_id: string;
  state: AutomationEnrollmentOperationState;
  enrollment: IssueAutomationEnrollment;
};

// Task 1 只读投影：唯一逻辑仓 UUID + 服务端与 prepare 同源解析的 options。
export type AutomationTargetQuery = {
  author_provider?: WorkspaceProviderName;
  reviewer_provider?: WorkspaceProviderName;
  review_rounds?: number;
  superpowers_enabled?: boolean;
  openspec_enabled?: boolean;
  include_integration_tests?: boolean;
  include_e2e_tests?: boolean;
  force_frontend_backend_split?: boolean;
  require_execution_plan_confirm?: boolean;
};

export type AutomationTarget = {
  logical_repository_id: string;
  /** C1：双载体 target 投影（Enable/Rebind 原样回传）。 */
  enrollment_target: EnrollmentTarget;
  resolved_options: EnrollmentOptions;
};
