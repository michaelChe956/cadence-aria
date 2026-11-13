import type {
  ProviderConfigSnapshot,
  ProviderPermissionMode,
  ProviderWorkspaceConfigInput,
  ReviewGate,
  ReviewVerdictType,
  StructuredFeedback,
  WorkspaceProviderName,
  WorkspaceReviewFindingSeverity,
} from "./common";
import type {
  HumanPresentationRevision,
  PlanProjectionBundle,
  ProjectionValidationReport,
  WorkItemBatchStatePayload,
  WorkItemDraftCandidatePayload,
  WorkItemPlanArtifactPayload,
  WorkItemPlanCandidateDto,
  WorkItemPlanCompileReportPayload,
  WorkItemPlanContextBlockerPayload,
  WorkItemPlanOutlineCandidatePayload,
  WorkItemProjectionBundle,
  WorkItemRevisionHistoryDto,
} from "./work-item-plan";
import type {
  PlanAmendmentManifest,
  PlanRepairSessionSnapshot,
  WorkspaceSessionLink,
} from "./coding-plan-repair";
import type { ScFailureRepairPackage } from "./sc-failure-repair";

export type WorkspaceMessage = {
  role: string;
  content: string;
  created_at: string;
};

export type WorkspaceSessionStatus =
  | "open"
  | "running"
  | "waiting_for_human"
  | "confirmed"
  | "change_requested"
  | "blocked_provider_unavailable"
  | "terminated"
  | "stopped_needs_human"
  | "failed";

export type WorkItemPlanFlowKind = "legacy" | "single_candidate";
export type WorkItemPlanRunPolicy = "interactive" | "auto_if_valid";
export type WorkItemPlanSingleCandidatePhase =
  | "prepare"
  | "generate"
  | "evaluate"
  | "approval"
  | "completed"
  | "failed";
export type WorkItemPlanRunHistory = {
  seen_fingerprints: string[];
  repairs_used: number;
  manual_repairs_used: number;
  transitions_used: number;
  initial_review_count: number;
  verification_review_count: number;
  review_cycles?: Record<
    string,
    { repairs_used: number; initial_count: number; verification_count: number }
  >;
};

export type WorkItemPlanHumanGateSnapshot = {
  findings: Array<{
    class: "mechanical_error" | "repairable" | "human_required" | "advisory";
    fingerprint: string;
    category: string | null;
    severity: string;
    message: string;
    evidence: string | null;
    required_action: string | null;
    contract_field: string | null;
    /** C1（REQ-TOP-04 场景 4）：无稳定 ID 措辞域身份——true 时跨轮对比禁推断。 */
    identity_unstable?: boolean;
  }>;
  repeated_fingerprints: string[];
  attempts_used: number;
  manual_repairs_remaining: number;
  /** C2（REQ-HGC-01）：gate-local 已接受反馈轮次；旧会话缺席=预算历史不可用。 */
  accepted_feedback_turns?: number | null;
  trigger:
    | "native_human_required"
    | "repeated_fingerprint"
    | "verification_new_findings"
    | "repair_budget_exhausted";
  resumable: boolean;
};

/**
 * REQ-WSC-18（surface-compile-failure-on-human-gate）：**当前候选**的
 * Final Compile 失败事实（Rust `session_state` 顶层字段，与
 * `human_gate_turn_rejection` 同族同层）——失败属于旧候选/旧会话或无失败
 * 时整体省略（serde skip），不构成「候选编译不过」态。
 * - `code`：稳定机器码（如 `final_compile_prepare_failed`）。
 * - `message`：完整人话原因（不截断）。
 * - `detail`：完整诊断原文（含 failure_id），可选。
 * - `rewrite_guidance` / `owner_hint`：owner 2026-10-31 意见 2——重新生成
 *   着重要点与 design 可能需重写的提醒（与 author 教学重驱同源）。
 */
export type WorkItemPlanCandidateCompileFailure = {
  code: string;
  message: string;
  detail?: string;
  rewrite_guidance?: string[];
  owner_hint?: string;
  /**
   * owner 2026-10-31 追加裁定（后端已确认为最终形状）：自动重写两阶段
   * 事实，与失败事实同一载体（同一「当前候选」判据）；数字均为后端权威
   * 值，前端不自数。缺席=无自动重写信息（旧记录/非编译类失败），按单阶
   * 段呈现，不兜底猜 running。
   * - `running`：首次候选无效失败即出现（与是否还在重写无关，第一次就给
   *   owner 看）；`remaining` = 该会话剩余自动重写预算。
   * - `stopped`：同一依赖环结构指纹（成员+边集合规范化摘要）重复出现 →
   *   plan 层修不了，提前停剩余预算；`repeated` = 该环已出现次数；此态
   *   进人工门 + 升级提示「自动重写无法解决，请改 design」。
   */
  auto_rewrite?: {
    phase: "running" | "stopped";
    remaining?: number | null;
    repeated?: number | null;
  };
};

export type WorkItemPlanRepairReservation = {
  token: string;
  owner_session_id: string;
  owner_run_id: string;
  provider_start_idempotency_key: string;
  state: "reserved" | "provider_started" | "committed" | "released";
  commit_id: string | null;
};

export type WorkItemPlanPolicyDiagnostic = {
  code: string;
  message: string;
  field: string | null;
};

/**
 * SG3b：Final Compile typed 失败的最低必要 wire 投影（Rust
 * `workspace_ws_types::out::PlanningCompileFailureDto` 镜像）。同一
 * failure_id/approval/source context 直达 WS ProtocolError.context 与
 * REST 422 details；机器修复/预算/快照字段由 SG7 扩展。
 */
export type PlanningCompileFailurePresentation = {
  failure_id: string;
  kind: "structure" | "operational";
  stable_code: string;
  approval_attempt_id?: string | null;
  source_revision_ref?: string | null;
  plan_candidate_ir_ref?: string | null;
  mechanical_report_ref?: string | null;
  context_digest?: string | null;
  compile_run_id?: string | null;
  cause?: string | null;
  structure_findings?: Array<{
    code: string;
    category: string;
    affected_ids: Array<string>;
  }>;
  /** 仅本 close 运行（compile_run_id 匹配）的 Failed tx 携带时非空。 */
  validator_findings?: Array<{
    severity: string;
    code: string;
    message: string;
    work_item_ids: Array<string>;
  }>;
};

/** SG7（Rust `workspace_ws_types::review::MachineRepairStatus` 镜像）：
 * 服务端已证 L1 且显式配对时唯一生产值 `offered`；执行态沿结构治理
 * snapshot 的 grant/receipt 投影，不在 DTO 复制第二状态机。 */
export type MachineRepairStatus = "offered";

/** SG7（Rust `workspace_ws_types::review::MachineRepairDto` 镜像）：机器
 * 修复 finding 的服务端只读投影——identity 字段逐字来自已证 L1 的
 * `StructuralRepairInstruction`，`instruction_ref` 按 instruction digest
 * 内容寻址。只读呈现用，不构成入站许可；Raw provider 输出恒不携带。 */
export type MachineRepairPresentation = {
  repair_id: string;
  failure_id: string;
  source_hash: string;
  context_digest: string;
  status: MachineRepairStatus;
  instruction_ref: string;
};

/** SG7（Rust `workspace_ws_types::out::PlanningFailurePresentation` 镜像）：
 * 单条 typed 结构失败的完整 wire presentation；与失败结果/snapshot 同
 * failure_id 家族，机器修复仅在显式配对时携带。 */
export type PlanningFailurePresentation = {
  failure_id: string;
  code: string;
  category:
    | "grammar"
    | "dependency"
    | "attribution"
    | "regression"
    | "operational"
    | "unknown";
  origin:
    | "compiler"
    | "canonical_contract"
    | "split_preflight"
    | "target_attribution"
    | "effective_graph"
    | "projection"
    | "system";
  stage: "story" | "design" | "plan" | "coding" | "review";
  affected_ids?: Array<string>;
  details?: Record<string, string>;
  diagnostic_ref: string;
  source_ref: string;
  source_hash: string;
  machine_repair?: MachineRepairPresentation | null;
};

/** SG7/SG7a（Rust `workspace_ws_types::out::StructuralGrantSnapshot` 镜像）：
 * 结构修订授权窗口摘要（只读投影；缺字段 null/Unknown 无阶段权）。 */
export type StructuralGrantSnapshot = {
  grant_id: string;
  authorization_hash: string;
  turn_id?: string | null;
  command_id?: string | null;
  candidate_source_revision_ref?: string | null;
  plan_candidate_ir_ref?: string | null;
  mechanical_report_ref?: string | null;
  phase?: string | null;
  attempt_no: number;
  prompt_ref?: string | null;
  prompt_digest?: string | null;
  instruction_digest?: string | null;
  failing_source_hash?: string | null;
  review_identity?: string | null;
  granted_at: string;
};


/** SG7（Rust `models::UserBudgetHold` 镜像）：活动用户预算 hold
 * （按 turn 幂等；不含 home/secret）。 */
export type UserBudgetHold = {
  turn_id: string;
  command_id: string;
  source_hash: string;
  held_at: string;
};

/** SG7/SG7a（Rust `models::UserBudgetReceipt`＝out.rs `UserBudgetReceiptDto`
 * 镜像）：三终态用户预算 receipt——delivered/nonstructural_failure 计费、
 * released_structural 不实扣，按 turn 幂等互斥。 */
export type UserBudgetReceipt = {
  kind: "delivered_charge" | "nonstructural_failure_charge" | "released_structural";
  turn_id: string;
  command_id: string;
  attempt_reason?:
    | "human_revision"
    | "mechanical_recovery"
    | "structural_repair"
    | "explicit_fresh"
    | null;
  source_revision_ref?: string | null;
  plan_candidate_ir_ref?: string | null;
  structure_report_ref?: string | null;
  context_digest?: string | null;
  reason?: string | null;
  settled_at: string;
};
/** SG7/SG7a（Rust `workspace_ws_types::out::StructuralGovernanceSnapshot`
 * 镜像）：结构治理只读快照——accepted 单源沿 gate 快照、actual start
 * 单源沿 provider ledger，receipt/launch 摘要不含 home/secret；本快照
 * 不构成任何启动/授权。 */
export type StructuralGovernanceSnapshot = {
  current_grant?: StructuralGrantSnapshot | null;
  user_budget_total: number;
  available_user_budget: number;
  active_hold_count: number;
  user_holds?: Array<UserBudgetHold>;
  user_receipts?: Array<UserBudgetReceipt>;
  structure_checkpoints?: Array<unknown>;
  resolved_structure_facts?: Array<unknown>;
  source_ref?: string | null;
  plan_candidate_ir_ref?: string | null;
  structure_report_ref?: string | null;
  context_digest?: string | null;
  accepted_feedback_turns?: number | null;
  actual_provider_starts: number;
  current_turn_summary?: string | null;
  launch_reason?: string | null;
  launch_evidence?: unknown | null;
};

export type WorkItemPlanProviderStartLedgerEntry = {
  provider_start_idempotency_key: string;
  started: boolean;
};

/**
 * P0 1.2（REQ-WIGA-08）：durable automation 归属投影——与 WS session_state
 * `automation` 同形。HTTP 出口恒存在（读取失败在服务端显式报错）；WS 帧
 * 可省略（未知 → 前端保持 null，不猜 client）。
 */
export type AutomationOwnership = {
  owner: "client" | "server";
  enrollment_id: string | null;
  policy_revision: number | null;
  enabled: boolean;
};

export type WorkspaceSessionSummary = {
  workspace_session_id: string;
  issue_id: string;
  entity_id: string;
  workspace_type: "story" | "design" | "work_item" | "work_item_plan";
  status: WorkspaceSessionStatus;
  author_provider: WorkspaceProviderName;
  reviewer_provider: WorkspaceProviderName;
  review_rounds: number;
  superpowers_enabled: boolean;
  openspec_enabled: boolean;
  automation: AutomationOwnership;
};

export type WorkspaceSession = WorkspaceSessionSummary & {
  messages: WorkspaceMessage[];
  single_candidate_phase?: WorkItemPlanSingleCandidatePhase | null;
  work_item_plan_source_revision_ref?: string | null;
  plan_candidate_ir_ref?: string | null;
  mechanical_report_ref?: string | null;
  publication_provenance_ref?: string | null;
};

export type TakeoverResponse = WorkspaceSession & {
  parent_session_id: string;
  takeover_event_id: string;
};

// REQ-DLS-03：只读租约诊断端点响应（活跃 manager 内存快照优先，回收后
// durable jsonl 尾读回退——两形态同构）。事件行只声明消费面字段，其余打点
// 字段（role/reason/from_holder/to_holder/epoch 等）由前端防御性解析忽略。
export interface LeaseDiagnosticsResponse {
  session_id: string;
  holder: string | null;
  epoch: number | null;
  last_holder: string | null;
  events: unknown[];
}

// P15（A4 wire）：WS artifact_update／REST artifact／session_state artifact 三入口共用的原始键联合；原 markdown/candidate/outline/context/draft/batch/compile/projection/history/validation/manifest 十二键全保留，新增 sc_failure_repair_package 独立键（Plan §2.2）。
export type WorkspaceArtifactWire =
  | { markdown: string; diff?: string | null }
  | { candidate: WorkItemPlanCandidateDto }
  | { outline_candidate: WorkItemPlanOutlineCandidatePayload }
  | { context_blocker: WorkItemPlanContextBlockerPayload }
  | { draft_candidate: WorkItemDraftCandidatePayload }
  | { batch_state: WorkItemBatchStatePayload }
  | { compile_report: WorkItemPlanCompileReportPayload }
  | { plan_projection: PlanProjectionBundle }
  | { work_item_projection: WorkItemProjectionBundle }
  | { work_item_revision_history: WorkItemRevisionHistoryDto }
  | { projection_validation: ProjectionValidationReport }
  | { plan_amendment_manifest: PlanAmendmentManifest }
  | { sc_failure_repair_package: ScFailureRepairPackage };

export type ArtifactUpdateMessage = { type: "artifact_update"; version: number } & WorkspaceArtifactWire;

// 退役留档（T5/REQ-RET-02）：RevertWorkItemMessage/SaveHumanPresentationRevision
// 消息族类型随 wire 变体删除（wp5-attribution-table.md §1/§2）。

export type WorkItemPlanCompileRecoveryAction =
  | "continue"
  | "abort_and_rollback"
  | "human_triage";

export type RecoverableInterruptedRun = {
  failed_node_id: string;
  operation: "review" | "work_item_draft_generation";
  label: string;
};

// C1（Task 6.1）：LC 恢复 wire 投影的 TS 镜像——与后端
// src/web/workspace_ws_types/out.rs 的 LcResumeRecoveryDto/LcFreshOperationDto
//（serde snake_case）逐字段一致；native session ID、full fingerprint 与秘密
// 配置不在对外 DTO 中。resolved 保留证据与 last_operation 供 Replay。
export type LcResumeRecoveryState =
  | "waiting"
  | "fresh_reserved"
  | "fresh_running"
  | "resolved";

export type LcFreshOperationState =
  | "not_started"
  | "reserved"
  | "running"
  | "completed"
  | "failed"
  | "unknown";

/** 设计 §5.1：显式 fresh operation 的受理/回放投递标记。 */
export type LcResumeRecoveryDelivery = "accepted" | "replayed";

/** SessionState.lc_resume_recovery.last_operation（接口字典：
 * operation_id/state/reason?/fresh_node?）。 */
export type LcFreshOperationDto = {
  operation_id: string;
  state: LcFreshOperationState;
  reason?: string | null;
  fresh_node?: string | null;
};

export type LcResumeRecoveryDto = {
  recovery_id: string;
  state: LcResumeRecoveryState;
  reason_code: string;
  provider: WorkspaceProviderName;
  failed_node_id: string;
  run_kind: string;
  detected_at: string;
  updated_at: string;
  /** A/B 发布纪律：B 波接齐前恒空集；前端镜像事实，不推导动作。 */
  allowed_actions: string[];
  last_operation?: LcFreshOperationDto | null;
};

const LC_RESUME_RECOVERY_STATES: Record<string, true> = {
  waiting: true,
  fresh_reserved: true,
  fresh_running: true,
  resolved: true,
};

const LC_FRESH_OPERATION_STATES: Record<string, true> = {
  not_started: true,
  reserved: true,
  running: true,
  completed: true,
  failed: true,
  unknown: true,
};

function isNonEmptyString(value: unknown): value is string {
  return typeof value === "string" && value.length > 0;
}

/**
 * C1（Task 6.1）运行时守卫：session_state.lc_resume_recovery 的形状校验。
 * wire 无编译期保障，畸形载荷不得进入 store 半成品事实（调用方决定保留
 * 旧投影或落 null）；state 枚举严格校验（驱动卡可见性），provider/run_kind
 * 只验非空字符串（后端扩展不致前端丢弃整个恢复事实）。
 */
export function isLcResumeRecoveryDto(value: unknown): value is LcResumeRecoveryDto {
  if (typeof value !== "object" || value === null) {
    return false;
  }
  const record = value as Record<string, unknown>;
  if (
    !isNonEmptyString(record.recovery_id) ||
    !LC_RESUME_RECOVERY_STATES[record.state as string] ||
    !isNonEmptyString(record.reason_code) ||
    !isNonEmptyString(record.provider) ||
    !isNonEmptyString(record.failed_node_id) ||
    !isNonEmptyString(record.run_kind) ||
    !isNonEmptyString(record.detected_at) ||
    !isNonEmptyString(record.updated_at) ||
    !Array.isArray(record.allowed_actions) ||
    !record.allowed_actions.every((action) => isNonEmptyString(action))
  ) {
    return false;
  }
  const lastOperation = record.last_operation;
  if (lastOperation === null || lastOperation === undefined) {
    return true;
  }
  if (typeof lastOperation !== "object") {
    return false;
  }
  const operation = lastOperation as Record<string, unknown>;
  return (
    isNonEmptyString(operation.operation_id) &&
    LC_FRESH_OPERATION_STATES[operation.state as string] &&
    (operation.reason === undefined ||
      operation.reason === null ||
      typeof operation.reason === "string") &&
    (operation.fresh_node === undefined ||
      operation.fresh_node === null ||
      typeof operation.fresh_node === "string")
  );
}

export type LinkedWorkspaceAmendmentTarget = {
  entity_id: string;
  workspace_type: "story" | "design";
  relation: "story_amendment" | "design_amendment";
};

export type LinkedWorkspaceSessionSnapshot = {
  link: WorkspaceSessionLink;
  entity_id: string;
  workspace_type: "story" | "design" | "work_item" | "work_item_plan";
  artifact_version_id: number | null;
  timeline_nodes: TimelineNode[];
  selected_timeline_node_id: string | null;
  human_confirm_state:
    | "open"
    | "running"
    | "waiting_for_human"
    | "confirmed"
    | "change_requested"
    | "blocked_provider_unavailable"
    | "terminated";
};

export type WsInMessage =
  | { type: "user_message"; content: string }
  | { type: "context_note"; content: string }
  | {
      type: "start_generation";
      provider_config: ProviderConfigSnapshot;
      reviewer_enabled: boolean;
    }
  | { type: "retry_interrupted_run"; failed_node_id: string }
  | { type: "rollback"; checkpoint_id: string }
  | { type: "confirm" }
  | { type: "provider_select"; role: string; provider: WorkspaceProviderName }
  | { type: "permission_response"; id: string; approved: boolean; reason?: string | null }
  | {
      type: "choice_response";
      id: string;
      selected_option_ids: string[];
      free_text?: string | null;
      answers?: ChoiceAnswer[];
    }
  | { type: "request_revision"; feedback: StructuredFeedback }
  | {
      type: "work_item_plan_compile_recovery_action";
      action: WorkItemPlanCompileRecoveryAction;
      reason?: string | null;
    }
  | { type: "abandon_human_gate"; command_id: string }
  | {
      type: "human_gate_feedback";
      command_id: string;
      feedback: string;
      /** REQ-WSC-13：逐条「排除此项」的 finding key；缺省/空＝旧行为。 */
      excluded_finding_ids?: string[];
    }
  | { type: "advance"; command_id: string }
  | { type: "confirm_plan_amendment"; amendment_id: string }
  | {
      type: "cancel_plan_amendment";
      amendment_id: string;
      reason?: string | null;
    }
  | {
      type: "start_linked_workspace_amendment";
      target: LinkedWorkspaceAmendmentTarget;
    }
  | { type: "abort" }
  | {
      /** trust 指引卡「登记并重试」按钮：用户显式授权按需登记该 provider
       * 的用户级 workspace trust；root 由服务端解析（wire 不带路径字段）。 */
      type: "register_trust_entry";
      provider: WorkspaceProviderName;
    }
  | {
      type: "hello";
      session_id: string;
      last_seen_node_id?: string | null;
      role?: "driver" | "observer";
      after_event_seq?: number;
    }
  | { type: "ping" };

export type TimelineNodeType =
  | "prepare_context"
  | "context_note"
  | "start_generation"
  | "author_confirm"
  | "author_run"
  | "reviewer_run"
  | "review_decision"
  | "revision"
  | "human_confirm"
  | "work_item_plan_outline_run"
  | "work_item_plan_outline_confirm"
  | "work_item_plan_outline_review"
  | "work_item_plan_context_blocker"
  | "work_item_generation_mode"
  | "work_item_draft_run"
  | "work_item_draft_confirm"
  | "work_item_draft_review"
  | "work_item_batch_run"
  | "work_item_batch_confirm"
  | "work_item_batch_review"
  | "work_item_plan_compile"
  | "work_item_plan_compile_recovery"
  | "aborted_by_disconnect"
  | "protocol_error"
  | "completed"
  | (string & {});

export type TimelineNodeStatus = "active" | "paused" | "completed" | "failed" | "skipped" | "aborted";
export type ProviderStatus =
  | "starting"
  | "running"
  | "waiting_approval"
  | "completed"
  | "failed"
  | "aborted";
export type ExecutionEventKind = "provider" | "turn" | "command" | "output" | "artifact" | "usage";
export type ExecutionEventStatus =
  | "started"
  | "running"
  | "waiting_approval"
  | "completed"
  | "failed"
  | "aborted";

/** REQ-WSC-11：`needs_human` 结论面向 owner 的单条互斥选项——`label` 是
 * 选项名（人话），`consequence` 是「选它之后会发生什么」。 */
export type WorkspaceOwnerOption = {
  label: string;
  consequence: string;
};

export type WorkspaceReviewFinding = {
  severity: WorkspaceReviewFindingSeverity;
  message: string;
  evidence: string;
  required_action: string;
  /** SG7：服务端已证 L1 且同 source/context 显式配对的机器修复投影；
   * 旧记录/provider Raw 输出恒缺省（wire 不携带）。 */
  machine_repair?: MachineRepairPresentation | null;
  /** REQ-WSC-11：面向 owner 的一句话人话提问（仅 needs_human 携带）。 */
  owner_question?: string | null;
  /** REQ-WSC-11：owner 可选的互斥处置项（2–3 项）。 */
  owner_options?: WorkspaceOwnerOption[] | null;
  /** REQ-WSC-12：处置归属。`advisory` 只作提示（其它文档当前状态、产品侧
   * 规范/流程待办、与 Work Item 无关的外部事实）——平台只把 `actionable`
   * 计入阻断与返修；缺省语义＝`actionable`（兼容历史记录）。 */
  disposition?: WorkspaceReviewFindingDisposition;
  /** REQ-WSC-13：平台算的 finding 内容身份（sha256(severity␁message) 前
   * 16 位小写 hex）；owner「排除此项」与呈现共用的稳定 key。 */
  finding_key?: string;
  /** REQ-WSC-12：能指到具体 Work Item 时填写（如 `WI-001`）。 */
  target_work_item_id?: string;
};

/** REQ-WSC-12：finding 处置归属（wire 词表，与 Rust
 * `ReviewFindingDisposition` 逐字同源）。 */
export type WorkspaceReviewFindingDisposition = "actionable" | "advisory";

export type StructuredOutputDiagnostic = {
  code: string;
  message: string;
  repair_attempted: boolean;
  repair_succeeded: boolean;
  raw_output_preview?: string | null;
};

export type WsMessage = {
  id: string;
  role: string;
  content: string;
  checkpoint_id?: string | null;
  created_at: string;
};

export type WsCheckpoint = {
  id: string;
  message_index: number;
  stage: string;
  created_at: string;
};

export type WsProviderConfig = {
  author: WorkspaceProviderName;
  reviewer?: WorkspaceProviderName | null;
  permission_modes?: {
    author: ProviderPermissionMode;
    reviewer: ProviderPermissionMode;
  };
};

export type ProviderDefaults = {
  author: WorkspaceProviderName;
  reviewer?: WorkspaceProviderName | null;
  review_rounds: number;
};

export type TimelineNodeRetryError = {
  code: string;
  message: string;
};

export type TimelineNodeRetry = {
  retry_of_node_id: string;
  retry_attempt: number;
  retry_reason: string;
  retry_error: TimelineNodeRetryError;
};

export type TimelineNode = {
  node_id: string;
  node_type: TimelineNodeType;
  agent?: WorkspaceProviderName | null;
  stage: string;
  round?: number | null;
  status: TimelineNodeStatus;
  title: string;
  summary?: string | null;
  started_at: string;
  completed_at?: string | null;
  duration_ms?: number | null;
  artifact_ref?: string | null;
  provider_config_snapshot: ProviderConfigSnapshot;
  retry?: TimelineNodeRetry | null;
};

export type ReviewVerdict = {
  verdict: ReviewVerdictType;
  comments: string;
  summary: string;
  findings?: WorkspaceReviewFinding[];
  review_gate?: ReviewGate;
  structured_output_diagnostic?: StructuredOutputDiagnostic | null;
};

export type UsageReportPayload = {
  role?: string | null;
  input_tokens?: number | null;
  output_tokens?: number | null;
  cache_read_tokens?: number | null;
  cache_creation_tokens?: number | null;
};

export type ExecutionEvent = {
  event_id: string;
  node_id?: string | null;
  agent?: WorkspaceProviderName | string | null;
  kind: ExecutionEventKind;
  status: ExecutionEventStatus;
  title: string;
  detail?: string | null;
  command?: string | null;
  cwd?: string | null;
  output?: string | null;
  exit_code?: number | null;
};

export type ChoiceOption = {
  id: string;
  label: string;
  description?: string | null;
};

export type ChoiceQuestion = {
  id: string;
  prompt: string;
  options: ChoiceOption[];
  allow_multiple: boolean;
  allow_free_text: boolean;
};

export type ChoiceAnswer = {
  question_id: string;
  selected_option_ids: string[];
  free_text?: string | null;
};

/**
 * REQ-WSC-14（k3 REQ-WSC-14 不符合项）：门 turn 以 validation_reject 失败时的
 * 可操作事实（durable 投影）。`session_state` 全量投影携带最近一次，刷新后
 * 门卡仍能说清「发生了什么、下一步怎么做」；与 `human_gate_turn_rejected`
 * 帧字段同形。
 */
export type HumanGateTurnRejectionProjection = {
  turn_id: string;
  rejected_feedback: string;
  rejection_reason: string;
  rejection_next_actions: string[];
  rejection_diagnostics: string[];
  /** 拒收落盘时间（RFC3339）；旧投影缺省。 */
  recorded_at?: string;
};

/**
 * P0 1.3（REQ-WIGA-05）Task 10：无 driver 的人工门命令族（wire 与服务端
 * `HumanActionRequest` 同形，serde tag="type" snake_case）。`expected_gate_id`
 * 与服务端当前 active gate/timeline node 比对（不匹配 409 零副作用）；
 * `command_id` 为幂等/审计键（与 WS HumanGateFeedback/AbandonHumanGate 同族）。
 */
export type WorkspaceHumanAction =
  | { type: "approve"; command_id: string; expected_gate_id: string }
  | { type: "abandon"; command_id: string; expected_gate_id: string }
  | {
      type: "feedback";
      command_id: string;
      expected_gate_id: string;
      feedback: string;
      /** REQ-WSC-13：owner 逐条「排除此项」的 finding key（旧客户端缺省＝空）。 */
      excluded_finding_ids?: string[];
    }
  | {
      type: "compile_recovery";
      command_id: string;
      expected_gate_id: string;
      action: WorkItemPlanCompileRecoveryAction;
      reason?: string | null;
    }
  /** C1 Task 4（REQ-C1-GATE-02）：孤儿候选门恢复（recover=恢复原门/轮次，
   * rebuild=从权威 source/IR/report 重建呈现面）；无完整快照禁 approve。 */
  | {
      type: "candidate_recovery";
      command_id: string;
      expected_gate_id: string;
      action: "recover" | "rebuild";
    }
  /** REQ-WSC-13/14（k3 P0-2C）：撤销 owner 的「排除此项」——会话级 durable
   * 事实，与门身份无关（因此不带 `expected_gate_id`）；撤销后该 finding 恢复
   * 可被报为 actionable。 */
  | { type: "revoke_finding_exclusions"; command_id: string; finding_keys: string[] };

/** 撤销排除的回执：`remaining_finding_keys` 是撤销后仍生效的排除项。 */
export type RevokeFindingExclusionsResult = {
  command_id: string;
  state: WorkspaceHumanActionState;
  remaining_finding_keys: string[];
};

/** 人工命令回执状态（snake_case wire）：accepted=已受理（含幂等重放/AlreadyClosed）；busy=门/轮次占用（409）；rejected=引擎语义拒绝（422）。 */
export type WorkspaceHumanActionState = "accepted" | "busy" | "rejected";

export type WorkspaceHumanActionStatus = {
  command_id: string;
  state: WorkspaceHumanActionState;
  gate_id: string;
};

/**
 * P2 GAP-E/G（Task 0.1）：失败 SingleCandidate 评审运行的人工显式重驱回执
 *（与服务端 `RetryFailedScRunStatus` 同形）：accepted=首次受理；
 * replayed=同键重放读原结果；needs_human=派发副作用不明，人工分诊。
 */
export type RetryFailedScRunState = "accepted" | "replayed" | "needs_human";

export type RetryFailedScRunStatus = {
  command_id: string;
  failed_node_id: string;
  state: RetryFailedScRunState;
};

/** P0 1.3（REQ-WIGA-05）：choice 应答请求（与服务端 `ChoiceResponseRequest` 同形）。 */
export type ChoiceResponseRequest = {
  command_id: string;
  expected_run_id: string;
  answers: ChoiceAnswer[];
};

/**
 * P0 1.3：choice 回执状态（与服务端 `ChoiceReplyStatus` / cross_cutting
 * `ChoiceReplyState` 同形）：submitting/resolving 对应 HTTP 202（未承诺送达），
 * delivered 才对应 200；rejected/expired 对应 4xx（失效不得路由到新 run）。
 */
export type ChoiceReplyState =
  | "submitting"
  | "resolving"
  | "delivered"
  | "rejected"
  | "expired";

export type ChoiceReplyStatus = {
  command_id: string;
  expected_run_id: string;
  choice_id: string;
  state: ChoiceReplyState;
};

export type WorkspaceChoiceRequestSource =
  | "ask_user_question"
  | "request_user_input"
  | "text_fallback"
  | "provider_choice";

export type ArtifactVersion = {
  version: number;
  markdown: string;
  generated_by: WorkspaceProviderName;
  reviewed_by?: WorkspaceProviderName | null;
  review_verdict?: ReviewVerdictType | null;
  confirmed_by?: string | null;
  is_current?: boolean;
  created_at: string;
  source_node_id: string;
};

export type ArtifactVersionSummary = Omit<ArtifactVersion, "markdown"> & { markdown?: string };

export type WorkspaceStructuredArtifactVersion = ArtifactVersionSummary & {
  plan_projection?: PlanProjectionBundle;
  work_item_projection?: WorkItemProjectionBundle;
  work_item_revision_history?: WorkItemRevisionHistoryDto;
  projection_validation?: ProjectionValidationReport;
  plan_amendment_manifest?: PlanAmendmentManifest;
  sc_failure_repair_package?: ScFailureRepairPackage;
};

export type ProviderSnapshot = {
  name: string;
  model: string;
};

export type ArtifactRef = {
  artifact_id: string;
  version: number;
};

export type PermissionEvent = {
  request_id: string;
  request: unknown;
  response: unknown | null;
  ts: string;
};

export type NodeDetail = {
  node_id: string;
  session_id: string;
  node_type: TimelineNodeType;
  status: TimelineNodeStatus;
  agent_role: "author" | "reviewer" | null;
  provider: ProviderSnapshot | null;
  prompt?: string | null;
  messages: WsMessage[];
  streaming_content: string;
  execution_events: ExecutionEvent[];
  permission_events: PermissionEvent[];
  verdict: ReviewVerdict | null;
  artifact_ref: ArtifactRef | null;
  is_revision: boolean;
  revision_feedback?: string | null;
  base_artifact_ref: ArtifactRef | null;
  started_at: string;
  ended_at: string | null;
};

export type WorkspaceNodeDetailResponse = NodeDetail;

export type WorkspacePromptResponse = {
  node_id: string;
  prompt: string;
};

export type WorkspaceEventOutputResponse = {
  node_id: string;
  event_id: string;
  output: string;
};

export type WorkspaceArtifactVersionResponse = {
  version: number;
  markdown: string;
  artifact?: WorkspaceArtifactWire | WorkItemPlanArtifactPayload | null;
  generated_by?: WorkspaceProviderName;
  reviewed_by?: WorkspaceProviderName | null;
  review_verdict?: ReviewVerdictType | null;
  confirmed_by?: string | null;
  is_current?: boolean;
  created_at?: string;
  source_node_id?: string;
};

export type WithEventSeq<T> = T & { event_seq?: number };

export type WsOutMessage = WithEventSeq<
  | { type: "stream_chunk"; role: string; content: string; node_id?: string | null }
  | {
      type: "message_complete";
      message_id: string;
      checkpoint_id: string;
      node_id?: string | null;
    }
  | { type: "stage_change"; stage: string }
  | ArtifactUpdateMessage
  | { type: "provider_select_request"; stage: string; defaults: ProviderDefaults }
  | {
      type: "permission_request";
      id: string;
      tool_name: string;
      description: string;
      risk_level: "low" | "medium" | "high";
    }
  | {
      type: "choice_request";
      id: string;
      prompt: string;
      options: ChoiceOption[];
      allow_multiple: boolean;
      allow_free_text: boolean;
      questions?: ChoiceQuestion[];
      source: WorkspaceChoiceRequestSource;
    }
  | { type: "provider_status"; status: ProviderStatus }
  | { type: "execution_event"; event: ExecutionEvent }
  | { type: "timeline_node_created"; node: TimelineNode }
  | {
      type: "timeline_node_updated";
      node_id: string;
      status: TimelineNodeStatus;
      summary?: string | null;
      completed_at?: string | null;
    }
  | {
      type: "review_complete";
      node_id: string;
      round: number;
      verdict: ReviewVerdictType;
      comments: string;
      summary: string;
      findings?: WorkspaceReviewFinding[];
      review_gate?: ReviewGate;
      structured_output_diagnostic?: StructuredOutputDiagnostic | null;
    }
  | { type: "review_decision_required"; node_id: string; round: number; options: string[] }
  | { type: "human_gate_closed"; decision: "confirm" | "terminate"; stage: string }
  | {
      type: "human_gate_turn_open";
      turn_id: string;
      command_id: string;
      remaining_budget: number;
    }
  | { type: "human_gate_turn_completed"; turn_id: string; artifact_ref: string }
  | {
      type: "human_gate_turn_failed";
      turn_id: string;
      failure_class: string;
      message: string;
    }
  /** REQ-WSC-14：门 turn 以 validation_reject 失败时的可操作回执（已 durable
   * 落盘）——被拒反馈原文、可读原因、下一步动作与逐字诊断。 */
  | {
      type: "human_gate_turn_rejected";
      turn_id: string;
      rejected_feedback: string;
      rejection_reason: string;
      rejection_next_actions: string[];
      rejection_diagnostics: string[];
    }
  /** REQ-WSC-13（k3 P0-2A）：排除-only 帧（`feedback` 为空 + 平台 key）的
   * 受理回执——引擎已落 durable 已处置清单，不建修订 turn。 */
  | {
      type: "human_gate_exclusions_accepted";
      command_id: string;
      excluded_finding_ids: string[];
    }
  | { type: "human_gate_busy"; turn_id: string }
  | {
      type: "advance_completed";
      command_id: string;
      attempt_id: string;
      workspace_entry: string;
    }
  | { type: "advance_rejected"; command_id: string; code: string; reason: string }
  // 退役留档（T5/REQ-RET-02）：human_presentation_revision_saved/save_failed 出站
  // 变体随保存命令族删除（wp5-attribution-table.md §2）。
  | {
      type: "linked_workspace_amendment_created";
      snapshot: LinkedWorkspaceSessionSnapshot;
    }
  | {
      type: "session_state";
      session_id: string;
      connection_id?: string;
      workspace_type: string;
      stage: string;
      superpowers_enabled: boolean;
      openspec_enabled: boolean;
      messages: WsMessage[];
      checkpoints: WsCheckpoint[];
      artifact: string | null | WorkspaceArtifactWire;
      providers: WsProviderConfig;
      timeline_nodes: TimelineNode[];
      active_node_id: string | null;
      artifact_versions: WorkspaceStructuredArtifactVersion[];
      artifact_version_summaries?: ArtifactVersionSummary[];
      timeline_node_details: Record<string, NodeDetail>;
      active_run_id: string | null;
      human_presentation_revisions: HumanPresentationRevision[];
      reviewer_enabled_at_start?: boolean | null;
      recoverable_interrupted_run?: RecoverableInterruptedRun | null;
      plan_repair?: PlanRepairSessionSnapshot | null;
      session_status: WorkspaceSessionStatus;
      flow_kind: WorkItemPlanFlowKind;
      run_policy: WorkItemPlanRunPolicy;
      run_history: WorkItemPlanRunHistory;
      review_invocation_scope?: unknown | null;
      human_gate_snapshot?: WorkItemPlanHumanGateSnapshot | null;
      /** REQ-WSC-14：该会话最近一次门 turn 拒收（四要素，与
       * `human_gate_turn_rejected` 帧同源）——刷新后重建门卡用；缺席省略。 */
      human_gate_turn_rejection?: HumanGateTurnRejectionProjection | null;
      /** REQ-WSC-18：当前候选 Final Compile 失败事实（缺席=无失败/旧候选）。 */
      candidate_compile_failure?: WorkItemPlanCandidateCompileFailure | null;
      repair_reservation?: WorkItemPlanRepairReservation | null;
      policy_diagnostics?: WorkItemPlanPolicyDiagnostic[];
      provider_start_ledger?: WorkItemPlanProviderStartLedgerEntry[];
      /** P0 1.2：durable automation 归属；服务端读取失败/未知时省略。 */
      automation?: AutomationOwnership | null;
      /** REQ-HGC-04：本页底部「继续自动化」入口的服务端权威事实（与 HTTP control
       * 的 `allowed_actions` 含 `continue_plan` 同源）；缺席/null=服务端未判定。 */
      plan_continue_available?: boolean | null;
      /** C1（Task 6.1）：durable LC 恢复纯投影（waiting/fresh_reserved/
       * fresh_running/resolved 主状态＋last_operation）；空/旧 wire 缺省
       * 省略；resolved 保留证据供 Replay，allowed_actions 由后端权威下发。 */
      lc_resume_recovery?: LcResumeRecoveryDto | null;
      /** SG7/SG7a（§0.6.1）：结构治理只读快照（durable 聚合状态＋同
       * record gate/ledger/三 refs 的纯投影）；旧记录无聚合状态时省略，
       * 不构成任何启动/授权。 */
      structural_governance?: StructuralGovernanceSnapshot | null;
    }
  | {
      /** C1（Task 6.1）：显式 fresh operation 的 typed 受理/回放结果（设计
       * §5.1）——operation_state 取 durable 事实，不从客户端乐观态猜；
       * 无 native/秘密字段；随 SessionState.last_operation 对账。 */
      type: "lc_resume_recovery_operation_result";
      recovery_id: string;
      operation_id: string;
      delivery: LcResumeRecoveryDelivery;
      operation_state: LcFreshOperationState;
      fresh_node_id?: string | null;
      reason_code?: string | null;
    }
  | { type: "error"; message: string }
  | { type: "protocol_error"; code: string; message: string; context?: unknown }
  | { type: "provider_locked"; snapshot: ProviderConfigSnapshot; locked_at: string }
  | {
      /** 指引卡结构化指导（owner 2026-10-12 直批简化）：紧随 error 帧到达。
       * providers 为 wire 标签（codex/kimi_code，卡面按实际单复数）；
       * command_markdown 仅 entry 缺失携带（同源 adapter manual_command），
       * 冲突等分型省略。 */
      type: "trust_missing_guidance";
      providers: string[];
      canonical_root: string;
      reason_code: string;
      retry_action: string;
      command_markdown?: string | null;
    }
  | { type: "resync_required"; event_seq: number }
  | {
      /** 「登记并重试」typed 回执：action 与 ensure 审计 outcome 同词表
       * （registered/replay/reuse_user_entry）；仅回执，不自动重新生成。 */
      type: "trust_entry_registered";
      provider: WorkspaceProviderName;
      action: "registered" | "replay" | "reuse_user_entry";
      trust_key: string;
      canonical_root: string;
      registered_at: string;
    }
  | { type: "pong" }
>;
