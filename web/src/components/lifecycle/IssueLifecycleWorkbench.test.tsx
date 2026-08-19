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

describe("IssueLifecycleWorkbench base workflow", () => {
  installIssueLifecycleWorkbenchTestHooks();

  it("loads the aggregate index, disables rebuild while pending, replaces it on success, and shows API errors", async () => {
    const rebuildingResponse = deferred<Response>();
    const fetchMock = lifecycleFetch({
      projects: [projectRecord("project_0001", "Aria")],
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
            "/api/projects/project_0001/logical-codebase/aggregate-indexes/rebuild" &&
          init?.method === "POST"
        ) {
          return rebuildingResponse.promise;
        }
        return fetchMock(input, init);
      }),
    );
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);

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
      aggregateIndex: aggregateIndex({ state: "stale" }),
    });
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        if (
          String(input) ===
            "/api/projects/project_0001/logical-codebase/aggregate-indexes/rebuild" &&
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
      pointerPublications: [initialPublication],
    });
    vi.stubGlobal("fetch", fetchMock);
    vi.spyOn(window, "confirm").mockReturnValue(true);
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);

    await user.click(await screen.findByRole("button", { name: "全量发布" }));
    await waitFor(() =>
      expect(fetchMock).toHaveBeenCalledWith(
        "/api/projects/project_0001/logical-codebase/pointer-publications",
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
        "/api/projects/project_0001/logical-codebase/pointer-publications",
        expect.objectContaining({
          method: "POST",
          body: JSON.stringify({ batch_kind: "incremental" }),
        }),
      ),
    );

    await user.click(screen.getByRole("button", { name: "重试" }));
    await waitFor(() =>
      expect(fetchMock).toHaveBeenCalledWith(
        "/api/projects/project_0001/logical-codebase/pointer-publications/publication_0000/retry-repo",
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
        "/api/projects/project_0001/logical-codebase/pointer-publications/publication_0000/revoke",
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

    const hint = await screen.findByTestId(
      "pointer-publication-new-members-hint",
    );
    expect(hint).toHaveTextContent("检测到新增成员，建议增量发布");
    await user.click(within(hint).getByRole("button", { name: "增量发布" }));
    await waitFor(() =>
      expect(fetchMock).toHaveBeenCalledWith(
        "/api/projects/project_0001/logical-codebase/pointer-publications",
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

    render(<IssueLifecycleWorkbench />);

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
