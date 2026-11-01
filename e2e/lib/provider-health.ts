import { apiGet, type ApiReadResult } from "./api-reader.ts";
import { delay } from "../env/wait.ts";

/// provider health 预热(矩阵 t12 先例的 Node 面):
/// POST /api/providers/recheck 主动刷新(真实 CLI --version 探测)→
/// GET /api/providers/status 有界轮询,直到目标 provider available。
/// 只读+产品自带 recheck 面,不发任何业务变更请求;超时如实返回未就绪事实。

export type ProviderStatusEntry = {
  provider: string;
  available: boolean;
  version?: string | null;
  detail?: string;
};

export type ProviderHealthSnapshot = {
  stateStatus: string | null;
  providers: ProviderStatusEntry[];
  httpStatus: number;
  at: string;
};

export type HealthWaitOutcome = {
  ready: boolean;
  provider: string;
  version: string | null;
  waitedMs: number;
  lastSnapshot: ProviderHealthSnapshot | null;
};

async function postRecheck(baseURL: string): Promise<void> {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), 15_000);
  try {
    await fetch(`${baseURL}/api/providers/recheck`, {
      method: "POST",
      headers: { accept: "application/json" },
      signal: controller.signal,
    });
  } catch {
    // recheck 失败不阻断:下一轮采样继续;status 读面才是判定来源。
  } finally {
    clearTimeout(timer);
  }
}

export async function readProviderStatus(baseURL: string): Promise<ProviderHealthSnapshot> {
  const response: ApiReadResult = await apiGet(baseURL, "/api/providers/status", 15_000);
  const body = (response.body ?? {}) as {
    state_status?: string;
    providers?: { provider?: string; available?: boolean; version?: string | null; detail?: string }[];
  };
  return {
    stateStatus: body.state_status ?? null,
    providers: (body.providers ?? []).map((entry) => ({
      provider: entry.provider ?? "",
      available: entry.available === true,
      version: entry.version ?? null,
      detail: entry.detail,
    })),
    httpStatus: response.status,
    at: response.at,
  };
}

/**
 * 有界轮询直到 provider available(recheck+status 双面;矩阵同款 5s 采样)。
 * 不抛超时:返回 ready=false + 最后快照,由调用方按 BLOCKED 如实落证。
 */
export async function waitForProviderHealth(options: {
  baseURL: string;
  provider: string;
  timeoutMs: number;
  onHeartbeat?: (snapshot: ProviderHealthSnapshot | null, waitedMs: number) => void;
}): Promise<HealthWaitOutcome> {
  const startedAt = Date.now();
  let lastSnapshot: ProviderHealthSnapshot | null = null;
  for (;;) {
    await postRecheck(options.baseURL);
    try {
      lastSnapshot = await readProviderStatus(options.baseURL);
    } catch {
      // status 读取失败(网络瞬断):保留上一快照,继续轮询。
    }
    const entry = lastSnapshot?.providers.find((candidate) => candidate.provider === options.provider);
    if (lastSnapshot && lastSnapshot.stateStatus === "ready" && entry?.available === true) {
      return {
        ready: true,
        provider: options.provider,
        version: entry.version ?? null,
        waitedMs: Date.now() - startedAt,
        lastSnapshot,
      };
    }
    const waitedMs = Date.now() - startedAt;
    options.onHeartbeat?.(lastSnapshot, waitedMs);
    if (waitedMs >= options.timeoutMs) {
      return { ready: false, provider: options.provider, version: null, waitedMs, lastSnapshot };
    }
    await delay(Math.min(5_000, options.timeoutMs - waitedMs));
  }
}
