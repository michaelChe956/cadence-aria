// 票3「一键核验全部」批量作业回归（迁自原 ProviderCapabilityBatchBar.test.tsx）。
//
// - 点击 → POST 发起异步作业（立即拿到 job_id），按钮「核验中（N/4）」+ 禁用；
// - 终态停止轮询（不无限轮询）；
// - 按钮区说明作用域与成本（逐 LC 核验 / 真实探测 / 几分钟 / 消耗额度）。
//
// owner 2026-11-04：逐项结果（逐项状态 / 失败项【重试】/ 跳过项【安装】/
// 版本落后【更新】）已迁到能力卡的**行级列表**，原 `ProviderCapabilityBatchResults`
// 组件随之删除 ⇒ 与逐项结果相关的 4 组断言迁到 `ProviderCapabilityCard.test.tsx`
// （断言对象换成行级列表的对应行，断言内容不弱化）；本文件只保留与作业状态 /
// 按钮 / 说明区相关的断言。
import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  ProviderCapabilityBatchScope,
  ProviderCapabilityBatchStartButton,
  useProviderCapabilityBatch,
} from "./ProviderCapabilityBatch";
import type {
  CapabilityRevalidateBatchItemDto,
  CapabilityRevalidateBatchJobDto,
} from "../../api/types/capability-revalidate-batch";
import type { ProviderCapabilityType } from "../../api/types/provider-capability";

const PROJECT_ID = "project_0001";
const LC_ID = "logical_codebase_0001";
const BATCH_URL = `/api/projects/${PROJECT_ID}/logical-codebases/${LC_ID}/capabilities/revalidate-all`;
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

/** POST 发起 / GET 轮询作业的真实发送器路由（逐项结果的渲染已迁到能力卡）。 */
function installBatchRouter(options: {
  jobIds?: string[];
  jobs?: (() => CapabilityRevalidateBatchJobDto)[];
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
    return jsonResponse(
      { code: "not_found", message: `no route for ${method} ${url}` },
      404,
    );
  };
  vi.stubGlobal("fetch", vi.fn(router));
  return { requests };
}

/** 测试宿主：把 hook 与按钮 / 说明区按能力卡的摆法组合起来。 */
function BatchHost() {
  const state = useProviderCapabilityBatch(PROJECT_ID, LC_ID);
  return (
    <div>
      <ProviderCapabilityBatchStartButton state={state} />
      <ProviderCapabilityBatchScope />
    </div>
  );
}

function renderBatch() {
  return render(<BatchHost />);
}

afterEach(() => {
  vi.unstubAllGlobals();
  vi.useRealTimers();
  window.localStorage.clear();
});

describe("ProviderCapabilityBatch（票3 一键批量核验）", () => {
  it("点击后显示「核验中（N/4）」、按钮禁用，只发起一次整批作业", async () => {
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
    renderBatch();

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

    // 逐项状态（已确认 / 核验中）本身由能力卡的行级列表承载 ⇒ 迁到
    // ProviderCapabilityCard.test.tsx 的行级列表断言（内容不弱化）。
    expect(startButton).toHaveTextContent("核验中（1/4）");
    expect(
      requests.filter(
        (request) => request.method === "POST" && request.url === BATCH_URL,
      ),
    ).toHaveLength(1);
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
    renderBatch();

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

  it("按钮区说明核验作用域与成本", () => {
    installBatchRouter({});
    renderBatch();

    const scope = screen.getByTestId("capability-batch-scope");
    expect(scope).toHaveTextContent("逐 LC 核验");
    expect(scope).toHaveTextContent("真实跑一次探测");
    expect(scope).toHaveTextContent("几分钟");
    expect(scope).toHaveTextContent("额度");
  });
});
