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
