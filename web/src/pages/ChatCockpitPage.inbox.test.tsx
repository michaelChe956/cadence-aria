import { useProviderAvailabilityStore } from "../state/provider-availability-store";
import type * as WorkspaceWsModule from "../hooks/useWorkspaceWs";
import type * as ApiClient from "../api/client";
import {
  ApiRequestError,
  getCodingAttemptSnapshot,
  getWorkspaceChoiceResponseStatus,
  postCodingChoiceResponse,
  postWorkspaceChoiceResponse,
  postWorkspaceHumanAction,
  takeoverWorkspaceSession,
} from "../api/client";
import type { CodingAttempt, CodingAttemptSnapshotResponse } from "../api/types";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
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
  postWorkspaceHumanAction: vi.fn(),
  postWorkspaceChoiceResponse: vi.fn(),
  getWorkspaceChoiceResponseStatus: vi.fn(),
  postCodingChoiceResponse: vi.fn(),
  getCodingAttemptSnapshot: vi.fn(),
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

const codingAttemptHolder = vi.hoisted(() => ({
  attempt: null as CodingAttempt | null,
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
  useCockpitCodingAttemptForSession: () => (sessionId: string) =>
    codingAttemptHolder.attempt === null || sessionId !== "session_001"
      ? null
      : codingAttemptHolder.attempt,
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
    expect(within(inbox).getByText("需要人工确认")).toBeInTheDocument();
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
    expect(within(gateEntry).getByRole("button", { name: "确认当前版本" })).toBeVisible();
    expect(within(gateEntry).getByRole("button", { name: "终止此门" })).toBeVisible();
  });
  it("sends plan gate confirmation from the 确认当前版本 button", async () => {
    const sendHumanConfirm = vi.fn(() => true);
    const sendAbandonGate = vi.fn(() => true);
    mockWorkspaceWs({ sendConfirmGate: sendHumanConfirm, sendAbandonGate });
    const store = useWorkspaceStore.getState();
    store.setStage("human_confirm");
    useWorkspaceStore.setState({
      flowKind: "single_candidate",
      singleCandidatePhase: "approval",
    });
    store.applyHumanGateTurnOpen("turn_confirm", "cmd_confirm", 1);
    store.rebuildChatEntries();

    renderCockpit("session_001", false);

    await userEvent.click(screen.getByRole("button", { name: "确认当前版本" }));

    expect(sendHumanConfirm).toHaveBeenCalledOnce();
    expect(sendHumanConfirm).toHaveBeenCalledWith();
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
      choice: null,
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
    expect(within(inbox).getByText("操作被拒绝")).toBeInTheDocument();
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
    expect(within(screen.getByTestId("cockpit-inbox")).getByText("需要人工确认")).toBeInTheDocument();

    act(() => {
      useWorkspaceStore.getState().applyHumanGateClosed("confirm", "human_confirm");
    });

    expect(within(screen.getByTestId("cockpit-inbox")).queryByText("需要人工确认")).toBeNull();
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
    // F-50 fix round 标题去重：triage 短语只保留主区门卡卡头一处——抽屉门条目
    // 与非 triage 门同题「需要人工确认」，不再复述「需要判断 reviewer 意图」。
    expect(within(inbox).getAllByText(/需要判断 reviewer 意图|需要人工确认/)).toHaveLength(1);
    expect(within(inbox).getByText("需要人工确认")).toBeInTheDocument();
    expect(within(inbox).queryByText("需要判断 reviewer 意图")).toBeNull();
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

  it("抽屉收起时反馈热键先展开抽屉再聚焦抽屉内的反馈框（F-31 story 确认面）", () => {
    // story/design 的 author_confirm 会自动切到产物下钻视图，主区的门卡
    // （GatePromptEntry）不在树上——唯一的反馈框在默认收起的抽屉里，此前
    // focus() 对 display:none 子树静默 no-op，Ctrl+F 看着像坏了。
    const store = useWorkspaceStore.getState();
    useWorkspaceStore.setState({
      flowKind: "single_candidate",
      workspaceType: "story",
      stage: "author_confirm",
    });
    store.applyHumanGateTurnOpen("turn_1", "cmd_1", 1);
    store.rebuildChatEntries();

    renderCockpit();

    const drawer = screen.getByTestId("cockpit-inbox-drawer");
    expect(drawer).toHaveAttribute("data-state", "closed");
    const editor = within(drawer).getByLabelText("门禁反馈");

    fireEvent.keyDown(document, { code: COCKPIT_HOTKEYS.feedback.code, ctrlKey: true });

    expect(drawer).toHaveAttribute("data-state", "open");
    expect(drawer).not.toHaveClass("hidden");
    expect(editor).toHaveFocus();

    // 抽屉已展开时再按一次同样要回到反馈框（挂起聚焦不能只在「展开那一次」生效）。
    editor.blur();
    expect(editor).not.toHaveFocus();

    fireEvent.keyDown(document, { code: COCKPIT_HOTKEYS.feedback.code, ctrlKey: true });

    expect(editor).toHaveFocus();
  });

  // P0 1.3（REQ-WIGA-05）Task 10：durable owner=server 的观察态会话——驾驶舱
  // 门卡 feedback/terminate/compile recovery 一律走 REST 人工命令端点
  //（postWorkspaceHumanAction），不借 driver WS 三命令；owner=client/未知
  //（null）保留手动 WS 语义（上方 typed 用例即对照）。
  function serverOwnedObserverSession(overrides: Partial<WorkspaceWsState> = {}) {
    useWorkspaceStore.setState({
      sessionId: "session_server",
      stage: "human_confirm",
      flowKind: "single_candidate",
      singleCandidatePhase: "approval",
      sessionStatus: "waiting_for_human",
      humanGateTurn: null,
      humanGateSnapshot: {
        findings: [],
        repeated_fingerprints: [],
        attempts_used: 1,
        manual_repairs_remaining: 1,
        trigger: "verification_new_findings",
        resumable: true,
      },
      humanGateClosure: null,
      activeNodeId: "node_gate",
      timelineNodes: [
        timelineNode({ node_id: "node_gate", stage: "human_confirm", status: "active" }),
      ],
      automation: {
        owner: "server",
        enrollment_id: "enr-1",
        policy_revision: 1,
        enabled: true,
      },
      ...overrides,
    });
    useWorkspaceStore.getState().rebuildChatEntries();
  }

  beforeEach(() => {
    vi.mocked(postWorkspaceHumanAction).mockReset();
    vi.mocked(postWorkspaceHumanAction).mockResolvedValue({
      command_id: "cmd-rest",
      state: "accepted",
      gate_id: "node_gate",
    });
  });

  it("sends observer gate feedback through the REST human action endpoint for a server-owned session", async () => {
    const user = userEvent.setup();
    serverOwnedObserverSession();
    const workspaceWs = mockWorkspaceWs();
    renderCockpitWith(workspaceWs, "session_server");

    const inbox = screen.getByTestId("cockpit-inbox");
    await user.type(within(inbox).getByLabelText("门禁反馈"), "请补齐边界");
    await user.click(within(inbox).getByRole("button", { name: "提交反馈" }));

    expect(postWorkspaceHumanAction).toHaveBeenCalledTimes(1);
    expect(postWorkspaceHumanAction).toHaveBeenCalledWith(
      "session_server",
      expect.objectContaining({
        type: "feedback",
        expected_gate_id: "node_gate",
        feedback: "请补齐边界",
      }),
    );
    expect(workspaceWs.sendHumanGateFeedback).not.toHaveBeenCalled();
  });

  it("terminates a server-owned observer gate through the REST abandon action", async () => {
    const user = userEvent.setup();
    serverOwnedObserverSession();
    const workspaceWs = mockWorkspaceWs();
    renderCockpitWith(workspaceWs, "session_server");

    const inbox = screen.getByTestId("cockpit-inbox");
    await user.click(within(inbox).getByRole("button", { name: "终止此门" }));
    await user.click(within(inbox).getByRole("button", { name: "确认终止此门" }));

    expect(postWorkspaceHumanAction).toHaveBeenCalledTimes(1);
    expect(postWorkspaceHumanAction).toHaveBeenCalledWith(
      "session_server",
      expect.objectContaining({ type: "abandon", expected_gate_id: "node_gate" }),
    );
    expect(workspaceWs.sendAbandonGate).not.toHaveBeenCalled();
  });

  it("recovers a server-owned compile gate through the REST human action endpoint", async () => {
    const user = userEvent.setup();
    serverOwnedObserverSession({
      humanGateSnapshot: null,
      activeNodeId: "node_recovery",
      timelineNodes: [
        timelineNode({
          node_id: "node_recovery",
          node_type: "work_item_plan_compile_recovery",
          stage: "human_confirm",
          status: "active",
        }),
      ],
    });
    const workspaceWs = mockWorkspaceWs();
    renderCockpitWith(workspaceWs, "session_server");

    const inbox = screen.getByTestId("cockpit-inbox");
    await user.click(within(inbox).getByRole("button", { name: "继续" }));

    expect(postWorkspaceHumanAction).toHaveBeenCalledTimes(1);
    expect(postWorkspaceHumanAction).toHaveBeenCalledWith(
      "session_server",
      expect.objectContaining({
        type: "compile_recovery",
        expected_gate_id: "node_recovery",
        action: "continue",
      }),
    );
    expect(workspaceWs.sendWorkItemPlanCompileRecoveryAction).not.toHaveBeenCalled();
  });

  it("surfaces a REST human action rejection as the protocol error without touching the WS channel", async () => {
    const user = userEvent.setup();
    serverOwnedObserverSession();
    vi.mocked(postWorkspaceHumanAction).mockRejectedValue(
      new ApiRequestError({
        code: "human_action_gate_mismatch",
        message: "门身份不匹配",
        details: {},
      }),
    );
    const workspaceWs = mockWorkspaceWs();
    renderCockpitWith(workspaceWs, "session_server");

    const inbox = screen.getByTestId("cockpit-inbox");
    await user.type(within(inbox).getByLabelText("门禁反馈"), "请补齐边界");
    await user.click(within(inbox).getByRole("button", { name: "提交反馈" }));

    expect(await screen.findByText("门身份不匹配")).toBeInTheDocument();
    expect(workspaceWs.sendHumanGateFeedback).not.toHaveBeenCalled();
  });

  // P0 1.3（REQ-WIGA-05）Task 11：驾驶舱 choice 就地作答——REST 通道（无
  // driver WS 也能答）；202 保卡+同 command GET 复查；409/410 就地反馈；
  // coding choice 按 attempt 地址经 REST 作答，不依赖 Coding Workspace。
  const CHOICE_ANSWERS = [
    { question_id: "q-1", selected_option_ids: ["yes"], free_text: null },
    { question_id: "q-2", selected_option_ids: ["two"], free_text: null },
  ];

  function pendingChoiceSession() {
    useWorkspaceStore.setState({
      sessionId: "session_001",
      stage: "human_confirm",
      sessionStatus: "waiting_for_human",
      flowKind: "single_candidate",
      singleCandidatePhase: "approval",
      pendingChoiceRequests: [
        {
          id: "choice-1",
          prompt: "拆分方案确认",
          role: "author",
          created_at_ms: null,
          first_seen_at_ms: null,
          expected_run_id: "run-1",
          options: [],
          allow_multiple: false,
          allow_free_text: false,
          questions: [
            {
              id: "q-1",
              prompt: "是否包含集成测试",
              options: [
                { id: "yes", label: "包含" },
                { id: "no", label: "不包含" },
              ],
              allow_multiple: false,
              allow_free_text: false,
            },
            {
              id: "q-2",
              prompt: "评审轮数",
              options: [
                { id: "one", label: "一轮" },
                { id: "two", label: "两轮" },
              ],
              allow_multiple: false,
              allow_free_text: false,
            },
          ],
          source: "ask_user_question",
        },
      ],
    });
  }

  beforeEach(() => {
    codingAttemptHolder.attempt = null;
    vi.mocked(postWorkspaceChoiceResponse).mockReset();
    vi.mocked(getWorkspaceChoiceResponseStatus).mockReset();
    vi.mocked(postCodingChoiceResponse).mockReset();
    vi.mocked(getCodingAttemptSnapshot).mockReset();
  });

  it("从收件箱就地作答 workspace choice：REST 单命令携两题独立答案", async () => {
    const user = userEvent.setup();
    pendingChoiceSession();
    vi.mocked(postWorkspaceChoiceResponse).mockResolvedValue({
      command_id: "cmd-choice-1",
      expected_run_id: "run-1",
      choice_id: "choice-1",
      state: "delivered",
    });
    const workspaceWs = mockWorkspaceWs();
    renderCockpitWith(workspaceWs, "session_001");

    const inbox = screen.getByTestId("cockpit-inbox");
    await user.click(within(inbox).getByRole("radio", { name: "包含" }));
    await user.click(within(inbox).getByRole("radio", { name: "两轮" }));
    await user.click(within(inbox).getByRole("button", { name: "提交选择" }));

    expect(postWorkspaceChoiceResponse).toHaveBeenCalledTimes(1);
    expect(postWorkspaceChoiceResponse).toHaveBeenCalledWith("session_001", "choice-1", {
      command_id: expect.any(String),
      expected_run_id: "run-1",
      answers: CHOICE_ANSWERS,
    });
    expect(workspaceWs.sendChoiceResponse).not.toHaveBeenCalled();
  });

  it("202 后保卡显示处理中，同 command GET 复查 Delivered 后卡片消失", async () => {
    const user = userEvent.setup();
    pendingChoiceSession();
    vi.mocked(postWorkspaceChoiceResponse).mockResolvedValue({
      command_id: "cmd-rest",
      expected_run_id: "run-1",
      choice_id: "choice-1",
      state: "submitting",
    });
    vi.mocked(getWorkspaceChoiceResponseStatus).mockResolvedValue({
      command_id: "cmd-rest",
      expected_run_id: "run-1",
      choice_id: "choice-1",
      state: "delivered",
    });
    renderCockpit("session_001");

    const inbox = screen.getByTestId("cockpit-inbox");
    await user.click(within(inbox).getByRole("radio", { name: "包含" }));
    await user.click(within(inbox).getByRole("radio", { name: "两轮" }));
    await user.click(within(inbox).getByRole("button", { name: "提交选择" }));

    expect(await screen.findByText("处理中")).toBeVisible();
    await waitFor(() => {
      expect(getWorkspaceChoiceResponseStatus).toHaveBeenCalledWith(
        "session_001",
        "choice-1",
        "cmd-rest",
      );
    });
    await waitFor(() => {
      expect(screen.getByTestId("cockpit-inbox").textContent).not.toContain("拆分方案确认");
    });
  });

  it("409 冲突就地亮冲突码，重试复用同 command 同 payload", async () => {
    const user = userEvent.setup();
    pendingChoiceSession();
    vi.mocked(postWorkspaceChoiceResponse)
      .mockRejectedValueOnce(
        new ApiRequestError({
          code: "workspace_choice_conflict",
          message: "应答与在途命令冲突",
          details: {},
        }),
      )
      .mockResolvedValueOnce({
        command_id: "cmd-retry",
        expected_run_id: "run-1",
        choice_id: "choice-1",
        state: "delivered",
      });
    renderCockpit("session_001");

    const inbox = screen.getByTestId("cockpit-inbox");
    await user.click(within(inbox).getByRole("radio", { name: "包含" }));
    await user.click(within(inbox).getByRole("radio", { name: "两轮" }));
    await user.click(within(inbox).getByRole("button", { name: "提交选择" }));

    expect(await screen.findByText(/workspace_choice_conflict/)).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "提交选择" }));

    const calls = vi.mocked(postWorkspaceChoiceResponse).mock.calls;
    expect(calls[1][2].command_id).toBe(calls[0][2].command_id);
    expect(calls[1][2].answers).toEqual(calls[0][2].answers);
  });

  it("coding choice 从驾驶舱按 attempt 地址经 REST 作答（无 coding socket）", async () => {
    const user = userEvent.setup();
    codingAttemptHolder.attempt = {
      project_id: "project_0001",
      issue_id: "issue_0001",
      attempt_id: "attempt_0001",
    } as CodingAttempt;
    vi.mocked(getCodingAttemptSnapshot).mockResolvedValue({
      pending_choices: [
        {
          gate_id: "coding_gate_1",
          choice_id: "choice-coding",
          attempt_id: "attempt_0001",
          node_id: null,
          stage: "coding",
          role: "coder",
          provider: "fake",
          source: "provider",
          prompt: "编码拆分确认",
          options: [],
          allow_multiple: false,
          allow_free_text: false,
          status: "open",
          response: null,
          questions: [
            {
              id: "q-1",
              prompt: "是否补齐测试",
              options: [
                { id: "yes", label: "方案A" },
                { id: "no", label: "方案B" },
              ],
              allow_multiple: false,
              allow_free_text: false,
            },
            {
              id: "q-2",
              prompt: "评审节奏",
              options: [
                { id: "one", label: "先自审" },
                { id: "two", label: "直接评审" },
              ],
              allow_multiple: false,
              allow_free_text: false,
            },
          ],
          created_at: "2026-09-26T00:00:00Z",
          updated_at: "2026-09-26T00:00:00Z",
          expected_run_id: "run-coding",
        },
      ],
    } as unknown as CodingAttemptSnapshotResponse);
    vi.mocked(postCodingChoiceResponse).mockResolvedValue({
      command_id: "cmd-coding",
      expected_run_id: "run-coding",
      choice_id: "choice-coding",
      state: "delivered",
    });
    const workspaceWs = mockWorkspaceWs();
    renderCockpitWith(workspaceWs, "session_001");

    const inbox = screen.getByTestId("cockpit-inbox");
    expect((await screen.findAllByText("编码拆分确认")).length).toBeGreaterThan(0);
    await user.click(within(inbox).getByRole("radio", { name: "方案A" }));
    await user.click(within(inbox).getByRole("radio", { name: "直接评审" }));
    await user.click(within(inbox).getByRole("button", { name: "提交选择" }));

    expect(postCodingChoiceResponse).toHaveBeenCalledTimes(1);
    expect(postCodingChoiceResponse).toHaveBeenCalledWith(
      { projectId: "project_0001", issueId: "issue_0001", attemptId: "attempt_0001" },
      "choice-coding",
      {
        command_id: expect.any(String),
        expected_run_id: "run-coding",
        answers: [
          { question_id: "q-1", selected_option_ids: ["yes"], free_text: null },
          { question_id: "q-2", selected_option_ids: ["two"], free_text: null },
        ],
      },
    );
    expect(workspaceWs.sendChoiceResponse).not.toHaveBeenCalled();
  });
});
