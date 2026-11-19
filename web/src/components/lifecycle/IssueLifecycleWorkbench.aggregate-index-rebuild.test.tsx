import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { AggregateIndexActiveResponse } from "../../api/types";
import { IssueLifecycleWorkbench } from "./IssueLifecycleWorkbench";
import {
  deferred,
  installIssueLifecycleWorkbenchTestHooks,
  jsonResponse,
  lifecycleFetch,
  projectRecord,
} from "./IssueLifecycleWorkbench.test-utils";

vi.mock("../shared/MonacoViewer", () => ({
  MonacoViewer: ({ value }: { value: string }) => (
    <div data-testid="monaco-viewer">{value}</div>
  ),
}));

function member(
  logical_repository_id: string,
  alias: string,
  physical_repository_id: string | null,
) {
  return {
    logical_repository_id,
    alias,
    status: "active" as const,
    physical_repository_id,
  };
}

function activeIndex(
  overrides: Partial<AggregateIndexActiveResponse> = {},
): AggregateIndexActiveResponse {
  return {
    state: "stale",
    revision: 3,
    indexed_at: "2026-11-01T00:00:00Z",
    warning: "旧告警：历史扫描诊断",
    ...overrides,
  };
}

function errorResponse(status: number, message: string): Response {
  return new Response(
    JSON.stringify({ code: "aggregate_index_failed", message }),
    { status },
  );
}

type FetchCall = { method: string; url: string };

/** GET active 的返回：静态快照，或按调用时点求值（先旧后新/后续失败）。 */
type ActiveIndexSupplier =
  | AggregateIndexActiveResponse
  | (() => AggregateIndexActiveResponse | Response | Promise<Response>);

/**
 * 聚合索引重建测试专用 fetch 装载：按 LC 区分 GET active 的返回，
 * POST rebuild 交给 onRebuild（可返回 deferred 实现延迟响应）。
 */
function installAggregateRebuildFetch(setup: {
  logicalCodebases?: Array<{ id: string; name: string }>;
  activeIndexByLc?: Record<string, ActiveIndexSupplier>;
  onRebuild?: (lcId: string) => Response | Promise<Response>;
}) {
  const calls: FetchCall[] = [];
  const logicalCodebases = setup.logicalCodebases ?? [
    { id: "lc_0001", name: "platform" },
  ];
  const activeIndexByLc: Record<string, ActiveIndexSupplier> = {
    lc_0001: activeIndex(),
    lc_0002: activeIndex({
      state: "active",
      revision: 5,
      indexed_at: "2026-11-02T00:00:00Z",
      warning: null,
    }),
    ...setup.activeIndexByLc,
  };
  const fetchMock = lifecycleFetch({
    projects: [projectRecord("project_0001", "Aria")],
    logicalCodebases: logicalCodebases.map((codebase) => ({
      ...codebase,
      member_count: 1,
    })),
    logicalCodebaseMembersByLc: Object.fromEntries(
      logicalCodebases.map((codebase, index) => [
        codebase.id,
        [
          member(
            `lr-${index + 1}`,
            codebase.name,
            `repository_${1000 + index + 1}`,
          ),
        ],
      ]),
    ),
  });
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = String(input);
      const aggregateMatch = url.match(
        /^\/api\/projects\/([^/]+)\/logical-codebases\/([^/]+)\/aggregate-indexes\/(active|rebuild)$/,
      );
      if (aggregateMatch) {
        const lcId = decodeURIComponent(aggregateMatch[2]);
        calls.push({
          method:
            aggregateMatch[3] === "rebuild" ? (init?.method ?? "POST") : "GET",
          url,
        });
        if (aggregateMatch[3] === "rebuild") {
          return (
            setup.onRebuild?.(lcId) ??
            jsonResponse(activeIndexByLc[lcId] ?? activeIndex())
          );
        }
        const source = activeIndexByLc[lcId];
        const supplied = typeof source === "function" ? await source() : (source ?? activeIndex());
        return supplied instanceof Response
          ? supplied
          : jsonResponse(supplied);
      }
      calls.push({ method: init?.method ?? "GET", url });
      return fetchMock(input, init);
    }),
  );
  return calls;
}

function rebuildPostIndex(calls: FetchCall[], lcId: string): number {
  return calls.findIndex(
    (call) =>
      call.method === "POST" &&
      call.url ===
        `/api/projects/project_0001/logical-codebases/${lcId}/aggregate-indexes/rebuild`,
  );
}

function activeReadbackCount(
  calls: FetchCall[],
  lcId: string,
  afterIndex: number,
): number {
  return calls
    .slice(afterIndex + 1)
    .filter(
      (call) =>
        call.method === "GET" &&
        call.url ===
          `/api/projects/project_0001/logical-codebases/${lcId}/aggregate-indexes/active`,
    ).length;
}

describe("IssueLifecycleWorkbench 聚合索引重建失败回读与作用域防串写", () => {
  installIssueLifecycleWorkbenchTestHooks();
  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it("重建失败保留 POST 错误并补读一次 active 刷新卡片（消除残留旧告警），不重试", async () => {
    // 客户实测症状：POST 失败后 banner 已更新，卡片仍残留旧告警。
    const refreshed = activeIndex({
      state: "degraded",
      revision: 3,
      warning: "新告警：本次扫描失败原因",
    });
    let currentActive: AggregateIndexActiveResponse | Response = activeIndex();
    const calls = installAggregateRebuildFetch({
      activeIndexByLc: {
        // GET active 按调用时点返回：作用域初载旧快照，失败补读拿新判定。
        lc_0001: () => currentActive,
      },
      onRebuild: () => {
        currentActive = refreshed;
        return errorResponse(422, "rebuild rejected");
      },
    });
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);
    await user.click(await screen.findByTestId("lc-summary-toggle"));
    expect(await screen.findByTestId("aggregate-index-status")).toHaveTextContent(
      "stale",
    );
    expect(screen.getByTestId("aggregate-index-warning")).toHaveTextContent(
      "旧告警：历史扫描诊断",
    );

    await user.click(screen.getByRole("button", { name: "重建索引" }));

    expect(await screen.findByText("rebuild rejected")).toBeInTheDocument();
    await waitFor(() =>
      expect(screen.getByTestId("aggregate-index-status")).toHaveTextContent(
        "degraded",
      ),
    );
    expect(screen.getByTestId("aggregate-index-warning")).toHaveTextContent(
      "新告警：本次扫描失败原因",
    );
    expect(screen.queryByText(/旧告警/)).not.toBeInTheDocument();

    const postIndex = rebuildPostIndex(calls, "lc_0001");
    expect(postIndex).toBeGreaterThanOrEqual(0);
    // 恰好补读一次 GET active，失败后不重试、不轮询、不再 POST。
    expect(activeReadbackCount(calls, "lc_0001", postIndex)).toBe(1);
    expect(
      calls.slice(postIndex + 1).filter((call) => call.method === "POST").length,
    ).toBe(0);
  });

  it("补读失败：保留原 POST 错误与旧卡片，不再重试", async () => {
    let activeGetCount = 0;
    const calls = installAggregateRebuildFetch({
      activeIndexByLc: {
        // 作用域初载成功；失败后的补读 GET 返回 500。
        lc_0001: () => {
          activeGetCount += 1;
          return activeGetCount > 1
            ? errorResponse(500, "active read failed")
            : activeIndex();
        },
      },
      onRebuild: () => errorResponse(500, "sync failed"),
    });
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);
    await user.click(await screen.findByTestId("lc-summary-toggle"));
    expect(await screen.findByTestId("aggregate-index-status")).toHaveTextContent(
      "stale",
    );

    await user.click(screen.getByRole("button", { name: "重建索引" }));

    expect(await screen.findByText("sync failed")).toBeInTheDocument();
    // 补读失败不覆盖原 POST 错误。
    expect(screen.queryByText("active read failed")).not.toBeInTheDocument();
    // 旧卡片保留：仍是 stale + 旧告警。
    await waitFor(() =>
      expect(screen.getByTestId("aggregate-index-status")).toHaveTextContent(
        "stale",
      ),
    );
    expect(screen.getByTestId("aggregate-index-warning")).toHaveTextContent(
      "旧告警：历史扫描诊断",
    );

    const postIndex = rebuildPostIndex(calls, "lc_0001");
    // 恰好补读一次，失败即止（无循环）。
    expect(activeReadbackCount(calls, "lc_0001", postIndex)).toBe(1);
  });

  it("重建成功直接采用 POST 响应，不额外发 GET active", async () => {
    const calls = installAggregateRebuildFetch({
      onRebuild: () =>
        jsonResponse(
          activeIndex({
            state: "active",
            revision: 8,
            indexed_at: "2026-11-19T00:00:00Z",
            warning: null,
          }),
        ),
    });
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);
    await user.click(await screen.findByTestId("lc-summary-toggle"));
    expect(await screen.findByTestId("aggregate-index-status")).toHaveTextContent(
      "stale",
    );

    await user.click(screen.getByRole("button", { name: "重建索引" }));

    await waitFor(() =>
      expect(screen.getByTestId("aggregate-index-status")).toHaveTextContent(
        "active",
      ),
    );
    expect(screen.getByText(/成员版本：8/)).toBeInTheDocument();
    const postIndex = rebuildPostIndex(calls, "lc_0001");
    expect(activeReadbackCount(calls, "lc_0001", postIndex)).toBe(0);
  });

  it("在途重建跨 LC 切换：busy 即刻复位，迟到的成功响应不回写新 LC；回到 A 显示作用域数据而非旧响应", async () => {
    const rebuildResponse = deferred<Response>();
    const calls = installAggregateRebuildFetch({
      logicalCodebases: [
        { id: "lc_0001", name: "platform" },
        { id: "lc_0002", name: "web" },
      ],
      onRebuild: () => rebuildResponse.promise,
    });
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);
    await user.click(await screen.findByTestId("lc-summary-toggle"));
    expect(await screen.findByTestId("aggregate-index-status")).toHaveTextContent(
      "stale",
    );

    await user.click(screen.getByRole("button", { name: "重建索引" }));
    expect(screen.getByTestId("aggregate-index-spinner")).toBeInTheDocument();

    // A→B：切换即作废在途请求，busy 不得黏在 B 的卡片上。
    await user.click(screen.getByTestId("lc-selector-web"));
    await waitFor(() =>
      expect(
        screen.queryByTestId("aggregate-index-spinner"),
      ).not.toBeInTheDocument(),
    );
    await waitFor(() =>
      expect(screen.getByText(/成员版本：5/)).toBeInTheDocument(),
    );

    // 迟到的 A 成功响应（revision 9）不得覆盖 B 的卡片。
    const postIndex = rebuildPostIndex(calls, "lc_0001");
    await act(async () => {
      rebuildResponse.resolve(
        jsonResponse(
          activeIndex({ state: "active", revision: 9, warning: null }),
        ),
      );
    });
    expect(screen.getByText(/成员版本：5/)).toBeInTheDocument();
    expect(screen.queryByText(/成员版本：9/)).not.toBeInTheDocument();
    expect(
      screen.queryByTestId("aggregate-index-spinner"),
    ).not.toBeInTheDocument();

    // A→B→A：回到 A 后显示作用域重拉的 revision 3，而不是迟到的 revision 9。
    await user.click(screen.getByTestId("lc-selector-platform"));
    await waitFor(() =>
      expect(screen.getByText(/成员版本：3/)).toBeInTheDocument(),
    );
    expect(screen.queryByText(/成员版本：9/)).not.toBeInTheDocument();
  });

  it("上一 LC 的初始 GET 迟到时不能覆盖当前重建失败后读回的卡片", async () => {
    const pendingA = deferred<Response>();
    let bReads = 0;
    const calls = installAggregateRebuildFetch({
      logicalCodebases: [{ id: "lc_0001", name: "platform" }, { id: "lc_0002", name: "web" }],
      activeIndexByLc: {
        lc_0001: () => pendingA.promise,
        lc_0002: () => activeIndex({ revision: ++bReads === 1 ? 5 : 8 }),
      },
      onRebuild: () => errorResponse(422, "本次未通过"),
    });
    const user = userEvent.setup();
    render(<IssueLifecycleWorkbench />);
    await user.click(await screen.findByTestId("lc-summary-toggle"));
    await waitFor(() => expect(calls.some(call => call.url.endsWith("lc_0001/aggregate-indexes/active"))).toBe(true));
    await user.click(screen.getByTestId("lc-selector-web"));
    await screen.findByText(/成员版本：5/);
    await user.click(screen.getByRole("button", { name: "重建索引" }));
    await screen.findByText(/成员版本：8/);
    await act(async () => { pendingA.resolve(jsonResponse(activeIndex({ revision: 99 }))); });
    expect(screen.getByText(/成员版本：8/)).toBeInTheDocument();
    expect(screen.queryByText(/成员版本：99/)).not.toBeInTheDocument();
  });

  it("在途重建跨 LC 切换后失败：不向新 LC 写错误，也不对旧 LC 补读 active", async () => {
    const rebuildResponse = deferred<Response>();
    const calls = installAggregateRebuildFetch({
      logicalCodebases: [
        { id: "lc_0001", name: "platform" },
        { id: "lc_0002", name: "web" },
      ],
      onRebuild: () => rebuildResponse.promise,
    });
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);
    await user.click(await screen.findByTestId("lc-summary-toggle"));
    expect(await screen.findByTestId("aggregate-index-status")).toHaveTextContent(
      "stale",
    );

    await user.click(screen.getByRole("button", { name: "重建索引" }));
    await user.click(screen.getByTestId("lc-selector-web"));
    await waitFor(() =>
      expect(
        screen.queryByTestId("aggregate-index-spinner"),
      ).not.toBeInTheDocument(),
    );

    const postIndex = rebuildPostIndex(calls, "lc_0001");
    await act(async () => {
      rebuildResponse.resolve(errorResponse(422, "late rebuild failure"));
    });

    // 旧请求的错误与补读都不得落到新作用域。
    expect(screen.queryByText("late rebuild failure")).not.toBeInTheDocument();
    expect(activeReadbackCount(calls, "lc_0001", postIndex)).toBe(0);
    expect(screen.getByText(/成员版本：5/)).toBeInTheDocument();
    expect(
      screen.queryByTestId("aggregate-index-spinner"),
    ).not.toBeInTheDocument();
  });
});
