import { ApiRequestError, normalizeApiError } from "./client";
import type {
  AggregateInitializationOperationSnapshot,
  CancelAggregateInitializationRequest,
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

function initializationsPath(projectId: string): string {
  return `/api/projects/${encodeURIComponent(projectId)}/logical-codebase/initializations`;
}

export function startAggregateInitialization(
  projectId: string,
  idempotencyKey: string,
): Promise<AggregateInitializationOperationSnapshot> {
  return requestJson<AggregateInitializationOperationSnapshot>(
    initializationsPath(projectId),
    {
      method: "POST",
      body: JSON.stringify({ idempotency_key: idempotencyKey }),
    },
  );
}

export function getAggregateInitialization(
  projectId: string,
  operationId: string,
): Promise<AggregateInitializationOperationSnapshot> {
  return requestJson<AggregateInitializationOperationSnapshot>(
    `${initializationsPath(projectId)}/${encodeURIComponent(operationId)}`,
  );
}

export function cancelAggregateInitialization(
  projectId: string,
  operationId: string,
  request: CancelAggregateInitializationRequest,
): Promise<AggregateInitializationOperationSnapshot> {
  return requestJson<AggregateInitializationOperationSnapshot>(
    `${initializationsPath(projectId)}/${encodeURIComponent(operationId)}/cancel`,
    {
      method: "POST",
      body: JSON.stringify(request),
    },
  );
}
