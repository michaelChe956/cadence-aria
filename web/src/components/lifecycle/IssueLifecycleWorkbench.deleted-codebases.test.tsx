// 2026-10-29（owner）：已软删（墓碑）LC 的「已删除」分区——渲染 deleted_at、
// 恢复（POST restore）与彻底删除（DELETE purge，不可恢复 + 二次确认）。
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { IssueLifecycleWorkbench } from "./IssueLifecycleWorkbench";
import {
  installIssueLifecycleWorkbenchTestHooks,
  lifecycleFetch,
  projectRecord,
} from "./IssueLifecycleWorkbench.test-utils";

vi.mock("../shared/MonacoViewer", () => ({
  MonacoViewer: ({ value }: { value: string }) => (
    <div data-testid="monaco-viewer">{value}</div>
  ),
}));

type FetchCall = { method: string; url: string };

function installFetch(
  options: Parameters<typeof lifecycleFetch>[0],
): FetchCall[] {
  const calls: FetchCall[] = [];
  const fetchMock = lifecycleFetch(options);
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      calls.push({ method: init?.method ?? "GET", url: String(input) });
      return fetchMock(input, init);
    }),
  );
  return calls;
}

function restorableFixture() {
  return {
    projects: [projectRecord("project_0001", "Aria")],
    logicalCodebases: [{ id: "lc_alive", name: "alive", member_count: 0 }],
    deletedLogicalCodebases: [
      { id: "lc_gone", name: "gone", deleted_at: "2026-10-28T08:00:00Z" },
    ],
  };
}

describe("IssueLifecycleWorkbench 已删除逻辑代码库分区（2026-10-29）", () => {
  installIssueLifecycleWorkbenchTestHooks();
  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it("已删除分区渲染墓碑 LC 与 deleted_at；存活条目没有恢复/彻底删除按钮", async () => {
    const calls = installFetch(restorableFixture());

    render(<IssueLifecycleWorkbench />);

    const section = await screen.findByTestId("deleted-logical-codebases");
    expect(within(section).getByText("已删除")).toBeInTheDocument();
    expect(within(section).getByText("gone")).toBeInTheDocument();
    expect(section).toHaveTextContent("2026-10-28T08:00:00Z");
    expect(
      within(section).getByRole("button", { name: "恢复逻辑代码库 gone" }),
    ).toBeInTheDocument();
    expect(
      within(section).getByRole("button", { name: "彻底删除逻辑代码库 gone" }),
    ).toBeInTheDocument();

    // 存活条目（不在已删除分区）绝不出现这两个按钮。
    expect(
      screen.queryByRole("button", { name: "恢复逻辑代码库 alive" }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "彻底删除逻辑代码库 alive" }),
    ).not.toBeInTheDocument();
    expect(
      calls.some((call) => call.url.includes("include_deleted=true")),
    ).toBe(true);
  });

  it("点恢复调用 POST restore，条目从已删除分区消失", async () => {
    const calls = installFetch(restorableFixture());
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);

    const section = await screen.findByTestId("deleted-logical-codebases");
    await user.click(
      within(section).getByRole("button", { name: "恢复逻辑代码库 gone" }),
    );

    await waitFor(() =>
      expect(
        calls.filter(
          (call) =>
            call.method === "POST" &&
            call.url ===
              "/api/projects/project_0001/logical-codebases/lc_gone/restore",
        ),
      ).toHaveLength(1),
    );
    await waitFor(() =>
      expect(
        screen.queryByTestId("deleted-logical-codebases"),
      ).not.toBeInTheDocument(),
    );
  });

  it("彻底删除需二次确认，文案写明不可恢复；取消则不发 purge 请求", async () => {
    const calls = installFetch(restorableFixture());
    const confirmSpy = vi.spyOn(window, "confirm").mockReturnValue(false);
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);

    const section = await screen.findByTestId("deleted-logical-codebases");
    await user.click(
      within(section).getByRole("button", {
        name: "彻底删除逻辑代码库 gone",
      }),
    );

    expect(confirmSpy).toHaveBeenCalledTimes(1);
    expect(confirmSpy).toHaveBeenCalledWith(
      expect.stringContaining("不可恢复"),
    );
    // 边界写清：清政策产物与成员指针，不动 AGENTS.md/CLAUDE.md。
    expect(confirmSpy).toHaveBeenCalledWith(
      expect.stringContaining("AGENTS.md"),
    );
    expect(confirmSpy).toHaveBeenCalledWith(
      expect.stringContaining("CLAUDE.md"),
    );
    expect(calls.some((call) => call.url.includes("/purge"))).toBe(false);
    expect(screen.getByTestId("deleted-logical-codebases")).toBeInTheDocument();

    // 二次确认同意后才真正硬删；成功后条目消失。
    confirmSpy.mockReturnValue(true);
    await user.click(
      within(screen.getByTestId("deleted-logical-codebases")).getByRole(
        "button",
        { name: "彻底删除逻辑代码库 gone" },
      ),
    );
    await waitFor(() =>
      expect(
        calls.filter(
          (call) =>
            call.method === "DELETE" &&
            call.url ===
              "/api/projects/project_0001/logical-codebases/lc_gone/purge",
        ),
      ).toHaveLength(1),
    );
    await waitFor(() =>
      expect(
        screen.queryByTestId("deleted-logical-codebases"),
      ).not.toBeInTheDocument(),
    );
  });
});
