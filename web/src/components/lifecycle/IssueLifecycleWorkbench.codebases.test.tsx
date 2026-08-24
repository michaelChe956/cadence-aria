import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { IssueLifecycleWorkbench } from "./IssueLifecycleWorkbench";
import {
  installIssueLifecycleWorkbenchTestHooks,
  lifecycleFetch,
  projectRecord,
  repositoryRecord,
} from "./IssueLifecycleWorkbench.test-utils";

vi.mock("../shared/MonacoViewer", () => ({
  MonacoViewer: ({ value }: { value: string }) => (
    <div data-testid="monaco-viewer">{value}</div>
  ),
}));

describe("IssueLifecycleWorkbench codebases 混合列表与添加弹窗", () => {
  installIssueLifecycleWorkbenchTestHooks();
  afterEach(() => vi.unstubAllGlobals());

  it("混合列表展示单仓/逻辑徽标与成员数；登记按钮在无 LC 时禁用", async () => {
    vi.stubGlobal(
      "fetch",
      lifecycleFetch({
        projects: [projectRecord("project_0001", "Aria")],
        logicalCodebases: [{ id: "lc_0001", name: "monorepo", member_count: 3 }],
      }),
    );
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);

    const items = await screen.findAllByTestId("codebase-list-item");
    expect(items).toHaveLength(2);
    expect(screen.getByTestId("codebase-kind-Aria Repo")).toHaveTextContent("单仓");
    expect(screen.getByTestId("codebase-kind-monorepo")).toHaveTextContent("逻辑");
    expect(screen.getByText("成员：3")).toBeInTheDocument();
    // Task 8：「登记成员」位于运维面板内，默认折叠，先点展开。
    await user.click(screen.getByTestId("lc-summary-toggle"));
    expect(
      await screen.findByRole("button", { name: "登记成员" }),
    ).toBeEnabled();
  });

  it("无逻辑代码库时登记成员禁用；单仓库模式走既有添加对话框", async () => {
    vi.stubGlobal(
      "fetch",
      lifecycleFetch({
        // hotfix：仅单仓 codebase 时面板整体不渲染，无「登记成员」入口
        projects: [projectRecord("project_0001", "Aria")],
      }),
    );
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);

    expect(
      await screen.findByRole("button", { name: "添加代码库" }),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "登记成员" }),
    ).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "添加代码库" }));
    expect(
      screen.getByRole("dialog", { name: "添加代码库" }),
    ).toBeInTheDocument();
    expect(screen.getByRole("radio", { name: /单仓库/ })).toBeChecked();

    await user.click(screen.getByRole("button", { name: "继续添加单仓库" }));
    await waitFor(() =>
      expect(screen.getByRole("dialog", { name: "添加代码库" })).toBeInTheDocument(),
    );
  });

  it("多仓库模式：创建 LC 后进入该 LC 的登记向导（lc_id 新路径）", async () => {
    const calls: string[] = [];
    const fetchMock = lifecycleFetch({
      projects: [projectRecord("project_0001", "Aria")],
      logicalCodebases: [{ id: "lc_0001", name: "monorepo", member_count: 0 }],
      registrationPreflight: {
        preflight_id: "preflight_0001",
        created_at: "2026-08-19T00:00:00Z",
        items: [
          { path: "/root/api", class: "eligible", reason: null },
        ],
      },
      registrationSubmit: {
        batch_id: "batch_0001",
        status: "completed",
        items: [{ path: "/root/api", status: "completed", failure_reason: null }],
      },
    });
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = String(input);
        calls.push(`${init?.method ?? "GET"} ${url}`);
        if (
          url === "/api/projects/project_0001/logical-codebases" &&
          init?.method === "POST"
        ) {
          return new Response(
            JSON.stringify({
              id: "lc_0001",
              name: "monorepo",
              aggregate_root: "/root",
              created_at: "2026-08-19T00:00:00Z",
            }),
          );
        }
        return fetchMock(input, init);
      }),
    );
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);

    await user.click(await screen.findByRole("button", { name: "添加代码库" }));
    await user.click(screen.getByRole("radio", { name: /多仓库逻辑代码库/ }));
    await user.type(screen.getByLabelText("聚合根目录"), "/root");
    expect(screen.getByLabelText("名称")).toHaveValue("root");
    await user.click(screen.getByRole("button", { name: "创建逻辑代码库" }));

    // 创建成功 → 打开登记向导（自动发现模式）
    await user.type(
      await screen.findByLabelText("聚合根目录"),
      "-override",
    );
    await user.click(
      screen.getByRole("button", { name: "确认聚合根并自动发现" }),
    );
    expect(
      await screen.findByLabelText("选择 /root/api（eligible）"),
    ).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "提交登记" }));

    await waitFor(() =>
      expect(
        calls.filter((call) =>
          call.startsWith(
            "POST /api/projects/project_0001/logical-codebases/lc_0001/registrations",
          ),
        ).length,
      ).toBeGreaterThan(0),
    );
    // 旧别名不再被前端使用
    expect(
      calls.filter((call) => call.includes("/logical-codebase/registrations")),
    ).toEqual([]);
  });

  it("已有 LC 时登记成员直接打开向导（首个 LC）", async () => {
    vi.stubGlobal(
      "fetch",
      lifecycleFetch({
        projects: [projectRecord("project_0001", "Aria")],
        logicalCodebases: [{ id: "lc_0001", name: "monorepo" }],
      }),
    );
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);

    // Task 8：先展开运维面板（默认折叠为摘要条）才能点「登记成员」。
    await user.click(await screen.findByTestId("lc-summary-toggle"));
    await user.click(await screen.findByRole("button", { name: "登记成员" }));
    expect(
      await screen.findByRole("dialog", { name: "登记成员" }),
    ).toBeInTheDocument();
    expect(screen.getByLabelText("聚合根目录")).toBeInTheDocument();
    expect(repositoryRecord).toBeDefined();
  });
});
