import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import {
  fetchWorkspaceArtifactVersion,
  fetchWorkspaceEventOutput,
  fetchWorkspaceNodeDetail,
} from "../api/workspace-content";
import { useUnloadGuard } from "../hooks/useUnloadGuard";
import { useWorkspaceWs } from "../hooks/useWorkspaceWs";
import {
  emptyWorkspaceContentCache,
  workspaceContentCacheValues,
} from "../state/workspace-content-cache";
import {
  selectChatPanelState,
  selectWorkspaceHeaderState,
  useWorkspaceStore,
} from "../state/workspace-ws-store";
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

describe("ChatWorkspacePage shell and content loading", () => {
  installChatWorkspacePageTestHooks();

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
      <ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />,
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
      <ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />,
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
      <ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />,
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
      <ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />,
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
      <ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />,
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
      <ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />,
    );

    await waitFor(() => {
      expect(fetchWorkspaceNodeDetail).toHaveBeenCalledWith(
        "workspace_session_0001",
        "timeline_node_017",
      );
    });
    expect(await screen.findByText("完整 review 输出")).toBeInTheDocument();
  });
});
