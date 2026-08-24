import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type {
  AggregateIndexActiveResponse,
  PointerPublicationDto,
  RegistrationBatchDto,
  RegistrationPreflightResponse,
} from "../../api/types";
import { useLifecycleWorkbenchStore } from "../../state/lifecycle-workbench-store";
import { defaultCollapsedGroups } from "./issue-queue-derivation";
import {
  defaultLaunchTitle,
  IssueLifecycleWorkbench,
} from "./IssueLifecycleWorkbench";
import {
  deferred,
  installIssueLifecycleWorkbenchTestHooks,
  issueWorkItemPlanRecord,
  jsonResponse,
  jsonResponseValue,
  lifecycleCardTitle,
  lifecycleFetch,
  projectRecord,
  projectsBody,
  repositoryRecord,
} from "./IssueLifecycleWorkbench.test-utils";

vi.mock("../shared/MonacoViewer", () => ({
  MonacoViewer: ({ value, height }: { value: string; height?: string }) => (
    <div data-testid="monaco-viewer" data-height={height}>
      {value}
    </div>
  ),
}));

function aggregateIndex(
  overrides: Partial<AggregateIndexActiveResponse> = {},
): AggregateIndexActiveResponse {
  return {
    state: "active",
    revision: 7,
    indexed_at: "2026-08-18T00:00:00Z",
    warning: null,
    ...overrides,
  };
}

function pointerPublication(
  overrides: Partial<PointerPublicationDto> = {},
): PointerPublicationDto {
  return {
    id: "publication_0001",
    project_id: "project_0001",
    logical_codebase_id: "logical_0001",
    batch_kind: "full",
    entries: [],
    status: "completed_all",
    created_at: "2026-08-14T00:00:00Z",
    updated_at: "2026-08-14T00:00:00Z",
    ...overrides,
  };
}

// Task 7：批量 Issue fixture（全部无 Story -> 单一 needs_story 组），用于验证
// 「显示更多」跨越 deriveIssueQueue 默认 perGroupLimit（50）后真正追加渲染。
function bulkIssuesFetch(titles: string[]) {
  const issues = titles.map((title, index) => ({
    issue_id: `issue_${String(index + 1).padStart(4, "0")}`,
    project_id: "project_0001",
    repo_id: "repository_0001",
    workspace_id: null,
    task_id: null,
    session_id: null,
    title,
    description: "描述",
    change_id: null,
    phase: "clarification",
    status: "draft",
    active_binding_id: null,
    artifacts: [],
    created_at: "2026-05-16T00:00:00Z",
    updated_at: "2026-05-16T00:00:00Z",
  }));

  return vi.fn(async (input: RequestInfo | URL) => {
    const url = String(input);
    if (url === "/api/projects") {
      return jsonResponse({ projects: [projectRecord("project_0001", "Aria")] });
    }
    if (url === "/api/projects/project_0001/repositories") {
      return jsonResponse({ repositories: [repositoryRecord()] });
    }
    if (url === "/api/projects/project_0001/codebases") {
      return jsonResponse({ codebases: [] });
    }
    if (url === "/api/projects/project_0001/issues") {
      return jsonResponse({ issues });
    }
    const lifecycleMatch = url.match(
      /^\/api\/issues\/([^/]+)\/lifecycle\?project_id=([^&]+)$/,
    );
    if (lifecycleMatch) {
      const issue = issues.find(
        (candidate) => candidate.issue_id === lifecycleMatch[1],
      );
      return jsonResponse({
        issue,
        story_specs: [],
        design_specs: [],
        work_item_plans: [],
        work_items: [],
        work_item_repository_groups: [],
        workspace_sessions: [],
        coding_attempts: [],
      });
    }
    return jsonResponse({});
  });
}

describe("IssueLifecycleWorkbench base workflow", () => {
  installIssueLifecycleWorkbenchTestHooks();

  it("loads the aggregate index, disables rebuild while pending, replaces it on success, and shows API errors", async () => {
    const rebuildingResponse = deferred<Response>();
    const fetchMock = lifecycleFetch({
      projects: [projectRecord("project_0001", "Aria")],
      logicalCodebases: [{ id: "logical-0001", name: "平台" }],
      aggregateIndex: aggregateIndex({ state: "missing", revision: null, indexed_at: null }),
      aggregateIndexRebuild: aggregateIndex({
        state: "active",
        revision: 8,
        indexed_at: "2026-08-18T01:00:00Z",
      }),
    });
    vi.stubGlobal(
      "fetch",
      vi.fn((input: RequestInfo | URL, init?: RequestInit) => {
        if (
          String(input) ===
            "/api/projects/project_0001/logical-codebases/logical-0001/aggregate-indexes/rebuild" &&
          init?.method === "POST"
        ) {
          return rebuildingResponse.promise;
        }
        return fetchMock(input, init);
      }),
    );
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);

    // Task 8：运维面板默认折叠为摘要条，先点「管理」展开。
    await user.click(await screen.findByTestId("lc-summary-toggle"));

    expect(await screen.findByTestId("aggregate-index-status")).toHaveTextContent(
      "missing",
    );
    await user.click(screen.getByRole("button", { name: "重建索引" }));
    expect(screen.getByRole("button", { name: "重建索引" })).toBeDisabled();
    expect(screen.getByTestId("aggregate-index-spinner")).toBeInTheDocument();

    await act(async () => {
      rebuildingResponse.resolve(
        new Response(JSON.stringify(aggregateIndex({ state: "active", revision: 8 }))),
      );
    });

    await waitFor(() =>
      expect(screen.getByTestId("aggregate-index-status")).toHaveTextContent(
        "active",
      ),
    );
    expect(screen.getByText(/成员版本：8/)).toBeInTheDocument();
  });

  it("shows ApiRequestError.message when aggregate index rebuild fails", async () => {
    const fetchMock = lifecycleFetch({
      projects: [projectRecord("project_0001", "Aria")],
      logicalCodebases: [{ id: "logical-0001", name: "平台" }],
      aggregateIndex: aggregateIndex({ state: "stale" }),
    });
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        if (
          String(input) ===
            "/api/projects/project_0001/logical-codebases/logical-0001/aggregate-indexes/rebuild" &&
          init?.method === "POST"
        ) {
          return new Response(
            JSON.stringify({
              code: "aggregate_index_unavailable",
              message: "sync failed",
            }),
            { status: 422 },
          );
        }
        return fetchMock(input, init);
      }),
    );
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);

    // Task 8：先展开运维面板才能触及「重建索引」。
    await user.click(await screen.findByTestId("lc-summary-toggle"));
    await user.click(await screen.findByRole("button", { name: "重建索引" }));

    expect(await screen.findByText("sync failed")).toBeInTheDocument();
  });

  it("wires pointer publication actions to APIs and upserts each returned publication", async () => {
    const initialPublication = pointerPublication({
      id: "publication_0000",
      status: "completed_partial",
      entries: [
        {
          member_repo_id: "repository_0001",
          state: "failed",
          branch_name: null,
          commit_sha: null,
          push_error: "remote rejected",
          conflict_detail: null,
        },
      ],
    });
    const fetchMock = lifecycleFetch({
      logicalCodebases: [{ id: "logical-0001", name: "平台" }],
      pointerPublications: [initialPublication],
    });
    vi.stubGlobal("fetch", fetchMock);
    vi.spyOn(window, "confirm").mockReturnValue(true);
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);

    // Task 8：先展开运维面板。
    await user.click(await screen.findByTestId("lc-summary-toggle"));
    // R8：LC 作用域数据（成员/发布）异步加载，先等面板数据就绪再操作。
    await screen.findByTestId("pointer-publication-badge");
    await user.click(screen.getByRole("button", { name: "全量发布" }));
    await waitFor(() =>
      expect(fetchMock).toHaveBeenCalledWith(
        "/api/projects/project_0001/logical-codebases/logical-0001/pointer-publications",
        expect.objectContaining({
          method: "POST",
          body: JSON.stringify({ batch_kind: "full" }),
        }),
      ),
    );
    expect(screen.getByTestId("pointer-publication-badge")).toHaveAttribute(
      "data-status",
      "completed_partial",
    );
    expect(
      screen.getByText("aria-pointer/repository_0001/full"),
    ).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "增量发布" }));
    await waitFor(() =>
      expect(fetchMock).toHaveBeenCalledWith(
        "/api/projects/project_0001/logical-codebases/logical-0001/pointer-publications",
        expect.objectContaining({
          method: "POST",
          body: JSON.stringify({ batch_kind: "incremental" }),
        }),
      ),
    );

    await user.click(screen.getByRole("button", { name: "重试" }));
    await waitFor(() =>
      expect(fetchMock).toHaveBeenCalledWith(
        "/api/projects/project_0001/logical-codebases/logical-0001/pointer-publications/publication_0000/retry-repo",
        expect.objectContaining({
          method: "POST",
          body: JSON.stringify({ member_repo_id: "repository_0001" }),
        }),
      ),
    );
    expect(screen.getByText("aria-pointer/repository_0001/retried")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "撤回" }));
    await waitFor(() =>
      expect(fetchMock).toHaveBeenCalledWith(
        "/api/projects/project_0001/logical-codebases/logical-0001/pointer-publications/publication_0000/revoke",
        expect.objectContaining({
          method: "POST",
          body: JSON.stringify({}),
        }),
      ),
    );
    expect(screen.getAllByText("已撤回").length).toBeGreaterThan(0);
  });

  it("shows the new-members hint and publishes incrementally when member count increased", async () => {
    const initialPublication = pointerPublication({
      entries: [
        {
          member_repo_id: "repository_0001",
          state: "pushed",
          branch_name: null,
          commit_sha: null,
          push_error: null,
          conflict_detail: null,
        },
      ],
    });
    const fetchMock = lifecycleFetch({
      logicalCodebases: [{ id: "logical-0001", name: "平台" }],
      pointerPublications: [initialPublication],
      logicalCodebaseMembers: [
        {
          logical_repository_id: "repository_0001",
          alias: "api",
          status: "active",
        },
        {
          logical_repository_id: "repository_0002",
          alias: "web",
          status: "active",
        },
      ],
    });
    vi.stubGlobal("fetch", fetchMock);
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);

    // Task 8：先展开运维面板。
    await user.click(await screen.findByTestId("lc-summary-toggle"));
    const hint = await screen.findByTestId(
      "pointer-publication-new-members-hint",
    );
    expect(hint).toHaveTextContent("检测到新增成员，建议增量发布");
    await user.click(within(hint).getByRole("button", { name: "增量发布" }));
    await waitFor(() =>
      expect(fetchMock).toHaveBeenCalledWith(
        "/api/projects/project_0001/logical-codebases/logical-0001/pointer-publications",
        expect.objectContaining({
          method: "POST",
          body: JSON.stringify({ batch_kind: "incremental" }),
        }),
      ),
    );
  });

  it("does not show the new-members hint when member count does not exceed entries", async () => {
    vi.stubGlobal(
      "fetch",
      lifecycleFetch({
        // hotfix：面板仅在存在逻辑条目时渲染，补一条 LC 使指针发布面板可见
        logicalCodebases: [
          { id: "lc_0001", name: "monorepo", member_count: 1 },
        ],
        pointerPublications: [
          pointerPublication({
            entries: [
              {
                member_repo_id: "repository_0001",
                state: "pushed",
                branch_name: null,
                commit_sha: null,
                push_error: null,
                conflict_detail: null,
              },
            ],
          }),
        ],
        logicalCodebaseMembers: [
          {
            logical_repository_id: "repository_0001",
            alias: "api",
            status: "active",
          },
        ],
      }),
    );
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);

    // Task 8：先展开运维面板。
    await user.click(await screen.findByTestId("lc-summary-toggle"));
    await screen.findByTestId("pointer-publication-panel");
    expect(
      screen.queryByTestId("pointer-publication-new-members-hint"),
    ).not.toBeInTheDocument();
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

    // Task 7：队列由 IssueQueue 承载，行选择按钮名为「选择 Issue <标题>」。
    await user.click(
      screen.getByRole("button", { name: "选择 Issue 登录会话过期" }),
    );

    expect(
      screen.getByRole("region", { name: "Issue 生命周期详情" }),
    ).toBeInTheDocument();
    // Task 6：单阶段面板一次只渲染当前阶段区域；story/design 均有产物 -> 默认 work_item。
    expect(screen.getByTestId("stage-tab-work_item")).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(
      screen.getByRole("region", { name: "Work Item 内容" }),
    ).toHaveTextContent("Work Item Group");
    expect(
      screen.getByRole("region", { name: "Work Item 内容" }),
    ).not.toHaveTextContent("实现提示组件");
    expect(
      screen.queryByRole("region", { name: "Story Spec 内容" }),
    ).not.toBeInTheDocument();

    await user.click(screen.getByTestId("stage-tab-story"));
    expect(
      screen.getByRole("region", { name: "Story Spec 内容" }),
    ).toHaveTextContent("会话过期提示");

    await user.click(screen.getByTestId("stage-tab-design"));
    expect(
      screen.getByRole("region", { name: "Design Spec 内容" }),
    ).toHaveTextContent("前端提示设计");
  });

  it("generates a story spec from the empty story stage panel action", async () => {
    const fetchMock = lifecycleFetch({ emptyLifecycle: true });
    vi.stubGlobal("fetch", fetchMock);
    const user = userEvent.setup();
    const onOpenWorkspace = vi.fn();

    render(<IssueLifecycleWorkbench onOpenWorkspace={onOpenWorkspace} />);

    await screen.findByTestId("stage-stepper");
    // 全空生命周期 -> 默认 story 阶段，空面板提供生成入口。
    expect(screen.getByTestId("stage-tab-story")).toHaveAttribute(
      "aria-selected",
      "true",
    );
    const storyRegion = screen.getByRole("region", {
      name: "Story Spec 内容",
    });
    await user.click(
      within(storyRegion).getByRole("button", { name: "生成 Story Spec" }),
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
    expect(onOpenWorkspace).toHaveBeenCalledWith(
      "workspace_session_story_0001",
    );
  });

  it("generates a design spec from the empty design stage panel via the latest story card", async () => {
    const fetchMock = lifecycleFetch({ emptyDesignSpecs: true });
    vi.stubGlobal("fetch", fetchMock);
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);

    await screen.findByTestId("stage-stepper");
    // 有 story 无 design -> 默认 design 阶段。
    expect(screen.getByTestId("stage-tab-design")).toHaveAttribute(
      "aria-selected",
      "true",
    );
    const designRegion = screen.getByRole("region", {
      name: "Design Spec 内容",
    });
    await user.click(
      within(designRegion).getByRole("button", { name: "生成 Design Spec" }),
    );

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
    // 生成后 design 阶段不再为空：主按钮消失、新卡片可见。
    await waitFor(() =>
      expect(designRegion).toHaveTextContent("会话过期提示 Design Spec"),
    );
    expect(
      within(designRegion).queryByRole("button", {
        name: "生成 Design Spec",
      }),
    ).not.toBeInTheDocument();
  });

  it("opens the work item plan options from the empty work item stage panel via the latest design card", async () => {
    const fetchMock = lifecycleFetch({ workItemPlans: [] });
    vi.stubGlobal("fetch", fetchMock);
    const user = userEvent.setup();
    const onOpenWorkspace = vi.fn();

    render(<IssueLifecycleWorkbench onOpenWorkspace={onOpenWorkspace} />);

    await screen.findByTestId("stage-stepper");
    // story/design 均有产物且无 plan -> 默认 work_item 阶段，空面板提供准备入口。
    expect(screen.getByTestId("stage-tab-work_item")).toHaveAttribute(
      "aria-selected",
      "true",
    );
    const workItemRegion = screen.getByRole("region", {
      name: "Work Item 内容",
    });
    await user.click(
      within(workItemRegion).getByRole("button", {
        name: "准备 Work Item Plan",
      }),
    );

    const dialog = await screen.findByRole("dialog", {
      name: "Work Item Plan 配置",
    });
    await user.click(
      within(dialog).getByRole("button", { name: "创建并打开 Workspace" }),
    );

    const prepareCall = fetchMock.mock.calls.find(([url]) =>
      String(url).includes("/work-item-plans:prepare"),
    );
    expect(prepareCall).toBeDefined();
    expect(JSON.parse(String(prepareCall?.[1]?.body))).toMatchObject({
      title: "前端提示设计 Work Item",
      story_spec_ids: ["story_spec_0001"],
      design_spec_ids: ["design_spec_0001"],
    });
  });

  it("keeps the issue card highlighted while a derived story card is selected", async () => {
    vi.stubGlobal("fetch", lifecycleFetch());
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);

    await user.click(
      await screen.findByRole("button", { name: "选择 Issue 登录会话过期" }),
    );
    // Task 6：story 卡片在 story 阶段页内可见。
    await user.click(screen.getByTestId("stage-tab-story"));
    await user.click(screen.getByRole("button", { name: "会话过期提示" }));

    // Task 7：焦点高亮的载体由 LifecycleCard 换为 IssueQueueRow（aria-current）。
    const focusedRow = screen
      .getAllByTestId("issue-queue-row")
      .find((row) => row.getAttribute("data-issue-id") === "issue_0001");
    expect(focusedRow).toHaveAttribute("aria-current", "true");
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
      await screen.findByRole("button", { name: "选择 Issue 登录会话过期" }),
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
        focusEntityKey="story_spec:issue_0001:story_spec_0001"
        onDrawerFocusChange={onDrawerFocusChange}
      />,
    );

    await waitFor(() =>
      expect(useLifecycleWorkbenchStore.getState().focusedEntityKey).toBe(
        "story_spec:issue_0001:story_spec_0001",
      ),
    );
    expect(useLifecycleWorkbenchStore.getState().isDrawerOpen).toBe(true);
    await waitFor(() =>
      expect(onDrawerFocusChange).toHaveBeenCalledWith(
        "story_spec:issue_0001:story_spec_0001",
      ),
    );

    view.rerender(
      <IssueLifecycleWorkbench
        focusEntityKey={null}
        onDrawerFocusChange={onDrawerFocusChange}
      />,
    );

    await waitFor(() =>
      expect(useLifecycleWorkbenchStore.getState().focusedEntityKey).toBeNull(),
    );
    expect(useLifecycleWorkbenchStore.getState().isDrawerOpen).toBe(false);

    act(() => {
      useLifecycleWorkbenchStore
        .getState()
        .openDrawer("design_spec:issue_0001:design_spec_0001");
    });

    await waitFor(() =>
      expect(onDrawerFocusChange).toHaveBeenCalledWith(
        "design_spec:issue_0001:design_spec_0001",
      ),
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
      await screen.findByRole("button", { name: "选择 Issue 登录会话过期" }),
    ).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Mobile" }));

    expect(
      await screen.findByRole("button", { name: "选择 Issue 移动端刷新" }),
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

  it("opens the member registration wizard from the logical codebase panel", async () => {
    const preflight: RegistrationPreflightResponse = {
      preflight_id: "preflight_0001",
      created_at: "2026-08-18T00:00:00Z",
      items: [{ path: "/root/api", class: "eligible", reason: null }],
    };
    const batch: RegistrationBatchDto = {
      batch_id: "batch_0001",
      status: "completed",
      items: [{ path: "/root/api", status: "completed", failure_reason: null }],
    };
    vi.stubGlobal(
      "fetch",
      lifecycleFetch({
        logicalCodebases: [{ id: "lc_0001", name: "monorepo" }],
        registrationPreflight: preflight,
        registrationSubmit: batch,
      }),
    );
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);
    // Task 8：「登记成员」位于运维面板内，先点摘要条展开。
    await user.click(await screen.findByTestId("lc-summary-toggle"));
    await user.click(await screen.findByRole("button", { name: "登记成员" }));
    await user.type(screen.getByLabelText("聚合根目录"), "/root");
    await user.click(screen.getByRole("button", { name: "确认聚合根并自动发现" }));
    await user.click(await screen.findByRole("button", { name: "提交登记" }));

    expect(await screen.findByText("completed")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "关闭" })).toBeInTheDocument();
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

// Task 7：外壳双密度 + 队列折叠 + 接线。队列区 w-72 shrink-0，折叠为 w-10 细轨；
// 折叠态与分组折叠态按 projectId 记忆并持久化 localStorage；折叠/展开不改变
// focusedIssueId / selectedCardKey，也不触发任何网络请求。
describe("IssueLifecycleWorkbench 队列折叠双密度 (Task 7)", () => {
  installIssueLifecycleWorkbenchTestHooks();

  function queueRegion() {
    return screen.getByRole("region", { name: "Issue 卡片列表" });
  }

  // 折叠按钮在队列列内、region 之外（IssueQueue 契约不含折叠控件）。
  function queueColumn() {
    return screen.getByTestId("issue-queue-column");
  }

  it("外壳与队列列宽满足双密度规格，队列与工作区各自可滚动", async () => {
    vi.stubGlobal("fetch", lifecycleFetch());

    render(<IssueLifecycleWorkbench />);
    await screen.findByRole("button", { name: "选择 Issue 登录会话过期" });

    expect(screen.getByTestId("workbench-shell")).toHaveClass("h-[100dvh]");
    const column = screen.getByTestId("issue-queue-column");
    expect(column).toHaveClass("w-72", "shrink-0");
    expect(screen.getByTestId("issue-queue-group-list")).toHaveClass(
      "min-h-0",
      "overflow-y-auto",
    );
  });

  it("折叠队列显示细轨（含展开按钮与计数）且工作区仍在，展开后恢复队列", async () => {
    vi.stubGlobal("fetch", lifecycleFetch());
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);
    await screen.findByRole("button", { name: "选择 Issue 登录会话过期" });

    await user.click(
      within(queueColumn()).getByRole("button", { name: "折叠 Issue 队列" }),
    );

    const rail = screen.getByTestId("issue-queue-collapsed-rail");
    expect(rail).toHaveClass("w-10", "shrink-0");
    expect(
      within(rail).getByRole("button", { name: "展开 Issue 队列" }),
    ).toBeInTheDocument();
    expect(within(rail).getByTestId("issue-queue-rail-count")).toHaveTextContent(
      "1",
    );
    expect(
      screen.queryByRole("region", { name: "Issue 卡片列表" }),
    ).not.toBeInTheDocument();
    // 工作区仍在（专注密度只切换队列，不影响工作区）。
    expect(
      screen.getByRole("region", { name: "Issue 生命周期详情" }),
    ).toBeInTheDocument();
    expect(screen.getByTestId("stage-stepper")).toBeInTheDocument();

    await user.click(
      within(rail).getByRole("button", { name: "展开 Issue 队列" }),
    );

    expect(
      screen.getByRole("region", { name: "Issue 卡片列表" }),
    ).toBeInTheDocument();
    expect(screen.queryByTestId("issue-queue-collapsed-rail")).toBeNull();
  });

  it("折叠状态按 projectId 写入 localStorage 并在重挂载后恢复", async () => {
    vi.stubGlobal("fetch", lifecycleFetch());
    const user = userEvent.setup();

    const view = render(<IssueLifecycleWorkbench />);
    await screen.findByRole("button", { name: "选择 Issue 登录会话过期" });

    await user.click(
      within(queueColumn()).getByRole("button", { name: "折叠 Issue 队列" }),
    );
    expect(
      window.localStorage.getItem("aria.workbench.queueCollapsed.project_0001"),
    ).toBe("1");

    view.unmount();
    render(<IssueLifecycleWorkbench />);

    expect(
      await screen.findByTestId("issue-queue-collapsed-rail"),
    ).toBeInTheDocument();

    await user.click(
      screen.getByRole("button", { name: "展开 Issue 队列" }),
    );
    expect(
      window.localStorage.getItem("aria.workbench.queueCollapsed.project_0001"),
    ).toBe("0");
  });

  it("折叠状态按 Project 相互独立", async () => {
    vi.stubGlobal(
      "fetch",
      lifecycleFetch({
        projects: [
          projectRecord("project_0001", "Aria"),
          projectRecord("project_0002", "Mobile"),
        ],
        issueTitlesByProject: {
          project_0001: "登录会话过期",
          project_0002: "移动端刷新",
        },
      }),
    );
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);
    await screen.findByRole("button", { name: "选择 Issue 登录会话过期" });

    await user.click(
      within(queueColumn()).getByRole("button", { name: "折叠 Issue 队列" }),
    );
    expect(screen.getByTestId("issue-queue-collapsed-rail")).toBeInTheDocument();

    // 切到 project_0002：该 Project 未折叠，队列可见。
    await user.click(screen.getByRole("button", { name: "Mobile" }));
    await screen.findByRole("button", { name: "选择 Issue 移动端刷新" });
    expect(screen.queryByTestId("issue-queue-collapsed-rail")).toBeNull();

    // 切回 project_0001：恢复其折叠态。
    await user.click(screen.getByRole("button", { name: "Aria" }));
    expect(
      await screen.findByTestId("issue-queue-collapsed-rail"),
    ).toBeInTheDocument();
    expect(
      window.localStorage.getItem("aria.workbench.queueCollapsed.project_0002"),
    ).not.toBe("1");
  });

  it("折叠与展开不改变聚焦 Issue、不改变选中实体、不触发网络请求", async () => {
    const fetchMock = lifecycleFetch();
    vi.stubGlobal("fetch", fetchMock);
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);
    await user.click(
      await screen.findByRole("button", { name: "选择 Issue 登录会话过期" }),
    );
    await user.click(screen.getByTestId("stage-tab-story"));
    await user.click(screen.getByRole("button", { name: "会话过期提示" }));
    await waitFor(() =>
      expect(useLifecycleWorkbenchStore.getState().focusedEntityKey).toBe(
        "story_spec:issue_0001:story_spec_0001",
      ),
    );
    const callsBefore = fetchMock.mock.calls.length;

    await user.click(
      within(queueColumn()).getByRole("button", { name: "折叠 Issue 队列" }),
    );
    await user.click(
      screen.getByRole("button", { name: "展开 Issue 队列" }),
    );

    expect(fetchMock.mock.calls).toHaveLength(callsBefore);
    // 聚焦 Issue 未变：其行仍高亮（aria-current）。
    const focusedRow = screen
      .getAllByTestId("issue-queue-row")
      .find((row) => row.getAttribute("data-issue-id") === "issue_0001");
    expect(focusedRow).toHaveAttribute("aria-current", "true");
    // 选中实体未变：drawer 仍聚焦同一 Story Spec。
    expect(useLifecycleWorkbenchStore.getState().focusedEntityKey).toBe(
      "story_spec:issue_0001:story_spec_0001",
    );
  });

  it("分组折叠默认为 defaultCollapsedGroups 并按 projectId 持久化", async () => {
    vi.stubGlobal("fetch", lifecycleFetch());
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);
    await screen.findByRole("button", { name: "选择 Issue 登录会话过期" });

    // 缺省折叠组 = defaultCollapsedGroups()（["completed"]）。
    for (const key of defaultCollapsedGroups()) {
      const header = screen
        .queryAllByTestId("issue-queue-group-header")
        .find((node) => node.getAttribute("data-group-key") === key);
      if (header) {
        expect(header).toHaveAttribute("aria-expanded", "false");
      }
    }

    // fixture 的唯一 Issue 落在 needs_work_item 组（story/design 有产物、无 work item plan 时）
    // 或 coding/blocked 组；取当前渲染出的第一个组头折叠它并断言持久化。
    const header = screen.getAllByTestId("issue-queue-group-header")[0];
    const groupKey = header.getAttribute("data-group-key");
    expect(groupKey).not.toBeNull();
    expect(header).toHaveAttribute("aria-expanded", "true");
    await user.click(header);

    expect(
      screen
        .getAllByTestId("issue-queue-group-header")
        .find((node) => node.getAttribute("data-group-key") === groupKey),
    ).toHaveAttribute("aria-expanded", "false");
    const stored = window.localStorage.getItem(
      "aria.workbench.groups.project_0001",
    );
    expect(stored).not.toBeNull();
    expect(JSON.parse(String(stored))).toEqual(
      expect.arrayContaining([...defaultCollapsedGroups(), groupKey]),
    );
  });

  it("队列过滤命中时保留 Issue，未命中时显示空态", async () => {
    vi.stubGlobal("fetch", lifecycleFetch());
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);
    await screen.findByRole("button", { name: "选择 Issue 登录会话过期" });

    await user.type(screen.getByLabelText("过滤 Issues"), "登录");
    expect(
      screen.getByRole("button", { name: "选择 Issue 登录会话过期" }),
    ).toBeInTheDocument();

    await user.clear(screen.getByLabelText("过滤 Issues"));
    await user.type(screen.getByLabelText("过滤 Issues"), "不存在的关键字");
    expect(
      screen.queryByRole("button", { name: "选择 Issue 登录会话过期" }),
    ).toBeNull();
    expect(queueRegion()).toHaveTextContent("没有匹配的 Issue。");
  });

  it("「显示更多」真正追加渲染该组行，追加后入口消失", async () => {
    // 构造 60 个 Issue（> DEFAULT_PER_GROUP_LIMIT=50）使组内 rows 被截断。
    const titles = Array.from(
      { length: 60 },
      (_, index) => `批量 Issue ${String(index + 1).padStart(2, "0")}`,
    );
    vi.stubGlobal("fetch", bulkIssuesFetch(titles));
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);
    await screen.findByTestId("issue-queue-group-list");

    await waitFor(() =>
      expect(screen.getAllByTestId("issue-queue-row")).toHaveLength(50),
    );
    const showMore = screen.getByTestId("issue-queue-show-more");
    expect(showMore).toHaveTextContent("显示更多（+10）");

    await user.click(showMore);

    await waitFor(() =>
      expect(screen.getAllByTestId("issue-queue-row")).toHaveLength(60),
    );
    expect(screen.queryByTestId("issue-queue-show-more")).toBeNull();
  });

  it("refresh 不重置队列折叠态、过滤文本与聚焦 Issue（轮询上下文冻结）", async () => {
    const fetchMock = lifecycleFetch();
    vi.stubGlobal("fetch", fetchMock);
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);
    await user.click(
      await screen.findByRole("button", { name: "选择 Issue 登录会话过期" }),
    );

    // 折叠一个分组 + 输入过滤文本（仍命中当前 Issue）。
    const header = screen.getAllByTestId("issue-queue-group-header")[0];
    const groupKey = header.getAttribute("data-group-key");
    await user.click(header);
    await user.type(screen.getByLabelText("过滤 Issues"), "登录");

    // 触发一次 refresh（与 2s 轮询同一条 refresh() 链路）。
    const callsBefore = fetchMock.mock.calls.length;
    await user.click(screen.getByRole("button", { name: "刷新" }));
    await waitFor(() =>
      expect(fetchMock.mock.calls.length).toBeGreaterThan(callsBefore),
    );
    await waitFor(() => expect(screen.queryByRole("status")).toBeNull());

    // 分组折叠态、过滤文本、聚焦 Issue 全部保持。
    expect(
      screen
        .getAllByTestId("issue-queue-group-header")
        .find((node) => node.getAttribute("data-group-key") === groupKey),
    ).toHaveAttribute("aria-expanded", "false");
    expect(screen.getByLabelText("过滤 Issues")).toHaveValue("登录");
    expect(
      screen.getByRole("region", { name: "Issue 生命周期详情" }),
    ).toHaveTextContent("登录会话过期");
  });

  it("refresh（deferred）冻结聚焦 Issue、抽屉、分组折叠与过滤文本", async () => {
    const firstProjects = deferred<Response>();
    const secondProjects = deferred<Response>();
    const baseFetch = lifecycleFetch({
      projectResponses: [firstProjects.promise, secondProjects.promise],
    });
    // 双 Issue fixture：issue_0001（coding 组，完整生命周期）与 issue_0002
    // （needs_story 组，空生命周期）。这样可同时断言「被折叠的分组」与「聚焦
    // Issue 行」（若折叠唯一分组会连聚焦行一起隐藏，无法验证 aria-current 保留）。
    const issueOne = {
      issue_id: "issue_0001",
      project_id: "project_0001",
      repo_id: "repository_0001",
      workspace_id: null,
      task_id: null,
      session_id: null,
      title: "登录会话过期",
      description: "描述",
      change_id: null,
      phase: "clarification",
      status: "draft",
      active_binding_id: null,
      artifacts: [],
      created_at: "2026-05-16T00:00:00Z",
      updated_at: "2026-05-16T00:00:00Z",
    };
    const issueTwo = { ...issueOne, issue_id: "issue_0002", title: "缺少 Story" };
    const fetchMock = vi.fn(
      async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = String(input);
        if (url === "/api/projects/project_0001/issues") {
          return jsonResponse({ issues: [issueOne, issueTwo] });
        }
        if (url === "/api/issues/issue_0002/lifecycle?project_id=project_0001") {
          return jsonResponse({
            issue: issueTwo,
            story_specs: [],
            design_specs: [],
            work_item_plans: [],
            work_items: [],
            work_item_repository_groups: [],
            workspace_sessions: [],
            coding_attempts: [],
          });
        }
        return baseFetch(input, init);
      },
    );
    vi.stubGlobal("fetch", fetchMock);
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);

    // 首次加载：放行 /api/projects（与后续 refresh 返回相同数据）。
    await act(async () => {
      firstProjects.resolve(jsonResponseValue(projectsBody()));
    });

    await user.click(
      await screen.findByRole("button", { name: "选择 Issue 登录会话过期" }),
    );
    expect(
      screen
        .getAllByTestId("issue-queue-row")
        .find((row) => row.getAttribute("data-issue-id") === "issue_0001"),
    ).toHaveAttribute("aria-current", "true");

    // 折叠不含聚焦 Issue 的分组（needs_story）+ 输入过滤文本（仍命中两个 Issue）。
    const header = screen.getAllByTestId("issue-queue-group-header")[0];
    const groupKey = header.getAttribute("data-group-key");
    await user.click(header);
    await user.type(screen.getByLabelText("过滤 Issues"), "issue");

    // Task 6：Story 卡在 Story 阶段页内；切到 Story 阶段后点击打开抽屉。
    await user.click(screen.getByTestId("stage-tab-story"));
    await user.click(screen.getByRole("button", { name: "会话过期提示" }));
    expect(screen.getByTestId("lifecycle-card-drawer")).toBeInTheDocument();

    // 触发一次 refresh（与 2s 轮询同一条 refresh() 链路），用 deferred
    // 控制 /api/projects 返回相同数据。
    await user.click(screen.getByRole("button", { name: "刷新" }));
    await act(async () => {
      secondProjects.resolve(jsonResponseValue(projectsBody()));
    });
    await waitFor(() => expect(screen.queryByRole("status")).toBeNull());

    // 上下文冻结：聚焦 Issue（aria-current）、抽屉、分组折叠、过滤文本全部保持。
    expect(
      screen
        .getAllByTestId("issue-queue-row")
        .find((row) => row.getAttribute("data-issue-id") === "issue_0001"),
    ).toHaveAttribute("aria-current", "true");
    expect(screen.getByTestId("lifecycle-card-drawer")).toBeInTheDocument();
    expect(
      screen
        .getAllByTestId("issue-queue-group-header")
        .find((node) => node.getAttribute("data-group-key") === groupKey),
    ).toHaveAttribute("aria-expanded", "false");
    expect(screen.getByLabelText("过滤 Issues")).toHaveValue("issue");
  });

  it("队列行动作复用既有 handler：选择、生成 Story Spec、删除 Issue", async () => {
    const fetchMock = lifecycleFetch({ emptyLifecycle: true });
    vi.stubGlobal("fetch", fetchMock);
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);
    await user.click(
      await screen.findByRole("button", { name: "选择 Issue 登录会话过期" }),
    );
    expect(
      screen.getByRole("region", { name: "Issue 生命周期详情" }),
    ).toHaveTextContent("登录会话过期");

    await user.click(
      within(queueRegion()).getByRole("button", { name: "生成 Story Spec 登录会话过期" }),
    );
    expect(fetchMock).toHaveBeenCalledWith(
      "/api/projects/project_0001/issues/issue_0001/story-specs:generate",
      expect.objectContaining({ method: "POST" }),
    );

    await user.click(
      within(queueRegion()).getByRole("button", {
        name: "删除 Issue 登录会话过期",
      }),
    );
    expect(fetchMock).toHaveBeenCalledWith(
      "/api/projects/project_0001/issues/issue_0001",
      expect.objectContaining({ method: "DELETE" }),
    );
  });
});
