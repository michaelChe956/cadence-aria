import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
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

describe("CodingWorkspacePage reports and history", () => {
  installCodingWorkspacePageTestHooks();

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
});
