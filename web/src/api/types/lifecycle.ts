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

// C2 Task 12（coding-execution-resilience-and-product-triage）：等待项操作
// 上下文——与服务端 WaitingItemAction 一一对应（稳定 command_id＋expected
// 对象版本）；驾驶舱动作按 action 名路由到对应 REST，前端不判定业务成功。
export type WaitingItemAction = {
  action: string;
  command_id: string;
  expected_version: number;
};

// C1 Task 9（enrollment-recovery-surface）：issue 级 durable 恢复等待项
//（孤儿候选/lease 三态/Failed advance/intent 停等/换代历史）。actions 为
// 服务端 durable 事实派生的可用操作名（recover_candidate | confirm_takeover |
// retry_initialization | rebind | C2: restart_coding | gate action_id）；
// 前端只按名触发对应 REST，不自行判定成功。C2 additive：kind 扩展
// coding_completion_unconfirmed / coding_already_running / coding_takeover_required /
// coding_lease_unknown / coding_restart_available / reviewer_configuration_missing /
// verification_triage / policy_verification / instruction_claim_interrupted /
// large_candidate_blocked（宽 string 直传），expected_version／action_context
// 为 additive 字段（旧响应缺失按缺省解释）。
export type C1WaitingItem = {
  id: string;
  kind: string;
  reason: string;
  completed_steps: string[];
  target?: EnrollmentTarget | null;
  plan_id?: string | null;
  session_id?: string | null;
  attempt_id?: string | null;
  gate_id?: string | null;
  possible_side_effect?: string | null;
  actions: string[];
  next_phase?: string | null;
  expected_version?: number | null;
  action_context?: WaitingItemAction[];
  // C5 Task 6 additive：project 级 repository 初始化失败等待项身份与诊断。
  // project 条目 project_id 必有、issue_id 缺省（serde skip）；恢复关联字段
  // 为 None 时 JSON 缺省（#[serde(default)]，前端按缺省解释）。
  operation_id?: string | null;
  diagnostics?: RepositoryInitializationFailureDiagnostics | null;
  project_id?: string;
  issue_id?: string | null;
  parent_operation_id?: string | null;
  superseded_by?: string | null;
};

// P4-d（REQ-CRO-12/契约 A1 · REQ-UI37-19）：coding 待处理事项 DTO——
// 服务端 `CodingAttentionItemDto` 的 snake_case 镜像（durable 事实只读投影，
// 事实关闭后条目自然消失）。kind 语义：choice|triage|blocked|stalled|
// manual_recovery|final_confirm|plan_final_confirm（宽 string，前端不枚举
// 过滤——未知未来 kind 不丢）；身份 = [project_id, issue_id, key]。
export type CodingAttentionFocusDto = {
  gate_id?: string;
  choice_id?: string;
};

// 事项目标：coding_attempt|coding_plan|workspace_session（project/issue 取
// 条目顶层，地址域不重复；attempt 目标必有 attempt_id、plan 目标必有 plan_id）。
export type CodingAttentionTargetDto = {
  type: string;
  attempt_id?: string;
  plan_id?: string;
  session_id?: string;
};

export type CodingAttentionItemDto = {
  /** 稳定身份第三段（前两段为 project/issue）。 */
  key: string;
  kind: string;
  project_id: string;
  issue_id: string;
  plan_id?: string | null;
  attempt_id?: string | null;
  logical_work_item_id?: string | null;
  repository_name?: string | null;
  reason_code?: string | null;
  title: string;
  summary: string;
  /** RFC3339 事实时间（gate/choice created_at、node started_at 等）。 */
  occurred_at: string;
  target: CodingAttentionTargetDto;
  focus?: CodingAttentionFocusDto | null;
};

// C5 Task 6（REQ-INIT-C5-RESUME）：repository 初始化失败的结构化诊断
//（服务端 RepositoryInitializationFailureDiagnostics snake_case 镜像）。
export type RepositoryInitializationFailureDiagnostics = {
  failed_step: string;
  reason_code: string;
  provider: string | null;
  stderr_summary: string | null;
  changed_paths: string[];
  retryable: boolean;
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
  // C1（enrollment-recovery-surface Task 9）：durable 恢复等待项。additive：
  // 旧响应缺失时前端按无等待项处理。
  c1_waiting_items?: C1WaitingItem[];
  // P4-d（REQ-UI37-19）：durable 事实待处理事项全局目录（无 watched 过滤）。
  // additive：旧响应缺失时前端按无待处理事项处理。
  coding_attention_items?: CodingAttentionItemDto[];
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

// B2-R3 波 1（REQ-WIGA-02）：镜像 `src/product/models/automation.rs::
// PlanApprovalPolicy` 的 serde snake_case wire；缺省 manual。
export type PlanApprovalPolicy = "manual" | "auto";

export type EnrollmentOptions = {
  author_provider: WorkspaceProviderName;
  reviewer_provider: WorkspaceProviderName;
  review_rounds: number;
  superpowers_enabled: boolean;
  openspec_enabled: boolean;
  // 可选缺省 manual：后端 `#[serde(default)]`，旧 JSON/旧客户端 payload
  // 不含该字段按 manual 读；新自动化入口显式提交 "auto"。
  plan_approval?: PlanApprovalPolicy;
  plan_options: EnrollmentPlanOptions;
};

/**
 * 发起侧载体（Enable / Rebind 命令的 target）：**只有逻辑代码库两变体**。
 * 后端载体判定不再产出单仓载体（REQ-ROUTE-C5-TARGET）⇒ 命令侧不得再接受
 * `single_repository`；转换守卫见 `automation-reenable.ts` 的
 * `carrierTargetFromEnrollmentTarget`。
 */
export type EnrollmentCarrierTarget =
  | {
      kind: "logical_codebase";
      logical_codebase_id: string;
      logical_repository_id: string;
    }
  // 多成员逻辑代码库：成员集合 target（后端 EnrollmentTarget 第三变体，
  // wire `kind` = logical_codebase_members）。
  | {
      kind: "logical_codebase_members";
      logical_codebase_id: string;
      logical_repository_ids: string[];
    };

/**
 * 投影侧载体（服务端持久化投影的类型）：**三变体保留**。
 * ⚠ 第三变体 `single_repository` **仅 legacy 只读**——老单仓 enrollment 记录
 * 仍要能显示与反序列化（老记录不退化为「target 未知」）；但**新投影不再
 * 产出**它，命令侧也不接受它（命令 target 用 `EnrollmentCarrierTarget`）。
 */
export type EnrollmentTarget =
  | { kind: "single_repository"; repository_id: string }
  | EnrollmentCarrierTarget;

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
  prepare_intent_id: string;
  plan_id: string | null;
  session_id: string | null;
  created_at: string;
  updated_at: string;
  /** C1：双载体 enrollment target（Enable 写入后必在；旧投影缺字段按
   * 旧代无绑定解释，前端不回填替身身份）。 */
  target?: EnrollmentTarget;
  binding_history?: EnrollmentBindingHistory;
  /** 批 2 T4（REQ-WIGA-09）：整 plan 自动化终止时间（RFC3339）。非空即
   * 该 enrollment 已被终止——人工可显式重新开启（Enable 取代），自动路径
   * 不得复活。旧投影缺字段按未终止读。 */
  terminated_at?: string | null;
  /** REQ-WIGA-VOID-01：登记随工作项组删除作废的时间（RFC3339）。非空即该
   * 登记已作废——视为无约束力的历史记录，不再约束该 issue 上的新工作项组
   *（重新开启按换代放行 provider/source/target 全部漂移）。与 terminated_at
   * 是两个独立维度；旧投影缺字段按未作废读。 */
  voided_at?: string | null;
  /** REQ-WIGA-VOID-01：作废原因机器码（如 work_item_group_deleted）；只追加
   * 不改写。 */
  void_reason?: string | null;
};

export type AutomationEnrollmentEnableCommand = {
  type: "enable";
  /** 批 2 T4（REQ-WIGA-09 修订）：稳定命令身份——同一次用户动作（同一
   * payload）的重试复用同一个 command_id，服务端据此幂等重放（缺此字段
   * 后端 422；同 command_id 配异 payload 判 409）。 */
  command_id: string;
  selection_key: string;
  source: EnrollmentSource;
  options: EnrollmentOptions;
  /** C1：服务端 automation-target 投影原样回传（必填，前端不拼凑）。
   * ⚠ 发起侧载体只有逻辑代码库两变体：legacy `single_repository` 投影不得
   * 进命令（守卫 `carrierTargetFromEnrollmentTarget` 返回 null ⇒ 禁用提交）。 */
  target: EnrollmentCarrierTarget;
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
    /** 发起侧载体只有逻辑代码库两变体（legacy 单仓载体不得进命令）。 */
    target: EnrollmentCarrierTarget;
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

// C1 Task 6（REQ-WIGA-03）：lease 三态与确认接管 DTO（服务端 snake_case）。
export type LeaseDisposition =
  | "active_wait"
  | "dead_needs_takeover"
  | "unknown_needs_human";

export type LeaseDecision = {
  disposition: LeaseDisposition;
  lease_id: string;
  last_activity_at?: string | null;
  evidence: string[];
};

export type AutomationEnrollmentLeaseTakeoverRequest = {
  command_id: string;
  expected_binding: EnrollmentBindingIdentity;
  expected_lease_id: string;
  expected_attempt_id: string;
};

export type AutomationEnrollmentLeaseTakeoverResult = {
  command_id: string;
  state: AutomationEnrollmentOperationState;
  lease: LeaseDecision;
};

// C1 Task 7（REQ-ADV-C1-RETRY）：Failed advance 显式 retry DTO。
export type AdvanceInitializationCheckpoint =
  | "record_persisted"
  | "journal_prepared"
  | "attempt_persisted"
  | "worktree_bound"
  | "plan_binding_saved"
  | "units_materialized"
  | "ready";

export type AutomationEnrollmentRetryInitializationRequest = {
  command_id: string;
  expected_binding: EnrollmentBindingIdentity;
  expected_attempt_id: string;
  expected_checkpoint: AdvanceInitializationCheckpoint;
  confirm_unknown_side_effect: boolean;
};

export type AutomationEnrollmentRetryInitializationResult = {
  command_id: string;
  state: AutomationEnrollmentOperationState;
  retry: {
    retry_id: string;
    command_id: string;
    advance_id: string;
    attempt_id: string;
    state: AutomationEnrollmentOperationState;
    checkpoint: AdvanceInitializationCheckpoint;
    created_at: string;
    updated_at: string;
  };
  outcome?: unknown | null;
};

// C5 Task 1：只读投影——双载体 enrollment target + 服务端与 prepare
// 同源解析的 options（响应不再携带顶层 logical_repository_id 冗余字段）。
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
  /** C1：双载体 target 投影（Enable/Rebind 原样回传）。 */
  enrollment_target: EnrollmentTarget;
  resolved_options: EnrollmentOptions;
};
