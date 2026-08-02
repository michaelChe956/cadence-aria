import type {
  ProviderHealthEntry,
  ProviderHealthResponse,
  RealProviderName,
  WorkspaceProviderName,
} from "../api/types";

export type ProviderOption = {
  value: WorkspaceProviderName;
  label: string;
  visible: boolean;
  disabled: boolean;
  available: boolean;
  reason: string | null;
  installHint: string | null;
  real: boolean;
};

type RealProviderCatalogEntry = {
  value: RealProviderName;
  fallbackLabel: string;
};

const REAL_PROVIDER_CATALOG: readonly RealProviderCatalogEntry[] = [
  { value: "claude_code", fallbackLabel: "Claude Code" },
  { value: "codex", fallbackLabel: "Codex" },
  { value: "pi", fallbackLabel: "Pi" },
];

export const PROVIDER_ORDER: readonly WorkspaceProviderName[] = [
  "claude_code",
  "codex",
  "pi",
  "fake",
];

function blockedReason(
  snapshot: ProviderHealthResponse | null,
  entry: ProviderHealthEntry | undefined,
): string | null {
  if (!snapshot || !entry) {
    return "Provider 状态尚未确认";
  }
  if (entry.reason) {
    return entry.reason;
  }
  if (!entry.available) {
    return "Provider 当前不可用";
  }
  if (!snapshot.real_workflow_blocked) {
    return null;
  }
  if (snapshot.state_error) {
    return snapshot.state_error;
  }
  if (snapshot.state_status === "degraded") {
    return "Provider 健康状态已降级";
  }
  return "真实 Provider 工作流暂不可用";
}

function realProviderOption(
  catalogEntry: RealProviderCatalogEntry,
  snapshot: ProviderHealthResponse | null,
): ProviderOption {
  const entry = snapshot?.providers.find(
    (provider) => provider.provider === catalogEntry.value,
  );
  const available = Boolean(
    entry?.available && !snapshot?.real_workflow_blocked,
  );

  return {
    value: catalogEntry.value,
    label: entry?.display_name.trim() || catalogEntry.fallbackLabel,
    visible: true,
    disabled: !available,
    available,
    reason: blockedReason(snapshot, entry),
    installHint: entry?.install_hint ?? null,
    real: true,
  };
}

function fakeProviderOption(
  snapshot: ProviderHealthResponse | null,
): ProviderOption {
  const visible = snapshot?.test_provider_enabled === true;
  return {
    value: "fake",
    label: "Fake",
    visible,
    disabled: !visible,
    available: visible,
    reason: visible ? null : "Fake Provider 仅在测试模式下可用",
    installHint: null,
    real: false,
  };
}

export function getProviderOptions(
  snapshot: ProviderHealthResponse | null,
): ProviderOption[] {
  return [
    ...REAL_PROVIDER_CATALOG.map((entry) =>
      realProviderOption(entry, snapshot),
    ),
    fakeProviderOption(snapshot),
  ];
}

export function getProviderOption(
  snapshot: ProviderHealthResponse | null,
  provider: WorkspaceProviderName,
): ProviderOption {
  return getProviderOptions(snapshot).find((option) => option.value === provider)!;
}
