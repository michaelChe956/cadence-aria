import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useUnloadGuard } from "../hooks/useUnloadGuard";
import { useWorkspaceWs } from "../hooks/useWorkspaceWs";
import {
  useWorkspaceStore,
  type TimelineNode,
  type TimelineNodeDetail,
} from "../state/workspace-ws-store";
import { WorkspacePage } from "./WorkspacePage";

vi.mock("../hooks/useWorkspaceWs", () => ({
  useWorkspaceWs: vi.fn(),
}));

vi.mock("../hooks/useUnloadGuard", () => ({
  useUnloadGuard: vi.fn(),
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
    isReconnecting: false,
    reconnectAttemptCount: 0,
    retryNow: vi.fn(),
    ...overrides,
  };
  vi.mocked(useWorkspaceWs).mockReturnValue(api);
  return api;
}

describe("WorkspacePage", () => {
  beforeEach(() => {
    window.localStorage.clear();
    useWorkspaceStore.getState().reset();
    vi.clearAllMocks();
  });

  it("renders workspace header, timeline, node detail panel, and stage actions", async () => {
    mockWorkspaceWs();
    useWorkspaceStore.setState({
      sessionId: "workspace_session_0001",
      workspaceType: "story",
      stage: "running",
      providers: { author: "claude_code", reviewer: "codex" },
      timelineNodes: [timelineNode()],
      selectedNodeId: "node-1",
      nodeDetails: { "node-1": nodeDetail({ streaming_content: "review output" }) },
    });

    render(<WorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />);

    expect(screen.getByText(/Story Spec #workspace_session_0001/)).toBeInTheDocument();
    expect(screen.getAllByText("Review Round 1").length).toBeGreaterThan(0);
    expect(screen.getByTestId("node-detail-panel")).toBeInTheDocument();
    expect(screen.getByTestId("stage-actions-bar")).toBeInTheDocument();

    await userEvent.click(screen.getByTestId("tab-streaming"));
    expect(screen.getByTestId("streaming-content")).toHaveTextContent("review output");
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

  it("starts generation from a prepared workspace through protocol v2", async () => {
    const api = mockWorkspaceWs();
    useWorkspaceStore.setState({
      sessionId: "workspace_session_0001",
      stage: "prepare_context",
      providers: { author: "fake", reviewer: "codex" },
    });

    render(<WorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />);

    expect(screen.getByTestId("stage-badge")).toHaveTextContent("准备中");
    expect(screen.getByTestId("prepare-context-panel")).toBeInTheDocument();

    await userEvent.click(screen.getByTestId("start-generation"));

    expect(api.sendStartGeneration).toHaveBeenCalledWith(
      { author: "fake", reviewer: "codex", review_rounds: 1 },
      true,
    );
    expect(api.startGeneration).not.toHaveBeenCalled();
    expect(api.sendMessage).not.toHaveBeenCalled();
  });

  it("keeps start generation disabled until the session snapshot is hydrated", async () => {
    const api = mockWorkspaceWs();
    useWorkspaceStore.setState({
      sessionId: null,
      stage: "prepare_context",
      providers: null,
    });

    render(<WorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />);

    expect(screen.getAllByRole("button", { name: "开始生成" })[0]).toBeDisabled();
    await userEvent.click(screen.getAllByRole("button", { name: "开始生成" })[0]);
    expect(api.sendStartGeneration).not.toHaveBeenCalled();
  });

  it("opens provider config from a button dialog and disables controls outside prepare context", async () => {
    mockWorkspaceWs();
    useWorkspaceStore.setState({
      stage: "running",
      providers: { author: "claude_code", reviewer: "codex" },
    });

    render(<WorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />);

    expect(screen.queryByLabelText("Author")).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Provider 配置" }));

    expect(screen.getByRole("dialog", { name: "Provider 配置" })).toBeInTheDocument();
    expect(screen.getByLabelText("Author")).toBeDisabled();
    expect(screen.getByLabelText("Reviewer")).toBeDisabled();
  });

  it("passes provider locked timestamp to the header", () => {
    mockWorkspaceWs();
    useWorkspaceStore.setState({
      stage: "running",
      providerLocked: true,
      providerLockedAt: "2026-05-20T14:35:00Z",
      providers: { author: "claude_code", reviewer: "codex" },
    });

    render(<WorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />);

    expect(screen.getByLabelText("Provider 已锁定")).toHaveAttribute(
      "data-locked-at",
      "2026-05-20T14:35:00Z",
    );
  });

  it("renders workspace discipline flags from the hydrated session snapshot", () => {
    mockWorkspaceWs();
    useWorkspaceStore.getState().setSessionState({
      session_id: "workspace_session_0001",
      workspace_type: "story",
      stage: "prepare_context",
      messages: [],
      checkpoints: [],
      artifact: null,
      providers: { author: "codex", reviewer: "claude_code" },
      superpowers_enabled: true,
      openspec_enabled: true,
    } as never);

    render(<WorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />);

    expect(screen.getByText("Superpowers: on")).toBeInTheDocument();
    expect(screen.getByText("OpenSpec: on")).toBeInTheDocument();
  });

  it("sends review decision paths through protocol v2", async () => {
    const api = mockWorkspaceWs();
    useWorkspaceStore.setState({
      stage: "review_decision",
      providers: { author: "claude_code", reviewer: "codex" },
      pendingReviewDecision: { verdict: "revise", summary: "缺少边界场景" },
    });

    render(<WorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />);

    await userEvent.click(screen.getByRole("button", { name: "确定路径" }));
    expect(api.sendSelectRevisionPath).toHaveBeenCalledWith("revise", undefined);

    await userEvent.click(screen.getByLabelText("补充上下文后返修"));
    await userEvent.type(screen.getByLabelText("补充上下文"), "补充登录错误码");
    await userEvent.click(screen.getByRole("button", { name: "确定路径" }));
    expect(api.sendSelectRevisionPath).toHaveBeenCalledWith(
      "revise-with-context",
      "补充登录错误码",
    );

    await userEvent.click(screen.getByLabelText("跳过审核结论，进入人工确认"));
    await userEvent.click(screen.getByRole("button", { name: "确定路径" }));
    expect(api.sendSelectRevisionPath).toHaveBeenCalledWith("skip-to-human", undefined);
    expect(api.sendReviewDecision).not.toHaveBeenCalled();
  });

  it("sends human confirmation and structured feedback through protocol v2", async () => {
    const api = mockWorkspaceWs();
    useWorkspaceStore.setState({
      stage: "human_confirm",
      artifactVersions: [
        {
          version: 1,
          markdown: "# v1",
          generated_by: "claude_code",
          created_at: "2026-05-20T00:00:00Z",
          source_node_id: "node-0",
        },
        {
          version: 2,
          markdown: "# v2",
          generated_by: "claude_code",
          created_at: "2026-05-20T00:01:00Z",
          source_node_id: "node-1",
        },
      ],
      pendingReviewerSummary: { verdict: "pass", points: ["边界场景已补齐"] },
    });

    render(<WorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />);

    await userEvent.click(screen.getAllByRole("button", { name: "确认" })[0]);
    expect(api.sendHumanConfirm).toHaveBeenCalledWith("confirm");

    await userEvent.click(screen.getAllByRole("button", { name: "要求修改" })[0]);
    await userEvent.click(screen.getByLabelText("内容缺失"));
    await userEvent.type(screen.getByLabelText("具体描述"), "缺少错误处理");
    await userEvent.click(screen.getByRole("button", { name: "提交" }));

    expect(api.sendHumanConfirm).toHaveBeenCalledWith("request-change", {
      feedback_types: ["内容缺失"],
      description: "缺少错误处理",
      target_artifact_version: 2,
    });
    expect(api.confirm).not.toHaveBeenCalled();
  });

  it("aborts running workspace from StageActionsBar", async () => {
    const api = mockWorkspaceWs();
    useWorkspaceStore.setState({ stage: "running" });

    render(<WorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />);

    await userEvent.click(screen.getByRole("button", { name: "中止" }));

    expect(api.abort).toHaveBeenCalled();
  });

  it("enables unload guard while provider work is running", () => {
    mockWorkspaceWs();
    useWorkspaceStore.setState({ stage: "running" });

    render(<WorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />);

    expect(useUnloadGuard).toHaveBeenCalledWith({
      enabled: true,
      message: "运行中。刷新/关闭将中止当前 Provider 运行，是否继续？",
    });
  });

  it("renders reconnect progress banner and retries manually", async () => {
    const retryNow = vi.fn();
    mockWorkspaceWs({
      isReconnecting: true,
      reconnectAttemptCount: 2,
      retryNow,
    });

    render(<WorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />);

    expect(screen.getByText(/重连中/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "手动重连" }));
    expect(retryNow).toHaveBeenCalled();
  });

  it("renders aborted-by-disconnect banner and stores acknowledgement", async () => {
    mockWorkspaceWs();
    useWorkspaceStore.setState({
      timelineNodes: [
        {
          ...timelineNode(),
          node_id: "node-aborted-1",
          node_type: "aborted_by_disconnect",
          status: "failed",
          title: "运行因断开中止",
        },
      ],
    });

    render(<WorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />);

    expect(screen.getByText(/上次运行因断开被中止/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "我知道了" }));

    expect(useWorkspaceStore.getState().acknowledgedAbortedNodes).toEqual(["node-aborted-1"]);
  });

  it("renders protocol errors from workspace websocket", () => {
    mockWorkspaceWs();
    useWorkspaceStore.setState({
      protocolError: {
        code: "INVALID_MESSAGE_FOR_STAGE",
        message: "message context_note not allowed in stage running",
      },
    });

    render(<WorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />);

    expect(screen.getByRole("alert")).toHaveTextContent("INVALID_MESSAGE_FOR_STAGE");
    expect(screen.getByRole("alert")).toHaveTextContent("context_note");
  });
});

function timelineNode(): TimelineNode {
  return {
    node_id: "node-1",
    node_type: "reviewer_run" as const,
    agent: "codex",
    stage: "cross_review",
    round: 1,
    status: "active" as const,
    title: "Review Round 1",
    summary: "正在审核",
    started_at: "2026-05-20T00:00:00Z",
    completed_at: null,
    duration_ms: null,
    artifact_ref: null,
    provider_config_snapshot: {
      author: "claude_code" as const,
      reviewer: "codex" as const,
      review_rounds: 1,
    },
  };
}

function nodeDetail(overrides?: Partial<TimelineNodeDetail>): TimelineNodeDetail {
  return {
    node_id: "node-1",
    session_id: "workspace_session_0001",
    node_type: "reviewer_run",
    status: "active",
    agent_role: "reviewer",
    provider: { name: "codex", model: "gpt-5" },
    messages: [],
    streaming_content: "",
    execution_events: [],
    permission_events: [],
    verdict: null,
    artifact_ref: null,
    is_revision: false,
    base_artifact_ref: null,
    started_at: "2026-05-20T00:00:00Z",
    ended_at: null,
    ...overrides,
  };
}
