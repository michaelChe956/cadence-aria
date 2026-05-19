import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useWorkspaceWs } from "../hooks/useWorkspaceWs";
import { useWorkspaceStore } from "../state/workspace-ws-store";
import { WorkspacePage } from "./WorkspacePage";

vi.mock("../hooks/useWorkspaceWs", () => ({
  useWorkspaceWs: vi.fn(),
}));

type WorkspaceWsApi = ReturnType<typeof useWorkspaceWs>;

function mockWorkspaceWs(overrides: Partial<WorkspaceWsApi> = {}) {
  const api: WorkspaceWsApi = {
    sendMessage: vi.fn(),
    startGeneration: vi.fn(),
    rollback: vi.fn(),
    confirm: vi.fn(),
    abort: vi.fn(),
    selectProvider: vi.fn(),
    sendReviewDecision: vi.fn(),
    respondPermission: vi.fn(),
    connectionStatus: "connected",
    ...overrides,
  };
  vi.mocked(useWorkspaceWs).mockReturnValue(api);
  return api;
}

describe("WorkspacePage", () => {
  beforeEach(() => {
    useWorkspaceStore.getState().reset();
    vi.clearAllMocks();
  });

  it("renders permission request card and sends approval", async () => {
    const api = mockWorkspaceWs();
    useWorkspaceStore.setState({
      pendingPermissions: [
        {
          id: "perm_001",
          tool_name: "bash",
          description: "Run cargo test",
          risk_level: "medium",
        },
      ],
    });

    render(<WorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />);

    expect(screen.getByText("bash")).toBeInTheDocument();
    expect(screen.getByText("Run cargo test")).toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: "允许" }));

    expect(api.respondPermission).toHaveBeenCalledWith("perm_001", true, undefined);
  });

  it("shows provider command progress in the execution tab", async () => {
    mockWorkspaceWs();
    useWorkspaceStore.setState({
      providerStatus: "running",
      executionEvents: [
        {
          event_id: "command_cmd_001",
          agent: "codex",
          kind: "command",
          status: "completed",
          title: "Command completed",
          detail: "exit code 0",
          command: "pwd",
          cwd: "/tmp/repo",
          output: "/tmp/repo\n",
          exit_code: 0,
        },
      ],
    });

    render(<WorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />);
    await userEvent.click(screen.getByRole("button", { name: "执行" }));

    expect(screen.getByText("运行中")).toBeInTheDocument();
    expect(screen.getByText("pwd")).toBeInTheDocument();
    expect(screen.getByText("/tmp/repo")).toBeInTheDocument();
    expect(screen.getByText(/exit code 0/)).toBeInTheDocument();
    expect(screen.getByText("Codex")).toBeInTheDocument();
  });

  it("starts generation from a prepared workspace without requiring typed input", async () => {
    const api = mockWorkspaceWs();
    useWorkspaceStore.setState({
      stage: "prepare_context",
      messages: [
        {
          id: "msg_001",
          role: "system",
          content: "Workspace 生成任务已准备",
          checkpoint_id: null,
          created_at: "2026-05-18T00:00:00Z",
        },
      ],
    });

    render(<WorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />);

    await userEvent.click(screen.getByRole("button", { name: "开始生成" }));

    expect(api.startGeneration).toHaveBeenCalledTimes(1);
    expect(api.sendMessage).not.toHaveBeenCalled();
  });

  it("marks fast intermediate stages as visited in the flow rail", () => {
    mockWorkspaceWs();
    useWorkspaceStore.setState({
      stage: "human_confirm",
      visitedStages: ["prepare_context", "running", "cross_review", "human_confirm"],
    });

    render(<WorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />);

    expect(screen.getByLabelText("运行中 已经过")).toBeInTheDocument();
    expect(screen.getByLabelText("交叉审查 已经过")).toBeInTheDocument();
    expect(screen.getByLabelText("人工确认 当前阶段")).toBeInTheDocument();
  });

  it("renders timeline nodes with agent badge and selected node detail", () => {
    mockWorkspaceWs();
    useWorkspaceStore.setState({
      timelineNodes: [
        {
          node_id: "timeline_node_001",
          node_type: "review",
          agent: "codex",
          stage: "cross_review",
          round: 1,
          status: "active",
          title: "Review Round 1",
          summary: "正在审核",
          started_at: "2026-05-19T00:00:00Z",
          completed_at: null,
          duration_ms: null,
          artifact_ref: "artifact_current",
          provider_config_snapshot: {
            author: "claude_code",
            reviewer: "codex",
            review_rounds: 2,
          },
        },
      ],
      selectedNodeId: "timeline_node_001",
      activeNodeId: "timeline_node_001",
      nodeDetails: {
        timeline_node_001: {
          nodeId: "timeline_node_001",
          messages: [],
          streamingContent: "review output",
          executionEvents: [],
          verdict: null,
        },
      },
    });

    render(<WorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />);

    expect(screen.getAllByText("Review Round 1").length).toBeGreaterThan(0);
    expect(screen.getAllByText("Codex").length).toBeGreaterThan(0);
    expect(screen.getByText("review output")).toBeInTheDocument();
  });

  it("locks provider selects after generation starts", async () => {
    mockWorkspaceWs();
    useWorkspaceStore.setState({
      stage: "running",
      providers: { author: "claude_code", reviewer: "codex" },
    });

    render(<WorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />);
    await userEvent.click(screen.getByTitle("Provider 配置"));

    expect(screen.getByLabelText("Author")).toBeDisabled();
    expect(screen.getByLabelText("Reviewer")).toBeDisabled();
  });

  it("renders review decision actions and sends the selected decision", async () => {
    const api = mockWorkspaceWs();
    useWorkspaceStore.setState({
      stage: "review_decision",
      visitedStages: ["prepare_context", "running", "cross_review"],
      pendingDecision: {
        node_id: "timeline_node_004",
        round: 1,
        options: ["continue", "continue_with_context", "human_intervene"],
      },
    });

    render(<WorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />);

    expect(screen.getByLabelText("交叉审查 当前阶段")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "直接返修" }));
    expect(api.sendReviewDecision).toHaveBeenCalledWith("continue", undefined);

    await userEvent.click(screen.getByRole("button", { name: "补充信息后返修" }));
    await userEvent.type(screen.getByLabelText("返修补充信息"), "补充登录错误码");
    await userEvent.click(screen.getByRole("button", { name: "提交返修" }));
    expect(api.sendReviewDecision).toHaveBeenCalledWith(
      "continue_with_context",
      "补充登录错误码",
    );

    await userEvent.click(screen.getByRole("button", { name: "人工介入" }));
    expect(api.sendReviewDecision).toHaveBeenCalledWith("human_intervene", undefined);
  });
});
