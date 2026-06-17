import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  fetchWorkspaceArtifactVersion,
  fetchWorkspaceEventOutput,
  fetchWorkspaceNodeDetail,
} from "../api/workspace-content";
import type { NodeDetail } from "../api/types";
import { useUnloadGuard } from "../hooks/useUnloadGuard";
import { useWorkspaceWs } from "../hooks/useWorkspaceWs";
import type { ChatEntry } from "../state/chat-entries";
import {
  emptyWorkspaceContentCache,
  workspaceContentCacheValues,
} from "../state/workspace-content-cache";
import {
  selectChatPanelState,
  selectWorkspaceHeaderState,
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

vi.mock("../api/workspace-content", () => ({
  fetchWorkspaceArtifactVersion: vi.fn(),
  fetchWorkspaceEventOutput: vi.fn(),
  fetchWorkspaceNodeDetail: vi.fn(),
  fetchWorkspacePrompt: vi.fn(),
}));

vi.mock("../components/shared/MonacoViewer", () => ({
  MonacoViewer: ({ value, height }: { value: string; height?: string }) => (
    <div data-testid="monaco-viewer" data-height={height}>
      {value}
    </div>
  ),
}));

vi.mock("../components/shared/MonacoDiffViewer", () => ({
  MonacoDiffViewer: ({ original, modified }: { original: string; modified: string }) => (
    <div data-testid="monaco-diff-viewer">
      {original}
      {modified}
    </div>
  ),
}));

type WorkspaceWsApi = ReturnType<typeof useWorkspaceWs>;

function mockWorkspaceWs(overrides: Partial<WorkspaceWsApi> = {}) {
  const api: WorkspaceWsApi = {
    sendMessage: vi.fn(),
    sendContextNote: vi.fn(),
    sendStartGeneration: vi.fn(),
    sendSelectRevisionPath: vi.fn(),
    sendAuthorDecision: vi.fn(),
    sendRequestRevision: vi.fn(),
    sendRevertWorkItem: vi.fn(),
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
    sendChoiceResponse: vi.fn(),
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

  it("renders chat workspace shell with timeline and keeps artifact content secondary until selected", async () => {
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
    expect(screen.queryByTestId("monaco-viewer")).not.toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: "Artifact" }));

    expect(screen.getByTestId("monaco-viewer")).toHaveTextContent("Artifact v1");
    expect(screen.getByTestId("workspace-status-bar")).toHaveTextContent("running");
  });

  it("loads artifact summary markdown through the workspace content cache", async () => {
    mockWorkspaceWs();
    let resolveArtifact!: (value: { version: number; markdown: string }) => void;
    vi.mocked(fetchWorkspaceArtifactVersion).mockReturnValue(
      new Promise((resolve) => {
        resolveArtifact = resolve;
      }),
    );
    useWorkspaceStore.setState({
      sessionId: "workspace_session_0001",
      workspaceType: "story",
      stage: "completed",
      providers: { author: "claude_code", reviewer: "codex" },
      artifactVersions: [
        {
          version: 1,
          generated_by: "claude_code",
          created_at: "2026-05-21T10:00:00Z",
          source_node_id: "node-1",
        },
      ],
      artifactContentCache: emptyWorkspaceContentCache(),
    });

    render(<ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />);
    await userEvent.click(screen.getByRole("button", { name: "Artifact" }));

    await waitFor(() => {
      expect(screen.getByTestId("artifact-loading")).toHaveTextContent("正在加载 v1");
    });
    resolveArtifact({ version: 1, markdown: "# Loaded Artifact\n\n内容" });

    expect(await screen.findByText(/Loaded Artifact/)).toBeInTheDocument();
    expect(fetchWorkspaceArtifactVersion).toHaveBeenCalledWith("workspace_session_0001", 1);
    expect(workspaceContentCacheValues(useWorkspaceStore.getState().artifactContentCache)["1"]).toBe(
      "# Loaded Artifact\n\n内容",
    );
  });

  it("does not cache artifact content when the workspace session changes before load resolves", async () => {
    mockWorkspaceWs();
    let resolveArtifact!: (value: { version: number; markdown: string }) => void;
    vi.mocked(fetchWorkspaceArtifactVersion).mockReturnValue(
      new Promise((resolve) => {
        resolveArtifact = resolve;
      }),
    );
    useWorkspaceStore.setState({
      sessionId: "workspace_session_0001",
      workspaceType: "story",
      stage: "completed",
      providers: { author: "claude_code", reviewer: "codex" },
      artifactVersions: [
        {
          version: 1,
          generated_by: "claude_code",
          created_at: "2026-05-21T10:00:00Z",
          source_node_id: "node-1",
        },
      ],
      artifactContentCache: emptyWorkspaceContentCache(),
    });

    render(<ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />);
    await userEvent.click(screen.getByRole("button", { name: "Artifact" }));
    await waitFor(() => expect(fetchWorkspaceArtifactVersion).toHaveBeenCalledWith("workspace_session_0001", 1));

    useWorkspaceStore.setState({
      sessionId: "workspace_session_0002",
      artifactContentCache: emptyWorkspaceContentCache(),
    });
    resolveArtifact({ version: 1, markdown: "# Stale Artifact" });
    await Promise.resolve();

    expect(workspaceContentCacheValues(useWorkspaceStore.getState().artifactContentCache)["1"]).toBeUndefined();
  });

  it("does not cache chat content when the workspace session changes before load resolves", async () => {
    mockWorkspaceWs();
    let resolveOutput!: (value: { node_id: string; event_id: string; output: string }) => void;
    vi.mocked(fetchWorkspaceEventOutput).mockReturnValue(
      new Promise((resolve) => {
        resolveOutput = resolve;
      }),
    );
    useWorkspaceStore.setState({
      sessionId: "workspace_session_0001",
      workspaceType: "story",
      stage: "running",
      providers: { author: "codex", reviewer: "claude_code" },
      contentCache: emptyWorkspaceContentCache(),
      chatEntries: [
        chatEntry({
          id: "entry-stream",
          type: "provider_stream",
          role: "author",
          content: "stream summary",
          node_id: "timeline_node_001",
        }),
        chatEntry({
          id: "entry-output",
          type: "execution_event",
          role: "author",
          content: "Execution Output · 按需加载",
          node_id: "timeline_node_001",
          content_ref: {
            kind: "execution_output",
            nodeId: "timeline_node_001",
            eventId: "timeline_node_001_output",
          },
          metadata: {
            event_id: "timeline_node_001_output",
            title: "Execution Output",
            detail: "Provider execution output 按需加载",
          },
        }),
      ],
    });

    render(<ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />);
    await userEvent.click(screen.getByRole("button", { name: /Execution Output/ }));
    await waitFor(() => {
      expect(fetchWorkspaceEventOutput).toHaveBeenCalledWith(
        "workspace_session_0001",
        "timeline_node_001",
        "timeline_node_001_output",
      );
    });

    useWorkspaceStore.setState({
      sessionId: "workspace_session_0002",
      contentCache: emptyWorkspaceContentCache(),
    });
    resolveOutput({
      node_id: "timeline_node_001",
      event_id: "timeline_node_001_output",
      output: "stale output",
    });
    await waitFor(() => {
      expect(fetchWorkspaceEventOutput).toHaveResolved();
    });

    expect(workspaceContentCacheValues(useWorkspaceStore.getState().contentCache)).toEqual({});
  });

  it("hydrates selected node detail after restored lightweight session state", async () => {
    mockWorkspaceWs();
    vi.mocked(fetchWorkspaceNodeDetail).mockResolvedValue(
      makeNodeDetail({
        node_id: "timeline_node_017",
        node_type: "reviewer_run",
        streaming_content: "完整 review 输出",
        verdict: {
          verdict: "needs_human",
          comments: "完整 comments",
          summary: "仅有可选建议",
          findings: [],
          review_gate: "user_confirm_allowed",
        },
      }),
    );
    useWorkspaceStore.setState({
      sessionId: "workspace_session_0001",
      workspaceType: "design",
      stage: "human_confirm",
      selectedNodeId: "timeline_node_017",
      activeNodeId: "timeline_node_017",
      timelineNodes: [
        timelineNode({
          node_id: "timeline_node_017",
          node_type: "reviewer_run",
          title: "Review Round 1",
          status: "completed",
        }),
      ],
      nodeDetails: {
        timeline_node_017: makeNodeDetail({
          node_id: "timeline_node_017",
          node_type: "reviewer_run",
          streaming_content: "摘要",
        }),
      },
    });

    render(<ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />);

    await waitFor(() => {
      expect(fetchWorkspaceNodeDetail).toHaveBeenCalledWith(
        "workspace_session_0001",
        "timeline_node_017",
      );
    });
    expect(await screen.findByText("完整 review 输出")).toBeInTheDocument();
  });

  it("starts generation with provider config from the chat input", async () => {
    const api = mockWorkspaceWs();
    useWorkspaceStore.setState({
      sessionId: "workspace_session_0001",
      stage: "prepare_context",
      providers: { author: "claude_code", reviewer: "codex" },
      reviewerEnabled: true,
      reviewRounds: 1,
    });

    render(<ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />);
    useWorkspaceStore.setState({
      providers: { author: "fake", reviewer: "codex" },
      reviewerEnabled: true,
      reviewRounds: 2,
    });

    await userEvent.click(screen.getByTestId("start-generation"));

    expect(api.sendStartGeneration).toHaveBeenCalledWith(
      { author: "fake", reviewer: "codex", review_rounds: 2 },
      true,
    );
  });

  it("exposes focused selectors for the workspace header and chat panel", () => {
    useWorkspaceStore.setState({
      sessionId: "workspace_session_0001",
      workspaceType: "design",
      stage: "review_decision",
      providers: { author: "fake", reviewer: "codex" },
      reviewRounds: 3,
      providerLocked: true,
      providerLockedAt: "2026-06-06T00:00:00Z",
      superpowersEnabled: true,
      openSpecEnabled: true,
      selectedNodeId: "node-1",
      chatEntries: [chatEntry({ id: "entry-1", node_id: "node-1" })],
    });

    const state = useWorkspaceStore.getState();

    expect(selectWorkspaceHeaderState(state)).toEqual({
      sessionId: "workspace_session_0001",
      workspaceType: "design",
      providers: { author: "fake", reviewer: "codex" },
      reviewRounds: 3,
      stage: "review_decision",
      providerLocked: true,
      providerLockedAt: "2026-06-06T00:00:00Z",
      superpowersEnabled: true,
      openSpecEnabled: true,
    });
    expect(selectChatPanelState(state)).toEqual({
      chatEntries: [chatEntry({ id: "entry-1", node_id: "node-1" })],
      stage: "review_decision",
      selectedNodeId: "node-1",
    });
  });

  it("sends author confirmation decisions from the chat input", async () => {
    const api = mockWorkspaceWs();
    useWorkspaceStore.setState({
      sessionId: "workspace_session_0001",
      stage: "author_confirm",
      providers: { author: "fake", reviewer: "codex" },
      artifact: "# Story Spec",
    });

    render(<ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />);

    await userEvent.click(screen.getByRole("button", { name: "进入 Review" }));
    await userEvent.click(screen.getByRole("button", { name: "重新编写" }));

    expect(api.sendAuthorDecision).toHaveBeenNthCalledWith(1, "accept");
    expect(api.sendAuthorDecision).toHaveBeenNthCalledWith(2, "reject");
  });

  it.each(["story", "design", "work_item"])(
    "shows review decision actions when restored %s chat lacks a review verdict entry",
    async (workspaceType) => {
      const api = mockWorkspaceWs();
      useWorkspaceStore.setState({
        sessionId: "workspace_session_0001",
        workspaceType,
        stage: "review_decision",
        providers: { author: "claude_code", reviewer: "codex" },
        timelineNodes: [
          timelineNode({
            node_id: "timeline_node_017",
            node_type: "review_decision",
            stage: "review_decision",
            status: "paused",
            title: "Review Decision Round 4",
            summary: "需要继续返修",
          }),
        ],
        activeNodeId: "timeline_node_017",
        selectedNodeId: "timeline_node_017",
        chatEntries: [
          chatEntry({
            id: "timeline_node_017:timeline-anchor",
            type: "stage_change",
            role: "system",
            content: "Review Decision Round 4 · 需要继续返修",
            node_id: "timeline_node_017",
          }),
        ],
      });

      render(<ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />);

      expect(screen.getByRole("button", { name: "接受修订建议" })).toBeInTheDocument();
      expect(screen.getByRole("button", { name: "补充上下文后修订" })).toBeInTheDocument();
      expect(screen.getByRole("button", { name: "跳过，人工处理" })).toBeInTheDocument();

      await userEvent.click(screen.getByRole("button", { name: "补充上下文后修订" }));
      await userEvent.type(screen.getByLabelText("补充返修上下文"), "补充 provider gate 细节");
      await userEvent.click(screen.getByRole("button", { name: "提交补充并修订" }));

      expect(api.sendSelectRevisionPath).toHaveBeenCalledWith(
        "revise-with-context",
        "补充 provider gate 细节",
      );
    },
  );

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

  it.each([
    ["story", "Story Spec 生成"],
    ["design", "Design Spec 生成"],
    ["work_item", "Work Item 生成"],
  ])(
    "scrolls timeline provider nodes to their rendered stream group for %s workspaces",
    async (workspaceType, title) => {
      mockWorkspaceWs();
      const scrolledEntryIds: Array<string | undefined> = [];
      Object.defineProperty(HTMLElement.prototype, "scrollIntoView", {
        configurable: true,
        value: function scrollIntoView() {
          scrolledEntryIds.push((this as HTMLElement).dataset.entryId);
        },
      });
      useWorkspaceStore.setState({
        sessionId: "workspace_session_0001",
        workspaceType,
        timelineNodes: [
          timelineNode({ node_id: "node-1", node_type: "context_note", title: "补充上下文" }),
          timelineNode({ node_id: "node-2", node_type: "author_run", title }),
        ],
        activeNodeId: "node-2",
        selectedNodeId: "node-1",
        chatEntries: [
          chatEntry({
            id: "entry-context",
            node_id: "node-1",
            type: "context_note",
            role: "user",
          }),
          chatEntry({
            id: "entry-prompt",
            node_id: "node-2",
            type: "execution_event",
            role: "author",
            content: "Provider Prompt",
          }),
          chatEntry({
            id: "entry-stream",
            node_id: "node-2",
            type: "provider_stream",
            role: "author",
            content: "生成内容",
          }),
        ],
      });

      render(<ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />);
      scrolledEntryIds.length = 0;

      await userEvent.click(screen.getByTestId("timeline-node-author_run"));

      expect(scrolledEntryIds).toContain("entry-stream");
    },
  );

  it.each(["story", "design", "work_item"])(
    "scrolls author confirm timeline nodes to their rendered anchor for %s workspaces",
    async (workspaceType) => {
      mockWorkspaceWs();
      const scrolledEntryIds: Array<string | undefined> = [];
      Object.defineProperty(HTMLElement.prototype, "scrollIntoView", {
        configurable: true,
        value: function scrollIntoView() {
          scrolledEntryIds.push((this as HTMLElement).dataset.entryId);
        },
      });
      useWorkspaceStore.setState({
        sessionId: "workspace_session_0001",
        workspaceType,
        timelineNodes: [
          timelineNode({ node_id: "node-1", node_type: "revision", title: "Author 返修 Round 2" }),
          timelineNode({
            node_id: "node-2",
            node_type: "author_confirm",
            title: "Author 结果确认",
            summary: "已进入 Review",
          }),
        ],
        activeNodeId: "node-2",
        selectedNodeId: "node-1",
        chatEntries: [
          chatEntry({
            id: "entry-revision",
            node_id: "node-1",
            type: "provider_stream",
            role: "author",
            content: "返修内容",
          }),
          chatEntry({
            id: "entry-author-confirm",
            node_id: "node-2",
            type: "stage_change",
            role: "system",
            content: "Author 结果确认 · 已进入 Review",
          }),
        ],
      });

      render(<ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />);
      scrolledEntryIds.length = 0;

      await userEvent.click(screen.getByTestId("timeline-node-author_confirm"));

      expect(scrolledEntryIds).toContain("entry-author-confirm");
    },
  );

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

  it("allows confirming the current version from human confirm after optional review findings", async () => {
    const api = mockWorkspaceWs();
    useWorkspaceStore.setState({
      sessionId: "workspace_session_0001",
      workspaceType: "design",
      stage: "human_confirm",
      providers: { author: "claude_code", reviewer: "codex" },
      timelineNodes: [
        timelineNode({
          node_id: "timeline_node_human",
          node_type: "human_confirm",
          stage: "human_confirm",
          status: "paused",
          title: "人工确认",
          summary: "仅有可选建议",
        }),
      ],
      chatEntries: [
        chatEntry({
          type: "review_verdict",
          role: "reviewer",
          content: "仅有可选建议",
          metadata: {
            verdict: "needs_human",
            summary: "仅有可选建议",
            review_gate: "user_confirm_allowed",
            findings: [
              {
                severity: "suggestion",
                message: "建议补充说明",
                evidence: "当前版本可用",
                impact: "不影响下一阶段",
                required_action: "可后续优化",
              },
            ],
          },
        }),
        chatEntry({
          id: "timeline_node_human:gate-prompt",
          type: "gate_prompt",
          role: "system",
          content: "等待人工确认",
          node_id: "timeline_node_human",
          metadata: {
            verdict: "needs_human",
            summary: "仅有可选建议",
            review_gate: "user_confirm_allowed",
          },
        }),
      ],
    });

    render(<ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />);

    await userEvent.click(screen.getByRole("button", { name: "确认使用当前版本" }));

    expect(api.sendHumanConfirm).toHaveBeenCalledWith("confirm");
  });

  it("sends request-change payload when adopting optional review findings", async () => {
    const api = mockWorkspaceWs();
    useWorkspaceStore.setState({
      sessionId: "workspace_session_0001",
      workspaceType: "work_item",
      stage: "human_confirm",
      providers: { author: "claude_code", reviewer: "codex" },
      timelineNodes: [
        timelineNode({
          node_id: "timeline_node_human",
          node_type: "human_confirm",
          stage: "human_confirm",
          status: "paused",
          title: "人工确认",
          summary: "仅有可选建议",
        }),
      ],
      chatEntries: [
        chatEntry({
          type: "review_verdict",
          role: "reviewer",
          content: "仅有可选建议",
          metadata: {
            verdict: "needs_human",
            comments: "当前版本可用，但建议补充说明。",
            summary: "仅有可选建议",
            review_gate: "user_confirm_allowed",
            findings: [
              {
                severity: "optional",
                message: "建议补充说明",
                evidence: "当前版本可用",
                impact: "不影响下一阶段",
                required_action: "补充说明段落",
              },
            ],
          },
        }),
        chatEntry({
          id: "timeline_node_human:gate-prompt",
          type: "gate_prompt",
          role: "system",
          content: "等待人工确认",
          node_id: "timeline_node_human",
          metadata: {
            verdict: "needs_human",
            comments: "当前版本可用，但建议补充说明。",
            summary: "仅有可选建议",
            review_gate: "user_confirm_allowed",
            findings: [
              {
                severity: "optional",
                message: "建议补充说明",
                evidence: "当前版本可用",
                impact: "不影响下一阶段",
                required_action: "补充说明段落",
              },
            ],
          },
        }),
      ],
    });

    render(<ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />);

    await userEvent.click(screen.getByRole("button", { name: "采纳建议并返修" }));

    expect(api.sendHumanConfirm).toHaveBeenCalledWith(
      "request-change",
      expect.objectContaining({
        description: expect.stringContaining("建议补充说明"),
      }),
    );
    const payload = vi.mocked(api.sendHumanConfirm).mock.calls[0][1] as { description: string };
    expect(payload.description).toContain("补充说明段落");
  });

  it("renders work item plan candidate panel for work_item_plan workspaces", async () => {
    const api = mockWorkspaceWs();
    useWorkspaceStore.setState({
      sessionId: "workspace_session_0001",
      workspaceType: "work_item_plan",
      stage: "author_confirm",
      providers: { author: "claude_code", reviewer: "codex" },
      workItemPlanCandidate: workItemPlanCandidate(),
    });

    render(<ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />);
    await userEvent.click(screen.getByRole("button", { name: "Artifact" }));

    expect(screen.getByTestId("work-item-plan-candidate-panel")).toBeInTheDocument();
    expect(screen.getByText("Work Item Plan 候选")).toBeInTheDocument();

    await userEvent.click(screen.getByTestId("accept-plan-button"));
    expect(api.sendAuthorDecision).toHaveBeenCalledWith("accept");
  });

  it("shows empty state when work_item_plan candidate is missing", async () => {
    mockWorkspaceWs();
    useWorkspaceStore.setState({
      sessionId: "workspace_session_0001",
      workspaceType: "work_item_plan",
      stage: "author_confirm",
      providers: { author: "claude_code", reviewer: "codex" },
      workItemPlanCandidate: null,
    });

    render(<ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />);
    await userEvent.click(screen.getByRole("button", { name: "Artifact" }));

    expect(screen.queryByTestId("work-item-plan-candidate-panel")).not.toBeInTheDocument();
    expect(screen.getByText("尚未生成候选，请点击开始生成")).toBeInTheDocument();
  });

  it("keeps markdown artifact pane for story workspaces", async () => {
    mockWorkspaceWs();
    useWorkspaceStore.setState({
      sessionId: "workspace_session_0001",
      workspaceType: "story",
      stage: "author_confirm",
      providers: { author: "claude_code", reviewer: "codex" },
      artifact: "# Story",
      artifactVersions: [
        {
          version: 1,
          markdown: "# Story",
          generated_by: "claude_code",
          created_at: "2026-06-17T00:00:00Z",
          source_node_id: "node-1",
        },
      ],
    });

    render(<ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />);
    await userEvent.click(screen.getByRole("button", { name: "Artifact" }));

    expect(screen.queryByTestId("work-item-plan-candidate-panel")).not.toBeInTheDocument();
    expect(screen.getByTestId("monaco-viewer")).toHaveTextContent("# Story");
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

function makeNodeDetail(overrides: Partial<NodeDetail> = {}): NodeDetail {
  return {
    node_id: "timeline_node_001",
    session_id: "workspace_session_0001",
    node_type: "author_run",
    status: "completed",
    agent_role: "author",
    provider: { name: "claude_code", model: "claude-opus-4" },
    messages: [],
    streaming_content: "",
    execution_events: [],
    permission_events: [],
    verdict: null,
    artifact_ref: null,
    is_revision: false,
    base_artifact_ref: null,
    started_at: "2026-05-20T14:30:00Z",
    ended_at: null,
    ...overrides,
  };
}

function workItemPlanCandidate(): import("../api/types").WorkItemPlanCandidateDto {
  return {
    plan: {
      plan_id: "plan_001",
      project_id: "project_001",
      issue_id: "issue_001",
      title: "Plan 001",
      source_story_spec_ids: [],
      source_design_spec_ids: [],
      options: {
        include_integration_tests: false,
        include_e2e_tests: false,
        force_frontend_backend_split: false,
        require_execution_plan_confirm: false,
      },
      status: "draft",
      work_item_ids: [],
      repository_profile_ref: null,
      verification_plan_ids: [],
      dependency_graph: [],
      created_from_provider_run: null,
      validator_findings: [],
      review_summary: null,
      created_at: "2026-06-17T00:00:00Z",
      updated_at: "2026-06-17T00:00:00Z",
    },
    work_items: [
      {
        candidate_id: "wi_001",
        title: "Frontend Auth",
        kind: "frontend",
        exclusive_write_scopes: ["src/auth"],
        depends_on: [],
        verification_plan_ref: null,
        meta: { summary: "前端登录" },
      },
    ],
    verification_plans: [],
    repository_profile: null,
    validator_findings: [],
  };
}
