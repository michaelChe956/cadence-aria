import { ApiRequestError, normalizeApiError } from "./client";
import type {
  CapabilityRevalidateResult,
  ProviderCapabilitiesResponse,
  ProviderCapabilityType,
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

function logicalCodebasePath(projectId: string, logicalCodebaseId: string): string {
  return `/api/projects/${encodeURIComponent(
    projectId,
  )}/logical-codebases/${encodeURIComponent(logicalCodebaseId)}`;
}

/** 只读 durable capability 投影（零写入、零探针触发）。 */
export function getLogicalCodebaseCapabilities(
  projectId: string,
  logicalCodebaseId: string,
): Promise<ProviderCapabilitiesResponse> {
  return requestJson<ProviderCapabilitiesResponse>(
    `${logicalCodebasePath(projectId, logicalCodebaseId)}/capabilities`,
  );
}

/**
 * 用户显式 capability 重验证（真实探针，同步等待——首次分钟级；全行
 * 已验证同版本时秒回 already_confirmed）。失败抛 ApiRequestError：
 * `provider_capability_probe_unsupported`（422）/ `provider_capability_probe_failed`
 * （503，details.detail 携带 action 与原始错误）。
 */
export function postLogicalCodebaseCapabilityRevalidate(
  projectId: string,
  logicalCodebaseId: string,
  providerType: ProviderCapabilityType,
): Promise<CapabilityRevalidateResult> {
  return requestJson<CapabilityRevalidateResult>(
    `${logicalCodebasePath(projectId, logicalCodebaseId)}/capability-revalidate`,
    {
      method: "POST",
      body: JSON.stringify({ provider_type: providerType }),
    },
  );
}
