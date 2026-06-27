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

describe("ChatWorkspacePage work item plan artifacts", () => {
  installChatWorkspacePageTestHooks();

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

    render(
      <ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />,
    );
    await userEvent.click(screen.getByRole("button", { name: "Artifact" }));

    expect(
      screen.getByTestId("work-item-plan-artifact-panel"),
    ).toHaveTextContent("outline_backend -> outline_frontend");
    expect(
      screen.getByTestId("work-item-plan-artifact-panel"),
    ).toHaveTextContent("validation_failed");
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
      workItemPlanArtifact: {
        type: "batch_state",
        payload: workItemBatchPayload(true),
      },
    });

    render(
      <ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />,
    );
    await userEvent.click(screen.getByRole("button", { name: "Artifact" }));

    await userEvent.click(screen.getByRole("button", { name: "接受全部" }));
    await userEvent.click(screen.getByRole("button", { name: "整组重写" }));
    await userEvent.click(screen.getByRole("button", { name: "暂停" }));
    await userEvent.click(screen.getByRole("button", { name: "降级串行" }));

    expect(api.sendWorkItemBatchDecision).toHaveBeenNthCalledWith(
      1,
      "accept_all",
    );
    expect(api.sendWorkItemBatchDecision).toHaveBeenNthCalledWith(
      2,
      "rewrite_batch",
    );
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

    render(
      <ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />,
    );
    await userEvent.click(screen.getByRole("button", { name: "Artifact" }));

    expect(
      screen.queryByRole("button", { name: "放弃并回滚" }),
    ).not.toBeInTheDocument();
    expect(
      screen.getByTestId("work-item-plan-artifact-panel"),
    ).toHaveTextContent("work_item_backend");
    expect(
      screen.queryByTestId("compile-report-before-after"),
    ).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "继续" }));
    await userEvent.click(screen.getByRole("button", { name: "转人工" }));

    expect(api.sendWorkItemPlanCompileRecoveryAction).toHaveBeenNthCalledWith(
      1,
      "continue",
    );
    expect(api.sendWorkItemPlanCompileRecoveryAction).toHaveBeenNthCalledWith(
      2,
      "human_triage",
    );
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

    render(
      <ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />,
    );
    await userEvent.click(screen.getByRole("button", { name: "Artifact" }));

    expect(
      screen.getByTestId("work-item-plan-artifact-panel"),
    ).toHaveTextContent("Backend flow v1");
    expect(
      screen.getByTestId("work-item-plan-artifact-panel"),
    ).toHaveTextContent("只读历史");
    expect(
      screen.getByTestId("work-item-plan-artifact-panel"),
    ).toHaveTextContent("正在查看历史版本 v1，不影响当前流程。");
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
          artifact: {
            type: "outline_candidate",
            payload: workItemPlanOutlinePayload(),
          },
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

    render(
      <ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />,
    );
    await userEvent.click(screen.getByRole("button", { name: "Artifact" }));

    const versionRail = screen.getByTestId("work-item-plan-version-rail");
    expect(versionRail).toHaveTextContent("Outline");
    expect(versionRail).toHaveTextContent("Draft");
    expect(versionRail).toHaveTextContent(
      "outline_backend / draft_backend_001",
    );
    expect(versionRail).toHaveTextContent("v3");

    await userEvent.selectOptions(screen.getByLabelText("Artifact version"), "2");

    expect(
      screen.getByTestId("work-item-plan-artifact-panel"),
    ).toHaveTextContent("Backend flow v1");
    expect(
      screen.getByTestId("work-item-plan-artifact-panel"),
    ).toHaveTextContent("只读历史");
    expect(
      screen.getByTestId("work-item-plan-artifact-panel"),
    ).toHaveTextContent("正在查看历史版本 v2，不影响当前流程。");
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
      workItemPlanArtifact: {
        type: "outline_candidate",
        payload: workItemPlanOutlinePayload(),
      },
    });

    render(
      <ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />,
    );
    await userEvent.click(screen.getByRole("button", { name: "Artifact" }));

    expect(screen.getByTestId("work-item-plan-staged-panel")).toHaveTextContent(
      "系统处理中",
    );
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

    render(
      <ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />,
    );
    await userEvent.click(screen.getByRole("button", { name: "Artifact" }));

    expect(
      screen.queryByTestId("work-item-plan-candidate-panel"),
    ).not.toBeInTheDocument();
    expect(screen.getByTestId("monaco-viewer")).toHaveTextContent("# Story");
  });
});
