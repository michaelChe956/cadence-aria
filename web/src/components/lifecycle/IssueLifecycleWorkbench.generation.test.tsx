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

describe("IssueLifecycleWorkbench generation actions", () => {
  installIssueLifecycleWorkbenchTestHooks();

  it("shows spec version badges on lifecycle cards when generated content exists", async () => {
    vi.stubGlobal("fetch", lifecycleFetch());
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);

    // Task 6：单阶段面板——完整生命周期默认落在 work_item，先切到 Story 阶段。
    await user.click(await screen.findByTestId("stage-tab-story"));
    const storyColumn = screen.getByRole("region", {
      name: "Story Spec 内容",
    });

    expect(storyColumn).toHaveTextContent("v1");
  });

  it("shows generated spec markdown previews on lifecycle cards", async () => {
    vi.stubGlobal("fetch", lifecycleFetch());
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);

    await user.click(await screen.findByTestId("stage-tab-story"));
    const storyColumn = screen.getByRole("region", {
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
    // Task 6：空 story 阶段面板也有同名主按钮，本例验证 Issue 卡入口，故限定在卡片列表内。
    await user.click(
      within(
        screen.getByRole("region", { name: "Issue 卡片列表" }),
      ).getByRole("button", { name: "生成 Story Spec" }),
    );

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
    // Task 6：除 Issue 卡入口外，空 story 阶段面板也常驻提供该动作。
    expect(
      within(
        screen.getByRole("region", { name: "Issue 卡片列表" }),
      ).getByRole("button", { name: "生成 Story Spec" }),
    ).toBeInTheDocument();
    expect(
      within(
        screen.getByRole("region", { name: "Story Spec 内容" }),
      ).getByRole("button", { name: "生成 Story Spec" }),
    ).toBeInTheDocument();
  });

  it("prepares work item plan from design spec drawer and opens workspace", async () => {
    const fetchMock = lifecycleFetch();
    vi.stubGlobal("fetch", fetchMock);
    const user = userEvent.setup();
    const onOpenWorkspace = vi.fn();

    render(<IssueLifecycleWorkbench onOpenWorkspace={onOpenWorkspace} />);

    // Task 6：Design 卡片在 Design 阶段页内。
    await user.click(await screen.findByTestId("stage-tab-design"));
    await user.click(screen.getByRole("button", { name: "前端提示设计" }));
    await user.click(screen.getByRole("button", { name: "生成 Work Item" }));
    const dialog = await screen.findByRole("dialog", {
      name: "Work Item Plan 配置",
    });
    await user.click(
      within(dialog).getByRole("button", { name: "创建并打开 Workspace" }),
    );

    await waitFor(() =>
      expect(onOpenWorkspace).toHaveBeenCalledWith(
        "workspace_session_plan_group_0001",
      ),
    );
    await user.click(screen.getByTestId("stage-tab-work_item"));
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

    // Task 6：Story 卡片在 Story 阶段页内。
    await user.click(await screen.findByTestId("stage-tab-story"));
    await user.click(screen.getByRole("button", { name: "会话过期提示" }));
    await user.click(screen.getByRole("button", { name: "生成 Design Spec" }));

    // Task 6：新生成的 Design 卡片属于 Design 阶段页，切过去断言它已入列。
    await user.click(await screen.findByTestId("stage-tab-design"));
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
      expect(useLifecycleWorkbenchStore.getState().focusedEntityKey).toBe(
        "design_spec:issue_0001:design_spec_0002",
      ),
    );
  });

  it("opens work item plan options before preparing plan from a confirmed design", async () => {
    const user = userEvent.setup();
    const fetchMock = lifecycleFetch();
    vi.stubGlobal("fetch", fetchMock);

    render(<IssueLifecycleWorkbench />);

    await user.click(await screen.findByTestId("stage-tab-design"));
    await user.click(screen.getByText("前端提示设计"));
    await user.click(screen.getByRole("button", { name: "生成 Work Item" }));

    expect(
      await screen.findByRole("dialog", { name: "Work Item Plan 配置" }),
    ).toBeInTheDocument();
    expect(
      fetchMock.mock.calls.some(([url]) =>
        String(url).includes("/work-item-plans:prepare"),
      ),
    ).toBe(false);
  });

  it("sends default work item split options after confirming the dialog", async () => {
    const user = userEvent.setup();
    const fetchMock = lifecycleFetch();
    vi.stubGlobal("fetch", fetchMock);
    const onOpenWorkspace = vi.fn();

    render(<IssueLifecycleWorkbench onOpenWorkspace={onOpenWorkspace} />);

    await user.click(await screen.findByTestId("stage-tab-design"));
    await user.click(screen.getByText("前端提示设计"));
    await user.click(screen.getByRole("button", { name: "生成 Work Item" }));
    const dialog = await screen.findByRole("dialog", {
      name: "Work Item Plan 配置",
    });

    await user.click(
      within(dialog).getByRole("button", { name: "创建并打开 Workspace" }),
    );

    await waitFor(() =>
      expect(fetchMock).toHaveBeenCalledWith(
        "/api/projects/project_0001/issues/issue_0001/work-item-plans:prepare",
        expect.objectContaining({
          method: "POST",
          body: expect.stringContaining('"force_frontend_backend_split":true'),
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

  it("sends selected work item split options after confirming the dialog", async () => {
    const user = userEvent.setup();
    const fetchMock = lifecycleFetch();
    vi.stubGlobal("fetch", fetchMock);
    const onOpenWorkspace = vi.fn();

    render(<IssueLifecycleWorkbench onOpenWorkspace={onOpenWorkspace} />);

    await user.click(await screen.findByTestId("stage-tab-design"));
    await user.click(screen.getByText("前端提示设计"));
    await user.click(screen.getByRole("button", { name: "生成 Work Item" }));
    const dialog = await screen.findByRole("dialog", {
      name: "Work Item Plan 配置",
    });

    await user.click(within(dialog).getByLabelText("包含 E2E 测试 Work Item"));
    await user.click(
      within(dialog).getByLabelText("子 Work Item 执行前需要确认 Plan"),
    );
    await user.click(
      within(dialog).getByRole("button", { name: "创建并打开 Workspace" }),
    );

    await waitFor(() =>
      expect(onOpenWorkspace).toHaveBeenCalledWith(
        "workspace_session_plan_group_0001",
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
      include_e2e_tests: true,
      force_frontend_backend_split: true,
      require_execution_plan_confirm: true,
    });
  });

  it("does not prepare a work item plan when options dialog is cancelled", async () => {
    const user = userEvent.setup();
    const fetchMock = lifecycleFetch();
    vi.stubGlobal("fetch", fetchMock);

    render(<IssueLifecycleWorkbench />);

    await user.click(await screen.findByTestId("stage-tab-design"));
    await user.click(screen.getByText("前端提示设计"));
    await user.click(screen.getByRole("button", { name: "生成 Work Item" }));
    const dialog = await screen.findByRole("dialog", {
      name: "Work Item Plan 配置",
    });

    await user.click(within(dialog).getByRole("button", { name: "取消" }));

    expect(
      screen.queryByRole("dialog", { name: "Work Item Plan 配置" }),
    ).not.toBeInTheDocument();
    expect(
      fetchMock.mock.calls.some(([url]) =>
        String(url).includes("/work-item-plans:prepare"),
      ),
    ).toBe(false);
  });
});
