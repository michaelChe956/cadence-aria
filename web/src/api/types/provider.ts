export type RealProviderName = "claude_code" | "codex" | "pi" | "kimi_code";

export type ProviderHealthStateStatus = "ready" | "degraded";

export type ProviderHealthReasonCode =
  | "command_missing"
  | "timeout"
  | "non_zero_exit"
  | "version_unparseable"
  | "version_too_low"
  | "io_error";

export type ProviderHealthEntry = {
  provider: RealProviderName;
  display_name: string;
  available: boolean;
  version: string | null;
  reason_code: ProviderHealthReasonCode | null;
  reason: string | null;
  checked_at: string;
  install_hint: string;
};

export type ProviderHealthResponse = {
  schema_version: number;
  generation: number;
  checked_at: string;
  state_status: ProviderHealthStateStatus;
  state_error: string | null;
  real_workflow_blocked: boolean;
  test_provider_enabled: boolean;
  providers: ProviderHealthEntry[];
};
