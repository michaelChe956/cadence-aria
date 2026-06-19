import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type {
  CodingAttempt,
  IssueWorkItemPlanDetailDto,
  LifecycleWorkItem,
  WorkspaceSession,
} from "../../api/types";
import {
  useLifecycleWorkbenchStore,
  type LifecycleCard as LifecycleCardData,
} from "../../state/lifecycle-workbench-store";
import { CreateLifecycleIssueDialog } from "./CreateLifecycleIssueDialog";
import {
  defaultLaunchTitle,
  IssueLifecycleWorkbench,
} from "./IssueLifecycleWorkbench";

vi.mock("../shared/MonacoViewer", () => ({
  MonacoViewer: ({ value, height }: { value: string; height?: string }) => (
    <div data-testid="monaco-viewer" data-height={height}>
      {value}
    </div>
  ),
}));

type MockLifecycleData = {
  story_specs: Array<Record<string, unknown>>;
  design_specs: Array<Record<string, unknown>>;
  work_item_plans: unknown[];
  work_items: Array<Record<string, unknown>>;
  workspace_sessions: WorkspaceSession[];
  coding_attempts: CodingAttempt[];
};

describe("IssueLifecycleWorkbench", () => {
  beforeEach(() => {
    useLifecycleWorkbenchStore.setState({
      focusedEntityId: null,
      isDrawerOpen: false,
    });
  });

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

  it("creates repository from the left sidebar and enables issue creation", async () => {
    const fetchMock = lifecycleFetch({
      repositoriesByProject: { project_0001: [] },
    });
    vi.stubGlobal("fetch", fetchMock);
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);

    await screen.findByText("还没有代码库");
    expect(screen.getByRole("button", { name: "新建 Issue" })).toBeDisabled();

    await user.click(screen.getByRole("button", { name: "添加代码库" }));
    const dialog = screen.getByRole("dialog", { name: "添加代码库" });
    await user.type(within(dialog).getByLabelText("代码库名称"), "New Repo");
    await user.type(within(dialog).getByLabelText("本地路径"), "/tmp/new-repo");
    await user.selectOptions(
      within(dialog).getByLabelText("Policy"),
      "manual-all",
    );
    await user.selectOptions(
      within(dialog).getByLabelText("Provider"),
      "claude_code",
    );
    await user.click(
      within(dialog).getByRole("button", { name: "添加代码库" }),
    );

    expect(await screen.findByText("New Repo")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "新建 Issue" })).toBeEnabled();
    expect(fetchMock).toHaveBeenCalledWith(
      "/api/projects/project_0001/repositories",
      expect.objectContaining({
        method: "POST",
        body: JSON.stringify({
          name: "New Repo",
          path: "/tmp/new-repo",
          default_policy_preset: "manual-all",
          default_provider_mode: "claude_code",
        }),
      }),
    );
  });

  it("deletes project repositories and lifecycle issues from the lifecycle workbench", async () => {
    const fetchMock = lifecycleFetch();
    vi.stubGlobal("fetch", fetchMock);
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);

    expect(
      await screen.findByRole("button", { name: "登录会话过期" }),
    ).toBeInTheDocument();

    await user.click(
      screen.getByRole("button", { name: "删除代码库 Aria Repo" }),
    );

    expect(fetchMock).toHaveBeenCalledWith(
      "/api/projects/project_0001/repositories/repository_0001",
      expect.objectContaining({ method: "DELETE" }),
    );
    expect(await screen.findByText("还没有代码库")).toBeInTheDocument();

    await user.click(
      screen.getByRole("button", { name: "删除 Issue 登录会话过期" }),
    );

    expect(fetchMock).toHaveBeenCalledWith(
      "/api/projects/project_0001/issues/issue_0001",
      expect.objectContaining({ method: "DELETE" }),
    );
    await waitFor(() =>
      expect(
        screen.queryByRole("button", { name: "登录会话过期" }),
      ).not.toBeInTheDocument(),
    );

    await user.click(screen.getByRole("button", { name: "删除 Project Aria" }));

    expect(fetchMock).toHaveBeenCalledWith(
      "/api/projects/project_0001",
      expect.objectContaining({ method: "DELETE" }),
    );
    expect(await screen.findByText("还没有 Project")).toBeInTheDocument();
  });

  it("deletes specs from selected issue content and does not expose group deletion", async () => {
    const fetchMock = lifecycleFetch();
    vi.stubGlobal("fetch", fetchMock);
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);

    await user.click(
      await screen.findByRole("button", { name: "登录会话过期" }),
    );

    await user.click(
      within(screen.getByRole("region", { name: "Story Spec 内容" })).getByRole(
        "button",
        { name: "删除 Story Spec 会话过期提示" },
      ),
    );
    expect(fetchMock).toHaveBeenCalledWith(
      "/api/projects/project_0001/issues/issue_0001/story-specs/story_spec_0001",
      expect.objectContaining({ method: "DELETE" }),
    );
    await waitFor(() =>
      expect(
        screen.getByRole("region", { name: "Story Spec 内容" }),
      ).not.toHaveTextContent("会话过期提示"),
    );

    await user.click(
      within(
        screen.getByRole("region", { name: "Design Spec 内容" }),
      ).getByRole("button", { name: "删除 Design Spec 前端提示设计" }),
    );
    expect(fetchMock).toHaveBeenCalledWith(
      "/api/projects/project_0001/issues/issue_0001/design-specs/design_spec_0001",
      expect.objectContaining({ method: "DELETE" }),
    );
    await waitFor(() =>
      expect(
        screen.getByRole("region", { name: "Design Spec 内容" }),
      ).not.toHaveTextContent("前端提示设计"),
    );

    expect(
      within(
        screen.getByRole("region", { name: "Work Item 内容" }),
      ).queryByLabelText(/删除/u),
    ).not.toBeInTheDocument();
    expect(fetchMock).not.toHaveBeenCalledWith(
      expect.stringMatching(/\/work-items\//u),
      expect.objectContaining({ method: "DELETE" }),
    );
  });

  it("requires repository when creating issue", async () => {
    vi.stubGlobal("fetch", lifecycleFetch());
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);

    await screen.findByRole("region", { name: "Issue 卡片列表" });
    await user.click(screen.getByRole("button", { name: "新建 Issue" }));
    const dialog = screen.getByRole("dialog", { name: "新建 Issue" });
    await user.type(
      within(dialog).getByLabelText("Issue 标题"),
      "新增安全提示",
    );
    await user.click(
      within(dialog).getByRole("button", { name: "创建 Issue" }),
    );

    expect(within(dialog).getByText("请选择代码库")).toBeInTheDocument();
  });

  it("shows an alert for invalid lifecycle responses", async () => {
    vi.stubGlobal("fetch", lifecycleFetch({ invalidLifecycle: true }));

    render(<IssueLifecycleWorkbench />);

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "invalid lifecycle response",
    );
  });

  it("shows an alert for invalid work item plan detail fields", async () => {
    const { options: _options, ...missingOptions } = issueWorkItemPlanRecord();
    vi.stubGlobal(
      "fetch",
      lifecycleFetch({
        workItemPlans: [
          missingOptions,
          {
            ...issueWorkItemPlanRecord({ id: "issue_plan_wrong_shape" }),
            work_item_ids: "work_item_0001",
          },
        ],
      }),
    );

    render(<IssueLifecycleWorkbench />);

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "invalid lifecycle response",
    );
    expect(
      screen.queryByRole("button", { name: "Work Item Group" }),
    ).not.toBeInTheDocument();
  });

  it("keeps the latest refresh result when an older request finishes later", async () => {
    const firstProjects = deferred<Response>();
    const secondProjects = deferred<Response>();
    const fetchMock = lifecycleFetch({
      projectResponses: [firstProjects.promise, secondProjects.promise],
      issueTitles: ["最新 Issue", "旧 Issue"],
    });
    vi.stubGlobal("fetch", fetchMock);
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);
    await user.click(screen.getByRole("button", { name: "刷新" }));

    secondProjects.resolve(jsonResponseValue(projectsBody()));
    expect(
      await screen.findByRole("button", { name: "最新 Issue" }),
    ).toBeInTheDocument();

    firstProjects.resolve(jsonResponseValue(projectsBody()));
    await waitFor(() => expect(fetchMock).toHaveBeenCalled());
    expect(
      screen.queryByRole("button", { name: "旧 Issue" }),
    ).not.toBeInTheDocument();
  });

  it("does not mark derived cards selected when their id matches an issue id", async () => {
    vi.stubGlobal("fetch", lifecycleFetch({ duplicateCardIds: true }));
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);

    await user.click(
      await screen.findByRole("button", { name: "重复 ID Issue" }),
    );

    expect(
      screen.getByRole("button", { name: "重复 ID Issue" }),
    ).toHaveAttribute("aria-pressed", "true");
    expect(
      screen.getByRole("button", { name: "重复 ID Story" }),
    ).toHaveAttribute("aria-pressed", "false");
  });

  it("opens drawer from derived lifecycle cards and opens full screen workspace from drawer CTA", async () => {
    const fetchMock = lifecycleFetch();
    vi.stubGlobal("fetch", fetchMock);
    const user = userEvent.setup();
    const onOpenWorkspace = vi.fn();

    render(<IssueLifecycleWorkbench onOpenWorkspace={onOpenWorkspace} />);

    await screen.findByRole("button", { name: "会话过期提示" });
    await user.click(screen.getByRole("button", { name: "会话过期提示" }));

    expect(screen.getByTestId("lifecycle-card-drawer")).toHaveTextContent(
      "会话过期提示",
    );
    expect(onOpenWorkspace).not.toHaveBeenCalled();

    await user.click(screen.getByTestId("drawer-open-workspace"));
    expect(onOpenWorkspace).toHaveBeenCalledWith(
      "workspace_session_story_0001",
    );
    expect(fetchMock).not.toHaveBeenCalledWith(
      expect.stringMatching(
        /^\/api\/workspace-sessions\/.+\/(?:run-next|message|confirm)$/,
      ),
      expect.anything(),
    );
  });

  it("selects confirmed story cards so downstream design generation is reachable", async () => {
    const fetchMock = lifecycleFetch();
    vi.stubGlobal("fetch", fetchMock);
    const user = userEvent.setup();
    const onOpenWorkspace = vi.fn();

    render(<IssueLifecycleWorkbench onOpenWorkspace={onOpenWorkspace} />);

    await user.click(
      await screen.findByRole("button", { name: "会话过期提示" }),
    );

    expect(screen.getByTestId("lifecycle-card-drawer")).toHaveTextContent(
      "会话过期提示",
    );
    expect(onOpenWorkspace).not.toHaveBeenCalled();
    expect(
      screen.getByRole("button", { name: "生成 Design Spec" }),
    ).toBeInTheDocument();
  });

  it("shows spec version badges on lifecycle cards when generated content exists", async () => {
    vi.stubGlobal("fetch", lifecycleFetch());

    render(<IssueLifecycleWorkbench />);

    const storyColumn = await screen.findByRole("region", {
      name: "Story Spec 内容",
    });

    expect(storyColumn).toHaveTextContent("v1");
  });

  it("shows generated spec markdown previews on lifecycle cards", async () => {
    vi.stubGlobal("fetch", lifecycleFetch());

    render(<IssueLifecycleWorkbench />);

    const storyColumn = await screen.findByRole("region", {
      name: "Story Spec 内容",
    });
    expect(storyColumn).toHaveTextContent("[REQ-001] 显示会话过期提示");
  });

  it("generates story workspace from the issue card action and opens the story session", async () => {
    const fetchMock = lifecycleFetch({ emptyLifecycle: true });
    vi.stubGlobal("fetch", fetchMock);
    const user = userEvent.setup();
    const onOpenWorkspace = vi.fn();

    render(<IssueLifecycleWorkbench onOpenWorkspace={onOpenWorkspace} />);

    await screen.findByRole("button", { name: "登录会话过期" });
    await user.click(screen.getByRole("button", { name: "生成 Story Spec" }));

    expect(
      await screen.findByRole("button", { name: "登录会话过期 Story Spec" }),
    ).toBeInTheDocument();
    expect(onOpenWorkspace).toHaveBeenCalledWith(
      "workspace_session_story_0001",
    );
    expect(fetchMock).toHaveBeenCalledWith(
      "/api/projects/project_0001/issues/issue_0001/story-specs:generate",
      expect.objectContaining({
        method: "POST",
        body: JSON.stringify({
          title: "登录会话过期 Story Spec",
        }),
      }),
    );
  });

  it("does not expose the story generation action as a global header action", async () => {
    vi.stubGlobal("fetch", lifecycleFetch({ emptyLifecycle: true }));
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);

    await user.click(
      await screen.findByRole("button", { name: "登录会话过期" }),
    );

    const header = screen.getAllByRole("banner")[0];
    expect(
      within(header).queryByRole("button", { name: "生成 Story Spec" }),
    ).not.toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "生成 Story Spec" }),
    ).toBeInTheDocument();
  });

  it("prepares work item plan from design spec drawer and opens workspace", async () => {
    const fetchMock = lifecycleFetch();
    vi.stubGlobal("fetch", fetchMock);
    const user = userEvent.setup();
    const onOpenWorkspace = vi.fn();

    render(<IssueLifecycleWorkbench onOpenWorkspace={onOpenWorkspace} />);

    await user.click(
      await screen.findByRole("button", { name: "前端提示设计" }),
    );
    await user.click(screen.getByRole("button", { name: "生成 Work Item" }));

    await waitFor(() =>
      expect(onOpenWorkspace).toHaveBeenCalledWith(
        "workspace_session_plan_group_0001",
      ),
    );
    expect(
      screen.getByRole("region", { name: "Work Item 内容" }),
    ).toHaveTextContent("0 个 Work Item");
    expect(fetchMock).toHaveBeenCalledWith(
      "/api/projects/project_0001/issues/issue_0001/work-item-plans:prepare",
      expect.objectContaining({
        method: "POST",
        body: JSON.stringify({
          title: "前端提示设计 Work Item",
          story_spec_ids: ["story_spec_0001"],
          design_spec_ids: ["design_spec_0001"],
          include_integration_tests: true,
          include_e2e_tests: false,
          force_frontend_backend_split: true,
          require_execution_plan_confirm: false,
        }),
      }),
    );
    expect(fetchMock).not.toHaveBeenCalledWith(
      expect.stringMatching(
        /^\/api\/workspace-sessions\/.+\/(?:run-next|message|confirm)$/,
      ),
      expect.anything(),
    );

    await user.click(screen.getByRole("button", { name: "登录会话过期" }));
    const workItemRegion = screen.getByRole("region", {
      name: "Work Item 内容",
    });
    await user.click(
      within(workItemRegion).getByRole("button", { name: "Work Item Group" }),
    );
    expect(await screen.findByTestId("work-item-group-children")).toHaveTextContent(
      "暂无子 Work Item",
    );

    onOpenWorkspace.mockClear();
    await user.click(screen.getByTestId("drawer-open-workspace"));
    expect(onOpenWorkspace).toHaveBeenCalledWith(
      "workspace_session_plan_group_0001",
    );
  });

  it("generates next design spec from story spec drawer without opening workspace or running providers", async () => {
    const fetchMock = lifecycleFetch();
    vi.stubGlobal("fetch", fetchMock);
    const user = userEvent.setup();
    const onOpenWorkspace = vi.fn();

    render(<IssueLifecycleWorkbench onOpenWorkspace={onOpenWorkspace} />);

    await user.click(
      await screen.findByRole("button", { name: "会话过期提示" }),
    );
    await user.click(screen.getByRole("button", { name: "生成 Design Spec" }));

    expect(
      await screen.findByRole("button", { name: "会话过期提示 Design Spec" }),
    ).toBeInTheDocument();
    expect(onOpenWorkspace).not.toHaveBeenCalled();
    expect(fetchMock).toHaveBeenCalledWith(
      "/api/projects/project_0001/issues/issue_0001/design-specs:generate",
      expect.objectContaining({
        method: "POST",
        body: JSON.stringify({
          title: "会话过期提示 Design Spec",
          story_spec_ids: ["story_spec_0001"],
        }),
      }),
    );
    expect(fetchMock).not.toHaveBeenCalledWith(
      expect.stringMatching(
        /^\/api\/workspace-sessions\/.+\/(?:run-next|message|confirm)$/,
      ),
      expect.anything(),
    );
    await waitFor(() =>
      expect(useLifecycleWorkbenchStore.getState().focusedEntityId).toBe(
        "design_spec_0002",
      ),
    );
  });

  it("sends default work item split options when preparing plan from a confirmed design", async () => {
    const user = userEvent.setup();
    const fetchMock = lifecycleFetch();
    vi.stubGlobal("fetch", fetchMock);
    const onOpenWorkspace = vi.fn();

    render(<IssueLifecycleWorkbench onOpenWorkspace={onOpenWorkspace} />);

    await user.click(await screen.findByText("前端提示设计"));
    await user.click(screen.getByRole("button", { name: "生成 Work Item" }));

    await waitFor(() =>
      expect(fetchMock).toHaveBeenCalledWith(
        "/api/projects/project_0001/issues/issue_0001/work-item-plans:prepare",
        expect.objectContaining({
          method: "POST",
          body: expect.stringContaining(
            '"force_frontend_backend_split":true',
          ),
        }),
      ),
    );
    const prepareCall = fetchMock.mock.calls.find(([url]) =>
      String(url).includes("/work-item-plans:prepare"),
    );
    expect(prepareCall).toBeDefined();
    const body = JSON.parse(prepareCall?.[1]?.body as string) as Record<
      string,
      unknown
    >;
    expect(body).toMatchObject({
      include_integration_tests: true,
      include_e2e_tests: false,
      force_frontend_backend_split: true,
      require_execution_plan_confirm: false,
    });
  });

  it("opens the work item plan workspace from the group drawer", async () => {
    const fetchMock = lifecycleFetch({ confirmedWorkItem: true });
    vi.stubGlobal("fetch", fetchMock);
    const user = userEvent.setup();
    const onOpenWorkspace = vi.fn();

    render(
      <IssueLifecycleWorkbench onOpenWorkspace={onOpenWorkspace} />,
    );

    const workItemRegion = await screen.findByRole("region", {
      name: "Work Item 内容",
    });
    expect(workItemRegion).toHaveTextContent("confirmed");

    await user.click(
      await screen.findByRole("button", { name: "Work Item Group" }),
    );
    expect(screen.getByTestId("lifecycle-card-drawer")).toHaveTextContent(
      "confirmed",
    );
    await user.click(screen.getByTestId("drawer-open-workspace"));

    await waitFor(() =>
      expect(onOpenWorkspace).toHaveBeenCalledWith(
        "workspace_session_plan_group_0001",
      ),
    );
    expect(fetchMock).not.toHaveBeenCalledWith(
      expect.stringMatching(/\/coding-attempts$/u),
      expect.anything(),
    );
  });

  it("reveals child work items from the work item group drawer", async () => {
    vi.stubGlobal(
      "fetch",
      lifecycleFetch({
        confirmedWorkItem: true,
        workItemPlans: [
          issueWorkItemPlanRecord({
            id: "issue_plan_0001",
            status: "confirmed",
            work_item_ids: ["work_item_0001"],
          }),
        ],
      }),
    );
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);

    await user.click(
      await screen.findByRole("button", { name: "Work Item Group" }),
    );

    const children = await screen.findByTestId("work-item-group-children");
    expect(children).toHaveTextContent("实现提示组件");
    expect(children).toHaveTextContent("work_item_0001");
    expect(children).toHaveTextContent("backend");
    expect(children).toHaveTextContent("confirmed");
    expect(children).toHaveTextContent("planning");
    expect(screen.queryByRole("button", { name: "开始 Coding" })).not.toBeInTheDocument();
    expect(
      screen.queryByTestId("drawer-open-coding-workspace"),
    ).not.toBeInTheDocument();
    expect(screen.queryByTestId("drawer-delete-work-item")).not.toBeInTheDocument();
  });

  it("keeps drawer URL focus while opening the work item plan workspace", async () => {
    vi.stubGlobal("fetch", lifecycleFetch({ confirmedWorkItem: true }));
    const user = userEvent.setup();
    const onDrawerFocusChange = vi.fn();
    const onOpenWorkspace = vi.fn();

    render(
      <IssueLifecycleWorkbench
        focusEntityId="issue_plan_0001"
        onDrawerFocusChange={onDrawerFocusChange}
        onOpenWorkspace={onOpenWorkspace}
      />,
    );

    await screen.findByTestId("drawer-open-workspace");
    onDrawerFocusChange.mockClear();

    await user.click(screen.getByTestId("drawer-open-workspace"));

    await waitFor(() =>
      expect(onOpenWorkspace).toHaveBeenCalledWith(
        "workspace_session_plan_group_0001",
      ),
    );
    expect(onDrawerFocusChange).not.toHaveBeenCalledWith(null);
  });

  it("shows one work item group and reveals child work items in the drawer", async () => {
    vi.stubGlobal(
      "fetch",
      lifecycleFetch({
        splitWorkItems: true,
        workItemPlans: [
          issueWorkItemPlanRecord({
            id: "issue_plan_0001",
            status: "confirmed",
            work_item_ids: ["work_item_backend", "work_item_frontend"],
            validator_findings: [
              {
                finding_id: "finding_0001",
                level: "warning",
                code: "integration_or_e2e_skipped_risk",
                message: "需要补充贯通测试风险说明",
                affected_scopes: ["web/**"],
              },
            ],
            dependency_graph: [
              {
                from_work_item_id: "work_item_backend",
                to_work_item_id: "work_item_frontend",
              },
            ],
          }),
        ],
      }),
    );
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);
    await user.click(await screen.findByRole("button", { name: "登录会话过期" }));

    const workItemRegion = screen.getByRole("region", { name: "Work Item 内容" });
    expect(workItemRegion).toHaveTextContent("Work Item Group");
    expect(workItemRegion).not.toHaveTextContent("后端 API");
    expect(workItemRegion).not.toHaveTextContent("前端 UI");

    await user.click(
      within(workItemRegion).getByRole("button", { name: "Work Item Group" }),
    );

    const children = await screen.findByTestId("work-item-group-children");
    expect(children).toHaveTextContent("后端 API");
    expect(children).toHaveTextContent("work_item_backend");
    expect(children).toHaveTextContent("frontend");
    expect(children).toHaveTextContent("confirmed");
    expect(children).toHaveTextContent("pending");
    expect(children).toHaveTextContent("前端 UI");
    expect(screen.getByTestId("lifecycle-card-drawer")).toHaveTextContent(
      "story_spec_0001",
    );
    expect(screen.getByTestId("lifecycle-card-drawer")).toHaveTextContent(
      "design_spec_0001",
    );
    expect(screen.getByTestId("lifecycle-card-drawer")).toHaveTextContent(
      "confirmed",
    );
    expect(screen.getByTestId("lifecycle-card-drawer")).toHaveTextContent(
      "需要补充贯通测试风险说明",
    );
    expect(screen.getByTestId("lifecycle-card-drawer")).toHaveTextContent(
      "work_item_backend -> work_item_frontend",
    );
  });
});

describe("CreateLifecycleIssueDialog", () => {
  it("shows submit errors and prevents duplicate submissions while pending", async () => {
    const submit = deferred<void>();
    const onCreate = vi.fn(() => submit.promise);
    const user = userEvent.setup();

    render(
      <CreateLifecycleIssueDialog
        repositories={[repositoryRecord()]}
        onCreate={onCreate}
        onClose={vi.fn()}
      />,
    );

    await user.type(screen.getByLabelText("Issue 标题"), "新增安全提示");
    await user.selectOptions(
      screen.getByLabelText("代码库"),
      "repository_0001",
    );
    await user.click(screen.getByRole("button", { name: "创建 Issue" }));
    await user.click(screen.getByRole("button", { name: "创建 Issue" }));

    expect(onCreate).toHaveBeenCalledTimes(1);

    submit.reject(new Error("create issue failed"));
    expect(await screen.findByText("create issue failed")).toBeInTheDocument();
  });
});

function lifecycleFetch(options?: {
  duplicateCardIds?: boolean;
  emptyLifecycle?: boolean;
  invalidLifecycle?: boolean;
  confirmedWorkItem?: boolean;
  issueDescription?: string;
  issueTitles?: string[];
  issueTitlesByProject?: Record<string, string>;
  projects?: Array<ReturnType<typeof projectRecord>>;
  repositoriesByProject?: Record<string, ReturnType<typeof repositoryRecord>[]>;
  projectResponses?: Array<Promise<Response>>;
  splitWorkItems?: boolean;
  workItemPlans?: unknown[];
  skippedIntegrationRisk?: boolean;
}) {
  const projects = [
    ...(options?.projects ?? [projectRecord("project_0001", "Aria")]),
  ];
  const repositoriesByProject = new Map<
    string,
    ReturnType<typeof repositoryRecord>[]
  >(
    Object.entries(
      options?.repositoriesByProject ?? { project_0001: [repositoryRecord()] },
    ),
  );
  const deletedIssueIdsByProject = new Map<string, Set<string>>();
  let projectCall = 0;
  const issueCallsByProject = new Map<string, number>();
  const latestIssueTitlesByProject = new Map<string, string>();
  const lifecycleByIssue = new Map<string, MockLifecycleData>();

  function lifecycleData(issueId: string) {
    const existing = lifecycleByIssue.get(issueId);
    if (existing) {
      return existing;
    }
    const initial = initialLifecycleData(
      issueId,
      options?.duplicateCardIds,
      options?.emptyLifecycle,
      options?.confirmedWorkItem,
      options?.splitWorkItems,
      options?.workItemPlans,
      options?.skippedIntegrationRisk,
    );
    lifecycleByIssue.set(issueId, initial);
    return initial;
  }

  return vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input);
    if (url === "/api/projects" && init?.method === "POST") {
      const payload = JSON.parse(String(init.body)) as {
        name: string;
        description?: string | null;
      };
      const project = projectRecord(
        `project_${String(projects.length + 1).padStart(4, "0")}`,
        payload.name,
        payload.description ?? null,
      );
      projects.push(project);
      return jsonResponse(project);
    }
    const projectDeleteMatch = url.match(/^\/api\/projects\/([^/]+)$/);
    if (projectDeleteMatch && init?.method === "DELETE") {
      const projectId = projectDeleteMatch[1];
      const index = projects.findIndex(
        (project) => project.project_id === projectId,
      );
      if (index >= 0) {
        projects.splice(index, 1);
      }
      repositoriesByProject.delete(projectId);
      deletedIssueIdsByProject.delete(projectId);
      return jsonResponse({ status: "deleted" });
    }
    if (url === "/api/projects") {
      const response = options?.projectResponses?.[projectCall];
      projectCall += 1;
      return response ?? jsonResponse({ projects });
    }
    const repositoryDeleteMatch = url.match(
      /^\/api\/projects\/([^/]+)\/repositories\/([^/]+)$/,
    );
    if (repositoryDeleteMatch && init?.method === "DELETE") {
      const projectId = repositoryDeleteMatch[1];
      const repositoryId = repositoryDeleteMatch[2];
      repositoriesByProject.set(
        projectId,
        (repositoriesByProject.get(projectId) ?? []).filter(
          (repository) => repository.repository_id !== repositoryId,
        ),
      );
      return jsonResponse({ status: "deleted" });
    }
    const repositoryMatch = url.match(
      /^\/api\/projects\/([^/]+)\/repositories$/,
    );
    if (repositoryMatch) {
      const projectId = repositoryMatch[1];
      if (init?.method === "POST") {
        const payload = JSON.parse(String(init.body)) as {
          name: string;
          path: string;
          default_policy_preset?: string | null;
          default_provider_mode?: string | null;
        };
        const repositories = repositoriesByProject.get(projectId) ?? [];
        const repository = repositoryRecord({
          repository_id: `repository_${String(repositories.length + 1).padStart(4, "0")}`,
          project_id: projectId,
          name: payload.name,
          path: payload.path,
          default_policy_preset:
            payload.default_policy_preset ?? "manual-write",
          default_provider_mode: payload.default_provider_mode ?? "fake",
        });
        repositoriesByProject.set(projectId, [...repositories, repository]);
        return jsonResponse(repository);
      }
      return jsonResponse({
        repositories: repositoriesByProject.get(projectId) ?? [],
      });
    }
    if (
      url === "/api/projects/project_0001/issues" &&
      init?.method === "POST"
    ) {
      return jsonResponse({
        issue_id: "issue_0002",
        project_id: "project_0001",
        repo_id: "repository_0001",
        workspace_id: null,
        task_id: null,
        session_id: null,
        title: "新增安全提示",
        description: null,
        change_id: "new-security-hint",
        phase: "clarification",
        status: "draft",
        active_binding_id: null,
        artifacts: [],
        created_at: "2026-05-16T00:00:00Z",
        updated_at: "2026-05-16T00:00:00Z",
      });
    }
    const issueDeleteMatch = url.match(
      /^\/api\/projects\/([^/]+)\/issues\/([^/]+)$/,
    );
    if (issueDeleteMatch && init?.method === "DELETE") {
      const projectId = issueDeleteMatch[1];
      const issueId = issueDeleteMatch[2];
      const deletedIssueIds =
        deletedIssueIdsByProject.get(projectId) ?? new Set<string>();
      deletedIssueIds.add(issueId);
      deletedIssueIdsByProject.set(projectId, deletedIssueIds);
      lifecycleByIssue.delete(issueId);
      return jsonResponse({ status: "deleted" });
    }
    const storySpecDeleteMatch = url.match(
      /^\/api\/projects\/([^/]+)\/issues\/([^/]+)\/story-specs\/([^/]+)$/,
    );
    if (storySpecDeleteMatch && init?.method === "DELETE") {
      const issueId = storySpecDeleteMatch[2];
      const storySpecId = storySpecDeleteMatch[3];
      const lifecycle = lifecycleData(issueId);
      lifecycle.story_specs = lifecycle.story_specs.filter(
        (story) => story.story_spec_id !== storySpecId,
      );
      lifecycle.workspace_sessions = lifecycle.workspace_sessions.filter(
        (session) =>
          !(
            session.workspace_type === "story" &&
            session.entity_id === storySpecId
          ),
      );
      return jsonResponse({ status: "deleted" });
    }
    const designSpecDeleteMatch = url.match(
      /^\/api\/projects\/([^/]+)\/issues\/([^/]+)\/design-specs\/([^/]+)$/,
    );
    if (designSpecDeleteMatch && init?.method === "DELETE") {
      const issueId = designSpecDeleteMatch[2];
      const designSpecId = designSpecDeleteMatch[3];
      const lifecycle = lifecycleData(issueId);
      lifecycle.design_specs = lifecycle.design_specs.filter(
        (design) => design.design_spec_id !== designSpecId,
      );
      lifecycle.workspace_sessions = lifecycle.workspace_sessions.filter(
        (session) =>
          !(
            session.workspace_type === "design" &&
            session.entity_id === designSpecId
          ),
      );
      return jsonResponse({ status: "deleted" });
    }
    const workItemDeleteMatch = url.match(
      /^\/api\/projects\/([^/]+)\/issues\/([^/]+)\/work-items\/([^/]+)$/,
    );
    if (workItemDeleteMatch && init?.method === "DELETE") {
      const issueId = workItemDeleteMatch[2];
      const workItemId = workItemDeleteMatch[3];
      const lifecycle = lifecycleData(issueId);
      lifecycle.work_items = lifecycle.work_items.filter(
        (workItem) => workItem.work_item_id !== workItemId,
      );
      lifecycle.workspace_sessions = lifecycle.workspace_sessions.filter(
        (session) =>
          !(
            session.workspace_type === "work_item" &&
            session.entity_id === workItemId
          ),
      );
      lifecycle.coding_attempts = lifecycle.coding_attempts.filter(
        (attempt) => attempt.work_item_id !== workItemId,
      );
      return jsonResponse({ status: "deleted" });
    }
    const workspaceRunNextMatch = url.match(
      /^\/api\/workspace-sessions\/([^/]+)\/run-next$/,
    );
    if (workspaceRunNextMatch) {
      const payload = JSON.parse(String(init?.body ?? "{}")) as {
        user_prompt?: string;
      };
      const session = findSession(lifecycleByIssue, workspaceRunNextMatch[1]);
      if (session) {
        session.status = "waiting_for_human";
        session.messages = [
          ...session.messages,
          {
            role: "user",
            content: payload.user_prompt ?? "",
            created_at: "2026-05-16T00:00:00Z",
          },
          {
            role: "provider",
            content: "provider result",
            created_at: "2026-05-16T00:00:01Z",
          },
          {
            role: "reviewer",
            content: "reviewer result",
            created_at: "2026-05-16T00:00:02Z",
          },
        ];
      }
      return jsonResponse(session ?? {});
    }
    if (
      url === "/api/workspace-sessions/workspace_session_story_0001/message"
    ) {
      return jsonResponse({
        ...workspaceSessionRecord(
          "story",
          "story_spec_0001",
          "workspace_session_story_0001",
        ),
        messages: [
          {
            role: "user",
            content: "请补充验收标准",
            created_at: "2026-05-16T00:00:00Z",
          },
        ],
      });
    }
    const workspaceMessageMatch = url.match(
      /^\/api\/workspace-sessions\/([^/]+)\/message$/,
    );
    if (workspaceMessageMatch) {
      const session = findSession(lifecycleByIssue, workspaceMessageMatch[1]);
      return jsonResponse(session ?? {});
    }
    const workspaceConfirmMatch = url.match(
      /^\/api\/workspace-sessions\/([^/]+)\/confirm$/,
    );
    if (workspaceConfirmMatch) {
      const session = findSession(lifecycleByIssue, workspaceConfirmMatch[1]);
      if (session) {
        session.status = "confirmed";
      }
      const story = findStoryBySession(
        lifecycleByIssue,
        workspaceConfirmMatch[1],
      );
      if (story) {
        story.confirmation_status = "confirmed";
      }
      const design = findDesignBySession(
        lifecycleByIssue,
        workspaceConfirmMatch[1],
      );
      if (design) {
        design.confirmation_status = "confirmed";
      }
      return jsonResponse(session ?? {});
    }
    const storyGenerateMatch = url.match(
      /^\/api\/projects\/([^/]+)\/issues\/([^/]+)\/story-specs:generate$/,
    );
    if (storyGenerateMatch) {
      const issueId = storyGenerateMatch[2];
      const payload = JSON.parse(String(init?.body ?? "{}")) as {
        title: string;
        author_provider: "claude_code" | "codex" | "fake";
        reviewer_provider: "claude_code" | "codex" | "fake";
        review_rounds: number;
        superpowers_enabled: boolean;
        openspec_enabled: boolean;
      };
      const lifecycle = lifecycleData(issueId);
      const story = {
        story_spec_id: "story_spec_0001",
        issue_id: issueId,
        repository_id: "repository_0001",
        title: payload.title,
        current_version: null,
        current_markdown_preview: null,
        confirmation_status: "confirmed",
        artifact_versions: [],
      };
      const session = workspaceSessionRecord(
        "story",
        "story_spec_0001",
        "workspace_session_story_0001",
        {
          author_provider: payload.author_provider,
          reviewer_provider: payload.reviewer_provider,
          review_rounds: payload.review_rounds,
          superpowers_enabled: payload.superpowers_enabled,
          openspec_enabled: payload.openspec_enabled,
          status: "open",
        },
      );
      lifecycle.story_specs = [story];
      lifecycle.workspace_sessions = [session];
      return jsonResponse({ story_specs: [story], workspace_session: session });
    }
    const designGenerateMatch = url.match(
      /^\/api\/projects\/([^/]+)\/issues\/([^/]+)\/design-specs:generate$/,
    );
    if (designGenerateMatch) {
      const issueId = designGenerateMatch[2];
      const payload = JSON.parse(String(init?.body ?? "{}")) as {
        title: string;
        story_spec_ids: string[];
        author_provider: "claude_code" | "codex" | "fake";
        reviewer_provider: "claude_code" | "codex" | "fake";
        review_rounds: number;
        superpowers_enabled: boolean;
        openspec_enabled: boolean;
      };
      const lifecycle = lifecycleData(issueId);
      const designId = lifecycle.design_specs.some(
        (candidate) => candidate.design_spec_id === "design_spec_0001",
      )
        ? "design_spec_0002"
        : "design_spec_0001";
      const design = {
        design_spec_id: designId,
        issue_id: issueId,
        story_spec_ids: payload.story_spec_ids,
        title: payload.title,
        current_version: null,
        current_markdown_preview: null,
        confirmation_status: "confirmed",
        artifact_versions: [],
      };
      const session = workspaceSessionRecord(
        "design",
        designId,
        designId === "design_spec_0001"
          ? "workspace_session_design_0001"
          : "workspace_session_design_0002",
        {
          author_provider: payload.author_provider,
          reviewer_provider: payload.reviewer_provider,
          review_rounds: payload.review_rounds,
          superpowers_enabled: payload.superpowers_enabled,
          openspec_enabled: payload.openspec_enabled,
          status: "open",
        },
      );
      lifecycle.design_specs = [...lifecycle.design_specs, design];
      lifecycle.workspace_sessions.push(session);
      return jsonResponse({
        design_specs: [design],
        workspace_session: session,
      });
    }
    const prepareWorkItemPlanMatch = url.match(
      /^\/api\/projects\/([^/]+)\/issues\/([^/]+)\/work-item-plans:prepare$/,
    );
    if (prepareWorkItemPlanMatch) {
      const issueId = prepareWorkItemPlanMatch[2];
      const projectId = prepareWorkItemPlanMatch[1];
      const payload = JSON.parse(String(init?.body ?? "{}")) as {
        title: string;
        story_spec_ids: string[];
        design_spec_ids: string[];
        author_provider: "claude_code" | "codex" | "fake";
        reviewer_provider: "claude_code" | "codex" | "fake";
        review_rounds: number;
        superpowers_enabled: boolean;
        openspec_enabled: boolean;
        include_integration_tests?: boolean;
        include_e2e_tests?: boolean;
        force_frontend_backend_split?: boolean;
        require_execution_plan_confirm?: boolean;
      };
      const lifecycle = lifecycleData(issueId);
      const workItemPlan = issueWorkItemPlanRecord({
        id: "issue_plan_0001",
        project_id: projectId,
        issue_id: issueId,
        source_story_spec_ids: payload.story_spec_ids,
        source_design_spec_ids: payload.design_spec_ids,
        options: {
          include_integration_tests: payload.include_integration_tests ?? true,
          include_e2e_tests: payload.include_e2e_tests ?? false,
          force_frontend_backend_split:
            payload.force_frontend_backend_split ?? true,
          require_execution_plan_confirm:
            payload.require_execution_plan_confirm ?? false,
        },
        work_item_ids: [],
        status: "draft",
      });
      const session = workspaceSessionRecord(
        "work_item_plan",
        workItemPlan.id,
        "workspace_session_plan_group_0001",
        {
          author_provider: payload.author_provider,
          reviewer_provider: payload.reviewer_provider,
          review_rounds: payload.review_rounds,
          superpowers_enabled: payload.superpowers_enabled,
          openspec_enabled: payload.openspec_enabled,
          status: "open",
        },
      );
      lifecycle.work_item_plans = [workItemPlan];
      lifecycle.workspace_sessions.push(session);
      return jsonResponse({
        work_item_plan: workItemPlan,
        workspace_session: session,
      });
    }
    const codingAttemptCreateMatch = url.match(
      /^\/api\/projects\/([^/]+)\/issues\/([^/]+)\/work-items\/([^/]+)\/coding-attempts$/,
    );
    if (codingAttemptCreateMatch && init?.method === "POST") {
      const issueId = codingAttemptCreateMatch[2];
      const workItemId = codingAttemptCreateMatch[3];
      const lifecycle = lifecycleData(issueId);
      const attempt = codingAttemptRecord(workItemId);
      lifecycle.coding_attempts.push(attempt);
      const workItem = lifecycle.work_items.find(
        (candidate) => candidate.work_item_id === workItemId,
      );
      if (workItem) {
        workItem.latest_attempt = attempt;
        workItem.execution_status = "coding";
      }
      return jsonResponse(attempt);
    }
    const issuesMatch = url.match(/^\/api\/projects\/([^/]+)\/issues$/);
    if (issuesMatch) {
      const projectId = issuesMatch[1];
      const issueCall = issueCallsByProject.get(projectId) ?? 0;
      const title =
        options?.issueTitlesByProject?.[projectId] ??
        options?.issueTitles?.[issueCall] ??
        "登录会话过期";
      latestIssueTitlesByProject.set(projectId, title);
      issueCallsByProject.set(projectId, issueCall + 1);
      const issueId = options?.duplicateCardIds ? "shared_id" : "issue_0001";
      if (deletedIssueIdsByProject.get(projectId)?.has(issueId)) {
        return jsonResponse({ issues: [] });
      }
      if (projectId !== "project_0001") {
        if (deletedIssueIdsByProject.get(projectId)?.has("issue_0002")) {
          return jsonResponse({ issues: [] });
        }
        return jsonResponse({
          issues: title
            ? [
                {
                  issue_id: "issue_0002",
                  project_id: projectId,
                  repo_id: null,
                  workspace_id: null,
                  task_id: null,
                  session_id: null,
                  title,
                  description: options?.issueDescription ?? "描述",
                  change_id: "mobile-refresh",
                  phase: "clarification",
                  status: "draft",
                  active_binding_id: null,
                  artifacts: [],
                  created_at: "2026-05-16T00:00:00Z",
                  updated_at: "2026-05-16T00:00:00Z",
                },
              ]
            : [],
        });
      }
      return jsonResponse({
        issues: [
          {
            issue_id: issueId,
            project_id: "project_0001",
            repo_id: "repository_0001",
            workspace_id: null,
            task_id: null,
            session_id: null,
            title: options?.duplicateCardIds ? "重复 ID Issue" : title,
            description: options?.issueDescription ?? "描述",
            change_id: "login-session-expired",
            phase: "clarification",
            status: "draft",
            active_binding_id: null,
            artifacts: [],
            created_at: "2026-05-16T00:00:00Z",
            updated_at: "2026-05-16T00:00:00Z",
          },
        ],
      });
    }
    const lifecycleMatch = url.match(
      /^\/api\/issues\/([^/]+)\/lifecycle\?project_id=([^&]+)$/,
    );
    if (lifecycleMatch) {
      if (options?.invalidLifecycle) {
        return jsonResponse({});
      }
      const duplicate = options?.duplicateCardIds ?? false;
      const requestIssueId = lifecycleMatch[1];
      const projectId = lifecycleMatch[2];
      const issueId = duplicate ? "shared_id" : requestIssueId;
      const issueTitle = duplicate
        ? "重复 ID Issue"
        : latestIssueTitlesByProject.get(projectId) ??
          options?.issueTitlesByProject?.[projectId] ??
          "登录会话过期";
      if (projectId !== "project_0001") {
        return jsonResponse({
          issue: {
            issue_id: issueId,
            project_id: projectId,
            repo_id: null,
            workspace_id: null,
            task_id: null,
            session_id: null,
            title: issueTitle,
            description: options?.issueDescription ?? "描述",
            change_id: "mobile-refresh",
            phase: "clarification",
            status: "draft",
            active_binding_id: null,
            artifacts: [],
            created_at: "2026-05-16T00:00:00Z",
            updated_at: "2026-05-16T00:00:00Z",
          },
          story_specs: [],
          design_specs: [],
          work_item_plans: [],
          work_items: [],
          workspace_sessions: [],
          coding_attempts: [],
        });
      }
      const data = lifecycleData(issueId);
      return jsonResponse({
        issue: {
          issue_id: issueId,
          project_id: "project_0001",
          repo_id: "repository_0001",
          workspace_id: null,
          task_id: null,
          session_id: null,
          title: issueTitle,
          description: options?.issueDescription ?? "描述",
          change_id: "login-session-expired",
          phase: "clarification",
          status: "draft",
          active_binding_id: null,
          artifacts: [],
          created_at: "2026-05-16T00:00:00Z",
          updated_at: "2026-05-16T00:00:00Z",
        },
        story_specs: data.story_specs,
        design_specs: data.design_specs,
        work_item_plans: data.work_item_plans,
        work_items: data.work_items,
        workspace_sessions: data.workspace_sessions,
        coding_attempts: data.coding_attempts,
      });
    }
    return jsonResponse({});
  });
}

function jsonResponse(body: unknown) {
  return Promise.resolve(jsonResponseValue(body));
}

function jsonResponseValue(body: unknown) {
  return new Response(JSON.stringify(body), { status: 200 });
}

function projectsBody() {
  return {
    projects: [projectRecord("project_0001", "Aria")],
  };
}

function projectRecord(
  projectId: string,
  name: string,
  description: string | null = null,
) {
  return {
    project_id: projectId,
    name,
    description,
    created_at: "2026-05-16T00:00:00Z",
    updated_at: "2026-05-16T00:00:00Z",
    last_opened_at: null,
  };
}

function repositoryRecord(
  overrides?: Partial<ReturnType<typeof repositoryRecordShape>>,
) {
  return {
    ...repositoryRecordShape(),
    ...(overrides ?? {}),
  };
}

function repositoryRecordShape() {
  return {
    repository_id: "repository_0001",
    project_id: "project_0001",
    name: "Aria Repo",
    path: "/tmp/aria",
    repo_hash: "hash",
    runtime_root: "/tmp/aria/.aria/runtime",
    default_policy_preset: "manual-write",
    default_provider_mode: "fake",
    created_at: "2026-05-16T00:00:00Z",
    updated_at: "2026-05-16T00:00:00Z",
  };
}

function workspaceSessionRecord(
  workspaceType: "story" | "design" | "work_item" | "work_item_plan",
  entityId: string,
  sessionId: string,
  overrides?: Partial<WorkspaceSession>,
): WorkspaceSession {
  return {
    ...workspaceSessionRecordShape(workspaceType, entityId, sessionId),
    ...(overrides ?? {}),
  };
}

function workspaceSessionRecordShape(
  workspaceType: "story" | "design" | "work_item" | "work_item_plan",
  entityId: string,
  sessionId: string,
): WorkspaceSession {
  return {
    workspace_session_id: sessionId,
    issue_id: "issue_0001",
    entity_id: entityId,
    workspace_type: workspaceType,
    status: "waiting_for_human",
    author_provider: "codex",
    reviewer_provider: "claude_code",
    review_rounds: 2,
    superpowers_enabled: true,
    openspec_enabled: true,
    messages: [],
  };
}

function initialLifecycleData(
  issueId: string,
  duplicate: boolean | undefined,
  empty: boolean | undefined,
  confirmedWorkItem: boolean | undefined,
  splitWorkItems: boolean | undefined,
  workItemPlans: unknown[] | undefined,
  skippedIntegrationRisk: boolean | undefined,
): MockLifecycleData {
  if (empty) {
    return {
      story_specs: [],
      design_specs: [],
      work_item_plans: [],
      work_items: [],
      workspace_sessions: [],
      coding_attempts: [],
    };
  }

  const storyId = duplicate ? "shared_id" : "story_spec_0001";
  const workItems = splitWorkItems
    ? [
        workItemRecord({
          work_item_id: "work_item_backend",
          issue_id: issueId,
          title: "后端 API",
          kind: "backend",
          plan_status: "confirmed",
          execution_status: "pending",
          depends_on: [],
          validator_findings: skippedIntegrationRisk
            ? [
                {
                  finding_id: "finding_0001",
                  level: "warning",
                  code: "integration_or_e2e_skipped_risk",
                  message: "integration or e2e work item was skipped",
                  affected_scopes: [],
                },
              ]
            : undefined,
        }),
        workItemRecord({
          work_item_id: "work_item_frontend",
          issue_id: issueId,
          title: "前端 UI",
          kind: "frontend",
          plan_status: "confirmed",
          execution_status: "pending",
          depends_on: ["work_item_backend"],
        }),
      ]
    : [
        workItemRecord({
          issue_id: issueId,
          plan_status: confirmedWorkItem ? "confirmed" : "draft",
          artifact_versions: [
            {
              version: 1,
              markdown: "## 实施计划\n\n[TASK-001] 实现会话过期提示组件。",
              generated_by: "claude_code",
              reviewed_by: "codex",
              review_verdict: "pass",
              confirmed_by: confirmedWorkItem ? "human" : null,
              created_at: "2026-05-20T00:02:00Z",
              source_node_id: "timeline_node_work_item_001",
            },
          ],
        }),
      ];

  const defaultWorkItemPlans =
    workItems.length > 0
      ? [
          issueWorkItemPlanRecord({
            issue_id: issueId,
            work_item_ids: workItems.map((item) => item.work_item_id),
            status: confirmedWorkItem ? "confirmed" : "draft",
          }),
        ]
      : [];

  return {
    story_specs: [
      {
        story_spec_id: storyId,
        issue_id: issueId,
        repository_id: "repository_0001",
        title: duplicate ? "重复 ID Story" : "会话过期提示",
        current_version: 1,
        current_markdown_preview: "## 功能需求\n\n[REQ-001] 显示会话过期提示。",
        confirmation_status: "confirmed",
        artifact_versions: [],
      },
    ],
    design_specs: [
      {
        design_spec_id: "design_spec_0001",
        issue_id: issueId,
        story_spec_ids: [storyId],
        title: "前端提示设计",
        current_version: 1,
        current_markdown_preview: "## 关键决策\n\n[DEC-001] 使用全局提示条。",
        confirmation_status: "confirmed",
        artifact_versions: [],
      },
    ],
    work_item_plans: workItemPlans ?? defaultWorkItemPlans,
    work_items: workItems,
    workspace_sessions: [
      workspaceSessionRecord("story", storyId, "workspace_session_story_0001"),
      workspaceSessionRecord(
        "design",
        "design_spec_0001",
        "workspace_session_design_0001",
      ),
      workspaceSessionRecord(
        "work_item",
        "work_item_0001",
        "workspace_session_work_item_0001",
      ),
      workspaceSessionRecord(
        "work_item_plan",
        "issue_plan_0001",
        "workspace_session_plan_group_0001",
      ),
    ],
    coding_attempts: [],
  };
}

function codingAttemptRecord(workItemId: string): CodingAttempt {
  return {
    attempt_id: "coding_attempt_0001",
    work_item_id: workItemId,
    attempt_no: 1,
    status: "created",
    stage: "prepare_context",
    branch_name: `aria/work-items/${workItemId}/attempt-1`,
    base_branch: "main",
    worktree_path: null,
    rework_count: 0,
    head_commit: null,
    push_status: null,
    review_request_url: null,
    created_at: "2026-05-23T00:00:00Z",
    updated_at: "2026-05-23T00:00:00Z",
  };
}

function issueWorkItemPlanRecord(
  overrides: Partial<IssueWorkItemPlanDetailDto> = {},
): IssueWorkItemPlanDetailDto {
  return {
    id: "issue_plan_0001",
    issue_id: "issue_0001",
    project_id: "project_0001",
    status: "draft",
    source_story_spec_ids: ["story_spec_0001"],
    source_design_spec_ids: ["design_spec_0001"],
    work_item_ids: ["work_item_0001"],
    verification_plan_ids: [],
    dependency_graph: [],
    repository_profile_ref: null,
    options: {
      include_integration_tests: true,
      include_e2e_tests: false,
      force_frontend_backend_split: true,
      require_execution_plan_confirm: false,
    },
    validator_findings: [],
    created_at: "2026-05-20T00:00:00Z",
    updated_at: "2026-05-20T00:00:00Z",
    ...overrides,
  };
}

function workItemRecord(
  overrides: Partial<LifecycleWorkItem> = {},
): LifecycleWorkItem {
  return {
    work_item_id: "work_item_0001",
    issue_id: "issue_0001",
    repository_id: "repository_0001",
    story_spec_ids: ["story_spec_0001"],
    design_spec_ids: ["design_spec_0001"],
    title: "实现提示组件",
    plan_status: "draft",
    execution_status: "planning",
    latest_attempt: null,
    artifact_versions: [
      {
        version: 1,
        markdown: "## 实施计划\n\n[TASK-001] 实现会话过期提示组件。",
        generated_by: "claude_code",
        reviewed_by: "codex",
        review_verdict: "pass",
        confirmed_by: null,
        created_at: "2026-05-20T00:02:00Z",
        source_node_id: "timeline_node_work_item_001",
      },
    ],
    work_item_set_id: null,
    kind: "backend",
    sequence_hint: null,
    depends_on: [],
    exclusive_write_scopes: [],
    forbidden_write_scopes: [],
    context_budget: {
      target_context_k: "30-50",
      max_summary_chars: 20000,
      max_handoff_chars: 12000,
      max_code_context_chars: 30000,
      max_context_file_refs: 80,
      max_traceability_refs: 40,
      max_dependency_handoffs: 3,
    },
    required_handoff_from: [],
    verification_plan_ref: null,
    require_execution_plan_confirm: false,
    execution_plan_status: "not_started",
    handoff_summary_ref: null,
    completion_commit: null,
    completion_diff_summary_ref: null,
    ...overrides,
  };
}

function findSession(
  lifecycles: Map<
    string,
    {
      workspace_sessions: WorkspaceSession[];
    }
  >,
  sessionId: string,
) {
  for (const lifecycle of lifecycles.values()) {
    const session = lifecycle.workspace_sessions.find(
      (candidate) => candidate.workspace_session_id === sessionId,
    );
    if (session) {
      return session;
    }
  }
  return null;
}

function findStoryBySession(
  lifecycles: Map<
    string,
    {
      story_specs: Array<Record<string, unknown>>;
      workspace_sessions: WorkspaceSession[];
    }
  >,
  sessionId: string,
) {
  for (const lifecycle of lifecycles.values()) {
    const session = lifecycle.workspace_sessions.find(
      (candidate) =>
        candidate.workspace_session_id === sessionId &&
        candidate.workspace_type === "story",
    );
    if (session) {
      return (
        lifecycle.story_specs.find(
          (story) => story.story_spec_id === session.entity_id,
        ) ?? null
      );
    }
  }
  return null;
}

function findDesignBySession(
  lifecycles: Map<
    string,
    {
      design_specs: Array<Record<string, unknown>>;
      workspace_sessions: WorkspaceSession[];
    }
  >,
  sessionId: string,
) {
  for (const lifecycle of lifecycles.values()) {
    const session = lifecycle.workspace_sessions.find(
      (candidate) =>
        candidate.workspace_session_id === sessionId &&
        candidate.workspace_type === "design",
    );
    if (session) {
      return (
        lifecycle.design_specs.find(
          (design) => design.design_spec_id === session.entity_id,
        ) ?? null
      );
    }
  }
  return null;
}

function lifecycleCardTitle(
  kind: LifecycleCardData["kind"],
  title: string,
): LifecycleCardData {
  return {
    kind,
    title,
  } as LifecycleCardData;
}

function deferred<T>() {
  let resolve!: (value: T | PromiseLike<T>) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((promiseResolve, promiseReject) => {
    resolve = promiseResolve;
    reject = promiseReject;
  });
  return { promise, resolve, reject };
}
