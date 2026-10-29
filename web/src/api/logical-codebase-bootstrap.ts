import { ApiRequestError, normalizeApiError } from "./client";
import type {
  BootstrapActionRequest,
  BootstrapActionResult,
  BootstrapAdvanceResult,
  LogicalCodebaseBootstrapProjection,
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

function bootstrapPath(projectId: string, logicalCodebaseId: string): string {
  return `/api/projects/${encodeURIComponent(
    projectId,
  )}/logical-codebases/${encodeURIComponent(logicalCodebaseId)}/bootstrap`;
}

export function getLogicalCodebaseBootstrap(
  projectId: string,
  logicalCodebaseId: string,
): Promise<LogicalCodebaseBootstrapProjection> {
  return requestJson<LogicalCodebaseBootstrapProjection>(
    bootstrapPath(projectId, logicalCodebaseId),
  );
}

export function postLogicalCodebaseBootstrapAction(
  projectId: string,
  logicalCodebaseId: string,
  request: BootstrapActionRequest,
): Promise<BootstrapActionResult> {
  return requestJson<BootstrapActionResult>(
    `${bootstrapPath(projectId, logicalCodebaseId)}/actions`,
    {
      method: "POST",
      body: JSON.stringify({
        command_id: request.command_id,
        step: request.step,
        action: request.action,
        expected_revision: request.expected_revision ?? null,
        expected_object_id: request.expected_object_id,
      }),
    },
  );
}

/**
 * 冷启动一键推进：服务端按 identity→manifest_checkout→rules_policy→
 * member_index→aggregate_index_active 顺序推进到可动步骤耗尽或 PlanningReady。
 * 幂等：重复调用不重复推进、不重复 provider turn（advanced_steps 为空）。
 */
export function postLogicalCodebaseBootstrapAdvance(
  projectId: string,
  logicalCodebaseId: string,
): Promise<BootstrapAdvanceResult> {
  return requestJson<BootstrapAdvanceResult>(
    `${bootstrapPath(projectId, logicalCodebaseId)}/advance`,
    {
      method: "POST",
      body: JSON.stringify({}),
    },
  );
}
