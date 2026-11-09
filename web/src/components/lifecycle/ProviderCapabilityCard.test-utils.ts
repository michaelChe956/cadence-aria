// ProviderCapabilityCard 测试夹具（共享常量 / 投影 / 批量作业 / fetch 路由）。
//
// 2026-11-09：ProviderCapabilityCard.test.tsx 触及 1200 行硬门禁
// （tests/it_core/large_file_guard.rs）⇒ 共享夹具整体搬到本文件；
// 用例与断言逐字保留在测试文件里，行为零变化。
import { vi } from "vitest";
import type {
  ProviderCapabilityActionName,
  ProviderCapabilityActionRowDto,
  ProviderCapabilityEvidenceStatus,
  ProviderCapabilitiesResponse,
} from "../../api/types";

export const BATCH_JOB_ID = "job_0001";
export const BATCH_STORAGE_KEY = "aria:capability-batch";
export const CAPABILITIES_URL = "/api/providers/capabilities";
export const REVALIDATE_URL = "/api/providers/capabilities/revalidate";
export const BATCH_URL = "/api/providers/capabilities/revalidate-all";

export type CapturedRequest = { url: string; method: string; body: unknown };

/** 真实 fetch 发送器收到的请求（installFetchRouter 写入，用例逐条断言）。 */
export const captured: CapturedRequest[] = [];

export function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}

export type RevalidateResponder = () => Promise<Response>;

/** GET 投影 + 安装目标面 + 可配置 POST 应答的 fetch 路由（真实发送器路径）。 */
export function installFetchRouter(
  getResponse: () => ProviderCapabilitiesResponse,
  postResponse?: RevalidateResponder,
  installTargets: unknown = { targets: [] },
  batchJob: unknown = null,
): void {
  captured.length = 0;
  const router = async (
    input: RequestInfo | URL,
    init?: RequestInit,
  ): Promise<Response> => {
    const url = typeof input === "string" ? input : input.toString();
    const method = (init?.method ?? "GET").toUpperCase();
    const rawBody = typeof init?.body === "string" ? init.body : null;
    captured.push({
      url,
      method,
      body: rawBody === null ? null : JSON.parse(rawBody),
    });
    if (method === "GET" && url === CAPABILITIES_URL) {
      return jsonResponse(getResponse());
    }
    if (method === "GET" && url === "/api/providers/install-targets") {
      return jsonResponse(installTargets);
    }
    if (method === "GET" && url === `${BATCH_URL}/${BATCH_JOB_ID}`) {
      return jsonResponse(batchJob);
    }
    if (method === "POST" && url === REVALIDATE_URL) {
      if (!postResponse) {
        return jsonResponse(
          { code: "not_found", message: `no POST route for ${url}` },
          404,
        );
      }
      return postResponse();
    }
    return jsonResponse(
      { code: "not_found", message: `no route for ${method} ${url}` },
      404,
    );
  };
  vi.stubGlobal("fetch", vi.fn(router));
}

export const row = (
  action: ProviderCapabilityActionName,
  launch: ProviderCapabilityEvidenceStatus,
  resume: ProviderCapabilityEvidenceStatus,
  writeBoundary: ProviderCapabilityEvidenceStatus,
): ProviderCapabilityActionRowDto => ({
  action,
  launch,
  resume,
  write_boundary: writeBoundary,
  evidence_ref: `evidence/${action}`,
});

/** 未探测基线：bootstrap 默认记录不是真实证据，全 Unknown。 */
export function unknownProjection(): ProviderCapabilitiesResponse {
  return {
    providers: [
      {
        provider_type: "claude_code",
        cli_program: "claude",
        version: "1.0.34",
        probed_at: "2026-10-10T00:00:00Z",
        probe_artifact_ref: "artifact://probe/claude",
        rows: [
          row("coding_target_write", "confirmed", "confirmed", "confirmed"),
          row("planning_read_only", "confirmed", "confirmed", "confirmed"),
          row("review_read_only", "confirmed", "confirmed", "confirmed"),
        ],
      },
      {
        provider_type: "codex",
        cli_program: "codex",
        version: null,
        probed_at: null,
        probe_artifact_ref: null,
        rows: [
          row("coding_target_write", "unknown", "unknown", "unknown"),
          row("planning_read_only", "unknown", "unknown", "unknown"),
          row("review_read_only", "unknown", "unknown", "unknown"),
        ],
      },
      {
        provider_type: "pi",
        cli_program: "pi",
        version: null,
        probed_at: null,
        probe_artifact_ref: null,
        rows: [
          row("coding_target_write", "unknown", "unknown", "unknown"),
          row("planning_read_only", "unknown", "unknown", "unknown"),
          row("review_read_only", "unknown", "unknown", "unknown"),
        ],
      },
      {
        provider_type: "kimi_code",
        cli_program: "kimi",
        version: "0.9.1",
        probed_at: "2026-10-09T00:00:00Z",
        probe_artifact_ref: "artifact://probe/kimi",
        rows: [
          row("coding_target_write", "confirmed", "confirmed", "denied"),
          row("planning_read_only", "unknown", "confirmed", "denied"),
          row("review_read_only", "unknown", "confirmed", "denied"),
        ],
      },
    ],
  };
}

/** 全行 Confirmed 投影（revalidate 成功后 GET 的刷新事实）。 */
export function confirmedCodexProjection(): ProviderCapabilitiesResponse {
  const projection = unknownProjection();
  const codex = projection.providers.find(
    (provider) => provider.provider_type === "codex",
  )!;
  codex.version = "0.44.0";
  codex.probed_at = "2026-10-10T01:00:00Z";
  codex.probe_artifact_ref = "artifact://probe/codex";
  codex.rows = [
    row("coding_target_write", "confirmed", "confirmed", "confirmed"),
    row("planning_read_only", "confirmed", "confirmed", "confirmed"),
    row("review_read_only", "confirmed", "confirmed", "confirmed"),
  ];
  return projection;
}

/** 批量核验作业（终态）：逐项状态并入行级列表。 */
export function batchJob(codexStatus: "confirmed" | "failed" = "failed"): unknown {
  return {
    job_id: BATCH_JOB_ID,
    status: "completed",
    created_at: "2026-11-04T00:00:00Z",
    updated_at: "2026-11-04T00:01:00Z",
    finished_at: "2026-11-04T00:01:00Z",
    results: [
      {
        provider_type: "claude_code",
        status: "confirmed",
        reason: "revalidated",
        detail: null,
        version: "1.0.34",
        started_at: null,
        finished_at: null,
      },
      {
        provider_type: "codex",
        status: codexStatus,
        reason:
          codexStatus === "confirmed"
            ? "revalidated"
            : "probe_failed",
        detail:
          codexStatus === "confirmed"
            ? null
            : "codex --version exited 127: command not found",
        version: codexStatus === "confirmed" ? "0.44.0" : null,
        started_at: null,
        finished_at: null,
      },
      {
        provider_type: "pi",
        status: "skipped",
        reason: "cli_not_installed",
        detail: null,
        version: null,
        started_at: null,
        finished_at: null,
      },
      {
        provider_type: "kimi_code",
        status: "confirmed",
        reason: "already_confirmed",
        detail: null,
        version: "0.9.1",
        started_at: null,
        finished_at: null,
      },
    ],
  };
}

/** 批量核验作业（进行中）：逐项状态随轮询更新。 */
export function runningBatchJob(): unknown {
  return {
    job_id: BATCH_JOB_ID,
    status: "running",
    created_at: "2026-11-04T00:00:00Z",
    updated_at: "2026-11-04T00:00:30Z",
    finished_at: null,
    results: [
      {
        provider_type: "claude_code",
        status: "confirmed",
        reason: "revalidated",
        detail: null,
        version: "1.0.34",
        started_at: null,
        finished_at: null,
      },
      {
        provider_type: "codex",
        status: "running",
        reason: null,
        detail: null,
        version: null,
        started_at: null,
        finished_at: null,
      },
      {
        provider_type: "pi",
        status: "running",
        reason: null,
        detail: null,
        version: null,
        started_at: null,
        finished_at: null,
      },
      {
        provider_type: "kimi_code",
        status: "running",
        reason: null,
        detail: null,
        version: null,
        started_at: null,
        finished_at: null,
      },
    ],
  };
}
