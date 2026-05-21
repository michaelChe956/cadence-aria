import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useUnloadGuard } from "../hooks/useUnloadGuard";
import { useWorkspaceWs } from "../hooks/useWorkspaceWs";
import type { ChatEntry } from "../state/chat-entries";
import {
  useWorkspaceStore,
  type TimelineNode,
} from "../state/workspace-ws-store";
import { ChatWorkspacePage } from "./ChatWorkspacePage";

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

describe("ChatWorkspacePage", () => {
  beforeEach(() => {
    window.localStorage.clear();
    Object.defineProperty(HTMLElement.prototype, "scrollIntoView", {
      configurable: true,
      value: vi.fn(),
    });
    useWorkspaceStore.getState().reset();
    vi.clearAllMocks();
  });

  it("renders chat workspace shell with timeline, chat list, artifact pane and status", () => {
    mockWorkspaceWs();
    useWorkspaceStore.setState({
      sessionId: "workspace_session_0001",
      workspaceType: "story",
      stage: "running",
      providers: { author: "claude_code", reviewer: "codex" },
      timelineNodes: [timelineNode()],
      activeNodeId: "node-1",
      selectedNodeId: "node-1",
      chatEntries: [chatEntry({ node_id: "node-1", content: "review output" })],
      artifactVersions: [
        {
          version: 1,
          markdown: "# Artifact v1\n\n内容",
          generated_by: "claude_code",
          created_at: "2026-05-21T10:00:00Z",
          source_node_id: "node-1",
        },
      ],
    });

    render(<ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />);

    expect(screen.getAllByText(/Story Spec #workspace_session_0001/).length).toBeGreaterThan(0);
    expect(screen.getByTestId("timeline-node-list")).toBeInTheDocument();
    expect(screen.getByTestId("chat-entry-list")).toHaveTextContent("review output");
    expect(screen.getByTestId("artifact-pane")).toHaveTextContent("Artifact v1");
    expect(screen.getByTestId("workspace-status-bar")).toHaveTextContent("running");
  });

  it("starts generation with provider config from the chat input", async () => {
    const api = mockWorkspaceWs();
    useWorkspaceStore.setState({
      sessionId: "workspace_session_0001",
      stage: "prepare_context",
      providers: { author: "fake", reviewer: "codex" },
      reviewerEnabled: true,
      reviewRounds: 2,
    });

    render(<ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />);

    await userEvent.click(screen.getByTestId("start-generation"));

    expect(api.sendStartGeneration).toHaveBeenCalledWith(
      { author: "fake", reviewer: "codex", review_rounds: 2 },
      true,
    );
  });

  it("sends permission responses from permission request entries", async () => {
    const api = mockWorkspaceWs();
    useWorkspaceStore.setState({
      sessionId: "workspace_session_0001",
      chatEntries: [
        chatEntry({
          type: "permission_request",
          role: "system",
          content: "shell · cargo test",
          metadata: {
            request_id: "perm_001",
            request: {
              tool_name: "shell",
              description: "cargo test",
              risk_level: "medium",
            },
            risk_level: "medium",
          },
        }),
      ],
    });

    render(<ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />);

    await userEvent.click(screen.getByRole("button", { name: "允许" }));

    expect(api.respondPermission).toHaveBeenCalledWith("perm_001", true, undefined);
  });

  it("selects timeline nodes and scrolls to their first chat entry", async () => {
    mockWorkspaceWs();
    const scrollIntoView = vi.fn();
    Object.defineProperty(HTMLElement.prototype, "scrollIntoView", {
      configurable: true,
      value: scrollIntoView,
    });
    useWorkspaceStore.setState({
      sessionId: "workspace_session_0001",
      timelineNodes: [
        timelineNode({ node_id: "node-1", node_type: "context_note", title: "补充上下文" }),
        timelineNode({ node_id: "node-2", node_type: "author_run", title: "Story Spec 生成" }),
      ],
      activeNodeId: "node-2",
      selectedNodeId: "node-1",
      chatEntries: [
        chatEntry({ id: "entry-1", node_id: "node-1", content: "第一条" }),
        chatEntry({ id: "entry-2", node_id: "node-2", content: "第二条" }),
      ],
    });

    render(<ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />);

    await userEvent.click(screen.getByTestId("timeline-node-author_run"));

    expect(useWorkspaceStore.getState().selectedNodeId).toBe("node-2");
    expect(scrollIntoView).toHaveBeenCalled();
  });

  it("renders protocol errors and enables unload guard while running", () => {
    mockWorkspaceWs();
    useWorkspaceStore.setState({
      sessionId: "workspace_session_0001",
      stage: "running",
      protocolError: {
        code: "INVALID_MESSAGE_FOR_STAGE",
        message: "message context_note not allowed in stage running",
      },
    });

    render(<ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />);

    expect(screen.getByTestId("protocol-error-alert")).toHaveTextContent(
      "INVALID_MESSAGE_FOR_STAGE",
    );
    expect(useUnloadGuard).toHaveBeenCalledWith({
      enabled: true,
      message: "运行中。刷新/关闭将中止当前 Provider 运行，是否继续？",
    });
  });

  it("hides review verdict path buttons once the workspace reaches human_confirm", () => {
    mockWorkspaceWs();
    useWorkspaceStore.setState({
      sessionId: "workspace_session_0001",
      stage: "human_confirm",
      chatEntries: [
        chatEntry({
          type: "review_verdict",
          role: "reviewer",
          content: "可以进入人工确认",
          metadata: {
            verdict: "pass",
            comments: "覆盖核心路径",
            summary: "可以进入人工确认",
          },
        }),
      ],
    });

    render(<ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />);

    expect(screen.getByTestId("review-verdict-entry")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "接受修订建议" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "补充上下文后修订" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "跳过，人工处理" })).not.toBeInTheDocument();
  });
});

function timelineNode(overrides: Partial<TimelineNode> = {}): TimelineNode {
  return {
    node_id: "node-1",
    node_type: "reviewer_run",
    agent: "codex",
    stage: "cross_review",
    round: 1,
    status: "active",
    title: "Review Round 1",
    summary: "正在审核",
    started_at: "2026-05-20T00:00:00Z",
    completed_at: null,
    duration_ms: null,
    artifact_ref: null,
    provider_config_snapshot: {
      author: "claude_code",
      reviewer: "codex",
      review_rounds: 1,
    },
    ...overrides,
  };
}

function chatEntry(overrides: Partial<ChatEntry> = {}): ChatEntry {
  return {
    id: "entry-1",
    type: "provider_stream",
    role: "reviewer",
    content: "review output",
    timestamp: "2026-05-20T00:00:00Z",
    ...overrides,
  };
}
