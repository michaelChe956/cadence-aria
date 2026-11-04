// 票3「一键核验全部」批量条回归（T6.10~T6.14）。
//
// - 点击 → POST 发起异步作业（立即拿到 job_id），按钮「核验中（N/4）」+ 禁用，
//   逐项状态随 2.5s 轮询更新；
// - 失败行【重试】只打既有单 provider 端点（不重跑整批）；
// - 跳过行显示【安装】（块2 组件），不显示为「核验失败」；
// - 终态停止轮询（不无限轮询）；
// - 按钮区说明作用域与成本（逐 LC 核验 / 真实探测 / 几分钟 / 消耗额度）。
import { act, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ProviderCapabilityBatchBar } from "./ProviderCapabilityBatchBar";
import type {
  CapabilityRevalidateBatchItemDto,
  CapabilityRevalidateBatchJobDto,
} from "../../api/types/capability-revalidate-batch";
import type { ProviderCapabilityType } from "../../api/types/provider-capability";
import type { InstallTargetView } from "../../api/types/provider";

const PROJECT_ID = "project_0001";
const LC_ID = "logical_codebase_0001";
const BATCH_URL = `/api/projects/${PROJECT_ID}/logical-codebases/${LC_ID}/capabilities/revalidate-all`;
const SINGLE_URL = `/api/projects/${PROJECT_ID}/logical-codebases/${LC_ID}/capability-revalidate`;
const STORAGE_KEY = `aria:capability-batch:${PROJECT_ID}:${LC_ID}`;

const PROVIDERS: ProviderCapabilityType[] = [
  "claude_code",
  "codex",
  "pi",
  "kimi_code",
];

type CapturedRequest = { url: string; method: string; body: unknown };

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}

function item(
  providerType: ProviderCapabilityType,
  status: CapabilityRevalidateBatchItemDto["status"],
  reason: string | null = null,
): CapabilityRevalidateBatchItemDto {
  return {
    provider_type: providerType,
    status,
    reason,
    detail: null,
    version: status === "confirmed" ? "1.0.0" : null,
    started_at: "2026-11-03T00:00:00.000Z",
    finished_at: status === "running" ? null : "2026-11-03T00:01:00.000Z",
  };
}

function job(
  jobId: string,
  status: CapabilityRevalidateBatchJobDto["status"],
  results: CapabilityRevalidateBatchItemDto[],
): CapabilityRevalidateBatchJobDto {
  return {
    job_id: jobId,
    status,
    created_at: "2026-11-03T00:00:00.000Z",
    updated_at: "2026-11-03T00:01:00.000Z",
    finished_at: status === "completed" ? "2026-11-03T00:01:00.000Z" : null,
    results,
  };
}

function allRunning(jobId: string): CapabilityRevalidateBatchJobDto {
  return job(
    jobId,
    "running",
    PROVIDERS.map((provider) => item(provider, "running")),
  );
}

function targetView(
  overrides: Partial<InstallTargetView> & { target: InstallTargetView["target"] },
): InstallTargetView {
  return {
    display_name: overrides.target,
    cli_available: false,
    local_version: null,
    latest_version: null,
    latest_checked_at: null,
    install_source: "npm",
    action: "none",
    ...overrides,
  };
}

/** POST 发起 / GET 轮询 / 单 provider 重试 + 块2 install-targets 的真实发送器路由。 */
function installBatchRouter(options: {
  jobIds?: string[];
  jobs?: (() => CapabilityRevalidateBatchJobDto)[];
  installTargets?: InstallTargetView[];
}): { requests: CapturedRequest[] } {
  const requests: CapturedRequest[] = [];
  const jobIds = [...(options.jobIds ?? ["job_0001"])];
  const jobs = [...(options.jobs ?? [])];
  const router = async (
    input: RequestInfo | URL,
    init?: RequestInit,
  ): Promise<Response> => {
    const url = typeof input === "string" ? input : input.toString();
    const method = (init?.method ?? "GET").toUpperCase();
    const rawBody = typeof init?.body === "string" ? init.body : null;
    requests.push({
      url,
      method,
      body: rawBody === null ? null : JSON.parse(rawBody),
    });
    if (method === "POST" && url === BATCH_URL) {
      return jsonResponse(
        { job_id: jobIds.shift() ?? "job_0001", status: "running" },
        202,
      );
    }
    const jobMatch = /\/capabilities\/revalidate-all\/(.+)$/.exec(url);
    if (method === "GET" && jobMatch) {
      const responder = jobs.length > 1 ? jobs.shift()! : jobs[0];
      return jsonResponse(responder ? responder() : allRunning(jobMatch[1]));
    }
    if (method === "GET" && url === "/api/providers/install-targets") {
      return jsonResponse({ targets: options.installTargets ?? [] });
    }
    if (method === "POST" && url === SINGLE_URL) {
      return jsonResponse({
        provider_type: "codex",
        outcome: "already_confirmed",
        version: "1.0.0",
      });
    }
    return jsonResponse(
      { code: "not_found", message: `no route for ${method} ${url}` },
      404,
    );
  };
  vi.stubGlobal("fetch", vi.fn(router));
  return { requests };
}

function renderBar() {
  return render(
    <ProviderCapabilityBatchBar
      projectId={PROJECT_ID}
      logicalCodebaseId={LC_ID}
    />,
  );
}

afterEach(() => {
  vi.unstubAllGlobals();
  vi.useRealTimers();
  window.localStorage.clear();
});

describe("ProviderCapabilityBatchBar（票3 一键批量核验）", () => {
  it("点击后显示「核验中（N/4）」、按钮禁用，逐项状态随轮询更新", async () => {
    vi.useFakeTimers();
    const { requests } = installBatchRouter({
      jobIds: ["job_0001"],
      jobs: [
        () =>
          job("job_0001", "running", [
            item("claude_code", "confirmed", "revalidated"),
            item("codex", "running"),
            item("pi", "running"),
            item("kimi_code", "running"),
          ]),
      ],
    });
    renderBar();

    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "一键核验全部" }));
      await Promise.resolve();
    });

    const startButton = screen.getByTestId("capability-batch-start");
    expect(startButton).toBeDisabled();
    expect(startButton).toHaveTextContent("核验中（0/4）");

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    expect(
      screen.getByTestId("capability-batch-status-claude_code"),
    ).toHaveTextContent("已确认");
    expect(
      screen.getByTestId("capability-batch-status-codex"),
    ).toHaveTextContent("核验中");
    expect(startButton).toHaveTextContent("核验中（1/4）");
    expect(
      requests.filter(
        (request) => request.method === "POST" && request.url === BATCH_URL,
      ),
    ).toHaveLength(1);
  });

  it("失败行保留【重试】，只打单 provider 端点且不重跑整批", async () => {
    vi.useFakeTimers();
    window.localStorage.setItem(STORAGE_KEY, "job_0001");
    const { requests } = installBatchRouter({
      jobs: [
        () =>
          job("job_0001", "completed", [
            item("claude_code", "confirmed", "revalidated"),
            item("codex", "failed", "probe_failed"),
            item("pi", "confirmed", "revalidated"),
            item("kimi_code", "confirmed", "revalidated"),
          ]),
      ],
    });
    renderBar();

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    expect(screen.getByTestId("capability-batch-status-codex")).toHaveTextContent(
      "核验失败",
    );
    expect(
      screen.queryByTestId("capability-batch-retry-claude_code"),
    ).not.toBeInTheDocument();

    await act(async () => {
      fireEvent.click(screen.getByTestId("capability-batch-retry-codex"));
      await Promise.resolve();
      await Promise.resolve();
    });

    const singleRetries = requests.filter(
      (request) => request.method === "POST" && request.url === SINGLE_URL,
    );
    expect(singleRetries).toHaveLength(1);
    expect(singleRetries[0].body).toEqual({ provider_type: "codex" });
    expect(
      requests.filter(
        (request) => request.method === "POST" && request.url === BATCH_URL,
      ),
    ).toHaveLength(0);
  });

  it("跳过行显示【安装】入口，不显示为核验失败", async () => {
    vi.useFakeTimers();
    window.localStorage.setItem(STORAGE_KEY, "job_0001");
    installBatchRouter({
      jobs: [
        () =>
          job("job_0001", "completed", [
            item("claude_code", "confirmed", "revalidated"),
            item("codex", "confirmed", "revalidated"),
            item("pi", "skipped", "cli_not_installed"),
            item("kimi_code", "confirmed", "revalidated"),
          ]),
      ],
      installTargets: [
        targetView({
          target: "pi",
          cli_available: false,
          latest_version: "1.2.3",
          action: "install",
        }),
      ],
    });
    renderBar();

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    const skippedRow = screen.getByTestId("capability-batch-row-pi");
    expect(
      within(skippedRow).getByTestId("capability-batch-status-pi"),
    ).toHaveTextContent("跳过");
    expect(within(skippedRow).queryByText(/核验失败/)).not.toBeInTheDocument();
    expect(
      within(skippedRow).getByRole("button", { name: "安装" }),
    ).toBeInTheDocument();
    // 安装行必须同时给出可抄的手动命令（块2 组件按 cliAvailable=false 渲染，
    // 批量条不隐藏、不改写）。
    expect(
      within(skippedRow).getByTestId("provider-install-manual-command-pi"),
    ).toHaveTextContent("npm install -g");
    expect(
      within(skippedRow).queryByRole("button", { name: "重试" }),
    ).not.toBeInTheDocument();
  });

  it("终态后停止轮询，不无限轮询", async () => {
    vi.useFakeTimers();
    window.localStorage.setItem(STORAGE_KEY, "job_0001");
    const { requests } = installBatchRouter({
      jobs: [
        () =>
          job("job_0001", "running", [
            item("claude_code", "confirmed", "revalidated"),
            item("codex", "running"),
            item("pi", "running"),
            item("kimi_code", "running"),
          ]),
        () =>
          job(
            "job_0001",
            "completed",
            PROVIDERS.map((provider) => item(provider, "confirmed", "revalidated")),
          ),
      ],
    });
    renderBar();

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(2_500);
    });

    expect(screen.getByTestId("capability-batch-start")).toHaveTextContent(
      "一键核验全部",
    );
    const getCount = () =>
      requests.filter((request) => request.method === "GET").length;
    const settledPolls = getCount();
    expect(settledPolls).toBeGreaterThanOrEqual(2);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(10_000);
    });
    expect(getCount()).toBe(settledPolls);
  });

  it("版本落后行渲染「版本落后」+【更新】与当前/最新版本（不是【安装】）", async () => {
    vi.useFakeTimers();
    window.localStorage.setItem(STORAGE_KEY, "job_0001");
    installBatchRouter({
      jobs: [
        () =>
          job(
            "job_0001",
            "completed",
            PROVIDERS.map((provider) => item(provider, "confirmed", "revalidated")),
          ),
      ],
      installTargets: [
        targetView({
          target: "codex",
          cli_available: true,
          local_version: "1.0.0",
          latest_version: "2.0.0",
          action: "update",
        }),
      ],
    });
    renderBar();

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    const row = screen.getByTestId("capability-batch-row-codex");
    expect(
      within(row).getByTestId("capability-batch-outdated-codex"),
    ).toHaveTextContent("版本落后");
    expect(within(row).getByRole("button", { name: "更新" })).toBeInTheDocument();
    const versions = within(row).getByTestId("provider-install-versions-codex");
    expect(versions).toHaveTextContent("1.0.0");
    expect(versions).toHaveTextContent("2.0.0");
    expect(
      within(row).queryByRole("button", { name: "安装" }),
    ).not.toBeInTheDocument();
  });

  it("按钮区说明核验作用域与成本", () => {
    installBatchRouter({});
    renderBar();

    const scope = screen.getByTestId("capability-batch-scope");
    expect(scope).toHaveTextContent("逐 LC 核验");
    expect(scope).toHaveTextContent("真实跑一次探测");
    expect(scope).toHaveTextContent("几分钟");
    expect(scope).toHaveTextContent("额度");
  });
});
