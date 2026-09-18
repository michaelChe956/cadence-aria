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
});
