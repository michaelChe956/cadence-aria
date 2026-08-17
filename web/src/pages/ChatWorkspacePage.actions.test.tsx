import { act, render, screen, waitFor } from "@testing-library/react";
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

describe("ChatWorkspacePage chat actions", () => {
  installChatWorkspacePageTestHooks();

  it("starts generation with provider config from the chat input", async () => {
    const api = mockWorkspaceWs();
    useWorkspaceStore.setState({
      sessionId: "workspace_session_0001",
      stage: "prepare_context",
      providers: { author: "claude_code", reviewer: "codex" },
      reviewerEnabled: true,
      reviewRounds: 1,
    });

    render(
      <ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />,
    );
    useWorkspaceStore.setState({
      providers: { author: "fake", reviewer: "codex" },
      reviewerEnabled: true,
      reviewRounds: 2,
    });

    await userEvent.click(screen.getByTestId("start-generation"));

    expect(api.sendStartGeneration).toHaveBeenCalledWith(
      {
        author: "fake",
        reviewer: "codex",
        review_rounds: 2,
        permission_modes: { author: "auto", reviewer: "auto" },
      },
      true,
    );
  });

  it("retries the recoverable interrupted run and hides start generation", async () => {
    const api = mockWorkspaceWs();
    useWorkspaceStore.setState({
      sessionId: "workspace_session_0001",
      workspaceType: "work_item_plan",
      stage: "prepare_context",
      providers: { author: "claude_code", reviewer: "codex" },
      recoverableInterruptedRun: {
        failed_node_id: "timeline_node_054",
        operation: "review",
        label: "重试中断审核",
      },
      timelineNodes: [
        timelineNode({
          node_id: "timeline_node_058",
          node_type: "aborted_by_disconnect",
          stage: "prepare_context",
          status: "failed",
          title: "运行因断开中止",
          completed_at: "2026-07-11T17:41:03Z",
        }),
      ],
      acknowledgedAbortedNodes: [],
    });

    render(
      <ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />,
    );

    expect(screen.queryByRole("button", { name: "开始生成" })).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "重试中断审核" }));

    expect(api.retryInterruptedRun).toHaveBeenCalledWith("timeline_node_054");
    expect(screen.getByRole("button", { name: "重试中断审核" })).toBeDisabled();

    useWorkspaceStore.setState({ error: "provider unavailable: Codex" });
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "重试中断审核" })).toBeEnabled(),
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

  // spec-workbench-canvas-experience T4：确认并送审/确认定稿迁移至面板
  // actions 插槽；反馈仍从输入发送。决策 payload 不变。
  // spec-workbench-canvas-experience T5：dismissed 时双列 grid 降为单列，
  // 对话流在 ≥1440px 恢复全宽，展开入口沉入输入区上方工具行。
  it("collapses to a single column when the review panel is dismissed", async () => {
    mockWorkspaceWs();
    useWorkspaceStore.setState({
      sessionId: "workspace_session_0001",
      workspaceType: "story",
      stage: "author_confirm",
      providers: { author: "claude_code", reviewer: "codex" },
      artifact: "# Story Spec",
    });

    render(
      <ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />,
    );

    const grid = screen.getByTestId("review-split-grid");
    expect(grid.className).toContain("min-[1440px]:grid-cols-[");

    // 输入聚焦 → 收起 → 单列全宽 + 工具行展开入口。
    await userEvent.click(
      screen.getByPlaceholderText(/输入修改意见/),
    );
    expect(screen.queryByTestId("artifact-review-panel")).not.toBeInTheDocument();
    expect(grid.className).not.toContain("min-[1440px]:grid-cols-[");
    expect(screen.getByTestId("review-panel-restore-slot")).toBeInTheDocument();

    // 从工具行重新展开。
    await userEvent.click(
      screen.getByRole("button", { name: "展开 Artifact 审核" }),
    );
    expect(screen.getByTestId("artifact-review-panel")).toBeInTheDocument();
    expect(grid.className).toContain("min-[1440px]:grid-cols-[");
    expect(
      screen.queryByTestId("review-panel-restore-slot"),
    ).not.toBeInTheDocument();
  });

  // spec-workbench-canvas-experience T5：work_item_plan 在 author_confirm 不走
  // Canvas 审核面板，WorkItemPlanCandidatePanel 自有终局按钮兜底，无死路。
  it("keeps final actions reachable for work_item_plan author_confirm", async () => {
    mockWorkspaceWs();
    useWorkspaceStore.setState({
      sessionId: "workspace_session_0001",
      workspaceType: "work_item_plan",
      stage: "author_confirm",
      providers: { author: "claude_code", reviewer: "codex" },
      workItemPlanCandidate: workItemPlanCandidate(),
    });

    render(
      <ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />,
    );

    expect(
      screen.queryByTestId("artifact-review-panel"),
    ).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Artifact" }));
    expect(
      screen.getByTestId("work-item-plan-candidate-panel"),
    ).toBeInTheDocument();
    expect(screen.getByTestId("accept-plan-button")).toBeEnabled();
    expect(screen.getByTestId("request-revision-button")).toBeInTheDocument();
  });

  // spec-workbench-canvas-experience T5：空 workspaceType（store 默认）在
  // author_confirm 不渲染审核面板，输入区可用、不崩溃。
  it("keeps the chat input usable for the default empty workspaceType", () => {
    mockWorkspaceWs();
    useWorkspaceStore.setState({
      sessionId: "workspace_session_0001",
      stage: "author_confirm",
      providers: { author: "claude_code", reviewer: "codex" },
    });

    render(
      <ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />,
    );

    expect(
      screen.queryByTestId("artifact-review-panel"),
    ).not.toBeInTheDocument();
    expect(
      screen.getByPlaceholderText(/输入修改意见/),
    ).toBeEnabled();
  });

  it("sends author confirmation decisions from the review panel", async () => {
    const api = mockWorkspaceWs();
    useWorkspaceStore.setState({
      sessionId: "workspace_session_0001",
      workspaceType: "story",
      stage: "author_confirm",
      reviewerEnabled: true,
      providers: { author: "fake", reviewer: "codex" },
      artifact: "# Story Spec",
    });

    render(
      <ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />,
    );

    expect(
      screen.queryByRole("button", { name: "重新编写" }),
    ).not.toBeInTheDocument();
    expect(screen.getByTestId("artifact-review-panel")).toBeInTheDocument();

    await userEvent.type(
      screen.getByPlaceholderText(/输入修改意见/),
      "补充回滚策略",
    );
    await userEvent.click(screen.getByRole("button", { name: "发送反馈" }));

    // 输入聚焦会收起面板，重新展开后再点终局确认对。
    await userEvent.click(
      screen.getByRole("button", { name: "展开 Artifact 审核" }),
    );
    expect(screen.getByTestId("artifact-review-panel")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "确认并送审" }));
    await userEvent.click(screen.getByRole("button", { name: "确认定稿" }));

    expect(api.sendAuthorDecision).toHaveBeenNthCalledWith(
      1,
      "revise",
      "补充回滚策略",
    );
    expect(api.sendAuthorDecision).toHaveBeenNthCalledWith(
      2,
      "accept_with_review",
    );
    expect(api.sendAuthorDecision).toHaveBeenNthCalledWith(
      3,
      "accept_finalize",
    );
  });

  describe("author_confirm 产物审核面板开合状态机（spec-workbench-canvas-experience T4）", () => {
    it.each(["story", "design"])(
      "author_confirm 阶段 %s 工作区自动展示产物面板",
      (workspaceType) => {
        mockWorkspaceWs();
        useWorkspaceStore.setState({
          sessionId: "workspace_session_0001",
          workspaceType,
          stage: "author_confirm",
          providers: { author: "claude_code", reviewer: "codex" },
          artifact: "# Spec",
        });

        render(
          <ChatWorkspacePage
            sessionId="workspace_session_0001"
            onBack={vi.fn()}
          />,
        );

        expect(
          screen.getByTestId("artifact-review-panel"),
        ).toBeInTheDocument();
        expect(
          screen.queryByRole("button", { name: "展开 Artifact 审核" }),
        ).not.toBeInTheDocument();
      },
    );

    it("stage 离开 author_confirm 时面板收起且无展开残留", () => {
      mockWorkspaceWs();
      useWorkspaceStore.setState({
        sessionId: "workspace_session_0001",
        workspaceType: "story",
        stage: "running",
        providers: { author: "claude_code", reviewer: "codex" },
        artifact: "# Spec",
      });

      render(
        <ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />,
      );

      expect(
        screen.queryByTestId("artifact-review-panel"),
      ).not.toBeInTheDocument();
      expect(
        screen.queryByRole("button", { name: "展开 Artifact 审核" }),
      ).not.toBeInTheDocument();
    });

    it("stage 重新进入 author_confirm 时重置用户收起（重连恢复自动滑出）", () => {
      mockWorkspaceWs();
      useWorkspaceStore.setState({
        sessionId: "workspace_session_0001",
        workspaceType: "story",
        stage: "author_confirm",
        providers: { author: "claude_code", reviewer: "codex" },
        artifact: "# Spec",
      });

      render(
        <ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />,
      );

      // 输入聚焦 → 收起。
      act(() => {
        screen.getByPlaceholderText(/输入修改意见/).focus();
      });
      expect(
        screen.queryByTestId("artifact-review-panel"),
      ).not.toBeInTheDocument();

      // 离开再回来 → 自动重开。
      act(() => {
        useWorkspaceStore.setState({ stage: "running" });
      });
      act(() => {
        useWorkspaceStore.setState({ stage: "author_confirm" });
      });
      expect(
        screen.getByTestId("artifact-review-panel"),
      ).toBeInTheDocument();
    });

    it("点击采纳 Review 意见预填输入并收起面板", async () => {
      mockWorkspaceWs();
      useWorkspaceStore.setState({
        sessionId: "workspace_session_0001",
        workspaceType: "story",
        stage: "author_confirm",
        reviewerEnabled: true,
        providers: { author: "claude_code", reviewer: "codex" },
        artifact: "# Spec",
        chatEntries: [
          chatEntry({
            type: "review_verdict",
            role: "reviewer",
            content: "发现 3 个问题：第二节缺少回滚策略。",
          }),
        ],
      });

      render(
        <ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />,
      );

      const adoptButton = screen.getByRole("button", { name: "采纳 Review 意见" });
      expect(adoptButton).toBeInTheDocument();
      await userEvent.click(adoptButton);

      const feedbackInput = screen.getByPlaceholderText(
        /输入修改意见/,
      ) as HTMLTextAreaElement;
      expect(feedbackInput.value).toBe(
        "按以下 review 意见修订：\n\n发现 3 个问题：第二节缺少回滚策略。",
      );
      expect(
        screen.queryByTestId("artifact-review-panel"),
      ).not.toBeInTheDocument();
      expect(
        screen.getByRole("button", { name: "展开 Artifact 审核" }),
      ).toBeInTheDocument();
    });

    it("无 review 报告时不渲染采纳按钮，主次样式随 reviewerEnabled", async () => {
      const api = mockWorkspaceWs();
      useWorkspaceStore.setState({
        sessionId: "workspace_session_0001",
        workspaceType: "design",
        stage: "author_confirm",
        reviewerEnabled: false,
        providers: { author: "claude_code", reviewer: "codex" },
        artifact: "# Design",
      });

      render(
        <ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />,
      );

      expect(
        screen.queryByRole("button", { name: "采纳 Review 意见" }),
      ).not.toBeInTheDocument();
      expect(
        screen.getByRole("button", { name: "确认并送审" }).className,
      ).not.toContain("btn-primary");
      expect(
        screen.getByRole("button", { name: "确认定稿" }).className,
      ).toContain("btn-primary");

      await userEvent.click(screen.getByRole("button", { name: "确认并送审" }));
      expect(api.sendAuthorDecision).toHaveBeenCalledWith("accept_with_review");
    });
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

      render(
        <ChatWorkspacePage
          sessionId="workspace_session_0001"
          onBack={vi.fn()}
        />,
      );

      expect(
        screen.getByRole("button", { name: "接受修订建议" }),
      ).toBeInTheDocument();
      expect(
        screen.getByRole("button", { name: "补充上下文后修订" }),
      ).toBeInTheDocument();
      expect(
        screen.getByRole("button", { name: "跳过，人工处理" }),
      ).toBeInTheDocument();

      await userEvent.click(
        screen.getByRole("button", { name: "补充上下文后修订" }),
      );
      await userEvent.type(
        screen.getByLabelText("补充返修上下文"),
        "补充 provider gate 细节",
      );
      await userEvent.click(
        screen.getByRole("button", { name: "提交补充并修订" }),
      );

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

    render(
      <ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />,
    );

    await userEvent.click(screen.getByRole("button", { name: "允许" }));

    expect(api.respondPermission).toHaveBeenCalledWith(
      "perm_001",
      true,
      undefined,
    );
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
        timelineNode({
          node_id: "node-1",
          node_type: "context_note",
          title: "补充上下文",
        }),
        timelineNode({
          node_id: "node-2",
          node_type: "author_run",
          title: "Story Spec 生成",
        }),
      ],
      activeNodeId: "node-2",
      selectedNodeId: "node-1",
      chatEntries: [
        chatEntry({ id: "entry-1", node_id: "node-1", content: "第一条" }),
        chatEntry({ id: "entry-2", node_id: "node-2", content: "第二条" }),
      ],
    });

    render(
      <ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />,
    );

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
          timelineNode({
            node_id: "node-1",
            node_type: "context_note",
            title: "补充上下文",
          }),
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

      render(
        <ChatWorkspacePage
          sessionId="workspace_session_0001"
          onBack={vi.fn()}
        />,
      );
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
          timelineNode({
            node_id: "node-1",
            node_type: "revision",
            title: "Author 返修 Round 2",
          }),
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

      render(
        <ChatWorkspacePage
          sessionId="workspace_session_0001"
          onBack={vi.fn()}
        />,
      );
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

    render(
      <ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />,
    );

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

    render(
      <ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />,
    );

    expect(screen.getByTestId("review-verdict-entry")).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "接受修订建议" }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "补充上下文后修订" }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "跳过，人工处理" }),
    ).not.toBeInTheDocument();
  });
});
