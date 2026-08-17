import { ApiRequestError, normalizeApiError } from "./client";
import type { LogicalCodebaseMembersResponse } from "./types";

async function requestJson<T>(path: string): Promise<T> {
  const response = await fetch(path, {
    headers: { "content-type": "application/json" },
  });
  if (!response.ok) {
    throw new ApiRequestError(await normalizeApiError(response));
  }
  return (await response.json()) as T;
}

function membersPath(projectId: string): string {
  return `/api/projects/${encodeURIComponent(projectId)}/logical-codebase/members`;
}

export function listLogicalCodebaseMembers(
  projectId: string,
): Promise<LogicalCodebaseMembersResponse> {
  return requestJson<LogicalCodebaseMembersResponse>(membersPath(projectId));
}
