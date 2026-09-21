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
  mockWorkspaceWs,
} from "./ChatWorkspacePage.test-utils";
import {
  bulkConfirmStart,
  cockpitInbox,
  cockpitObservedRecords,
  gateItem,
  hardErrorItem,
  installCockpitPageTestHooks,
  renderCockpit,
  renderCockpitWith,
  stoppedItem,
  timelineNode,
  watchSession,
} from "./ChatCockpitPage.test-utils";

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

describe("ChatCockpitPage", () => {
  installCockpitPageTestHooks();

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

  // F-19（cadence/notes 2026-09-19 阶段4监控）：v27 story 会话 codex run 楔死
  // 27min，运行中页面无任何 中止/终止/放弃 控件（abortish 空），用户无法脱困。
  // cockpit 页 ChatInputBar 此前仅在 prepare_context/author_confirm 渲染；
  // 生成期（running，矩阵 protocol.rs 已放行 WsInMessage::Abort）必须暴露
  // 中止入口。
  it("exposes the abort entry during a running story generation and wires it to ws abort", async () => {
    const user = userEvent.setup();
    const abort = vi.fn();
    const runningWorkspaceWs = mockWorkspaceWs({ abort, connectionStatus: "connected" });
    useWorkspaceStore.setState({ stage: "running" });

    renderCockpitWith(runningWorkspaceWs);

    const abortButton = screen.getByRole("button", { name: /中止/ });
    expect(abortButton).toBeEnabled();

    await user.click(abortButton);

    expect(abort).toHaveBeenCalledTimes(1);
    expect(screen.queryByRole("button", { name: "开始生成" })).toBeNull();
  });

  // F-28（v33 复验 3）：start_generation 被仲裁层 STALE_DRIVER_LEASE 拒收后仅落
  // 左侧收件箱 hard_error 条目（远离视线），用户在生成按钮旁得不到任何反馈。
  // 就地错误面必须直出在生成动作区（不受收件箱 actionable 条件限制），
  // 且重接管保留 ConfirmTwiceButton 二次确认（复用 F-11 sendHello 回调链）。
  it("shows the stale-lease inline error beside the start generation button and retakes via confirm-twice (F-28)", async () => {
    const user = userEvent.setup();
    const sendHello = vi.fn();
    const workspaceWs = mockWorkspaceWs({ sendHello, connectionStatus: "connected" });
    useWorkspaceStore.setState({
      stage: "prepare_context",
      protocolError: {
        code: "STALE_DRIVER_LEASE",
        message: "driver connection no longer holds the lease for write message start_generation",
      },
      activeNodeId: "node-1",
      timelineNodes: [timelineNode({ node_id: "node-1" })],
    });

    renderCockpitWith(workspaceWs);

    const inputBar = screen.getByTestId("chat-input-bar");
    expect(within(inputBar).getByRole("alert")).toHaveTextContent(/连接租约已过期/);
    expect(
      within(inputBar).getByRole("button", { name: "开始生成" }),
    ).toBeInTheDocument();

    await user.click(within(inputBar).getByRole("button", { name: "重新接管" }));
    expect(sendHello).not.toHaveBeenCalled();

    await user.click(within(inputBar).getByRole("button", { name: "确认重新接管" }));
    expect(sendHello).toHaveBeenCalledWith("session_001", "node-1");
    expect(useWorkspaceStore.getState().protocolError).toBeNull();
    expect(within(inputBar).queryByRole("alert")).toBeNull();
  });

  it("clears the stale-lease inline error once the protocol error is externally cleared (F-28)", () => {
    mockWorkspaceWs({ connectionStatus: "connected" });
    useWorkspaceStore.setState({
      stage: "prepare_context",
      protocolError: {
        code: "STALE_DRIVER_LEASE",
        message: "driver connection no longer holds the lease",
      },
    });

    renderCockpit();
    expect(screen.getByTestId("chat-input-bar")).toHaveTextContent(/连接租约已过期/);

    act(() => {
      useWorkspaceStore.getState().setProtocolError(null);
    });

    expect(screen.queryByText(/连接租约已过期/)).toBeNull();
  });

  it("does not surface the stale-lease inline error for other protocol codes (F-28)", () => {
    mockWorkspaceWs({ connectionStatus: "connected" });
    useWorkspaceStore.setState({
      stage: "prepare_context",
      protocolError: { code: "OTHER_PROTOCOL_CODE", message: "unrelated hard error" },
    });

    renderCockpit();

    expect(screen.queryByText(/连接租约已过期/)).toBeNull();
  });
});
