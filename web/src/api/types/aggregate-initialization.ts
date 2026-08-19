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
  | "failed";

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
