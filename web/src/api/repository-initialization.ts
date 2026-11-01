import { ApiRequestError, normalizeApiError } from "./client";
import type {
  C1WaitingItem,
  RepositoryInitializationOperationSnapshot,
} from "./types";

// C5 Task 6/7（REQ-INIT-C5-RESUME）：project 级 repository 初始化失败
// 等待项与“网关恢复后继续” resume REST。沿 logical-codebase-bootstrap.ts
// 先例：错误原样抛 ApiRequestError（code/details 保留），前端不判定成功。

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

export function listProjectRepositoryInitializationWaitingItems(
  projectId: string,
): Promise<C1WaitingItem[]> {
  return requestJson<C1WaitingItem[]>(
    `/api/projects/${encodeURIComponent(projectId)}/repository-initializations/waiting-items`,
  );
}

export function postRepositoryInitializationResume(
  projectId: string,
  operationId: string,
  commandId: string,
): Promise<RepositoryInitializationOperationSnapshot> {
  return requestJson<RepositoryInitializationOperationSnapshot>(
    `/api/projects/${encodeURIComponent(projectId)}/repository-initializations/${encodeURIComponent(operationId)}/resume`,
    {
      method: "POST",
      body: JSON.stringify({ command_id: commandId }),
    },
  );
}
