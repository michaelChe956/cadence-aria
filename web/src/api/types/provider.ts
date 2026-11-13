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

// provider-cli-installation（REQ-PCI-01~06）：安装 / 更新的 serde snake_case
// DTO。对应 src/web/handlers/provider_installation.rs 的 GET install-targets、
// POST {target}/install 与 GET install/{job_id} 三个响应面。
// 四家 provider + codegraph 共用同一套安装机制（Kimi Code 走官方脚本，其余 npm）。
export type InstallTargetWire =
  | "claude_code"
  | "codex"
  | "pi"
  | "kimi_code"
  | "codegraph";

/** 后端判定的按钮动作：安装 / 更新 / 无需动作 / 只需核验。 */
export type InstallTargetStatus = "install" | "update" | "none" | "verify";

/** GET /api/providers/install-targets 的单行（一次返回五类对象）。 */
export type InstallTargetView = {
  target: InstallTargetWire;
  display_name: string;
  cli_available: boolean;
  local_version: string | null;
  latest_version: string | null;
  latest_checked_at: string | null;
  install_source: string;
  action: InstallTargetStatus;
  /** 已安装但所在目录不在登录 shell PATH 上（REQ-PCI-08，仅提示；行动走
   *  既有「暴露到终端 PATH」入口，未安装恒 false）。 */
  off_path: boolean;
};

export type InstallTargetsResponse = { targets: InstallTargetView[] };

/** 作业状态 durable（刷新/重启不丢进度）。 */
export type InstallJobStatus = "running" | "completed" | "failed";

export type InstallOutcome = {
  installed_version: string | null;
  /** 命令不在 PATH 时的提示目录（<prefix>/bin）。 */
  path_hint: string | null;
};

/** REQ-PCI-06 的四类失败：权限 / 网络 / 包不存在 / 其它。 */
export type InstallFailureKind =
  | "permission"
  | "network"
  | "package_not_found"
  | "other";

export type InstallFailure = {
  kind: InstallFailureKind;
  /** 后端分类后的中文提示（已含成因与处置要点）。 */
  message: string;
  /** 用户可自行运行的手动命令（npm 带 --registry；Kimi 为官方脚本）。 */
  manual_command: string | null;
};

export type InstallJob = {
  job_id: string;
  target: InstallTargetWire;
  status: InstallJobStatus;
  created_at: string;
  updated_at: string;
  outcome: InstallOutcome | null;
  failure: InstallFailure | null;
};

export type StartInstallResponse = { job_id: string };

/** POST /api/providers/{target}/expose 的响应（§五 2.11）：软链已落在
 *  登录 shell PATH 上的可写目录 `dir`，新终端可直接运行 `command`。 */
export type ExposeTerminalPathResult = {
  exposed: boolean;
  command: string;
  dir: string;
  target: string;
};
