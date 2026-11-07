// 票3「一键核验全部」：批量核验作业的发起与轮询 API（独立于块2 的
// provider-capability.ts / provider-install.ts，互不改写）。
// 2026-11-05 机器级化（REQ-PCR-03）：批量作业不绑定项目/逻辑代码库 ⇒
// 端点不再携带 project/logical-codebase 标识。
import { ApiRequestError, normalizeApiError } from "./client";
import type {
  CapabilityRevalidateBatchJobDto,
  CapabilityRevalidateBatchStartResult,
} from "./types/capability-revalidate-batch";

async function requestJson<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(path, {
    ...init,
    headers: {
      "content-type": "application/json",
      ...(init?.headers ?? {}),
    },
  });
  if (!response.ok) {
    throw new ApiRequestError(await normalizeApiError(response));
  }
  const text = await response.text();
  if (!text.trim()) {
    return undefined as T;
  }
  return JSON.parse(text) as T;
}

/**
 * 显式发起「一键核验全部」：四家并发、异步作业，立即返回 `job_id`
 * （202；MUST NOT 在响应内等待探针）。
 */
export function startCapabilityRevalidateBatch(): Promise<CapabilityRevalidateBatchStartResult> {
  return requestJson<CapabilityRevalidateBatchStartResult>(
    "/api/providers/capabilities/revalidate-all",
    { method: "POST" },
  );
}

/**
 * 轮询批量核验作业：durable（刷新后按 `job_id` 继续），终态仍可读。
 * 未知 job_id 抛 `capability_revalidate_batch_job_not_found`（404）。
 */
export function getCapabilityRevalidateBatchJob(
  jobId: string,
): Promise<CapabilityRevalidateBatchJobDto> {
  return requestJson<CapabilityRevalidateBatchJobDto>(
    `/api/providers/capabilities/revalidate-all/${encodeURIComponent(jobId)}`,
  );
}
