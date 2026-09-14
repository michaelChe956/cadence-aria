import type * as ApiClient from "../api/client";
import type { TakeoverResponse } from "../api/types";
import { ApiRequestError, takeoverWorkspaceSession } from "../api/client";
import { act, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  selectCockpitInbox,
  type CockpitInboxItem,
} from "../state/workspace-cockpit-projection";
import { useWorkspaceWs } from "../hooks/useWorkspaceWs";
import {
  useWorkspaceStore,
  type TimelineNode,
  type WorkspaceWsState,
} from "../state/workspace-ws-store";
import { observerStateFromSessionState } from "../state/workspace-observer-store";
import { readCockpitSettings } from "../state/cockpit-settings";
import { ChatCockpitPage } from "./ChatCockpitPage";
import { installChatWorkspacePageTestHooks, mockWorkspaceWs } from "./ChatWorkspacePage.test-utils";

vi.mock("../hooks/useWorkspaceWs", () => ({
  useWorkspaceWs: vi.fn(),
}));


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
  });
  const renderCockpit = (sessionId = "session_001", mockWs = true) => {
    if (mockWs) {
      mockWorkspaceWs();
    }
    useWorkspaceStore.getState().setSessionIdForTest(sessionId);
    return render(<ChatCockpitPage sessionId={sessionId} onBack={vi.fn()} />);
  };

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
    });

    renderCockpit("session_001", false);
    await user.click(screen.getByRole("button", { name: "重试" }));

    expect(sendAdvance).toHaveBeenCalledWith("command_001");
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

  it("renders the three cockpit zones", () => {
    renderCockpit();

    expect(screen.getByTestId("cockpit-page")).toBeInTheDocument();
    expect(screen.getByTestId("cockpit-inbox")).toBeInTheDocument();
    expect(screen.getByTestId("cockpit-execution-flow")).toBeInTheDocument();
    expect(screen.getByTestId("cockpit-conversation-flow")).toBeInTheDocument();
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

  it("dispatches typed gate feedback through the typed websocket helper", async () => {
    const feedback = vi.fn(() => true);
    mockWorkspaceWs({ sendHumanGateFeedback: feedback });
    const user = userEvent.setup();
    const store = useWorkspaceStore.getState();
    store.setSessionIdForTest("session_001");
    useWorkspaceStore.setState({ flowKind: "single_candidate" });
    store.applyHumanGateTurnOpen("turn_1", "cmd_1", 1);
    store.rebuildChatEntries();

    render(<ChatCockpitPage sessionId="session_001" onBack={vi.fn()} />);
    const gateEntry = screen.getByTestId("gate-prompt-entry");
    const submit = within(gateEntry).getByRole("button", { name: "提交反馈" });
    expect(submit).toBeDisabled();
    await user.type(within(gateEntry).getByLabelText("反馈内容"), "请补齐边界");
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
    await user.click(screen.getByRole("button", { name: "编辑反馈" }));
    const feedbackInput = screen.getByLabelText("门禁反馈");
    const submit = within(screen.getByTestId("cockpit-inbox")).getByRole("button", {
      name: "提交反馈",
    });
    expect(submit).toBeDisabled();
    await user.type(feedbackInput, "请补齐边界");
    await user.click(submit);

    expect(feedback).toHaveBeenCalledWith("请补齐边界", "cmd_1");
  });

  it("keeps a typed snapshot inbox gate waiting until its command synchronizes", () => {
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
    });

    renderCockpit();

    const inbox = screen.getByTestId("cockpit-inbox");
    expect(within(inbox).getByText("等待门禁命令同步后再提交反馈")).toBeVisible();
    expect(within(inbox).queryByRole("button", { name: "编辑反馈" })).toBeNull();
    expect(within(inbox).queryByRole("button", { name: "提交反馈" })).toBeNull();
    expect(within(inbox).queryByRole("button", { name: "采纳建议并返修" })).toBeNull();
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
});
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
