// v1.3 §4/R2/R7：统一 codebases 列表 + 逻辑代码库创建（登记向导入口链路）。
import { ApiRequestError, normalizeApiError } from "./client";
import type {
  CodebaseListResponse,
  CreateLogicalCodebaseRequest,
  LogicalCodebaseDto,
  LogicalCodebaseListResponse,
} from "./types";

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

export function listCodebases(projectId: string): Promise<CodebaseListResponse> {
  return requestJson<CodebaseListResponse>(
    `/api/projects/${encodeURIComponent(projectId)}/codebases`,
  );
}

/**
 * 2026-10-29：LC 列表。`includeDeleted` 时后端追加已软删（墓碑）LC
 * （`deleted=true` + `deleted_at`）；默认只返回存活 LC。
 */
export function listLogicalCodebases(
  projectId: string,
  options?: { includeDeleted?: boolean },
): Promise<LogicalCodebaseListResponse> {
  const query = options?.includeDeleted ? "?include_deleted=true" : "";
  return requestJson<LogicalCodebaseListResponse>(
    `/api/projects/${encodeURIComponent(projectId)}/logical-codebases${query}`,
  );
}

/** 恢复软删 LC（摘墓碑，id 不变）。 */
export function restoreLogicalCodebase(
  projectId: string,
  logicalCodebaseId: string,
): Promise<{ status: string; codebase: LogicalCodebaseDto }> {
  return requestJson<{ status: string; codebase: LogicalCodebaseDto }>(
    `/api/projects/${encodeURIComponent(
      projectId,
    )}/logical-codebases/${encodeURIComponent(logicalCodebaseId)}/restore`,
    { method: "POST" },
  );
}

/**
 * 彻底删除（硬删）已软删 LC：不可恢复，清政策产物与成员指针
 * （不动 AGENTS.md/CLAUDE.md 等 Cadence-skills 产物）。
 */
export function purgeLogicalCodebase(
  projectId: string,
  logicalCodebaseId: string,
): Promise<{ status: string }> {
  return requestJson<{ status: string }>(
    `/api/projects/${encodeURIComponent(
      projectId,
    )}/logical-codebases/${encodeURIComponent(logicalCodebaseId)}/purge`,
    { method: "DELETE" },
  );
}

export function deleteLogicalCodebase(
  projectId: string,
  logicalCodebaseId: string,
): Promise<{ status: string }> {
  return requestJson<{ status: string }>(
    `/api/projects/${encodeURIComponent(
      projectId,
    )}/logical-codebases/${encodeURIComponent(logicalCodebaseId)}`,
    { method: "DELETE" },
  );
}

export function createLogicalCodebase(
  projectId: string,
  payload: CreateLogicalCodebaseRequest,
): Promise<LogicalCodebaseDto> {
  return requestJson<LogicalCodebaseDto>(
    `/api/projects/${encodeURIComponent(projectId)}/logical-codebases`,
    {
      method: "POST",
      body: JSON.stringify(payload),
    },
  );
}
