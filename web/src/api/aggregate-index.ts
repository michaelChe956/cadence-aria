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

function aggregateIndexPath(
  projectId: string,
  logicalCodebaseId: string,
): string {
  return `/api/projects/${encodeURIComponent(
    projectId,
  )}/logical-codebases/${encodeURIComponent(
    logicalCodebaseId,
  )}/aggregate-indexes`;
}

export function getActiveAggregateIndex(
  projectId: string,
  logicalCodebaseId: string,
): Promise<AggregateIndexActiveResponse> {
  return requestJson<AggregateIndexActiveResponse>(
    `${aggregateIndexPath(projectId, logicalCodebaseId)}/active`,
  );
}

export function rebuildAggregateIndex(
  projectId: string,
  logicalCodebaseId: string,
): Promise<AggregateIndexActiveResponse> {
  return requestJson<AggregateIndexActiveResponse>(
    `${aggregateIndexPath(projectId, logicalCodebaseId)}/rebuild`,
    {
      method: "POST",
      body: JSON.stringify({}),
    },
  );
}
