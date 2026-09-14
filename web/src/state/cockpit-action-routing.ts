import type { WorkspaceWsState } from "./workspace-ws-store-types";

export type CockpitActionFacade = {
  confirm(): void;
  requestChange(payload: CockpitRequestChangePayload): void;
  feedback(feedback: string): void;
  terminate(): void;
};

export type CockpitRequestChangePayload = {
  description: string;
  source: "human" | "review_findings";
};

export function createCockpitActionFacade(input: {
  flowKind: WorkspaceWsState["flowKind"];
  commandId: string | null;
  sendHumanConfirm: (
    decision: "confirm" | "request-change" | "terminate",
    payload?: unknown,
  ) => boolean;
  sendHumanGateFeedback: (feedback: string, commandId?: string) => boolean;
}): CockpitActionFacade {
  return {
    confirm() {
      input.sendHumanConfirm("confirm");
    },
    requestChange(payload) {
      if (input.flowKind !== "single_candidate") {
        input.sendHumanConfirm("request-change", payload);
      }
    },
    feedback(feedback) {
      if (input.flowKind === "single_candidate") {
        input.sendHumanGateFeedback(feedback, input.commandId ?? undefined);
      }
    },
    terminate() {
      input.sendHumanConfirm("terminate");
    },
  };
}


export type ProtocolErrorDisposition =
  | { kind: "gate"; turnId: string }
  | { kind: "advance"; commandId: string }
  | { kind: "hard_error" };

type ProtocolRoutingState = Pick<WorkspaceWsState, "humanGateTurn" | "advanceCommands">;

const GATE_REJECTION_CODES: Record<string, true> = {
  INVALID_HUMAN_CONFIRM_ACTION: true,
  HUMAN_GATE_NOT_READY: true,
  HUMAN_GATE_FEEDBACK_REJECTED: true,
};

const ADVANCE_REPLAY_CODES: Record<string, true> = {
  ADVANCE_REPLAY_NOT_READY: true,
  ADVANCE_REPLAY_INCOMPLETE: true,
};

export function classifyProtocolError(
  code: string,
  context: unknown,
  state: ProtocolRoutingState,
): ProtocolErrorDisposition {
  const turnId = contextString(context, "turn_id");
  if (
    turnId !== null &&
    GATE_REJECTION_CODES[code] === true &&
    state.humanGateTurn?.turn_id === turnId
  ) {
    return { kind: "gate", turnId };
  }

  const commandId = contextString(context, "command_id");
  if (
    commandId !== null &&
    ADVANCE_REPLAY_CODES[code] === true &&
    state.advanceCommands[commandId] !== undefined
  ) {
    return { kind: "advance", commandId };
  }

  return { kind: "hard_error" };
}

export function gateIdentityFromState(
  state: Pick<WorkspaceWsState, "humanGateTurn" | "humanGateSnapshot" | "stage">,
): string | null {
  if (state.humanGateTurn) {
    return state.humanGateTurn.turn_id;
  }
  if (state.humanGateSnapshot?.opened_at) {
    return `snapshot:${state.humanGateSnapshot.opened_at}:${snapshotGateFingerprint(state.humanGateSnapshot)}`;
  }
  if (state.humanGateSnapshot) {
    return `snapshot:${state.stage}`;
  }
  return state.stage === "human_confirm" ? `legacy:${state.stage}` : null;
}

export function snapshotGateFingerprint(
  snapshot: Pick<
    NonNullable<WorkspaceWsState["humanGateSnapshot"]>,
    "trigger" | "manual_repairs_remaining" | "resumable" | "repeated_fingerprints" | "findings"
  >,
): string {
  return `${snapshot.trigger}|${snapshot.manual_repairs_remaining}|${snapshot.resumable}|${snapshot.repeated_fingerprints.join(",")}|${snapshot.findings.map((finding) => finding.fingerprint).join(",")}`;
}

function contextString(context: unknown, key: string): string | null {
  if (typeof context !== "object" || context === null) {
    return null;
  }
  const value = (context as Record<string, unknown>)[key];
  return typeof value === "string" ? value : null;
}
