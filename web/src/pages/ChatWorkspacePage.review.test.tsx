import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  fetchWorkspaceArtifactVersion,
  fetchWorkspaceEventOutput,
  fetchWorkspaceNodeDetail,
} from "../api/workspace-content";
import { useUnloadGuard } from "../hooks/useUnloadGuard";
import type * as WorkspaceWsModule from "../hooks/useWorkspaceWs";
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

describe("ChatWorkspacePage review decisions", () => {
  installChatWorkspacePageTestHooks();
  beforeEach(() => {
    window.localStorage.setItem("aria.chat.cockpit", "legacy");
  });

  // L1/OQ2（REQ-RET-02）：Legacy 页 review_decision 决策面（ReviewDecisionActionBar
  // 的 修复这些建议/不修复，继续生成 按钮与 review_decision 发送）剥离——
  // 原「renders suggestion review decision actions from pending decision options」
  // 「infers suggestion review decision actions…」两测退役，重钉为只读呈现。
  it("renders the review decision stage read-only on the legacy page", async () => {
    mockWorkspaceWs();
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
      chatEntries: [],
    });

    render(
      <ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} onOpenSession={vi.fn()} />,
    );

    expect(
      screen.queryByRole("button", { name: "修复这些建议" }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "不修复，继续生成" }),
    ).not.toBeInTheDocument();
  });

  // 「infers suggestion review decision actions…」同上退役（只读重钉见上测）。

  it("renders the human confirm gate read-only on the legacy page", async () => {
    mockWorkspaceWs();
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
      <ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} onOpenSession={vi.fn()} />,
    );

    // OQ2：Legacy 页门卡只读呈现——typed 动作钮（确认/终止）不装配；
    // typed 动作面由 cockpit 页测试覆盖（ChatCockpitPage.gate）。
    expect(screen.getByTestId("gate-prompt-entry")).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "确认当前版本" }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "终止此门" }),
    ).not.toBeInTheDocument();
  });

  // 「sends request-change payload when adopting suggestion review findings」随
  // request-change 按钮删除退役（L1/REQ-RET-02：前端不再发 legacy request-change）。

  it("renders work item plan candidate panel for work_item_plan workspaces", async () => {
    mockWorkspaceWs();
    useWorkspaceStore.setState({
      sessionId: "workspace_session_0001",
      workspaceType: "work_item_plan",
      stage: "author_confirm",
      providers: { author: "claude_code", reviewer: "codex" },
      workItemPlanCandidate: workItemPlanCandidate(),
    });

    render(
      <ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} onOpenSession={vi.fn()} />,
    );
    await userEvent.click(screen.getByRole("button", { name: "Artifact" }));

    expect(
      screen.getByTestId("work-item-plan-candidate-panel"),
    ).toBeInTheDocument();
    expect(screen.getByText("Work Item Plan 候选")).toBeInTheDocument();

    // OQ2：candidate panel 在 Legacy 页只读——accept-plan-button 不渲染。
    expect(
      screen.queryByTestId("accept-plan-button"),
    ).not.toBeInTheDocument();
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
      <ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} onOpenSession={vi.fn()} />,
    );

    expect(screen.getByTestId("chat-entry-list")).toHaveTextContent(
      "正在生成 Work Item Plan",
    );
    expect(
      screen.getAllByText("Work Item Plan #workspace_session_0001").length,
    ).toBeGreaterThan(0);
  });

  // 「work_item_plan candidate panel supports revert, request revision and accept」
  // 随 OQ2 只读化退役（Legacy 页不再向 candidate panel 提供决策回调；只读呈现断言
  // 由 work_item_plan 面板渲染测试覆盖）。
});
