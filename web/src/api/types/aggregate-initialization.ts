// 后端 AggregateInitializationOperation / AggregateInitializationStep /
// AggregateMemberProjection / AggregateCancellation 的 serde snake_case 投影。
// 对应 src/web/types/aggregate_initialization.rs 与
// src/web/handlers/aggregate_initialization 的响应 DTO。
import type { ApiError } from "./common";

export type AggregateInitializationOperationStatus =
  | "created"
  | "running"
  | "completed"
  | "failed"
  | "cancelled";

export type AggregateInitializationStepStatus =
  | "pending"
  | "running"
  | "completed"
  | "failed"
  // 聚合根已完成 Cadence-skills 初始化时，三个 provider-turn 步骤被判定跳过
  // （后端 `AggregateInitializationStepStatus::Skipped` → DTO "skipped"）。
  | "skipped";

export type AggregateInitializationStepDto = {
  step_id: string;
  status: AggregateInitializationStepStatus;
};

export type AggregateMemberProjectionDto = {
  logical_repository_id: string;
  checkout_id: string;
  revision: string;
  dirty: boolean;
  profile_digest?: string | null;
};

export type AggregateCancellationDto = {
  reason_code: string;
  cancelled_at: string;
  detail?: string | null;
};
/// 冻结的 provider 选择候选行（对应后端 `AggregateProviderCandidate`）。
export type AggregateProviderCandidateDto = {
  provider: string;
  display_name: string;
  cli_available: boolean;
  cli_reason_code?: string | null;
  capability_state: string;
  skipped_reason?: string | null;
  action_hint: string;
  warning?: string | null;
};

/// 创建期冻结的 provider 选择结果（REQ-BOOT-07：selected 恒等于 input.provider）。
export type AggregateProviderSelectionDto = {
  selected: string;
  candidates: AggregateProviderCandidateDto[];
  rule_version: string;
  decided_at: string;
};


export type AggregateInitializationOperationSnapshot = {
  operation_id: string;
  project_id: string;
  status: AggregateInitializationOperationStatus;
  profile: string | null;
  steps: AggregateInitializationStepDto[];
  current_step: string | null;
  failed_step: string | null;
  member_projections: AggregateMemberProjectionDto[];
  cancellation: AggregateCancellationDto | null;
  error: ApiError | null;
  provider_selection?: AggregateProviderSelectionDto | null;
  created_at: string;
  updated_at: string;
  completed_at: string | null;
};

export type CreateAggregateInitializationRequest = {
  idempotency_key: string;
};

export type CancelAggregateInitializationRequest = {
  reason: string;
  detail?: string | null;
};
