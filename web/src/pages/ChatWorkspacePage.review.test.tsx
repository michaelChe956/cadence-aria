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

describe("ChatWorkspacePage review decisions", () => {
  installChatWorkspacePageTestHooks();

  it("renders optional review decision actions from pending decision options", async () => {
    const api = mockWorkspaceWs();
    useWorkspaceStore.setState({
      sessionId: "workspace_session_0001",
      workspaceType: "work_item_plan",
      stage: "review_decision",
      providers: { author: "claude_code", reviewer: "codex" },
      pendingDecision: {
        node_id: "timeline_node_decision",
        round: 1,
        options: ["apply_optional_findings", "skip_optional_findings"],
      },
      timelineNodes: [
        timelineNode({
          node_id: "timeline_node_decision",
          node_type: "review_decision",
          stage: "review_decision",
          status: "paused",
          title: "Review Decision Round 1",
          summary: "仅有可选建议",
        }),
      ],
      chatEntries: [
        chatEntry({
          type: "review_verdict",
          role: "reviewer",
          content: "仅有可选建议",
          metadata: {
            verdict: "pass",
            comments: "当前 outline 可继续，但建议补充 handoff。",
            summary: "仅有可选建议",
            review_gate: "user_confirm_allowed",
            findings: [
              {
                severity: "optional",
                message: "handoff 描述可以更明确",
                evidence: "handoff_strategy 只有简短描述",
                impact: "不影响 Draft 生成",
                required_action: "补充上下游交接说明",
              },
            ],
          },
        }),
      ],
    });

    render(
      <ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />,
    );

    expect(
      screen.getByRole("button", { name: "修复这些建议" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "不修复，继续生成" }),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "跳过，人工处理" }),
    ).not.toBeInTheDocument();

    await userEvent.click(
      screen.getByRole("button", { name: "不修复，继续生成" }),
    );
    await userEvent.click(screen.getByRole("button", { name: "修复这些建议" }));

    expect(api.sendReviewDecision).toHaveBeenNthCalledWith(
      1,
      "skip_optional_findings",
    );
    expect(api.sendReviewDecision).toHaveBeenNthCalledWith(
      2,
      "apply_optional_findings",
    );
    expect(api.sendSelectRevisionPath).not.toHaveBeenCalled();
  });

  it("infers optional review decision actions from the latest work item plan verdict", async () => {
    const api = mockWorkspaceWs();
    useWorkspaceStore.setState({
      sessionId: "workspace_session_0001",
      workspaceType: "work_item_plan",
      stage: "review_decision",
      providers: { author: "claude_code", reviewer: "codex" },
      pendingDecision: null,
      timelineNodes: [
        timelineNode({
          node_id: "timeline_node_decision",
          node_type: "review_decision",
          stage: "review_decision",
          status: "paused",
          title: "Review Decision Round 1",
          summary: "仅有可选建议",
        }),
      ],
      chatEntries: [
        chatEntry({
          type: "review_verdict",
          role: "reviewer",
          content: "仅有可选建议",
          metadata: {
            verdict: "pass",
            comments: "当前 outline 可继续，但建议补充 handoff。",
            summary: "仅有可选建议",
            review_gate: "user_confirm_allowed",
            findings: [
              {
                severity: "minor",
                message: "handoff 描述可以更明确",
                evidence: "handoff_strategy 只有简短描述",
                impact: "不影响 Draft 生成",
                required_action: "补充上下游交接说明",
              },
            ],
          },
        }),
      ],
    });

    render(
      <ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />,
    );

    expect(
      screen.getByRole("button", { name: "修复这些建议" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "不修复，继续生成" }),
    ).toBeInTheDocument();

    await userEvent.click(
      screen.getByRole("button", { name: "不修复，继续生成" }),
    );

    expect(api.sendReviewDecision).toHaveBeenCalledWith(
      "skip_optional_findings",
    );
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

    render(
      <ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />,
    );

    await userEvent.click(
      screen.getByRole("button", { name: "确认使用当前版本" }),
    );

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

    render(
      <ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />,
    );

    await userEvent.click(
      screen.getByRole("button", { name: "采纳建议并返修" }),
    );

    expect(api.sendHumanConfirm).toHaveBeenCalledWith(
      "request-change",
      expect.objectContaining({
        description: expect.stringContaining("建议补充说明"),
      }),
    );
    const payload = vi.mocked(api.sendHumanConfirm).mock.calls[0][1] as {
      description: string;
    };
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

    render(
      <ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />,
    );
    await userEvent.click(screen.getByRole("button", { name: "Artifact" }));

    expect(
      screen.getByTestId("work-item-plan-candidate-panel"),
    ).toBeInTheDocument();
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

    render(
      <ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />,
    );

    expect(screen.getByTestId("chat-entry-list")).toHaveTextContent(
      "正在生成 Work Item Plan",
    );
    expect(
      screen.getAllByText("Work Item Plan #workspace_session_0001").length,
    ).toBeGreaterThan(0);
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

    render(
      <ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />,
    );
    await userEvent.click(screen.getByRole("button", { name: "Artifact" }));

    await userEvent.click(screen.getByTestId("start-revert-wi_001"));
    await userEvent.type(
      screen.getByTestId("revert-feedback-input-wi_001"),
      "拆得太粗",
    );
    await userEvent.click(screen.getByTestId("submit-revert-wi_001"));
    expect(api.sendRevertWorkItem).toHaveBeenCalledWith(
      "wi_001",
      "拆得太粗",
      false,
    );

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

    await waitFor(() =>
      expect(screen.getByText(/已标记撤销/)).toBeInTheDocument(),
    );
    const requestRevisionButton = screen.getByTestId("request-revision-button");
    await waitFor(() => expect(requestRevisionButton).not.toBeDisabled());
    expect(requestRevisionButton).toHaveTextContent("重新生成被标记的 1 项");
    await userEvent.click(requestRevisionButton);
    expect(api.sendRequestRevision).toHaveBeenCalled();

    await userEvent.click(screen.getByTestId("accept-plan-button"));
    expect(api.sendAuthorDecision).toHaveBeenCalledWith("accept");
  });
});
