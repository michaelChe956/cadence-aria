// 2026-11-05（owner 真机事故回归）：侧栏垃圾桶一击即硬删整个项目工作区
//（项目 + 全部逻辑代码库 + 成员登记 + capability 记录）且无任何确认。
// 修复形态对齐逻辑代码库删除/清除的二次确认：点击先弹确认——取消零副作用
//（零出站删除请求）；确认才发带确认意图的 DELETE ?confirmed=true（后端
// 409 deletion_confirmation_required 强制兜底，不靠前端自律）。
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { IssueLifecycleWorkbench } from "./IssueLifecycleWorkbench";
import {
  deferred,
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

function installFetch(): FetchCall[] {
  const calls: FetchCall[] = [];
  const fetchMock = lifecycleFetch({
    projects: [projectRecord("project_0001", "Aria")],
  });
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      calls.push({ method: init?.method ?? "GET", url: String(input) });
      return fetchMock(input, init);
    }),
  );
  return calls;
}

describe("IssueLifecycleWorkbench 删除 Project 二次确认（2026-11-05）", () => {
  installIssueLifecycleWorkbenchTestHooks();
  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it("取消确认零副作用：不发删除请求，Project 保留", async () => {
    const calls = installFetch();
    const confirmSpy = vi.spyOn(window, "confirm").mockReturnValue(false);
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);

    await user.click(
      await screen.findByRole("button", { name: "删除 Project Aria" }),
    );

    expect(confirmSpy).toHaveBeenCalledTimes(1);
    // 文案必须说清删除范围（整个项目及其全部逻辑代码库/成员登记）与不可恢复。
    const message = confirmSpy.mock.calls[0]?.[0] ?? "";
    expect(message).toContain("不可恢复");
    expect(message).toContain("逻辑代码库");
    expect(message).toContain("成员登记");
    // 取消 ⇒ 零出站删除请求。
    expect(calls.filter((call) => call.method === "DELETE")).toHaveLength(0);
    // Project 仍在侧栏。
    expect(screen.getByRole("button", { name: "Aria" })).toBeInTheDocument();
  });

  it("确认后发带确认意图的删除请求并刷新列表", async () => {
    const calls = installFetch();
    const confirmSpy = vi.spyOn(window, "confirm").mockReturnValue(true);
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);

    await user.click(
      await screen.findByRole("button", { name: "删除 Project Aria" }),
    );

    expect(confirmSpy).toHaveBeenCalledTimes(1);
    expect(calls).toContainEqual({
      method: "DELETE",
      url: "/api/projects/project_0001?confirmed=true",
    });
    expect(await screen.findByText("还没有 Project")).toBeInTheDocument();
  });
});

// 2026-11-19（复审 93c73b8 新增，请求次序竞争回归）：只测真实组件 + 受控
// HTTP 次序，底层用 lifecycleFetch 夹具、仅覆盖相关端点，不用计时器。
describe("IssueLifecycleWorkbench 创建/删除 Project 请求次序竞争（2026-11-19）", () => {
  installIssueLifecycleWorkbenchTestHooks();
  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it.each([true, false])("创建 Gamma 与切换 Beta 重叠时只对在途目录合并回执（创建先返回：%s）", async (createFirst) => {
    const pendingCreate = deferred<Response>();
    const betaDirectoryGet = deferred<Response>();
    const baseFetch = lifecycleFetch({
      projects: [
        projectRecord("project_0001", "Alpha"),
        projectRecord("project_0002", "Beta"),
      ],
      issueTitlesByProject: {
        project_0001: "Alpha issue",
        project_0002: "Beta issue",
        project_0003: "Gamma issue",
      },
    });
    // 仅覆盖创建 POST 与 Beta 显式切换发出的目录 GET（第 2 次列表读取：
    // 第 1 次是挂载读取），其余请求全部透传基础夹具。
    let projectListGets = 0;
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const path = String(input);
        if (path === "/api/projects" && init?.method === "POST") {
          return pendingCreate.promise;
        }
        if (path === "/api/projects") {
          projectListGets += 1;
          if (projectListGets === 2) return betaDirectoryGet.promise;
        }
        return baseFetch(input, init);
      }),
    );
    const user = userEvent.setup();
    render(<IssueLifecycleWorkbench />);
    const detail = screen.getByRole("region", { name: "Issue 生命周期详情" });
    await within(detail).findByRole("heading", { name: "Alpha issue" });

    await user.click(screen.getByRole("button", { name: "新建 Project" }));
    const dialog = screen.getByRole("dialog", { name: "新建 Project" });
    await user.type(within(dialog).getByLabelText("Project 名称"), "Gamma");
    await user.click(
      within(dialog).getByRole("button", { name: "创建 Project" }),
    );
    // 创建在途时用弹窗自身支持的取消关闭（输入有意重叠，不靠无关延迟）。
    await user.click(within(dialog).getByRole("button", { name: "取消" }));

    const beta = screen.getByRole("button", { name: "Beta" });
    await waitFor(() => expect(beta).toBeEnabled());
    await user.click(beta);
    const directoryBeforeCreate = await baseFetch("/api/projects");

    if (!createFirst) {
      await act(async () => betaDirectoryGet.resolve(directoryBeforeCreate));
      await within(detail).findByRole("heading", { name: "Beta issue" });
    }
    // 创建 Gamma 先于 Beta 的目录 GET 完成，再放行 Beta 的目录读取。
    await act(async () => {
      pendingCreate.resolve(
        await baseFetch("/api/projects", {
          method: "POST",
          body: JSON.stringify({ name: "Gamma", description: null }),
        }),
      );
    });
    if (createFirst) {
      await act(async () => betaDirectoryGet.resolve(directoryBeforeCreate));
    }

    expect(screen.getByRole("button", { name: "Beta" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    expect(
      await within(detail).findByRole("heading", { name: "Beta issue" }),
    ).toBeVisible();
    // Gamma 必须可发现（侧栏可见），但不得被强制选中。
    const gamma = screen.getByRole("button", { name: "Gamma" });
    expect(gamma).toBeInTheDocument();
    expect(gamma).toHaveAttribute("aria-pressed", "false");
    if (!createFirst) {
      // A later read must reflect a deletion from another tab, not the old create receipt.
      await baseFetch("/api/projects/project_0003?confirmed=true", { method: "DELETE" });
      await user.click(screen.getByRole("button", { name: "刷新" }));
      await waitFor(() => expect(screen.getByRole("button", { name: "Beta" })).toBeEnabled());
      expect(screen.queryByRole("button", { name: "Gamma" })).not.toBeInTheDocument();
    }
  });

  it("删除当前 Aria 成功但紧随目录读取失败：旧行与旧 Issue 内容消失且保留读取错误", async () => {
    const baseFetch = lifecycleFetch({
      projects: [projectRecord("project_0001", "Aria")],
    });
    let ariaDeleted = false;
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const path = String(input);
        if (
          path === "/api/projects/project_0001?confirmed=true" &&
          init?.method === "DELETE"
        ) {
          const response = await baseFetch(input, init);
          ariaDeleted = response.ok;
          return response;
        }
        // DELETE 成功后的目录 GET 失败（服务端 503），其余端点透传夹具。
        if (path === "/api/projects" && init?.method !== "POST" && ariaDeleted) {
          return new Response(JSON.stringify({ message: "项目目录读取失败" }), {
            status: 503,
            headers: { "content-type": "application/json" },
          });
        }
        return baseFetch(input, init);
      }),
    );
    vi.spyOn(window, "confirm").mockReturnValue(true);
    const user = userEvent.setup();
    render(<IssueLifecycleWorkbench />);
    const detail = screen.getByRole("region", { name: "Issue 生命周期详情" });
    await within(detail).findByRole("heading", { name: "登录会话过期" });

    const deleteButton = screen.getByRole("button", { name: "删除 Project Aria" });
    await waitFor(() => expect(deleteButton).toBeEnabled());
    await user.click(deleteButton);

    // 删除成功但目录读取失败：列表读取错误必须可见。
    await waitFor(() =>
      expect(screen.getByRole("alert")).toHaveTextContent("项目目录读取失败"),
    );
    // 已删除 Project 的旧行与旧 Issue 内容不得因读取失败而冻结在界面上。
    expect(
      screen.queryByRole("button", { name: "Aria" }),
    ).not.toBeInTheDocument();
    expect(
      within(detail).queryByRole("heading", { name: "登录会话过期" }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "选择 Issue 登录会话过期" }),
    ).not.toBeInTheDocument();
    // 清理之后列表读取错误仍然展示（不被吞掉）。
    expect(screen.getByRole("alert")).toHaveTextContent("项目目录读取失败");
  });

  it("删除回退后已选中的 Alpha 时重新选择存活项目，不等旧下游读取", async () => {
    const pendingDelete = deferred<Response>();
    const fallbackIssues = deferred<Response>();
    const fallbackStarted = deferred<void>();
    const deletePath = "/api/projects/project_0001?confirmed=true";
    const baseFetch = lifecycleFetch({
      projects: [projectRecord("project_0001", "Alpha"), projectRecord("project_0002", "Beta"), projectRecord("project_0003", "Gamma")],
    });
    let holdAlphaIssues = false;
    vi.stubGlobal("fetch", vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const path = String(input);
      if (path === deletePath && init?.method === "DELETE") return pendingDelete.promise;
      if (holdAlphaIssues && path === "/api/projects/project_0001/issues") {
        fallbackStarted.resolve();
        return fallbackIssues.promise;
      }
      return baseFetch(input, init);
    }));
    vi.spyOn(window, "confirm").mockReturnValue(true);
    const user = userEvent.setup();
    render(<IssueLifecycleWorkbench />);
    const deleteAlpha = await screen.findByRole("button", { name: "删除 Project Alpha" });
    await waitFor(() => expect(deleteAlpha).toBeEnabled());
    await user.click(deleteAlpha);

    // Beta disappears before its switch request. The directory commits Alpha as fallback.
    await baseFetch("/api/projects/project_0002?confirmed=true", { method: "DELETE" });
    holdAlphaIssues = true;
    await user.click(screen.getByRole("button", { name: "Beta" }));
    await fallbackStarted.promise;
    expect(screen.getByRole("button", { name: "Alpha" })).toHaveAttribute("aria-pressed", "true");

    await act(async () => pendingDelete.resolve(await baseFetch(deletePath, { method: "DELETE" })));
    expect(screen.getByRole("button", { name: "Gamma" })).toHaveAttribute("aria-pressed", "true");
    await act(async () => fallbackIssues.resolve(await baseFetch("/api/projects/project_0001/issues")));
    expect(screen.getByRole("button", { name: "Gamma" })).toHaveAttribute("aria-pressed", "true");
  });

  it("旧项目的删除回执不会使已选中的 Beta 加载失效", async () => {
    const pendingDelete = deferred<Response>();
    const pendingBetaIssues = deferred<Response>();
    const betaIssuesStarted = deferred<void>();
    const deletePath = "/api/projects/project_0001?confirmed=true";
    const betaIssuesPath = "/api/projects/project_0002/issues";
    const baseFetch = lifecycleFetch({
      projects: [projectRecord("project_0001", "Alpha"), projectRecord("project_0002", "Beta")],
      issueTitlesByProject: { project_0002: "Beta issue" },
    });
    let alphaDeleted = false;
    vi.stubGlobal("fetch", vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const path = String(input);
      if (path === deletePath && init?.method === "DELETE") return pendingDelete.promise;
      if (path === betaIssuesPath) {
        betaIssuesStarted.resolve();
        return pendingBetaIssues.promise;
      }
      if (path === "/api/projects" && alphaDeleted) return new Response("{}", { status: 503 });
      return baseFetch(input, init);
    }));
    vi.spyOn(window, "confirm").mockReturnValue(true);
    const user = userEvent.setup();
    render(<IssueLifecycleWorkbench />);
    const deleteAlpha = await screen.findByRole("button", { name: "删除 Project Alpha" });
    await waitFor(() => expect(deleteAlpha).toBeEnabled());
    await user.click(deleteAlpha);
    await user.click(screen.getByRole("button", { name: "Beta" }));
    await betaIssuesStarted.promise;
    expect(screen.getByRole("button", { name: "Beta" })).toHaveAttribute("aria-pressed", "true");

    await act(async () => {
      const receipt = await baseFetch(deletePath, { method: "DELETE" });
      alphaDeleted = receipt.ok;
      pendingDelete.resolve(receipt);
    });
    await act(async () => pendingBetaIssues.resolve(await baseFetch(betaIssuesPath)));
    expect(await screen.findByRole("button", { name: "选择 Issue Beta issue" })).toBeEnabled();
    expect(screen.queryByRole("button", { name: "Alpha" })).not.toBeInTheDocument();
  });
});
