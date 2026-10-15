import type {
  ChoiceAnswer,
  ChoiceOption,
  ExecutionEvent,
  WorkspaceChoiceRequestSource,
} from "./workspace";
import type {
  ProviderConfigSnapshot,
  WorkItemExecutionPlan,
  WorkspaceProviderName,
} from "./common";
import type { CodingAttentionItemDto } from "./lifecycle";
import type {
  PlanAmendmentManifest,
  PlanRepairRequest,
  PlanRepairSessionSnapshot,
  WorkspaceSessionLink,
} from "./coding-plan-repair";

export type {
  ContractDeltaKind,
  ContractImpactReport,
  ContractValidationFinding,
  ImpactExplanationPath,
  PlanAmendmentManifest,
  PlanDefectClass,
  PlanDefectEvidence,
  PlanRepairImpactScopeReview,
  PlanRepairPackageIdentity,
  PlanRepairRequest,
  PlanRepairSessionSnapshot,
  PlanValidationReportArtifact,
  RepairTarget,
  WorkItemPlanReviewComplete,
  WorkspaceSessionLink,
} from "./coding-plan-repair";

export type CodingAttemptStatus =
  | "created"
  | "running"
  | "waiting_for_human"
  | "blocked"
  | "awaiting_manual_recovery"
  | "awaiting_plan_amendment"
  | "applying_plan_amendment"
  | "amendment_apply_failed"
  | "completed"
  | "failed"
  | "aborted";

export type CodingAttemptScope = "work_item" | "work_item_group";

export type CodingAttemptAddress = {
  projectId: string;
  issueId: string;
  attemptId: string;
};

export type CodingExecutionStage =
  | "prepare_context"
  | "worktree_prepare"
  | "coding"
  | "code_review"
  | "review_request"
  | "internal_pr_review"
  | "final_confirm";

export type CodingExecutionUnitStatus =
  | "pending"
  | "running"
  | "waiting_for_human"
  | "completed"
  | "failed"
  | "blocked"
  | "blocked_by_plan_defect"
  | "awaiting_amendment"
  | "needs_revalidation"
  | "stale"
  | "superseded"
  | "skipped";

export type CodingExecutionUnit = {
  unit_id: string;
  logical_work_item_id: string;
  work_item_revision_id: string;
  dependency_logical_work_item_ids: string[];
  order_index: number;
  status: CodingExecutionUnitStatus;
  summary: string | null;
  latest_handoff_revision_id: string | null;
  completion_commit: string | null;
};

export type CodingAttempt = {
  project_id: string;
  issue_id: string;
  attempt_id: string;
  work_item_id: string;
  attempt_scope: CodingAttemptScope;
  work_item_group_id: string | null;
  current_work_item_id: string | null;
  active_unit_id: string | null;
  attempt_no: number;
  status: CodingAttemptStatus;
  stage: CodingExecutionStage;
  branch_name: string;
  base_branch: string;
  worktree_path: string | null;
  rework_count: number;
  head_commit: string | null;
  // 复用 PushStatus 单一值域别名（含 skipped_not_configured 本地交付）；不再内联
  // 复制一份联合类型，避免两套值域漂移。
  push_status: PushStatus | null;
  review_request_url: string | null;
  created_at: string;
  updated_at: string;
};

export type CodingTimelineNodeStatus = "pending" | "running" | "completed" | "failed" | "blocked";
export type CodingAgentRole = "author" | "reviewer" | "git" | "system";
export type CodingProviderRole = "coder" | "code_reviewer" | "internal_reviewer";
export type CodingProviderSelectRole =
  | "author"
  | "reviewer"
  | CodingProviderRole;
export type CodingProviderPermissionMode = "auto" | "supervised";
export type CodingRoleRunStatus =
  | "running"
  | "completed"
  | "failed"
  | "blocked"
  | "superseded"
  | "aborted";
export type CodingRoleRunTrigger =
  | "initial"
  | "automatic_retry"
  | "manual_retry"
  | "retry_review"
  | "retry_internal_review";

export type CodingRoleRunEventType =
  | "provider_prompt"
  | "provider_start"
  | "text_delta"
  | "execution_event"
  | "tool_call"
  | "tool_result"
  | "status_changed"
  | "permission_request"
  | "choice_request"
  | "message_complete"
  | "provider_failed"
  | "timeout"
  | "aborted"
  | "persistence_warning";

export type CodingRoleRunEventSummary = {
  event_count: number;
  last_event_at?: string | null;
  last_event_type?: CodingRoleRunEventType | null;
  last_event_title?: string | null;
  last_event_status?: string | null;
  terminal_event_type?: CodingRoleRunEventType | null;
  terminal_reason?: string | null;
};

export type CodingRoleRunEventPreview = {
  sequence: number;
  event_type: CodingRoleRunEventType;
  created_at: string;
  title?: string | null;
  status?: string | null;
  detail?: string | null;
  truncated: boolean;
  artifact_ref?: string | null;
};

export type CodingRoleRunRetryMetadata = {
  cycle_id: string;
  attempt_no: number;
  prior_run_id?: string | null;
};

export type CodingRolePermissionModes = {
  coder: CodingProviderPermissionMode;
  code_reviewer: CodingProviderPermissionMode;
  internal_reviewer: CodingProviderPermissionMode;
};

export type CodingTimelineNode = {
  id: string;
  attempt_id: string;
  stage: CodingExecutionStage;
  title: string;
  status: CodingTimelineNodeStatus;
  agent_role: CodingAgentRole | null;
  summary: string | null;
  started_at: string;
  completed_at: string | null;
  artifact_refs: string[];
};

export type CodingRoleProviderConfigSnapshot = {
  coder: WorkspaceProviderName;
  code_reviewer: WorkspaceProviderName;
  internal_reviewer: WorkspaceProviderName;
  review_rounds: number;
  permission_modes: CodingRolePermissionModes;
};

export type CodingRoleRun = {
  id: string;
  attempt_id: string;
  stage: CodingExecutionStage;
  role: CodingProviderRole;
  run_no: number;
  status: CodingRoleRunStatus;
  trigger: CodingRoleRunTrigger;
  retry_metadata?: CodingRoleRunRetryMetadata | null;
  retry_exhausted?: boolean;
  node_id: string | null;
  started_at: string;
  completed_at: string | null;
  supersedes_run_id?: string | null;
  superseded_by_run_id?: string | null;
  reason_code?: string | null;
  raw_provider_output_refs: string[];
  artifact_refs: string[];
  event_summary?: CodingRoleRunEventSummary | null;
  recent_events?: CodingRoleRunEventPreview[];
};

export type CodingReviewVerdict = "approve" | "request_changes" | "blocked";
export type FindingSeverity = "error" | "warning" | "info";

export type ReviewFinding = {
  severity: FindingSeverity;
  file_path: string | null;
  line: number | null;
  message: string;
  required_action: string | null;
  source_stage: CodingExecutionStage;
  evidence?: string[];
  related_requirements?: string[];
  related_design_constraints?: string[];
  related_work_item_tasks?: string[];
};

export type CodeReviewReport = {
  id: string;
  attempt_id: string;
  round: number;
  verdict: CodingReviewVerdict;
  findings: ReviewFinding[];
  tested_evidence_refs: string[];
  diff_refs: string[];
  summary: string;
  created_at: string;
  raw_provider_output_ref?: string | null;
  role_run_id?: string | null;
  run_no?: number | null;
};

export type ReviewRequestKind =
  | "git_branch_only"
  | "gitlab_merge_request"
  | "github_pull_request"
  | "manual_external_request";
export type RemoteKind = "github" | "gitlab" | "generic_git" | "unknown";
// push_status 值域。FE-1（设计 Q1 冻结）：新增 skipped_not_configured —— 仓库未
// 配置远端时的本地交付终态，既不是 pushed 也不是 failed；UI 必须与「已推送」
// 区分（标签「本地交付(无远端)」）。
export type PushStatus =
  | "not_pushed"
  | "pushed"
  | "failed"
  | "skipped_not_configured";

export type ReviewRequest = {
  id: string;
  attempt_id: string;
  kind: ReviewRequestKind;
  remote_kind: RemoteKind;
  remote: string;
  base_branch: string;
  branch_name: string;
  commit_sha: string;
  push_status: PushStatus;
  external_url: string | null;
  manual_instructions: string[];
  push_error: string | null;
  created_at: string;
  updated_at: string;
};

export type InternalPrReview = {
  id: string;
  attempt_id: string;
  review_request_id: string;
  verdict: CodingReviewVerdict;
  findings: ReviewFinding[];
  impact_scope: string[];
  pr_description: string;
  commit_message_suggestion: string;
  tested_evidence_refs: string[];
  diff_refs: string[];
  summary: string;
  created_at: string;
  raw_provider_output_ref?: string | null;
  role_run_id?: string | null;
  run_no?: number | null;
};

export type GroupReviewArtifactRef = {
  id: string;
  raw_provider_output_refs: string[];
};

export type GroupReviewArtifactProjection = {
  shard_reports: GroupReviewArtifactRef[];
  reduction_reports: GroupReviewArtifactRef[];
};

export type GroupFinalReadinessStatus = "complete" | "incomplete";

export type GroupFinalReadinessDiagnosticKind =
  | "unit_run_missing"
  | "completion_commit_missing"
  | "code_review_missing"
  | "handoff_missing"
  | "plan_binding_mismatch"
  | "identity_mismatch";

export type GroupFinalReadinessDiagnostic = {
  kind: GroupFinalReadinessDiagnosticKind;
  unit_id: string | null;
  message: string;
};

export type GroupFinalReadinessUnit = {
  unit_id: string;
  logical_work_item_id: string;
  unit_run_id: string | null;
  start_commit: string | null;
  completion_commit: string | null;
  commit_shas: string[];
  diff_ref: string;
  empty_observation: boolean;
  code_review_report_id: string | null;
  review_verdict: CodingReviewVerdict | null;
  review_summary: string | null;
  review_findings: ReviewFinding[] | null;
  review_raw_provider_output_ref: string | null;
  handoff_revision_id: string | null;
  plan_revision_id: string | null;
};

export type GroupFinalReadinessSnapshot = {
  attempt_id: string;
  status: GroupFinalReadinessStatus;
  units: GroupFinalReadinessUnit[];
  diagnostics: GroupFinalReadinessDiagnostic[];
  created_at: string;
};

export type CodingEntryType =
  | { type: "user_message" }
  | { type: "assistant_message" }
  | { type: "tool_call"; tool_name: string; input: unknown }
  | { type: "tool_result"; tool_use_id: string; output: string; is_error: boolean }
  | { type: "stage_gate"; stage: CodingExecutionStage; countdown_seconds: number }
  | { type: "stage_summary"; stage: CodingExecutionStage; summary: string }
  | { type: "system_event"; event_type: string; message: string };

export type CodingChatEntry = {
  id: string;
  attempt_id: string;
  node_id: string | null;
  role: CodingAgentRole;
  entry_type: CodingEntryType;
  content: string | null;
  metadata: Record<string, unknown> | null;
  created_at: string;
};

export type CodingExecutionEventReplay = {
  event: ExecutionEvent;
  created_at: string;
};

export type CodingGateActionType =
  | "send_to_coder"
  | "confirm_stage"
  | "accept_risk"
  | "abort"
  | "retry_push"
  | "manual_fix"
  | "provide_context"
  | "manual_continue"
  | "retry_coding"
  | "retry_coding_with_context"
  | "retry_review"
  | "retry_internal_review"
  | "retry_group_review_shard"
  | "retry_group_reduction";
export type CodingGateKind = "permission" | "stage_gate" | "blocked" | "final_confirm";

export type CodingGateAction = {
  action_id: string;
  label: string;
  action_type: CodingGateActionType;
};

export type CodingGateRequired = {
  gate_id: string;
  kind: CodingGateKind;
  title: string;
  description: string;
  stage?: CodingExecutionStage | null;
  role?: CodingProviderRole | null;
  expires_at?: string | null;
  provider_snapshot?: CodingRoleProviderConfigSnapshot | null;
  available_actions: CodingGateAction[];
  reason_code?: string | null;
  evidence_refs?: string[];
  raw_provider_output_ref?: string | null;
  diagnostic?: CodingGateDiagnostic | null;
  /** P2-c（REQ-CRO-09）：分诊卡三段大白话投影；旧门 JSON 无该字段 → 前端回落 description。 */
  triage_summary?: CodingTriageSummary | null;
};

export type CodingGateDiagnostic = {
  actual_value?: string | null;
  limit?: string | null;
  phase: string;
  run_failure_code: string;
};

/** P2-d（REQ-CRO-09）：与后端 `CodingTriageSummary` 同构（块 2 设计 §4.2）。 */
export type CodingTriageFinding = {
  reason_code: string;
  message: string;
};

export type CodingTriageDid = {
  changed_files: string[];
  commit_sha?: string | null;
  verification_exit_code?: number | null;
  environment_note?: string | null;
};

export type CodingTriageSummary = {
  problem: string;
  findings?: CodingTriageFinding[];
  did?: CodingTriageDid | null;
  unverified?: string[];
};

/** C2 Task 8（#19）：attempt-scoped 验证处理记录（旁路 DTO，不进门动作枚举）。 */
export type VerificationTriageStatus = "pending" | "approved" | "rejected";

export type VerificationTriageConclusion =
  | "approve_plan_revision"
  | "accept_equivalent_evidence"
  | "grant_scoped_environment_exception";

export type VerificationTriageRecord = {
  triage_id: string;
  attempt_id: string;
  finding_id: string;
  check_id: string;
  plan_revision_id: string;
  original_command?: string | null;
  alternative_command?: string | null;
  cwd?: string | null;
  outcome?: string | null;
  test_execution_count?: number | null;
  environment?: string | null;
  scope: string[];
  expires_at: string;
  status: VerificationTriageStatus;
  conclusion?: VerificationTriageConclusion | null;
  reason?: string | null;
  decided_by?: string | null;
  decided_at?: string | null;
};

/** C2 Task 9（#2/#9）：计划命令与实际命令并列证据。actual 缺失 → "未记录"。 */
export type VerificationCommandEvidence = {
  check_id: string;
  planned_command?: string | null;
  planned_manual_instruction?: string | null;
  actual_command?: string | null;
  actual_cwd?: string | null;
  exit_code?: number | null;
  test_execution_count?: number | null;
  environment_summary?: string | null;
  mismatch: boolean;
};

export type CodingChoiceGateStatus = "open" | "resolved" | "stale" | "cancelled";

export type CodingChoiceGateResponse = {
  selected_option_ids: string[];
  free_text?: string | null;
  responded_at: string;
  /** P0 1.3：完整逐题答案原样落盘；旧单题 gate 缺省为空。 */
  answers?: ChoiceAnswer[];
};

/** P0 1.3：coding choice 逐题结构（与 workspace `ChoiceQuestion` 同构）。 */
export type CodingChoiceQuestion = {
  id: string;
  prompt: string;
  options: ChoiceOption[];
  allow_multiple: boolean;
  allow_free_text: boolean;
};

export type CodingChoiceGate = {
  gate_id: string;
  choice_id: string;
  attempt_id: string;
  node_id?: string | null;
  stage: CodingExecutionStage;
  role: CodingProviderRole;
  provider: WorkspaceProviderName;
  source: WorkspaceChoiceRequestSource;
  prompt: string;
  options: ChoiceOption[];
  allow_multiple: boolean;
  allow_free_text: boolean;
  status: CodingChoiceGateStatus;
  response?: CodingChoiceGateResponse | null;
  /** P0 1.3：多问题透传；旧 gate 缺省为空（前端按单一 default 题兼容）。 */
  questions?: CodingChoiceQuestion[];
  /**
   * P0 1.3（REQ-WIGA-05）Task 11：应答须绑定的 run 化身（REST claim 精确匹配
   * active incarnation）；服务端 handler 层注入——无活跃 run 时缺省，不猜 run。
   */
  expected_run_id?: string | null;
  created_at: string;
  updated_at: string;
};

export type CodingAttemptSnapshotResponse = {
  attempt: CodingAttempt;
  attempt_scope: CodingAttemptScope;
  work_item_group_id: string | null;
  current_work_item_id: string | null;
  active_unit_id: string | null;
  units: CodingExecutionUnit[];
  provider_config_snapshot: ProviderConfigSnapshot;
  timeline_nodes: CodingTimelineNode[];
  active_node_id: string | null;
  code_review_reports: CodeReviewReport[];
  review_request: ReviewRequest | null;
  internal_pr_review: InternalPrReview | null;
  group_review_artifacts?: GroupReviewArtifactProjection | null;
  group_final_readiness?: GroupFinalReadinessSnapshot | null;
  pending_gates: CodingGateRequired[];
  pending_choices: CodingChoiceGate[];
  role_runs?: CodingRoleRun[];
  work_item_execution_plan: WorkItemExecutionPlan | null;
  require_execution_plan_confirm: boolean;
};

export type CodingAttemptDiffResponse = {
  attempt_id: string;
  base_branch: string;
  worktree_path: string;
  diff: string;
};

export type ArtifactContentResponse = {
  artifact_ref: string;
  artifact_kind: string;
  producer_node: string | null;
  path: string;
  content_type: string;
  content: string;
};

// ── N6（REQ-CS-01/06）：plan 一键启动与聚合进度 DTO（N3 冻结，前端只读镜像）──

export type PlanCodingStartRequest = {
  command_id: string;
  expected_plan_revision_id: string;
};

// lease_state 沿 repo worktree lock 事实（absent|held|contested），不猜 provider 运行。
export type PlanCodingLeaseState = "absent" | "held" | "contested";

// execution_state 值域 initialized|running|waiting_dependency|waiting_slot|paused|
// completed|invalidated——provider 事实与初始化桶首 unit Running（物化事实）
// 严格区分；invalidated 为 plan-run 授权失效投影（成员删除/精确 purge 后），
// 不与 unit 事实状态混写。
// FE-1（设计 Q3 冻结，additive）：attempt 状态守卫生效后新增
// awaiting_manual_recovery|waiting_human|failed|aborted —— unit 仍是 Running
// 但 attempt 已进入人工恢复/等待/终态时，投影不得继续报 provider 运行中。
export type PlanCodingExecutionState =
  | "initialized"
  | "running"
  | "waiting_dependency"
  | "waiting_slot"
  | "paused"
  | "completed"
  | "invalidated"
  | "awaiting_manual_recovery"
  | "waiting_human"
  | "failed"
  | "aborted";

// waiting_reason 值域 dependency_not_ready|execution_slot_occupied|
// plan_coding_continuation_not_enabled（依赖优先于暂停文案）——三种「编排等待」
// 语义仍由本联合类型描述。
// FE-1（设计 Q3）：awaiting_manual_recovery 时 waiting_reason 透传
// attempt.manual_recovery_reason（开放值集，如 coding_runner_failed_while_running），
// 故 DTO 字段放宽为 string | null，本类型只作为编排等待子集保留。
export type PlanCodingWaitingReason =
  | "dependency_not_ready"
  | "execution_slot_occupied"
  | "plan_coding_continuation_not_enabled";

// operation_state 沿现 accepted|replayed|needs_human|rejected 语义。
export type PlanCodingOperationState =
  | "accepted"
  | "replayed"
  | "needs_human"
  | "rejected";

// N2 tagged 首启真实回执（kind=started|already_started|needs_human）。
export type CodingPlanStartOutcome =
  | { kind: "started"; attempt_id: string }
  | { kind: "already_started"; attempt_id: string }
  | { kind: "needs_human"; attempt_id: string; reason: string };

// FE-1（设计 Q5 / S3 后端契约冻结）：PlanCodingTargetDto 补九字段逐仓事实镜像，
// 供 coding 页在多 target（work_item_group）时逐仓渲染 PR 面板；单仓/非 group 的
// 顶部 header 零变化。字段名与 S3 后端 serde snake_case 一致，类型均 string|null
// （stage/push_status 保持字符串口径，未命中标签时回落原值）。
export type PlanCodingTargetDto = {
  target_repository_id: string;
  attempt_id: string;
  attempt_status: CodingAttemptStatus;
  worktree_path: string | null;
  lease_state: PlanCodingLeaseState;
  repository_name: string | null;
  branch_name: string | null;
  base_branch: string | null;
  stage: string | null;
  head_commit: string | null;
  push_status: string | null;
  push_error: string | null;
  review_request_id: string | null;
  manual_recovery_reason: string | null;
};

export type PlanCodingUnitProgressDto = {
  unit_id: string;
  logical_work_item_id: string;
  target_repository_id: string;
  attempt_id: string;
  unit_status: CodingExecutionUnitStatus;
  execution_state: PlanCodingExecutionState;
  unsatisfied_dependency_ids: string[];
  // 编排等待取 PlanCodingWaitingReason 三值；awaiting_manual_recovery 时为
  // attempt.manual_recovery_reason 原文（开放值集），故放宽为 string。
  waiting_reason: string | null;
  // Slice B（仪表盘跨仓依赖投影）：全量直接依赖（权威绑定，跨桶不失真）、
  // 全图拓扑序与桶内摘要，供前端重建 plan 全图拓扑/依赖链。
  dependency_logical_work_item_ids: string[];
  order_index: number;
  summary: string | null;
};

// 按 bound target attempts 聚合的 plan 进度；primary 仅打开入口，不代表整 plan
// 执行或完成（plan_complete 只在全部 authoritative units Completed 时为 true）。
export type PlanCodingProgressDto = {
  plan_id: string;
  bound_plan_revision_id: string;
  command_id: string | null;
  mode: string | null;
  phase: string | null;
  primary_attempt_id: string | null;
  selected_unit_id: string | null;
  selected_attempt_id: string | null;
  target_attempts: PlanCodingTargetDto[];
  units: PlanCodingUnitProgressDto[];
  // P1-b（REQ-MTG-11 / 契约 A2）：plan 级聚合最终确认就绪投影——state 值域
  // not_ready|ready|partial|done（Confirmed 视为已确认）。additive：旧响应
  // 缺省按 not_ready 空投影处理（服务端 serde default 同口径）。
  final_confirm?: PlanCodingFinalConfirmDto;
  // P1-b（REQ-MTG-12）：本 plan 同源待处理事项（块 4 A1 镜像），总览待办
  // 徽标消费。additive：旧响应缺省按无待办处理。
  attention_items?: CodingAttentionItemDto[];
  plan_complete: boolean;
  // H5（设计 C 节冻结，additive）：workspace_lifecycle 事实——generation、现存/
  // 缺失成员数、状态；数据来自服务端交叉核验，不从 attempt_status 猜物理存在
  // 性。旧响应缺省（H4 端点后行）。
  workspace_lifecycle?: PlanWorkspaceLifecycleDto | null;
};

// workspace_lifecycle.state 值域：clean=无执行内容、complete=整轮完成、
// incomplete=成员缺失（部分删除后）、clearing=清理进行中（旧 runner 退出中）。
export type PlanWorkspaceLifecycleState =
  | "clean"
  | "complete"
  | "incomplete"
  | "clearing";

export type PlanWorkspaceLifecycleDto = {
  generation: number;
  existing_member_count: number;
  missing_member_count: number;
  state: PlanWorkspaceLifecycleState;
};

// P1-b（REQ-MTG-11 / 契约 A2）：未齐备的冻结 target（GET `blocking` 条目；
// POST not_ready 回执的失败项同构）。
export type PlanCodingFinalConfirmBlockingDto = {
  attempt_id: string;
  reason_code: string;
  message: string;
};

// P1-b（REQ-MTG-11 / 契约 A2）：plan 级聚合最终确认投影——Confirmed 视为已
// 确认；全 Completed → done；部分 Completed 且剩余全就绪 → partial；零确认且
// 全就绪 → ready；其余（含 blocking 非空/plan 未执行完成）→ not_ready。
export type PlanCodingFinalConfirmDto = {
  state: PlanCodingFinalConfirmState;
  ready_attempt_ids: string[];
  confirmed_attempt_ids: string[];
  blocking: PlanCodingFinalConfirmBlockingDto[];
};

export type PlanCodingFinalConfirmState =
  | "not_ready"
  | "ready"
  | "partial"
  | "done";

// P1-b（契约 A2）：POST `.../coding-final-confirm` 回执逐仓结果——
// status 值域 confirmed|already_completed|failed|not_attempted。
export type PlanCodingFinalConfirmResultItemDto = {
  attempt_id: string;
  target_repository_id: string;
  status: PlanCodingFinalConfirmResultStatus;
  error_code?: string | null;
  message?: string | null;
};

export type PlanCodingFinalConfirmResultStatus =
  | "confirmed"
  | "already_completed"
  | "failed"
  | "not_attempted";

// P1-b（契约 A2）：POST 回执——outcome 值域 all_confirmed|partially_confirmed|
// not_ready；partial 只表示本次未全成，成功数按 `results` 计算（失败仓
// error_code/message 可见；不加 command_id/generation 防重放）。
export type PlanCodingFinalConfirmResultDto = {
  plan_id: string;
  outcome: PlanCodingFinalConfirmOutcome;
  results: PlanCodingFinalConfirmResultItemDto[];
};

export type PlanCodingFinalConfirmOutcome =
  | "all_confirmed"
  | "partially_confirmed"
  | "not_ready";

// H5（设计 C 节冻结）：DELETE …/coding-workspaces 成功 200 回执；重复同
// command 同 generation 返回 replayed，不扫下一代刚创建的 attempts。
export type PlanCodingWorkspaceDeletionResultDto = {
  plan_id: string;
  command_id: string;
  operation_state: "accepted" | "replayed";
  generation: number;
  deleted_attempt_ids: string[];
  remaining_attempt_ids: string[];
};

// 一键启动结果：start_outcome 为冻结首启回执；waiting_units 为等待绑定集合。
export type PlanCodingStartResultDto = {
  plan_id: string;
  bound_plan_revision_id: string;
  command_id: string;
  operation_state: PlanCodingOperationState;
  primary_attempt_id: string | null;
  target_attempts: PlanCodingTargetDto[];
  selected_unit_id: string | null;
  selected_attempt_id: string | null;
  start_outcome: CodingPlanStartOutcome | null;
  waiting_units: PlanCodingUnitProgressDto[];
  progress: PlanCodingProgressDto;
};

export type CodingWsInMessage =
  | { type: "coding_hello"; attempt_id: string; last_seen_node_id?: string | null }
  | { type: "start_coding" }
  | { type: "context_note"; content: string }
  | { type: "permission_response"; id: string; approved: boolean; reason?: string | null }
  | {
      type: "choice_response";
      id: string;
      selected_option_ids: string[];
      free_text?: string | null;
    }
  | {
      type: "gate_response";
      gate_id: string;
      action_id: string;
      extra_context?: string | null;
    }
  | { type: "provider_select"; role: CodingProviderSelectRole; provider: WorkspaceProviderName }
  | {
      type: "permission_mode_select";
      role: CodingProviderRole;
      permission_mode: CodingProviderPermissionMode;
    }
  | { type: "max_auto_rework_select"; max_auto_rework: number }
  | { type: "stage_gate_confirm"; stage: CodingExecutionStage }
  | { type: "final_confirm" }
  | { type: "retry_push" }
  | { type: "abort_attempt" }
  /** F-16：awaiting_manual_recovery 显式恢复动作（重走 admission CAS 回 running）。 */
  | { type: "recover_coding" }
  /** F-44：aborted/failed 终态显式重新开始动作（重走 admission CAS 回 running 并重启 runner）。 */
  | { type: "restart_coding" }
  | { type: "request_manual_pause" }
  | { type: "coding_ping" };

export type CodingWsOutMessage =
  | ({
      type: "coding_session_state";
      project_id: string;
      issue_id: string;
      attempt_id: string;
      attempt_scope: CodingAttemptScope;
      work_item_group_id: string | null;
      current_work_item_id: string | null;
      active_unit_id: string | null;
      units: CodingExecutionUnit[];
      status: CodingAttemptStatus;
      stage: CodingExecutionStage;
      branch_name: string;
      base_branch: string;
      worktree_path: string | null;
      rework_count: number;
      max_auto_rework: number;
      head_commit: string | null;
      pushed_remote: string | null;
      role_provider_config_snapshot: CodingRoleProviderConfigSnapshot;
      chat_entries: CodingChatEntry[];
      execution_events?: CodingExecutionEventReplay[];
      work_item_markdown: string | null;
      verification_commands: string[];
      work_item_execution_plan: WorkItemExecutionPlan | null;
      linked_plan_repair: PlanRepairSessionSnapshot | null;
      group_review_artifacts?: GroupReviewArtifactProjection | null;
      require_execution_plan_confirm: boolean;
    } & Omit<CodingAttemptSnapshotResponse, "attempt">)
  | { type: "coding_stage_change"; stage: CodingExecutionStage }
  | { type: "coding_timeline_node_created"; node: CodingTimelineNode }
  | {
      type: "coding_timeline_node_updated";
      node_id: string;
      status: CodingTimelineNodeStatus;
      summary?: string | null;
      completed_at?: string | null;
    }
  | { type: "coding_execution_event"; event: ExecutionEvent }
  | {
      type: "coding_permission_request";
      id: string;
      tool_name: string;
      description: string;
      risk_level: "low" | "medium" | "high";
    }
  | {
      type: "coding_choice_request";
      id: string;
      prompt: string;
      source: WorkspaceChoiceRequestSource;
      options: ChoiceOption[];
      allow_multiple: boolean;
      allow_free_text: boolean;
      /** P2-e（REQ-CRO-11）：多题透传（与快照 `CodingChoiceGate.questions` 同形）；旧帧缺省。 */
      questions?: CodingChoiceQuestion[];
    }
  | {
      type: "coding_choice_response_ack";
      id: string;
      selected_option_ids: string[];
      free_text?: string | null;
    }
  | { type: "coding_stream_chunk"; content: string; node_id?: string | null }
  | { type: "coding_message_complete"; node_id?: string | null }
  | { type: "code_review_complete"; report: CodeReviewReport }
  | { type: "review_request_update"; review_request: ReviewRequest }
  | { type: "internal_pr_review_complete"; review: InternalPrReview }
  | { type: "coding_gate_required"; gate: CodingGateRequired }
  | { type: "coding_chat_entry_created"; entry: CodingChatEntry }
  | {
      type: "coding_provider_config_updated";
      role: CodingProviderRole;
      provider: WorkspaceProviderName;
    }
  | { type: "coding_protocol_error"; code: string; message: string }
  | { type: "coding_pong" }
  | {
      type: "plan_repair_required";
      request: PlanRepairRequest;
      session_link: WorkspaceSessionLink | null;
    }
  | {
      type: "plan_amendment_updated";
      event_id: string;
      amendment: PlanAmendmentManifest;
    };
