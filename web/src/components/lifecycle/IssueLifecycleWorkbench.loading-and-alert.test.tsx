import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { IssueLifecycleWorkbench } from "./IssueLifecycleWorkbench";
import {
  deferred,
  installIssueLifecycleWorkbenchTestHooks,
  lifecycleFetch,
} from "./IssueLifecycleWorkbench.test-utils";

// 回归（PR #92 预合并评审两条 Important）：
// 1) 刷新 A 已置位队列加载态并进入 issue 列表读取后，刷新 B 启动且其项目
//    目录读取失败：B 从未进入列表读取分支，A 的结算又被「不再是最新刷新」
//    拦下，issueListReadPending 永久卡死——队列停在「正在加载议题…」且
//    aria-busy 挂真，实际已无任何请求在途。
// 2) 页面级错误（如项目元数据读取失败）与深链目标自身 lifecycle 读取失败
//    同时存在时，入口 alert 位只显示前者，深链目标静默打不开——违反
//    「深链目标读取失败有提示」。
describe("IssueLifecycleWorkbench 队列加载态归属与告警合流", () => {
  installIssueLifecycleWorkbenchTestHooks();

  function failedResponse(message: string) {
    return new Response(JSON.stringify({ message }), {
      status: 500,
      headers: { "Content-Type": "application/json" },
    });
  }

  function queueRegion() {
    return screen.getByRole("region", { name: "Issue 卡片列表" });
  }

  it("更新一轮刷新在进入列表读取前失败时，旧一轮结算仍解除队列加载态", async () => {
    const issues = deferred<Response>();
    const baseFetch = lifecycleFetch();
    let projectDirectoryCalls = 0;
    const fetchMock = vi.fn(
      (input: RequestInfo | URL, init?: RequestInit) => {
        const url = String(input);
        if (url === "/api/projects") {
          projectDirectoryCalls += 1;
          // 刷新 B 的项目目录读取失败（B 从此不会进入列表读取分支）。
          if (projectDirectoryCalls > 1) {
            return Promise.resolve(failedResponse("项目目录读取失败"));
          }
        }
        if (url === "/api/projects/project_0001/issues") {
          return issues.promise;
        }
        return baseFetch(input, init);
      },
    );
    vi.stubGlobal("fetch", fetchMock);
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);

    // 刷新 A 的列表读取已在途（加载态已由 A 置位）。
    await waitFor(() => {
      expect(
        fetchMock.mock.calls.some(
          ([input]) => String(input) === "/api/projects/project_0001/issues",
        ),
      ).toBe(true);
    });
    expect(await within(queueRegion()).findByText("正在加载议题…")).toBeVisible();

    // 刷新 B：项目目录读取失败，B 报页面级错误。
    await user.click(screen.getByRole("button", { name: "刷新" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "项目目录读取失败",
    );

    // A 的列表读取随后结算：数据因被 B 取代而丢弃，但加载态必须解除——
    // 队列显示空态文案，不得停留在无请求在途的「正在加载议题…」。
    await act(async () => {
      issues.resolve(await baseFetch("/api/projects/project_0001/issues"));
    });
    const queue = queueRegion();
    expect(await within(queue).findByText("没有匹配的 Issue。")).toBeVisible();
    expect(within(queue).queryByText("正在加载议题…")).toBeNull();
    expect(queue).not.toHaveAttribute("aria-busy");
  });

  it("页面级错误与深链目标读取失败并存时两条提示都可见", async () => {
    const repositories = deferred<Response>();
    const baseFetch = lifecycleFetch({ sharedLifecycleIdsAcrossIssues: true });
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = String(input);
        if (url === "/api/projects/project_0001/repositories") {
          return repositories.promise;
        }
        if (
          url === "/api/issues/issue_0002/lifecycle?project_id=project_0001"
        ) {
          return failedResponse("生命周期服务暂不可用");
        }
        return baseFetch(input, init);
      }),
    );

    render(
      <IssueLifecycleWorkbench focusEntityKey="work_item:issue_0002:work_item_0001" />,
    );

    // 页面级错误（项目元数据读取失败）落位。
    await act(async () => {
      repositories.resolve(failedResponse("仓库目录读取失败"));
    });
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "仓库目录读取失败",
    );

    // 深链目标（issue_0002）自身 lifecycle 读取失败：深链失败提示不被页面级
    // 错误吞掉，两条都可见；页面级错误保持在前的措辞顺序。
    await waitFor(() => {
      expect(screen.getByRole("alert")).toHaveTextContent(
        "生命周期加载失败，可刷新重试。",
      );
    });
    const alert = screen.getByRole("alert");
    expect(alert).toHaveTextContent("work_item:issue_0002:work_item_0001");
    expect(alert).toHaveTextContent("生命周期服务暂不可用");
    const text = alert.textContent ?? "";
    expect(text.indexOf("仓库目录读取失败")).toBeLessThan(
      text.indexOf("生命周期加载失败"),
    );
  });
});
