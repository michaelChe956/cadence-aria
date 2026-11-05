import { act, cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi, type Mock } from "vitest";
import type { ProviderHealthResponse } from "../../api/types";
import { useLifecycleWorkbenchStore } from "../../state/lifecycle-workbench-store";
import { useProviderAvailabilityStore } from "../../state/provider-availability-store";
import { cascadeDeleteConfirmationFetch } from "./IssueLifecycleWorkbench.test-data";
import {
  defaultLaunchTitle,
  IssueLifecycleWorkbench,
} from "./IssueLifecycleWorkbench";
import {
  deferred,
  installIssueLifecycleWorkbenchTestHooks,
  issueWorkItemPlanRecord,
  jsonResponseValue,
  lifecycleCardTitle,
  lifecycleFetch,
  projectRecord,
  projectsBody,
} from "./IssueLifecycleWorkbench.test-utils";

vi.mock("../shared/MonacoViewer", () => ({
  MonacoViewer: ({ value, height }: { value: string; height?: string }) => (
    <div data-testid="monaco-viewer" data-height={height}>
      {value}
    </div>
  ),
}));

function setRepositoryProviderHealth() {
  const snapshot: ProviderHealthResponse = {
    schema_version: 1,
    generation: 1,
    checked_at: "2026-07-14T00:00:00Z",
    state_status: "ready",
    state_error: null,
    real_workflow_blocked: false,
    test_provider_enabled: false,
    providers: [
      {
        provider: "claude_code",
        display_name: "Claude Code",
        available: true,
        version: "1.0.0",
        reason_code: null,
        reason: null,
        checked_at: "2026-07-14T00:00:00Z",
        install_hint: "",
      },
      {
        provider: "codex",
        display_name: "Codex",
        available: true,
        version: "1.0.0",
        reason_code: null,
        reason: null,
        checked_at: "2026-07-14T00:00:00Z",
        install_hint: "",
      },
    ],
  };
  useProviderAvailabilityStore.setState({ snapshot, loadStatus: "loaded" });
}

describe("IssueLifecycleWorkbench project and lifecycle CRUD", () => {
  installIssueLifecycleWorkbenchTestHooks();
  beforeEach(() => {
    setRepositoryProviderHealth();
  });
  afterEach(() => {
    vi.clearAllTimers();
    vi.useRealTimers();
    useProviderAvailabilityStore.getState().reset();
  });

  it("添加代码库只走多仓入口：登记完成后才刷新代码库列表", async () => {
    const calls: string[] = [];
    const fetchMock = lifecycleFetch({
      projects: [projectRecord("project_0001", "Aria")],
      repositoriesByProject: { project_0001: [] },
      registrationPreflight: {
        preflight_id: "preflight_0001",
        created_at: "2026-08-19T00:00:00Z",
        items: [{ path: "/root/api", class: "eligible", reason: null }],
      },
      registrationSubmit: {
        batch_id: "batch_0001",
        status: "completed",
        items: [{ path: "/root/api", status: "completed", failure_reason: null }],
      },
    });
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        calls.push(`${init?.method ?? "GET"} ${String(input)}`);
        return fetchMock(input, init);
      }),
    );
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);

    await user.click(await screen.findByRole("button", { name: "添加代码库" }));
    // REQ-REG-16：无单仓入口——对话框直接是建逻辑代码库（选聚合根）。
    expect(
      screen.queryByRole("button", { name: "继续添加单仓库" }),
    ).not.toBeInTheDocument();
    await user.type(screen.getByLabelText("聚合根目录"), "/root");
    await user.click(screen.getByRole("button", { name: "创建逻辑代码库" }));

    // 登记在同弹窗内完成；登记批完成后才再拉一次代码库列表。
    await user.click(await screen.findByRole("button", { name: "提交登记" }));
    expect(
      calls.filter(
        (call) =>
          call ===
          "POST /api/projects/project_0001/logical-codebases/lc_0001/registrations",
      ),
    ).toHaveLength(1);
    const codebasesGets = () =>
      calls.filter((call) => call === "GET /api/projects/project_0001/codebases")
        .length;
    await waitFor(() =>
      expect(codebasesGets()).toBeGreaterThanOrEqual(2),
    );
    expect(screen.getByRole("region", { name: "登记结果" })).toHaveTextContent(
      "completed",
    );
  });

  it("登记失败时保留错误详情且不刷新代码库列表", async () => {
    const calls: string[] = [];
    const baseFetch = lifecycleFetch({
      projects: [projectRecord("project_0001", "Aria")],
      repositoriesByProject: { project_0001: [] },
      registrationPreflight: {
        preflight_id: "preflight_0001",
        created_at: "2026-08-19T00:00:00Z",
        items: [{ path: "/root/api", class: "eligible", reason: null }],
      },
    });
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = String(input);
        calls.push(`${init?.method ?? "GET"} ${url}`);
        if (url.endsWith("/registrations") && init?.method === "POST") {
          return new Response(
            JSON.stringify({
              code: "aggregate_root_member_outside_root",
              message: "登记成员不在聚合根内",
              details: { reason: "成员目录 /root/api 不在聚合根内" },
            }),
            { status: 422, headers: { "content-type": "application/json" } },
          );
        }
        return baseFetch(input, init);
      }),
    );
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);

    await user.click(await screen.findByRole("button", { name: "添加代码库" }));
    await user.type(screen.getByLabelText("聚合根目录"), "/root");
    await user.click(screen.getByRole("button", { name: "创建逻辑代码库" }));
    const codebasesGetsBefore = calls.filter(
      (call) => call === "GET /api/projects/project_0001/codebases",
    ).length;

    await user.click(await screen.findByRole("button", { name: "提交登记" }));

    const dialog = screen.getByRole("dialog", { name: "添加代码库" });
    expect(await within(dialog).findByRole("alert")).toHaveTextContent(
      "成员目录 /root/api 不在聚合根内",
    );
    // 登记失败不刷新代码库列表，也不产生代码库条目。
    expect(
      calls.filter(
        (call) => call === "GET /api/projects/project_0001/codebases",
      ),
    ).toHaveLength(codebasesGetsBefore);
    expect(screen.getByText("还没有代码库")).toBeInTheDocument();
  });

  it("deletes project repositories and lifecycle issues from the lifecycle workbench", async () => {
    const fetchMock = lifecycleFetch();
    vi.stubGlobal("fetch", fetchMock);
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);

    // Task 7：队列改为 IssueQueue，行选择按钮名为「选择 Issue <标题>」。
    expect(
      await screen.findByRole("button", { name: "选择 Issue 登录会话过期" }),
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
        screen.queryByRole("button", { name: "选择 Issue 登录会话过期" }),
      ).not.toBeInTheDocument(),
    );

    // 2026-11-05：删除 Project 先过二次确认；确认后 DELETE 带确认意图
    //（未确认路径见 IssueLifecycleWorkbench.project-delete.test.tsx）。
    const confirmSpy = vi.spyOn(window, "confirm").mockReturnValue(true);
    await user.click(screen.getByRole("button", { name: "删除 Project Aria" }));

    expect(confirmSpy).toHaveBeenCalledTimes(1);
    expect(fetchMock).toHaveBeenCalledWith(
      "/api/projects/project_0001?confirmed=true",
      expect.objectContaining({ method: "DELETE" }),
    );
    expect(await screen.findByText("还没有 Project")).toBeInTheDocument();
  });

  it("deletes specs and work item groups from selected issue content", async () => {
    const fetchMock = lifecycleFetch();
    vi.stubGlobal("fetch", fetchMock);
    const user = userEvent.setup();
    const confirmSpy = vi.spyOn(window, "confirm").mockReturnValue(true);

    render(<IssueLifecycleWorkbench />);

    await user.click(
      await screen.findByRole("button", { name: "选择 Issue 登录会话过期" }),
    );

    // Task 6：单阶段面板——每个阶段的删除入口在对应阶段页内。
    await user.click(screen.getByTestId("stage-tab-story"));
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

    await user.click(screen.getByTestId("stage-tab-design"));
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

    await user.click(screen.getByTestId("stage-tab-work_item"));
    await user.click(
      within(screen.getByRole("region", { name: "Work Item 内容" })).getByRole(
        "button",
        { name: "Work Item Group" },
      ),
    );
    await user.click(
      within(screen.getByTestId("lifecycle-card-drawer")).getByRole("button", {
        name: "删除计划及工作项",
      }),
    );
    // D2.3 机械迁移：group drawer 旧无条件确认移除——确认语义由服务端三分类
    // 权威给出（本夹具 200=Draft 语义，confirm 0 次；确认流见
    // cascade_d2_3_single_confirmation）。
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

  it("requires logical codebase when creating issue", async () => {
    // C5：新建 Issue 的门已收窄为「必须存在含 active 成员的 LC」——该夹具补一条
    // 有成员的 LC 才能进入对话框；本用例断言的是对话框内的必选校验，不是门本身。
    vi.stubGlobal(
      "fetch",
      lifecycleFetch({
        logicalCodebases: [{ id: "lc_0001", name: "platform", member_count: 1 }],
      }),
    );
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

    // REQ-MRE-01：issue 必须落在某个逻辑代码库上（单仓路径已下线）。
    expect(within(dialog).getByText("请选择逻辑代码库")).toBeInTheDocument();
  });

  it("keeps a durably created issue searchable while the list index is stale", async () => {
    const baseFetch = lifecycleFetch({
      // REQ-MRE-01：issue 只能挂在逻辑代码库上（单仓创建入口已下线）。
      logicalCodebases: [{ id: "lc_0001", name: "platform", member_count: 1 }],
    });
    const createdIssue = {
      issue_id: "issue_0277",
      project_id: "project_0001",
      repo_id: "repository_0001",
      workspace_id: null,
      task_id: null,
      session_id: null,
      title: "P1 补丁批",
      description: "索引延迟期间仍应可见",
      change_id: "p1-patch",
      phase: "clarification",
      status: "draft",
      active_binding_id: null,
      artifacts: [],
      created_at: "2026-09-17T10:00:00Z",
      updated_at: "2026-09-17T10:00:00Z",
    };
    const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = String(input);
      if (url === "/api/projects/project_0001/issues" && init?.method === "POST") {
        return jsonResponseValue(createdIssue);
      }
      if (url === "/api/issues/issue_0277/lifecycle?project_id=project_0001") {
        return jsonResponseValue({
          issue: createdIssue,
          story_specs: [], design_specs: [], work_item_plans: [], work_items: [],
          work_item_repository_groups: [], workspace_sessions: [], coding_attempts: [],
        });
      }
      return baseFetch(input, init);
    });
    vi.stubGlobal("fetch", fetchMock);
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);
    await user.click(await screen.findByRole("button", { name: "新建 Issue" }));
    const dialog = screen.getByRole("dialog", { name: "新建 Issue" });
    await user.type(within(dialog).getByLabelText("Issue 标题"), "P1 补丁批");
    await user.selectOptions(within(dialog).getByLabelText("代码库"), "lc_0001");
    const memberGroup = await within(dialog).findByRole("group", {
      name: "成员",
    });
    await user.click(
      within(memberGroup).getByRole("checkbox", { name: /api · repository_0001/ }),
    );
    await user.click(within(dialog).getByRole("button", { name: "创建 Issue" }));

    expect(await screen.findByRole("button", { name: "选择 Issue P1 补丁批" })).toBeVisible();
    const filter = screen.getByRole("textbox", { name: "过滤 Issues" });
    await user.type(filter, "0277");
    expect(screen.getByRole("button", { name: "选择 Issue P1 补丁批" })).toBeVisible();
  });

  it("keeps a durably created issue visible across later stale refreshes", async () => {
    const baseFetch = lifecycleFetch({
      // REQ-MRE-01：issue 只能挂在逻辑代码库上（单仓创建入口已下线）。
      logicalCodebases: [{ id: "lc_0001", name: "platform", member_count: 1 }],
    });
    const createdIssue = {
      issue_id: "issue_0277",
      project_id: "project_0001",
      repo_id: "repository_0001",
      workspace_id: null,
      task_id: null,
      session_id: null,
      title: "P1 补丁批",
      description: "索引延迟期间仍应可见",
      change_id: "p1-patch",
      phase: "clarification",
      status: "draft",
      active_binding_id: null,
      artifacts: [],
      created_at: "2026-09-17T10:00:00Z",
      updated_at: "2026-09-17T10:00:00Z",
    };
    const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = String(input);
      if (url === "/api/projects/project_0001/issues" && init?.method === "POST") {
        return jsonResponseValue(createdIssue);
      }
      if (url === "/api/issues/issue_0277/lifecycle?project_id=project_0001") {
        return jsonResponseValue({
          issue: createdIssue,
          story_specs: [], design_specs: [], work_item_plans: [], work_items: [],
          work_item_repository_groups: [], workspace_sessions: [], coding_attempts: [],
        });
      }
      return baseFetch(input, init);
    });
    vi.stubGlobal("fetch", fetchMock);
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);
    await user.click(await screen.findByRole("button", { name: "新建 Issue" }));
    const dialog = screen.getByRole("dialog", { name: "新建 Issue" });
    await user.type(within(dialog).getByLabelText("Issue 标题"), "P1 补丁批");
    await user.selectOptions(within(dialog).getByLabelText("代码库"), "lc_0001");
    const memberGroup = await within(dialog).findByRole("group", {
      name: "成员",
    });
    await user.click(
      within(memberGroup).getByRole("checkbox", { name: /api · repository_0001/ }),
    );
    await user.click(within(dialog).getByRole("button", { name: "创建 Issue" }));

    expect(await screen.findByRole("button", { name: "选择 Issue P1 补丁批" })).toBeVisible();
    await user.click(screen.getByRole("button", { name: "刷新" }));
    expect(await screen.findByRole("button", { name: "选择 Issue P1 补丁批" })).toBeVisible();
  });

  it("preserves the service issue order when no created issue remains pending", async () => {
    const olderIssue = {
      issue_id: "issue_0001",
      project_id: "project_0001",
      repo_id: "repository_0001",
      workspace_id: null,
      task_id: null,
      session_id: null,
      title: "服务端第一项",
      description: "描述",
      change_id: "first",
      phase: "clarification",
      status: "draft",
      active_binding_id: null,
      artifacts: [],
      created_at: "2026-09-16T10:00:00Z",
      updated_at: "2026-09-16T10:00:00Z",
    };
    const newerIssue = {
      ...olderIssue,
      issue_id: "issue_0002",
      title: "服务端第二项",
      change_id: "second",
      created_at: "2026-09-17T10:00:00Z",
      updated_at: "2026-09-17T10:00:00Z",
    };
    vi.stubGlobal(
      "fetch",
      lifecycleFetch({
        issuesByProject: { project_0001: [olderIssue, newerIssue] },
      }),
    );

    render(<IssueLifecycleWorkbench />);

    const rows = await screen.findAllByTestId("issue-queue-row");
    expect(rows.map((row) => row.getAttribute("data-issue-id"))).toEqual([
      "issue_0001",
      "issue_0002",
    ]);
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
      await screen.findByRole("button", { name: "选择 Issue 最新 Issue" }),
    ).toBeInTheDocument();

    firstProjects.resolve(jsonResponseValue(projectsBody()));
    await waitFor(() => expect(fetchMock).toHaveBeenCalled());
    expect(
      screen.queryByRole("button", { name: "选择 Issue 旧 Issue" }),
    ).not.toBeInTheDocument();
  });

  it("does not mark derived cards selected when their id matches an issue id", async () => {
    vi.stubGlobal("fetch", lifecycleFetch({ duplicateCardIds: true }));
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);

    await user.click(
      await screen.findByRole("button", { name: "选择 Issue 重复 ID Issue" }),
    );

    // Task 7：Issue 焦点高亮载体为 IssueQueueRow（aria-current），
    // 契约不变——同名 id 的派生卡片不得被标记为选中。
    expect(
      screen
        .getAllByTestId("issue-queue-row")
        .find((row) => row.getAttribute("data-issue-id") === "shared_id"),
    ).toHaveAttribute("aria-current", "true");
    // Task 6：Story 卡片在 Story 阶段页内。
    await user.click(screen.getByTestId("stage-tab-story"));
    expect(
      screen.getByRole("button", { name: "重复 ID Story" }),
    ).toHaveAttribute("aria-pressed", "false");
  });

  it("cascade_d2_3_single_confirmation", async () => {
    // 级联删除 D2.3（REQ-DEL-04/05）：Draft 零弹窗；required 409 恰一次确认、
    // 同意后第二 delete 带 confirmed=true；取消只发第一非破坏请求、无刷新/
    // 退场；旧未知 reason 同一次确认流；drawer group 不双弹；coding/
    // reference/busy 零确认、提示先删后置。
    const user = userEvent.setup();
    // 三类实体 DELETE 的出站请求（story-specs/design-specs/work-item-plans）。
    const entityDeleteCalls = (fetchMock: Mock) =>
      fetchMock.mock.calls.filter(
        ([input, init]) =>
          init?.method === "DELETE" &&
          /\/(?:story-specs|design-specs|work-item-plans)\//.test(String(input)),
      );

    // —— 场景 A：三类 Draft 200 → confirm 0 次 / delete 各 1 次、URL 无 query ——
    {
      const fetchMock = vi.fn(
        cascadeDeleteConfirmationFetch(
          lifecycleFetch({ storyDraftInitially: true, designDraftInitially: true }),
        ),
      );
      vi.stubGlobal("fetch", fetchMock);
      const confirmSpy = vi.spyOn(window, "confirm").mockReturnValue(true);

      render(<IssueLifecycleWorkbench />);
      await user.click(
        await screen.findByRole("button", { name: "选择 Issue 登录会话过期" }),
      );
      await user.click(screen.getByTestId("stage-tab-story"));
      await user.click(
        within(screen.getByRole("region", { name: "Story Spec 内容" })).getByRole(
          "button",
          { name: "删除 Story Spec 会话过期提示" },
        ),
      );
      await user.click(screen.getByTestId("stage-tab-design"));
      await user.click(
        within(screen.getByRole("region", { name: "Design Spec 内容" })).getByRole(
          "button",
          { name: "删除 Design Spec 前端提示设计" },
        ),
      );
      await waitFor(() =>
        expect(entityDeleteCalls(fetchMock)).toHaveLength(2),
      );
      await user.click(screen.getByTestId("stage-tab-work_item"));
      await user.click(screen.getByRole("button", { name: "Work Item Group" }));
      await user.click(
        within(screen.getByTestId("lifecycle-card-drawer")).getByRole("button", {
          name: "删除计划及工作项",
        }),
      );
      await waitFor(() =>
        expect(entityDeleteCalls(fetchMock)).toHaveLength(3),
      );

      expect(confirmSpy).toHaveBeenCalledTimes(0);
      expect(
        entityDeleteCalls(fetchMock).every(([input]) => !String(input).includes("?")),
      ).toBe(true);
      confirmSpy.mockRestore();
      cleanup();
    }

    // —— 场景 B：required(previously_confirmed) 409 → confirm 恰 1 次，同意后
    //    第二 delete 带 confirmed=true，成功后沿原刷新模式收口 ——
    {
      const fetchMock = vi.fn(
        cascadeDeleteConfirmationFetch(lifecycleFetch(), {
          requiredReasonByEntityId: { story_spec_0001: "previously_confirmed" },
        }),
      );
      vi.stubGlobal("fetch", fetchMock);
      const confirmSpy = vi.spyOn(window, "confirm").mockReturnValue(true);

      render(<IssueLifecycleWorkbench />);
      await user.click(
        await screen.findByRole("button", { name: "选择 Issue 登录会话过期" }),
      );
      await user.click(screen.getByTestId("stage-tab-story"));
      await user.click(
        within(screen.getByRole("region", { name: "Story Spec 内容" })).getByRole(
          "button",
          { name: "删除 Story Spec 会话过期提示" },
        ),
      );

      await waitFor(() =>
        expect(
          entityDeleteCalls(fetchMock).map(([input]) => String(input)),
        ).toEqual([
          "/api/projects/project_0001/issues/issue_0001/story-specs/story_spec_0001",
          "/api/projects/project_0001/issues/issue_0001/story-specs/story_spec_0001?confirmed=true",
        ]),
      );
      expect(confirmSpy).toHaveBeenCalledTimes(1);
      expect(confirmSpy).toHaveBeenCalledWith(
        expect.stringContaining("永久清除内容、工作项、会话和 review"),
      );
      expect(confirmSpy).toHaveBeenCalledWith(
        expect.stringContaining("不可恢复"),
      );
      await waitFor(() =>
        expect(
          screen.getByRole("region", { name: "Story Spec 内容" }),
        ).not.toHaveTextContent("会话过期提示"),
      );
      confirmSpy.mockRestore();
      cleanup();
    }

    // —— 场景 C：取消 → 只发第一非破坏请求、无刷新/退场，卡片保留 ——
    // —— 场景 C 同时覆盖旧未知 reason（legacy_history_unknown）恰一次确认 ——
    {
      const fetchMock = vi.fn(
        cascadeDeleteConfirmationFetch(lifecycleFetch(), {
          requiredReasonByEntityId: {
            design_spec_0001: "legacy_history_unknown",
          },
        }),
      );
      vi.stubGlobal("fetch", fetchMock);
      const confirmSpy = vi.spyOn(window, "confirm").mockReturnValue(false);

      render(<IssueLifecycleWorkbench />);
      await user.click(
        await screen.findByRole("button", { name: "选择 Issue 登录会话过期" }),
      );
      await user.click(screen.getByTestId("stage-tab-design"));
      await user.click(
        within(screen.getByRole("region", { name: "Design Spec 内容" })).getByRole(
          "button",
          { name: "删除 Design Spec 前端提示设计" },
        ),
      );

      await waitFor(() => expect(confirmSpy).toHaveBeenCalledTimes(1));
      await act(async () => {
        // 退场动画窗口（220ms）过后复核：无第二请求、无刷新清卡。
        // tsconfig lib 低于 es2024（Promise.withResolvers 不可用），executor
        // 形式为目标库约束下的例外。
        await new Promise((resolve) => {
          setTimeout(resolve, 320);
        });
      });
      expect(entityDeleteCalls(fetchMock)).toHaveLength(1);
      expect(entityDeleteCalls(fetchMock)[0]?.[0]).toBe(
        "/api/projects/project_0001/issues/issue_0001/design-specs/design_spec_0001",
      );
      expect(
        screen.getByRole("region", { name: "Design Spec 内容" }),
      ).toHaveTextContent("前端提示设计");
      confirmSpy.mockRestore();
      cleanup();
    }

    // —— 场景 D：drawer group 不双弹——服务端 required 恰一次确认 ——
    {
      const fetchMock = vi.fn(
        cascadeDeleteConfirmationFetch(lifecycleFetch(), {
          requiredReasonByEntityId: {
            issue_plan_0001: "previously_confirmed",
          },
        }),
      );
      vi.stubGlobal("fetch", fetchMock);
      const confirmSpy = vi.spyOn(window, "confirm").mockReturnValue(true);

      render(<IssueLifecycleWorkbench />);
      await user.click(
        await screen.findByRole("button", { name: "选择 Issue 登录会话过期" }),
      );
      await user.click(screen.getByTestId("stage-tab-work_item"));
      await user.click(screen.getByRole("button", { name: "Work Item Group" }));
      await user.click(
        within(screen.getByTestId("lifecycle-card-drawer")).getByRole("button", {
          name: "删除计划及工作项",
        }),
      );

      await waitFor(() =>
        expect(
          entityDeleteCalls(fetchMock).map(([input]) => String(input)),
        ).toEqual([
          "/api/projects/project_0001/issues/issue_0001/work-item-plans/issue_plan_0001",
          "/api/projects/project_0001/issues/issue_0001/work-item-plans/issue_plan_0001?confirmed=true",
        ]),
      );
      // 旧 drawer 无条件预确认会叠加本地+服务端两次弹窗；新契约恰一次。
      expect(confirmSpy).toHaveBeenCalledTimes(1);
      await waitFor(() =>
        expect(
          screen.queryByTestId("lifecycle-card-drawer"),
        ).not.toBeInTheDocument(),
      );
      confirmSpy.mockRestore();
      cleanup();
    }

    // —— 场景 E：coding/reference/busy 错误 confirm 0 次、提示先删后置 ——
    {
      const fetchMock = vi.fn(
        cascadeDeleteConfirmationFetch(lifecycleFetch(), {
          rejectedCodeByEntityId: {
            story_spec_0001: "entity_referenced",
            design_spec_0001: "entity_busy",
            issue_plan_0001: "coding_workspace_exists",
          },
        }),
      );
      vi.stubGlobal("fetch", fetchMock);
      const confirmSpy = vi.spyOn(window, "confirm").mockReturnValue(true);

      render(<IssueLifecycleWorkbench />);
      await user.click(
        await screen.findByRole("button", { name: "选择 Issue 登录会话过期" }),
      );
      await user.click(screen.getByTestId("stage-tab-story"));
      await user.click(
        within(screen.getByRole("region", { name: "Story Spec 内容" })).getByRole(
          "button",
          { name: "删除 Story Spec 会话过期提示" },
        ),
      );
      expect(
        await screen.findByText(
          "实体仍被存活后置对象引用，请先删除引用它的对象",
        ),
      ).toBeInTheDocument();
      expect(entityDeleteCalls(fetchMock)).toHaveLength(1);

      await user.click(screen.getByTestId("stage-tab-design"));
      await user.click(
        within(screen.getByRole("region", { name: "Design Spec 内容" })).getByRole(
          "button",
          { name: "删除 Design Spec 前端提示设计" },
        ),
      );
      expect(
        await screen.findByText(
          "实体存在进行中的运行或并发写入，请等待收口后重试删除",
        ),
      ).toBeInTheDocument();

      await user.click(screen.getByTestId("stage-tab-work_item"));
      await user.click(screen.getByRole("button", { name: "Work Item Group" }));
      await user.click(
        within(screen.getByTestId("lifecycle-card-drawer")).getByRole("button", {
          name: "删除计划及工作项",
        }),
      );
      expect(
        await screen.findByText(
          "存在 coding workspace，请先删除 coding workspace 再删除 work item group",
        ),
      ).toBeInTheDocument();

      expect(confirmSpy).toHaveBeenCalledTimes(0);
      expect(entityDeleteCalls(fetchMock)).toHaveLength(3);
      confirmSpy.mockRestore();
      cleanup();
    }
  }, 30_000);

  it("opens drawer from derived lifecycle cards and opens full screen workspace from drawer CTA", async () => {
    const fetchMock = lifecycleFetch();
    vi.stubGlobal("fetch", fetchMock);
    const user = userEvent.setup();
    const onOpenWorkspace = vi.fn();

    render(<IssueLifecycleWorkbench onOpenWorkspace={onOpenWorkspace} />);

    // Task 6：Story 卡片在 Story 阶段页内。
    await user.click(await screen.findByTestId("stage-tab-story"));
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

    // Task 6：Story 卡片在 Story 阶段页内。
    await user.click(await screen.findByTestId("stage-tab-story"));
    await user.click(screen.getByRole("button", { name: "会话过期提示" }));

    expect(screen.getByTestId("lifecycle-card-drawer")).toHaveTextContent(
      "会话过期提示",
    );
    expect(onOpenWorkspace).not.toHaveBeenCalled();
    expect(
      screen.getByRole("button", { name: "生成 Design Spec" }),
    ).toBeInTheDocument();
  });

  it("does not show the aggregate initialization entry for single repo projects", async () => {
    vi.stubGlobal("fetch", lifecycleFetch({ logicalCodebaseMembers: [] }));
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);

    await screen.findByRole("navigation", { name: "Project 切换" });
    expect(
      screen.queryByTestId("aggregate-initialization-card"),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "启动聚合初始化" }),
    ).not.toBeInTheDocument();
  });

});
