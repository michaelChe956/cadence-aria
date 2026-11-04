// 票3「一键核验全部」：批量核验作业的发起与轮询 API（独立于块2 的
// provider-capability.ts / provider-install.ts，互不改写）。
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

function batchPath(projectId: string, logicalCodebaseId: string): string {
  return `/api/projects/${encodeURIComponent(
    projectId,
  )}/logical-codebases/${encodeURIComponent(
    logicalCodebaseId,
  )}/capabilities/revalidate-all`;
}

/**
 * 显式发起「一键核验全部」：四家并发、异步作业，立即返回 `job_id`
 * （202；MUST NOT 在响应内等待探针）。
 */
export function startCapabilityRevalidateBatch(
  projectId: string,
  logicalCodebaseId: string,
): Promise<CapabilityRevalidateBatchStartResult> {
  return requestJson<CapabilityRevalidateBatchStartResult>(
    batchPath(projectId, logicalCodebaseId),
    { method: "POST" },
  );
}

/**
 * 轮询批量核验作业：durable（刷新后按 `job_id` 继续），终态仍可读。
 * 未知 job_id 抛 `capability_revalidate_batch_job_not_found`（404）。
 */
export function getCapabilityRevalidateBatchJob(
  projectId: string,
  logicalCodebaseId: string,
  jobId: string,
): Promise<CapabilityRevalidateBatchJobDto> {
  return requestJson<CapabilityRevalidateBatchJobDto>(
    `${batchPath(projectId, logicalCodebaseId)}/${encodeURIComponent(jobId)}`,
  );
}
