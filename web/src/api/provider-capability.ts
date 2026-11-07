// Provider capability 能力面（2026-11-05 机器级化，REQ-PCR-08）：provider 是
// 平台公用能力，与项目、逻辑代码库无关 ⇒ 四条端点全部按机器级路由定位，
// 不再携带 project/logical-codebase 标识。
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

/** 只读 durable capability 投影（零写入、零探针触发）。机器一份事实。 */
export function getProviderCapabilities(): Promise<ProviderCapabilitiesResponse> {
  return requestJson<ProviderCapabilitiesResponse>(
    "/api/providers/capabilities",
  );
}

/**
 * 用户显式 capability 重验证（真实探针，同步等待——首次分钟级；全行
 * 已验证同版本时秒回 already_confirmed）。失败抛 ApiRequestError：
 * `provider_capability_probe_unsupported`（422）/ `provider_capability_probe_failed`
 * （503，details.detail 携带 action 与原始错误）。
 */
export function postProviderCapabilityRevalidate(
  providerType: ProviderCapabilityType,
): Promise<CapabilityRevalidateResult> {
  return requestJson<CapabilityRevalidateResult>(
    "/api/providers/capabilities/revalidate",
    {
      method: "POST",
      body: JSON.stringify({ provider_type: providerType }),
    },
  );
}
