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
    sendSelectWorkItemGenerationMode: vi.fn(),
    sendRequestOutlineRevision: vi.fn(),
    sendWorkItemDraftDecision: vi.fn(),
    sendWorkItemBatchDecision: vi.fn(),
    sendWorkItemPlanCompileRecoveryAction: vi.fn(),
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

  it("renders work item plan generation progress as a provider stream bubble", () => {
    mockWorkspaceWs();
    useWorkspaceStore.setState({
      sessionId: "workspace_session_0001",
      workspaceType: "work_item_plan",
      stage: "running",
      providers: { author: "claude_code", reviewer: "codex" },
      timelineNodes: [
        timelineNode({
          node_id: "timeline_node_work_item_plan_author",
          node_type: "author_run",
          agent: "claude_code",
          stage: "running",
          status: "active",
          title: "Work Item Plan 生成",
        }),
      ],
      activeNodeId: "timeline_node_work_item_plan_author",
      selectedNodeId: "timeline_node_work_item_plan_author",
      chatEntries: [
        chatEntry({
          id: "timeline_node_work_item_plan_author:stream",
          type: "provider_stream",
          role: "author",
          content: "正在生成 Work Item Plan",
          node_id: "timeline_node_work_item_plan_author",
          metadata: { provider: "claude_code" },
        }),
      ],
    });

    render(<ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />);

    expect(screen.getByTestId("chat-entry-list")).toHaveTextContent("正在生成 Work Item Plan");
    expect(screen.getAllByText("Work Item Plan #workspace_session_0001").length).toBeGreaterThan(0);
  });

  it("work_item_plan candidate panel supports revert, request revision and accept", async () => {
    const api = mockWorkspaceWs();
    useWorkspaceStore.setState({
      sessionId: "workspace_session_0001",
      workspaceType: "work_item_plan",
      stage: "author_confirm",
      providers: { author: "claude_code", reviewer: "codex" },
      workItemPlanCandidate: workItemPlanCandidate({
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
          {
            candidate_id: "wi_002",
            title: "Backend API",
            kind: "backend",
            exclusive_write_scopes: ["src/api"],
            depends_on: ["wi_001"],
            verification_plan_ref: null,
            meta: { summary: "后端接口" },
          },
        ],
      }),
    });

    render(<ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />);
    await userEvent.click(screen.getByRole("button", { name: "Artifact" }));

    await userEvent.click(screen.getByTestId("start-revert-wi_001"));
    await userEvent.type(screen.getByTestId("revert-feedback-input-wi_001"), "拆得太粗");
    await userEvent.click(screen.getByTestId("submit-revert-wi_001"));
    expect(api.sendRevertWorkItem).toHaveBeenCalledWith("wi_001", "拆得太粗", false);

    useWorkspaceStore.getState().setWorkItemPlanCandidate(
      workItemPlanCandidate({
        work_items: [
          {
            candidate_id: "wi_001",
            title: "Frontend Auth",
            kind: "frontend",
            exclusive_write_scopes: ["src/auth"],
            depends_on: [],
            verification_plan_ref: null,
            meta: { summary: "前端登录" },
            reverted: true,
            revert_feedback: "拆得太粗",
          },
          {
            candidate_id: "wi_002",
            title: "Backend API",
            kind: "backend",
            exclusive_write_scopes: ["src/api"],
            depends_on: ["wi_001"],
            verification_plan_ref: null,
            meta: { summary: "后端接口" },
          },
        ],
      }),
    );

    await waitFor(() => expect(screen.getByText(/已标记撤销/)).toBeInTheDocument());
    const requestRevisionButton = screen.getByTestId("request-revision-button");
    await waitFor(() => expect(requestRevisionButton).not.toBeDisabled());
    expect(requestRevisionButton).toHaveTextContent("重新生成被标记的 1 项");
    await userEvent.click(requestRevisionButton);
    expect(api.sendRequestRevision).toHaveBeenCalled();

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

  it("generation mode node shows serial batch revision buttons", async () => {
    const api = mockWorkspaceWs();
    useWorkspaceStore.setState({
      sessionId: "workspace_session_0001",
      workspaceType: "work_item_plan",
      stage: "author_confirm",
      providers: { author: "claude_code", reviewer: "codex" },
      activeNodeId: "node_mode",
      selectedNodeId: "node_mode",
      timelineNodes: [
        timelineNode({
          node_id: "node_mode",
          node_type: "work_item_generation_mode",
          stage: "author_confirm",
          title: "选择生成模式",
        }),
      ],
      workItemPlanArtifact: { type: "outline_candidate", payload: workItemPlanOutlinePayload() },
    });

    render(<ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />);
    await userEvent.click(screen.getByRole("button", { name: "Artifact" }));

    await userEvent.click(screen.getByRole("button", { name: "逐个生成" }));
    await userEvent.click(screen.getByRole("button", { name: "自动生成" }));
    await userEvent.click(screen.getByRole("button", { name: "返回 Outline 返修" }));

    expect(api.sendSelectWorkItemGenerationMode).toHaveBeenNthCalledWith(1, "serial");
    expect(api.sendSelectWorkItemGenerationMode).toHaveBeenNthCalledWith(2, "batch");
    expect(api.sendRequestOutlineRevision).toHaveBeenCalledWith();
    expect(screen.getByTestId("work-item-plan-artifact-panel")).toHaveTextContent(
      "Split frontend and backend work.",
    );
  });

  it("generation mode node shows mode actions in chat controls instead of review actions", async () => {
    const api = mockWorkspaceWs();
    useWorkspaceStore.setState({
      sessionId: "workspace_session_0001",
      workspaceType: "work_item_plan",
      stage: "author_confirm",
      providers: { author: "claude_code", reviewer: "codex" },
      activeNodeId: "node_mode",
      selectedNodeId: "node_mode",
      timelineNodes: [
        timelineNode({
          node_id: "node_mode",
          node_type: "work_item_generation_mode",
          stage: "author_confirm",
          title: "选择生成模式",
        }),
      ],
      workItemPlanArtifact: { type: "outline_candidate", payload: workItemPlanOutlinePayload() },
    });

    render(<ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />);

    expect(screen.getByRole("textbox")).toHaveAttribute(
      "placeholder",
      "请选择 Work Item 生成模式",
    );
    expect(screen.queryByRole("button", { name: "进入 Review" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "重新编写" })).not.toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: "逐个生成" }));
    await userEvent.click(screen.getByRole("button", { name: "自动生成" }));
    await userEvent.click(screen.getByRole("button", { name: "返回 Outline 返修" }));

    expect(api.sendSelectWorkItemGenerationMode).toHaveBeenNthCalledWith(1, "serial");
    expect(api.sendSelectWorkItemGenerationMode).toHaveBeenNthCalledWith(2, "batch");
    expect(api.sendRequestOutlineRevision).toHaveBeenCalledWith();
    expect(api.sendAuthorDecision).not.toHaveBeenCalled();
  });

  it("outline confirm node shows accept and rewrite actions", async () => {
    const api = mockWorkspaceWs();
    useWorkspaceStore.setState({
      sessionId: "workspace_session_0001",
      workspaceType: "work_item_plan",
      stage: "author_confirm",
      providers: { author: "claude_code", reviewer: "codex" },
      activeNodeId: "node_outline",
      selectedNodeId: "node_outline",
      timelineNodes: [
        timelineNode({
          node_id: "node_outline",
          node_type: "work_item_plan_outline_confirm",
          stage: "author_confirm",
          title: "确认 Outline",
        }),
      ],
      workItemPlanArtifact: { type: "outline_candidate", payload: workItemPlanOutlinePayload() },
    });

    render(<ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />);
    await userEvent.click(screen.getByRole("button", { name: "Artifact" }));

    await userEvent.click(screen.getByRole("button", { name: "接受 Outline" }));
    await userEvent.click(screen.getByRole("button", { name: "重写 Outline" }));

    expect(api.sendAuthorDecision).toHaveBeenCalledWith("accept");
    expect(api.sendRequestOutlineRevision).toHaveBeenCalledWith();
  });

  it("renders outline then mode then serial draft confirm", async () => {
    const api = mockWorkspaceWs();
    useWorkspaceStore.setState({
      sessionId: "workspace_session_0001",
      workspaceType: "work_item_plan",
      stage: "author_confirm",
      providers: { author: "claude_code", reviewer: "codex" },
      activeNodeId: "node_outline",
      selectedNodeId: "node_outline",
      timelineNodes: [
        timelineNode({
          node_id: "node_outline",
          node_type: "work_item_plan_outline_confirm",
          stage: "author_confirm",
          title: "确认 Outline",
        }),
      ],
      workItemPlanArtifact: { type: "outline_candidate", payload: workItemPlanOutlinePayload() },
    });

    render(<ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />);
    await userEvent.click(screen.getByRole("button", { name: "Artifact" }));

    expect(screen.getByTestId("work-item-plan-artifact-panel")).toHaveTextContent(
      "Split frontend and backend work.",
    );
    await userEvent.click(screen.getByRole("button", { name: "接受 Outline" }));
    expect(api.sendAuthorDecision).toHaveBeenCalledWith("accept");

    useWorkspaceStore.setState({
      activeNodeId: "node_mode",
      selectedNodeId: "node_mode",
      timelineNodes: [
        timelineNode({
          node_id: "node_mode",
          node_type: "work_item_generation_mode",
          stage: "author_confirm",
          title: "选择生成模式",
        }),
      ],
      workItemPlanArtifact: { type: "outline_candidate", payload: workItemPlanOutlinePayload() },
    });
    await waitFor(() => expect(screen.getByRole("button", { name: "逐个生成" })).toBeInTheDocument());
    await userEvent.click(screen.getByRole("button", { name: "逐个生成" }));
    expect(api.sendSelectWorkItemGenerationMode).toHaveBeenCalledWith("serial");

    useWorkspaceStore.setState({
      activeNodeId: "node_draft",
      selectedNodeId: "node_draft",
      timelineNodes: [
        timelineNode({
          node_id: "node_draft",
          node_type: "work_item_draft_confirm",
          stage: "author_confirm",
          title: "确认 Draft",
        }),
      ],
      workItemPlanArtifact: { type: "draft_candidate", payload: workItemDraftPayload() },
    });
    await waitFor(() => expect(screen.getByRole("button", { name: "接受" })).toBeInTheDocument());
    expect(screen.getByTestId("work-item-plan-artifact-panel")).toHaveTextContent("Backend flow");
    await userEvent.click(screen.getByRole("button", { name: "接受" }));
    expect(api.sendWorkItemDraftDecision).toHaveBeenCalledWith("outline_backend", "accept");
  });

  it("draft confirm hides accept when validation failed", async () => {
    const api = mockWorkspaceWs();
    useWorkspaceStore.setState({
      sessionId: "workspace_session_0001",
      workspaceType: "work_item_plan",
      stage: "author_confirm",
      providers: { author: "claude_code", reviewer: "codex" },
      activeNodeId: "node_draft",
      selectedNodeId: "node_draft",
      timelineNodes: [
        timelineNode({
          node_id: "node_draft",
          node_type: "work_item_draft_confirm",
          stage: "author_confirm",
          title: "确认 Draft",
        }),
      ],
      workItemPlanArtifact: {
        type: "draft_candidate",
        payload: { ...workItemDraftPayload(), can_accept: false },
      },
    });

    render(<ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />);
    await userEvent.click(screen.getByRole("button", { name: "Artifact" }));

    expect(screen.queryByRole("button", { name: "接受" })).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "重写" }));
    await userEvent.click(screen.getByRole("button", { name: "暂停" }));

    expect(api.sendWorkItemDraftDecision).toHaveBeenNthCalledWith(
      1,
      "outline_backend",
      "rewrite",
    );
    expect(api.sendWorkItemDraftDecision).toHaveBeenNthCalledWith(
      2,
      "outline_backend",
      "pause",
    );
  });

  it("draft confirm chat controls send work item draft decisions instead of author review decisions", async () => {
    const api = mockWorkspaceWs();
    useWorkspaceStore.setState({
      sessionId: "workspace_session_0001",
      workspaceType: "work_item_plan",
      stage: "author_confirm",
      providers: { author: "claude_code", reviewer: "codex" },
      activeNodeId: "node_draft",
      selectedNodeId: "node_draft",
      timelineNodes: [
        timelineNode({
          node_id: "node_draft",
          node_type: "work_item_draft_confirm",
          stage: "author_confirm",
          title: "确认 Draft",
        }),
      ],
      workItemPlanArtifact: { type: "draft_candidate", payload: workItemDraftPayload() },
    });

    render(<ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />);

    expect(screen.getByRole("textbox")).toHaveAttribute(
      "placeholder",
      "请确认当前 Work Item Draft",
    );
    expect(screen.queryByRole("button", { name: "进入 Review" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "重新编写" })).not.toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: "接受" }));
    await userEvent.click(screen.getByRole("button", { name: "重写" }));
    await userEvent.click(screen.getByRole("button", { name: "暂停" }));

    expect(api.sendWorkItemDraftDecision).toHaveBeenNthCalledWith(
      1,
      "outline_backend",
      "accept",
    );
    expect(api.sendWorkItemDraftDecision).toHaveBeenNthCalledWith(
      2,
      "outline_backend",
      "rewrite",
    );
    expect(api.sendWorkItemDraftDecision).toHaveBeenNthCalledWith(
      3,
      "outline_backend",
      "pause",
    );
    expect(api.sendAuthorDecision).not.toHaveBeenCalled();
  });

  it("invalid draft chat controls hide accept and do not expose review actions", async () => {
    const api = mockWorkspaceWs();
    useWorkspaceStore.setState({
      sessionId: "workspace_session_0001",
      workspaceType: "work_item_plan",
      stage: "author_confirm",
      providers: { author: "claude_code", reviewer: "codex" },
      activeNodeId: "node_draft",
      selectedNodeId: "node_draft",
      timelineNodes: [
        timelineNode({
          node_id: "node_draft",
          node_type: "work_item_draft_confirm",
          stage: "author_confirm",
          title: "确认 Draft",
        }),
      ],
      workItemPlanArtifact: {
        type: "draft_candidate",
        payload: { ...workItemDraftPayload(), can_accept: false },
      },
    });

    render(<ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />);

    expect(screen.queryByRole("button", { name: "接受" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "进入 Review" })).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "重写" }));
    await userEvent.click(screen.getByRole("button", { name: "暂停" }));

    expect(api.sendWorkItemDraftDecision).toHaveBeenNthCalledWith(
      1,
      "outline_backend",
      "rewrite",
    );
    expect(api.sendWorkItemDraftDecision).toHaveBeenNthCalledWith(
      2,
      "outline_backend",
      "pause",
    );
    expect(api.sendAuthorDecision).not.toHaveBeenCalled();
  });

  it("renders batch queue and review findings", async () => {
    const api = mockWorkspaceWs();
    useWorkspaceStore.setState({
      sessionId: "workspace_session_0001",
      workspaceType: "work_item_plan",
      stage: "author_confirm",
      providers: { author: "claude_code", reviewer: "codex" },
      activeNodeId: "node_batch",
      selectedNodeId: "node_batch",
      timelineNodes: [
        timelineNode({
          node_id: "node_batch",
          node_type: "work_item_batch_confirm",
          stage: "author_confirm",
          title: "确认 Batch",
        }),
      ],
      workItemPlanArtifact: {
        type: "batch_state",
        payload: {
          ...workItemBatchPayload(true),
          queue: ["outline_backend", "outline_frontend"],
        },
      },
    });

    render(<ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />);
    await userEvent.click(screen.getByRole("button", { name: "Artifact" }));

    expect(screen.getByTestId("work-item-plan-artifact-panel")).toHaveTextContent(
      "outline_backend -> outline_frontend",
    );
    expect(screen.getByTestId("work-item-plan-artifact-panel")).toHaveTextContent(
      "validation_failed",
    );
    await userEvent.click(screen.getByRole("button", { name: "降级串行" }));
    expect(api.sendWorkItemBatchDecision).toHaveBeenCalledWith(
      "downgrade_to_serial",
      undefined,
      "outline_backend",
    );
  });

  it("batch confirm shows accept all rewrite pause and downgrade actions", async () => {
    const api = mockWorkspaceWs();
    useWorkspaceStore.setState({
      sessionId: "workspace_session_0001",
      workspaceType: "work_item_plan",
      stage: "author_confirm",
      providers: { author: "claude_code", reviewer: "codex" },
      activeNodeId: "node_batch",
      selectedNodeId: "node_batch",
      timelineNodes: [
        timelineNode({
          node_id: "node_batch",
          node_type: "work_item_batch_confirm",
          stage: "author_confirm",
          title: "确认 Batch",
        }),
      ],
      workItemPlanArtifact: { type: "batch_state", payload: workItemBatchPayload(true) },
    });

    render(<ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />);
    await userEvent.click(screen.getByRole("button", { name: "Artifact" }));

    await userEvent.click(screen.getByRole("button", { name: "接受全部" }));
    await userEvent.click(screen.getByRole("button", { name: "整组重写" }));
    await userEvent.click(screen.getByRole("button", { name: "暂停" }));
    await userEvent.click(screen.getByRole("button", { name: "降级串行" }));

    expect(api.sendWorkItemBatchDecision).toHaveBeenNthCalledWith(1, "accept_all");
    expect(api.sendWorkItemBatchDecision).toHaveBeenNthCalledWith(2, "rewrite_batch");
    expect(api.sendWorkItemBatchDecision).toHaveBeenNthCalledWith(3, "pause");
    expect(api.sendWorkItemBatchDecision).toHaveBeenNthCalledWith(
      4,
      "downgrade_to_serial",
      undefined,
      "outline_backend",
    );
  });

  it("compile recovery hides abort rollback after committed marker", async () => {
    const api = mockWorkspaceWs();
    useWorkspaceStore.setState({
      sessionId: "workspace_session_0001",
      workspaceType: "work_item_plan",
      stage: "human_confirm",
      providers: { author: "claude_code", reviewer: "codex" },
      activeNodeId: "node_recovery",
      selectedNodeId: "node_recovery",
      timelineNodes: [
        timelineNode({
          node_id: "node_recovery",
          node_type: "work_item_plan_compile_recovery",
          stage: "human_confirm",
          title: "Compile Recovery",
        }),
      ],
      workItemPlanArtifact: {
        type: "compile_report",
        payload: workItemCompileReportPayload("committed"),
      },
    });

    render(<ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />);
    await userEvent.click(screen.getByRole("button", { name: "Artifact" }));

    expect(screen.queryByRole("button", { name: "放弃并回滚" })).not.toBeInTheDocument();
    expect(screen.getByTestId("work-item-plan-artifact-panel")).toHaveTextContent("Before");
    expect(screen.getByTestId("work-item-plan-artifact-panel")).toHaveTextContent("After");
    await userEvent.click(screen.getByRole("button", { name: "继续" }));
    await userEvent.click(screen.getByRole("button", { name: "转人工" }));

    expect(api.sendWorkItemPlanCompileRecoveryAction).toHaveBeenNthCalledWith(1, "continue");
    expect(api.sendWorkItemPlanCompileRecoveryAction).toHaveBeenNthCalledWith(2, "human_triage");
  });

  it("timeline selection shows historical draft artifact as readonly", async () => {
    mockWorkspaceWs();
    const oldDraft = workItemDraftPayload("Backend flow v1");
    const currentDraft = workItemDraftPayload("Backend flow v2");
    useWorkspaceStore.setState({
      sessionId: "workspace_session_0001",
      workspaceType: "work_item_plan",
      stage: "author_confirm",
      providers: { author: "claude_code", reviewer: "codex" },
      activeNodeId: "node_current_draft",
      selectedNodeId: "node_old_draft",
      timelineNodes: [
        timelineNode({
          node_id: "node_old_draft",
          node_type: "work_item_draft_confirm",
          stage: "author_confirm",
          title: "历史 Draft",
          status: "completed",
        }),
        timelineNode({
          node_id: "node_current_draft",
          node_type: "work_item_draft_confirm",
          stage: "author_confirm",
          title: "当前 Draft",
          status: "active",
        }),
      ],
      workItemPlanArtifact: { type: "draft_candidate", payload: currentDraft },
      workItemPlanArtifactVersions: [
        {
          version: 1,
          generated_by: "claude_code",
          reviewed_by: null,
          review_verdict: null,
          confirmed_by: null,
          is_current: false,
          created_at: "2026-06-23T00:00:00Z",
          source_node_id: "node_old_draft",
          artifact: { type: "draft_candidate", payload: oldDraft },
        },
        {
          version: 2,
          generated_by: "claude_code",
          reviewed_by: null,
          review_verdict: null,
          confirmed_by: null,
          is_current: true,
          created_at: "2026-06-23T00:01:00Z",
          source_node_id: "node_current_draft",
          artifact: { type: "draft_candidate", payload: currentDraft },
        },
      ],
    });

    render(<ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />);
    await userEvent.click(screen.getByRole("button", { name: "Artifact" }));

    expect(screen.getByTestId("work-item-plan-artifact-panel")).toHaveTextContent(
      "Backend flow v1",
    );
    expect(screen.getByTestId("work-item-plan-artifact-panel")).toHaveTextContent("只读历史");
  });

  it("lists all work item plan artifact versions and switches between draft history", async () => {
    mockWorkspaceWs();
    const oldDraft = workItemDraftPayload("Backend flow v1");
    const currentDraft = workItemDraftPayload("Backend flow v2");
    useWorkspaceStore.setState({
      sessionId: "workspace_session_0001",
      workspaceType: "work_item_plan",
      stage: "author_confirm",
      providers: { author: "claude_code", reviewer: "codex" },
      activeNodeId: "node_current_draft",
      selectedNodeId: "node_current_draft",
      timelineNodes: [
        timelineNode({
          node_id: "node_outline",
          node_type: "work_item_plan_outline_confirm",
          stage: "author_confirm",
          title: "Plan Outline",
          status: "completed",
        }),
        timelineNode({
          node_id: "node_old_draft",
          node_type: "work_item_draft_confirm",
          stage: "author_confirm",
          title: "历史 Draft",
          status: "completed",
        }),
        timelineNode({
          node_id: "node_current_draft",
          node_type: "work_item_draft_confirm",
          stage: "author_confirm",
          title: "当前 Draft",
          status: "active",
        }),
      ],
      workItemPlanArtifact: { type: "draft_candidate", payload: currentDraft },
      workItemPlanArtifactVersions: [
        {
          version: 1,
          generated_by: "claude_code",
          reviewed_by: null,
          review_verdict: null,
          confirmed_by: null,
          is_current: false,
          created_at: "2026-06-23T00:00:00Z",
          source_node_id: "node_outline",
          artifact: { type: "outline_candidate", payload: workItemPlanOutlinePayload() },
        },
        {
          version: 2,
          generated_by: "claude_code",
          reviewed_by: null,
          review_verdict: null,
          confirmed_by: null,
          is_current: false,
          created_at: "2026-06-23T00:01:00Z",
          source_node_id: "node_old_draft",
          artifact: { type: "draft_candidate", payload: oldDraft },
        },
        {
          version: 3,
          generated_by: "claude_code",
          reviewed_by: null,
          review_verdict: null,
          confirmed_by: null,
          is_current: true,
          created_at: "2026-06-23T00:02:00Z",
          source_node_id: "node_current_draft",
          artifact: { type: "draft_candidate", payload: currentDraft },
        },
      ],
    });

    render(<ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />);
    await userEvent.click(screen.getByRole("button", { name: "Artifact" }));

    const versionList = screen.getByTestId("work-item-plan-artifact-version-list");
    expect(versionList).toHaveTextContent("Plan Outline");
    expect(versionList).toHaveTextContent("outline_backend / draft_backend_001");
    expect(versionList).toHaveTextContent("v3");

    await userEvent.click(screen.getByTestId("work-item-plan-artifact-version-2"));

    expect(screen.getByTestId("work-item-plan-artifact-panel")).toHaveTextContent(
      "Backend flow v1",
    );
    expect(screen.getByTestId("work-item-plan-artifact-panel")).toHaveTextContent("只读历史");
  });

  it("unknown work item plan node type renders processing card", async () => {
    mockWorkspaceWs();
    useWorkspaceStore.setState({
      sessionId: "workspace_session_0001",
      workspaceType: "work_item_plan",
      stage: "human_confirm",
      providers: { author: "claude_code", reviewer: "codex" },
      activeNodeId: "node_future",
      selectedNodeId: "node_future",
      timelineNodes: [
        timelineNode({
          node_id: "node_future",
          node_type: "work_item_plan_future_phase",
          stage: "human_confirm",
          title: "Future phase",
        }),
      ],
      workItemPlanArtifact: { type: "outline_candidate", payload: workItemPlanOutlinePayload() },
    });

    render(<ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />);
    await userEvent.click(screen.getByRole("button", { name: "Artifact" }));

    expect(screen.getByTestId("work-item-plan-staged-panel")).toHaveTextContent("系统处理中");
    expect(screen.getByTestId("work-item-plan-staged-panel")).toHaveTextContent(
      "work_item_plan_future_phase",
    );
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

function workItemPlanCandidate(
  overrides: Partial<import("../api/types").WorkItemPlanCandidateDto> = {},
): import("../api/types").WorkItemPlanCandidateDto {
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
    ...overrides,
  };
}

function workItemPlanOutlinePayload() {
  return {
    outline: {
      id: "outline_version_001",
      plan_id: "plan_001",
      strategy_summary: "Split frontend and backend work.",
      work_items: [
        {
          outline_id: "outline_backend",
          title: "Backend flow",
          kind: "backend",
          sequence_hint: 1,
          depends_on_outline_ids: [],
          exclusive_write_scopes: ["src/product"],
          forbidden_write_scopes: [],
          context_budget: {
            target_context_k: "medium",
            max_summary_chars: 4000,
            max_handoff_chars: 2000,
            max_code_context_chars: 12000,
            max_context_file_refs: 12,
            max_traceability_refs: 12,
            max_dependency_handoffs: 4,
          },
          required_handoff_from_outline_ids: [],
          verification_strategy: "cargo test --locked",
          risk_notes: [],
        },
      ],
      dependency_graph: [],
      risks: [],
      handoff_plan: [],
      created_at: "2026-06-23T00:00:00Z",
      updated_at: "2026-06-23T00:00:00Z",
    },
    design_context_gaps: [],
    validator_findings: [],
    context_blockers: [],
    current_generation_round_id: "round_001",
    selected_generation_mode: null,
  };
}

function workItemDraftPayload(title = "Backend flow") {
  return {
    draft_record: {
      draft_id: "draft_backend_001",
      plan_id: "plan_001",
      generation_round_id: "round_001",
      outline_id: "outline_backend",
      batch_id: null,
      candidate: {
        outline_id: "outline_backend",
        title,
        kind: "backend",
        implementation_context: "Implement backend state transitions.",
        exclusive_write_scopes: ["src/product"],
        forbidden_write_scopes: [],
        depends_on_outline_ids: [],
        required_handoff_from_outline_ids: [],
        verification_plan: {
          commands: [],
          manual_checks: [],
          required_gates: [],
          risk_notes: [],
        },
        handoff_summary: "Backend state is ready for frontend.",
      },
      status: "draft",
      active: true,
      superseded: false,
      superseded_by_draft_id: null,
      supersede_reason: null,
      copied_from_draft_id: null,
      generated_from_node_id: "node_draft",
      accepted_by_node_id: null,
      created_at: "2026-06-23T00:00:00Z",
      updated_at: "2026-06-23T00:00:00Z",
    },
    validator_findings: [],
    can_accept: true,
  };
}

function workItemBatchPayload(withFailure = false) {
  return {
    batch_id: "batch_001",
    generation_round_id: "round_001",
    queue: ["outline_backend"],
    draft_records: [workItemDraftPayload().draft_record],
    batch_status: "completed",
    failure_summary: withFailure
      ? [
          {
            draft_id: "draft_backend_001",
            outline_id: "outline_backend",
            status: "validation_failed",
          },
        ]
      : [],
  };
}

function workItemCompileReportPayload(planCommitState: string) {
  return {
    compile_id: "compile_001",
    generation_round_id: "round_001",
    status: "recovery_required",
    plan_commit_state: planCommitState,
    work_item_ids: ["work_item_backend"],
    verification_plan_ids: ["verification_backend"],
    child_session_ids: ["session_child_backend"],
    validator_findings: [],
  };
}
