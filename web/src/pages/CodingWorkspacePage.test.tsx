import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import {
  createMemoryHistory,
  RouterProvider,
} from "@tanstack/react-router";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type {
  CodingAttentionItemDto,
  CodingExecutionUnit,
  CodingWsOutMessage,
  ProviderHealthResponse,
} from "../api/types";
import {
  confirmWorkItemExecutionPlan,
  getCodingAttemptDiff,
  getPlanCodingProgress,
  requestWorkItemExecutionPlanChange,
} from "../api/client";
import {
  codingAttentionLocation,
  createAppRouter,
  router,
} from "../router";
import { useProviderAvailabilityStore } from "../state/provider-availability-store";
import { useCodingWorkspaceWs } from "../hooks/useCodingWorkspaceWs";
import { useWorkspaceSessionObservers } from "../hooks/useWorkspaceSessionObservers";
import { useCodingWorkspaceStore } from "../state/coding-workspace-store";
import {
  CODING_WORKSPACE_HOTKEYS,
  COCKPIT_HOTKEYS,
} from "../state/cockpit-operation-semantics";
import { CodingWorkspacePage } from "./CodingWorkspacePage";
import {
  CODING_ATTEMPT_ADDRESS,
  DEFAULT_PERMISSION_MODES,
  executionPlan,
  installCodingWorkspacePageTestHooks,
  mockCodingWs,
  readyCodingState,
} from "./CodingWorkspacePage.test-utils";

vi.mock("../api/client", () => ({
  confirmWorkItemExecutionPlan: vi.fn(),
  getCodingAttemptDiff: vi.fn(),
  // 默认无 plan 聚合（旧用例降级回局部投影）；需要聚合的用例单独 mockResolvedValue。
  getPlanCodingProgress: vi.fn(async () => {
    throw new Error("plan aggregate not configured for this test");
  }),
  requestWorkItemExecutionPlanChange: vi.fn(),
}));

vi.mock("../hooks/useCodingWorkspaceWs", () => ({
  useCodingWorkspaceWs: vi.fn(),
}));

vi.mock("../hooks/useUnloadGuard", () => ({
  useUnloadGuard: vi.fn(),
}));

// P1-c 路由集成用例渲染 RootRouteComponent（CockpitShell）——observer 依赖
// 统一打桩（与 router.test/pending-choice 同款空投影）。
vi.mock("../hooks/useWorkspaceSessionObservers", () => ({
  useWorkspaceSessionObservers: vi.fn(),
}));

vi.mock("../components/shared/MonacoViewer", () => ({
  MonacoViewer: ({
    value,
    language,
    height,
  }: {
    value: string;
    language?: string;
    height?: string;
  }) => (
    <div data-testid="monaco-viewer" data-language={language} data-height={height}>
      {value}
    </div>
  ),
}));

vi.mock("../components/shared/MonacoDiffViewer", () => ({
  MonacoDiffViewer: ({
    original,
    modified,
    language,
    height,
  }: {
    original: string;
    modified: string;
    language?: string;
    height?: string;
  }) => (
    <div data-testid="monaco-diff-viewer" data-language={language} data-height={height}>
      <span data-testid="monaco-diff-original">{original}</span>
      <span data-testid="monaco-diff-modified">{modified}</span>
    </div>
  ),
}));

// P1-c：定位断言共用——记录每个 scrollIntoView 调用落点的元素。
function spyScrollIntoView() {
  const scrolled: HTMLElement[] = [];
  Object.defineProperty(HTMLElement.prototype, "scrollIntoView", {
    configurable: true,
    value(this: HTMLElement) {
      scrolled.push(this);
    },
  });
  return scrolled;
}

describe("CodingWorkspacePage shell and actions", () => {
  installCodingWorkspacePageTestHooks();

  function mockCodingSessionState(
    overrides: Partial<Extract<CodingWsOutMessage, { type: "coding_session_state" }>>,
  ) {
    useCodingWorkspaceStore.getState().setSessionState({
      type: "coding_session_state",
      project_id: "project_0001",
      issue_id: "issue_0001",
      attempt_id: "coding_attempt_0001",
      attempt_scope: "work_item",
      work_item_group_id: null,
      current_work_item_id: "work_item_0001",
      active_unit_id: null,
      units: [],
      status: "running",
      stage: "coding",
      branch_name: "aria/work-items/work_item_0001/attempt-1",
      base_branch: "main",
      worktree_path: "/tmp/worktree",
      rework_count: 0,
      max_auto_rework: 2,
      head_commit: null,
      pushed_remote: null,
      provider_config_snapshot: {
        author: "fake",
        reviewer: "fake",
        review_rounds: 1,
      },
      role_provider_config_snapshot: {
        coder: "fake",
        code_reviewer: "fake",
        internal_reviewer: "fake",
        review_rounds: 1,
        permission_modes: { ...DEFAULT_PERMISSION_MODES },
      },
      timeline_nodes: [],
      active_node_id: null,
      code_review_reports: [],
      review_request: null,
      internal_pr_review: null,
      pending_gates: [],
      pending_choices: [],
      role_runs: [],
      chat_entries: [],
      work_item_markdown: null,
      verification_commands: [],
      work_item_execution_plan: null,
      linked_plan_repair: null,
      require_execution_plan_confirm: false,
      ...overrides,
    });
  }

  it("opens the coding audit drawer", async () => {
    const user = userEvent.setup();
    mockCodingWs();
    useCodingWorkspaceStore.setState({
      ...readyCodingState(),
      timelineNodes: [{
        id: "final_confirm_1",
        attempt_id: "coding_attempt_0001",
        stage: "final_confirm",
        title: "最终确认",
        status: "completed",
        agent_role: "system",
        summary: null,
        started_at: "2026-09-16T00:00:00.000Z",
        completed_at: "2026-09-16T00:01:00.000Z",
        artifact_refs: [],
      }],
    });

    render(<CodingWorkspacePage address={CODING_ATTEMPT_ADDRESS} onBack={vi.fn()} />);
    await user.click(screen.getByRole("button", { name: "操作审计" }));

    expect(screen.getByTestId("operation-audit-view")).toBeVisible();
    expect(screen.getByTestId("operation-audit-view")).toHaveTextContent("REST 快照");
  });

  // v40 复验 #2：coding 页根与 cockpit 页同族——h-screen 会与外壳
  // 「待处理」告警条（52px）叠加产生文档级滚动条；高度改由外壳高度链
  // 供给（h-full）。
  it("keeps the page height shell-owned (h-full, not h-screen)", () => {
    mockCodingWs();
    useCodingWorkspaceStore.setState({ ...readyCodingState() });

    render(<CodingWorkspacePage address={CODING_ATTEMPT_ADDRESS} onBack={vi.fn()} />);

    const root = screen.getByTestId("coding-workspace-page");
    expect(root.className).toContain("h-full");
    expect(root.className).not.toContain("h-screen");
  });

  it("renders coding workspace shell with timeline and keeps result tabs secondary until selected", async () => {
    mockCodingWs();
    vi.mocked(getCodingAttemptDiff).mockResolvedValue({
      attempt_id: "coding_attempt_0001",
      base_branch: "main",
      worktree_path: "/tmp/worktree",
      diff: "",
    });
    useCodingWorkspaceStore.setState({
      attemptId: "coding_attempt_0001",
      status: "running",
      stage: "code_review",
      activeTab: "logs",
      branchName: "aria/work-items/work_item_0001/attempt-1",
      baseBranch: "main",
      worktreePath: "/tmp/worktree",
      timelineNodes: [
        {
          id: "coding_node_0001",
          attempt_id: "coding_attempt_0001",
          stage: "code_review",
          title: "Code Review",
          status: "running",
          agent_role: "reviewer",
          summary: null,
          started_at: "2026-05-23T00:00:00Z",
          completed_at: null,
          artifact_refs: [],
        },
      ],
      activeNodeId: "coding_node_0001",
      selectedNodeId: "coding_node_0001",
      chatEntries: [
        {
          id: "entry-1",
          type: "execution_event",
          role: "system",
          content: "reviewing changes",
          timestamp: "2026-05-23T00:00:01Z",
          node_id: "coding_node_0001",
        },
      ],
      logs: [
        {
          id: "log_0001",
          message: "review command completed",
          timestamp: "2026-05-23T00:00:02Z",
        },
      ],
    });

    render(
      <CodingWorkspacePage
        address={CODING_ATTEMPT_ADDRESS}
        onBack={vi.fn()}
      />
    );

    expect(useCodingWorkspaceWs).toHaveBeenCalledWith(CODING_ATTEMPT_ADDRESS);
    expect(screen.getByText("Coding Attempt #coding_attempt_0001")).toBeInTheDocument();
    expect(screen.getByTestId("coding-timeline")).toHaveTextContent("Code Review");
    expect(screen.getByTestId("chat-entry-list")).toHaveTextContent("reviewing changes");
    expect(screen.queryByTestId("coding-artifact-tabs")).not.toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: "运行结果" }));

    expect(screen.getByTestId("coding-artifact-tabs")).toHaveTextContent(
      "review command completed",
    );
    expect(screen.getByTestId("coding-status-bar")).toHaveTextContent("Code Reviewer");
    expect(screen.getByTestId("coding-status-bar")).toHaveTextContent("Coder 修复次数 0/2");
  });

  it("shows group progress and current work item for group attempts", async () => {
    mockCodingWs();
    mockCodingSessionState({
      attempt_scope: "work_item_group",
      work_item_group_id: "work_item_plan_0001",
      current_work_item_id: "work_item_0001",
      active_unit_id: "coding_unit_0001",
      units: [
        {
          unit_id: "coding_unit_0001",
          logical_work_item_id: "work_item_0001",
          work_item_revision_id: "work_item_revision_0001",
          dependency_logical_work_item_ids: [],
          order_index: 0,
          status: "running",
          summary: null,
          latest_handoff_revision_id: null,
          completion_commit: null,
        },
        {
          unit_id: "coding_unit_0002",
          logical_work_item_id: "work_item_0002",
          work_item_revision_id: "work_item_revision_0002",
          dependency_logical_work_item_ids: ["work_item_0001"],
          order_index: 1,
          status: "pending",
          summary: null,
          latest_handoff_revision_id: null,
          completion_commit: null,
        },
      ],
    });

    render(
      <CodingWorkspacePage
        address={CODING_ATTEMPT_ADDRESS}
        onBack={vi.fn()}
      />
    );

    expect(await screen.findByText("WorkItemGroup")).toBeInTheDocument();
    expect(screen.getByText("1 / 2")).toBeInTheDocument();
    expect(screen.getAllByText("work_item_0001").length).toBeGreaterThan(0);
  });

  it("passes the current address to the plan aggregate and never starts the WS", async () => {
    const api = mockCodingWs();
    mockCodingSessionState({
      attempt_scope: "work_item_group",
      work_item_group_id: "work_item_plan_0001",
      current_work_item_id: "work_item_0001",
      active_unit_id: "coding_unit_0001",
      units: [
        {
          unit_id: "coding_unit_0001",
          logical_work_item_id: "work_item_0001",
          work_item_revision_id: "work_item_revision_0001",
          dependency_logical_work_item_ids: [],
          order_index: 0,
          status: "running",
          summary: null,
          latest_handoff_revision_id: null,
          completion_commit: null,
        },
      ],
    });
    vi.mocked(getPlanCodingProgress).mockResolvedValueOnce({
      plan_id: "work_item_plan_0001",
      bound_plan_revision_id: "plan_revision_0002",
      command_id: "plan_start_abc123",
      mode: "first_ready_unit",
      phase: "started",
      primary_attempt_id: "coding_attempt_0001",
      selected_unit_id: "coding_unit_0001",
      selected_attempt_id: "coding_attempt_0001",
      target_attempts: [
        {
          target_repository_id: "repo_busi",
          attempt_id: "coding_attempt_0001",
          attempt_status: "running",
          worktree_path: "/tmp/wt/busi",
          lease_state: "held",
          // FE-1：PlanCodingTargetDto 新增 S3 逐仓事实九字段（本夹具一律 null）。
          repository_name: null,
          branch_name: null,
          base_branch: null,
          stage: null,
          head_commit: null,
          push_status: null,
          push_error: null,
          review_request_id: null,
          manual_recovery_reason: null,
        },
      ],
      units: [
        {
          unit_id: "coding_unit_0001",
          logical_work_item_id: "work_item_0001",
          target_repository_id: "repo_busi",
          attempt_id: "coding_attempt_0001",
          unit_status: "running",
          execution_state: "running",
          unsatisfied_dependency_ids: [],
          waiting_reason: null,
          dependency_logical_work_item_ids: [],
          order_index: 0,
          summary: null,
        },
        {
          unit_id: "coding_unit_0005",
          logical_work_item_id: "work_item_0005",
          target_repository_id: "repo_frontend",
          attempt_id: "coding_attempt_0002",
          unit_status: "pending",
          execution_state: "waiting_dependency",
          unsatisfied_dependency_ids: ["work_item_0001"],
          waiting_reason: "dependency_not_ready",
          dependency_logical_work_item_ids: ["work_item_0001"],
          order_index: 1,
          summary: null,
        },
      ],
      plan_complete: false,
    });

    render(
      <CodingWorkspacePage address={CODING_ATTEMPT_ADDRESS} onBack={vi.fn()} />,
    );

    expect(await screen.findByTestId("plan-coding-progress")).toBeInTheDocument();
    expect(getPlanCodingProgress).toHaveBeenCalledWith(
      "project_0001",
      "issue_0001",
      "work_item_plan_0001",
    );
    expect(screen.getByTestId("plan-coding-progress")).toHaveTextContent(
      "等待依赖完成",
    );
    // selected attempt WS 仍是局部实时通道：聚合展示不触发任何 WS start。
    expect(api.startCoding).not.toHaveBeenCalled();
  });

  it("renders automatic retry history and waits for human handling after the third failure", async () => {
    mockCodingWs();
    mockCodingSessionState({
      role_runs: [
        {
          id: "coding_role_run_0001",
          attempt_id: "coding_attempt_0001",
          stage: "code_review",
          role: "code_reviewer",
          run_no: 1,
          status: "failed",
          trigger: "initial",
          retry_metadata: {
            cycle_id: "provider_retry_cycle_0001",
            attempt_no: 1,
            prior_run_id: null,
          },
          node_id: "coding_node_0001",
          started_at: "2026-06-13T00:00:00Z",
          completed_at: "2026-06-13T00:00:01Z",
          reason_code: "provider_503",
          raw_provider_output_refs: ["provider-raw/review_0001.txt"],
          artifact_refs: [],
        },
        {
          id: "coding_role_run_0002",
          attempt_id: "coding_attempt_0001",
          stage: "code_review",
          role: "code_reviewer",
          run_no: 2,
          status: "running",
          trigger: "automatic_retry",
          retry_metadata: {
            cycle_id: "provider_retry_cycle_0001",
            attempt_no: 2,
            prior_run_id: "coding_role_run_0001",
          },
          node_id: "coding_node_0002",
          started_at: "2026-06-13T00:00:02Z",
          completed_at: null,
          reason_code: null,
          raw_provider_output_refs: [],
          artifact_refs: [],
        },
      ],
    });

    render(<CodingWorkspacePage address={CODING_ATTEMPT_ADDRESS} onBack={vi.fn()} />);

    await userEvent.click(screen.getByRole("button", { name: "角色运行历史" }));

    expect(screen.getByText("第 2/3 次自动重试")).toBeInTheDocument();
    expect(screen.getByText("失败：provider_503")).toBeInTheDocument();
    await waitFor(() => {
      expect(screen.queryByText("自动重试已耗尽，等待人工处理")).not.toBeInTheDocument();
    });

    mockCodingSessionState({
      status: "blocked",
      role_runs: [
        {
          id: "coding_role_run_0003",
          attempt_id: "coding_attempt_0001",
          stage: "code_review",
          role: "code_reviewer",
          run_no: 3,
          status: "failed",
          trigger: "automatic_retry",
          retry_metadata: {
            cycle_id: "provider_retry_cycle_0001",
            attempt_no: 3,
            prior_run_id: "coding_role_run_0002",
          },
          node_id: "coding_node_0003",
          started_at: "2026-06-13T00:00:03Z",
          completed_at: "2026-06-13T00:00:04Z",
          reason_code: "provider_503",
          raw_provider_output_refs: ["provider-raw/review_0003.txt"],
          artifact_refs: [],
        },
      ],
    });

    await waitFor(() => {
      expect(screen.getByText("第 3/3 次自动重试")).toBeInTheDocument();
      expect(screen.queryByText("自动重试已耗尽，等待人工处理")).not.toBeInTheDocument();
    });

    mockCodingSessionState({
      status: "blocked",
      role_runs: [
        {
          id: "coding_role_run_0003",
          attempt_id: "coding_attempt_0001",
          stage: "code_review",
          role: "code_reviewer",
          run_no: 3,
          status: "failed",
          trigger: "automatic_retry",
          retry_metadata: {
            cycle_id: "provider_retry_cycle_0001",
            attempt_no: 3,
            prior_run_id: "coding_role_run_0002",
          },
          node_id: "coding_node_0003",
          started_at: "2026-06-13T00:00:03Z",
          completed_at: "2026-06-13T00:00:04Z",
          reason_code: "provider_503",
          raw_provider_output_refs: ["provider-raw/review_0003.txt"],
          artifact_refs: [],
        },
      ],
      pending_gates: [
        {
          gate_id: "coding_blocked_gate_0001",
          kind: "blocked",
          title: "Provider 重试已耗尽",
          description: "等待人工处理",
          stage: "code_review",
          role: "code_reviewer",
          reason_code: "code_review_provider_interrupted",
          available_actions: [
            {
              action_id: "retry_review",
              label: "重试代码审查",
              action_type: "retry_review",
            },
          ],
        },
      ],
    });

    expect(await screen.findByText("自动重试已耗尽，等待人工处理")).toBeInTheDocument();
  });

  it("loads and renders the coding attempt git diff in result tabs", async () => {
    mockCodingWs();
    vi.mocked(getCodingAttemptDiff).mockResolvedValue({
      attempt_id: "coding_attempt_0001",
      base_branch: "main",
      worktree_path: "/tmp/worktree",
      diff: [
        "diff --git a/climbing_stairs.py b/climbing_stairs.py",
        "new file mode 100644",
        "index 0000000..a56d173",
        "--- /dev/null",
        "+++ b/climbing_stairs.py",
        "@@ -0,0 +1,2 @@",
        "+def climb_stairs(n):",
        "+    return n",
      ].join("\n"),
    });
    useCodingWorkspaceStore.setState({
      attemptId: "coding_attempt_0001",
      status: "completed",
      stage: "final_confirm",
      activeTab: "diff",
      branchName: "aria/work-items/work_item_0001/attempt-1",
      baseBranch: "main",
      worktreePath: "/tmp/worktree",
    });

    render(
      <CodingWorkspacePage
        address={CODING_ATTEMPT_ADDRESS}
        onBack={vi.fn()}
      />
    );

    await userEvent.click(screen.getByRole("button", { name: "运行结果" }));

    await waitFor(() => {
      expect(getCodingAttemptDiff).toHaveBeenCalledWith(CODING_ATTEMPT_ADDRESS);
    });
    const viewer = await screen.findByTestId("monaco-diff-viewer");
    expect(viewer).toHaveAttribute("data-language", "python");
    expect(screen.getByText("climbing_stairs.py")).toBeInTheDocument();
    expect(screen.getByTestId("monaco-diff-original").textContent).toBe("");
    expect(screen.getByTestId("monaco-diff-modified").textContent).toBe(
      "def climb_stairs(n):\n    return n",
    );
  });

  it("reloads diff by full address and hides the previous scoped result", async () => {
    mockCodingWs();
    let resolveSecondDiff:
      | ((value: Awaited<ReturnType<typeof getCodingAttemptDiff>>) => void)
      | undefined;
    const secondDiff = new Promise<
      Awaited<ReturnType<typeof getCodingAttemptDiff>>
    >((resolve) => {
      resolveSecondDiff = resolve;
    });
    vi.mocked(getCodingAttemptDiff)
      .mockResolvedValueOnce({
        attempt_id: "coding_attempt_0001",
        base_branch: "main",
        worktree_path: "/tmp/worktree-a",
        diff: [
          "diff --git a/scope.txt b/scope.txt",
          "--- a/scope.txt",
          "+++ b/scope.txt",
          "@@ -1 +1 @@",
          "-old",
          "+project-one-result",
        ].join("\n"),
      })
      .mockReturnValueOnce(secondDiff);
    useCodingWorkspaceStore.setState({
      projectId: CODING_ATTEMPT_ADDRESS.projectId,
      issueId: CODING_ATTEMPT_ADDRESS.issueId,
      attemptId: CODING_ATTEMPT_ADDRESS.attemptId,
      status: "completed",
      stage: "final_confirm",
      activeTab: "diff",
    });

    const view = render(
      <CodingWorkspacePage
        address={CODING_ATTEMPT_ADDRESS}
        onBack={vi.fn()}
      />,
    );
    await userEvent.click(screen.getByRole("button", { name: "运行结果" }));
    expect(await screen.findByText("project-one-result")).toBeInTheDocument();

    view.rerender(
      <CodingWorkspacePage
        address={{
          projectId: "project_0002",
          issueId: "issue_0002",
          attemptId: "coding_attempt_0001",
        }}
        onBack={vi.fn()}
      />,
    );

    await waitFor(() =>
      expect(getCodingAttemptDiff).toHaveBeenNthCalledWith(2, {
        projectId: "project_0002",
        issueId: "issue_0002",
        attemptId: "coding_attempt_0001",
      }),
    );
    expect(screen.queryByText("project-one-result")).not.toBeInTheDocument();

    resolveSecondDiff?.({
      attempt_id: "coding_attempt_0001",
      base_branch: "main",
      worktree_path: "/tmp/worktree-b",
      diff: "",
    });
    expect(await screen.findByText("暂无代码变更")).toBeInTheDocument();
  });

  it("scrolls the chat list to the first entry for a selected timeline node", async () => {
    const scrollIntoView = vi.fn();
    Object.defineProperty(HTMLElement.prototype, "scrollIntoView", {
      configurable: true,
      value: scrollIntoView,
    });
    mockCodingWs();
    useCodingWorkspaceStore.setState({
      attemptId: "coding_attempt_0001",
      status: "running",
      stage: "code_review",
      timelineNodes: [
        {
          id: "coding_node_0001",
          attempt_id: "coding_attempt_0001",
          stage: "coding",
          title: "代码编写",
          status: "completed",
          agent_role: "author",
          summary: "完成",
          started_at: "2026-05-23T00:00:00Z",
          completed_at: "2026-05-23T00:01:00Z",
          artifact_refs: [],
        },
        {
          id: "coding_node_0002",
          attempt_id: "coding_attempt_0001",
          stage: "code_review",
          title: "Code Review",
          status: "running",
          agent_role: "reviewer",
          summary: null,
          started_at: "2026-05-23T00:01:00Z",
          completed_at: null,
          artifact_refs: [],
        },
      ],
      chatEntries: [
        {
          id: "entry-coding",
          type: "provider_stream",
          role: "coder",
          content: "实现完成",
          timestamp: "2026-05-23T00:00:30Z",
          node_id: "coding_node_0001",
        },
        {
          id: "entry-review",
          type: "provider_stream",
          role: "code_reviewer",
          content: "审查中",
          timestamp: "2026-05-23T00:01:30Z",
          node_id: "coding_node_0002",
        },
      ],
    });

    render(
      <CodingWorkspacePage
        address={CODING_ATTEMPT_ADDRESS}
        onBack={vi.fn()}
      />
    );
    scrollIntoView.mockClear();
    await userEvent.click(screen.getByRole("button", { name: /Code Review/ }));

    expect(useCodingWorkspaceStore.getState().selectedNodeId).toBe("coding_node_0002");
    expect(scrollIntoView).toHaveBeenCalled();
  });

  it("starts coding from prepare context", async () => {
    const api = mockCodingWs();
    useCodingWorkspaceStore.setState({
      attemptId: "coding_attempt_0001",
      status: "created",
      stage: "prepare_context",
    });

    render(
      <CodingWorkspacePage
        address={CODING_ATTEMPT_ADDRESS}
        onBack={vi.fn()}
      />
    );

    await userEvent.click(screen.getByRole("button", { name: "开始 Coding" }));

    expect(api.startCoding).toHaveBeenCalled();
  });


  it("does not disable startup for a non-start command sharing coding_message_not_allowed", () => {
    mockCodingWs();
    useCodingWorkspaceStore.setState({
      ...readyCodingState(),
      status: "created",
      stage: "prepare_context",
      protocolError: { code: "coding_message_not_allowed", message: "context note rejected" },
    });

    render(<CodingWorkspacePage address={CODING_ATTEMPT_ADDRESS} onBack={vi.fn()} />);

    expect(screen.queryByRole("status")).toBeNull();
    expect(screen.getByRole("button", { name: "开始 Coding" })).not.toBeDisabled();
  });

  it("uses the same exported mapping object to start coding from prepare context", () => {
    const api = mockCodingWs();
    useCodingWorkspaceStore.setState({
      ...readyCodingState(),
      status: "created",
      stage: "prepare_context",
    });

    render(<CodingWorkspacePage address={CODING_ATTEMPT_ADDRESS} onBack={vi.fn()} />);
    fireEvent.keyDown(document, { code: COCKPIT_HOTKEYS.advance.code, ctrlKey: true });

    expect(api.startCoding).toHaveBeenCalledOnce();
    expect(CODING_WORKSPACE_HOTKEYS).toBe(COCKPIT_HOTKEYS);
  });

  it("does not dispatch hotkeys from the coding composer textarea", () => {
    const api = mockCodingWs();
    useCodingWorkspaceStore.setState({
      ...readyCodingState(),
      status: "created",
      stage: "prepare_context",
    });

    render(<CodingWorkspacePage address={CODING_ATTEMPT_ADDRESS} onBack={vi.fn()} />);
    fireEvent.keyDown(screen.getByLabelText("补充 Coding 上下文"), {
      code: COCKPIT_HOTKEYS.advance.code,
      ctrlKey: true,
    });

    expect(api.startCoding).not.toHaveBeenCalled();
  });

  it("resumes coding from review request when a group unit needs recovery", async () => {
    const api = mockCodingWs();
    useCodingWorkspaceStore.setState({
      attemptId: "coding_attempt_0001",
      status: "running",
      stage: "review_request",
    });

    render(
      <CodingWorkspacePage
        address={CODING_ATTEMPT_ADDRESS}
        onBack={vi.fn()}
      />
    );

    await userEvent.click(screen.getByRole("button", { name: "继续 Coding" }));

    expect(api.startCoding).toHaveBeenCalled();
  });

  it("shows dependency completion commit in execution plan", () => {
    mockCodingWs();
    useCodingWorkspaceStore.setState({
      ...readyCodingState(),
      stage: "prepare_context",
      workItemExecutionPlan: executionPlan({
        dependency_handoffs: [
          {
            work_item_id: "work_item_0001",
            commit_sha: "abc123",
          },
        ],
      }),
    });

    render(
      <CodingWorkspacePage
        address={{
          ...CODING_ATTEMPT_ADDRESS,
          attemptId: "coding_attempt_0002",
        }}
        onBack={vi.fn()}
      />,
    );

    expect(screen.getByText("abc123")).toBeInTheDocument();
  });

  // UI-2（选 A）：单删入口（「仅删除当前 Coding 工作区」）已删除——顶栏只剩
  // 状态/连接/审计/设置；单仓 scope 下页内不再有任何删除入口。
  it("no longer renders a single attempt delete entry", () => {
    mockCodingWs();
    useCodingWorkspaceStore.setState({
      projectId: CODING_ATTEMPT_ADDRESS.projectId,
      issueId: CODING_ATTEMPT_ADDRESS.issueId,
      attemptId: CODING_ATTEMPT_ADDRESS.attemptId,
      status: "running",
      stage: "coding",
    });

    render(
      <CodingWorkspacePage address={CODING_ATTEMPT_ADDRESS} onBack={vi.fn()} />,
    );

    expect(
      screen.queryByRole("button", { name: "仅删除当前 Coding 工作区" }),
    ).not.toBeInTheDocument();
    // 全量入口只属于 plan 聚合标题区（group scope），单仓 scope 不渲染。
    expect(screen.queryByTestId("plan-coding-delete-all")).not.toBeInTheDocument();
  });

  it("disables final confirm for incomplete readiness but leaves abort available", () => {
    mockCodingWs();
    useCodingWorkspaceStore.setState({
      attemptId: "coding_attempt_0001",
      status: "waiting_for_human",
      stage: "final_confirm",
      groupFinalReadiness: {
        attempt_id: "coding_attempt_0001",
        status: "incomplete",
        units: [],
        diagnostics: [
          {
            kind: "code_review_missing",
            unit_id: "coding_unit_0001",
            message: "C1 缺少独立代码审查",
          },
        ],
        created_at: "2026-08-07T00:00:00Z",
      },
    });

    render(<CodingWorkspacePage address={CODING_ATTEMPT_ADDRESS} onBack={vi.fn()} />);

    expect(screen.getByRole("button", { name: "确认完成" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "中止" })).toBeEnabled();
    expect(screen.getByText("C1 缺少独立代码审查")).toBeInTheDocument();
  });

  it("sends final confirm and abort actions when readiness is complete", async () => {
    const api = mockCodingWs();
    useCodingWorkspaceStore.setState({
      attemptId: "coding_attempt_0001",
      status: "waiting_for_human",
      stage: "final_confirm",
      groupFinalReadiness: {
        attempt_id: "coding_attempt_0001",
        status: "complete",
        units: [],
        diagnostics: [],
        created_at: "2026-08-07T00:00:00Z",
      },
    });

    render(
      <CodingWorkspacePage
        address={CODING_ATTEMPT_ADDRESS}
        onBack={vi.fn()}
      />
    );

    await userEvent.click(screen.getByRole("button", { name: "确认完成" }));
    await userEvent.click(screen.getByRole("button", { name: "中止" }));

    expect(api.finalConfirm).toHaveBeenCalled();
    expect(api.abortAttempt).toHaveBeenCalled();
  });

  // P3 WIGA Task 4（tasks.md §4.2 / D5）：readiness Complete 且
  // waiting_for_human/final_confirm 时明确「编码执行完成，待最终确认」并
  // 有人手确认按钮；完成横幅只在人手确认（completed）后出现，unit 全完成
  // 不提前冒称完成。
  it("labels the waiting final-confirm state and completes only after human confirmation", async () => {
    mockCodingWs();
    useCodingWorkspaceStore.setState({
      attemptId: "coding_attempt_0001",
      attemptScope: "work_item_group",
      status: "waiting_for_human",
      stage: "final_confirm",
      units: dashboardUnits().map((unit) => ({ ...unit, status: "completed" as const })),
      groupFinalReadiness: {
        attempt_id: "coding_attempt_0001",
        status: "complete",
        units: [],
        diagnostics: [],
        created_at: "2026-08-07T00:00:00Z",
      },
    });

    render(<CodingWorkspacePage address={CODING_ATTEMPT_ADDRESS} onBack={vi.fn()} />);

    expect(screen.getByText("编码执行完成，待最终确认")).toBeInTheDocument();
    expect(screen.queryByText("组级 Coding Workspace 已完成")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "确认完成" })).toBeEnabled();

    await userEvent.click(screen.getByRole("button", { name: "确认完成" }));

    // 人手确认回执（status=completed）后才显示完成横幅；等待文案消失。
    act(() => {
      useCodingWorkspaceStore.setState({ status: "completed" });
    });
    expect(screen.getByText("组级 Coding Workspace 已完成")).toBeInTheDocument();
    expect(screen.queryByText("编码执行完成，待最终确认")).not.toBeInTheDocument();
  });

  it("shows abort action for awaiting_manual_recovery status", async () => {
    const api = mockCodingWs();
    useCodingWorkspaceStore.setState({
      attemptId: "coding_attempt_0001",
      status: "awaiting_manual_recovery",
      stage: "code_review",
    });

    render(
      <CodingWorkspacePage
        address={CODING_ATTEMPT_ADDRESS}
        onBack={vi.fn()}
      />
    );

    const abortButton = screen.getByRole("button", { name: "中止" });
    expect(abortButton).toBeEnabled();

    await userEvent.click(abortButton);
    expect(api.abortAttempt).toHaveBeenCalled();
  });

  it("renders pending gate actions and sends gate responses", async () => {
    const api = mockCodingWs();
    useCodingWorkspaceStore.setState({
      attemptId: "coding_attempt_0001",
      status: "blocked",
      stage: "code_review",
      pendingGates: [
        {
          gate_id: "gate_0001",
          kind: "blocked",
          title: "需要人工处理",
          description: "Code Reviewer 被阻塞，等待人工处理",
          reason_code: "code_review_blocked",
          available_actions: [
            {
              action_id: "accept_risk",
              label: "接受风险",
              action_type: "accept_risk",
            },
            {
              action_id: "abort",
              label: "中止 Attempt",
              action_type: "abort",
            },
          ],
        },
      ],
    });

    render(
      <CodingWorkspacePage
        address={CODING_ATTEMPT_ADDRESS}
        onBack={vi.fn()}
      />
    );

    expect(screen.getByTestId("coding-pending-gate")).toHaveTextContent("需要人工处理");
    expect(screen.getAllByText("Code Reviewer").length).toBeGreaterThan(0);
    expect(screen.queryByText(/^rework$/)).not.toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: "中止 Attempt" }));

    expect(api.respondGate).toHaveBeenCalledWith("gate_0001", "abort", undefined);
  });

  it("shows completion banner when a group attempt is completed", () => {
    mockCodingWs();
    useCodingWorkspaceStore.setState({
      attemptId: "coding_attempt_0001",
      attemptScope: "work_item_group",
      status: "completed",
      stage: "final_confirm",
      units: [
        {
          unit_id: "coding_unit_0001",
          logical_work_item_id: "work_item_0001",
          work_item_revision_id: "work_item_revision_0001",
          dependency_logical_work_item_ids: [],
          order_index: 0,
          status: "completed",
          summary: null,
          latest_handoff_revision_id: null,
          completion_commit: null,
        },
        {
          unit_id: "coding_unit_0002",
          logical_work_item_id: "work_item_0002",
          work_item_revision_id: "work_item_revision_0002",
          dependency_logical_work_item_ids: [],
          order_index: 1,
          status: "completed",
          summary: null,
          latest_handoff_revision_id: null,
          completion_commit: null,
        },
      ],
      headCommit: "abcdef1234567890abcdef1234567890abcdef12",
    });

    render(<CodingWorkspacePage address={CODING_ATTEMPT_ADDRESS} onBack={vi.fn()} />);

    expect(screen.getByText("组级 Coding Workspace 已完成")).toBeInTheDocument();
    expect(screen.getByText("2 个 Work Item 已完成并确认")).toBeInTheDocument();
    expect(screen.getByText(/最终提交 abcdef123456/)).toBeInTheDocument();
  });

  function dashboardUnits(): CodingExecutionUnit[] {
    const base = {
      work_item_revision_id: "rev_0001",
      dependency_logical_work_item_ids: [] as string[],
      summary: null,
      latest_handoff_revision_id: null,
      completion_commit: null,
    };
    return [
      { ...base, unit_id: "u_a", logical_work_item_id: "wi_a", order_index: 0, status: "completed" as const },
      { ...base, unit_id: "u_b", logical_work_item_id: "wi_b", order_index: 1, status: "running" as const, dependency_logical_work_item_ids: ["wi_a"] },
      { ...base, unit_id: "u_c", logical_work_item_id: "wi_c", order_index: 2, status: "blocked" as const, dependency_logical_work_item_ids: ["wi_b"] },
    ];
  }

  it("renders the coding dashboard with topology, dependency chain, budget gates and log console", async () => {
    mockCodingWs();
    useCodingWorkspaceStore.setState({
      ...readyCodingState(),
      attemptScope: "work_item_group",
      workItemGroupId: "work_item_group_0001",
      currentWorkItemId: "wi_b",
      units: dashboardUnits(),
      timelineNodes: [
        {
          id: "node_coding_1",
          attempt_id: "coding_attempt_0001",
          stage: "coding",
          title: "Coder",
          status: "running",
          agent_role: "author",
          summary: null,
          started_at: new Date(Date.now() - 1_800_000).toISOString(),
          completed_at: null,
          artifact_refs: [],
        },
      ],
      status: "running",
      stage: "coding",
    });
    render(<CodingWorkspacePage address={CODING_ATTEMPT_ADDRESS} onBack={() => undefined} />);

    expect(screen.getByTestId("coding-dashboard")).toBeInTheDocument();
    expect(screen.getByTestId("coding-topology-graph")).toBeInTheDocument();
    expect(screen.getByTestId("coding-dependency-chain")).toBeInTheDocument();
    expect(screen.getByTestId("coding-budget-gates")).toBeInTheDocument();
    expect(screen.getByTestId("coding-log-console")).toBeInTheDocument();
    // 当前 work item 的依赖链默认展开：wi_b 的下游 wi_c 可见
    expect(screen.getByTestId("coding-chain-downstream")).toHaveTextContent("wi_c");
    // 预算门自计时可见且口径免责在场
    expect(screen.getByTestId("coding-budget-disclaimer")).toBeInTheDocument();
  });

  it("collapses the dashboard without unmounting the page", async () => {
    const user = userEvent.setup();
    mockCodingWs();
    useCodingWorkspaceStore.setState({ ...readyCodingState(), units: dashboardUnits(), attemptScope: "work_item_group", workItemGroupId: "work_item_group_0001" });
    render(<CodingWorkspacePage address={CODING_ATTEMPT_ADDRESS} onBack={() => undefined} />);

    await user.click(screen.getByRole("button", { name: "折叠编码仪表盘" }));
    expect(screen.queryByTestId("coding-topology-graph")).toBeNull();
    expect(screen.getByTestId("coding-dashboard")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "展开编码仪表盘" })).toBeInTheDocument();
  });
});

describe("CodingWorkspacePage P1-c navigation and focus", () => {
  installCodingWorkspacePageTestHooks();

  function mockCodingSessionState(
    overrides: Partial<Extract<CodingWsOutMessage, { type: "coding_session_state" }>>,
  ) {
    useCodingWorkspaceStore.getState().setSessionState({
      type: "coding_session_state",
      project_id: "project_0001",
      issue_id: "issue_0001",
      attempt_id: "coding_attempt_0001",
      attempt_scope: "work_item",
      work_item_group_id: null,
      current_work_item_id: "work_item_0001",
      active_unit_id: null,
      units: [],
      status: "running",
      stage: "coding",
      branch_name: "aria/work-items/work_item_0001/attempt-1",
      base_branch: "main",
      worktree_path: "/tmp/worktree",
      rework_count: 0,
      max_auto_rework: 2,
      head_commit: null,
      pushed_remote: null,
      provider_config_snapshot: {
        author: "fake",
        reviewer: "fake",
        review_rounds: 1,
      },
      role_provider_config_snapshot: {
        coder: "fake",
        code_reviewer: "fake",
        internal_reviewer: "fake",
        review_rounds: 1,
        permission_modes: { ...DEFAULT_PERMISSION_MODES },
      },
      timeline_nodes: [],
      active_node_id: null,
      code_review_reports: [],
      review_request: null,
      internal_pr_review: null,
      pending_gates: [],
      pending_choices: [],
      role_runs: [],
      chat_entries: [],
      work_item_markdown: null,
      verification_commands: [],
      work_item_execution_plan: null,
      linked_plan_repair: null,
      require_execution_plan_confirm: false,
      ...overrides,
    });
  }

  function blockedGateFixture() {
    return {
      gate_id: "gate_0001",
      kind: "blocked" as const,
      title: "需要人工处理",
      description: "Code Reviewer 被阻塞，等待人工处理",
      reason_code: "code_review_blocked",
      available_actions: [
        {
          action_id: "accept_risk",
          label: "接受风险",
          action_type: "accept_risk" as const,
        },
      ],
    };
  }

  // P1-c（REQ-MTG-12）红测③：focus.gateId 从「运行结果」页签下钻——切回
  // 对话页签并定位到门卡（GatePanel 只在对话页签渲染）。
  it("locates the focused pending gate card from the results panel", async () => {
    const scrolled = spyScrollIntoView();
    mockCodingWs();
    useCodingWorkspaceStore.setState({
      ...readyCodingState(),
      status: "blocked",
      stage: "code_review",
      pendingGates: [blockedGateFixture()],
    });
    vi.mocked(getCodingAttemptDiff).mockResolvedValue({
      attempt_id: CODING_ATTEMPT_ADDRESS.attemptId,
      base_branch: "main",
      worktree_path: "/tmp/worktree",
      diff: "",
    });

    const view = render(
      <CodingWorkspacePage address={CODING_ATTEMPT_ADDRESS} onBack={vi.fn()} />,
    );
    await userEvent.click(screen.getByRole("button", { name: "运行结果" }));
    expect(screen.queryByTestId("coding-pending-gate")).toBeNull();

    view.rerender(
      <CodingWorkspacePage
        address={CODING_ATTEMPT_ADDRESS}
        onBack={vi.fn()}
        focus={{ gateId: "gate_0001" }}
      />,
    );

    const card = await screen.findByTestId("coding-pending-gate");
    expect(screen.getByTestId("coding-chat-entry-list")).toBeInTheDocument();
    expect(scrolled).toContain(card);
  });

  // P1-c 红测③：final_confirm 待办下钻——定位「待最终确认」确认区横幅。
  it("locates the final-confirm waiting banner for a final_confirm focus target", async () => {
    const scrolled = spyScrollIntoView();
    mockCodingWs();
    useCodingWorkspaceStore.setState({
      ...readyCodingState(),
      attemptScope: "work_item_group",
      status: "waiting_for_human",
      stage: "final_confirm",
    });

    render(
      <CodingWorkspacePage
        address={CODING_ATTEMPT_ADDRESS}
        onBack={vi.fn()}
        focus={{ attentionKind: "final_confirm" }}
      />,
    );

    const banner = await screen.findByTestId("coding-final-confirm-waiting");
    expect(banner).toHaveTextContent("编码执行完成，待最终确认");
    expect(scrolled).toContain(banner);
  });

  // P1-c 红测④前置：GroupProgress「查看全部 WI」入口由页面透传回调（router
  // 接线导航到总览路由；页面自身不感知路由）。
  it("forwards the group progress overview entry through onOpenPlanOverview", async () => {
    const onOpenPlanOverview = vi.fn();
    mockCodingWs();
    mockCodingSessionState({
      attempt_scope: "work_item_group",
      work_item_group_id: "work_item_plan_0001",
      current_work_item_id: "work_item_0001",
      active_unit_id: "coding_unit_0001",
      units: [
        {
          unit_id: "coding_unit_0001",
          logical_work_item_id: "work_item_0001",
          work_item_revision_id: "work_item_revision_0001",
          dependency_logical_work_item_ids: [],
          order_index: 0,
          status: "running",
          summary: null,
          latest_handoff_revision_id: null,
          completion_commit: null,
        },
      ],
    });

    render(
      <CodingWorkspacePage
        address={CODING_ATTEMPT_ADDRESS}
        onBack={vi.fn()}
        onOpenPlanOverview={onOpenPlanOverview}
      />,
    );

    await userEvent.click(await screen.findByTestId("plan-coding-open-overview"));
    expect(onOpenPlanOverview).toHaveBeenCalledWith("work_item_plan_0001");
  });
});

describe("P1-c A1 attention target navigation", () => {
  function attentionItem(
    overrides: Partial<CodingAttentionItemDto> = {},
  ): CodingAttentionItemDto {
    return {
      key: "gate:gate_0001",
      kind: "choice",
      project_id: "project_1",
      issue_id: "issue_1",
      plan_id: "plan_1",
      attempt_id: "attempt_1",
      logical_work_item_id: "wi_1",
      repository_name: null,
      reason_code: null,
      title: "待处理",
      summary: "等待人工处理",
      occurred_at: "2026-10-16T00:00:00Z",
      target: { type: "coding_attempt", attempt_id: "attempt_1" },
      focus: { gate_id: "gate_1", choice_id: "choice_1" },
      ...overrides,
    };
  }

  // P1-c 红测①（地址组装）：A1 coding_attempt target 统一走既有 coding
  // 路由，focus/gateId/choiceId/planId/workItemId 全部经 search 传递。
  it("builds the coding attempt location with focus search for an A1 target", () => {
    expect(codingAttentionLocation(attentionItem())).toEqual({
      to: "/workbench/projects/$projectId/issues/$issueId/coding/$attemptId",
      params: { projectId: "project_1", issueId: "issue_1", attemptId: "attempt_1" },
      search: {
        planId: "plan_1",
        workItemId: "wi_1",
        gateId: "gate_1",
        choiceId: "choice_1",
      },
    });
  });

  it("marks a final_confirm attention item with the confirm focus", () => {
    expect(
      codingAttentionLocation(
        attentionItem({
          kind: "final_confirm",
          key: "final_confirm:attempt_1",
          focus: null,
        }),
      ),
    ).toEqual({
      to: "/workbench/projects/$projectId/issues/$issueId/coding/$attemptId",
      params: { projectId: "project_1", issueId: "issue_1", attemptId: "attempt_1" },
      search: { planId: "plan_1", workItemId: "wi_1", focus: "final_confirm" },
    });
  });

  it("builds the plan overview location for coding_plan targets", () => {
    expect(
      codingAttentionLocation(
        attentionItem({
          kind: "plan_final_confirm",
          key: "plan_final_confirm:plan_1",
          attempt_id: null,
          logical_work_item_id: null,
          target: { type: "coding_plan", plan_id: "plan_1" },
          focus: null,
        }),
      ),
    ).toEqual({
      to: "/workbench/projects/$projectId/issues/$issueId/coding-plans/$planId",
      params: { projectId: "project_1", issueId: "issue_1", planId: "plan_1" },
      search: {},
    });
  });

  it("builds the workspace session location for workspace_session targets", () => {
    expect(
      codingAttentionLocation(
        attentionItem({
          target: { type: "workspace_session", session_id: "session_1" },
          focus: null,
        }),
      ),
    ).toEqual({
      to: "/workbench/workspace/$sessionId",
      params: { sessionId: "session_1" },
      search: {},
    });
  });

  it("returns null for unknown or incomplete targets", () => {
    expect(
      codingAttentionLocation(
        attentionItem({ target: { type: "something_else", attempt_id: "x" } }),
      ),
    ).toBeNull();
    expect(
      codingAttentionLocation(attentionItem({ target: { type: "coding_attempt" } })),
    ).toBeNull();
  });

  it("registers the plan overview route", () => {
    expect(
      router.routesByPath[
        "/workbench/projects/$projectId/issues/$issueId/coding-plans/$planId"
      ],
    ).toBeDefined();
  });
});

describe("P1-c router integration", () => {
  installCodingWorkspacePageTestHooks();

  beforeEach(() => {
    vi.mocked(useWorkspaceSessionObservers).mockReturnValue({
      records: [],
      inbox: [],
      countedInbox: [],
      watchedSessionIds: [],
      watchSession: vi.fn(),
      codingAttemptForSession: () => null,
      displayItems: [],
      actionableCount: 0,
      notificationCandidates: [],
    });
  });


  function availableProviderHealth(): ProviderHealthResponse {
    return {
      schema_version: 1,
      generation: 1,
      checked_at: "2026-10-16T00:00:00Z",
      state_status: "ready",
      state_error: null,
      real_workflow_blocked: false,
      test_provider_enabled: false,
      providers: [],
    };
  }

  function loadProviderAvailability() {
    const health = availableProviderHealth();
    useProviderAvailabilityStore.setState({
      snapshot: health,
      loadStatus: "loaded",
      generation: health.generation,
      stateStatus: health.state_status,
      stateError: health.state_error,
      realWorkflowBlocked: health.real_workflow_blocked,
      testProviderEnabled: health.test_provider_enabled,
    });
  }

  // P1-c 红测①②：带 planId/focus search 的 coding URL 落到正确 attempt 地址
  // （WS 以该地址重置/重连），final_confirm 焦点定位确认区；红测④：返回
  // 按钮回到 plan 总览路由（planId 上下文）。
  it("routes a focused coding URL and returns to the plan overview", async () => {
    const scrolled = spyScrollIntoView();
    loadProviderAvailability();
    mockCodingWs();
    useCodingWorkspaceStore.setState({
      ...readyCodingState(),
      attemptScope: "work_item_group",
      status: "waiting_for_human",
      stage: "final_confirm",
    });

    const history = createMemoryHistory({
      initialEntries: [
        "/workbench/projects/project_0001/issues/issue_0001/coding/coding_attempt_0001" +
          "?planId=work_item_plan_0001&workItemId=work_item_0001&focus=final_confirm",
      ],
    });
    render(<RouterProvider router={createAppRouter(history)} />);

    expect(await screen.findByTestId("coding-workspace-page")).toBeInTheDocument();
    expect(vi.mocked(useCodingWorkspaceWs)).toHaveBeenCalledWith(
      expect.objectContaining({ attemptId: "coding_attempt_0001" }),
    );
    const banner = await screen.findByTestId("coding-final-confirm-waiting");
    expect(scrolled).toContain(banner);

    await userEvent.click(screen.getByRole("button", { name: "返回" }));

    await waitFor(() => {
      expect(history.location.pathname).toBe(
        "/workbench/projects/project_0001/issues/issue_0001/coding-plans/work_item_plan_0001",
      );
    });
    // getPlanCodingProgress 默认 mock 抛错——总览页错误面即“已回到总览”证据。
    expect(await screen.findByTestId("plan-overview-error")).toBeInTheDocument();
  });
});
