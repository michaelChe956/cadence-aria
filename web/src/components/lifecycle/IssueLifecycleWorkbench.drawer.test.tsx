import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { IssueLifecycleResponse } from "../../api/types";
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
  jsonResponse,
  lifecycleCardTitle,
  lifecycleFetch,
  projectRecord,
  repositoryRecord,
  workItemRecord,
  workItemRepositoryGroupRecord,
  type LifecycleFetchMock,
  type LifecycleFetchOptions,
} from "./IssueLifecycleWorkbench.test-utils";

vi.mock("../shared/MonacoViewer", () => ({
  MonacoViewer: ({ value, height }: { value: string; height?: string }) => (
    <div data-testid="monaco-viewer" data-height={height}>
      {value}
    </div>
  ),
}));

function repositoryGroupedLifecycleFetch(options: LifecycleFetchOptions = {}) {
  const fetchLifecycle = lifecycleFetch({
    confirmedWorkItem: true,
    splitWorkItems: true,
    ...options,
  });
  return vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const response = await fetchLifecycle(input, init);
    if (String(input) !== "/api/issues/issue_0001/lifecycle?project_id=project_0001") {
      return response;
    }
    const lifecycle = (await response.json()) as IssueLifecycleResponse;
    const workItems = lifecycle.work_items.map((item) => ({
      ...item,
      source_work_item_plan_id: "issue_plan_0001",
    }));
    return jsonResponse({
      ...lifecycle,
      work_items: workItems,
      work_item_repository_groups: workItems.length > 0
        ? [
            workItemRepositoryGroupRecord({
              target_repository_id: "repo_api",
              alias: "api",
              items: workItems.filter((item) => item.kind === "backend"),
            }),
            workItemRepositoryGroupRecord({
              target_repository_id: "repo_web",
              alias: "web",
              items: workItems.filter((item) => item.kind === "frontend"),
            }),
          ]
        : [],
    });
  });
}

// N6（REQ-CS-01/06）：多仓 plan 一键启动夹具——GET 返回聚合（bound revision
// 互证），POST 逐条回放 startResults 并记录出站 body。
function planCodingStartResultRecord(
  overrides: Record<string, unknown> = {},
): Record<string, unknown> {
  return {
    plan_id: "issue_plan_0001",
    bound_plan_revision_id: "plan_revision_0002",
    command_id: "plan_start_abc123",
    operation_state: "accepted",
    primary_attempt_id: "coding_attempt_selected_0001",
    target_attempts: [],
    selected_unit_id: "coding_unit_0001",
    selected_attempt_id: "coding_attempt_selected_0001",
    start_outcome: {
      kind: "started",
      attempt_id: "coding_attempt_selected_0001",
    },
    waiting_units: [],
    progress: {
      plan_id: "issue_plan_0001",
      bound_plan_revision_id: "plan_revision_0002",
      command_id: "plan_start_abc123",
      mode: "first_ready_unit",
      phase: "started",
      primary_attempt_id: "coding_attempt_selected_0001",
      selected_unit_id: "coding_unit_0001",
      selected_attempt_id: "coding_attempt_selected_0001",
      target_attempts: [],
      units: [],
      plan_complete: false,
    },
    ...overrides,
  };
}

function planCodingProgressRecord(
  overrides: Record<string, unknown> = {},
): Record<string, unknown> {
  return {
    plan_id: "issue_plan_0001",
    bound_plan_revision_id: "plan_revision_0002",
    command_id: null,
    mode: null,
    phase: null,
    primary_attempt_id: null,
    selected_unit_id: null,
    selected_attempt_id: null,
    target_attempts: [],
    units: [],
    plan_complete: false,
    ...overrides,
  };
}

function withPlanCodingStartRoutes(
  fetchLifecycle: LifecycleFetchMock,
  options: {
    getResponse?: () => Promise<Response> | Response;
    postResponses?: Array<{ status: number; body: unknown }>;
    onPostBody?: (body: {
      command_id: string;
      expected_plan_revision_id: string;
    }) => void;
  } = {},
) {
  const postResponses = [
    ...(options.postResponses ?? [
      { status: 200, body: planCodingStartResultRecord() },
    ]),
  ];
  return vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input);
    const match = url.match(
      /^\/api\/projects\/([^/]+)\/issues\/([^/]+)\/work-item-plans\/([^/]+)\/coding-start$/u,
    );
    if (!match) {
      return fetchLifecycle(input, init);
    }
    if (init?.method === "POST") {
      options.onPostBody?.(
        JSON.parse(String(init.body)) as {
          command_id: string;
          expected_plan_revision_id: string;
        },
      );
      const next =
        postResponses.shift() ??
        ({ status: 200, body: planCodingStartResultRecord() } as {
          status: number;
          body: unknown;
        });
      return new Response(JSON.stringify(next.body), { status: next.status });
    }
    return options.getResponse
      ? await options.getResponse()
      : jsonResponse(planCodingProgressRecord());
  });
}

const CODING_START_URL =
  "/api/projects/project_0001/issues/issue_0001/work-item-plans/issue_plan_0001/coding-start";

describe("IssueLifecycleWorkbench drawer and work item groups", () => {
  installIssueLifecycleWorkbenchTestHooks();

  it("多仓开始 Coding 先 GET bound revision 再单发 POST 并打开 selected workspace", async () => {
    const postBodies: Array<{ command_id: string; expected_plan_revision_id: string }> = [];
    const fetchMock = withPlanCodingStartRoutes(
      repositoryGroupedLifecycleFetch({
        workItemPlans: [
          issueWorkItemPlanRecord({
            id: "issue_plan_old",
            work_item_ids: [],
            updated_at: "2026-05-19T00:00:00Z",
          }),
          issueWorkItemPlanRecord({
            status: "confirmed",
            work_item_ids: ["work_item_backend", "work_item_frontend"],
            review_status: "completed",
            validator_findings: [{
              finding_id: "finding_plan",
              level: "warning",
              code: "integration_or_e2e_skipped_risk",
              message: "补充贯通验证证据",
              affected_scopes: ["web/**"],
            }],
            dependency_graph: [{
              from_work_item_id: "work_item_backend",
              to_work_item_id: "work_item_frontend",
            }],
          }),
        ],
      }),
      { onPostBody: (body) => postBodies.push(body) },
    );
    vi.stubGlobal("fetch", fetchMock);
    const user = userEvent.setup();
    const onOpenWorkspace = vi.fn();
    const onOpenCodingWorkspace = vi.fn();

    render(
      <IssueLifecycleWorkbench
        onOpenWorkspace={onOpenWorkspace}
        onOpenCodingWorkspace={onOpenCodingWorkspace}
      />,
    );

    const region = await screen.findByRole("region", { name: "Work Item 内容" });
    // owner 直批（2026-10-14）：仓库分组明细默认收起，先展开再断言分组内 Work Item。
    await user.click(
      within(region).getByRole("button", { name: /^展开 2 个仓库分组/u }),
    );
    const planCards = within(region).getAllByRole("button", { name: "Work Item Group" });
    expect(planCards).toHaveLength(1);
    expect(within(region).getAllByTestId(/^lifecycle-card-(?:work_item|work_item_group)$/u)[0])
      .toBe(within(region).getByTestId("lifecycle-card-work_item_group"));
    expect(region).toHaveTextContent("issue_plan_0001");
    expect(region).not.toHaveTextContent("issue_plan_old");
    expect(region).toHaveTextContent("后端 API");
    expect(region).toHaveTextContent("前端 UI");
    expect(within(region).queryByRole("button", { name: /^删除 Work Item Group/u })).toBeNull();
    await user.click(planCards[0]);

    const drawer = screen.getByTestId("lifecycle-card-drawer");
    expect(drawer).toHaveTextContent("work_item_backend");
    expect(drawer).toHaveTextContent("work_item_frontend");
    expect(drawer).toHaveTextContent("story_spec_0001");
    expect(drawer).toHaveTextContent("design_spec_0001");
    expect(drawer).toHaveTextContent("补充贯通验证证据");
    expect(drawer).toHaveTextContent("work_item_backend -> work_item_frontend");
    expect(within(drawer).getByTestId("drawer-review-status")).toHaveTextContent("Review 已完成");
    expect(within(drawer).getByRole("button", { name: "开始 Coding" })).toBeInTheDocument();
    expect(within(drawer).getByRole("button", { name: "删除 Work Item Group" })).toBeInTheDocument();
    await user.click(within(drawer).getByRole("button", { name: "打开 Workspace" }));
    await waitFor(() => expect(onOpenWorkspace).toHaveBeenCalledWith("workspace_session_plan_group_0001"));
    await user.click(screen.getByTestId("drawer-open-coding-workspace"));
    await waitFor(() => expect(onOpenCodingWorkspace).toHaveBeenCalledWith({
      projectId: "project_0001",
      issueId: "issue_0001",
      attemptId: "coding_attempt_selected_0001",
    }));

    // 一键顺序固定：先 GET 聚合取 bound revision，再精确 POST 一次。
    const startCalls = fetchMock.mock.calls.filter(
      ([input]) => String(input) === CODING_START_URL,
    );
    const firstInit = startCalls[0]?.[1] as RequestInit | undefined;
    expect(firstInit?.method).toBeUndefined();
    expect(firstInit?.body).toBeUndefined();
    expect(startCalls[1]?.[1]).toMatchObject({ method: "POST" });
    expect(postBodies).toHaveLength(1);
    expect(postBodies[0]?.command_id).toEqual(expect.any(String));
    expect(postBodies[0]?.expected_plan_revision_id).toBe("plan_revision_0002");
    // 旧 group 创建端点零调用（多仓不走旧 POST）。
    expect(fetchMock).not.toHaveBeenCalledWith(
      "/api/projects/project_0001/issues/issue_0001/work-item-plans/issue_plan_0001/coding-attempts",
      expect.objectContaining({ method: "POST" }),
    );
    expect(fetchMock).not.toHaveBeenCalledWith(
      expect.stringMatching(/\/work-items\/[^/]+\/coding-attempts$/u),
      expect.objectContaining({ method: "POST" }),
    );
  });

  it.each(["running", "completed"] as const)("分仓 Plan 卡复用已有 %s Coding Attempt 而不重复创建", async (status) => {
    const fetchMock = repositoryGroupedLifecycleFetch({
      codingAttempts: [{
        ...codingGroupAttemptRecord("issue_plan_0001"),
        attempt_id: "coding_attempt_existing_group",
        status,
      }],
    });
    vi.stubGlobal("fetch", fetchMock);
    const onOpenCodingWorkspace = vi.fn();
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench onOpenCodingWorkspace={onOpenCodingWorkspace} />);
    await user.click(await screen.findByRole("button", { name: "Work Item Group" }));
    await user.click(screen.getByRole("button", { name: "进入 Coding Workspace" }));
    await waitFor(() => expect(onOpenCodingWorkspace).toHaveBeenCalledWith({
      projectId: "project_0001",
      issueId: "issue_0001",
      attemptId: "coding_attempt_existing_group",
    }));
    expect(fetchMock).not.toHaveBeenCalledWith(
      expect.stringMatching(/\/coding-attempts$/u),
      expect.objectContaining({ method: "POST" }),
    );
    // 旧现 attempt 普通打开零 POST：也不触发新 coding-start GET/POST。
    expect(fetchMock).not.toHaveBeenCalledWith(CODING_START_URL, expect.anything());
  });

  it("多仓开始请求进行中重复点击不产生第二次出站", async () => {
    const getGate = deferred<Response>();
    const postBodies: Array<{ command_id: string; expected_plan_revision_id: string }> = [];
    const fetchMock = withPlanCodingStartRoutes(
      repositoryGroupedLifecycleFetch(),
      {
        getResponse: () => getGate.promise,
        onPostBody: (body) => postBodies.push(body),
      },
    );
    vi.stubGlobal("fetch", fetchMock);
    const user = userEvent.setup();
    const onOpenCodingWorkspace = vi.fn();

    render(<IssueLifecycleWorkbench onOpenCodingWorkspace={onOpenCodingWorkspace} />);
    await user.click(await screen.findByRole("button", { name: "Work Item Group" }));
    const startButton = screen.getByTestId("drawer-open-coding-workspace");
    await user.click(startButton);
    // GET 仍在途时第二次点击必须被忽略。
    await user.click(startButton);

    getGate.resolve(
      new Response(JSON.stringify(planCodingProgressRecord()), { status: 200 }),
    );
    await waitFor(() =>
      expect(onOpenCodingWorkspace).toHaveBeenCalledWith({
        projectId: "project_0001",
        issueId: "issue_0001",
        attemptId: "coding_attempt_selected_0001",
      }),
    );

    const startCalls = fetchMock.mock.calls.filter(
      ([input]) => String(input) === CODING_START_URL,
    );
    expect(startCalls).toHaveLength(2);
    expect(postBodies).toHaveLength(1);
  });

  it("多仓开始失败后重试复用同一 command 并重新互证 revision", async () => {
    const postBodies: Array<{ command_id: string; expected_plan_revision_id: string }> = [];
    const fetchMock = withPlanCodingStartRoutes(
      repositoryGroupedLifecycleFetch(),
      {
        postResponses: [
          { status: 500, body: { code: "internal_error", message: "启动失败", details: {} } },
          { status: 200, body: planCodingStartResultRecord() },
        ],
        onPostBody: (body) => postBodies.push(body),
      },
    );
    vi.stubGlobal("fetch", fetchMock);
    const user = userEvent.setup();
    const onOpenCodingWorkspace = vi.fn();

    render(<IssueLifecycleWorkbench onOpenCodingWorkspace={onOpenCodingWorkspace} />);
    await user.click(await screen.findByRole("button", { name: "Work Item Group" }));
    await user.click(screen.getByTestId("drawer-open-coding-workspace"));

    // 首次启动失败：错误可见且未打开工作台。
    expect(await screen.findByRole("alert")).toHaveTextContent("启动失败");
    expect(onOpenCodingWorkspace).not.toHaveBeenCalled();

    // 同一启动动作重试：GET 重新互证 revision，POST 复用同一 command_id。
    await user.click(screen.getByTestId("drawer-open-coding-workspace"));
    await waitFor(() =>
      expect(onOpenCodingWorkspace).toHaveBeenCalledWith({
        projectId: "project_0001",
        issueId: "issue_0001",
        attemptId: "coding_attempt_selected_0001",
      }),
    );

    expect(postBodies).toHaveLength(2);
    expect(postBodies[1]?.command_id).toBe(postBodies[0]?.command_id);
    expect(postBodies[0]?.expected_plan_revision_id).toBe("plan_revision_0002");
    expect(postBodies[1]?.expected_plan_revision_id).toBe("plan_revision_0002");
  });

  it("分仓 Plan 抽屉删除整组后关闭并恢复重新生成入口", async () => {
    const fetchMock = repositoryGroupedLifecycleFetch();
    vi.stubGlobal("fetch", fetchMock);
    const confirmSpy = vi.spyOn(window, "confirm").mockReturnValue(true);
    const user = userEvent.setup();
    const onOpenWorkspace = vi.fn();

    render(<IssueLifecycleWorkbench onOpenWorkspace={onOpenWorkspace} />);
    await user.click(await screen.findByRole("button", { name: "Work Item Group" }));
    await user.click(within(screen.getByTestId("lifecycle-card-drawer")).getByRole("button", {
      name: "删除 Work Item Group",
    }));

    // D2.3 机械迁移：group drawer 旧无条件预确认移除——删除一次直达（本夹具
    // 200=Draft 语义），确认流由服务端三分类给出（见 crud
    // cascade_d2_3_single_confirmation）；不再有「先取消再确认」两段式。
    expect(confirmSpy).toHaveBeenCalledTimes(0);
    expect(fetchMock).toHaveBeenCalledWith(
      "/api/projects/project_0001/issues/issue_0001/work-item-plans/issue_plan_0001",
      expect.objectContaining({ method: "DELETE" }),
    );
    expect(await screen.findByRole("button", { name: "准备 Work Item Plan" })).toBeInTheDocument();
    expect(screen.queryByTestId("lifecycle-card-drawer")).toBeNull();
    expect(screen.queryByRole("button", { name: "Work Item Group" })).toBeNull();
    expect(screen.queryByRole("button", { name: "后端 API" })).toBeNull();
    expect(screen.queryByRole("button", { name: "前端 UI" })).toBeNull();
    expect(useLifecycleWorkbenchStore.getState()).toMatchObject({
      isDrawerOpen: false,
      focusedEntityKey: null,
    });
    await user.click(screen.getByRole("button", { name: "准备 Work Item Plan" }));
    const dialog = await screen.findByRole("dialog", { name: "Work Item Plan 配置" });
    await user.click(within(dialog).getByRole("button", { name: "创建并打开 Workspace" }));
    await waitFor(() => expect(onOpenWorkspace).toHaveBeenCalledWith("workspace_session_plan_group_0001"));
    expect(fetchMock).toHaveBeenCalledWith(
      "/api/projects/project_0001/issues/issue_0001/work-item-plans:prepare",
      expect.objectContaining({ method: "POST" }),
    );
    expect(await screen.findByRole("button", { name: "Work Item Group" })).toBeInTheDocument();
    confirmSpy.mockRestore();
  });

  it.each([
    ["目标仓库", "repo_target_0001", false],
    ["兼容投影", null, true],
  ] as const)("点击%s分组中的 Work Item 后打开抽屉并保持刷新后的焦点", async (_label, repositoryId, compatibilityProjection) => {
    vi.stubGlobal(
      "fetch",
      lifecycleFetch({
        confirmedWorkItem: true,
        workItemRepositoryGroups: [
          workItemRepositoryGroupRecord({
            target_repository_id: repositoryId,
            compatibility_projection: compatibilityProjection,
            items: [workItemRecord({ plan_status: "confirmed" })],
          }),
        ],
      }),
    );
    const user = userEvent.setup();
    const onDrawerFocusChange = vi.fn();

    render(
      <IssueLifecycleWorkbench
        focusEntityKey={null}
        onDrawerFocusChange={onDrawerFocusChange}
      />,
    );

    await user.click(
      await screen.findByRole("button", { name: "选择 Issue 登录会话过期" }),
    );
    // owner 直批（2026-10-14）：仓库分组明细默认收起，先展开再点击分组内 Work Item。
    await user.click(
      screen.getByRole("button", { name: /^展开 1 个仓库分组/u }),
    );
    await user.click(screen.getByRole("button", { name: "实现提示组件" }));

    // 区分开关被重置与实体查找失败：开关和 URL 焦点正确仍必须渲染抽屉。
    expect(useLifecycleWorkbenchStore.getState()).toMatchObject({
      isDrawerOpen: true,
      focusedEntityKey: "work_item:issue_0001:work_item_0001",
    });
    expect(onDrawerFocusChange).toHaveBeenLastCalledWith(
      "work_item:issue_0001:work_item_0001",
    );
    expect(screen.getByTestId("lifecycle-card-drawer")).toHaveTextContent(
      "实现提示组件",
    );
    expect(screen.getByTestId("workbench-shell")).toHaveClass(
      "lg:pr-[calc(480px+1rem)]",
    );

    await user.click(screen.getByRole("button", { name: "刷新" }));
    expect(await screen.findByTestId("lifecycle-card-drawer")).toHaveTextContent(
      "实现提示组件",
    );
    await user.click(within(screen.getByTestId("lifecycle-card-drawer")).getByRole("button", { name: /^关闭$/u }));
    expect(screen.queryByTestId("lifecycle-card-drawer")).not.toBeInTheDocument();
    expect(onDrawerFocusChange).toHaveBeenLastCalledWith(null);
  });

  it("从 Work Item 深链接恢复抽屉且不串到相同局部 ID 的其他 Issue", async () => {
    vi.stubGlobal("fetch", lifecycleFetch({ sharedLifecycleIdsAcrossIssues: true }));
    const user = userEvent.setup();
    const onOpenWorkspace = vi.fn();

    render(
      <IssueLifecycleWorkbench
        focusEntityKey="work_item:issue_0002:work_item_0001"
        onOpenWorkspace={onOpenWorkspace}
      />,
    );

    const drawer = await screen.findByTestId("lifecycle-card-drawer");
    expect(drawer).toHaveTextContent("issue_0002 Child");
    expect(drawer).not.toHaveTextContent("issue_0001 Child");
    await user.click(screen.getByTestId("drawer-open-workspace"));
    await waitFor(() =>
      expect(onOpenWorkspace).toHaveBeenCalledWith(
        "workspace_session_issue_0002_work_item",
      ),
    );
  });

  it("刷新时重解析 Work Item，缺项时保持现有空抽屉语义", async () => {
    const fetchMock = lifecycleFetch({ confirmedWorkItem: true });
    let responseState: "initial" | "updated" | "missing" = "initial";
    vi.stubGlobal("fetch", async (input: RequestInfo | URL, init?: RequestInit) => {
      const response = await fetchMock(input, init);
      if (String(input) !== "/api/issues/issue_0001/lifecycle?project_id=project_0001") {
        return response;
      }
      const lifecycle = (await response.json()) as IssueLifecycleResponse;
      if (responseState === "updated") {
        lifecycle.work_items[0].title = "刷新后的任务详情";
      } else if (responseState === "missing") {
        lifecycle.work_items = [];
      }
      return new Response(JSON.stringify(lifecycle), {
        headers: { "Content-Type": "application/json" },
      });
    });
    const user = userEvent.setup();

    render(
      <IssueLifecycleWorkbench focusEntityKey="work_item:issue_0001:work_item_0001" />,
    );
    expect(await screen.findByTestId("lifecycle-card-drawer")).toHaveTextContent(
      "实现提示组件",
    );

    responseState = "updated";
    await user.click(screen.getByRole("button", { name: "刷新" }));
    expect(await within(screen.getByTestId("lifecycle-card-drawer")).findByRole(
      "heading", { name: "刷新后的任务详情" },
    )).toBeInTheDocument();

    responseState = "missing";
    await user.click(screen.getByRole("button", { name: "刷新" }));
    await waitFor(() =>
      expect(screen.queryByTestId("lifecycle-card-drawer")).not.toBeInTheDocument(),
    );
    expect(useLifecycleWorkbenchStore.getState()).toMatchObject({
      isDrawerOpen: true,
      focusedEntityKey: "work_item:issue_0001:work_item_0001",
    });
  });

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
    expect(workItemRegion).toHaveTextContent("已确认");
    expect(within(workItemRegion).queryByRole("button", { name: /^删除 Work Item Group/u })).toBeNull();

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

    // D2.3 机械迁移：drawer 旧无条件确认移除，确认语义由服务端三分类权威。
    expect(confirmSpy).toHaveBeenCalledTimes(0);
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
    // v1 旧动作不变：无分仓投影的单仓 group 仍走旧创建端点，零 coding-start 出站。
    expect(fetchMock).not.toHaveBeenCalledWith(CODING_START_URL, expect.anything());
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
