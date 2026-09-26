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
  useCockpitCodingAttemptForSession: () => () => null,
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

  // F-50 §4.1-8：无安全重放命令的重试不再渲染（引擎错误来源没有可重放的
  // advance 命令）；advance 来源才保留重试。硬错误永不提供「接管」。
  it("offers retry only for replayable advance errors and never takeover for hard error", () => {
    cockpitInbox.push(
      hardErrorItem("session_002"),
      {
        ...hardErrorItem("session_001"),
        id: "session_001:hard_error:advance:cmd_1",
        source: "advance",
        title: "推进被拒",
        summary: "ADVANCE_STAGE_INVALID · stage mismatch",
      },
    );

    renderCockpit();

    expect(screen.getByRole("button", { name: "重试推进" })).toBeVisible();
    expect(screen.getByRole("button", { name: "终止此门" })).toBeVisible();
    expect(screen.queryByRole("button", { name: "接管" })).toBeNull();
  });

  it("重新接管：STALE_DRIVER_LEASE 后重发 driver hello 夺回租约并撤下协议错误（F-11）", async () => {
    const user = userEvent.setup();
    const sendHello = vi.fn();
    const workspaceWs = mockWorkspaceWs({ sendHello });
    useWorkspaceStore.setState({
      protocolError: {
        code: "STALE_DRIVER_LEASE",
        message: "driver connection no longer holds the lease for write message advance",
      },
      activeNodeId: "author-1",
      timelineNodes: [timelineNode({ node_id: "author-1" })],
    });

    renderCockpitWith(workspaceWs);

    await user.click(screen.getByRole("button", { name: "重新接管" }));
    await user.click(screen.getByRole("button", { name: "确认重新接管" }));

    expect(sendHello).toHaveBeenCalledWith("session_001", "author-1");
    expect(useWorkspaceStore.getState().protocolError).toBeNull();
    expect(screen.queryByTestId("cockpit-inbox-item-hard_error")).toBeNull();
  });

  // F-50 裁决 7（动作面归属）：当前会话 hard error 的完整动作面只有一个——
  // 抽屉关：页级 ChatInputBar 自持重接管；抽屉开：完整动作归抽屉错误条，
  // 页级缩为引用面（只报状态与处理入口，不再重复重接管按钮）。
  it("动作面归属：抽屉打开时页级错误缩为引用面，抽屉承载完整动作（F-50）", async () => {
    const user = userEvent.setup();
    useWorkspaceStore.setState({
      stage: "prepare_context",
      protocolError: {
        code: "STALE_DRIVER_LEASE",
        message: "driver connection no longer holds the lease",
      },
    });

    renderCockpit();

    const notice = screen.getByTestId("hard-error-notice");
    expect(within(notice).getByRole("button", { name: "重新接管" })).toBeVisible();
    expect(notice).not.toHaveTextContent("处理入口在待处理抽屉");

    await user.click(screen.getByTestId("cockpit-inbox-drawer-trigger"));
    expect(screen.getByTestId("cockpit-inbox-drawer")).toHaveAttribute("data-state", "open");

    expect(notice).toHaveTextContent("处理入口在待处理抽屉");
    expect(within(notice).queryByRole("button", { name: "重新接管" })).toBeNull();
    expect(
      within(screen.getByTestId("cockpit-inbox-item-hard_error")).getByRole("button", {
        name: "重新接管",
      }),
    ).toBeVisible();
  });

  it("arms takeover on the first hotkey and calls the existing takeover only on the second", () => {
    vi.mocked(takeoverWorkspaceSession).mockResolvedValue({
      workspace_session_id: "child_001",
    } as TakeoverResponse);
    cockpitInbox.push(stoppedItem("session_001"));
    renderCockpit();

    // F-31：接管按钮随收件箱移入默认收起的抽屉。热键必须先展开抽屉——否则
    // arm() 出来的「确认接管」态落在 display:none 子树里，用户没看到确认态
    // 就要再按一次直接执行接管。jsdom 不加载 Tailwind，hidden 类不生效，
    // 因此以抽屉 data-state/class 断言「确认态所在子树已展开」。
    const drawer = screen.getByTestId("cockpit-inbox-drawer");
    expect(drawer).toHaveAttribute("data-state", "closed");

    fireEvent.keyDown(document, {
      code: COCKPIT_HOTKEYS.takeover.code,
      ctrlKey: true,
      shiftKey: true,
    });
    expect(takeoverWorkspaceSession).not.toHaveBeenCalled();
    expect(drawer).toHaveAttribute("data-state", "open");
    expect(drawer).not.toHaveClass("hidden");
    expect(drawer).toContainElement(screen.getByRole("button", { name: "确认接管" }));

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
});
