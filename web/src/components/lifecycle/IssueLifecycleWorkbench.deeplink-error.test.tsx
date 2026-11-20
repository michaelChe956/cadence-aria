import { act, render, screen, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { IssueLifecycleWorkbench } from "./IssueLifecycleWorkbench";
import {
  deferred,
  installIssueLifecycleWorkbenchTestHooks,
  lifecycleFetch,
} from "./IssueLifecycleWorkbench.test-utils";

// 回归（浏览器取证复现）：
// 1) /workbench?focus=work_item:issue_0002:work_item_0001 只让 issue_0002 的
//    lifecycle 读取失败时，页面毫无告警、抽屉也开不出来——URL 指向一个从未
//    打开的目标，入口必须可见地报错（复用 detailErrors + 页面 alert 位）。
// 2) 项目元数据读取先失败而 issue 列表仍在途时，队列谎报「没有匹配的
//    Issue。」——应显示加载态，列表到达后再显示行。
describe("IssueLifecycleWorkbench 深链失败可见化与队列加载态", () => {
  installIssueLifecycleWorkbenchTestHooks();

  function queueRegion() {
    return screen.getByRole("region", { name: "Issue 卡片列表" });
  }

  it("深链目标自身 lifecycle 读取失败时在入口显示可见告警", async () => {
    const baseFetch = lifecycleFetch({ sharedLifecycleIdsAcrossIssues: true });
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        if (
          String(input) ===
          "/api/issues/issue_0002/lifecycle?project_id=project_0001"
        ) {
          return new Response(
            JSON.stringify({ message: "生命周期服务暂不可用" }),
            {
              status: 500,
              headers: { "Content-Type": "application/json" },
            },
          );
        }
        return baseFetch(input, init);
      }),
    );

    render(
      <IssueLifecycleWorkbench focusEntityKey="work_item:issue_0002:work_item_0001" />,
    );

    // 链接目标（issue_0002）读取失败：入口 alert 位必须可见地说明链接目标
    // 没能加载，且带上目标实体键与底层错误。
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("生命周期加载失败，可刷新重试。");
    expect(alert).toHaveTextContent("work_item:issue_0002:work_item_0001");
    expect(alert).toHaveTextContent("生命周期服务暂不可用");
    // 抽屉本身仍开不出来（目标数据不存在），不得伪装成已打开。
    expect(screen.queryByTestId("lifecycle-card-drawer")).toBeNull();
    // 页面其余部分保持正常：首个 Issue 的详情照常渲染。
    const detail = screen.getByRole("region", { name: "Issue 生命周期详情" });
    expect(within(detail).queryByText("生命周期加载失败，可刷新重试。")).toBeNull();
  });

  it("深链目标读取成功时打开抽屉且不出现入口告警", async () => {
    vi.stubGlobal(
      "fetch",
      lifecycleFetch({ sharedLifecycleIdsAcrossIssues: true }),
    );

    render(
      <IssueLifecycleWorkbench focusEntityKey="work_item:issue_0002:work_item_0001" />,
    );

    expect(await screen.findByTestId("lifecycle-card-drawer")).toBeVisible();
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("项目元数据先失败时队列显示加载中，列表到达后再显示行", async () => {
    const repositories = deferred<Response>();
    const issues = deferred<Response>();
    const baseFetch = lifecycleFetch();
    vi.stubGlobal(
      "fetch",
      vi.fn((input: RequestInfo | URL, init?: RequestInit) => {
        const url = String(input);
        if (url === "/api/projects/project_0001/repositories") {
          return repositories.promise;
        }
        if (url === "/api/projects/project_0001/issues") {
          return issues.promise;
        }
        return baseFetch(input, init);
      }),
    );

    render(<IssueLifecycleWorkbench />);

    const queue = queueRegion();
    // 初始加载期间先看到加载文案（既有行为）。
    expect(await within(queue).findByText("正在加载议题…")).toBeVisible();

    // 元数据读取失败（busy 复位、页面 alert 报元数据错误），但 issue 列表
    // 仍在途——队列不得谎称「没有匹配的 Issue。」。
    await act(async () => {
      repositories.resolve(
        new Response(JSON.stringify({ message: "仓库目录读取失败" }), {
          status: 500,
          headers: { "Content-Type": "application/json" },
        }),
      );
    });
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "仓库目录读取失败",
    );
    expect(within(queue).getByText("正在加载议题…")).toBeVisible();
    expect(within(queue).queryByText("没有匹配的 Issue。")).toBeNull();

    // 列表随后到达：队列显示行（生命周期未就绪时先落「生命周期待查看」组）。
    await act(async () => {
      issues.resolve(await baseFetch("/api/projects/project_0001/issues"));
    });
    expect(
      await within(queue).findByRole("button", {
        name: "选择 Issue 登录会话过期",
      }),
    ).toBeVisible();
    expect(within(queue).queryByText("没有匹配的 Issue。")).toBeNull();
  });
});
