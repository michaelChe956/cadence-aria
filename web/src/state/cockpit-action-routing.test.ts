import { describe, expect, it, vi } from "vitest";
import {
  actionFacadeForFlowKind,
  classifyProtocolError,
  createCockpitActionFacade,
  type ProtocolErrorDisposition,
} from "./cockpit-action-routing";
import type { WorkspaceWsState } from "./workspace-ws-store-types";

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
  it("classifies every single-candidate gate, including a snapshot gate, as typed", () => {
    expect(actionFacadeForFlowKind("single_candidate")).toBe("typed");
    expect(actionFacadeForFlowKind("legacy")).toBe("legacy");
  });

  it("dispatches typed snapshot feedback with a freshly generated command id", () => {
    const sendHumanGateFeedback = vi.fn(
      (_feedback: string, _commandId?: string) => true,
    );
    const actions = createCockpitActionFacade({
      flowKind: "single_candidate",
      commandId: null,
      sendHumanConfirm: vi.fn(() => true),
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
      commandId: "cmd_1",
      sendHumanConfirm: vi.fn(() => true),
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
      sendHumanConfirm: vi.fn(() => true),
      sendHumanGateFeedback,
      sendAdvance: vi.fn(() => true),
    });
    actions.feedback("请补齐边界");

    expect(sendHumanGateFeedback).not.toHaveBeenCalled();
  });

  it("sends a manual advance exactly once through the same facade", () => {
    const sendAdvance = vi.fn<(commandId?: string) => boolean>(() => true);
    createCockpitActionFacade({
      flowKind: "legacy",
      commandId: null,
      sendHumanConfirm: vi.fn(() => true),
      sendHumanGateFeedback: vi.fn(() => true),
      sendAdvance,
    }).advance();

    expect(sendAdvance).toHaveBeenCalledTimes(1);
    expect(sendAdvance.mock.calls[0]?.[0]).toMatch(/^[0-9a-f-]{36}$/);
  });
});
