import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi, type Mock } from "vitest";
import type {
  ProviderHealthResponse,
  RepositoryInitializationOperationSnapshot,
} from "../../api/types";
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
  repositoryRecord,
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

function repositoryOperationSnapshot(
  status: RepositoryInitializationOperationSnapshot["status"],
  overrides?: Partial<RepositoryInitializationOperationSnapshot>,
): RepositoryInitializationOperationSnapshot {
  return {
    operation_id: "repository_initialization_0001",
    status,
    steps: [
      { step_id: "cadence_skills", status: "completed" },
      {
        step_id: "pre_check",
        status:
          status === "failed"
            ? "failed"
            : status === "completed"
              ? "completed"
              : "running",
      },
      {
        step_id: "rule_config",
        status: status === "completed" ? "completed" : "pending",
      },
      {
        step_id: "mcp_configuration",
        status: status === "completed" ? "completed" : "pending",
      },
      {
        step_id: "project_rules_examples",
        status: status === "completed" ? "completed" : "pending",
      },
    ],
    current_step: status === "completed" ? null : "pre_check",
    failed_step: status === "failed" ? "pre_check" : null,
    result: null,
    error: null,
    created_at: "2026-07-14T00:00:00Z",
    updated_at: "2026-07-14T00:00:00Z",
    completed_at: null,
    ...overrides,
  };
}

function repositoryOperationFetch({
  operationSnapshots,
}: {
  operationSnapshots: RepositoryInitializationOperationSnapshot[];
}) {
  const baseFetch = lifecycleFetch({
    repositoriesByProject: { project_0001: [] },
  });
  const repository = repositoryRecord({
    name: "New Repo",
    path: "/tmp/new-repo",
  });
  let operationFetchCount = 0;
  let completed = false;

  return vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input);
    const requestUrl = url.replace(/\/$/u, "");
    if (
      init?.method === "POST" &&
      requestUrl === "/api/projects/project_0001/repositories"
    ) {
      return new Response(
        JSON.stringify(repositoryOperationSnapshot("created")),
        { status: 202 },
      );
    }
    if (requestUrl.includes("repository-initializations")) {
      const snapshot = operationSnapshots[operationFetchCount];
      operationFetchCount += 1;
      if (!snapshot) {
        throw new Error("unexpected repository initialization fetch");
      }
      completed ||= snapshot.status === "completed";
      return new Response(
        JSON.stringify(snapshot),
        { status: 200, headers: { "content-type": "application/json" } },
      );
    }
    if (
      requestUrl === "/api/projects/project_0001/repositories" &&
      init?.method !== "POST"
    ) {
      if (completed) {
        return new Response(
          JSON.stringify({ repositories: [repository] }),
          { status: 200, headers: { "content-type": "application/json" } },
        );
      }
      return new Response(
        JSON.stringify({ repositories: [] }),
        { status: 200, headers: { "content-type": "application/json" } },
      );
    }
    if (
      requestUrl === "/api/projects/project_0001/codebases" &&
      init?.method !== "POST"
    ) {
      if (completed) {
        return new Response(
          JSON.stringify({
            codebases: [
              {
                id: repository.repository_id,
                name: repository.name,
                kind: "single_repo",
                repository_id: repository.repository_id,
                logical_codebase_id: null,
                member_count: null,
              },
            ],
          }),
          { status: 200, headers: { "content-type": "application/json" } },
        );
      }
      return new Response(JSON.stringify({ codebases: [] }), {
        status: 200,
        headers: { "content-type": "application/json" },
      });
    }
    return baseFetch(input, init);
  });
}

function repositoryFailureFetch() {
  return repositoryOperationFetch({
    operationSnapshots: [
      repositoryOperationSnapshot("failed", {
        error: {
          code: "repository_init_command_failed",
          message: "repository registration failed",
          details: {
            reason: "初始化命令执行失败",
            stage: "repository_init_command",
            provider: "claude_code",
            command: "/rule-config",
            reason_code: "repository_init_command_failed",
            stderr_summary: "permission denied",
            changed_paths: [".claude/rules/project.md"],
            retryable: true,
            action: "修复权限后重试",
          },
        },
      }),
    ],
  });
}

async function flushAsyncWork() {
  await act(async () => {
    await Promise.resolve();
  });
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

  it("only refreshes repositories after the initialization operation completes", async () => {
    const completed = repositoryOperationSnapshot("completed", {
      result: {
        repository: repositoryRecord({
          name: "New Repo",
          path: "/tmp/new-repo",
        }),
        initialization: {
          source: "offline",
          commands: [
            {
              index: 1,
              command: "/cadence-init:pre-check",
              status: "completed",
            },
          ],
          warnings: ["cadence_skills_conflict:<path>"],
          changed_paths: [".claude/rules/project.md"],
          git_finalize_warning: null,
          completed_at: "2026-07-14T00:01:00Z",
        },
      },
      completed_at: "2026-07-14T00:01:00Z",
    });
    const fetchMock = repositoryOperationFetch({
      operationSnapshots: [repositoryOperationSnapshot("running"), completed],
    });
    vi.stubGlobal("fetch", fetchMock);
    vi.useFakeTimers();

    render(<IssueLifecycleWorkbench />);
    await act(async () => {
      await vi.runAllTimersAsync();
    });
    fireEvent.click(screen.getByRole("button", { name: "添加代码库" }));
    fireEvent.click(screen.getByRole("button", { name: "继续添加单仓库" }));
    const dialog = screen.getByRole("dialog", { name: "添加代码库" });
    fireEvent.change(within(dialog).getByLabelText("代码库名称"), {
      target: { value: "New Repo" },
    });
    fireEvent.change(within(dialog).getByLabelText("本地路径"), {
      target: { value: "/tmp/new-repo" },
    });
    fireEvent.submit(within(dialog).getByLabelText("代码库名称").closest("form")!);
    await flushAsyncWork();

    expect(screen.getByText("还没有代码库")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "新建 Issue" })).toBeDisabled();
    expect(screen.getByRole("dialog", { name: "添加代码库" })).toHaveTextContent(
      "正在初始化，请保持此窗口打开",
    );
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1_000);
    });
    await flushAsyncWork();
    await vi.runAllTimersAsync();
    await flushAsyncWork();
    expect(
      fetchMock.mock.calls.filter(
        ([input, init]) =>
          input === "/api/projects/project_0001/repositories" &&
          init?.method !== "POST",
      ),
    ).toHaveLength(2);

    expect(screen.getByRole("dialog", { name: "添加代码库" })).toHaveTextContent(
      "代码库初始化完成",
    );
    expect(screen.getByRole("dialog", { name: "添加代码库" })).toHaveTextContent(
      "offline",
    );
    expect(screen.getByText("New Repo")).toBeInTheDocument();
    const operationFetches = fetchMock.mock.calls.filter(
      ([input]) =>
        input ===
        "/api/projects/project_0001/repository-initializations/repository_initialization_0001",
    );
    expect(operationFetches).toHaveLength(2);
    expect(fetchMock).toHaveBeenCalledWith(
      "/api/projects/project_0001/repository-initializations/repository_initialization_0001",
      expect.objectContaining({
        headers: expect.objectContaining({ "content-type": "application/json" }),
      }),
    );
    expect(
      fetchMock.mock.calls.filter(
        ([input, init]) =>
          input === "/api/projects/project_0001/repositories" &&
          init?.method !== "POST",
      ),
    ).toHaveLength(2);
    expect(screen.getByRole("button", { name: "新建 Issue" })).toBeEnabled();
    expect(screen.getByRole("dialog", { name: "添加代码库" })).toHaveTextContent(
      "cadence_skills_conflict:<path>",
    );
    fireEvent.click(
      within(screen.getByRole("dialog", { name: "添加代码库" })).getByRole(
        "button",
        { name: "完成" },
      ),
    );
    expect(
      screen.queryByRole("dialog", { name: "添加代码库" }),
    ).not.toBeInTheDocument();
  });

  it("keeps repositories empty and preserves recovery details when initialization fails", async () => {
    const fetchMock = repositoryFailureFetch();
    vi.stubGlobal("fetch", fetchMock);
    vi.useFakeTimers();

    render(<IssueLifecycleWorkbench />);
    await act(async () => {
      await vi.runAllTimersAsync();
    });
    fireEvent.click(screen.getByRole("button", { name: "添加代码库" }));
    fireEvent.click(screen.getByRole("button", { name: "继续添加单仓库" }));
    const dialog = screen.getByRole("dialog", { name: "添加代码库" });
    fireEvent.change(within(dialog).getByLabelText("代码库名称"), {
      target: { value: "New Repo" },
    });
    fireEvent.change(within(dialog).getByLabelText("本地路径"), {
      target: { value: "/tmp/new-repo" },
    });
    fireEvent.submit(within(dialog).getByLabelText("代码库名称").closest("form")!);
    await flushAsyncWork();
    expect(
      fetchMock.mock.calls.filter(
        ([input, init]) =>
          input === "/api/projects/project_0001/repositories" &&
          init?.method !== "POST",
      ),
    ).toHaveLength(1);
    expect(screen.getByText("还没有代码库")).toBeInTheDocument();
    const alert = screen.getByRole("alert");
    expect(
      fetchMock.mock.calls.filter(
        ([input, init]) =>
          input === "/api/projects/project_0001/repositories" &&
          init?.method !== "POST",
      ),
    ).toHaveLength(1);
    expect(alert).toHaveTextContent("repository registration failed");
    expect(alert).toHaveTextContent("repository_init_command");
    expect(alert).toHaveTextContent(".claude/rules/project.md");
    expect(alert).toHaveTextContent("修复权限后重试");
    expect(alert).toHaveTextContent("系统未执行破坏性回滚");
    expect(screen.getByRole("dialog", { name: "添加代码库" })).toBeInTheDocument();
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

    await user.click(screen.getByRole("button", { name: "删除 Project Aria" }));

    expect(fetchMock).toHaveBeenCalledWith(
      "/api/projects/project_0001",
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
        name: "删除 Work Item Group",
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

  it("keeps a durably created issue searchable while the list index is stale", async () => {
    const baseFetch = lifecycleFetch();
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
    await user.selectOptions(within(dialog).getByLabelText("代码库"), "repo:repository_0001");
    await user.click(within(dialog).getByRole("button", { name: "创建 Issue" }));

    expect(await screen.findByRole("button", { name: "选择 Issue P1 补丁批" })).toBeVisible();
    const filter = screen.getByRole("textbox", { name: "过滤 Issues" });
    await user.type(filter, "0277");
    expect(screen.getByRole("button", { name: "选择 Issue P1 补丁批" })).toBeVisible();
  });

  it("keeps a durably created issue visible across later stale refreshes", async () => {
    const baseFetch = lifecycleFetch();
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
    await user.selectOptions(within(dialog).getByLabelText("代码库"), "repo:repository_0001");
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
          name: "删除 Work Item Group",
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
          name: "删除 Work Item Group",
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
          name: "删除 Work Item Group",
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
