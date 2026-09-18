import { useProviderAvailabilityStore } from "../state/provider-availability-store";
import type * as WorkspaceWsModule from "../hooks/useWorkspaceWs";
import type * as ApiClient from "../api/client";
import type { TakeoverResponse } from "../api/types";
import { ApiRequestError, takeoverWorkspaceSession } from "../api/client";
import { act, fireEvent, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  selectCockpitInbox,
  type CockpitInboxItem,
} from "../state/workspace-cockpit-projection";
import { useWorkspaceWs, type WorkspaceWsApi } from "../hooks/useWorkspaceWs";
import { useUnloadGuard } from "../hooks/useUnloadGuard";
import {
  useWorkspaceStore,
  type TimelineNode,
  type WorkspaceWsState,
} from "../state/workspace-ws-store";
import type { PlanProjectionBundle } from "../api/types";
import { fetchWorkspaceArtifactVersion } from "../api/workspace-content";
import { planRepairSnapshotFixture } from "../state/workspace-plan-repair-test-fixtures";
import { observerStateFromSessionState } from "../state/workspace-observer-store";
import { readCockpitSettings } from "../state/cockpit-settings";
import { ChatCockpitPage } from "./ChatCockpitPage";
import { COCKPIT_HOTKEYS } from "../state/cockpit-operation-semantics";
import { useOperationAuditStore } from "../state/operation-audit-store";
import {
  currentMockWorkspaceWs,
  installChatWorkspacePageTestHooks,
  mockWorkspaceWs,
} from "./ChatWorkspacePage.test-utils";
const { bulkConfirmStart } = vi.hoisted(() => ({ bulkConfirmStart: vi.fn() }));

vi.mock("../state/bulk-confirm-store", () => ({
  useBulkConfirmStore: Object.assign(
    (selector: (state: { runs: readonly unknown[] }) => unknown) => selector({ runs: [] }),
    { getState: () => ({ start: bulkConfirmStart }) },
  ),
}));

vi.mock("../hooks/useWorkspaceWs", async (importOriginal) => ({
  ...(await importOriginal<typeof WorkspaceWsModule>()),
  useWorkspaceWs: vi.fn(),
}));
vi.mock("../hooks/useUnloadGuard", () => ({ useUnloadGuard: vi.fn() }));


vi.mock("../api/client", async (importOriginal) => ({
  ...(await importOriginal<typeof ApiClient>()),
  takeoverWorkspaceSession: vi.fn(),
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


const cockpitInbox: CockpitInboxItem[] = [];
const cockpitObservedRecords: Array<{ sessionId: string; state: WorkspaceWsState }> = [];
const watchSession = vi.fn();
vi.mock("../components/cockpit/CockpitShell", () => ({
  useCockpitShellInbox: () =>
    cockpitInbox.length > 0
      ? cockpitInbox
      : selectCockpitInbox(useWorkspaceStore.getState()).map((item) => ({
          ...item,
          id: `${useWorkspaceStore.getState().sessionId}:${item.id}`,
        })),
  useCockpitInboxPulse: () => false,
  useCockpitSessionWatch: () => watchSession,
  useCockpitSettings: () => readCockpitSettings(),
  useCockpitObservedRecords: () => cockpitObservedRecords,
}));
function timelineNode(overrides: Partial<TimelineNode> = {}): TimelineNode {
  return {
    node_id: "node-1",
    node_type: "author_run",
    agent: "claude_code",
    stage: "running",
    round: 1,
    status: "active",
    title: "Author 运行",
    summary: null,
    started_at: "2026-09-13T00:00:00Z",
    completed_at: null,
    duration_ms: null,
    artifact_ref: null,
    provider_config_snapshot: { author: "claude_code", reviewer: null, review_rounds: 1 },
    ...overrides,
  };
}

describe("ChatCockpitPage", () => {
  installChatWorkspacePageTestHooks();
  beforeEach(() => {
    cockpitInbox.splice(0);
    cockpitObservedRecords.splice(0);
    watchSession.mockReset();
    vi.mocked(takeoverWorkspaceSession).mockReset();
    sessionStorage.clear();
    useOperationAuditStore.getState().reset();
    bulkConfirmStart.mockReset();

    useWorkspaceStore.setState({
      stage: "human_confirm",
      flowKind: "single_candidate",
      singleCandidatePhase: "approval",
    });
  });
  const renderCockpit = (
    sessionId = "session_001",
    mockWs = true,
    onOpenSession = vi.fn(),
  ) => {
    if (mockWs) {
      mockWorkspaceWs();
    }
    useWorkspaceStore.getState().setSessionIdForTest(sessionId);
    return render(
      <ChatCockpitPage
        sessionId={sessionId}
        onBack={vi.fn()}
        onOpenSession={onOpenSession}
        workspaceWs={currentMockWorkspaceWs()}
      />,
    );
  };

  const renderCockpitWith = (
    workspaceWs: WorkspaceWsApi,
    sessionId = "session_001",
  ) => {
    useWorkspaceStore.getState().setSessionIdForTest(sessionId);
    return render(
      <ChatCockpitPage
        sessionId={sessionId}
        onBack={vi.fn()}
        onOpenSession={vi.fn()}
        workspaceWs={workspaceWs}
      />,
    );
  };


  it("renders only the unflushed stream increment beside the active streamed entry", () => {
    const streamingEntry = {
      id: "node-stream:stream-active",
      type: "provider_stream" as const,
      role: "author" as const,
      content: "正在起草第一段",
      timestamp: "2026-09-17T10:00:00Z",
      node_id: "node-stream",
    };
    useWorkspaceStore.setState({
      stage: "running",
      activeNodeId: "node-stream",
      chatEntries: [streamingEntry],
      streamBuffers: {
        "node-stream": {
          chunks: [" 尚未刷到正式条目"],
          visibleText: streamingEntry.content,
          role: "author",
        },
      },
    });

    const { rerender } = renderCockpit();
    expect(screen.getByTestId("cockpit-conversation-flow-list")).toHaveTextContent(
      "正在起草第一段",
    );
    expect(screen.getByTestId("cockpit-streaming-content")).toHaveTextContent(
      "尚未刷到正式条目",
    );
    expect(screen.getByTestId("cockpit-streaming-content")).not.toHaveTextContent(
      "正在起草第一段",
    );
    expect(screen.getByTestId("cockpit-streaming-content")).toHaveAttribute(
      "data-frame-window-ms",
      "50",
    );

    useWorkspaceStore.setState({
      streamBuffers: {},
      chatEntries: [{ ...streamingEntry, id: "message-complete" }],
    });
    rerender(
      <ChatCockpitPage
        sessionId="session_001"
        onBack={vi.fn()}
        onOpenSession={vi.fn()}
        workspaceWs={currentMockWorkspaceWs()}
      />,
    );

    expect(screen.queryByTestId("cockpit-streaming-content")).toBeNull();
    expect(screen.getByTestId("cockpit-conversation-flow-list")).toHaveTextContent(
      "正在起草第一段",
    );
  });
  it("starts generation from cockpit with the legacy provider payload and one optimistic entry", async () => {
    const user = userEvent.setup();
    const sendStartGeneration = vi.fn();
    const workspaceWs = mockWorkspaceWs({
      sendStartGeneration,
      connectionStatus: "connected",
    });
    useWorkspaceStore.setState({
      stage: "prepare_context",
      providers: { author: "pi", reviewer: "codex" },
      reviewerEnabled: true,
      reviewRounds: 2,
    });

    renderCockpitWith(workspaceWs);

    await user.click(screen.getByRole("button", { name: "开始生成" }));

    expect(sendStartGeneration).toHaveBeenCalledWith(
      expect.objectContaining({ author: "pi", reviewer: "codex", review_rounds: 2 }),
      true,
    );
    expect(
      useWorkspaceStore
        .getState()
        .chatEntries.filter((entry) => entry.type === "start_generation"),
    ).toHaveLength(1);
  });

  it("answers a permission request entry through the injected ws api", async () => {
    const user = userEvent.setup();
    const respondPermission = vi.fn();
    const workspaceWs = mockWorkspaceWs({ respondPermission });
    useWorkspaceStore.setState({
      stage: "running",
      chatEntries: [
        {
          id: "perm-1",
          type: "permission_request",
          role: "author",
          content: "请求执行命令",
          timestamp: new Date().toISOString(),
          metadata: { request_id: "perm-1" },
        },
      ],
    });

    renderCockpitWith(workspaceWs);

    await user.click(screen.getByRole("button", { name: "允许" }));

    expect(respondPermission).toHaveBeenCalledWith("perm-1", true, undefined);
  });

  it("answers a structured choice request through the injected ws api", async () => {
    const user = userEvent.setup();
    const sendChoiceResponse = vi.fn();
    const workspaceWs = mockWorkspaceWs({ sendChoiceResponse });
    useWorkspaceStore.setState({
      stage: "running",
      chatEntries: [
        {
          id: "choice-1",
          type: "choice_request",
          role: "system",
          content: "请选择下一步",
          timestamp: "2026-09-17T10:00:00Z",
          resolved: false,
          metadata: {
            request_id: "choice-1",
            prompt: "请选择下一步",
            allow_multiple: false,
            allow_free_text: false,
            options: [
              { id: "continue", label: "继续" },
              { id: "stop", label: "停止" },
            ],
          },
        },
      ],
    });

    renderCockpitWith(workspaceWs);

    await user.click(screen.getByLabelText("继续"));
    await user.click(screen.getByRole("button", { name: "提交选择" }));

    expect(sendChoiceResponse).toHaveBeenCalledWith(
      "choice-1",
      ["continue"],
      null,
      undefined,
    );
  });

  it("guides an unstarted session from the empty inbox instead of an error state", () => {
    const workspaceWs = mockWorkspaceWs();
    useWorkspaceStore.setState({ stage: "prepare_context", timelineNodes: [] });

    renderCockpitWith(workspaceWs);

    expect(
      screen.getByText("会话尚未开始——在右侧选择 Provider 并点击「开始生成」"),
    ).toBeVisible();
  });

  it("keeps interactive entries read-only for a takeover observed session", async () => {
    const user = userEvent.setup();
    vi.mocked(takeoverWorkspaceSession).mockResolvedValue({
      workspace_session_id: "child_perm_001",
    } as TakeoverResponse);
    cockpitInbox.push(stoppedItem("session_001"));
    cockpitObservedRecords.push({
      sessionId: "child_perm_001",
      state: {
        ...observerStateFromSessionState({
          type: "session_state",
          session_id: "child_perm_001",
          workspace_type: "work_item",
          stage: "running",
          superpowers_enabled: false,
          openspec_enabled: false,
          messages: [],
          checkpoints: [],
          artifact: null,
          providers: { author: "claude_code", reviewer: null },
          timeline_nodes: [],
          active_node_id: null,
          artifact_versions: [],
          timeline_node_details: {},
          active_run_id: null,
          human_presentation_revisions: [],
          session_status: "running",
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
        }),
        chatEntries: [
          {
            id: "perm-obs-1",
            type: "permission_request",
            role: "author",
            content: "请求执行命令",
            timestamp: "2026-09-17T10:00:00Z",
            metadata: { request_id: "perm-obs-1" },
          },
        ],
      } as WorkspaceWsState,
    });

    renderCockpit();
    await user.click(screen.getByRole("button", { name: "接管" }));
    await user.click(screen.getByRole("button", { name: "确认接管" }));

    expect(await screen.findByText("请求执行命令")).toBeVisible();
    expect(screen.queryByRole("button", { name: "允许" })).toBeNull();
    expect(screen.queryByRole("button", { name: "拒绝" })).toBeNull();
  });

  it("opens the shared provider configuration dialog during prepare context", async () => {
    const user = userEvent.setup();
    const workspaceWs = mockWorkspaceWs({ connectionStatus: "connected" });
    useWorkspaceStore.setState({ stage: "prepare_context" });

    renderCockpitWith(workspaceWs);

    await user.click(screen.getByRole("button", { name: "Provider 配置" }));

    expect(screen.getByRole("dialog", { name: "Provider 配置" })).toBeVisible();
  });

  it("saves the current provider selection as the workspace default", async () => {
    const user = userEvent.setup();
    const workspaceWs = mockWorkspaceWs();
    useWorkspaceStore.setState({
      stage: "prepare_context",
      providers: { author: "pi", reviewer: "codex" } as const,
      reviewerEnabled: true,
    });

    renderCockpitWith(workspaceWs);
    await user.click(screen.getByTestId("save-provider-defaults"));

    expect(window.localStorage.getItem("aria.workspace.provider-defaults")).toContain(
      '"author":"pi"',
    );
    expect(screen.getByText("已设为默认")).toBeVisible();
  });

  it("applies remembered defaults once when a fresh session enters the editable window", () => {
    window.localStorage.setItem(
      "aria.workspace.provider-defaults",
      JSON.stringify({ author: "pi", reviewer: "kimi_code", reviewerEnabled: true }),
    );
    const selectProvider = vi.fn();
    const workspaceWs = mockWorkspaceWs({ selectProvider });
    useWorkspaceStore.setState({
      sessionId: "session_001",
      stage: "prepare_context",
      providers: { author: "claude_code", reviewer: "codex" } as const,
      reviewerEnabled: false,
    });

    renderCockpitWith(workspaceWs, "session_001");

    expect(selectProvider).toHaveBeenCalledWith("author", "pi");
    expect(selectProvider).toHaveBeenCalledWith("reviewer", "kimi_code");
    expect(useWorkspaceStore.getState().reviewerEnabled).toBe(true);

    act(() => {
      useWorkspaceStore.setState({
        providers: { author: "codex", reviewer: "codex" } as const,
      });
    });
    expect(selectProvider).toHaveBeenCalledTimes(2);
  });

  it("does not apply defaults outside the editable window or for observed sessions", async () => {
    window.localStorage.setItem(
      "aria.workspace.provider-defaults",
      JSON.stringify({ author: "pi", reviewer: "kimi_code", reviewerEnabled: true }),
    );
    const selectProvider = vi.fn();
    useWorkspaceStore.setState({
      sessionId: "session_001",
      stage: "running",
      providers: { author: "codex", reviewer: "codex" } as const,
      reviewerEnabled: false,
    });
    const runningView = renderCockpitWith(mockWorkspaceWs({ selectProvider }));
    expect(selectProvider).not.toHaveBeenCalled();
    runningView.unmount();

    const user = userEvent.setup();
    vi.mocked(takeoverWorkspaceSession).mockResolvedValue({
      workspace_session_id: "child_defaults_001",
    } as TakeoverResponse);
    cockpitInbox.push(stoppedItem("session_001"));
    cockpitObservedRecords.push({
      sessionId: "child_defaults_001",
      state: observerStateFromSessionState({
        type: "session_state",
        session_id: "child_defaults_001",
        workspace_type: "work_item",
        stage: "prepare_context",
        superpowers_enabled: false,
        openspec_enabled: false,
        messages: [
          {
            id: "message_001",
            role: "author",
            content: "子会话待发起",
            created_at: "2026-09-17T00:00:00Z",
          },
        ],
        checkpoints: [],
        artifact: null,
        providers: { author: "claude_code", reviewer: null },
        timeline_nodes: [],
        active_node_id: null,
        artifact_versions: [],
        timeline_node_details: {},
        active_run_id: null,
        human_presentation_revisions: [],
        session_status: "running",
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
      }),
    });
    useWorkspaceStore.setState({ stage: "prepare_context", providers: null });
    renderCockpitWith(mockWorkspaceWs({ selectProvider }));
    await user.click(screen.getByRole("button", { name: "接管" }));
    await user.click(screen.getByRole("button", { name: "确认接管" }));
    expect(await screen.findByText(/Author：claude_code/)).toBeVisible();

    act(() => {
      useWorkspaceStore.setState({
        providers: { author: "codex", reviewer: "codex" } as const,
      });
    });

    expect(selectProvider).not.toHaveBeenCalled();
  });

  it("keeps provider configuration read-only while running", () => {
    useWorkspaceStore.setState({
      stage: "running",
      providers: { author: "pi", reviewer: "codex" } as const,
    });

    renderCockpitWith(mockWorkspaceWs());

    expect(screen.queryByTestId("save-provider-defaults")).toBeNull();
    expect(screen.queryByRole("button", { name: "Provider 配置" })).toBeNull();
    expect(screen.getByText(/Author：pi/)).toBeVisible();
  });

  it("selects providers through the injected workspace websocket", async () => {
    const user = userEvent.setup();
    const selectProvider = vi.fn();
    const workspaceWs = mockWorkspaceWs({
      connectionStatus: "connected",
      selectProvider,
    });
    useWorkspaceStore.setState({ stage: "prepare_context" });
    useProviderAvailabilityStore.setState({
      snapshot: {
        schema_version: 1,
        generation: 1,
        checked_at: "2026-09-17T00:00:00Z",
        state_status: "ready",
        state_error: null,
        real_workflow_blocked: false,
        test_provider_enabled: false,
        providers: [
          {
            provider: "claude_code",
            display_name: "Claude Code",
            available: true,
            version: "1.0.0",
            reason_code: null,
            reason: null,
            checked_at: "2026-09-17T00:00:00Z",
            install_hint: "",
          },
          {
            provider: "codex",
            display_name: "Codex",
            available: true,
            version: "1.0.0",
            reason_code: null,
            reason: null,
            checked_at: "2026-09-17T00:00:00Z",
            install_hint: "",
          },
          {
            provider: "pi",
            display_name: "Pi",
            available: true,
            version: "1.0.0",
            reason_code: null,
            reason: null,
            checked_at: "2026-09-17T00:00:00Z",
            install_hint: "",
          },
        ],
      },
    });

    renderCockpitWith(workspaceWs);
    await user.click(screen.getByRole("button", { name: "Provider 配置" }));
    await user.selectOptions(screen.getByLabelText("Author"), "pi");

    expect(selectProvider).toHaveBeenCalledWith("author", "pi");
  });

  it("hides editable generation controls while running and disables start generation when disconnected", () => {
    const runningWorkspaceWs = mockWorkspaceWs({ connectionStatus: "connected" });
    useWorkspaceStore.setState({ stage: "running" });

    const view = renderCockpitWith(runningWorkspaceWs);

    expect(screen.queryByRole("button", { name: "Provider 配置" })).toBeNull();
    expect(screen.queryByRole("button", { name: "开始生成" })).toBeNull();

    view.unmount();
    const disconnectedWorkspaceWs = mockWorkspaceWs({ connectionStatus: "disconnected" });
    useWorkspaceStore.setState({ stage: "prepare_context" });
    renderCockpitWith(disconnectedWorkspaceWs);

    expect(screen.getByRole("button", { name: "开始生成" })).toBeDisabled();
  });

  it("returns a plan-repair child through durable parent_session_id", async () => {
    const onOpenSession = vi.fn();
    useWorkspaceStore.setState({ planRepair: planRepairSnapshotFixture("child_001") });

    renderCockpit("child_001", true, onOpenSession);

    await userEvent.click(screen.getByRole("button", { name: "返回父会话" }));

    expect(onOpenSession).toHaveBeenCalledWith("session_parent");
  });

  it("restores a takeover parent only for this browser session", async () => {
    const onOpenSession = vi.fn();
    sessionStorage.setItem("aria.takeover-parent:child_002", "parent_002");

    renderCockpit("child_002", true, onOpenSession);

    await userEvent.click(screen.getByRole("button", { name: "返回父会话" }));

    expect(onOpenSession).toHaveBeenCalledWith("parent_002");
  });

  it("does not show a parent action on an ordinary session", () => {
    renderCockpit("ordinary");

    expect(screen.queryByRole("button", { name: "返回父会话" })).toBeNull();
  });

  it("opens cockpit audit and filters the current gate", async () => {
    const user = userEvent.setup();
    const store = useWorkspaceStore.getState();
    store.applyHumanGateTurnOpen("turn_001", "command_001", 1);
    store.setHumanGateSnapshot({
      findings: [],
      repeated_fingerprints: [],
      attempts_used: 1,
      manual_repairs_remaining: 1,
      trigger: "verification_new_findings",
      resumable: true,
    });
    useOperationAuditStore.getState().record({
      sessionId: "session_001",
      gateId: "turn_001",
      operation: "confirm",
      source: "chat",
      outcome: "sent",
      detail: null,
    });

    renderCockpit();
    await user.click(screen.getByRole("button", { name: "操作审计" }));

    const audit = screen.getByTestId("operation-audit-view");
    expect(audit).toBeVisible();
    expect(audit).toHaveTextContent("session_001 · turn_001");
  });

  it("shows takeover only on stopped and disables it with 409 code plus reason", async () => {
    const user = userEvent.setup();
    vi.mocked(takeoverWorkspaceSession).mockRejectedValue(
      new ApiRequestError({
        code: "workspace_session_takeover_not_allowed",
        message: "not allowed",
        details: { reason: "not stopped" },
      }),
    );

    cockpitInbox.push(stoppedItem("s1"), hardErrorItem("s2"));

    renderCockpit();


    expect(screen.queryAllByRole("button", { name: "接管" })).toHaveLength(1);
    await user.click(screen.getByRole("button", { name: "接管" }));
    await user.click(screen.getByRole("button", { name: "确认接管" }));

    expect(
      await screen.findByText("workspace_session_takeover_not_allowed · not stopped"),
    ).toBeVisible();
    expect(screen.getByRole("button", { name: "接管" })).toBeDisabled();
    expect(takeoverWorkspaceSession).toHaveBeenCalledWith("s1");
  });


  it("resets takeover confirmation and shows non-409 errors", async () => {
    const user = userEvent.setup();
    vi.mocked(takeoverWorkspaceSession).mockRejectedValue(new Error("服务暂不可用"));
    cockpitInbox.push(stoppedItem("session_001"));

    renderCockpit();
    await user.click(screen.getByRole("button", { name: "接管" }));
    await user.click(screen.getByRole("button", { name: "确认接管" }));

    expect(await screen.findByText("接管失败：服务暂不可用")).toBeVisible();
    expect(screen.getByRole("button", { name: "接管" })).toBeVisible();
  });

  it("resets takeover confirmation after success", async () => {
    const user = userEvent.setup();
    vi.mocked(takeoverWorkspaceSession).mockResolvedValue({
      workspace_session_id: "child_001",
    } as TakeoverResponse);
    cockpitInbox.push(stoppedItem("session_001"));

    renderCockpit();
    await user.click(screen.getByRole("button", { name: "接管" }));
    await user.click(screen.getByRole("button", { name: "确认接管" }));

    expect(await screen.findByRole("button", { name: "接管" })).toBeVisible();
  });

  it("offers retry and terminate but never takeover for hard error", () => {
    cockpitInbox.push(hardErrorItem("session_001"));

    renderCockpit();

    expect(screen.getByRole("button", { name: "重试" })).toBeVisible();
    expect(screen.getByRole("button", { name: "终止" })).toBeVisible();
    expect(screen.queryByRole("button", { name: "接管" })).toBeNull();
  });
  it("replays the rejected advance with its existing command ID", async () => {
    const user = userEvent.setup();
    const sendAdvance = vi.fn(() => true);
    mockWorkspaceWs({ sendAdvance });
    cockpitInbox.push({
      ...hardErrorItem("session_001"),
      id: "session_001:hard_error:advance:command_001",
      source: "advance",
      gate: null,
    });

    renderCockpit("session_001", false);
    await user.click(screen.getByRole("button", { name: "重试" }));

    expect(sendAdvance).toHaveBeenCalledWith("command_001");
  });

  it("routes a manual advance through workspace sendAdvance only after a confirmed gate", async () => {
    const sendAdvance = vi.fn(() => true);
    mockWorkspaceWs({ sendAdvance });
    useWorkspaceStore.setState({
      humanGateClosure: { decision: "confirm", stage: "human_confirm" },
    });
    renderCockpit("session_001", false);

    await userEvent.click(screen.getByRole("button", { name: "手动推进" }));
    expect(sendAdvance).toHaveBeenCalledTimes(1);
  });

  it("does not render or send manual advance without a confirmed advance state", () => {
    const sendAdvance = vi.fn(() => true);
    mockWorkspaceWs({ sendAdvance });
    renderCockpit("session_001", false);

    expect(screen.queryByRole("button", { name: "手动推进" })).toBeNull();
    expect(sendAdvance).not.toHaveBeenCalled();
  });

  it("跨会话批量：当前会话条目走既有 actions.confirm；其他会话条目进短命连接通道且不混入当前会话", async () => {
    const user = userEvent.setup();
    const sendHumanConfirm = vi.fn(() => true);
    mockWorkspaceWs({ sendHumanConfirm });
    const store = useWorkspaceStore.getState();
    store.applyHumanGateTurnOpen("g1", "confirm-g1", 1);
    store.rebuildChatEntries();
    cockpitInbox.push(gateItem("session_001", "g1"), gateItem("session_002", "g2"), stoppedItem("session_001"));

    renderCockpit("session_001", false);

    const gateChoices = screen.getAllByRole("checkbox", { name: "选择 门禁等待" });
    expect(gateChoices).toHaveLength(2);
    await user.click(gateChoices[0]);
    await user.click(gateChoices[1]);
    await user.click(screen.getByRole("button", { name: /批量确认/ }));

    expect(sendHumanConfirm).toHaveBeenCalledTimes(1);
    expect(sendHumanConfirm).toHaveBeenCalledWith("confirm");
    expect(bulkConfirmStart).toHaveBeenCalledTimes(1);
    expect(bulkConfirmStart).toHaveBeenCalledWith([
      {
        itemId: "session_002:gate:g2",
        sessionId: "session_002",
        gateKey: "g2",
        title: "门禁等待",
      },
    ]);
    expect(takeoverWorkspaceSession).not.toHaveBeenCalled();
  });

  it("confirms only while a current gate is open and does nothing without a gate", () => {
    const sendHumanConfirm = vi.fn(() => true);
    mockWorkspaceWs({ sendHumanConfirm });
    renderCockpit("session_001", false);
    const store = useWorkspaceStore.getState();

    for (const stage of ["running", "compile_plan", "completed"]) {
      useWorkspaceStore.setState({
        stage,
        humanGateClosure: null,
        humanGateSnapshot: null,
        humanGateTurn: null,
      });
      fireEvent.keyDown(document, { code: COCKPIT_HOTKEYS.confirm.code, ctrlKey: true });
    }
    expect(sendHumanConfirm).not.toHaveBeenCalled();

    store.setStage("human_confirm");
    fireEvent.keyDown(document, { code: COCKPIT_HOTKEYS.confirm.code, ctrlKey: true });

    expect(sendHumanConfirm).toHaveBeenCalledWith("confirm");

    store.applyHumanGateClosed("confirm", "human_confirm");
    fireEvent.keyDown(document, { code: COCKPIT_HOTKEYS.confirm.code, ctrlKey: true });
    expect(sendHumanConfirm).toHaveBeenCalledOnce();
  });

  it("does not dispatch cockpit hotkeys from gate feedback editors", () => {
    const sendHumanConfirm = vi.fn(() => true);
    mockWorkspaceWs({ sendHumanConfirm });
    const store = useWorkspaceStore.getState();
    useWorkspaceStore.setState({ flowKind: "single_candidate" });
    store.applyHumanGateTurnOpen("turn_1", "cmd_1", 1);
    store.rebuildChatEntries();
    renderCockpit("session_001", false);

    const feedbackEditors = screen.getAllByLabelText("门禁反馈");
    const gateCardInput = feedbackEditors.find((editor) => editor instanceof HTMLInputElement);
    if (!(gateCardInput instanceof HTMLInputElement)) {
      throw new Error("门卡反馈输入框缺失");
    }
    fireEvent.keyDown(gateCardInput, {
      code: COCKPIT_HOTKEYS.confirm.code,
      ctrlKey: true,
    });
    fireEvent.keyDown(
      within(screen.getByTestId("cockpit-inbox")).getByLabelText("门禁反馈"),
      { code: COCKPIT_HOTKEYS.confirm.code, ctrlKey: true },
    );

    expect(sendHumanConfirm).not.toHaveBeenCalled();
  });

  it("advances only from a confirmed state", () => {
    const sendAdvance = vi.fn(() => true);
    mockWorkspaceWs({ sendAdvance });
    renderCockpit("session_001", false);
    useWorkspaceStore.getState().setStage("running");

    fireEvent.keyDown(document, { code: COCKPIT_HOTKEYS.advance.code, ctrlKey: true });
    expect(sendAdvance).not.toHaveBeenCalled();

    useWorkspaceStore.setState({
      stage: "human_confirm",
      humanGateClosure: { decision: "confirm", stage: "human_confirm" },
    });
    fireEvent.keyDown(document, { code: COCKPIT_HOTKEYS.advance.code, ctrlKey: true });

    expect(sendAdvance).toHaveBeenCalledOnce();
  });

  it("blocks stale non-gate snapshots across card, hotkeys, bulk, and advance", () => {
    const sendHumanConfirm = vi.fn(() => true);
    const sendHumanGateFeedback = vi.fn(() => true);
    const sendAdvance = vi.fn(() => true);
    mockWorkspaceWs({ sendHumanConfirm, sendHumanGateFeedback, sendAdvance });
    useWorkspaceStore.setState({
      stage: "running",
      flowKind: "single_candidate",
      singleCandidatePhase: "generate",
      humanGateSnapshot: {
        findings: [],
        repeated_fingerprints: [],
        attempts_used: 1,
        manual_repairs_remaining: 1,
        trigger: "verification_new_findings",
        resumable: true,
      },
    });
    renderCockpit("session_001", false);

    expect(screen.getAllByText("已离开人工确认门").length).toBeGreaterThan(0);
    fireEvent.keyDown(document, { code: COCKPIT_HOTKEYS.confirm.code, ctrlKey: true });
    fireEvent.keyDown(document, { code: COCKPIT_HOTKEYS.advance.code, ctrlKey: true });
    expect(sendHumanConfirm).not.toHaveBeenCalled();
    expect(sendHumanGateFeedback).not.toHaveBeenCalled();
    expect(sendAdvance).not.toHaveBeenCalled();
  });

  it("blocks phase-mismatched gate actions from hotkeys", () => {
    const sendHumanConfirm = vi.fn(() => true);
    const sendHumanGateFeedback = vi.fn(() => true);
    const sendAdvance = vi.fn(() => true);
    mockWorkspaceWs({ sendHumanConfirm, sendHumanGateFeedback, sendAdvance });
    useWorkspaceStore.setState({
      stage: "human_confirm",
      flowKind: "single_candidate",
      singleCandidatePhase: "generate",
      humanGateSnapshot: {
        findings: [],
        repeated_fingerprints: [],
        attempts_used: 1,
        manual_repairs_remaining: 1,
        trigger: "verification_new_findings",
        resumable: true,
      },
    });
    renderCockpit("session_001", false);

    expect(screen.getAllByText("门相位与当前阶段不一致").length).toBeGreaterThan(0);
    fireEvent.keyDown(document, { code: COCKPIT_HOTKEYS.confirm.code, ctrlKey: true });
    fireEvent.keyDown(document, { code: COCKPIT_HOTKEYS.advance.code, ctrlKey: true });
    expect(sendHumanConfirm).not.toHaveBeenCalled();
    expect(sendHumanGateFeedback).not.toHaveBeenCalled();
    expect(sendAdvance).not.toHaveBeenCalled();
  });

  it.each([
    ["approval", "human_confirm"],
    ["evaluate", "human_confirm"],
    ["completed", "completed"],
  ] as const)("allows typed confirm for %s gate shape", (singleCandidatePhase, stage) => {
    const sendHumanConfirm = vi.fn(() => true);
    mockWorkspaceWs({ sendHumanConfirm });
    useWorkspaceStore.setState({
      stage,
      flowKind: "single_candidate",
      singleCandidatePhase,
      humanGateSnapshot: {
        findings: [],
        repeated_fingerprints: [],
        attempts_used: 1,
        manual_repairs_remaining: 1,
        trigger: "verification_new_findings",
        resumable: true,
      },
    });
    renderCockpit("session_001", false);

    fireEvent.keyDown(document, { code: COCKPIT_HOTKEYS.confirm.code, ctrlKey: true });

    expect(sendHumanConfirm).toHaveBeenCalledWith("confirm");
  });

  it("arms takeover on the first hotkey and calls the existing takeover only on the second", () => {
    vi.mocked(takeoverWorkspaceSession).mockResolvedValue({
      workspace_session_id: "child_001",
    } as TakeoverResponse);
    cockpitInbox.push(stoppedItem("session_001"));
    renderCockpit();

    fireEvent.keyDown(document, {
      code: COCKPIT_HOTKEYS.takeover.code,
      ctrlKey: true,
      shiftKey: true,
    });
    expect(takeoverWorkspaceSession).not.toHaveBeenCalled();
    expect(screen.getByRole("button", { name: "确认接管" })).toBeVisible();

    fireEvent.keyDown(document, {
      code: COCKPIT_HOTKEYS.takeover.code,
      ctrlKey: true,
      shiftKey: true,
    });
    expect(takeoverWorkspaceSession).toHaveBeenCalledWith("session_001");
  });
  it("watches and selects the child session after a successful takeover", async () => {
    const user = userEvent.setup();
    vi.mocked(takeoverWorkspaceSession).mockResolvedValue({
      workspace_session_id: "child_001",
    } as TakeoverResponse);
    cockpitInbox.push(stoppedItem("session_001"));
    cockpitObservedRecords.push({
      sessionId: "child_001",
      state: observerStateFromSessionState({
        type: "session_state",
        session_id: "child_001",
        workspace_type: "work_item",
        stage: "running",
        superpowers_enabled: false,
        openspec_enabled: false,
        messages: [{ id: "message_001", role: "author", content: "子会话对话", created_at: "2026-09-14T00:00:00Z" }],
        checkpoints: [],
        artifact: null,
        providers: { author: "claude_code", reviewer: null },
        timeline_nodes: [],
        active_node_id: null,
        artifact_versions: [],
        timeline_node_details: {},
        active_run_id: null,
        human_presentation_revisions: [],
        session_status: "running",
        flow_kind: "legacy",
        run_policy: "interactive",
        run_history: {
          seen_fingerprints: [], repairs_used: 0, manual_repairs_used: 0,
          transitions_used: 0, initial_review_count: 0, verification_review_count: 0,
        },
      }),
    });

    renderCockpit();
    await user.click(screen.getByRole("button", { name: "接管" }));
    await user.click(screen.getByRole("button", { name: "确认接管" }));

    expect(watchSession).toHaveBeenCalledWith("child_001");
    expect(await screen.findByText("子会话对话")).toBeVisible();
  });

  it("mirrors a successful takeover parent id and writes session-scoped storage", async () => {
    vi.mocked(takeoverWorkspaceSession).mockResolvedValue({
      workspace_session_id: "child_001",
      parent_session_id: "parent_001",
      takeover_event_id: "event_001",
    } as TakeoverResponse);
    cockpitInbox.push(stoppedItem("parent_001"));
    renderCockpit("parent_001");

    await userEvent.click(screen.getByRole("button", { name: "接管" }));
    await userEvent.click(screen.getByRole("button", { name: "确认接管" }));

    expect(useOperationAuditStore.getState().takeoverLinks.child_001).toEqual({
      parentSessionId: "parent_001",
      takeoverEventId: "event_001",
    });
    expect(sessionStorage.getItem("aria.takeover-parent:child_001")).toBe("parent_001");
  });

  it("renders the three cockpit zones", () => {
    renderCockpit();

    expect(screen.getByTestId("cockpit-page")).toBeInTheDocument();
    expect(screen.getByTestId("cockpit-inbox")).toBeInTheDocument();
    expect(screen.getByTestId("cockpit-execution-flow")).toBeInTheDocument();
    expect(screen.getByTestId("cockpit-conversation-flow")).toBeInTheDocument();
  });

  it("shows running state, timeline progress, and provider stream through existing cockpit zones", () => {
    useWorkspaceStore.setState({
      providerStatus: "running",
      stage: "running",
      activeNodeId: "author-1",
      timelineNodes: [timelineNode({ node_id: "author-1", title: "author-1", status: "active" })],
      chatEntries: [
        {
          id: "author-1:stream",
          type: "provider_stream",
          role: "author",
          content: "正在输出第一段",
          timestamp: "2026-09-13T00:00:00Z",
          node_id: "author-1",
        },
      ],
    });

    renderCockpitWith(mockWorkspaceWs());

    expect(screen.getByTestId("cockpit-generation-status")).toHaveTextContent("正在生成");
    expect(screen.getByTestId("cockpit-execution-flow")).toHaveTextContent("author-1");
    expect(screen.getByTestId("cockpit-conversation-flow")).toHaveTextContent("正在输出第一段");
  });

  it.each([
    ["failed", "生成失败"],
    ["completed", "生成完成"],
  ] as const)("describes a %s provider state without opening another progress view", (providerStatus, text) => {
    useWorkspaceStore.setState({ providerStatus, stage: "completed", chatEntries: [] });

    renderCockpitWith(mockWorkspaceWs());

    expect(screen.getByTestId("cockpit-generation-status")).toHaveTextContent(text);
    expect(screen.getByTestId("cockpit-conversation-flow")).toHaveTextContent("暂无聊天记录");
  });

describe("cockpit generation status identity and boundaries", () => {
  it("shows provider identity and elapsed time while a generation runs", () => {
    vi.useFakeTimers();
    try {
      vi.setSystemTime(new Date("2026-09-17T12:00:00Z"));
      const workspaceWs = mockWorkspaceWs();
      useWorkspaceStore.setState({
        providerStatus: "running",
        stage: "running",
        providers: { author: "pi", reviewer: "codex" } as const,
        activeNodeId: "author-1",
        timelineNodes: [
          timelineNode({
            node_id: "author-1",
            status: "active",
            started_at: new Date("2026-09-17T11:58:55Z").toISOString(),
          }),
        ],
      });

      renderCockpitWith(workspaceWs);

      const status = screen.getByTestId("cockpit-generation-status");
      expect(status).toHaveTextContent("正在生成");
      expect(status).toHaveTextContent("pi");
      expect(status).toHaveTextContent("1:05");
    } finally {
      vi.useRealTimers();
    }
  });

  it("labels the reviewer during cross review", () => {
    const workspaceWs = mockWorkspaceWs();
    useWorkspaceStore.setState({
      providerStatus: "running",
      stage: "cross_review",
      providers: { author: "pi", reviewer: "codex" } as const,
      activeNodeId: "reviewer-1",
      timelineNodes: [timelineNode({ node_id: "reviewer-1", stage: "cross_review" })],
    });

    renderCockpitWith(workspaceWs);

    expect(screen.getByTestId("cockpit-generation-status")).toHaveTextContent("codex");
  });

  it("omits provider and elapsed segments when their driving facts are absent", () => {
    const workspaceWs = mockWorkspaceWs();
    useWorkspaceStore.setState({
      providerStatus: "running",
      stage: "running",
      providers: null,
      activeNodeId: "author-1",
      timelineNodes: [timelineNode({ node_id: "author-1", started_at: "" })],
    });

    renderCockpitWith(workspaceWs);

    const status = screen.getByTestId("cockpit-generation-status");
    expect(status).toHaveTextContent("正在生成 · 运行中");
    expect(status).not.toHaveTextContent("undefined");
    expect(status).not.toHaveTextContent("NaN");
  });

  it("guides an empty unstarted session instead of reporting a fault", () => {
    const workspaceWs = mockWorkspaceWs();
    useWorkspaceStore.setState({
      providerStatus: "starting",
      stage: "prepare_context",
      timelineNodes: [],
    });

    renderCockpitWith(workspaceWs);

    expect(screen.getByTestId("cockpit-generation-status")).toHaveTextContent(
      "等待发起 · 选择 Provider 后点击「开始生成」",
    );
  });

  it.each([
    ["failed", "running", "生成失败"],
    ["completed", "completed", "生成完成"],
  ] as const)("marks %s provider state visibly", (providerStatus, stage, label) => {
    const workspaceWs = mockWorkspaceWs();
    useWorkspaceStore.setState({ providerStatus, stage, chatEntries: [] });

    renderCockpitWith(workspaceWs);

    const status = screen.getByTestId("cockpit-generation-status");
    expect(status).toHaveTextContent(label);
    expect(status).not.toHaveTextContent("正在生成");
  });
});

  it("shows a successful takeover through the shared audit view", async () => {
    useWorkspaceStore.setState({ stage: "running" });
    const user = userEvent.setup();
    vi.mocked(takeoverWorkspaceSession).mockResolvedValue({
      workspace_session_id: "child_001",
      parent_session_id: "parent_001",
      takeover_event_id: "event_001",
    } as TakeoverResponse);
    cockpitInbox.push(stoppedItem("parent_001"));
    renderCockpit("parent_001");

    await user.click(screen.getByRole("button", { name: "接管" }));
    await user.click(screen.getByRole("button", { name: "确认接管" }));
    await user.click(screen.getByRole("button", { name: "操作审计" }));

    expect(screen.getByTestId("operation-audit-view")).toHaveTextContent("takeover");
    expect(screen.getByTestId("operation-audit-view")).toHaveTextContent("child_001");
  });

  it("projects an opened typed gate into the inbox with trigger and budget", () => {
    const store = useWorkspaceStore.getState();
    store.applyHumanGateTurnOpen("turn_1", "cmd_1", 1);
    store.setHumanGateSnapshot({
      findings: [],
      repeated_fingerprints: [],
      attempts_used: 1,
      manual_repairs_remaining: 1,
      trigger: "verification_new_findings",
      resumable: true,
    });
    store.rebuildChatEntries();

    renderCockpit();

    const inbox = screen.getByTestId("cockpit-inbox");
    expect(within(inbox).getByText("门禁等待")).toBeInTheDocument();
    expect(within(inbox).getByText(/复验发现新问题/)).toBeInTheDocument();
    expect(within(inbox).getByText(/剩余修复轮次 1/)).toBeInTheDocument();
  });

  it("renders one observed gate through inbox and action card", () => {
    const store = useWorkspaceStore.getState();
    store.applyHumanGateTurnOpen("turn_gate", "cmd_gate", 1);
    store.rebuildChatEntries();

    renderCockpit();

    expect(screen.getByTestId("cockpit-inbox-item-gate")).toBeVisible();
    expect(screen.getByTestId("gate-prompt-entry")).toBeVisible();
  });

  it("exposes no input controls in the execution flow zone", () => {
    useWorkspaceStore.getState().setTimelineNodesForTest([timelineNode()]);

    renderCockpit();

    const flow = screen.getByTestId("cockpit-execution-flow");
    expect(flow.querySelectorAll("input, select, textarea")).toHaveLength(0);
    expect(within(flow).queryAllByRole("textbox")).toHaveLength(0);
  });

  it("exposes gate actions in the cockpit", () => {
    const store = useWorkspaceStore.getState();
    store.setStage("human_confirm");
    store.appendChatEntry({
      id: "human_confirm:gate-prompt",
      type: "gate_prompt",
      role: "system",
      content: "等待人工确认",
      timestamp: "2026-09-13T00:00:00Z",
      metadata: { action_facade: "legacy" },
    });

    renderCockpit();

    const gateEntry = screen.getByTestId("gate-prompt-entry");
    expect(gateEntry).toBeInTheDocument();
    expect(within(gateEntry).getByRole("button", { name: "确认产物" })).toBeVisible();
    expect(within(gateEntry).getByRole("button", { name: "终止" })).toBeVisible();
  });
  it("sends plan gate confirmation from the 确认产物 button", async () => {
    const sendHumanConfirm = vi.fn(() => true);
    mockWorkspaceWs({ sendHumanConfirm });
    const store = useWorkspaceStore.getState();
    store.setStage("human_confirm");
    useWorkspaceStore.setState({
      flowKind: "single_candidate",
      singleCandidatePhase: "approval",
    });
    store.applyHumanGateTurnOpen("turn_confirm", "cmd_confirm", 1);
    store.rebuildChatEntries();

    renderCockpit("session_001", false);

    await userEvent.click(screen.getByRole("button", { name: "确认产物" }));

    expect(sendHumanConfirm).toHaveBeenCalledOnce();
    expect(sendHumanConfirm).toHaveBeenCalledWith("confirm");
  });

  it("dispatches typed gate feedback through the typed websocket helper", async () => {
    const feedback = vi.fn(() => true);
    const workspaceWs = mockWorkspaceWs({ sendHumanGateFeedback: feedback });
    const user = userEvent.setup();
    const store = useWorkspaceStore.getState();
    store.setSessionIdForTest("session_001");
    useWorkspaceStore.setState({ flowKind: "single_candidate" });
    store.applyHumanGateTurnOpen("turn_1", "cmd_1", 1);
    store.rebuildChatEntries();

    render(
      <ChatCockpitPage
        sessionId="session_001"
        onBack={vi.fn()}
        onOpenSession={vi.fn()}
        workspaceWs={workspaceWs}
      />,
    );
    const gateEntry = screen.getByTestId("gate-prompt-entry");
    const submit = within(gateEntry).getByRole("button", { name: "提交反馈" });
    expect(submit).toBeDisabled();
    await user.type(within(gateEntry).getByLabelText("门禁反馈"), "请补齐边界");
    await user.click(submit);

    expect(feedback).toHaveBeenCalledWith("请补齐边界", "cmd_1");
  });

  it("edits typed inbox feedback before dispatching it", async () => {
    const user = userEvent.setup();
    const feedback = vi.fn(() => true);
    mockWorkspaceWs({ sendHumanGateFeedback: feedback });
    const store = useWorkspaceStore.getState();
    useWorkspaceStore.setState({ flowKind: "single_candidate" });
    store.applyHumanGateTurnOpen("turn_1", "cmd_1", 1);
    store.rebuildChatEntries();

    renderCockpit("session_001", false);
    const feedbackInput = within(screen.getByTestId("cockpit-inbox")).getByLabelText("门禁反馈");
    const submit = within(screen.getByTestId("cockpit-inbox")).getByRole("button", {
      name: "提交反馈",
    });
    await user.type(feedbackInput, "请补齐边界");
    await user.click(submit);

    expect(feedback).toHaveBeenCalledWith("请补齐边界", "cmd_1");
  });
  it("renders the current artifact and review summary in the actionable inbox gate", () => {
    useWorkspaceStore.setState({
      artifactVersions: [
        {
          version: 3,
          markdown: "# 发布方案 v3\n\n- 修复 Issue 索引\n- 补齐流式渲染",
          generated_by: "claude_code",
          reviewed_by: "codex",
          review_verdict: "pass",
          confirmed_by: null,
          is_current: true,
          created_at: "2026-09-17T10:00:00Z",
          source_node_id: "node-artifact",
        },
      ],
      chatEntries: [
        {
          id: "review-1",
          type: "review_verdict",
          role: "reviewer",
          content: "审核通过，允许人工确认",
          timestamp: "2026-09-17T10:00:00Z",
        },
      ],
    });

    renderCockpit();

    const inbox = screen.getByTestId("cockpit-inbox");
    expect(within(inbox).getByText("等待确认的内容")).toBeVisible();
    expect(within(inbox).getByText("发布方案 v3")).toBeVisible();
    expect(within(inbox).getByText("审核通过，允许人工确认")).toBeVisible();
  });

  it("renders typed snapshot inbox guidance when the active repair reservation needs a new command", async () => {
    const user = userEvent.setup();
    const feedback = vi.fn((_feedback: string, _commandId?: string) => true);
    mockWorkspaceWs({ sendHumanGateFeedback: feedback });
    useWorkspaceStore.setState({
      flowKind: "single_candidate",
      humanGateSnapshot: {
        findings: [],
        repeated_fingerprints: [],
        attempts_used: 1,
        manual_repairs_remaining: 1,
        trigger: "verification_new_findings",
        resumable: true,
      },
      repairReservation: {
        token: "reservation-1",
        owner_session_id: "session_001",
        owner_run_id: "run-1",
        provider_start_idempotency_key: "start-1",
        state: "reserved",
        commit_id: null,
      },
    });

    renderCockpit("session_001", false);

    const inbox = screen.getByTestId("cockpit-inbox");
    expect(within(inbox).getByText("未同步门命令，将以新命令提交")).toBeVisible();
    expect(within(inbox).queryByRole("button", { name: "采纳建议并返修" })).toBeNull();
    const submit = within(inbox).getByRole("button", { name: "提交反馈" });
    await user.type(within(inbox).getByLabelText("门禁反馈"), "请补齐边界");
    await user.click(submit);

    expect(feedback).toHaveBeenCalledTimes(1);
    const [feedbackText, commandId] = feedback.mock.calls[0];
    expect(feedbackText).toBe("请补齐边界");
    expect(commandId).toMatch(
      /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/,
    );
  });

  it("keeps typed snapshot inbox guidance hidden without a repair reservation", async () => {
    const user = userEvent.setup();
    const feedback = vi.fn((_feedback: string, _commandId?: string) => true);
    mockWorkspaceWs({ sendHumanGateFeedback: feedback });
    useWorkspaceStore.setState({
      flowKind: "single_candidate",
      humanGateSnapshot: {
        findings: [],
        repeated_fingerprints: [],
        attempts_used: 1,
        manual_repairs_remaining: 1,
        trigger: "verification_new_findings",
        resumable: true,
      },
      repairReservation: null,
    });

    renderCockpit("session_001", false);

    const inbox = screen.getByTestId("cockpit-inbox");
    expect(within(inbox).queryByText("未同步门命令，将以新命令提交")).toBeNull();
    const submit = within(inbox).getByRole("button", { name: "提交反馈" });
    await user.type(within(inbox).getByLabelText("门禁反馈"), "请补齐边界");
    await user.click(submit);

    expect(feedback).toHaveBeenCalledTimes(1);
  });

  it("uses the shared shell inbox instead of a second session observer", () => {
    cockpitInbox.push({
      id: "other:gate",
      kind: "gate",
      severity: 3,
      title: "全局待处理",
      summary: "来自根壳观察",
      triage: false,
      source: "gate",
      createdAt: null,
      gate: null,
      inlineError: null,
    });

    renderCockpit();

    expect(within(screen.getByTestId("cockpit-inbox")).getByText("全局待处理")).toBeInTheDocument();
  });

  it("projects stopped sessions and protocol errors as inbox items", () => {
    const store = useWorkspaceStore.getState();
    store.setSessionStatus("stopped_needs_human");
    store.setProtocolError({ code: "PROTOCOL_X", message: "协议错误原文" });

    renderCockpit();

    const inbox = screen.getByTestId("cockpit-inbox");
    expect(within(inbox).getByText("会话停在停点")).toBeInTheDocument();
    expect(within(inbox).getByText("协议错误 PROTOCOL_X")).toBeInTheDocument();
    expect(within(inbox).getByText("协议错误原文")).toBeInTheDocument();
  });

  it("exposes protocol diagnostic count beside the execution-flow title", async () => {
    const user = userEvent.setup();
    useWorkspaceStore.getState().setTimelineNodesForTest([timelineNode({ node_id: "node_diagnostic" })]);
    useWorkspaceStore.getState().setActiveNodeId("node_diagnostic");
    useWorkspaceStore.getState().recordProtocolDiagnostic({
      code: "UNRECOGNIZED_EVENT",
      message: "未知事件",
      at: "2026-09-14T00:00:00.000Z",
      type: "future_event",
    });

    renderCockpit();

    const diagnosticCount = screen.getByTestId("cockpit-protocol-diagnostic-count");
    expect(diagnosticCount).toHaveTextContent("1");
    await user.click(diagnosticCount);
    expect(screen.getByTestId("timeline-node-author_run")).toHaveAttribute("aria-current", "step");
  });

  it("removes a closed gate from the inbox", () => {
    useWorkspaceStore.getState().setStage("human_confirm");
    renderCockpit();
    expect(within(screen.getByTestId("cockpit-inbox")).getByText("门禁等待")).toBeInTheDocument();

    act(() => {
      useWorkspaceStore.getState().applyHumanGateClosed("confirm", "human_confirm");
    });

    expect(within(screen.getByTestId("cockpit-inbox")).queryByText("门禁等待")).toBeNull();
  });

  it("renders triage as a single gate item marked as triage", () => {
    const store = useWorkspaceStore.getState();
    store.setStage("human_confirm");
    store.appendChatEntry({
      id: "review_verdict:node-1",
      type: "review_verdict",
      role: "reviewer",
      content: "review",
      timestamp: "2026-09-13T00:00:00Z",
      metadata: { review_gate: "user_triage_required" },
    });

    renderCockpit();

    const inbox = screen.getByTestId("cockpit-inbox");
    expect(within(inbox).getAllByText(/门禁等待/)).toHaveLength(1);
    expect(within(inbox).getByText("门禁等待（需分诊）")).toBeInTheDocument();
  });

  it("drills down into the conversation flow when a flow row is clicked", async () => {
    const user = userEvent.setup();
    const scrollIntoView = vi.fn();
    Object.defineProperty(HTMLElement.prototype, "scrollIntoView", {
      configurable: true,
      value: scrollIntoView,
    });
    const store = useWorkspaceStore.getState();
    store.setTimelineNodesForTest([timelineNode()]);
    store.appendChatEntry({
      id: "node-1:stream",
      type: "provider_stream",
      role: "author",
      content: "作者输出",
      timestamp: "2026-09-13T00:00:00Z",
      node_id: "node-1",
    });

    renderCockpit();

    await user.click(screen.getByTestId("timeline-node-author_run"));

    expect(screen.getByTestId("timeline-node-author_run")).toHaveAttribute("aria-current", "step");
    expect(scrollIntoView).toHaveBeenCalled();
  });

  it("ticks the elapsed time so silent rows stay honest", () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-09-13T00:00:30Z"));
    useWorkspaceStore.getState().setTimelineNodesForTest([timelineNode()]);

    renderCockpit();

    expect(screen.getByTestId("flow-elapsed-author_run")).toHaveTextContent("30s");

    act(() => {
      vi.advanceTimersByTime(1000);
    });

    expect(screen.getByTestId("flow-elapsed-author_run")).toHaveTextContent("31s");
    vi.useRealTimers();
  });

  describe("plan approval view (Phase 3)", () => {
    function planProjectionBundleFixture(): PlanProjectionBundle {
      return {
        id: "bundle_0001",
        plan_revision_id: "plan_rev_0002",
        dependency_graph_revision_id: "graph_rev_0001",
        work_item_projection_bundle_refs: [],
        human_group_projection: {
          plan_id: "plan_0001",
          goal: "g",
          split_reason: "s",
          work_items: [],
          contract_flow: [
            {
              from: "wi_backend",
              to: "wi_frontend",
              contract_id: "contract_metrics",
              required_capabilities: ["cap_export_csv", "cap_export_json"],
              provided_capabilities: ["cap_export_csv"],
              missing_capabilities: ["cap_export_json"],
            },
          ],
          risks: [],
          source_refs: [],
          normative: false,
          used_by_provider: false,
        },
        coder_group_context: {
          plan_id: "plan_0001",
          ordered_logical_work_item_ids: [],
          dependency_edges: [],
          group_write_scopes: {},
        },
        reviewer_group_matrix: {
          plan_id: "plan_0001",
          work_items: [],
          dependency_edges: [],
          design_traceability_refs: [],
        },
        human_group_projection_hash: "h1",
        coder_group_context_hash: "h2",
        reviewer_group_matrix_hash: "h3",
        compiler_version: "v1",
        created_at: "2026-09-14T08:00:00Z",
      };
    }

    function enterPlanSession() {
      useWorkspaceStore.getState().setSessionState({
        session_id: "session_001",
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
        artifact: null,
        providers: { author: "fake", reviewer: null },
      });
    }

    it("shows the plan approval tab only for work_item_plan sessions", () => {
      renderCockpit();
      expect(screen.queryByTestId("cockpit-plan-approval-tab")).toBeNull();

      // store 更新发生在渲染之后，需要 act 刷新 zustand 驱动的重渲染。
      act(() => {
        enterPlanSession();
      });
      expect(screen.getByTestId("cockpit-plan-approval-tab")).toBeVisible();
    });

    it("swaps the drilldown zone to the plan approval panel and back", async () => {
      const user = userEvent.setup();
      enterPlanSession();
      renderCockpit();

      await user.click(screen.getByTestId("cockpit-plan-approval-tab"));
      expect(screen.getByTestId("cockpit-plan-approval-panel")).toBeVisible();
      expect(screen.queryByTestId("cockpit-conversation-flow-list")).toBeNull();

      await user.click(screen.getByTestId("cockpit-conversation-tab"));
      expect(screen.getByTestId("cockpit-conversation-flow-list")).toBeVisible();
      expect(screen.queryByTestId("cockpit-plan-approval-panel")).toBeNull();
    });

    it("renders the revision diff summary after loading two artifact rounds", async () => {
      const user = userEvent.setup();
      enterPlanSession();
      useWorkspaceStore.setState({
        artifactVersions: [
          {
            version: 1,
            markdown: undefined,
            generated_by: "fake",
            reviewed_by: null,
            review_verdict: null,
            confirmed_by: null,
            is_current: false,
            created_at: "2026-09-14T08:00:00Z",
            source_node_id: "node_1",
          },
          {
            version: 2,
            markdown: undefined,
            generated_by: "fake",
            reviewed_by: null,
            review_verdict: null,
            confirmed_by: null,
            is_current: true,
            created_at: "2026-09-14T09:00:00Z",
            source_node_id: "node_2",
          },
        ],
      });
      // beforeEach 只 clearAllMocks（不清 once 队列），这里显式重置避免吞前一测的残留实现。
      vi.mocked(fetchWorkspaceArtifactVersion).mockReset();
      vi.mocked(fetchWorkspaceArtifactVersion)
        .mockResolvedValueOnce({
          version: 1,
          markdown: "# 计划\n目标 A\n",
        })
        .mockResolvedValueOnce({
          version: 2,
          markdown: "# 计划\n目标 B\n",
        });

      renderCockpit();
      await user.click(screen.getByTestId("cockpit-plan-approval-tab"));
      await user.click(screen.getByTestId("plan-approval-tab-diff"));

      const summary = await screen.findByTestId("revision-diff-summary");
      expect(summary).toHaveTextContent("~1");
      expect(vi.mocked(fetchWorkspaceArtifactVersion)).toHaveBeenCalledWith("session_001", 1);
      expect(vi.mocked(fetchWorkspaceArtifactVersion)).toHaveBeenCalledWith("session_001", 2);
    });

    it("lists checklist gaps and jumps back to the gate entry finding", async () => {
      const user = userEvent.setup();
      const scrollIntoView = vi.fn();
      Object.defineProperty(HTMLElement.prototype, "scrollIntoView", {
        configurable: true,
        value: scrollIntoView,
      });
      enterPlanSession();
      useWorkspaceStore.setState({
        workItemPlanProjectionArtifacts: {
          planProjection: planProjectionBundleFixture(),
          workItemProjections: [],
          history: null,
          validation: null,
          missingWorkItemProjectionRefs: [],
        },
        chatEntries: [
          {
            id: "node_gate:gate-prompt",
            type: "gate_prompt",
            role: "system",
            content: "等待人工确认",
            timestamp: "2026-09-14T08:00:00Z",
            metadata: {
              findings: [
                {
                  class: "repairable",
                  fingerprint: "fp",
                  category: null,
                  severity: "major",
                  message: "契约缺口",
                  evidence: null,
                  required_action: null,
                  contract_field: "contract_metrics",
                },
              ],
            },
          },
        ],
      });

      renderCockpit();
      await user.click(screen.getByTestId("cockpit-plan-approval-tab"));
      await user.click(screen.getByTestId("plan-approval-tab-checklist"));

      expect(screen.getByTestId("contract-checklist-gap-count")).toHaveTextContent(
        "缺口 1 条 / 共 2 条",
      );
      await user.click(screen.getByRole("button", { name: "查看 finding" }));

      expect(screen.getByTestId("cockpit-conversation-flow-list")).toBeVisible();
      expect(screen.queryByTestId("cockpit-plan-approval-panel")).toBeNull();
      // 滚动断言（裁决补充）：跳转回对话流后落点滚动确实发生在 gate 条目上。
      const list = screen.getByTestId("cockpit-conversation-flow-list");
      expect(within(list).getByText("等待人工确认")).toBeVisible();
      expect(scrollIntoView).toHaveBeenCalled();
    });

    it("renders the read-only plan repair panel for the child session", async () => {
      const user = userEvent.setup();
      enterPlanSession();
      useWorkspaceStore.setState({
        planRepair: planRepairSnapshotFixture("session_001"),
      });

      renderCockpit();
      await user.click(screen.getByTestId("cockpit-plan-approval-tab"));
      await user.click(screen.getByTestId("plan-approval-tab-repair"));

      const panel = screen.getByTestId("plan-repair-read-only-panel");
      expect(panel).toHaveTextContent("修订编写中");
      expect(panel).toHaveTextContent("CONTRACT_CAPABILITY_MISSING");
      expect(
        within(panel).queryAllByRole("button"),
      ).toHaveLength(0);
    });

    it("renders the plan approval view for a takeover observed session without a white screen", async () => {
      const user = userEvent.setup();
      vi.mocked(takeoverWorkspaceSession).mockResolvedValue({
        workspace_session_id: "child_plan_001",
      } as TakeoverResponse);
      cockpitInbox.push(stoppedItem("session_001"));
      cockpitObservedRecords.push({
        sessionId: "child_plan_001",
        state: observerStateFromSessionState({
          type: "session_state",
          session_id: "child_plan_001",
          workspace_type: "work_item_plan",
          stage: "human_confirm",
          superpowers_enabled: false,
          openspec_enabled: false,
          messages: [],
          checkpoints: [],
          artifact: null,
          providers: { author: "claude_code", reviewer: null },
          timeline_nodes: [],
          active_node_id: null,
          artifact_versions: [],
          timeline_node_details: {},
          active_run_id: null,
          human_presentation_revisions: [],
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
        }),
      });

      renderCockpit();
      await user.click(screen.getByRole("button", { name: "接管" }));
      await user.click(screen.getByRole("button", { name: "确认接管" }));

      await user.click(await screen.findByTestId("cockpit-plan-approval-tab"));
      expect(screen.getByTestId("cockpit-plan-approval-panel")).toBeVisible();
      expect(screen.getByTestId("revision-diff-view")).toHaveTextContent(
        "当前会话只有一个 artifact 轮次，暂无对比对象。",
      );

      await user.click(screen.getByTestId("plan-approval-tab-checklist"));
      expect(screen.getByTestId("contract-checklist-view")).toHaveTextContent(
        "当前计划没有 capability / 契约条目",
      );
    });

    it("loads the observed child session's own artifact rounds instead of the parent cache", async () => {
      const user = userEvent.setup();
      vi.mocked(takeoverWorkspaceSession).mockResolvedValue({
        workspace_session_id: "child_plan_002",
      } as TakeoverResponse);
      cockpitInbox.push(stoppedItem("session_001"));
      // 操作者先在自己会话看过 v1/v2 轮次：主 store 缓存已持有主会话 markdown。
      const store = useWorkspaceStore.getState();
      store.setArtifactContentCacheEntry(1, "# 主会话\nAAA\n");
      store.setArtifactContentCacheEntry(2, "# 主会话\nBBB\n");
      cockpitObservedRecords.push({
        sessionId: "child_plan_002",
        state: observerStateFromSessionState({
          type: "session_state",
          session_id: "child_plan_002",
          workspace_type: "work_item_plan",
          stage: "human_confirm",
          superpowers_enabled: false,
          openspec_enabled: false,
          messages: [],
          checkpoints: [],
          artifact: null,
          providers: { author: "claude_code", reviewer: null },
          timeline_nodes: [],
          active_node_id: null,
          artifact_versions: [],
          artifact_version_summaries: [
            {
              version: 1,
              generated_by: "claude_code",
              created_at: "2026-09-14T08:00:00Z",
              source_node_id: "node_1",
            },
            {
              version: 2,
              generated_by: "claude_code",
              created_at: "2026-09-14T09:00:00Z",
              source_node_id: "node_2",
              is_current: true,
            },
          ],
          timeline_node_details: {},
          active_run_id: null,
          human_presentation_revisions: [],
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
        }),
      });
      vi.mocked(fetchWorkspaceArtifactVersion).mockReset();
      vi.mocked(fetchWorkspaceArtifactVersion)
        .mockResolvedValueOnce({ version: 1, markdown: "# 子会话\nCCC\n" })
        .mockResolvedValueOnce({ version: 2, markdown: "# 子会话\nDDD\n" });

      renderCockpit();
      await user.click(screen.getByRole("button", { name: "接管" }));
      await user.click(screen.getByRole("button", { name: "确认接管" }));
      await user.click(await screen.findByTestId("cockpit-plan-approval-tab"));

      const summary = await screen.findByTestId("revision-diff-summary");
      expect(summary).toHaveTextContent("基准 v1 → 目标 v2");
      // 防串显：渲染内容来自子会话 fetch，主会话缓存不得泄入。
      expect(screen.getByTestId("revision-diff-hunk")).toHaveTextContent("DDD");
      expect(screen.queryAllByText(/主会话/)).toHaveLength(0);
      expect(vi.mocked(fetchWorkspaceArtifactVersion)).toHaveBeenCalledWith("child_plan_002", 1);
      expect(vi.mocked(fetchWorkspaceArtifactVersion)).toHaveBeenCalledWith("child_plan_002", 2);
    });
  });
  describe("cockpit author confirm and review decision", () => {
    function artifactVersionSummaryFixture() {
      return {
        version: 1,
        markdown: "# 设计稿",
        generated_by: "claude_code" as const,
        reviewed_by: null,
        review_verdict: null,
        confirmed_by: null,
        is_current: true,
        created_at: "2026-09-17T10:00:00Z",
        source_node_id: "node-artifact",
      };
    }

    it("auto-switches a story session to the artifact tab at author confirm and finalizes", async () => {
      const user = userEvent.setup();
      const sendAuthorDecision = vi.fn();
      const workspaceWs = mockWorkspaceWs({ sendAuthorDecision });
      useWorkspaceStore.setState({
        stage: "author_confirm",
        workspaceType: "design",
        providers: { author: "pi", reviewer: "codex" },
        reviewerEnabled: true,
        artifactVersions: [artifactVersionSummaryFixture()],
        artifact: "# 设计稿",
      });

      renderCockpitWith(workspaceWs);

      const tab = screen.getByTestId("cockpit-artifact-review-tab");
      expect(tab).toHaveAttribute("aria-selected", "true");
      expect(screen.getByText("# 设计稿")).toBeVisible();

      await user.click(screen.getByRole("button", { name: "确认定稿" }));

      expect(sendAuthorDecision).toHaveBeenCalledWith("accept_finalize");
    });

    it("sends accept-with-review from the artifact tab", async () => {
      const user = userEvent.setup();
      const sendAuthorDecision = vi.fn();
      const workspaceWs = mockWorkspaceWs({ sendAuthorDecision });
      useWorkspaceStore.setState({
        stage: "author_confirm",
        workspaceType: "story",
        providers: { author: "pi", reviewer: "codex" },
        reviewerEnabled: true,
        artifactVersions: [artifactVersionSummaryFixture()],
        artifact: "# 用户故事",
      });

      renderCockpitWith(workspaceWs);
      await user.click(screen.getByRole("button", { name: "确认并送审" }));

      expect(sendAuthorDecision).toHaveBeenCalledWith("accept_with_review");
    });

    it("sends revision feedback from the input bar at author confirm", async () => {
      const user = userEvent.setup();
      const sendAuthorDecision = vi.fn();
      const workspaceWs = mockWorkspaceWs({ sendAuthorDecision });
      useWorkspaceStore.setState({ stage: "author_confirm", workspaceType: "story" });

      renderCockpitWith(workspaceWs);
      await user.click(screen.getByTestId("cockpit-conversation-tab"));
      await user.type(screen.getByTestId("context-note-input"), "第二段改成对话体");
      await user.click(screen.getByRole("button", { name: "发送反馈" }));

      expect(sendAuthorDecision).toHaveBeenCalledWith("revise", "第二段改成对话体");
    });

    it("prefills review findings into the input bar via the shared prefill handle", async () => {
      const user = userEvent.setup();
      const workspaceWs = mockWorkspaceWs();
      useWorkspaceStore.setState({
        stage: "author_confirm",
        workspaceType: "story",
        chatEntries: [
          {
            id: "review-1",
            type: "review_verdict",
            role: "reviewer",
            content: "第二段缺少冲突",
            timestamp: "2026-09-17T10:00:00Z",
          },
        ],
      });

      renderCockpitWith(workspaceWs);
      await user.click(screen.getByRole("button", { name: "采纳 Review 意见" }));

      expect(screen.getByTestId("cockpit-conversation-tab")).toHaveAttribute(
        "aria-selected",
        "true",
      );
      expect(screen.getByTestId("context-note-input")).toHaveValue(
        "按以下 review 意见修订：\n\n第二段缺少冲突",
      );
    });

    it("keeps the conversation tab for non story/design sessions", () => {
      useWorkspaceStore.setState({ stage: "author_confirm", workspaceType: "work_item_plan" });

      renderCockpitWith(mockWorkspaceWs());

      expect(screen.queryByTestId("cockpit-artifact-review-tab")).toBeNull();
      expect(screen.getByTestId("cockpit-conversation-tab")).toBeVisible();
    });

    it("keeps finalize actions hidden while observing another session at author confirm", async () => {
      const user = userEvent.setup();
      vi.mocked(takeoverWorkspaceSession).mockResolvedValue({
        workspace_session_id: "child_story_001",
      } as TakeoverResponse);
      cockpitInbox.push(stoppedItem("session_001"));
      cockpitObservedRecords.push({
        sessionId: "child_story_001",
        state: observerStateFromSessionState({
          type: "session_state",
          session_id: "child_story_001",
          workspace_type: "story",
          stage: "author_confirm",
          superpowers_enabled: false,
          openspec_enabled: false,
          messages: [],
          checkpoints: [],
          artifact: null,
          providers: { author: "claude_code", reviewer: null },
          timeline_nodes: [],
          active_node_id: null,
          artifact_versions: [],
          timeline_node_details: {},
          active_run_id: null,
          human_presentation_revisions: [],
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
        }),
      });
      useWorkspaceStore.setState({
        stage: "author_confirm",
        workspaceType: "design",
        providers: { author: "pi", reviewer: "codex" },
        reviewerEnabled: true,
      });

      renderCockpit();
      await user.click(screen.getByRole("button", { name: "接管" }));
      await user.click(screen.getByRole("button", { name: "确认接管" }));

      expect(await screen.findByText(/Author：claude_code/)).toBeVisible();
      expect(screen.getByTestId("cockpit-artifact-review-tab")).toHaveAttribute(
        "aria-selected",
        "true",
      );
      expect(screen.queryByRole("button", { name: "确认定稿" })).toBeNull();
      expect(screen.queryByRole("button", { name: "确认并送审" })).toBeNull();
    });

    it("exposes review decision actions at review decision stage", async () => {
      const user = userEvent.setup();
      const sendSelectRevisionPath = vi.fn();
      const workspaceWs = mockWorkspaceWs({ sendSelectRevisionPath });
      useWorkspaceStore.setState({ stage: "review_decision", workspaceType: "story" });

      renderCockpitWith(workspaceWs);
      await user.click(screen.getByRole("button", { name: "接受修订建议" }));

      expect(sendSelectRevisionPath).toHaveBeenCalledWith("revise", undefined);
    });

    it("offers optional work item plan finding decisions through the shared options helper", async () => {
      const user = userEvent.setup();
      const sendReviewDecision = vi.fn();
      const workspaceWs = mockWorkspaceWs({ sendReviewDecision });
      useWorkspaceStore.setState({
        stage: "review_decision",
        workspaceType: "work_item_plan",
        chatEntries: [
          {
            id: "review-optional-1",
            type: "review_verdict",
            role: "reviewer",
            content: "仅有可选建议",
            timestamp: "2026-09-17T10:00:00Z",
            metadata: {
              verdict: "pass",
              review_gate: "user_confirm_allowed",
              findings: [
                {
                  severity: "suggestion",
                  message: "补充 handoff",
                  evidence: "当前 handoff 说明过短",
                  required_action: "补充上下游交接说明",
                },
              ],
            },
          },
        ],
      });

      renderCockpitWith(workspaceWs);
      await user.click(screen.getByRole("button", { name: "修复这些建议" }));

      expect(sendReviewDecision).toHaveBeenCalledWith("apply_optional_findings");
    });

    it("does not render ChatInputBar during human confirm", () => {
      useWorkspaceStore.setState({ stage: "human_confirm", workspaceType: "story" });

      renderCockpitWith(mockWorkspaceWs());

      expect(screen.queryByTestId("context-note-input")).toBeNull();
    });
  });

  it("surfaces a recoverable interrupted run and retries it through the ws api", async () => {
    const user = userEvent.setup();
    const retryInterruptedRun = vi.fn(() => true);
    const workspaceWs = mockWorkspaceWs({ retryInterruptedRun });
    useWorkspaceStore.setState({
      stage: "prepare_context",
      recoverableInterruptedRun: {
        failed_node_id: "node-9",
        operation: "review",
        label: "恢复审核运行",
      },
    });

    renderCockpitWith(workspaceWs);

    expect(screen.getByText("检测到可恢复的中断任务")).toBeVisible();
    expect(screen.queryByTestId("start-generation")).toBeNull();

    await user.click(screen.getByRole("button", { name: "恢复审核运行" }));

    expect(retryInterruptedRun).toHaveBeenCalledWith("node-9");
  });

  it("acknowledges an aborted-by-disconnect node and drills down to it", async () => {
    const user = userEvent.setup();
    const workspaceWs = mockWorkspaceWs();
    const startedAt = new Date("2026-09-17T10:00:00Z").toISOString();
    useWorkspaceStore.setState({
      stage: "prepare_context",
      timelineNodes: [
        timelineNode({
          node_id: "node-aborted",
          node_type: "aborted_by_disconnect",
          title: "作者生成",
          status: "failed",
          started_at: startedAt,
          completed_at: startedAt,
        }),
      ],
    });

    renderCockpitWith(workspaceWs);

    expect(screen.getByText(/上次运行因断开被中止/)).toBeVisible();

    await user.click(screen.getByRole("button", { name: "查看 Timeline" }));
    expect(screen.getByTestId("timeline-node-aborted_by_disconnect")).toHaveAttribute(
      "aria-current",
      "step",
    );

    await user.click(screen.getByRole("button", { name: "我知道了" }));
    expect(screen.queryByText(/上次运行因断开被中止/)).toBeNull();
    expect(useWorkspaceStore.getState().acknowledgedAbortedNodes).toContain("node-aborted");
  });

  it("shows the reconnecting banner with manual retry while reconnecting", () => {
    const workspaceWs = mockWorkspaceWs({
      connectionStatus: "disconnected",
      isReconnecting: true,
      reconnectAttemptCount: 2,
    });
    useWorkspaceStore.setState({ stage: "prepare_context" });

    renderCockpitWith(workspaceWs);

    expect(screen.getByText(/连接断开，重连中（尝试 2 次）/)).toBeVisible();
  });

  it("arms the unload guard only during provider running stages", () => {
    const workspaceWs = mockWorkspaceWs();

    useWorkspaceStore.setState({ stage: "prepare_context" });
    const { unmount } = renderCockpitWith(workspaceWs);
    expect(vi.mocked(useUnloadGuard)).toHaveBeenLastCalledWith(
      expect.objectContaining({ enabled: false }),
    );

    act(() => useWorkspaceStore.setState({ stage: "running" }));
    expect(vi.mocked(useUnloadGuard)).toHaveBeenLastCalledWith(
      expect.objectContaining({
        enabled: true,
        message: expect.stringContaining("中止当前 Provider 运行"),
      }),
    );
    unmount();
  });
});
function gateItem(sessionId: string, key: string): CockpitInboxItem {
  return {
    id: `${sessionId}:gate:${key}`,
    kind: "gate",
    severity: 1,
    title: "门禁等待",
    summary: "等待人工确认",
    triage: false,
    source: "gate",
    createdAt: null,
    gate: {
      key,
      turn_id: key,
      stage: "human_confirm",
      flow_kind: "single_candidate",
      status: "open",
      trigger: null,
      remaining_budget: null,
      findings: [],
      resumable: false,
      triage: false,
      closed: null,
      closure_stage: null,
      opened_at: "2026-09-15T00:00:00.000Z",
      turn: null,
      action_block_reason: null,
    },
    inlineError: null,
  };
}

function stoppedItem(sessionId: string): CockpitInboxItem {
  return {
    id: `${sessionId}:stopped:${sessionId}`,
    kind: "stopped",
    severity: 2,
    title: "会话停在停点",
    summary: "等待人工接管后继续",
    triage: false,
    source: "session_status",
    createdAt: null,
    gate: null,
    inlineError: null,
  };
}

function hardErrorItem(sessionId: string): CockpitInboxItem {
  return {
    id: `${sessionId}:hard_error:error`,
    kind: "hard_error",
    severity: 3,
    title: "引擎错误",
    summary: "错误",
    triage: false,
    source: "engine_error",
    createdAt: null,
    gate: null,
    inlineError: null,
  };
}
