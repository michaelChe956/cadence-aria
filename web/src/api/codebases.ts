// v1.3 §4/R2/R7：统一 codebases 列表 + 逻辑代码库创建（登记向导入口链路）。
import { ApiRequestError, normalizeApiError } from "./client";
import type {
  CodebaseListResponse,
  CreateLogicalCodebaseRequest,
  LogicalCodebaseDto,
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
