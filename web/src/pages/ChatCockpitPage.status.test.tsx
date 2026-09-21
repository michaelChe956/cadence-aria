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
  useCockpitSettingsSlotRef: () => () => undefined,
}));

describe("ChatCockpitPage", () => {
  installCockpitPageTestHooks();

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
  it("freezes the elapsed clock on a failed run instead of ticking a dead timer (F-01)", () => {
    vi.useFakeTimers();
    try {
      vi.setSystemTime(new Date("2026-09-18T10:00:00Z"));
      const workspaceWs = mockWorkspaceWs();
      useWorkspaceStore.setState({
        providerStatus: "starting",
        stage: "prepare_context",
        providers: { author: "pi", reviewer: "codex" } as const,
        activeNodeId: "author-1",
        timelineNodes: [
          timelineNode({
            node_id: "author-1",
            status: "failed",
            started_at: new Date("2026-09-18T06:29:01Z").toISOString(),
            completed_at: new Date("2026-09-18T06:33:52Z").toISOString(),
          }),
        ],
      });

      renderCockpitWith(workspaceWs);

      const status = screen.getByTestId("cockpit-generation-status");
      expect(status).toHaveTextContent("生成失败");
      expect(status).not.toHaveTextContent("正在启动生成");

      act(() => {
        vi.advanceTimersByTime(10 * 60_000);
      });

      expect(status).toHaveTextContent("已用 4:51");
      expect(status).not.toHaveTextContent("3:40:59");
    } finally {
      vi.useRealTimers();
    }
  });

  it("unifies the running label once the engine stage is already generating (F-04)", () => {
    vi.useFakeTimers();
    try {
      vi.setSystemTime(new Date("2026-09-18T15:13:00Z"));
      const workspaceWs = mockWorkspaceWs();
      useWorkspaceStore.setState({
        providerStatus: "starting",
        stage: "running",
        providers: { author: "pi", reviewer: "codex" } as const,
        activeNodeId: "author-1",
        timelineNodes: [
          timelineNode({
            node_id: "author-1",
            status: "active",
            started_at: new Date("2026-09-18T14:48:33Z").toISOString(),
          }),
        ],
      });

      renderCockpitWith(workspaceWs);

      expect(screen.getByTestId("cockpit-generation-status")).toHaveTextContent(
        "正在生成 · pi · 运行中 · 已用 24:27",
      );
    } finally {
      vi.useRealTimers();
    }
  });

  it.each([
    ["completed", "生成完成"],
    ["skipped", "生成已跳过"],
  ] as const)(
    "reports a %s timeline node over the stale starting prefix (F-04)",
    (nodeStatus, label) => {
      const workspaceWs = mockWorkspaceWs();
      useWorkspaceStore.setState({
        providerStatus: "starting",
        stage: "prepare_context",
        activeNodeId: "author-1",
        timelineNodes: [
          timelineNode({
            node_id: "author-1",
            status: nodeStatus,
            started_at: new Date("2026-09-18T06:29:01Z").toISOString(),
            completed_at: new Date("2026-09-18T06:33:52Z").toISOString(),
          }),
        ],
      });

      renderCockpitWith(workspaceWs);

      const status = screen.getByTestId("cockpit-generation-status");
      expect(status).toHaveTextContent(label);
      expect(status).not.toHaveTextContent("正在启动生成");
      expect(status).toHaveTextContent("已用 4:51");
    },
  );
  // #4：会话完成（stage=completed/session confirmed）后引擎落「流程完成」标记
  // 节点（completed_at/duration_ms 均空）——状态行与执行流行都不得再随墙钟计时。
  it("freezes every visible timer once the session completes (#4)", () => {
    vi.useFakeTimers();
    try {
      vi.setSystemTime(new Date("2026-09-21T10:00:00Z"));
      const workspaceWs = mockWorkspaceWs();
      useWorkspaceStore.setState({
        providerStatus: "starting",
        stage: "completed",
        sessionStatus: "confirmed",
        activeNodeId: "node-6",
        timelineNodes: [
          timelineNode({
            node_id: "author-1",
            node_type: "author_run",
            status: "completed",
            started_at: new Date("2026-09-21T09:40:00Z").toISOString(),
            completed_at: new Date("2026-09-21T09:50:00Z").toISOString(),
          }),
          timelineNode({
            node_id: "node-6",
            node_type: "completed",
            title: "流程完成",
            status: "completed",
            started_at: new Date("2026-09-21T10:00:00Z").toISOString(),
            completed_at: null,
            duration_ms: null,
          }),
        ],
      });

      renderCockpitWith(workspaceWs);

      const status = screen.getByTestId("cockpit-generation-status");
      expect(status).toHaveTextContent("生成完成");
      expect(screen.getByTestId("flow-elapsed-completed")).toHaveTextContent(/^0s$/);

      act(() => {
        vi.advanceTimersByTime(10 * 60_000);
      });

      // 状态行不得出现随墙钟增长的「已用」段。
      expect(status).toHaveTextContent("生成完成");
      expect(status).not.toHaveTextContent("已用");
      // 完成节点冻结为 0；已完成 author 行冻结在 completed_at−started_at。
      expect(screen.getByTestId("flow-elapsed-completed")).toHaveTextContent(/^0s$/);
      expect(screen.getByTestId("flow-elapsed-author_run")).toHaveTextContent(/^10m0s$/);
    } finally {
      vi.useRealTimers();
    }
  });
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
