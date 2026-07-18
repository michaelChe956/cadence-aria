import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { CodingWsOutMessage } from "../api/types";
import {
  confirmWorkItemExecutionPlan,
  deleteCodingAttempt,
  getCodingAttemptDiff,
  requestWorkItemExecutionPlanChange,
} from "../api/client";
import { useCodingWorkspaceWs } from "../hooks/useCodingWorkspaceWs";
import { useCodingWorkspaceStore } from "../state/coding-workspace-store";
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
  deleteCodingAttempt: vi.fn(),
  getCodingAttemptDiff: vi.fn(),
  requestWorkItemExecutionPlanChange: vi.fn(),
}));

vi.mock("../hooks/useCodingWorkspaceWs", () => ({
  useCodingWorkspaceWs: vi.fn(),
}));

vi.mock("../hooks/useUnloadGuard", () => ({
  useUnloadGuard: vi.fn(),
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

describe("CodingWorkspacePage shell and actions", () => {
  installCodingWorkspacePageTestHooks();

  const OTHER_CODING_ATTEMPT_ADDRESS = {
    projectId: "project_0002",
    issueId: "issue_0002",
    attemptId: "coding_attempt_0002",
  } as const;

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
        tester_plan: "fake",
      tester_execute: "fake",
        code_reviewer: "fake",
        internal_reviewer: "fake",
        review_rounds: 1,
        permission_modes: { ...DEFAULT_PERMISSION_MODES },
      },
      timeline_nodes: [],
      active_node_id: null,
      testing_report: null,
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
      work_item_handoff: null,
      require_execution_plan_confirm: false,
      ...overrides,
    });
  }

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
      stage: "testing",
      activeTab: "tests",
      branchName: "aria/work-items/work_item_0001/attempt-1",
      baseBranch: "main",
      worktreePath: "/tmp/worktree",
      timelineNodes: [
        {
          id: "coding_node_0001",
          attempt_id: "coding_attempt_0001",
          stage: "testing",
          title: "执行测试",
          status: "running",
          agent_role: "tester",
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
          content: "cargo test",
          timestamp: "2026-05-23T00:00:01Z",
          node_id: "coding_node_0001",
        },
      ],
      testingReport: {
        id: "testing_report_0001",
        attempt_id: "coding_attempt_0001",
        overall_status: "passed",
        provider_claim: null,
        backend_verified: true,
        started_at: "2026-05-23T00:00:00Z",
        completed_at: "2026-05-23T00:00:02Z",
        commands: [
          {
            command: ["cargo", "test"],
            cwd: "/tmp/worktree",
            exit_code: 0,
            duration_ms: 100,
            stdout_ref: "stdout.log",
            stderr_ref: "stderr.log",
            status: "passed",
          },
        ],
      },
    });

    render(
      <CodingWorkspacePage
        address={CODING_ATTEMPT_ADDRESS}
        onBack={vi.fn()}
      />
    );

    expect(useCodingWorkspaceWs).toHaveBeenCalledWith(CODING_ATTEMPT_ADDRESS);
    expect(screen.getByText("Coding Attempt #coding_attempt_0001")).toBeInTheDocument();
    expect(screen.getByTestId("coding-timeline")).toHaveTextContent("执行测试");
    expect(screen.getByTestId("chat-entry-list")).toHaveTextContent("cargo test");
    expect(screen.queryByTestId("coding-artifact-tabs")).not.toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: "运行结果" }));

    expect(screen.getByTestId("coding-artifact-tabs")).toHaveTextContent("passed");
    expect(screen.getByTestId("coding-status-bar")).toHaveTextContent("Tester");
    expect(screen.getByTestId("coding-status-bar")).toHaveTextContent("Coder 修复次数 0/2");
  });

  it("renders tester assistant chat entries as bubbles", () => {
    mockCodingWs();
    useCodingWorkspaceStore.setState({
      attemptId: "coding_attempt_0001",
      status: "running",
      stage: "testing",
      timelineNodes: [
        {
          id: "coding_node_0003",
          attempt_id: "coding_attempt_0001",
          stage: "testing",
          title: "执行测试",
          status: "running",
          agent_role: "tester",
          summary: null,
          started_at: "2026-06-10T00:00:00Z",
          completed_at: null,
          artifact_refs: [],
        },
      ],
      chatEntries: [
        {
          id: "tester_entry_0001",
          type: "provider_stream",
          role: "tester",
          content: "TestPlan: unit checks",
          timestamp: "2026-06-10T00:00:01Z",
          node_id: "coding_node_0003",
          metadata: {
            phase: "plan_tests",
            test_plan_id: "test_plan_0001",
          },
        },
      ],
    });

    render(
      <CodingWorkspacePage
        address={CODING_ATTEMPT_ADDRESS}
        onBack={vi.fn()}
      />
    );

    const chatList = screen.getByTestId("chat-entry-list");
    expect(chatList).toHaveTextContent("Tester");
    expect(chatList).toHaveTextContent("TestPlan: unit checks");
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
    expect(screen.getByText("work_item_0001")).toBeInTheDocument();
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
      stage: "testing",
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
          stage: "testing",
          title: "测试执行",
          status: "running",
          agent_role: "tester",
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
          id: "entry-testing",
          type: "provider_stream",
          role: "tester",
          content: "测试中",
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
    await userEvent.click(screen.getByRole("button", { name: /测试执行/ }));

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

  it("shows dependency handoff summary in execution plan", () => {
    mockCodingWs();
    useCodingWorkspaceStore.setState({
      ...readyCodingState(),
      stage: "prepare_context",
      workItemExecutionPlan: executionPlan({
        dependency_handoffs: [
          {
            work_item_id: "work_item_0001",
            summary_ref: "handoffs/work_item_0001.json",
            summary: "后端 API 已完成",
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

    expect(screen.getByText("后端 API 已完成")).toBeInTheDocument();
    expect(screen.getByText("abc123")).toBeInTheDocument();
  });

  it("deletes the coding workspace after confirmation and navigates back", async () => {
    mockCodingWs();
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(true);
    vi.mocked(deleteCodingAttempt).mockResolvedValue(undefined);
    const onBack = vi.fn();
    useCodingWorkspaceStore.setState({
      projectId: CODING_ATTEMPT_ADDRESS.projectId,
      issueId: CODING_ATTEMPT_ADDRESS.issueId,
      attemptId: "coding_attempt_0001",
      status: "running",
      stage: "coding",
    });

    render(
      <CodingWorkspacePage address={CODING_ATTEMPT_ADDRESS} onBack={onBack} />
    );

    await userEvent.click(
      screen.getByRole("button", { name: "删除 Coding Workspace" }),
    );

    expect(confirm).toHaveBeenCalledWith(
      expect.stringContaining("日志、测试输出和 worktree"),
    );
    await waitFor(() =>
      expect(deleteCodingAttempt).toHaveBeenCalledWith(CODING_ATTEMPT_ADDRESS),
    );
    expect(onBack).toHaveBeenCalled();
    confirm.mockRestore();
  });

  it("disables deletion while the store belongs to a previous address", async () => {
    mockCodingWs();
    vi.mocked(deleteCodingAttempt).mockResolvedValue(undefined);
    useCodingWorkspaceStore.setState({
      projectId: CODING_ATTEMPT_ADDRESS.projectId,
      issueId: CODING_ATTEMPT_ADDRESS.issueId,
      attemptId: CODING_ATTEMPT_ADDRESS.attemptId,
      status: "running",
      stage: "coding",
    });

    render(
      <CodingWorkspacePage
        address={OTHER_CODING_ATTEMPT_ADDRESS}
        onBack={vi.fn()}
      />,
    );

    const deleteButton = screen.getByRole("button", {
      name: "删除 Coding Workspace",
    });
    expect(deleteButton).toBeDisabled();
    await userEvent.click(deleteButton);
    expect(deleteCodingAttempt).not.toHaveBeenCalled();
  });

  it("sends final confirm and abort actions", async () => {
    const api = mockCodingWs();
    useCodingWorkspaceStore.setState({
      attemptId: "coding_attempt_0001",
      status: "waiting_for_human",
      stage: "final_confirm",
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
});
