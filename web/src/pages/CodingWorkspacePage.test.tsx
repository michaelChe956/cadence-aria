import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  confirmWorkItemExecutionPlan,
  deleteCodingAttempt,
  getCodingAttemptDiff,
  requestWorkItemExecutionPlanChange,
} from "../api/client";
import { useCodingWorkspaceWs } from "../hooks/useCodingWorkspaceWs";
import { useCodingWorkspaceStore } from "../state/coding-workspace-store";
import type { WorkItemExecutionPlan } from "../api/types";
import { CodingWorkspacePage } from "./CodingWorkspacePage";

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

type CodingWsApi = ReturnType<typeof useCodingWorkspaceWs>;

const DEFAULT_PERMISSION_MODES = {
  coder: "supervised",
  tester: "auto",
  analyst: "auto",
  code_reviewer: "supervised",
  internal_reviewer: "supervised",
} as const;

function mockCodingWs(overrides: Partial<CodingWsApi> = {}) {
  const api: CodingWsApi = {
    startCoding: vi.fn(),
    sendContextNote: vi.fn(),
    sendProviderSelect: vi.fn(),
    sendPermissionModeSelect: vi.fn(),
    confirmStageGate: vi.fn(),
    respondPermission: vi.fn(),
    respondChoice: vi.fn(),
    respondGate: vi.fn(),
    continueRework: vi.fn(),
    finalConfirm: vi.fn(),
    abortAttempt: vi.fn(),
    requestManualPause: vi.fn(),
    sendHello: vi.fn(),
    sendPing: vi.fn(),
    ...overrides,
  };
  vi.mocked(useCodingWorkspaceWs).mockReturnValue(api);
  return api;
}

function readyCodingState() {
  return {
    attemptId: "coding_attempt_0001",
    status: "created" as const,
    stage: "prepare_context" as const,
    branchName: "aria/work-items/work_item_0001/attempt-1",
    baseBranch: "main",
    worktreePath: "/tmp/worktree",
    timelineNodes: [],
    chatEntries: [],
  };
}

function executionPlan(overrides: Partial<WorkItemExecutionPlan> = {}): WorkItemExecutionPlan {
  return {
    id: "work_item_execution_plan_0001",
    project_id: "project_0001",
    issue_id: "issue_0001",
    work_item_id: "work_item_0001",
    attempt_id: "coding_attempt_0001",
    status: "draft",
    goal: "实现后端 API",
    allowed_write_scopes: ["src/product/**"],
    forbidden_write_scopes: ["web/**"],
    dependency_handoffs: [],
    story_refs: ["story_spec_0001"],
    design_refs: ["design_spec_0001"],
    openspec_refs: ["REQ-001"],
    superpowers_contract: "use superpowers:test-driven-development",
    tdd_contract: "先写失败测试，再写实现",
    verification_plan_ref: "verification_plan_work_item_0001",
    verification_summary: "provider supplied required gate verify_backend_unit",
    risk_notes: [],
    created_at: "2026-06-16T00:00:00Z",
    updated_at: "2026-06-16T00:00:00Z",
    ...overrides,
  };
}

describe("CodingWorkspacePage", () => {
  beforeEach(() => {
    Object.defineProperty(HTMLElement.prototype, "scrollIntoView", {
      configurable: true,
      value: vi.fn(),
    });
    useCodingWorkspaceStore.getState().reset();
    vi.clearAllMocks();
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

    render(<CodingWorkspacePage attemptId="coding_attempt_0001" onBack={vi.fn()} />);

    expect(screen.getByText("Coding Attempt #coding_attempt_0001")).toBeInTheDocument();
    expect(screen.getByTestId("coding-timeline")).toHaveTextContent("执行测试");
    expect(screen.getByTestId("chat-entry-list")).toHaveTextContent("cargo test");
    expect(screen.queryByTestId("coding-artifact-tabs")).not.toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: "运行结果" }));

    expect(screen.getByTestId("coding-artifact-tabs")).toHaveTextContent("passed");
    expect(screen.getByTestId("coding-status-bar")).toHaveTextContent("testing");
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

    render(<CodingWorkspacePage attemptId="coding_attempt_0001" onBack={vi.fn()} />);

    const chatList = screen.getByTestId("chat-entry-list");
    expect(chatList).toHaveTextContent("Tester");
    expect(chatList).toHaveTextContent("TestPlan: unit checks");
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

    render(<CodingWorkspacePage attemptId="coding_attempt_0001" onBack={vi.fn()} />);

    await userEvent.click(screen.getByRole("button", { name: "运行结果" }));

    await waitFor(() => {
      expect(getCodingAttemptDiff).toHaveBeenCalledWith("coding_attempt_0001");
    });
    const viewer = await screen.findByTestId("monaco-diff-viewer");
    expect(viewer).toHaveAttribute("data-language", "python");
    expect(screen.getByText("climbing_stairs.py")).toBeInTheDocument();
    expect(screen.getByTestId("monaco-diff-original").textContent).toBe("");
    expect(screen.getByTestId("monaco-diff-modified").textContent).toBe(
      "def climb_stairs(n):\n    return n",
    );
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

    render(<CodingWorkspacePage attemptId="coding_attempt_0001" onBack={vi.fn()} />);
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

    render(<CodingWorkspacePage attemptId="coding_attempt_0001" onBack={vi.fn()} />);

    await userEvent.click(screen.getByRole("button", { name: "开始 Coding" }));

    expect(api.startCoding).toHaveBeenCalled();
  });

  it("deletes the coding workspace after confirmation and navigates back", async () => {
    mockCodingWs();
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(true);
    vi.mocked(deleteCodingAttempt).mockResolvedValue({ status: "deleted" });
    const onBack = vi.fn();
    useCodingWorkspaceStore.setState({
      attemptId: "coding_attempt_0001",
      status: "running",
      stage: "coding",
    });

    render(<CodingWorkspacePage attemptId="coding_attempt_0001" onBack={onBack} />);

    await userEvent.click(
      screen.getByRole("button", { name: "删除 Coding Workspace" }),
    );

    expect(confirm).toHaveBeenCalledWith(
      expect.stringContaining("日志、测试输出和 worktree"),
    );
    await waitFor(() =>
      expect(deleteCodingAttempt).toHaveBeenCalledWith("coding_attempt_0001"),
    );
    expect(onBack).toHaveBeenCalled();
    confirm.mockRestore();
  });

  it("sends final confirm and abort actions", async () => {
    const api = mockCodingWs();
    useCodingWorkspaceStore.setState({
      attemptId: "coding_attempt_0001",
      status: "waiting_for_human",
      stage: "final_confirm",
    });

    render(<CodingWorkspacePage attemptId="coding_attempt_0001" onBack={vi.fn()} />);

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
      stage: "rework",
      pendingGates: [
        {
          gate_id: "gate_0001",
          kind: "blocked",
          title: "需要人工处理",
          description: "自动返工次数已达上限",
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

    render(<CodingWorkspacePage attemptId="coding_attempt_0001" onBack={vi.fn()} />);

    expect(screen.getByTestId("coding-pending-gate")).toHaveTextContent("需要人工处理");

    await userEvent.click(screen.getByRole("button", { name: "中止 Attempt" }));

    expect(api.respondGate).toHaveBeenCalledWith("gate_0001", "abort", undefined);
  });

  it("renders tester contract blocked gate as blocked instead of failed test", async () => {
    mockCodingWs();
    useCodingWorkspaceStore.setState({
      attemptId: "coding_attempt_0001",
      status: "blocked",
      stage: "testing",
      pendingGates: [
        {
          gate_id: "gate_0001",
          kind: "blocked",
          title: "Testing blocked",
          description: "TestPlan parse failed",
          stage: "testing",
          role: "tester",
          reason_code: "test_plan_missing_json",
          evidence_refs: ["testing_report_0001.json"],
          raw_provider_output_ref: "provider-raw/testing/plan_tests_0001.txt",
          available_actions: [
            {
              action_id: "retry_test_plan",
              label: "重试测试计划",
              action_type: "retry_test_plan",
            },
          ],
        },
      ],
    });

    render(<CodingWorkspacePage attemptId="coding_attempt_0001" onBack={vi.fn()} />);

    const gate = screen.getByTestId("coding-pending-gate");
    expect(gate).toHaveTextContent("Tester 未返回测试计划 JSON");
    expect(gate).toHaveTextContent("测试被阻塞");
    expect(gate).not.toHaveTextContent("测试失败");
  });

  it("renders testing result review gate as human confirmation instead of blocked", async () => {
    const api = mockCodingWs();
    useCodingWorkspaceStore.setState({
      attemptId: "coding_attempt_0001",
      status: "blocked",
      stage: "testing",
      pendingGates: [
        {
          gate_id: "gate_0001",
          kind: "blocked",
          title: "确认 Tester 测试结果",
          description:
            "Tester 已完成测试报告 testing_report_0001（测试通过）。请确认是否进入 Analyst 或重新测试。",
          stage: "testing",
          role: "tester",
          reason_code: "testing_result_review_required",
          evidence_refs: ["testing_report_0001.json"],
          raw_provider_output_ref: "provider-raw/testing/execute_test_plan_0001.txt",
          available_actions: [
            {
              action_id: "accept_testing_result",
              label: "结果可用，进入 Analyst",
              action_type: "accept_testing_result",
            },
            {
              action_id: "rerun_testing",
              label: "不满意，重新测试",
              action_type: "rerun_testing",
            },
          ],
        },
      ],
    });

    render(<CodingWorkspacePage attemptId="coding_attempt_0001" onBack={vi.fn()} />);

    const gate = screen.getByTestId("coding-pending-gate");
    expect(gate).toHaveTextContent("确认 Tester 测试结果");
    expect(gate).toHaveTextContent("等待确认 Tester 结果");
    expect(gate).not.toHaveTextContent("测试被阻塞");

    await userEvent.click(screen.getByRole("button", { name: "结果可用，进入 Analyst" }));
    expect(api.respondGate).toHaveBeenCalledWith(
      "gate_0001",
      "accept_testing_result",
      undefined,
    );

    await userEvent.click(screen.getByRole("button", { name: "不满意，重新测试" }));
    expect(api.respondGate).toHaveBeenCalledWith("gate_0001", "rerun_testing", undefined);
  });

  it("renders skipped_required_steps blocked gate with dedicated label", async () => {
    mockCodingWs();
    useCodingWorkspaceStore.setState({
      attemptId: "coding_attempt_0001",
      status: "blocked",
      stage: "testing",
      pendingGates: [
        {
          gate_id: "gate_0001",
          kind: "blocked",
          title: "Testing blocked",
          description: "Required testing steps are missing or blocked",
          stage: "testing",
          role: "tester",
          reason_code: "skipped_required_steps",
          evidence_refs: ["testing_report_0001.json"],
          raw_provider_output_ref: "provider-raw/testing/execute_test_plan_0001.txt",
          available_actions: [
            {
              action_id: "retry_test_plan",
              label: "重试测试计划",
              action_type: "retry_test_plan",
            },
          ],
        },
      ],
    });

    render(<CodingWorkspacePage attemptId="coding_attempt_0001" onBack={vi.fn()} />);

    const gate = screen.getByTestId("coding-pending-gate");
    expect(gate).toHaveTextContent("required 测试步骤被阻塞（无法执行）");
    expect(gate).not.toHaveTextContent("缺少 required 测试步骤证据");
  });

  it("sends stage gate confirm for confirm-stage pending gate actions", async () => {
    const api = mockCodingWs();
    useCodingWorkspaceStore.setState({
      attemptId: "coding_attempt_0001",
      status: "running",
      stage: "testing",
      pendingGates: [
        {
          gate_id: "coding_stage_gate_0001",
          kind: "stage_gate",
          title: "Testing Stage Gate",
          description: "Waiting to start Testing",
          stage: "testing",
          role: "tester",
          expires_at: "2026-05-28T00:00:05Z",
          provider_snapshot: {
            coder: "fake",
            tester: "fake",
            analyst: "fake",
            code_reviewer: "fake",
            internal_reviewer: "fake",
            review_rounds: 1,
            permission_modes: DEFAULT_PERMISSION_MODES,
          },
          available_actions: [
            {
              action_id: "confirm_stage",
              label: "立即开始",
              action_type: "confirm_stage",
            },
          ],
        },
      ],
    });

    render(<CodingWorkspacePage attemptId="coding_attempt_0001" onBack={vi.fn()} />);

    await userEvent.click(screen.getByRole("button", { name: "Stage Gate 立即开始" }));

    expect(api.confirmStageGate).toHaveBeenCalledWith("testing");
    expect(api.respondGate).not.toHaveBeenCalled();
  });

  it("renders stage gate countdown with provider and abort action", async () => {
    const api = mockCodingWs();
    useCodingWorkspaceStore.setState({
      attemptId: "coding_attempt_0001",
      status: "running",
      stage: "coding",
      pendingGates: [
        {
          gate_id: "coding_stage_gate_0001",
          kind: "stage_gate",
          title: "Coding Stage Gate",
          description: "Waiting to start Coding",
          stage: "coding",
          role: "coder",
          expires_at: new Date(Date.now() + 5_000).toISOString(),
          provider_snapshot: {
            coder: "fake",
            tester: "codex",
            analyst: "fake",
            code_reviewer: "fake",
            internal_reviewer: "fake",
            review_rounds: 1,
            permission_modes: DEFAULT_PERMISSION_MODES,
          },
          available_actions: [
            {
              action_id: "confirm_stage",
              label: "立即开始",
              action_type: "confirm_stage",
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

    render(<CodingWorkspacePage attemptId="coding_attempt_0001" onBack={vi.fn()} />);

    expect(screen.getByTestId("coding-stage-gate-entry")).toHaveTextContent("Coding Stage Gate");
    expect(screen.getByTestId("coding-stage-gate-entry")).toHaveTextContent("Coder");
    expect(screen.getByTestId("coding-stage-gate-entry")).toHaveTextContent("fake");
    expect(screen.getByTestId("coding-stage-gate-entry")).toHaveTextContent("5s");

    await userEvent.click(screen.getByRole("button", { name: "Stage Gate 立即开始" }));
    await userEvent.click(screen.getByRole("button", { name: "Stage Gate 中止" }));

    expect(api.confirmStageGate).toHaveBeenCalledWith("coding");
    expect(api.abortAttempt).toHaveBeenCalled();
  });

  it("renders role provider panel and sends role-level provider selection", async () => {
    const api = mockCodingWs();
    useCodingWorkspaceStore.setState({
      attemptId: "coding_attempt_0001",
      status: "running",
      stage: "coding",
      roleProviderConfigSnapshot: {
        coder: "fake",
        tester: "fake",
        analyst: "fake",
        code_reviewer: "fake",
        internal_reviewer: "fake",
        review_rounds: 1,
        permission_modes: {
          coder: "supervised",
          tester: "auto",
          analyst: "auto",
          code_reviewer: "supervised",
          internal_reviewer: "supervised",
        },
      },
    });

    render(<CodingWorkspacePage attemptId="coding_attempt_0001" onBack={vi.fn()} />);

    expect(screen.getByTestId("coding-provider-config-panel")).toHaveTextContent("Coder");
    expect(screen.getByTestId("coding-provider-config-panel")).toHaveTextContent("Tester");
    expect(screen.getByTestId("coding-provider-config-panel")).toHaveTextContent("Auto");

    await userEvent.click(screen.getByRole("button", { name: "将 Tester 切换为 Codex" }));
    await userEvent.click(
      screen.getByRole("button", { name: "将 Tester 授权模式切换为 Supervised" }),
    );

    expect(api.sendProviderSelect).toHaveBeenCalledWith("tester", "codex");
    expect(api.sendPermissionModeSelect).toHaveBeenCalledWith("tester", "supervised");
  });

  it("sends coding context notes from the chat input", async () => {
    const api = mockCodingWs();
    useCodingWorkspaceStore.setState({
      attemptId: "coding_attempt_0001",
      status: "running",
      stage: "coding",
    });

    render(<CodingWorkspacePage attemptId="coding_attempt_0001" onBack={vi.fn()} />);

    const input = screen.getByLabelText("补充 Coding 上下文");
    await userEvent.type(input, "请覆盖空输入边界");
    await userEvent.click(screen.getByRole("button", { name: "发送上下文" }));

    expect(api.sendContextNote).toHaveBeenCalledWith("请覆盖空输入边界");
    expect(input).toHaveValue("");
  });

  it("keeps a manually selected artifact tab while the attempt is testing", async () => {
    mockCodingWs();
    useCodingWorkspaceStore.setState({
      attemptId: "coding_attempt_0001",
      status: "running",
      stage: "testing",
      activeTab: "tests",
      testingReport: {
        id: "testing_report_0001",
        attempt_id: "coding_attempt_0001",
        overall_status: "passed",
        provider_claim: null,
        backend_verified: true,
        started_at: "2026-05-23T00:00:00Z",
        completed_at: "2026-05-23T00:00:02Z",
        commands: [],
      },
      logs: [
        {
          id: "log_0001",
          message: "manual tab stays visible",
          timestamp: "2026-05-23T00:00:03Z",
        },
      ],
    });

    render(<CodingWorkspacePage attemptId="coding_attempt_0001" onBack={vi.fn()} />);

    await userEvent.click(screen.getByRole("button", { name: "运行结果" }));
    await userEvent.click(screen.getByRole("button", { name: "logs" }));

    expect(screen.getByTestId("coding-artifact-tabs")).toHaveTextContent(
      "manual tab stays visible",
    );
    expect(screen.getByTestId("coding-artifact-tabs")).not.toHaveTextContent("passed");
  });

  it("renders plan based testing report details", async () => {
    mockCodingWs();
    useCodingWorkspaceStore.setState({
      attemptId: "coding_attempt_0001",
      status: "blocked",
      stage: "testing",
      activeTab: "tests",
      testingReport: {
        id: "testing_report_0001",
        attempt_id: "coding_attempt_0001",
        commands: [],
        overall_status: "blocked",
        provider_claim: null,
        backend_verified: true,
        started_at: "2026-06-10T00:00:00Z",
        completed_at: "2026-06-10T00:00:01Z",
        plan_id: "test_plan_0001",
        plan_summary: "API smoke and security review",
        steps: [
          {
            step_id: "api_smoke",
            status: "passed",
            evidence_refs: ["stdout.log"],
            command: ["cargo", "test", "--locked", "--lib", "api_smoke"],
            provider_analysis: "API smoke passed",
          },
        ],
        missing_required_steps: ["security"],
        context_warnings: ["missing_design_spec"],
        raw_provider_output_ref: "provider-raw/testing/execute_test_plan_0001.txt",
      },
    });

    render(<CodingWorkspacePage attemptId="coding_attempt_0001" onBack={vi.fn()} />);

    await userEvent.click(screen.getByRole("button", { name: "运行结果" }));

    const tabs = screen.getByTestId("coding-artifact-tabs");
    expect(tabs).toHaveTextContent("API smoke and security review");
    expect(tabs).toHaveTextContent("api_smoke");
    expect(tabs).toHaveTextContent("missing required: security");
    expect(tabs).toHaveTextContent("missing_design_spec");
    expect(tabs).toHaveTextContent("provider-raw/testing/execute_test_plan_0001.txt");
  });

  it("renders analyst decision state beside testing report", async () => {
    mockCodingWs();
    useCodingWorkspaceStore.setState({
      attemptId: "coding_attempt_0001",
      status: "running",
      stage: "rework",
      activeTab: "tests",
      timelineNodes: [
        {
          id: "coding_node_analyst_0001",
          attempt_id: "coding_attempt_0001",
          stage: "rework",
          title: "Analyst 路由决策",
          status: "running",
          agent_role: "system",
          summary: null,
          started_at: "2026-06-12T00:00:01Z",
          completed_at: null,
          artifact_refs: [],
        },
      ],
      testingReport: {
        id: "testing_report_0001",
        attempt_id: "coding_attempt_0001",
        commands: [],
        overall_status: "blocked",
        provider_claim: null,
        backend_verified: true,
        started_at: "2026-06-12T00:00:00Z",
        completed_at: "2026-06-12T00:00:01Z",
        skipped_required_steps: ["browser_e2e"],
        raw_provider_output_ref: "provider-raw/testing/execute_test_plan_0001.txt",
      },
      latestAnalystDecision: null,
    });

    const { rerender } = render(
      <CodingWorkspacePage attemptId="coding_attempt_0001" onBack={vi.fn()} />,
    );

    await userEvent.click(screen.getByRole("button", { name: "运行结果" }));

    expect(screen.getByTestId("coding-artifact-tabs")).toHaveTextContent(
      "等待 Analyst 决策",
    );

    useCodingWorkspaceStore.setState({
      latestAnalystDecision: {
        id: "analyst_decision_0001",
        attempt_id: "coding_attempt_0001",
        source_stage: "testing",
        rework_round: 1,
        verdict: "needs_fix",
        next_stage: "coding",
        reason: "required 测试步骤被跳过，需要回到 Coder",
        evidence_refs: ["testing_report_0001.json"],
        raw_provider_output_refs: ["provider-raw/testing/execute_test_plan_0001.txt"],
        rework_instructions: null,
        human_gate: null,
        created_at: "2026-06-12T00:00:02Z",
        parse_error: null,
      },
    });
    rerender(<CodingWorkspacePage attemptId="coding_attempt_0001" onBack={vi.fn()} />);

    const tabs = screen.getByTestId("coding-artifact-tabs");
    expect(tabs).toHaveTextContent("Analyst 已决策");
    expect(tabs).toHaveTextContent("needs_fix -> coding");
    expect(tabs).toHaveTextContent("required 测试步骤被跳过，需要回到 Coder");
    expect(tabs).toHaveTextContent("testing_report_0001.json");
    expect(screen.getByTestId("coding-timeline")).toHaveTextContent("needs_fix -> coding");
  });

  it("renders legacy testing report without plan fields", async () => {
    mockCodingWs();
    useCodingWorkspaceStore.setState({
      attemptId: "coding_attempt_0001",
      status: "running",
      stage: "testing",
      activeTab: "tests",
      testingReport: {
        id: "testing_report_0001",
        attempt_id: "coding_attempt_0001",
        commands: [],
        overall_status: "passed",
        provider_claim: null,
        backend_verified: true,
        started_at: "2026-06-10T00:00:00Z",
        completed_at: "2026-06-10T00:00:01Z",
      },
    });

    render(<CodingWorkspacePage attemptId="coding_attempt_0001" onBack={vi.fn()} />);

    await userEvent.click(screen.getByRole("button", { name: "运行结果" }));

    const tabs = screen.getByTestId("coding-artifact-tabs");
    expect(tabs).toHaveTextContent("passed");
    expect(tabs).not.toHaveTextContent("Test Plan");
  });

  it("renders blocked gate metadata and sends recovery action", async () => {
    const api = mockCodingWs();
    useCodingWorkspaceStore.setState({
      attemptId: "coding_attempt_0001",
      status: "blocked",
      stage: "code_review",
      pendingGates: [
        {
          gate_id: "gate_0001",
          kind: "blocked",
          title: "审查输出需要处理",
          description: "Review payload parse failed",
          stage: "code_review",
          role: "code_reviewer",
          reason_code: "review_payload_parse_error",
          evidence_refs: ["code_review_0001.json"],
          raw_provider_output_ref: "provider-raw/code_review/code_review_0001.txt",
          available_actions: [
            {
              action_id: "retry_review",
              label: "重试审查",
              action_type: "retry_review",
            },
            {
              action_id: "manual_continue",
              label: "人工继续",
              action_type: "manual_continue",
            },
          ],
        },
      ],
    });

    render(<CodingWorkspacePage attemptId="coding_attempt_0001" onBack={vi.fn()} />);

    const gate = screen.getByTestId("coding-pending-gate");
    expect(gate).toHaveTextContent("review_payload_parse_error");
    expect(gate).toHaveTextContent("code_review_0001.json");
    expect(gate).toHaveTextContent("provider-raw/code_review/code_review_0001.txt");

    await userEvent.click(screen.getByRole("button", { name: "重试审查" }));

    expect(api.respondGate).toHaveBeenCalledWith("gate_0001", "retry_review", undefined);

    vi.mocked(api.respondGate).mockClear();
    await userEvent.click(screen.getByRole("button", { name: "人工继续" }));

    expect(api.respondGate).not.toHaveBeenCalled();

    await userEvent.type(
      screen.getByPlaceholderText("说明跳过该门禁的原因和后续风险处理"),
      "人工确认风险可接受，后续补充真实 E2E",
    );
    await userEvent.click(screen.getByRole("button", { name: "人工继续" }));

    expect(api.respondGate).toHaveBeenCalledWith(
      "gate_0001",
      "manual_continue",
      "人工确认风险可接受，后续补充真实 E2E",
    );
  });

  it("renders analyst human gate manual continue as quality bypass risk", async () => {
    mockCodingWs();
    useCodingWorkspaceStore.setState({
      attemptId: "coding_attempt_0001",
      status: "blocked",
      stage: "rework",
      pendingGates: [
        {
          gate_id: "gate_0001",
          kind: "blocked",
          title: "Rework limit reached",
          description: "已达到自动重写上限",
          stage: "rework",
          role: "analyst",
          reason_code: "max_auto_rework_exceeded",
          evidence_refs: ["testing_report_0001.json"],
          available_actions: [
            {
              action_id: "manual_continue",
              label: "人工继续",
              action_type: "manual_continue",
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

    render(<CodingWorkspacePage attemptId="coding_attempt_0001" onBack={vi.fn()} />);

    const gate = screen.getByTestId("coding-pending-gate");
    expect(gate).toHaveTextContent("Analyst 建议人工决策");
    expect(gate).toHaveTextContent("人工放行会记录质量豁免");
    expect(gate).toHaveTextContent("max_auto_rework_exceeded");
  });

  it("renders continue rework action for waiting rework attempts", async () => {
    const api = mockCodingWs();
    useCodingWorkspaceStore.setState({
      attemptId: "coding_attempt_0001",
      status: "waiting_for_human",
      stage: "rework",
    });

    render(<CodingWorkspacePage attemptId="coding_attempt_0001" onBack={vi.fn()} />);

    await userEvent.click(screen.getByRole("button", { name: "继续返修" }));

    expect(api.continueRework).toHaveBeenCalledWith(null);
    expect(api.abortAttempt).not.toHaveBeenCalled();
  });

  it("renders review findings with severity, location, and required action", async () => {
    mockCodingWs();
    useCodingWorkspaceStore.setState({
      attemptId: "coding_attempt_0001",
      status: "running",
      stage: "code_review",
      activeTab: "review",
      codeReviewReports: [
        {
          id: "code_review_0001",
          attempt_id: "coding_attempt_0001",
          round: 1,
          verdict: "request_changes",
          summary: "需要修复边界条件",
          tested_evidence_refs: [],
          diff_refs: [],
          created_at: "2026-05-23T00:00:00Z",
          findings: [
            {
              severity: "error",
              file_path: "src/solver.py",
              line: 42,
              message: "缺少 n=0 的处理",
              required_action: "补充空输入测试",
              source_stage: "code_review",
            },
          ],
        },
      ],
    });

    render(<CodingWorkspacePage attemptId="coding_attempt_0001" onBack={vi.fn()} />);

    await userEvent.click(screen.getByRole("button", { name: "运行结果" }));
    const tabs = screen.getByTestId("coding-artifact-tabs");
    expect(tabs).toHaveTextContent("error");
    expect(tabs).toHaveTextContent("src/solver.py:42");
    expect(tabs).toHaveTextContent("缺少 n=0 的处理");
    expect(tabs).toHaveTextContent("补充空输入测试");
    expect(screen.getByText("error").className).toContain("text-red");
  });

  it("renders internal PR review impact scope and PR text suggestions", async () => {
    mockCodingWs();
    useCodingWorkspaceStore.setState({
      attemptId: "coding_attempt_0001",
      status: "running",
      stage: "internal_pr_review",
      activeTab: "review",
      internalPrReview: {
        id: "internal_review_0001",
        attempt_id: "coding_attempt_0001",
        review_request_id: "review_request_0001",
        verdict: "approve",
        summary: "内部审查通过",
        findings: [],
        impact_scope: ["src/solver.py", "tests/test_solver.py"],
        pr_description: "实现 climb_stairs 动态规划函数，并覆盖 n=10。",
        commit_message_suggestion: "feat: implement climb stairs",
        tested_evidence_refs: [],
        diff_refs: [],
        created_at: "2026-05-23T00:00:00Z",
      },
    });

    render(<CodingWorkspacePage attemptId="coding_attempt_0001" onBack={vi.fn()} />);

    await userEvent.click(screen.getByRole("button", { name: "运行结果" }));
    const tabs = screen.getByTestId("coding-artifact-tabs");
    expect(tabs).toHaveTextContent("src/solver.py");
    expect(tabs).toHaveTextContent("tests/test_solver.py");
    expect(tabs).toHaveTextContent("实现 climb_stairs 动态规划函数");
    expect(tabs).toHaveTextContent("feat: implement climb stairs");
  });

  it("renders review request URL, push status, and manual instructions in the git tab", async () => {
    mockCodingWs();
    useCodingWorkspaceStore.setState({
      attemptId: "coding_attempt_0001",
      status: "waiting_for_human",
      stage: "final_confirm",
      activeTab: "git",
      baseBranch: "main",
      branchName: "aria/work-items/work_item_0001/attempt-1",
      headCommit: "abc1234",
      pushedRemote: "origin",
      reviewRequest: {
        id: "review_request_0001",
        attempt_id: "coding_attempt_0001",
        kind: "git_branch_only",
        remote_kind: "generic_git",
        remote: "origin",
        base_branch: "main",
        branch_name: "aria/work-items/work_item_0001/attempt-1",
        commit_sha: "abc1234",
        push_status: "pushed",
        external_url: "https://git.example/review/1",
        manual_instructions: ["打开平台创建 PR", "选择 attempt 分支"],
        created_at: "2026-05-23T00:00:00Z",
        updated_at: "2026-05-23T00:00:01Z",
      },
    });

    render(<CodingWorkspacePage attemptId="coding_attempt_0001" onBack={vi.fn()} />);

    await userEvent.click(screen.getByRole("button", { name: "运行结果" }));
    const tabs = screen.getByTestId("coding-artifact-tabs");
    expect(tabs).toHaveTextContent("pushed");
    expect(screen.getByRole("link", { name: "https://git.example/review/1" })).toHaveAttribute(
      "href",
      "https://git.example/review/1",
    );
    expect(tabs).toHaveTextContent("打开平台创建 PR");
    expect(tabs).toHaveTextContent("选择 attempt 分支");
  });

  it("renders analyst chat with role run metadata present", () => {
    mockCodingWs();
    useCodingWorkspaceStore.setState({
      attemptId: "coding_attempt_0001",
      status: "blocked",
      stage: "rework",
      chatEntries: [
        {
          id: "coding_node_0004_analyst_verdict",
          type: "analyst_verdict",
          role: "analyst",
          content: "Analyst 输出不是有效 JSON，已转人工确认。",
          timestamp: "2026-06-13T00:00:01Z",
          node_id: "coding_node_0004",
          metadata: {
            role_run_id: "coding_role_run_0001",
            run_no: 1,
            reason: "Analyst 输出不是有效 JSON，已转人工确认。",
          },
        },
      ],
    });

    render(<CodingWorkspacePage attemptId="coding_attempt_0001" onBack={vi.fn()} />);

    const chatList = screen.getByTestId("chat-entry-list");
    expect(chatList).toHaveTextContent("Analyst");
    expect(chatList).toHaveTextContent("Analyst 输出不是有效 JSON");
  });

  it("renders role run history and selects linked timeline nodes", async () => {
    mockCodingWs();
    useCodingWorkspaceStore.setState({
      attemptId: "coding_attempt_0001",
      status: "blocked",
      stage: "rework",
      timelineNodes: [
        {
          id: "coding_node_0003",
          attempt_id: "coding_attempt_0001",
          stage: "testing",
          title: "执行测试",
          status: "completed",
          agent_role: "tester",
          summary: "测试阻塞",
          started_at: "2026-06-13T00:00:00Z",
          completed_at: "2026-06-13T00:00:01Z",
          artifact_refs: [],
        },
        {
          id: "coding_node_0004",
          attempt_id: "coding_attempt_0001",
          stage: "rework",
          title: "Analyst 路由决策",
          status: "blocked",
          agent_role: "system",
          summary: "需要人工处理",
          started_at: "2026-06-13T00:00:02Z",
          completed_at: null,
          artifact_refs: [],
        },
      ],
      roleRuns: [
        {
          id: "coding_role_run_0001",
          attempt_id: "coding_attempt_0001",
          stage: "testing",
          role: "tester",
          run_no: 1,
          status: "completed",
          trigger: "initial",
          node_id: "coding_node_0003",
          started_at: "2026-06-13T00:00:00Z",
          completed_at: "2026-06-13T00:00:01Z",
          reason_code: null,
          raw_provider_output_refs: ["provider-raw/testing/plan_tests_0001.txt"],
          artifact_refs: [],
        },
        {
          id: "coding_role_run_0002",
          attempt_id: "coding_attempt_0001",
          stage: "rework",
          role: "analyst",
          run_no: 1,
          status: "blocked",
          trigger: "retry_analyst",
          node_id: "coding_node_0004",
          started_at: "2026-06-13T00:00:02Z",
          completed_at: null,
          reason_code: "analyst_human_gate",
          raw_provider_output_refs: [],
          artifact_refs: ["provider-raw/rework/analyst_evidence_0001.txt"],
        },
      ],
    });

    render(<CodingWorkspacePage attemptId="coding_attempt_0001" onBack={vi.fn()} />);

    const panel = screen.getByTestId("coding-role-run-history");
    expect(panel).toHaveTextContent("Tester #1");
    expect(panel).toHaveTextContent("provider-raw/testing/plan_tests_0001.txt");
    expect(panel).toHaveTextContent("Analyst #1");
    expect(panel).toHaveTextContent("analyst_human_gate");

    await userEvent.click(screen.getByRole("button", { name: /Analyst #1/ }));

    expect(useCodingWorkspaceStore.getState().selectedNodeId).toBe("coding_node_0004");
  });

  it("constrains role run history overflow inside the conversation column", () => {
    mockCodingWs();
    useCodingWorkspaceStore.setState({
      attemptId: "coding_attempt_0001",
      status: "waiting_for_human",
      stage: "rework",
      roleRuns: Array.from({ length: 12 }, (_, index) => ({
        id: `coding_role_run_${String(index + 1).padStart(4, "0")}`,
        attempt_id: "coding_attempt_0001",
        stage: index % 2 === 0 ? "testing" : "rework",
        role: index % 2 === 0 ? "tester" : "analyst",
        run_no: index + 1,
        status: index % 3 === 0 ? "blocked" : "completed",
        trigger: "initial",
        node_id: `coding_node_${String(index + 1).padStart(4, "0")}`,
        started_at: `2026-06-13T00:00:${String(index).padStart(2, "0")}Z`,
        completed_at: null,
        supersedes_run_id: null,
        superseded_by_run_id: null,
        reason_code: "max_auto_rework_exceeded",
        raw_provider_output_refs: [
          "provider-raw/rework/very-long-role-run-output-reference-that-must-not-widen-page.txt",
        ],
        artifact_refs: [
          "artifacts/rework/very-long-analyst-evidence-reference-that-must-scroll-inside-panel.json",
        ],
      })),
    });

    render(<CodingWorkspacePage attemptId="coding_attempt_0001" onBack={vi.fn()} />);

    const panel = screen.getByTestId("coding-role-run-history");
    expect(panel).toHaveClass("min-w-0", "overflow-hidden");
    expect(panel.parentElement).toHaveClass("min-w-0", "overflow-hidden");
    expect(panel.parentElement?.parentElement).toHaveClass("min-w-0", "overflow-hidden");
    expect(panel.parentElement?.parentElement?.parentElement).toHaveClass(
      "min-w-0",
      "overflow-hidden",
    );
    expect(screen.getByRole("button", { name: "继续返修" })).toBeInTheDocument();
  });

  it("shows work item execution plan during prepare stage as non blocking by default", () => {
    mockCodingWs();
    useCodingWorkspaceStore.setState({
      ...readyCodingState(),
      stage: "prepare_context",
      workItemExecutionPlan: executionPlan({ status: "draft" }),
    });

    render(<CodingWorkspacePage attemptId="coding_attempt_0001" onBack={vi.fn()} />);

    expect(screen.getByText("执行计划")).toBeInTheDocument();
    expect(screen.getByText("实现后端 API")).toBeInTheDocument();
    expect(screen.getByText("src/product/**")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "确认执行计划" })).not.toBeInTheDocument();
  });

  it("shows confirm and change request actions when execution plan confirmation is required", () => {
    mockCodingWs();
    useCodingWorkspaceStore.setState({
      ...readyCodingState(),
      stage: "prepare_context",
      // 门禁开关来自 work item / snapshot 的 require_execution_plan_confirm，
      // 而非 plan 对象自身字段。
      requireExecutionPlanConfirm: true,
      workItemExecutionPlan: executionPlan({
        status: "draft",
      }),
    });

    render(<CodingWorkspacePage attemptId="coding_attempt_0001" onBack={vi.fn()} />);

    expect(screen.getByRole("button", { name: "确认执行计划" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "请求修改" })).toBeInTheDocument();
  });
});
