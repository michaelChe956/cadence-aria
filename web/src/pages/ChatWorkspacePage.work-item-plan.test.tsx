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
import { CHAT_COCKPIT_STORAGE_KEY } from "../state/chat-cockpit-mode";
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
  workItemProjectionSessionArtifacts,
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

describe("ChatWorkspacePage work item plan flow", () => {
  installChatWorkspacePageTestHooks();
  beforeEach(() => {
    window.localStorage.setItem(CHAT_COCKPIT_STORAGE_KEY, "legacy");
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

    render(
      <ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} onOpenSession={vi.fn()} />,
    );
    await userEvent.click(screen.getByRole("button", { name: "Artifact" }));

    expect(
      screen.queryByTestId("work-item-plan-candidate-panel"),
    ).not.toBeInTheDocument();
    expect(
      screen.getByText("尚未生成候选，请点击开始生成"),
    ).toBeInTheDocument();
  });

  // L1/OQ2（REQ-RET-02）：Legacy 页逐段决策发送面（generation-mode/outline/draft/
  // batch 的 ChatInputBar 与 StagedPanel 按钮）剥离——下列七测退役：
  // 「generation mode node shows serial batch revision buttons」「generation mode
  // node shows mode actions in chat controls instead of review actions」「outline
  // confirm node shows accept and rewrite actions」「renders outline then mode then
  // serial draft confirm」「draft confirm announces validation findings at artifact
  // and chat entry points」「draft confirm chat controls send work item draft
  // decisions instead of author review decisions」「invalid draft chat controls
  // hide accept and do not expose review actions」。只读呈现断言保留于下方
  // restored projections 与历史 artifact 测试；cockpit 侧 staged 按钮装配（回调
  // 提供时渲染）由 ChatInputBar 组件契约承载，随 T5 逐段面删除一并退役。





  it("renders restored plan projections from full structured artifact versions", async () => {
    const api = mockWorkspaceWs();
    const projectionArtifacts = workItemProjectionSessionArtifacts();
    useWorkspaceStore.getState().setSessionState({
      session_id: "workspace_session_0001",
      workspace_type: "work_item_plan",
      stage: "human_confirm",
      session_status: "waiting_for_human",
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
      artifact: {
        plan_projection: projectionArtifacts.planProjection,
      } as never,
      providers: { author: "claude_code", reviewer: "codex" },
      artifact_versions: projectionArtifacts.artifactVersions as never,
      artifact_version_summaries:
        projectionArtifacts.artifactVersionSummaries,
      active_node_id: "node-compile",
      timeline_nodes: [
        timelineNode({
          node_id: "node-compile",
          node_type: "work_item_plan_compile",
          stage: "human_confirm",
          title: "Compile Work Item Plan",
        }),
      ],
    });

    render(
      <ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} onOpenSession={vi.fn()} />,
    );
    await userEvent.click(screen.getByRole("button", { name: "Artifact" }));

    expect(screen.getByText("仓库初始化实时进度")).toBeInTheDocument();
    expect(
      screen.getByText("Plan Projection plan-revision-01 已发布。"),
    ).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "Human Overview" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(
      screen.queryByTestId("work-item-plan-staged-panel"),
    ).not.toBeInTheDocument();
    // L1/OQ2：Legacy 页只读化——presentation 编辑表单随 onSavePresentation 不再
    // 提供而隐藏（保存发送面删除；union 变体留待 T5）。
    expect(
      screen.queryByRole("form", { name: "编辑人工说明" }),
    ).not.toBeInTheDocument();


    await userEvent.click(screen.getByRole("tab", { name: "Coder" }));
    expect(screen.getByText("初始化状态模型并提交契约")).toBeInTheDocument();
    expect(screen.getAllByText("等待运行时 Envelope（P5）")).toHaveLength(3);
  });

  it("fails closed when the restored plan references a missing work item projection", async () => {
    mockWorkspaceWs();
    const projectionArtifacts = workItemProjectionSessionArtifacts(true);
    useWorkspaceStore.getState().setSessionState({
      session_id: "workspace_session_0001",
      workspace_type: "work_item_plan",
      stage: "human_confirm",
      session_status: "waiting_for_human",
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
      artifact: {
        plan_projection: projectionArtifacts.planProjection,
      } as never,
      providers: { author: "claude_code", reviewer: "codex" },
      artifact_versions: projectionArtifacts.artifactVersions as never,
      artifact_version_summaries:
        projectionArtifacts.artifactVersionSummaries,
    });

    render(
      <ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} onOpenSession={vi.fn()} />,
    );
    await userEvent.click(screen.getByRole("button", { name: "Artifact" }));

    expect(screen.getByText("Projection artifacts 不完整")).toBeInTheDocument();
    expect(screen.getByText("projection-wi-missing")).toBeInTheDocument();
    expect(
      screen.queryByRole("tab", { name: "Human Overview" }),
    ).not.toBeInTheDocument();
  });

  it("keeps a selected historical draft out of the current projection view", async () => {
    mockWorkspaceWs();
    const projectionArtifacts = workItemProjectionSessionArtifacts();
    useWorkspaceStore.getState().setSessionState({
      session_id: "workspace_session_0001",
      workspace_type: "work_item_plan",
      stage: "human_confirm",
      session_status: "waiting_for_human",
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
      artifact: {
        plan_projection: projectionArtifacts.planProjection,
      } as never,
      providers: { author: "claude_code", reviewer: "codex" },
      artifact_versions: projectionArtifacts.artifactVersions as never,
      artifact_version_summaries:
        projectionArtifacts.artifactVersionSummaries,
      active_node_id: "node-compile",
      timeline_nodes: [
        timelineNode({
          node_id: "node-draft-history",
          node_type: "work_item_draft_confirm",
          stage: "author_confirm",
          title: "Historical Draft",
        }),
        timelineNode({
          node_id: "node-compile",
          node_type: "work_item_plan_compile",
          stage: "human_confirm",
          title: "Compile Work Item Plan",
        }),
      ],
    });
    const state = useWorkspaceStore.getState();
    useWorkspaceStore.setState({
      selectedNodeId: "node-draft-history",
      workItemPlanArtifactVersions: [
        {
          version: 10,
          generated_by: "claude_code",
          reviewed_by: "codex",
          review_verdict: "pass",
          confirmed_by: null,
          is_current: false,
          created_at: "2026-07-18T09:00:00Z",
          source_node_id: "node-draft-history",
          artifact: {
            type: "draft_candidate",
            payload: workItemDraftPayload(),
          },
        },
        ...state.workItemPlanArtifactVersions,
      ],
    });

    render(
      <ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} onOpenSession={vi.fn()} />,
    );
    await userEvent.click(screen.getByRole("button", { name: "Artifact" }));

    expect(screen.getByRole("button", { name: "Overview" })).toBeInTheDocument();
    expect(
      screen.queryByRole("tab", { name: "Human Overview" }),
    ).not.toBeInTheDocument();
    expect(screen.getByText("Historical Draft")).toBeInTheDocument();
    expect(screen.queryByRole("form", { name: "编辑人工说明" })).not.toBeInTheDocument();
  });
});
