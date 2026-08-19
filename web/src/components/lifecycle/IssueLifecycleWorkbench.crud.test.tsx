import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type {
  ProviderHealthResponse,
  RepositoryInitializationOperationSnapshot,
} from "../../api/types";
import { useLifecycleWorkbenchStore } from "../../state/lifecycle-workbench-store";
import { useProviderAvailabilityStore } from "../../state/provider-availability-store";
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
    fireEvent.submit(dialog);
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
    fireEvent.submit(dialog);
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

  it("deletes specs and work item groups from selected issue content", async () => {
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

    await user.click(
      within(screen.getByRole("region", { name: "Work Item 内容" })).getByRole(
        "button",
        { name: /删除 Work Item Group/u },
      ),
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
