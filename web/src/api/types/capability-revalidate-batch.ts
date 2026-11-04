// 票3「一键核验全部」：批量 capability 核验作业的 serde snake_case DTO。
// 对应 src/web/handlers/logical_codebase_capability_batch.rs 的 POST/GET 面
// （后端 src/product/logical_codebase/capability_revalidate_batch.rs）。
import type { ProviderCapabilityType } from "./provider-capability";

/** 作业总体状态：`running` → `completed`（终态仍可读）。 */
export type CapabilityRevalidateBatchJobStatus = "running" | "completed";

/** 逐项状态：`running` → `confirmed`/`failed`/`skipped`，终态未完成 → `timeout`。 */
export type CapabilityRevalidateBatchItemStatus =
  | "running"
  | "confirmed"
  | "failed"
  | "skipped"
  | "timeout";

/** 逐家核验结果（逐项进度载体）。 */
export type CapabilityRevalidateBatchItemDto = {
  provider_type: ProviderCapabilityType;
  status: CapabilityRevalidateBatchItemStatus;
  /** 稳定原因码：already_confirmed / revalidated / cli_not_installed /
   * probe_unsupported / probe_failed / store_error / job_deadline_exceeded。 */
  reason: string | null;
  detail: string | null;
  version: string | null;
  started_at: string | null;
  finished_at: string | null;
};

export type CapabilityRevalidateBatchJobDto = {
  job_id: string;
  status: CapabilityRevalidateBatchJobStatus;
  created_at: string;
  updated_at: string;
  finished_at: string | null;
  results: CapabilityRevalidateBatchItemDto[];
};

/** POST 发起结果：立即返回 job_id（异步作业，不等探针）。 */
export type CapabilityRevalidateBatchStartResult = {
  job_id: string;
  status: CapabilityRevalidateBatchJobStatus;
};
