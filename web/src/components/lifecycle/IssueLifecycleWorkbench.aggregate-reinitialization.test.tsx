import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { AggregateInitializationOperationSnapshot } from "../../api/types";
import { IssueLifecycleWorkbench } from "./IssueLifecycleWorkbench";
import {
  aggregateInitializationOperation,
  installIssueLifecycleWorkbenchTestHooks,
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

type FetchCall = { method: string; url: string; body: string | null };

function installFetch(options: {
  /** active=null → 该 LC 尚未初始化（后端 404 → 前端 operation=null）。 */
  active?: AggregateInitializationOperationSnapshot | null;
  reinitialize?: AggregateInitializationOperationSnapshot;
}) {
  const calls: FetchCall[] = [];
  const fetchMock = lifecycleFetch({
    projects: [projectRecord("project_0001", "Aria")],
    logicalCodebases: [{ id: "lc_0001", name: "platform", member_count: 1 }],
    logicalCodebaseMembersByLc: {
      lc_0001: [member("lr-1", "api", "repository_1001")],
    },
    aggregateInitializationActive:
      options.active === null
        ? undefined
        : (options.active ?? aggregateInitializationOperation("completed")),
    aggregateInitializationReinitialize:
      options.reinitialize ?? aggregateInitializationOperation("created"),
  });
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = String(input);
      calls.push({
        method: init?.method ?? "GET",
        url,
        body: init?.body ? String(init.body) : null,
      });
      return fetchMock(input, init);
    }),
  );
  return calls;
}

describe("IssueLifecycleWorkbench 聚合重新初始化（重新跑完整五步）", () => {
  installIssueLifecycleWorkbenchTestHooks();
  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it("完成态提供「重新初始化」入口，确认后打 /reinitializations（带 idempotency_key）", async () => {
    const calls = installFetch({});
    const confirmSpy = vi.spyOn(window, "confirm").mockReturnValue(true);
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);
    await user.click(await screen.findByTestId("lc-summary-toggle"));

    // 完成态：无「启动聚合初始化」，只有「重新初始化」。
    expect(
      await screen.findByTestId("aggregate-initialization-status"),
    ).toHaveAttribute("data-status", "completed");
    expect(
      screen.queryByRole("button", { name: "启动聚合初始化" }),
    ).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "重新初始化" }));

    // 二次确认文案写明：完整五步 / Cadence-skills / 版本。
    expect(confirmSpy).toHaveBeenCalledTimes(1);
    const confirmText = String(confirmSpy.mock.calls[0]?.[0] ?? "");
    expect(confirmText).toMatch(/完整五步/);
    expect(confirmText).toMatch(/Cadence-skills/);
    expect(confirmText).toMatch(/版本/);

    await waitFor(() =>
      expect(
        calls.some((call) => call.method === "POST" && call.url.endsWith("/reinitializations")),
      ).toBe(true),
    );

    const post = calls.find(
      (call) => call.method === "POST" && call.url.endsWith("/reinitializations"),
    )!;
    expect(post.url).toBe(
      "/api/projects/project_0001/logical-codebases/lc_0001/reinitializations",
    );
    // 不得误打初始化入口（/initializations）。
    expect(
      calls.some(
        (call) =>
          call.method === "POST" &&
          call.url.endsWith("/logical-codebases/lc_0001/initializations"),
      ),
    ).toBe(false);
    // 请求体与初始化入口同构：idempotency_key。
    const body = JSON.parse(post.body ?? "{}");
    expect(typeof body.idempotency_key).toBe("string");
    expect(body.idempotency_key.length).toBeGreaterThan(0);
  });

  it("取消确认不发任何重新初始化请求", async () => {
    const calls = installFetch({});
    const confirmSpy = vi.spyOn(window, "confirm").mockReturnValue(false);
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);
    await user.click(await screen.findByTestId("lc-summary-toggle"));
    await user.click(await screen.findByRole("button", { name: "重新初始化" }));

    expect(confirmSpy).toHaveBeenCalledTimes(1);
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "重新初始化" })).toBeEnabled(),
    );
    expect(
      calls.some((call) => call.method === "POST" && call.url.includes("reinitializations")),
    ).toBe(false);
    expect(
      calls.some(
        (call) =>
          call.method === "POST" &&
          call.url.endsWith("/logical-codebases/lc_0001/initializations"),
      ),
    ).toBe(false);
  });

  it("未初始化（无 operation）时只提供「启动聚合初始化」，不提供「重新初始化」", async () => {
    installFetch({ active: null });
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);
    await user.click(await screen.findByTestId("lc-summary-toggle"));

    expect(
      await screen.findByRole("button", { name: "启动聚合初始化" }),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "重新初始化" }),
    ).not.toBeInTheDocument();
  });

  it("进行中（running）时只提供「取消初始化」，不提供「重新初始化」", async () => {
    installFetch({ active: aggregateInitializationOperation("running") });
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);
    await user.click(await screen.findByTestId("lc-summary-toggle"));

    expect(
      await screen.findByRole("button", { name: "取消初始化" }),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "重新初始化" }),
    ).not.toBeInTheDocument();
  });
});
