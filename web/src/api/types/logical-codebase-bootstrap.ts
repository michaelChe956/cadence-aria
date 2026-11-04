// 后端 LogicalCodebaseBootstrapProjection / BootstrapActionResult 的 serde
// snake_case 投影。对应 src/web/types/logical_codebase_bootstrap.rs 与
// src/web/handlers/logical_codebase_bootstrap 的响应 DTO。
import type { ApiError } from "./common";

export type LogicalCodebaseBootstrapStepName =
  | "identity"
  | "manifest_checkout"
  | "rules_policy"
  | "member_index"
  | "aggregate_index_active";

export type LogicalCodebaseBootstrapStepStatus =
  | "not_started"
  | "running"
  | "completed"
  | "failed"
  | "waiting_for_human";

export type BootstrapActionName =
  | "prepare"
  | "continue"
  | "retry"
  | "revalidate"
  | "repair"
  | "republish";

export type BootstrapActionOutcome =
  | "accepted"
  | "replayed"
  | "waiting_for_human"
  | "completed";

export type BootstrapCheckpointDto = {
  object_id: string;
  input_digest: string | null;
  output_artifact_ref: string | null;
  expected_membership_revision: number | null;
  completed_at: string | null;
};

/**
 * 失败面三段成品（REQ-BOOT-08 / REQ-LAYOUT-02）——与 Rust `FailureCopyDto`
 * 逐字段同构。五个字段**全部必填**：`reason_zh` 不是可选，前端不得把缺失
 * 当降级路径（只有非法/损坏响应才可能缺，那种情况按防崩处理）。
 */
export type FailureCopyTriage = "environment" | "product_defect" | "retry_only";

export type FailureCopyDto = {
  title_zh: string;
  reason_zh: string;
  action_zh: string;
  triage: FailureCopyTriage;
  machine_line: string;
};

export type BootstrapFailureDto = {
  reason_code: string;
  detail: string;
  retryable: boolean;
  external_side_effect: string;
  /** 只由后端 copy.triage 派生（兼容旧响应才缺省）。 */
  triage?: FailureCopyTriage | null;
  /** 后端已渲染好的同源成品，前端不再自拼中文。 */
  copy?: FailureCopyDto | null;
};

export type BootstrapStepProjectionDto = {
  step: LogicalCodebaseBootstrapStepName;
  status: LogicalCodebaseBootstrapStepStatus;
  object_id: string;
  checkpoint: BootstrapCheckpointDto | null;
  failure: BootstrapFailureDto | null;
  allowed_actions: BootstrapActionName[];
};

export type LogicalCodebaseBootstrapNoticeDto = {
  key: string;
  step: LogicalCodebaseBootstrapStepName;
  object_id: string;
  reason_code: string;
  summary: string;
  external_side_effect: string;
  allowed_actions: BootstrapActionName[];
  next_step: LogicalCodebaseBootstrapStepName | null;
  created_at: string;
};

export type BootstrapPolicyReferenceDto = {
  policy_id: string;
  policy_revision: number;
  policy_digest: string;
  artifact_root: string;
};

/**
 * 冷启动推进的停止原因（advance / 手动「准备」被拒时落盘，GET 投影透出）。
 * 仅当被停步骤仍是 `not_started` 时出现——PlanningReady 或链启动后清除。
 * `reason_code` 与失败面同码表，两道闸门的区分键：
 * `aggregate_initialization_provider_needs_verification`（provider 选择门：
 * 装了但还没核验）与 `aggregate_initialization_trust_waiting`（trust 门：
 * 还没拿到这台机器的信任）。
 */
export type BootstrapAdvanceStopDto = {
  /** 停住的步骤（依赖顺序里的那一步）。 */
  step: LogicalCodebaseBootstrapStepName;
  /** 停止枚举（waiting_for_human / failed / no_actionable_step …），不是中文文案。 */
  reason: string;
  /** 稳定机器码——两道闸门的区分键。 */
  reason_code: string;
  /** 已脱敏的原始拒绝上下文（拒绝 message + details），可空。 */
  detail?: string | null;
  /** 停止时间。 */
  stopped_at: string;
};

export type LogicalCodebaseBootstrapProjection = {
  project_id: string;
  logical_codebase_id: string;
  authority_root: string;
  membership_revision: number | null;
  policy: BootstrapPolicyReferenceDto | null;
  steps: BootstrapStepProjectionDto[];
  planning_ready: boolean;
  notices: LogicalCodebaseBootstrapNoticeDto[];
  /** 最近一次可解释的停止（被停步骤仍「未开始」时才有；就绪后清除）。 */
  advance_stop?: BootstrapAdvanceStopDto | null;
};

export type BootstrapActionResult = {
  command_id: string;
  outcome: BootstrapActionOutcome;
  projection: LogicalCodebaseBootstrapProjection;
};

export type BootstrapActionRequest = {
  command_id: string;
  step: LogicalCodebaseBootstrapStepName;
  action: BootstrapActionName;
  expected_revision?: number | null;
  expected_object_id: string;
};

// 冷启动一键推进（POST .../bootstrap/advance）的停止原因：服务端按依赖顺序
// 推进到 planning_ready / 进行中（长任务在跑）/ 失败 / 等待人工 / 无可动步骤
// / 达到循环上限为止。
export type BootstrapAdvanceStopReason =
  | "planning_ready"
  | "in_progress"
  | "failed"
  | "waiting_for_human"
  | "no_actionable_step"
  | "iteration_limit";

export type BootstrapAdvanceResult = {
  project_id: string;
  logical_codebase_id: string;
  /** 本次真正推进的步骤（依赖顺序，可能为空——重复调用即幂等空转）。 */
  advanced_steps: LogicalCodebaseBootstrapStepName[];
  stopped_reason: BootstrapAdvanceStopReason | string;
  /** 停住的步骤（失败/等待人工时非空）。 */
  stopped_step?: LogicalCodebaseBootstrapStepName | null;
  /** 停住步骤的稳定原因码（既有投影 failure 或推进器稳定码）。 */
  stopped_reason_code?: string | null;
  /** 停住步骤的原始拒绝上下文（已脱敏的拒绝 message + details），可空。 */
  stopped_detail?: string | null;
  /** 推进后的最新投影（与 GET .../bootstrap 同构）。 */
  projection: LogicalCodebaseBootstrapProjection;
};

export type { ApiError };
