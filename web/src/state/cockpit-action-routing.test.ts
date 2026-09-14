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

  it("does not dispatch snapshot feedback before the server provides a command id", () => {
    const sendHumanGateFeedback = vi.fn(() => true);
    const actions = createCockpitActionFacade({
      flowKind: "single_candidate",
      commandId: null,
      sendHumanConfirm: vi.fn(() => true),
      sendHumanGateFeedback,
    });

    actions.feedback("请补齐边界");

    expect(sendHumanGateFeedback).not.toHaveBeenCalled();
  });
});
