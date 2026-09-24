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
    await user.click(screen.getByRole("button", { name: "重试推进" }));

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
    const sendAbandonGate = vi.fn(() => true);
    mockWorkspaceWs({ sendConfirmGate: sendHumanConfirm, sendAbandonGate });
    const store = useWorkspaceStore.getState();
    store.applyHumanGateTurnOpen("g1", "confirm-g1", 1);
    store.rebuildChatEntries();
    cockpitInbox.push(gateItem("session_001", "g1"), gateItem("session_002", "g2"), stoppedItem("session_001"));

    renderCockpit("session_001", false);

    const gateChoices = screen.getAllByRole("checkbox", { name: "选择 需要人工确认" });
    expect(gateChoices).toHaveLength(2);
    await user.click(gateChoices[0]);
    await user.click(gateChoices[1]);
    await user.click(screen.getByRole("button", { name: /批量确认/ }));

    expect(sendHumanConfirm).toHaveBeenCalledTimes(1);
    expect(sendHumanConfirm).toHaveBeenCalledWith();
    expect(bulkConfirmStart).toHaveBeenCalledTimes(1);
    expect(bulkConfirmStart).toHaveBeenCalledWith([
      {
        itemId: "session_002:gate:g2",
        sessionId: "session_002",
        gateKey: "g2",
        title: "需要人工确认",
      },
    ]);
    expect(takeoverWorkspaceSession).not.toHaveBeenCalled();
  });

  it("confirms only while a current gate is open and does nothing without a gate", () => {
    const sendHumanConfirm = vi.fn(() => true);
    const sendAbandonGate = vi.fn(() => true);
    mockWorkspaceWs({ sendConfirmGate: sendHumanConfirm, sendAbandonGate });
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

    expect(sendHumanConfirm).toHaveBeenCalledWith();

    store.applyHumanGateClosed("confirm", "human_confirm");
    fireEvent.keyDown(document, { code: COCKPIT_HOTKEYS.confirm.code, ctrlKey: true });
    expect(sendHumanConfirm).toHaveBeenCalledOnce();
  });

  it("does not dispatch cockpit hotkeys from gate feedback editors", () => {
    const sendHumanConfirm = vi.fn(() => true);
    const sendAbandonGate = vi.fn(() => true);
    mockWorkspaceWs({ sendConfirmGate: sendHumanConfirm, sendAbandonGate });
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
    const sendAbandonGate = vi.fn(() => true);
    const sendHumanGateFeedback = vi.fn(() => true);
    const sendAdvance = vi.fn(() => true);
    mockWorkspaceWs({ sendConfirmGate: sendHumanConfirm, sendAbandonGate, sendHumanGateFeedback, sendAdvance });
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
    const sendAbandonGate = vi.fn(() => true);
    const sendHumanGateFeedback = vi.fn(() => true);
    const sendAdvance = vi.fn(() => true);
    mockWorkspaceWs({ sendConfirmGate: sendHumanConfirm, sendAbandonGate, sendHumanGateFeedback, sendAdvance });
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
    const sendAbandonGate = vi.fn(() => true);
    mockWorkspaceWs({ sendConfirmGate: sendHumanConfirm, sendAbandonGate });
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

    expect(sendHumanConfirm).toHaveBeenCalledWith();
  });
});
