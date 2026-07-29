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
import { repairAwaitingConfirmationFixture } from "../components/coding-workspace/plan-repair-test-fixtures";
import { CodingWorkspacePage } from "./CodingWorkspacePage";
import {
  CODING_ATTEMPT_ADDRESS,
  DEFAULT_PERMISSION_MODES,
  executionPlan,
  installCodingWorkspacePageTestHooks,
  mockCodingWs,
  mockPlanRepairWs,
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

vi.mock("../hooks/useWorkspaceWs", () => ({
  useWorkspaceWs: vi.fn(),
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
  beforeEach(() => {
    mockPlanRepairWs();
  });

  it("keeps provider settings and role history out of the default chat layout", () => {
    mockCodingWs();
    useCodingWorkspaceStore.setState({
      attemptId: "coding_attempt_0001",
      status: "running",
      stage: "code_review",
      chatEntries: [],
      timelineNodes: [
        {
          id: "coding_node_review_0001",
          attempt_id: "coding_attempt_0001",
          stage: "code_review",
          title: "Code Review",
          status: "running",
          agent_role: "reviewer",
          summary: null,
          started_at: "2026-07-04T00:00:00Z",
          completed_at: null,
          artifact_refs: [],
        },
      ],
      roleRuns: [
        {
          id: "coding_role_run_0001",
          attempt_id: "coding_attempt_0001",
          stage: "code_review",
          role: "code_reviewer",
          run_no: 1,
          status: "running",
          trigger: "initial",
          node_id: "coding_node_review_0001",
          started_at: "2026-07-04T00:00:00Z",
          completed_at: null,
          supersedes_run_id: null,
          superseded_by_run_id: null,
          reason_code: null,
          raw_provider_output_refs: [],
          artifact_refs: [],
        },
      ],
      roleProviderConfigSnapshot: {
        coder: "fake",
        code_reviewer: "fake",
        internal_reviewer: "fake",
        review_rounds: 1,
        permission_modes: DEFAULT_PERMISSION_MODES,
      },
    });

    render(
      <CodingWorkspacePage
        address={CODING_ATTEMPT_ADDRESS}
        onBack={vi.fn()}
      />
    );

    expect(screen.getByTestId("coding-chat-entry-list")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Provider 设置" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "角色运行历史" })).toBeInTheDocument();
    expect(screen.queryByTestId("coding-provider-config-panel")).not.toBeInTheDocument();
    expect(screen.queryByTestId("coding-role-run-history")).not.toBeInTheDocument();
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

    render(
      <CodingWorkspacePage
        address={CODING_ATTEMPT_ADDRESS}
        onBack={vi.fn()}
      />
    );

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

    render(
      <CodingWorkspacePage
        address={CODING_ATTEMPT_ADDRESS}
        onBack={vi.fn()}
      />
    );

    await userEvent.click(screen.getByRole("button", { name: "运行结果" }));
    const tabs = screen.getByTestId("coding-artifact-tabs");
    expect(tabs).toHaveTextContent("error");
    expect(tabs).toHaveTextContent("src/solver.py:42");
    expect(tabs).toHaveTextContent("缺少 n=0 的处理");
    expect(tabs).toHaveTextContent("补充空输入测试");
    expect(screen.getByText("error").className).toContain("text-red");
  });

  it("renders GroupFinalReview impact scope and PR text suggestions", async () => {
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

    render(
      <CodingWorkspacePage
        address={CODING_ATTEMPT_ADDRESS}
        onBack={vi.fn()}
      />
    );

    await userEvent.click(screen.getByRole("button", { name: "运行结果" }));
    const tabs = screen.getByTestId("coding-artifact-tabs");
    expect(tabs).toHaveTextContent("GroupFinalReview");
    expect(tabs).not.toHaveTextContent("Internal PR Review");
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

    render(
      <CodingWorkspacePage
        address={CODING_ATTEMPT_ADDRESS}
        onBack={vi.fn()}
      />
    );

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

  it("renders role run history and selects linked timeline nodes", async () => {
    mockCodingWs();
    useCodingWorkspaceStore.setState({
      attemptId: "coding_attempt_0001",
      status: "blocked",
      stage: "code_review",
      timelineNodes: [
        {
          id: "coding_node_0003",
          attempt_id: "coding_attempt_0001",
          stage: "coding",
          title: "代码编写",
          status: "completed",
          agent_role: "author",
          summary: "代码编写完成",
          started_at: "2026-06-13T00:00:00Z",
          completed_at: "2026-06-13T00:00:01Z",
          artifact_refs: [],
        },
        {
          id: "coding_node_0004",
          attempt_id: "coding_attempt_0001",
          stage: "code_review",
          title: "Code Review",
          status: "blocked",
          agent_role: "reviewer",
          summary: "Code Reviewer 阻塞",
          started_at: "2026-06-13T00:00:02Z",
          completed_at: null,
          artifact_refs: [],
        },
      ],
      roleRuns: [
        {
          id: "coding_role_run_0001",
          attempt_id: "coding_attempt_0001",
          stage: "coding",
          role: "coder",
          run_no: 1,
          status: "completed",
          trigger: "initial",
          node_id: "coding_node_0003",
          started_at: "2026-06-13T00:00:00Z",
          completed_at: "2026-06-13T00:00:01Z",
          reason_code: null,
          raw_provider_output_refs: ["provider-raw/coding/coder_output_0001.txt"],
          artifact_refs: [],
        },
        {
          id: "coding_role_run_0002",
          attempt_id: "coding_attempt_0001",
          stage: "code_review",
          role: "code_reviewer",
          run_no: 1,
          status: "blocked",
          trigger: "retry_review",
          node_id: "coding_node_0004",
          started_at: "2026-06-13T00:00:02Z",
          completed_at: null,
          reason_code: "code_review_blocked",
          raw_provider_output_refs: ["provider-raw/code_review/code_review_0001.txt"],
          artifact_refs: [],
        },
      ],
    });

    render(
      <CodingWorkspacePage
        address={CODING_ATTEMPT_ADDRESS}
        onBack={vi.fn()}
      />
    );

    await userEvent.click(screen.getByRole("button", { name: "角色运行历史" }));

    const panel = screen.getByTestId("coding-role-run-history");
    expect(panel).toHaveTextContent("Coder #1");
    expect(panel).toHaveTextContent("Code Reviewer #1");
    expect(panel).toHaveTextContent("code_review_blocked");
    expect(panel).not.toHaveTextContent("provider-raw/coding/coder_output_0001.txt");

    await userEvent.click(screen.getByRole("button", { name: /Coder #1/ }));
    expect(panel).toHaveTextContent("provider-raw/coding/coder_output_0001.txt");

    await userEvent.click(screen.getByRole("button", { name: /Code Reviewer #1/ }));

    expect(useCodingWorkspaceStore.getState().selectedNodeId).toBe("coding_node_0004");
  });

  it("restores the unified plan repair timeline from visible projections", () => {
    const repair = repairAwaitingConfirmationFixture();
    mockCodingWs();
    mockPlanRepairWs();
    useCodingWorkspaceStore.setState({
      ...readyCodingState(),
      status: "awaiting_plan_amendment",
      stage: "code_review",
      activePlanRepair: repair,
      timelineNodes: [
        {
          id: "coding_node_review_0001",
          attempt_id: "coding_attempt_0001",
          stage: "code_review",
          title: "Code Review",
          status: "completed",
          agent_role: "reviewer",
          summary: "发现计划契约缺陷",
          started_at: "2026-07-20T00:00:00Z",
          completed_at: "2026-07-20T00:00:30Z",
          artifact_refs: [],
        },
        ...repair.timelineNodes,
      ],
    });

    render(
      <CodingWorkspacePage
        address={CODING_ATTEMPT_ADDRESS}
        onBack={vi.fn()}
      />,
    );

    expect(screen.getByRole("group", { name: "Plan Repair Timeline" }))
      .toHaveTextContent("等待一次性确认");
    expect(screen.getAllByText("修订 Work Item Contract")).toHaveLength(1);
  });
});
