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

describe("ChatWorkspacePage work item plan flow", () => {
  installChatWorkspacePageTestHooks();

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
      <ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />,
    );
    await userEvent.click(screen.getByRole("button", { name: "Artifact" }));

    expect(
      screen.queryByTestId("work-item-plan-candidate-panel"),
    ).not.toBeInTheDocument();
    expect(
      screen.getByText("尚未生成候选，请点击开始生成"),
    ).toBeInTheDocument();
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
      workItemPlanArtifact: {
        type: "outline_candidate",
        payload: workItemPlanOutlinePayload(),
      },
    });

    render(
      <ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />,
    );
    await userEvent.click(screen.getByRole("button", { name: "Artifact" }));

    await userEvent.click(screen.getByRole("button", { name: "逐个生成" }));
    await userEvent.click(screen.getByRole("button", { name: "自动生成" }));
    await userEvent.click(
      screen.getByRole("button", { name: "返回 Outline 返修" }),
    );

    expect(api.sendSelectWorkItemGenerationMode).toHaveBeenNthCalledWith(
      1,
      "serial",
    );
    expect(api.sendSelectWorkItemGenerationMode).toHaveBeenNthCalledWith(
      2,
      "batch",
    );
    expect(api.sendRequestOutlineRevision).toHaveBeenCalledWith();
    expect(
      screen.getByTestId("work-item-plan-artifact-panel"),
    ).toHaveTextContent("Split frontend and backend work.");
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
      workItemPlanArtifact: {
        type: "outline_candidate",
        payload: workItemPlanOutlinePayload(),
      },
    });

    render(
      <ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />,
    );

    expect(screen.getByRole("textbox")).toHaveAttribute(
      "placeholder",
      "请选择 Work Item 生成模式",
    );
    expect(
      screen.queryByRole("button", { name: "进入 Review" }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "重新编写" }),
    ).not.toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: "逐个生成" }));
    await userEvent.click(screen.getByRole("button", { name: "自动生成" }));
    await userEvent.click(
      screen.getByRole("button", { name: "返回 Outline 返修" }),
    );

    expect(api.sendSelectWorkItemGenerationMode).toHaveBeenNthCalledWith(
      1,
      "serial",
    );
    expect(api.sendSelectWorkItemGenerationMode).toHaveBeenNthCalledWith(
      2,
      "batch",
    );
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
      workItemPlanArtifact: {
        type: "outline_candidate",
        payload: workItemPlanOutlinePayload(),
      },
    });

    render(
      <ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />,
    );
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
      workItemPlanArtifact: {
        type: "outline_candidate",
        payload: workItemPlanOutlinePayload(),
      },
    });

    render(
      <ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />,
    );
    await userEvent.click(screen.getByRole("button", { name: "Artifact" }));

    expect(
      screen.getByTestId("work-item-plan-artifact-panel"),
    ).toHaveTextContent("Split frontend and backend work.");
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
      workItemPlanArtifact: {
        type: "outline_candidate",
        payload: workItemPlanOutlinePayload(),
      },
    });
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: "逐个生成" }),
      ).toBeInTheDocument(),
    );
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
      workItemPlanArtifact: {
        type: "draft_candidate",
        payload: workItemDraftPayload(),
      },
    });
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "接受" })).toBeInTheDocument(),
    );
    expect(
      screen.getByTestId("work-item-plan-artifact-panel"),
    ).toHaveTextContent("Backend flow");
    await userEvent.click(screen.getByRole("button", { name: "接受" }));
    expect(api.sendWorkItemDraftDecision).toHaveBeenCalledWith(
      "outline_backend",
      "accept",
    );
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

    render(
      <ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />,
    );
    await userEvent.click(screen.getByRole("button", { name: "Artifact" }));

    expect(
      screen.queryByRole("button", { name: "接受" }),
    ).not.toBeInTheDocument();
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
      workItemPlanArtifact: {
        type: "draft_candidate",
        payload: workItemDraftPayload(),
      },
    });

    render(
      <ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />,
    );

    expect(screen.getByRole("textbox")).toHaveAttribute(
      "placeholder",
      "请确认当前 Work Item Draft",
    );
    expect(
      screen.queryByRole("button", { name: "进入 Review" }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "重新编写" }),
    ).not.toBeInTheDocument();

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

    render(
      <ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />,
    );

    expect(
      screen.queryByRole("button", { name: "接受" }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "进入 Review" }),
    ).not.toBeInTheDocument();
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
});
