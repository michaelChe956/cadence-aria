import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
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

describe("IssueLifecycleWorkbench 逻辑代码库按 LC 分区（R8）", () => {
  installIssueLifecycleWorkbenchTestHooks();
  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it("多 LC 并存：面板数据随选中 LC 切换（members/初始化卡片按选中 LC 分区）", async () => {
    const calls: string[] = [];
    const fetchMock = lifecycleFetch({
      projects: [projectRecord("project_0001", "Aria")],
      logicalCodebases: [
        { id: "lc_0001", name: "platform", member_count: 1 },
        { id: "lc_0002", name: "web", member_count: 0 },
      ],
      logicalCodebaseMembersByLc: {
        lc_0001: [member("lr-1", "api", "repository_1001")],
        lc_0002: [],
      },
    });
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = String(input);
        calls.push(`${init?.method ?? "GET"} ${url}`);
        return fetchMock(input, init);
      }),
    );
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);

    // 默认选中首个 LC：其成员驱动的初始化卡片可见
    expect(
      await screen.findByTestId("aggregate-initialization-card"),
    ).toBeInTheDocument();
    expect(
      calls.filter((call) =>
        call.includes("/logical-codebases/lc_0001/members"),
      ).length,
    ).toBeGreaterThan(0);

    // 切换到第二个 LC：members 请求走该 LC 路径，且无成员 → 初始化卡片消失
    await user.click(screen.getByTestId("lc-selector-web"));
    await waitFor(() =>
      expect(
        calls.filter((call) =>
          call.includes("/logical-codebases/lc_0002/members"),
        ).length,
      ).toBeGreaterThan(0),
    );
    await waitFor(() =>
      expect(
        screen.queryByTestId("aggregate-initialization-card"),
      ).not.toBeInTheDocument(),
    );
  });

  it("LC 切换重置 aggregateInitialization：A 启动初始化后切 B 不显示 A 的 operation", async () => {
    const calls: string[] = [];
    const fetchMock = lifecycleFetch({
      projects: [projectRecord("project_0001", "Aria")],
      logicalCodebases: [
        { id: "lc_0001", name: "platform", member_count: 1 },
        { id: "lc_0002", name: "web", member_count: 1 },
      ],
      logicalCodebaseMembersByLc: {
        lc_0001: [member("lr-1", "api", "repository_1001")],
        lc_0002: [member("lr-2", "web", "repository_1002")],
      },
      aggregateInitializationStart: aggregateInitializationOperation("created"),
    });
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = String(input);
        calls.push(`${init?.method ?? "GET"} ${url}`);
        return fetchMock(input, init);
      }),
    );
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);

    // LC A：启动初始化，卡片显示 A 的 operation
    await screen.findByTestId("aggregate-initialization-card");
    await user.click(
      screen.getByRole("button", { name: "启动聚合初始化" }),
    );
    expect(
      await screen.findByTestId("aggregate-initialization-status"),
    ).toHaveAttribute("data-status", "created");

    // 切到 LC B：卡片仍可见（B 有成员），但不应显示 A 的 operation
    await user.click(screen.getByTestId("lc-selector-web"));
    await waitFor(() =>
      expect(screen.getByTestId("lc-selector-web")).toHaveAttribute(
        "aria-selected",
        "true",
      ),
    );
    await waitFor(() =>
      expect(
        screen.queryByTestId("aggregate-initialization-status"),
      ).not.toBeInTheDocument(),
    );
    // 也不应对 B 轮询 A 的 operation_id
    expect(
      calls.some((call) =>
        call.includes("/logical-codebases/lc_0002/initializations/")),
    ).toBe(false);
  });

  it("登记成员对选中 LC 打开向导（不再固定取首个）", async () => {
    const calls: string[] = [];
    const fetchMock = lifecycleFetch({
      projects: [projectRecord("project_0001", "Aria")],
      logicalCodebases: [
        { id: "lc_0001", name: "platform", member_count: 0 },
        { id: "lc_0002", name: "web", member_count: 0 },
      ],
      registrationPreflight: {
        preflight_id: "preflight_0001",
        created_at: "2026-08-19T00:00:00Z",
        items: [{ path: "/root/web", class: "eligible", reason: null }],
      },
      registrationSubmit: {
        batch_id: "batch_0001",
        status: "completed",
        items: [
          { path: "/root/web", status: "completed", failure_reason: null },
        ],
      },
    });
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = String(input);
        calls.push(`${init?.method ?? "GET"} ${url}`);
        return fetchMock(input, init);
      }),
    );
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);

    await screen.findByTestId("lc-selector-web");
    await user.click(screen.getByTestId("lc-selector-web"));
    await user.click(screen.getByRole("button", { name: "登记成员" }));
    await user.type(
      await screen.findByLabelText("聚合根目录"),
      "/root/web",
    );
    await user.click(
      screen.getByRole("button", { name: "确认聚合根并自动发现" }),
    );
    expect(
      await screen.findByLabelText("选择 /root/web（eligible）"),
    ).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "提交登记" }));

    await waitFor(() =>
      expect(
        calls.filter((call) =>
          call.startsWith(
            "POST /api/projects/project_0001/logical-codebases/lc_0002/registrations",
          ),
        ).length,
      ).toBeGreaterThan(0),
    );
    expect(
      calls.filter((call) =>
        call.includes("/logical-codebases/lc_0001/registrations"),
      ).length,
    ).toBe(0);
  });

  it("逻辑条目删除：二次确认后走 DELETE /logical-codebases/{lc_id} 并刷新列表", async () => {
    const calls: string[] = [];
    const fetchMock = lifecycleFetch({
      projects: [projectRecord("project_0001", "Aria")],
      logicalCodebases: [
        { id: "lc_0001", name: "platform", member_count: 0 },
        { id: "lc_0002", name: "web", member_count: 0 },
      ],
    });
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = String(input);
        calls.push(`${init?.method ?? "GET"} ${url}`);
        return fetchMock(input, init);
      }),
    );
    const confirmSpy = vi.spyOn(window, "confirm");
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);

    await screen.findByTestId("codebase-kind-platform");
    confirmSpy.mockReturnValue(false);
    await user.click(
      screen.getByRole("button", { name: "删除逻辑代码库 platform" }),
    );
    expect(confirmSpy).toHaveBeenCalledTimes(1);
    expect(
      calls.some((call) => call.startsWith("DELETE /api/projects/project_0001/logical-codebases/lc_0001")),
    ).toBe(false);
    expect(await screen.findByTestId("codebase-kind-platform")).toBeInTheDocument();

    confirmSpy.mockReturnValue(true);
    await user.click(
      screen.getByRole("button", { name: "删除逻辑代码库 platform" }),
    );
    await waitFor(() =>
      expect(
        calls.some((call) =>
          call.startsWith(
            "DELETE /api/projects/project_0001/logical-codebases/lc_0001",
          ),
        ),
      ).toBe(true),
    );
    await waitFor(() =>
      expect(
        screen.queryByTestId("codebase-kind-platform"),
      ).not.toBeInTheDocument(),
    );
  });

  it("issue 创建：选逻辑代码库时提交 logical_codebase_id + primary repository_id", async () => {
    const issueBodies: string[] = [];
    const fetchMock = lifecycleFetch({
      projects: [projectRecord("project_0001", "Aria")],
      logicalCodebases: [
        { id: "lc_0001", name: "platform", member_count: 1 },
      ],
      logicalCodebaseMembersByLc: {
        lc_0001: [member("lr-1", "api", "repository_1001")],
      },
    });
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = String(input);
        if (
          url === "/api/projects/project_0001/issues" &&
          init?.method === "POST"
        ) {
          issueBodies.push(String(init.body));
        }
        return fetchMock(input, init);
      }),
    );
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);

    await user.click(await screen.findByRole("button", { name: "新建 Issue" }));
    const dialog = await screen.findByRole("dialog", { name: "新建 Issue" });
    await user.type(within(dialog).getByLabelText("Issue 标题"), "跨仓需求");
    await user.selectOptions(
      within(dialog).getByLabelText("代码库"),
      "lc:lc_0001",
    );
    await user.selectOptions(
      await within(dialog).findByLabelText("Primary 成员"),
      "repository_1001",
    );
    await user.click(
      within(dialog).getByRole("button", { name: "创建 Issue" }),
    );

    await waitFor(() => expect(issueBodies).toHaveLength(1));
    expect(JSON.parse(issueBodies[0])).toMatchObject({
      title: "跨仓需求",
      repository_id: "repository_1001",
      logical_codebase_id: "lc_0001",
    });
  });
});
