// provider-cli-installation（块2 片7B）：安装 / 更新的前端唯一调用面。
// 三个端点对应 src/web/handlers/provider_installation.rs：
//   GET  /api/providers/install-targets      → 五类对象的本地/最新版本与动作
//   POST /api/providers/{target}/install     → 立即返回 { job_id }（异步作业）
//   GET  /api/providers/install/{job_id}     → 作业状态（running/completed/failed）
//   POST /api/providers/{target}/expose      → 把已装 CLI 软链到终端 PATH（§五 2.11）
// 刻意不写进 client.ts：保持既有 client 只做通用请求。
import { ApiRequestError, normalizeApiError } from "./client";
import type {
  ExposeTerminalPathResult,
  InstallJob,
  InstallTargetView,
  InstallTargetWire,
  InstallTargetsResponse,
  StartInstallResponse,
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

/** npm 安装默认镜像（REQ-PCI-03）；后端可被 NPM_REGISTRY 覆盖。 */
export const DEFAULT_NPM_REGISTRY = "https://registry.npmmirror.com";

/** Kimi Code 官方安装脚本（REQ-PCI-01；非 npm 包）。 */
export const KIMI_INSTALL_SCRIPT = "https://code.kimi.com/kimi-code/install.sh";

const NPM_PACKAGES: Record<Exclude<InstallTargetWire, "kimi_code">, string> = {
  claude_code: "@anthropic-ai/claude-code",
  codex: "@openai/codex",
  pi: "@earendil-works/pi-coding-agent",
  codegraph: "@colbymchenry/codegraph",
};

/** 手动兜底命令（REQ-PCI-06）：npm 类带 --registry；Kimi 为官方脚本。
 *  人工决定是否加 sudo —— 平台永不自动提权。 */
export function manualInstallCommand(target: InstallTargetWire): string {
  if (target === "kimi_code") {
    return `curl -fsSL ${KIMI_INSTALL_SCRIPT} | bash`;
  }
  return `npm install -g --registry=${DEFAULT_NPM_REGISTRY} ${NPM_PACKAGES[target]}`;
}

function parseVersion(raw: string): {
  core: [number, number, number];
  pre: string[];
} | null {
  const [withoutBuild] = raw.trim().replace(/^v/, "").split("+");
  const dashIndex = withoutBuild.indexOf("-");
  const corePart = dashIndex === -1 ? withoutBuild : withoutBuild.slice(0, dashIndex);
  const prePart = dashIndex === -1 ? "" : withoutBuild.slice(dashIndex + 1);
  const nums = corePart.split(".").map((part) => Number.parseInt(part, 10));
  if (nums.length === 0 || nums.some((value) => Number.isNaN(value))) {
    return null;
  }
  return {
    core: [nums[0] ?? 0, nums[1] ?? 0, nums[2] ?? 0],
    pre: prePart === "" ? [] : prePart.split("."),
  };
}

/** 精确 semver 比较（REQ-PCI-07）：预发布低于同号正式版；无法解析返回 NaN
 *  （调用方据此不提供「更新」，不做字符串近似比较）。 */
export function compareVersions(left: string, right: string): number {
  const a = parseVersion(left);
  const b = parseVersion(right);
  if (a === null || b === null) {
    return Number.NaN;
  }
  for (let index = 0; index < 3; index += 1) {
    if (a.core[index] !== b.core[index]) {
      return a.core[index] - b.core[index];
    }
  }
  if (a.pre.length === 0 && b.pre.length === 0) {
    return 0;
  }
  // 无预发布后缀 = 正式版 > 同号预发布。
  if (a.pre.length === 0) {
    return 1;
  }
  if (b.pre.length === 0) {
    return -1;
  }
  const length = Math.min(a.pre.length, b.pre.length);
  for (let index = 0; index < length; index += 1) {
    const leftId = a.pre[index];
    const rightId = b.pre[index];
    const leftNum = /^\d+$/.test(leftId) ? Number.parseInt(leftId, 10) : null;
    const rightNum = /^\d+$/.test(rightId) ? Number.parseInt(rightId, 10) : null;
    if (leftNum !== null && rightNum !== null) {
      if (leftNum !== rightNum) {
        return leftNum - rightNum;
      }
    } else if (leftNum !== null) {
      return -1; // 数字标识 < 字母标识
    } else if (rightNum !== null) {
      return 1;
    } else if (leftId !== rightId) {
      return leftId < rightId ? -1 : 1;
    }
  }
  return a.pre.length - b.pre.length;
}

/** 一次取回五类对象的本地状态、最新版本与后端判定的动作。 */
export async function getInstallTargets(): Promise<InstallTargetView[]> {
  const payload = await requestJson<
    InstallTargetsResponse | InstallTargetView[]
  >("/api/providers/install-targets");
  if (Array.isArray(payload)) {
    return payload;
  }
  if (payload && Array.isArray(payload.targets)) {
    return payload.targets;
  }
  throw new Error("安装目标面负载不合规（缺少 targets 数组）");
}

/** 发起安装：立即返回 job_id（REQ-PCI-05，不阻塞等待安装完成）。
 *  Kimi Code 走远程脚本，后端要求 acknowledge_remote_script=true。 */
export async function startInstall(
  target: InstallTargetWire,
  options?: { acknowledgeRemoteScript?: boolean },
): Promise<StartInstallResponse> {
  return requestJson<StartInstallResponse>(
    `/api/providers/${encodeURIComponent(target)}/install`,
    {
      method: "POST",
      body: JSON.stringify(
        options?.acknowledgeRemoteScript
          ? { acknowledge_remote_script: true }
          : {},
      ),
    },
  );
}

/** 轮询作业进度与结果（durable：刷新页面后仍可查）。 */
export async function getInstallJob(jobId: string): Promise<InstallJob> {
  return requestJson<InstallJob>(
    `/api/providers/install/${encodeURIComponent(jobId)}`,
  );
}

/** 把已安装的 CLI 暴露到用户终端 PATH（§五 2.11）：后端只在登录 shell
 *  PATH 上真实可写的目录建**单个软链**；不写 shell profile、不改 PATH
 *  环境变量。用户明确点击才调用（安装 / 更新后任意版本可用）。 */
export async function exposeToTerminalPath(
  target: InstallTargetWire,
): Promise<ExposeTerminalPathResult> {
  return requestJson<ExposeTerminalPathResult>(
    `/api/providers/${encodeURIComponent(target)}/expose`,
    { method: "POST" },
  );
}
