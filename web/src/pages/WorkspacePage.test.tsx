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
    sendContextNote: vi.fn(),
    sendStartGeneration: vi.fn(),
    sendSelectRevisionPath: vi.fn(),
    sendHumanConfirm: vi.fn(),
    sendHello: vi.fn(),
    sendPing: vi.fn(),
    startGeneration: vi.fn(),
    rollback: vi.fn(),
    confirm: vi.fn(),
    abort: vi.fn(),
    selectProvider: vi.fn(),
    sendProviderSelect: vi.fn(),
    sendReviewDecision: vi.fn(),
    respondPermission: vi.fn(),
    sendPermissionResponse: vi.fn(),
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
    expect(screen.getAllByText("Codex").length).toBeGreaterThan(0);
  });

  it("starts generation from a prepared workspace without requiring typed input", async () => {
    const api = mockWorkspaceWs();
    useWorkspaceStore.setState({
      stage: "prepare_context",
      providers: { author: "fake", reviewer: "codex" },
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

    expect(screen.getByTestId("stage-badge")).toHaveTextContent("准备中");
    expect(screen.getByTestId("prepare-context-panel")).toBeInTheDocument();
    expect(screen.queryByPlaceholderText("输入消息...")).not.toBeInTheDocument();

    await userEvent.click(screen.getByTestId("start-generation"));

    expect(api.sendStartGeneration).toHaveBeenCalledWith(
      { author: "fake", reviewer: "codex", review_rounds: 1 },
      true,
    );
    expect(api.startGeneration).not.toHaveBeenCalled();
    expect(api.sendMessage).not.toHaveBeenCalled();
  });

  it("sends prepare-context notes through protocol v2 without local optimistic append", async () => {
    const api = mockWorkspaceWs();
    useWorkspaceStore.setState({
      stage: "prepare_context",
      providers: { author: "claude_code", reviewer: "codex" },
      timelineNodes: [
        {
          node_id: "timeline_node_context_001",
          node_type: "context_note",
          agent: "claude_code",
          stage: "prepare_context",
          round: null,
          status: "completed",
          title: "Context Note",
          summary: "后端确认的上下文",
          started_at: "2026-05-20T00:00:00Z",
          completed_at: "2026-05-20T00:00:01Z",
          duration_ms: 1000,
          artifact_ref: null,
          provider_config_snapshot: {
            author: "claude_code",
            reviewer: "codex",
            review_rounds: 1,
          },
        },
      ],
      nodeDetails: {
        timeline_node_context_001: {
          node_id: "timeline_node_context_001",
          session_id: "workspace_session_0001",
          node_type: "context_note",
          status: "completed",
          agent_role: null,
          provider: null,
          messages: [],
          streaming_content: "后端详情中的上下文",
          execution_events: [],
          permission_events: [],
          verdict: null,
          artifact_ref: null,
          is_revision: false,
          base_artifact_ref: null,
          started_at: "2026-05-20T00:00:00Z",
          ended_at: "2026-05-20T00:00:01Z",
        },
      },
    });

    render(<WorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />);

    expect(screen.getByTestId("timeline-node-context_note")).toBeInTheDocument();
    expect(screen.getAllByText("后端详情中的上下文").length).toBeGreaterThan(0);

    await userEvent.type(screen.getByTestId("context-note-input"), "补充验收标准");
    await userEvent.click(screen.getByTestId("send-context-note"));

    expect(api.sendContextNote).toHaveBeenCalledWith("补充验收标准");
    expect(api.sendMessage).not.toHaveBeenCalled();
    expect(screen.queryByText("补充验收标准")).not.toBeInTheDocument();
  });

  it("keeps provider config visible and disables controls outside prepare context", () => {
    mockWorkspaceWs();
    useWorkspaceStore.setState({
      stage: "running",
      providers: { author: "claude_code", reviewer: "codex" },
    });

    render(<WorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />);

    expect(screen.getByLabelText("Provider 配置")).toBeInTheDocument();
    expect(screen.getByLabelText("Author")).toBeDisabled();
    expect(screen.getByLabelText("Reviewer")).toBeDisabled();
    expect(screen.queryByTitle("Provider 配置")).not.toBeInTheDocument();
  });

  it("sends human confirmation through protocol v2", async () => {
    const api = mockWorkspaceWs();
    useWorkspaceStore.setState({
      stage: "human_confirm",
    });

    render(<WorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />);

    await userEvent.click(screen.getByRole("button", { name: "确认通过" }));

    expect(api.sendHumanConfirm).toHaveBeenCalledWith("confirm");
    expect(api.confirm).not.toHaveBeenCalled();
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
          node_type: "reviewer_run",
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
          node_id: "timeline_node_001",
          session_id: "session_001",
          node_type: "reviewer_run",
          status: "active",
          agent_role: "reviewer",
          provider: { name: "codex", model: "gpt-5" },
          messages: [],
          streaming_content: "review output",
          execution_events: [],
          permission_events: [],
          verdict: null,
          artifact_ref: null,
          is_revision: false,
          base_artifact_ref: null,
          started_at: "2026-05-19T00:00:00Z",
          ended_at: null,
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
    expect(api.sendSelectRevisionPath).toHaveBeenCalledWith("revise", undefined);

    await userEvent.click(screen.getByRole("button", { name: "补充信息后返修" }));
    await userEvent.type(screen.getByLabelText("返修补充信息"), "补充登录错误码");
    await userEvent.click(screen.getByRole("button", { name: "提交返修" }));
    expect(api.sendSelectRevisionPath).toHaveBeenCalledWith(
      "revise-with-context",
      "补充登录错误码",
    );

    await userEvent.click(screen.getByRole("button", { name: "人工介入" }));
    expect(api.sendSelectRevisionPath).toHaveBeenCalledWith("skip-to-human", undefined);
    expect(api.sendReviewDecision).not.toHaveBeenCalled();
  });
});
