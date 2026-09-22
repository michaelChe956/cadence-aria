import type { WorkspaceProviderName } from "../api/types";
import { workspaceProviderName } from "./provider-options";

export const WORKSPACE_PROVIDER_DEFAULTS_STORAGE_KEY = "aria.workspace.provider-defaults";

export interface WorkspaceProviderDefaults {
  author: string;
  reviewer: string;
  reviewerEnabled: boolean;
}

function isWorkspaceProviderDefaults(value: unknown): value is WorkspaceProviderDefaults {
  if (typeof value !== "object" || value === null) {
    return false;
  }

  const defaults = value as Record<string, unknown>;
  return (
    typeof defaults.author === "string" &&
    typeof defaults.reviewer === "string" &&
    typeof defaults.reviewerEnabled === "boolean"
  );
}

export function readWorkspaceProviderDefaults(): WorkspaceProviderDefaults | null {
  try {
    const stored = window.localStorage.getItem(WORKSPACE_PROVIDER_DEFAULTS_STORAGE_KEY);
    if (stored === null) {
      return null;
    }

    const defaults: unknown = JSON.parse(stored);
    return isWorkspaceProviderDefaults(defaults) ? defaults : null;
  } catch {
    return null;
  }
}

export function writeWorkspaceProviderDefaults(defaults: WorkspaceProviderDefaults): void {
  try {
    window.localStorage.setItem(WORKSPACE_PROVIDER_DEFAULTS_STORAGE_KEY, JSON.stringify(defaults));
  } catch {
    // localStorage 不可用时静默降级，不影响当前会话内的 Provider 选择。
  }
}

/**
 * 创建请求的 provider 快照字段（REQ-PPS-01）：把用户默认里的裸字符串收窄为合法 provider
 * 名。非法名或缺失时不落键——由服务端兼容默认兜底，客户端不静默改写用户已选 provider。
 */
export function readWorkspaceProviderDefaultsSnapshot(): {
  author_provider?: WorkspaceProviderName;
  reviewer_provider?: WorkspaceProviderName;
} {
  const defaults = readWorkspaceProviderDefaults();
  const author = workspaceProviderName(defaults?.author);
  const reviewer = workspaceProviderName(defaults?.reviewer);
  return {
    ...(author ? { author_provider: author } : {}),
    ...(reviewer ? { reviewer_provider: reviewer } : {}),
  };
}
