import { act, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { useWorkspaceWs } from "../hooks/useWorkspaceWs";
import { useWorkspaceStore, type TimelineNode } from "../state/workspace-ws-store";
import { ChatCockpitPage } from "./ChatCockpitPage";
import { installChatWorkspacePageTestHooks, mockWorkspaceWs } from "./ChatWorkspacePage.test-utils";

vi.mock("../hooks/useWorkspaceWs", () => ({
  useWorkspaceWs: vi.fn(),
}));

vi.mock("../api/workspace-content", () => ({
  fetchWorkspaceArtifactVersion: vi.fn(),
  fetchWorkspaceEventOutput: vi.fn(),
  fetchWorkspaceNodeDetail: vi.fn(),
  fetchWorkspacePrompt: vi.fn(),
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

  const renderCockpit = (sessionId = "session_001") => {
    mockWorkspaceWs();
    useWorkspaceStore.getState().setSessionIdForTest(sessionId);
    return render(<ChatCockpitPage sessionId={sessionId} onBack={vi.fn()} />);
  };

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

  it("exposes no input controls in the execution flow zone", () => {
    useWorkspaceStore.getState().setTimelineNodesForTest([timelineNode()]);

    renderCockpit();

    const flow = screen.getByTestId("cockpit-execution-flow");
    expect(flow.querySelectorAll("input, select, textarea")).toHaveLength(0);
    expect(within(flow).queryAllByRole("textbox")).toHaveLength(0);
  });

  it("offers no gate action buttons in the read-only cockpit", () => {
    const store = useWorkspaceStore.getState();
    store.setStage("human_confirm");
    store.appendChatEntry({
      id: "human_confirm:gate-prompt",
      type: "gate_prompt",
      role: "system",
      content: "等待人工确认",
      timestamp: "2026-09-13T00:00:00Z",
    });

    renderCockpit();

    expect(screen.getByTestId("gate-prompt-entry")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /确认产物/ })).toBeNull();
    expect(screen.queryByRole("button", { name: /采纳建议并返修/ })).toBeNull();
    expect(screen.queryByRole("button", { name: /终止/ })).toBeNull();
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
