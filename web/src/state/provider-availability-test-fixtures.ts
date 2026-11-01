import type {
  ProviderHealthEntry,
  ProviderHealthResponse,
  RealProviderName,
} from "../api/types";

const PROVIDER_LABELS: Record<RealProviderName, string> = {
  claude_code: "Claude Code",
  codex: "Codex",
  pi: "Pi",
  kimi_code: "Kimi Code",
};

/**
 * `/api/providers/status` 快照 fixture：只声明关心的 provider 可用性，其余按未安装构造
 * （与 ProviderConfigPanel / provider-options 的置灰判据同形）。
 */
export function providerHealthSnapshot(
  available: Partial<Record<RealProviderName, boolean>>,
): ProviderHealthResponse {
  return {
    schema_version: 1,
    generation: 1,
    checked_at: "2026-09-22T00:00:00Z",
    state_status: "ready",
    state_error: null,
    real_workflow_blocked: false,
    test_provider_enabled: false,
    providers: (Object.keys(PROVIDER_LABELS) as RealProviderName[]).map(
      (provider): ProviderHealthEntry => {
        const isAvailable = available[provider] === true;
        return {
          provider,
          display_name: PROVIDER_LABELS[provider],
          available: isAvailable,
          version: isAvailable ? "1.0.0" : null,
          reason_code: isAvailable ? null : "command_missing",
          reason: isAvailable ? null : `${PROVIDER_LABELS[provider]} 未安装`,
          checked_at: "2026-09-22T00:00:00Z",
          install_hint: isAvailable ? "" : `请先安装 ${PROVIDER_LABELS[provider]}`,
        };
      },
    ),
  };
}
