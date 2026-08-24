import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { useLifecycleWorkbenchStore } from "../../state/lifecycle-workbench-store";
import {
  defaultLaunchTitle,
  IssueLifecycleWorkbench,
} from "./IssueLifecycleWorkbench";
import { resolveGroupCodingAttempt } from "./IssueLifecycleWorkbenchParts";
import {
  codingGroupAttemptRecord,
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

describe("IssueLifecycleWorkbench drawer and work item groups", () => {
  installIssueLifecycleWorkbenchTestHooks();

  it.each([
    [
      "Story Spec",
      "issue_0002 Story",
      "issue_0002 Story",
      "workspace_session_issue_0002_story",
    ],
    [
      "Design Spec",
      "issue_0002 Design",
      "issue_0002 Design",
      "workspace_session_issue_0002_design",
    ],
    [
      "Work Item",
      "Work Item Group",
      "issue_0002 Child",
      "workspace_session_issue_0002_work_item_plan",
    ],
  ] as const)(
    "opens issue2 %s drawer and workspace when local ids collide",
    async (regionName, cardName, drawerText, sessionId) => {
      vi.stubGlobal(
        "fetch",
        lifecycleFetch({ sharedLifecycleIdsAcrossIssues: true }),
      );
      const user = userEvent.setup();
      const onOpenWorkspace = vi.fn();

      render(
        <IssueLifecycleWorkbench onOpenWorkspace={onOpenWorkspace} />,
      );

      // Task 7：队列改为 IssueQueue，行选择按钮名为「选择 Issue <标题>」。
      await user.click(
        await screen.findByRole("button", { name: "选择 Issue Issue Two" }),
      );
      // Task 6：单阶段面板——先切到目标阶段，再在该阶段区域内取卡片。
      await user.click(
        screen.getByTestId(
          regionName === "Story Spec"
            ? "stage-tab-story"
            : regionName === "Design Spec"
              ? "stage-tab-design"
              : "stage-tab-work_item",
        ),
      );
      const region = screen.getByRole("region", {
        name: `${regionName} 内容`,
      });
      await user.click(
        within(region).getByRole("button", { name: cardName }),
      );

      expect(screen.getByTestId("lifecycle-card-drawer")).toHaveTextContent(
        drawerText,
      );
      await user.click(screen.getByTestId("drawer-open-workspace"));
      await waitFor(() =>
        expect(onOpenWorkspace).toHaveBeenCalledWith(sessionId),
      );
    },
  );

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

  it("deletes the work item group from the group drawer", async () => {
    const fetchMock = lifecycleFetch({ confirmedWorkItem: true });
    vi.stubGlobal("fetch", fetchMock);
    const confirmSpy = vi.spyOn(window, "confirm").mockReturnValue(true);
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);

    await user.click(
      await screen.findByRole("button", { name: "Work Item Group" }),
    );
    await user.click(
      screen.getByRole("button", { name: "删除 Work Item Group" }),
    );

    expect(confirmSpy).toHaveBeenCalledWith(
      expect.stringContaining("删除 Work Item Group"),
    );
    expect(fetchMock).toHaveBeenCalledWith(
      "/api/projects/project_0001/issues/issue_0001/work-item-plans/issue_plan_0001",
      expect.objectContaining({ method: "DELETE" }),
    );
    await waitFor(() =>
      expect(
        screen.getByRole("region", { name: "Work Item 内容" }),
      ).not.toHaveTextContent("Work Item Group"),
    );
    confirmSpy.mockRestore();
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
    expect(screen.getByRole("button", { name: "开始 Coding" })).toBeInTheDocument();
    expect(screen.getByTestId("drawer-open-coding-workspace")).toBeInTheDocument();
    expect(screen.queryByTestId("drawer-delete-work-item")).not.toBeInTheDocument();
  });

  it("creates a group coding attempt from the group drawer when no group attempt exists", async () => {
    const fetchMock = lifecycleFetch({
      confirmedWorkItem: true,
      splitWorkItems: true,
      workItemPlans: [
        issueWorkItemPlanRecord({
          id: "issue_plan_0001",
          status: "confirmed",
          work_item_ids: ["work_item_backend", "work_item_frontend"],
        }),
      ],
    });
    vi.stubGlobal("fetch", fetchMock);
    const user = userEvent.setup();
    const onOpenCodingWorkspace = vi.fn();

    render(
      <IssueLifecycleWorkbench onOpenCodingWorkspace={onOpenCodingWorkspace} />,
    );

    await user.click(
      await screen.findByRole("button", { name: "Work Item Group" }),
    );
    await user.click(screen.getByTestId("drawer-open-coding-workspace"));

    await waitFor(() =>
      expect(onOpenCodingWorkspace).toHaveBeenCalledWith({
        projectId: "project_0001",
        issueId: "issue_0001",
        attemptId: "coding_attempt_0001",
      }),
    );
    expect(fetchMock).toHaveBeenCalledWith(
      "/api/projects/project_0001/issues/issue_0001/work-item-plans/issue_plan_0001/coding-attempts",
      expect.objectContaining({ method: "POST" }),
    );
  });

  it("shows existing group coding label and reuses the active fallback attempt", async () => {
    const fetchMock = lifecycleFetch({
      confirmedWorkItem: true,
      splitWorkItems: true,
      workItemPlans: [
        issueWorkItemPlanRecord({
          id: "issue_plan_0001",
          status: "confirmed",
          work_item_ids: ["work_item_backend", "work_item_frontend"],
        }),
      ],
      codingAttempts: [
        {
          ...codingGroupAttemptRecord("issue_plan_0001"),
          attempt_id: "coding_attempt_active_group_0001",
          status: "running",
        },
      ],
    });
    vi.stubGlobal("fetch", fetchMock);
    const user = userEvent.setup();
    const onOpenCodingWorkspace = vi.fn();

    render(
      <IssueLifecycleWorkbench onOpenCodingWorkspace={onOpenCodingWorkspace} />,
    );

    await user.click(
      await screen.findByRole("button", { name: "Work Item Group" }),
    );

    expect(
      screen.getByRole("button", { name: "进入 Coding Workspace" }),
    ).toBeInTheDocument();

    await user.click(screen.getByTestId("drawer-open-coding-workspace"));

    await waitFor(() =>
      expect(onOpenCodingWorkspace).toHaveBeenCalledWith(
        {
          projectId: "project_0001",
          issueId: "issue_0001",
          attemptId: "coding_attempt_active_group_0001",
        },
      ),
    );
    expect(fetchMock).not.toHaveBeenCalledWith(
      "/api/projects/project_0001/issues/issue_0001/work-item-plans/issue_plan_0001/coding-attempts",
      expect.objectContaining({ method: "POST" }),
    );
  });

  it("reuses a completed group coding attempt instead of creating another one", async () => {
    const fetchMock = lifecycleFetch({
      confirmedWorkItem: true,
      splitWorkItems: true,
      workItemPlans: [
        issueWorkItemPlanRecord({
          id: "issue_plan_0001",
          status: "confirmed",
          work_item_ids: ["work_item_backend", "work_item_frontend"],
        }),
      ],
      codingAttempts: [
        {
          ...codingGroupAttemptRecord("issue_plan_0001"),
          attempt_id: "coding_attempt_completed_group_0001",
          status: "completed",
        },
      ],
    });
    vi.stubGlobal("fetch", fetchMock);
    const user = userEvent.setup();
    const onOpenCodingWorkspace = vi.fn();

    render(
      <IssueLifecycleWorkbench onOpenCodingWorkspace={onOpenCodingWorkspace} />,
    );

    await user.click(
      await screen.findByRole("button", { name: "Work Item Group" }),
    );

    expect(
      screen.getByRole("button", { name: "进入 Coding Workspace" }),
    ).toBeInTheDocument();

    await user.click(screen.getByTestId("drawer-open-coding-workspace"));

    await waitFor(() =>
      expect(onOpenCodingWorkspace).toHaveBeenCalledWith(
        {
          projectId: "project_0001",
          issueId: "issue_0001",
          attemptId: "coding_attempt_completed_group_0001",
        },
      ),
    );
    expect(fetchMock).not.toHaveBeenCalledWith(
      "/api/projects/project_0001/issues/issue_0001/work-item-plans/issue_plan_0001/coding-attempts",
      expect.objectContaining({ method: "POST" }),
    );
  });

  it.each(["failed", "aborted"] as const)(
    "resolves an existing %s group coding attempt",
    (status) => {
      const attempt = {
        ...codingGroupAttemptRecord("issue_plan_0001"),
        attempt_id: `coding_attempt_${status}_group_0001`,
        status,
      };

      expect(
        resolveGroupCodingAttempt({}, [attempt], "issue_plan_0001"),
      ).toEqual(attempt);
    },
  );

  it("keeps drawer URL focus while opening the work item plan workspace", async () => {
    vi.stubGlobal("fetch", lifecycleFetch({ confirmedWorkItem: true }));
    const user = userEvent.setup();
    const onDrawerFocusChange = vi.fn();
    const onOpenWorkspace = vi.fn();

    render(
      <IssueLifecycleWorkbench
        focusEntityKey="work_item_group:issue_0001:issue_plan_0001"
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
    await user.click(
      await screen.findByRole("button", { name: "选择 Issue 登录会话过期" }),
    );

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
