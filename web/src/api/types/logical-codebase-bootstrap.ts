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

export type BootstrapFailureDto = {
  reason_code: string;
  detail: string;
  retryable: boolean;
  external_side_effect: string;
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

export type LogicalCodebaseBootstrapProjection = {
  project_id: string;
  logical_codebase_id: string;
  authority_root: string;
  membership_revision: number | null;
  policy: BootstrapPolicyReferenceDto | null;
  steps: BootstrapStepProjectionDto[];
  planning_ready: boolean;
  notices: LogicalCodebaseBootstrapNoticeDto[];
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
  /** 推进后的最新投影（与 GET .../bootstrap 同构）。 */
  projection: LogicalCodebaseBootstrapProjection;
};

export type { ApiError };
