import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  actionFacadeForFlowKind,
  classifyProtocolError,
  createCockpitActionFacade,
  type ProtocolErrorDisposition,
} from "./cockpit-action-routing";
import type { WorkspaceWsState } from "./workspace-ws-store-types";
import { useWorkspaceStore } from "./workspace-ws-store";
import { installWorkspaceStoreTestHooks } from "./workspace-ws-store.test-utils";

function stateWithAdvance(commandId: string): Pick<WorkspaceWsState, "humanGateTurn" | "advanceCommands"> {
  return {
    humanGateTurn: null,
    advanceCommands: {
      [commandId]: {
        command_id: commandId,
        status: "pending",
        code: null,
        reason: null,
        attempt_id: null,
        workspace_entry: null,
        inlineError: null,
      },
    },
  };
}

function stateWithGate(turnId: string): Pick<WorkspaceWsState, "humanGateTurn" | "advanceCommands"> {
  return {
    humanGateTurn: {
      turn_id: turnId,
      command_id: "command-1",
      remaining_budget: 1,
      status: "open",
      artifact_ref: null,
      failure_class: null,
      failure_message: null,
      opened_at: "2026-09-14T00:00:00.000Z",
      inlineError: null,
    },
    advanceCommands: {},
  };
}

function emptyWorkspaceState(): Pick<WorkspaceWsState, "humanGateTurn" | "advanceCommands"> {
  return { humanGateTurn: null, advanceCommands: {} };
}

describe("classifyProtocolError", () => {
  it("routes replay rejection to its advance record instead of creating hard error", () => {
    expect(
      classifyProtocolError(
        "ADVANCE_REPLAY_NOT_READY",
        { command_id: "advance-1" },
        stateWithAdvance("advance-1"),
      ),
    ).toEqual({ kind: "advance", commandId: "advance-1" } satisfies ProtocolErrorDisposition);
  });

  it("routes an owned gate rejection to its gate record", () => {
    expect(
      classifyProtocolError(
        "INVALID_HUMAN_CONFIRM_ACTION",
        { turn_id: "turn-1" },
        stateWithGate("turn-1"),
      ),
    ).toEqual({ kind: "gate", turnId: "turn-1" } satisfies ProtocolErrorDisposition);
  });

  it("keeps an unowned protocol error as hard error", () => {
    expect(classifyProtocolError("UNEXPECTED_FRAME", {}, emptyWorkspaceState())).toEqual({
      kind: "hard_error",
    } satisfies ProtocolErrorDisposition);
  });
});

describe("cockpit gate action facade", () => {
  installWorkspaceStoreTestHooks();
  it("classifies every single-candidate gate, including a snapshot gate, as typed", () => {
    expect(actionFacadeForFlowKind("single_candidate")).toBe("typed");
    expect(actionFacadeForFlowKind("legacy")).toBe("legacy");
  });

  beforeEach(() => {
    useWorkspaceStore.setState({
      stage: "human_confirm",
      flowKind: "single_candidate",
      singleCandidatePhase: "approval",
      humanGateClosure: null,
      humanGateSnapshot: null,
    });
  });
  it("dispatches typed snapshot feedback with a freshly generated command id", () => {
    const sendHumanGateFeedback = vi.fn(
      (_feedback: string, _commandId?: string) => true,
    );
    const actions = createCockpitActionFacade({
      getState: useWorkspaceStore.getState,
      flowKind: "single_candidate",
      commandId: null,
      sendConfirm: vi.fn(() => true),
      sendAbandonGate: vi.fn(() => true),
      sendHumanGateFeedback,
      sendAdvance: vi.fn(() => true),
    });

    actions.feedback("请补齐边界");

    expect(sendHumanGateFeedback).toHaveBeenCalledTimes(1);
    const [feedback, commandId] = sendHumanGateFeedback.mock.calls[0];
    expect(feedback).toBe("请补齐边界");
    // 协议允许客户端自生成 command_id：断言拿到了新的 id，而不是拒发或透传空值。
    expect(commandId).toMatch(
      /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/,
    );
  });

  it("reuses the live turn command id instead of regenerating one", () => {
    const sendHumanGateFeedback = vi.fn(
      (_feedback: string, _commandId?: string) => true,
    );
    const actions = createCockpitActionFacade({
      flowKind: "single_candidate",
      getState: useWorkspaceStore.getState,
      commandId: "cmd_1",
      sendConfirm: vi.fn(() => true),
      sendAbandonGate: vi.fn(() => true),
      sendHumanGateFeedback,
      sendAdvance: vi.fn(() => true),
    });
    actions.feedback("请补齐边界");

    expect(sendHumanGateFeedback).toHaveBeenCalledWith("请补齐边界", "cmd_1");
  });

  it("keeps legacy flow feedback off the typed websocket helper", () => {
    const sendHumanGateFeedback = vi.fn(
      (_feedback: string, _commandId?: string) => true,
    );
    const actions = createCockpitActionFacade({
      flowKind: "legacy",
      commandId: "cmd_1",
      getState: useWorkspaceStore.getState,
      sendConfirm: vi.fn(() => true),
      sendAbandonGate: vi.fn(() => true),
      sendHumanGateFeedback,
      sendAdvance: vi.fn(() => true),
    });
    actions.feedback("请补齐边界");

    expect(sendHumanGateFeedback).not.toHaveBeenCalled();
  });

  it("sends a manual advance exactly once through the same facade", () => {
    useWorkspaceStore.setState({
      stage: "human_confirm",
      humanGateClosure: { decision: "confirm", stage: "human_confirm" },
    });
    const sendAdvance = vi.fn<(commandId?: string) => boolean>(() => true);
    createCockpitActionFacade({
      flowKind: "legacy",
      commandId: null,
      getState: useWorkspaceStore.getState,
      sendConfirm: vi.fn(() => true),
      sendAbandonGate: vi.fn(() => true),
      sendHumanGateFeedback: vi.fn(() => true),
      sendAdvance,
    }).advance();

    expect(sendAdvance).toHaveBeenCalledTimes(1);
    expect(sendAdvance.mock.calls[0]?.[0]).toMatch(/^[0-9a-f-]{36}$/);
  });
  it("allows manual advance after the engine has confirmed a completed single-candidate gate", () => {
    useWorkspaceStore.setState({
      stage: "human_confirm",
      flowKind: "single_candidate",
      singleCandidatePhase: "completed",
      sessionStatus: "confirmed",
      humanGateClosure: null,
    });
    const sendAdvance = vi.fn<(commandId?: string) => boolean>(() => true);

    expect(
      createCockpitActionFacade({
        flowKind: "single_candidate",
        commandId: null,
        getState: useWorkspaceStore.getState,
        sendConfirm: vi.fn(() => true),
      sendAbandonGate: vi.fn(() => true),
        sendHumanGateFeedback: vi.fn(() => true),
        sendAdvance,
      }).advance(),
    ).toBe(true);

    expect(sendAdvance).toHaveBeenCalledOnce();
  });

  it("re-reads actionability before every action so a stale snapshot cannot send", () => {
    const sendConfirm = vi.fn(() => true);
    const sendAbandonGate = vi.fn(() => true);
    const sendHumanGateFeedback = vi.fn(() => true);
    const sendAdvance = vi.fn(() => true);
    const actions = createCockpitActionFacade({
      flowKind: "single_candidate",
      commandId: "cmd_1",
      getState: useWorkspaceStore.getState,
      sendConfirm,
      sendHumanGateFeedback,
      sendAbandonGate,
      sendAdvance,
    });
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

    expect(actions.confirm()).toBe(false);
    expect(actions.feedback("请补齐边界")).toBe(false);
    expect(actions.terminate()).toBe(false);
    expect(actions.advance()).toBe(false);
    expect(sendConfirm).not.toHaveBeenCalled();
    expect(sendAbandonGate).not.toHaveBeenCalled();
    expect(sendHumanGateFeedback).not.toHaveBeenCalled();
    expect(sendAdvance).not.toHaveBeenCalled();
  });
});
