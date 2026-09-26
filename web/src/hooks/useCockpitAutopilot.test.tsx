import { render } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { AdvanceCommandState, WorkspaceWsState } from "../state/workspace-ws-store-types";
import { useCockpitAutopilot } from "./useCockpitAutopilot";

const defaultSettings = {
  soundEnabled: false,
  systemNotificationsEnabled: false,
  titleEmojiEnabled: false,
  watchLimit: 8,
  observerRefreshIntervalMs: 15_000,
  gateOpenEscalationMs: 600_000,
  escalationRepeatMs: 300_000,
  escalationBudgetThreshold: 1,
  escalationBudgetRepeats: [],
  stopPoints: [],
} as const;

type AutopilotState = Pick<
  WorkspaceWsState,
  | "sessionId"
  | "sessionStatus"
  | "humanGateTurn"
  | "humanGateClosure"
  | "advanceCommands"
  | "flowKind"
  | "automation"
>;

function state(overrides: Partial<AutopilotState> = {}): AutopilotState {
  return {
    sessionId: "session-1",
    sessionStatus: "waiting_for_human",
    flowKind: "single_candidate",
    humanGateTurn: null,
    humanGateClosure: null,
    advanceCommands: {},
    // P0 1.2（REQ-WIGA-08）：非 enrolled fixture 默认明确 client——显式 client
    // 保留旧行为，未知（null）/server 由新场景单独覆盖。
    automation: { owner: "client", enrollment_id: null, policy_revision: null, enabled: false },
    ...overrides,
  };
}

function confirmedGate(turnId: string): AutopilotState {
  return state({
    sessionStatus: "confirmed",
    humanGateTurn: {
      turn_id: turnId,
      command_id: `feedback-${turnId}`,
      remaining_budget: 1,
      status: "awaiting_confirm",
      artifact_ref: "artifact-1",
      failure_class: null,
      failure_message: null,
      opened_at: "2026-09-14T00:00:00.000Z",
      inlineError: null,
    },
    humanGateClosure: { decision: "confirm", stage: "compile_plan" },
  });
}

function advanceRejected(commandId: string): AdvanceCommandState {
  return {
    command_id: commandId,
    status: "rejected",
    code: "ADVANCE_NOT_READY",
    reason: "not ready",
    attempt_id: null,
    workspace_entry: null,
    inlineError: null,
  };
}

function renderAutopilot(
  autopilotState: AutopilotState,
  sendAdvance = vi.fn<(commandId: string) => boolean>(() => true),
  stopPoints: readonly ("human_gate" | "stopped" | "hard_error")[] = [],
  sessionId = "session-1",
) {
  return {
    sendAdvance,
    ...render(
      <AutopilotHarness
        sessionId={sessionId}
        state={autopilotState}
        sendAdvance={sendAdvance}
        stopPoints={stopPoints}
      />,
    ),
  };
}

describe("useCockpitAutopilot", () => {
  it("sends one stable-id advance only after durable confirmed", () => {
    const view = renderAutopilot(confirmedGate("turn-7"));

    expect(view.sendAdvance).toHaveBeenCalledTimes(1);
    const commandId = view.sendAdvance.mock.calls[0]?.[0];
    expect(commandId).toEqual(expect.any(String));

    view.rerender(
      <AutopilotHarness
        state={confirmedGate("turn-7")}
        sendAdvance={view.sendAdvance}
        stopPoints={[]}
      />,
    );

    expect(view.sendAdvance).toHaveBeenCalledWith(commandId);
    expect(view.sendAdvance).toHaveBeenCalledTimes(1);
  });

  it.each([
    ["feedback only", state({ humanGateTurn: { ...confirmedGate("turn-feedback").humanGateTurn!, status: "awaiting_confirm" } })],
    ["compile failure reopen", state({ humanGateTurn: { ...confirmedGate("turn-failed").humanGateTurn!, status: "failed" } })],
    ["busy", state({ humanGateTurn: { ...confirmedGate("turn-busy").humanGateTurn!, status: "busy" } })],
  ])("does not auto advance after %s", (_name, autopilotState) => {
    const view = renderAutopilot(autopilotState);

    expect(view.sendAdvance).not.toHaveBeenCalled();
  });

  it("stops the session after advance rejection and does not queue retry", () => {
    const view = renderAutopilot(confirmedGate("turn-1"));
    const commandId = view.sendAdvance.mock.calls[0]?.[0];
    if (!commandId) throw new Error("expected autopilot command id");

    view.rerender(
      <AutopilotHarness
        state={state({
          ...confirmedGate("turn-1"),
          advanceCommands: { [commandId]: advanceRejected(commandId) },
        })}
        sendAdvance={view.sendAdvance}
        stopPoints={[]}
      />,
    );
    view.rerender(
      <AutopilotHarness
        state={confirmedGate("turn-2")}
        sendAdvance={view.sendAdvance}
        stopPoints={[]}
      />,
    );

    expect(view.sendAdvance).toHaveBeenCalledTimes(1);
  });

  it("does not carry rejected commands into a new session before its snapshot arrives", () => {
    const sendAdvance = vi.fn<(commandId: string) => boolean>(() => true);
    const view = renderAutopilot(
      state({
        sessionId: "session-a",
        advanceCommands: { "advance-a": advanceRejected("advance-a") },
      }),
      sendAdvance,
      [],
      "session-a",
    );

    view.rerender(
      <AutopilotHarness
        sessionId="session-b"
        state={state({
          sessionId: "session-a",
          advanceCommands: { "advance-a": advanceRejected("advance-a") },
        })}
        sendAdvance={sendAdvance}
        stopPoints={[]}
      />,
    );
    view.rerender(
      <AutopilotHarness
        sessionId="session-b"
        state={state({ ...confirmedGate("turn-b"), sessionId: "session-b" })}
        sendAdvance={sendAdvance}
        stopPoints={[]}
      />,
    );

    expect(sendAdvance).toHaveBeenCalledTimes(1);
  });

  it("does not advance legacy or group flows", () => {
    const view = renderAutopilot(state({ ...confirmedGate("turn-legacy"), flowKind: "legacy" }));

    expect(view.sendAdvance).not.toHaveBeenCalled();
  });

  it("abandons a busy candidate without allocating a command id", () => {
    const randomUUID = vi.spyOn(crypto, "randomUUID");
    const busy = state({
      ...confirmedGate("turn-busy"),
      humanGateTurn: { ...confirmedGate("turn-busy").humanGateTurn!, status: "busy" },
    });
    const view = renderAutopilot(busy);

    expect(randomUUID).not.toHaveBeenCalled();
    view.rerender(
      <AutopilotHarness
        state={confirmedGate("turn-next")}
        sendAdvance={view.sendAdvance}
        stopPoints={[]}
      />,
    );

    expect(view.sendAdvance).toHaveBeenCalledTimes(1);
    randomUUID.mockRestore();
  });

  it("does not regenerate a command id while transport is unavailable", () => {
    const sendAdvance = vi.fn<(commandId: string) => boolean>(() => false);
    const view = renderAutopilot(confirmedGate("turn-offline"), sendAdvance);
    const commandId = sendAdvance.mock.calls[0]?.[0];

    view.rerender(
      <AutopilotHarness
        state={confirmedGate("turn-offline")}
        sendAdvance={sendAdvance}
        stopPoints={[]}
      />,
    );

    expect(sendAdvance).toHaveBeenCalledWith(commandId);
    expect(sendAdvance).toHaveBeenCalledTimes(1);
  });

  it("does not create an anchor for ordinary confirmed state", () => {
    const view = renderAutopilot(state({ sessionStatus: "confirmed" }));

    expect(view.sendAdvance).not.toHaveBeenCalled();
  });

  it("honors the human gate stop point", () => {
    const view = renderAutopilot(confirmedGate("turn-7"), vi.fn(() => true), ["human_gate"]);

    expect(view.sendAdvance).not.toHaveBeenCalled();
  });

  it("holds advance while automation ownership is unknown", () => {
    const randomUUID = vi.spyOn(crypto, "randomUUID");
    const view = renderAutopilot(state({ ...confirmedGate("turn-7"), automation: null }));

    expect(view.sendAdvance).not.toHaveBeenCalled();
    expect(randomUUID).not.toHaveBeenCalled();
    view.rerender(
      <AutopilotHarness
        state={state({ ...confirmedGate("turn-7"), automation: { owner: "client", enrollment_id: null, policy_revision: null, enabled: false } })}
        sendAdvance={view.sendAdvance}
        stopPoints={[]}
      />,
    );

    expect(view.sendAdvance).toHaveBeenCalledTimes(1);
    randomUUID.mockRestore();
  });

  it("retires sending under server ownership and allocates a fresh command id on client return", () => {
    const view = renderAutopilot(state({ ...confirmedGate("turn-7") }));
    const firstCommandId = view.sendAdvance.mock.calls[0]?.[0];
    if (!firstCommandId) throw new Error("expected client advance command id");

    view.rerender(
      <AutopilotHarness
        state={state({
          ...confirmedGate("turn-7"),
          automation: { owner: "server", enrollment_id: "en-1", policy_revision: 2, enabled: true },
        })}
        sendAdvance={view.sendAdvance}
        stopPoints={[]}
      />,
    );
    expect(view.sendAdvance).toHaveBeenCalledTimes(1);

    view.rerender(
      <AutopilotHarness
        state={state({
          ...confirmedGate("turn-7"),
          automation: { owner: "client", enrollment_id: "en-1", policy_revision: 3, enabled: false },
        })}
        sendAdvance={view.sendAdvance}
        stopPoints={[]}
      />,
    );
    expect(view.sendAdvance).toHaveBeenCalledTimes(2);
    const secondCommandId = view.sendAdvance.mock.calls[1]?.[0];
    expect(secondCommandId).toEqual(expect.any(String));
    expect(secondCommandId).not.toBe(firstCommandId);
  });

  it("keeps stable single send for non-enrolled client across rerenders", () => {
    const view = renderAutopilot(state({ ...confirmedGate("turn-7") }));

    view.rerender(
      <AutopilotHarness
        state={state({ ...confirmedGate("turn-7") })}
        sendAdvance={view.sendAdvance}
        stopPoints={[]}
      />,
    );
    view.rerender(
      <AutopilotHarness
        state={state({ ...confirmedGate("turn-7") })}
        sendAdvance={view.sendAdvance}
        stopPoints={[]}
      />,
    );

    expect(view.sendAdvance).toHaveBeenCalledTimes(1);
  });
});

function AutopilotHarness({
  sessionId = "session-1",
  state,
  sendAdvance,
  stopPoints,
}: {
  sessionId?: string;
  state: AutopilotState;
  sendAdvance: (commandId: string) => boolean;
  stopPoints: readonly ("human_gate" | "stopped" | "hard_error")[];
}) {
  useCockpitAutopilot({
    sessionId,
    state,
    settings: { ...defaultSettings, stopPoints },
    sendAdvance,
  });
  return null;
}
