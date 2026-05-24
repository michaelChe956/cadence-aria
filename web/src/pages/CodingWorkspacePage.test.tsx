import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useCodingWorkspaceWs } from "../hooks/useCodingWorkspaceWs";
import { useCodingWorkspaceStore } from "../state/coding-workspace-store";
import { CodingWorkspacePage } from "./CodingWorkspacePage";

vi.mock("../hooks/useCodingWorkspaceWs", () => ({
  useCodingWorkspaceWs: vi.fn(),
}));

vi.mock("../hooks/useUnloadGuard", () => ({
  useUnloadGuard: vi.fn(),
}));

type CodingWsApi = ReturnType<typeof useCodingWorkspaceWs>;

function mockCodingWs(overrides: Partial<CodingWsApi> = {}) {
  const api: CodingWsApi = {
    startCoding: vi.fn(),
    sendContextNote: vi.fn(),
    respondPermission: vi.fn(),
    respondGate: vi.fn(),
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

describe("CodingWorkspacePage", () => {
  beforeEach(() => {
    Object.defineProperty(HTMLElement.prototype, "scrollIntoView", {
      configurable: true,
      value: vi.fn(),
    });
    useCodingWorkspaceStore.getState().reset();
    vi.clearAllMocks();
  });

  it("renders coding workspace shell with timeline, chat, and artifact tabs", () => {
    mockCodingWs();
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
    expect(screen.getByTestId("coding-artifact-tabs")).toHaveTextContent("passed");
    expect(screen.getByTestId("coding-status-bar")).toHaveTextContent("testing");
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

    await userEvent.click(screen.getByRole("button", { name: "接受风险" }));

    expect(api.respondGate).toHaveBeenCalledWith("gate_0001", "accept_risk", undefined);
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

    await userEvent.click(screen.getByRole("button", { name: "logs" }));

    expect(screen.getByTestId("coding-artifact-tabs")).toHaveTextContent(
      "manual tab stays visible",
    );
    expect(screen.getByTestId("coding-artifact-tabs")).not.toHaveTextContent("passed");
  });

  it("renders review findings with severity, location, and required action", () => {
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

    const tabs = screen.getByTestId("coding-artifact-tabs");
    expect(tabs).toHaveTextContent("error");
    expect(tabs).toHaveTextContent("src/solver.py:42");
    expect(tabs).toHaveTextContent("缺少 n=0 的处理");
    expect(tabs).toHaveTextContent("补充空输入测试");
  });

  it("renders review request URL, push status, and manual instructions in the git tab", () => {
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

    const tabs = screen.getByTestId("coding-artifact-tabs");
    expect(tabs).toHaveTextContent("pushed");
    expect(screen.getByRole("link", { name: "https://git.example/review/1" })).toHaveAttribute(
      "href",
      "https://git.example/review/1",
    );
    expect(tabs).toHaveTextContent("打开平台创建 PR");
    expect(tabs).toHaveTextContent("选择 attempt 分支");
  });
});
