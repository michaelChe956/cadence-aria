import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ProviderHealthResponse } from "../api/types";
import {
  fetchWorkspaceArtifactVersion,
  fetchWorkspaceEventOutput,
  fetchWorkspaceNodeDetail,
} from "../api/workspace-content";
import { useUnloadGuard } from "../hooks/useUnloadGuard";
import type * as WorkspaceWsModule from "../hooks/useWorkspaceWs";
import { useWorkspaceWs } from "../hooks/useWorkspaceWs";
import { MockWebSocket } from "../hooks/useWorkspaceWs.test-utils";
import {
  emptyWorkspaceContentCache,
  workspaceContentCacheValues,
} from "../state/workspace-content-cache";
import {
  selectChatPanelState,
  selectWorkspaceHeaderState,
  useWorkspaceStore,
} from "../state/workspace-ws-store";
import { useProviderAvailabilityStore } from "../state/provider-availability-store";
import { ChatWorkspacePage } from "./ChatWorkspacePage";
import {
  chatEntry,
  installChatWorkspacePageTestHooks,
  makeNodeDetail,
  mockWorkspaceWs,
  timelineNode,
  workItemBatchPayload,
  workItemCompileReportPayload,
  workItemDraftPayload,
  workItemPlanCandidate,
  workItemPlanOutlinePayload,
} from "./ChatWorkspacePage.test-utils";

vi.mock("../hooks/useWorkspaceWs", async (importOriginal) => ({
  ...(await importOriginal<typeof WorkspaceWsModule>()),
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
  MonacoDiffViewer: ({
    original,
    modified,
  }: {
    original: string;
    modified: string;
  }) => (
    <div data-testid="monaco-diff-viewer">
      {original}
      {modified}
    </div>
  ),
}));

function setPageProviderHealth() {
  const snapshot: ProviderHealthResponse = {
    schema_version: 1,
    generation: 1,
    checked_at: "2026-07-14T00:00:00Z",
    state_status: "ready",
    state_error: null,
    real_workflow_blocked: false,
    test_provider_enabled: false,
    providers: [
      {
        provider: "claude_code",
        display_name: "Claude Code",
        available: false,
        version: null,
        reason_code: "command_missing",
        reason: "Claude Code 未安装",
        checked_at: "2026-07-14T00:00:00Z",
        install_hint: "请先安装 Claude Code",
      },
      {
        provider: "codex",
        display_name: "Codex",
        available: true,
        version: "1.0.0",
        reason_code: null,
        reason: null,
        checked_at: "2026-07-14T00:00:00Z",
        install_hint: "",
      },
    ],
  };
  useProviderAvailabilityStore.setState({ snapshot, loadStatus: "loaded" });
}

afterEach(() => {
  useProviderAvailabilityStore.getState().reset();
});

describe("ChatWorkspacePage shell and content loading", () => {
  installChatWorkspacePageTestHooks();
  beforeEach(() => {
    window.localStorage.setItem("aria.chat.cockpit", "legacy");
    vi.mocked(fetchWorkspaceNodeDetail).mockResolvedValue(makeNodeDetail());
  });

  it("assembles the shared Provider catalog without page-level availability wiring", async () => {
    mockWorkspaceWs();
    setPageProviderHealth();
    useWorkspaceStore.setState({
      sessionId: "workspace_session_0001",
      workspaceType: "story",
      stage: "prepare_context",
      providers: { author: "claude_code", reviewer: "codex" },
    });

    render(
      <ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} onOpenSession={vi.fn()} />,
    );
    await userEvent.click(screen.getByRole("button", { name: "Provider 配置" }));

    const author = screen.getByLabelText("Author");
    expect(within(author).getByRole("option", { name: "Claude Code" })).toBeDisabled();
    expect(screen.getByText("Claude Code 未安装")).toBeInTheDocument();
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

    render(
      <ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} onOpenSession={vi.fn()} />,
    );

    expect(
      screen.getAllByText(/Story Spec #workspace_session_0001/).length,
    ).toBeGreaterThan(0);
    expect(screen.getByTestId("timeline-node-list")).toBeInTheDocument();
    expect(screen.getByTestId("chat-entry-list")).toHaveTextContent(
      "review output",
    );
    expect(screen.queryByTestId("monaco-viewer")).not.toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: "Artifact" }));

    expect(screen.getByTestId("monaco-viewer")).toHaveTextContent(
      "Artifact v1",
    );
    expect(screen.getByTestId("workspace-status-bar")).toHaveTextContent(
      "running",
    );
  });

  it("loads artifact summary markdown through the workspace content cache", async () => {
    mockWorkspaceWs();
    let resolveArtifact!: (value: {
      version: number;
      markdown: string;
    }) => void;
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

    render(
      <ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} onOpenSession={vi.fn()} />,
    );
    await userEvent.click(screen.getByRole("button", { name: "Artifact" }));

    await waitFor(() => {
      expect(screen.getByTestId("artifact-loading")).toHaveTextContent(
        "正在加载 v1",
      );
    });
    resolveArtifact({ version: 1, markdown: "# Loaded Artifact\n\n内容" });

    expect(await screen.findByText(/Loaded Artifact/)).toBeInTheDocument();
    expect(fetchWorkspaceArtifactVersion).toHaveBeenCalledWith(
      "workspace_session_0001",
      1,
    );
    expect(
      workspaceContentCacheValues(
        useWorkspaceStore.getState().artifactContentCache,
      )["1"],
    ).toBe("# Loaded Artifact\n\n内容");
  });

  it("loads missing typed work item plan artifact versions before displaying history", async () => {
    window.localStorage.setItem("aria.chat.cockpit", "legacy");
    mockWorkspaceWs();
    const outlineArtifact = {
      type: "outline_candidate" as const,
      payload: workItemPlanOutlinePayload(),
    };
    const compileArtifact = {
      type: "compile_report" as const,
      payload: workItemCompileReportPayload("committed"),
    };
    vi.mocked(fetchWorkspaceArtifactVersion).mockResolvedValue({
      version: 10,
      markdown: "",
      artifact: outlineArtifact,
    } as never);
    useWorkspaceStore.setState({
      sessionId: "workspace_session_0001",
      workspaceType: "work_item_plan",
      stage: "human_confirm",
      providers: { author: "claude_code", reviewer: "codex" },
      workItemPlanArtifact: compileArtifact,
      workItemPlanArtifactVersions: [
        {
          version: 10,
          generated_by: "claude_code",
          reviewed_by: "codex",
          review_verdict: "pass",
          confirmed_by: "user",
          is_current: false,
          created_at: "2026-06-26T10:00:00Z",
          source_node_id: "node_outline",
          artifact: null,
        },
        {
          version: 12,
          generated_by: "claude_code",
          reviewed_by: null,
          review_verdict: null,
          confirmed_by: null,
          is_current: true,
          created_at: "2026-06-26T10:02:00Z",
          source_node_id: "node_compile",
          artifact: compileArtifact,
        },
      ],
      artifactVersions: [
        {
          version: 10,
          generated_by: "claude_code",
          reviewed_by: "codex",
          review_verdict: "pass",
          confirmed_by: "user",
          is_current: false,
          created_at: "2026-06-26T10:00:00Z",
          source_node_id: "node_outline",
        },
        {
          version: 12,
          generated_by: "claude_code",
          reviewed_by: null,
          review_verdict: null,
          confirmed_by: null,
          is_current: true,
          created_at: "2026-06-26T10:02:00Z",
          source_node_id: "node_compile",
        },
      ],
    });

    render(
      <ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} onOpenSession={vi.fn()} />,
    );
    await userEvent.click(screen.getByRole("button", { name: "Artifact" }));
    await userEvent.selectOptions(screen.getByLabelText("Artifact phase"), "unknown");
    await userEvent.selectOptions(screen.getByLabelText("Artifact version"), "10");

    await waitFor(() => {
      expect(fetchWorkspaceArtifactVersion).toHaveBeenCalledWith(
        "workspace_session_0001",
        10,
      );
    });
    expect(
      useWorkspaceStore
        .getState()
        .workItemPlanArtifactVersions.find((version) => version.version === 10)
        ?.artifact,
    ).toEqual(outlineArtifact);
    expect(await screen.findByText("Split frontend and backend work.")).toBeInTheDocument();
  });

  it("does not cache artifact content when the workspace session changes before load resolves", async () => {
    mockWorkspaceWs();
    let resolveArtifact!: (value: {
      version: number;
      markdown: string;
    }) => void;
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

    render(
      <ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} onOpenSession={vi.fn()} />,
    );
    await userEvent.click(screen.getByRole("button", { name: "Artifact" }));
    await waitFor(() =>
      expect(fetchWorkspaceArtifactVersion).toHaveBeenCalledWith(
        "workspace_session_0001",
        1,
      ),
    );

    useWorkspaceStore.setState({
      sessionId: "workspace_session_0002",
      artifactContentCache: emptyWorkspaceContentCache(),
    });
    resolveArtifact({ version: 1, markdown: "# Stale Artifact" });
    await Promise.resolve();

    expect(
      workspaceContentCacheValues(
        useWorkspaceStore.getState().artifactContentCache,
      )["1"],
    ).toBeUndefined();
  });

  it("does not cache chat content when the workspace session changes before load resolves", async () => {
    mockWorkspaceWs();
    let resolveOutput!: (value: {
      node_id: string;
      event_id: string;
      output: string;
    }) => void;
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

    render(
      <ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} onOpenSession={vi.fn()} />,
    );
    await userEvent.click(
      screen.getByRole("button", { name: /Execution Output/ }),
    );
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

    expect(
      workspaceContentCacheValues(useWorkspaceStore.getState().contentCache),
    ).toEqual({});
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

    render(
      <ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} onOpenSession={vi.fn()} />,
    );

    await waitFor(() => {
      expect(fetchWorkspaceNodeDetail).toHaveBeenCalledWith(
        "workspace_session_0001",
        "timeline_node_017",
      );
    });
    expect(await screen.findByText("完整 review 输出")).toBeInTheDocument();
  });

  it("hydrates active node detail when restored state has only timeline nodes", async () => {
    mockWorkspaceWs();
    vi.mocked(fetchWorkspaceNodeDetail).mockResolvedValue(
      makeNodeDetail({
        node_id: "timeline_node_codegen",
        node_type: "author_run",
        agent_role: "author",
        streaming_content: "",
        execution_events: [
          {
            event_id: "event_codegraph",
            node_id: "timeline_node_codegen",
            agent: "claude_code",
            kind: "command",
            status: "completed",
            title: "mcp__codegraph__codegraph_explore",
            command: "mcp__codegraph__codegraph_explore",
            cwd: null,
            output: "CodeGraph output",
            exit_code: 0,
          },
        ],
      }),
    );
    useWorkspaceStore.setState({
      sessionId: "workspace_session_0001",
      workspaceType: "story",
      stage: "running",
      providers: { author: "claude_code", reviewer: "codex" },
      selectedNodeId: "timeline_node_codegen",
      activeNodeId: "timeline_node_codegen",
      timelineNodes: [
        timelineNode({
          node_id: "timeline_node_codegen",
          node_type: "author_run",
          title: "Author Run",
          status: "completed",
        }),
      ],
      nodeDetails: {},
      chatEntries: [],
    });

    render(
      <ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} onOpenSession={vi.fn()} />,
    );

    await waitFor(() => {
      expect(fetchWorkspaceNodeDetail).toHaveBeenCalledWith(
        "workspace_session_0001",
        "timeline_node_codegen",
      );
    });
    expect(
      await screen.findByText("mcp__codegraph__codegraph_explore"),
    ).toBeInTheDocument();
  });
  // 「uses the stable typed gate command after a session snapshot arrives」随 OQ2
  // 只读化退役：Legacy 页门卡不再装配动作面（typed 反馈编辑器随之不渲染）；
  // stable command 语义由 cockpit-action-routing.test.ts
  // 「reuses the live turn command id instead of regenerating one」承接。

});

describe("ChatWorkspacePage dual track switch", () => {
  installChatWorkspacePageTestHooks();

  function setWorkspaceType(workspaceType: "story" | "design" | "work_item_plan") {
    useWorkspaceStore.getState().setSessionState({
      session_id: "session_switch",
      workspace_type: workspaceType,
      stage: "prepare_context",
      session_status: "open",
      flow_kind: "legacy",
      run_policy: "interactive",
      run_history: {
        seen_fingerprints: [],
        repairs_used: 0,
        manual_repairs_used: 0,
        transitions_used: 0,
        initial_review_count: 0,
        verification_review_count: 0,
      },
      messages: [],
      checkpoints: [],
      artifact: null,
      providers: { author: "claude_code", reviewer: "codex" },
    });
  }

  function renderWorkspace(mockWs = true) {
    if (mockWs) {
      mockWorkspaceWs();
    }
    return render(
      <ChatWorkspacePage
        sessionId="session_switch"
        onBack={vi.fn()}
        onOpenSession={vi.fn()}
      />,
    );
  }

  it.each(["work_item_plan", "story", "design"] as const)(
    "defaults %s sessions to the cockpit",
    (workspaceType) => {
      setWorkspaceType(workspaceType);

      renderWorkspace();

      expect(screen.getByTestId("cockpit-page")).toBeInTheDocument();
      expect(screen.queryByTestId("workspace-status-bar")).toBeNull();
    },
  );

  it("keeps a known session in the legacy form when legacy is explicitly set", () => {
    window.localStorage.setItem("aria.chat.cockpit", "legacy");
    setWorkspaceType("work_item_plan");

    renderWorkspace();

    expect(screen.getByTestId("workspace-status-bar")).toBeInTheDocument();
    expect(screen.queryByTestId("cockpit-page")).toBeNull();
  });

  it("keeps the legacy start-generation flow usable when legacy is explicitly set", () => {
    window.localStorage.setItem("aria.chat.cockpit", "legacy");
    setWorkspaceType("story");

    renderWorkspace();

    expect(screen.getByTestId("workspace-status-bar")).toBeInTheDocument();
    expect(screen.getByTestId("start-generation")).toBeInTheDocument();
  });

  it("renders a known session in the cockpit when cockpit is explicitly set", () => {
    window.localStorage.setItem("aria.chat.cockpit", "cockpit");
    setWorkspaceType("story");

    renderWorkspace();

    expect(screen.getByTestId("cockpit-page")).toBeInTheDocument();
    expect(screen.queryByTestId("workspace-status-bar")).toBeNull();
  });

  it("keeps the connection shell for a stale session type", () => {
    setWorkspaceType("work_item_plan");

    render(
      <ChatWorkspacePage sessionId="session_story" onBack={vi.fn()} onOpenSession={vi.fn()} />,
    );

    expect(screen.getByTestId("workspace-connection-shell")).toBeInTheDocument();
    expect(screen.queryByTestId("cockpit-page")).toBeNull();
    expect(screen.queryByTestId("workspace-status-bar")).toBeNull();
  });

  it("keeps the connection shell for a session following a cockpit-form session", () => {
    setWorkspaceType("story");

    render(
      <ChatWorkspacePage sessionId="session_next" onBack={vi.fn()} onOpenSession={vi.fn()} />,
    );

    expect(screen.getByTestId("workspace-connection-shell")).toBeInTheDocument();
    expect(screen.queryByTestId("cockpit-page")).toBeNull();
    expect(screen.queryByTestId("workspace-status-bar")).toBeNull();
  });

  it("keeps the connection shell when workspace type is missing", () => {
    renderWorkspace();

    expect(screen.getByTestId("workspace-connection-shell")).toBeInTheDocument();
    expect(screen.queryByTestId("cockpit-page")).toBeNull();
    expect(screen.queryByTestId("workspace-status-bar")).toBeNull();
  });

  it("uses one websocket while unknown becomes a known story", async () => {
    const { useWorkspaceWs: realUseWorkspaceWs } =
      await vi.importActual<typeof WorkspaceWsModule>("../hooks/useWorkspaceWs");
    MockWebSocket.instances = [];
    vi.mocked(useWorkspaceWs).mockImplementation(realUseWorkspaceWs);
    vi.stubGlobal("WebSocket", MockWebSocket);

    renderWorkspace(false);

    expect(screen.getByTestId("workspace-connection-shell")).toBeVisible();
    expect(MockWebSocket.instances).toHaveLength(1);

    act(() => {
      setWorkspaceType("story");
    });

    expect(screen.getByTestId("cockpit-page")).toBeVisible();
    expect(MockWebSocket.instances).toHaveLength(1);
    expect(MockWebSocket.instances[0]?.closeCodes).not.toContain(1000);
  });

  it("keeps one socket and the cockpit form when a started generation moves to running", async () => {
    const { useWorkspaceWs: realUseWorkspaceWs } =
      await vi.importActual<typeof WorkspaceWsModule>("../hooks/useWorkspaceWs");
    MockWebSocket.instances = [];
    vi.mocked(useWorkspaceWs).mockImplementation(realUseWorkspaceWs);
    vi.stubGlobal("WebSocket", MockWebSocket);

    setWorkspaceType("design");
    renderWorkspace(false);

    expect(screen.getByTestId("cockpit-page")).toBeVisible();
    act(() => {
      useWorkspaceStore.setState({ stage: "running", providerStatus: "running" });
    });

    expect(screen.getByTestId("cockpit-page")).toBeVisible();
    expect(MockWebSocket.instances).toHaveLength(1);
    expect(MockWebSocket.instances[0]?.closeCodes).not.toContain(1000);
  });
});
