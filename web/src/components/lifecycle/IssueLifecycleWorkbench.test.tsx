import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { useLifecycleWorkbenchStore } from "../../state/lifecycle-workbench-store";
import {
  defaultLaunchTitle,
  IssueLifecycleWorkbench,
} from "./IssueLifecycleWorkbench";
import {
  deferred,
  installIssueLifecycleWorkbenchTestHooks,
  issueWorkItemPlanRecord,
  lifecycleCardTitle,
  lifecycleFetch,
  projectRecord,
  repositoryRecord,
} from "./IssueLifecycleWorkbench.test-utils";

vi.mock("../shared/MonacoViewer", () => ({
  MonacoViewer: ({ value, height }: { value: string; height?: string }) => (
    <div data-testid="monaco-viewer" data-height={height}>
      {value}
    </div>
  ),
}));

describe("IssueLifecycleWorkbench base workflow", () => {
  installIssueLifecycleWorkbenchTestHooks();

  it("renders issues as the primary workbench and shows selected issue lifecycle content", async () => {
    vi.stubGlobal("fetch", lifecycleFetch());
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);

    expect(
      await screen.findByRole("navigation", { name: "Project 切换" }),
    ).toHaveTextContent("Aria");
    expect(
      await screen.findByRole("region", { name: "Issue 卡片列表" }),
    ).toHaveTextContent("登录会话过期");
    expect(
      screen.queryByRole("region", { name: "Story Spec 列" }),
    ).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "登录会话过期" }));

    expect(
      screen.getByRole("region", { name: "Issue 生命周期详情" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("region", { name: "Story Spec 内容" }),
    ).toHaveTextContent("会话过期提示");
    expect(
      screen.getByRole("region", { name: "Design Spec 内容" }),
    ).toHaveTextContent("前端提示设计");
    expect(
      screen.getByRole("region", { name: "Work Item 内容" }),
    ).toHaveTextContent("Work Item Group");
    expect(
      screen.getByRole("region", { name: "Work Item 内容" }),
    ).not.toHaveTextContent("实现提示组件");
  });

  it("keeps long selected issue descriptions compact and opens the full content in the drawer", async () => {
    vi.stubGlobal(
      "fetch",
      lifecycleFetch({
        issueDescription:
          "第 1 行：背景说明\n第 2 行：用户场景\n第 3 行：边界条件\n第 4 行：异常路径\n第 5 行：业务规则\n第 6 行：主要流程\n第 7 行：补充约束\n第 8 行：完整验收标准",
      }),
    );
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);

    await user.click(
      await screen.findByRole("button", { name: "登录会话过期" }),
    );

    const detail = screen.getByRole("region", { name: "Issue 生命周期详情" });
    expect(within(detail).getByTestId("selected-issue-preview")).toHaveClass(
      "line-clamp-6",
    );

    await user.click(
      within(detail).getByRole("button", { name: "查看完整 Issue" }),
    );

    expect(
      await screen.findByTestId("lifecycle-card-drawer"),
    ).toHaveTextContent("查看 Markdown 内容");
    await user.click(
      screen.getByRole("button", { name: "查看 Markdown 内容" }),
    );

    expect(screen.getByTestId("monaco-viewer")).toHaveTextContent(
      "第 8 行：完整验收标准",
    );
  });

  it("does not chain lifecycle type suffixes when generating default titles", () => {
    expect(
      defaultLaunchTitle({
        target: "design",
        card: lifecycleCardTitle("story_spec", "爬楼梯问题 Story Spec"),
      }),
    ).toBe("爬楼梯问题 Design Spec");
    expect(
      defaultLaunchTitle({
        target: "work_item",
        card: lifecycleCardTitle(
          "design_spec",
          "爬楼梯问题 Story Spec Design Spec",
        ),
      }),
    ).toBe("爬楼梯问题 Work Item");
  });

  it("syncs controlled URL focus with drawer state", async () => {
    vi.stubGlobal("fetch", lifecycleFetch());
    const onDrawerFocusChange = vi.fn();

    const view = render(
      <IssueLifecycleWorkbench
        focusEntityId="story_spec_0001"
        onDrawerFocusChange={onDrawerFocusChange}
      />,
    );

    await waitFor(() =>
      expect(useLifecycleWorkbenchStore.getState().focusedEntityId).toBe(
        "story_spec_0001",
      ),
    );
    expect(useLifecycleWorkbenchStore.getState().isDrawerOpen).toBe(true);
    await waitFor(() =>
      expect(onDrawerFocusChange).toHaveBeenCalledWith("story_spec_0001"),
    );

    view.rerender(
      <IssueLifecycleWorkbench
        focusEntityId={null}
        onDrawerFocusChange={onDrawerFocusChange}
      />,
    );

    await waitFor(() =>
      expect(useLifecycleWorkbenchStore.getState().focusedEntityId).toBeNull(),
    );
    expect(useLifecycleWorkbenchStore.getState().isDrawerOpen).toBe(false);

    act(() => {
      useLifecycleWorkbenchStore.getState().openDrawer("design_spec_0001");
    });

    await waitFor(() =>
      expect(onDrawerFocusChange).toHaveBeenCalledWith("design_spec_0001"),
    );
  });

  it("switches project from the left sidebar", async () => {
    const fetchMock = lifecycleFetch({
      projects: [
        projectRecord("project_0001", "Aria"),
        projectRecord("project_0002", "Mobile"),
      ],
      issueTitlesByProject: {
        project_0001: "登录会话过期",
        project_0002: "移动端刷新",
      },
    });
    vi.stubGlobal("fetch", fetchMock);
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);

    expect(
      await screen.findByRole("button", { name: "登录会话过期" }),
    ).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Mobile" }));

    expect(
      await screen.findByRole("button", { name: "移动端刷新" }),
    ).toBeInTheDocument();
    expect(fetchMock).toHaveBeenCalledWith(
      "/api/projects/project_0002/issues",
      expect.objectContaining({
        headers: expect.objectContaining({
          "content-type": "application/json",
        }),
      }),
    );
  });

  it("creates project from the left sidebar and selects it", async () => {
    const fetchMock = lifecycleFetch({ projects: [] });
    vi.stubGlobal("fetch", fetchMock);
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);

    expect(await screen.findByText("还没有 Project")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "新建 Project" }));
    const dialog = screen.getByRole("dialog", { name: "新建 Project" });
    await user.type(
      within(dialog).getByLabelText("Project 名称"),
      "New Project",
    );
    await user.type(
      within(dialog).getByLabelText("Project 描述"),
      "新的生命周期项目",
    );
    await user.click(
      within(dialog).getByRole("button", { name: "创建 Project" }),
    );

    expect(
      await screen.findByRole("button", { name: "New Project" }),
    ).toHaveAttribute("aria-pressed", "true");
    expect(fetchMock).toHaveBeenCalledWith(
      "/api/projects",
      expect.objectContaining({
        method: "POST",
        body: JSON.stringify({
          name: "New Project",
          description: "新的生命周期项目",
        }),
      }),
    );
  });

  it("shows project repositories in the left sidebar", async () => {
    vi.stubGlobal("fetch", lifecycleFetch());

    render(<IssueLifecycleWorkbench />);

    const sidebar = await screen.findByRole("navigation", {
      name: "Project 切换",
    });
    expect(sidebar).toHaveTextContent("Aria Repo");
    expect(sidebar).toHaveTextContent("/tmp/aria");
  });

  it("shows only repositories for the selected project", async () => {
    vi.stubGlobal(
      "fetch",
      lifecycleFetch({
        projects: [
          projectRecord("project_0001", "Aria"),
          projectRecord("project_0002", "Mobile"),
        ],
        repositoriesByProject: {
          project_0001: [
            repositoryRecord({
              repository_id: "repository_0001",
              project_id: "project_0001",
              name: "Aria Repo",
              path: "/tmp/aria",
            }),
          ],
          project_0002: [
            repositoryRecord({
              repository_id: "repository_0002",
              project_id: "project_0002",
              name: "Mobile Repo",
              path: "/tmp/mobile",
            }),
          ],
        },
        issueTitlesByProject: {
          project_0001: "登录会话过期",
          project_0002: "",
        },
      }),
    );
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);

    const sidebar = await screen.findByRole("navigation", {
      name: "Project 切换",
    });
    expect(sidebar).toHaveTextContent("Aria Repo");
    expect(sidebar).not.toHaveTextContent("Mobile Repo");

    await user.click(screen.getByRole("button", { name: "Mobile" }));

    expect(await screen.findByText("Mobile Repo")).toBeInTheDocument();
    expect(sidebar).not.toHaveTextContent("Aria Repo");
  });
});
