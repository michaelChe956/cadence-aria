import { ApiRequestError, normalizeApiError } from "./client";
import type { AggregateIndexActiveResponse } from "./types";

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

function aggregateIndexPath(projectId: string): string {
  return `/api/projects/${encodeURIComponent(projectId)}/logical-codebase/aggregate-indexes`;
}

export function getActiveAggregateIndex(
  projectId: string,
): Promise<AggregateIndexActiveResponse> {
  return requestJson<AggregateIndexActiveResponse>(
    `${aggregateIndexPath(projectId)}/active`,
  );
}

export function rebuildAggregateIndex(
  projectId: string,
): Promise<AggregateIndexActiveResponse> {
  return requestJson<AggregateIndexActiveResponse>(
    `${aggregateIndexPath(projectId)}/rebuild`,
    {
      method: "POST",
      body: JSON.stringify({}),
    },
  );
}
