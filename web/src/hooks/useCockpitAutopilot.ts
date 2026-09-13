import { useEffect, useRef } from "react";
import { newCommandId } from "./useWorkspaceWs";
import { gateIdentityFromState } from "../state/cockpit-action-routing";
import type { CockpitSettings } from "../state/cockpit-settings";
import type { WorkspaceWsState } from "../state/workspace-ws-store-types";

type AutopilotState = Pick<
  WorkspaceWsState,
  "sessionStatus" | "humanGateTurn" | "humanGateClosure" | "advanceCommands" | "flowKind"
>;

interface AutopilotAnchor {
  commandId: string;
  sent: boolean;
  stopped: boolean;
}

export interface CockpitAutopilotInput {
  sessionId: string;
  state: AutopilotState;
  settings: CockpitSettings;
  sendAdvance: (commandId: string) => boolean;
}


export function useCockpitAutopilot({
  sessionId,
  state,
  settings,
  sendAdvance,
}: CockpitAutopilotInput): void {
  const { sessionStatus, humanGateTurn, humanGateClosure, advanceCommands, flowKind } = state;
  const stopPoints = settings.stopPoints;
  const anchorsRef = useRef(new Map<string, AutopilotAnchor>());
  const stoppedSessionRef = useRef<string | null>(null);
  const sessionRef = useRef(sessionId);

  useEffect(() => {
    if (sessionRef.current === sessionId) {
      return;
    }
    sessionRef.current = sessionId;
    anchorsRef.current.clear();
    stoppedSessionRef.current = null;
  }, [sessionId]);

  useEffect(() => {
    if (stoppedSessionRef.current === sessionId) {
      return;
    }

    const rejected = Object.values(advanceCommands).some(
      (command) => command.status === "rejected",
    );
    if (rejected) {
      stoppedSessionRef.current = sessionId;
      return;
    }

    if (flowKind !== "single_candidate") {
      return;
    }

    const gateIdentity = gateIdentityFromState({
      humanGateTurn,
      humanGateSnapshot: null,
      stage: humanGateClosure?.stage ?? "",
    });
    if (!gateIdentity || stopPoints.includes("human_gate")) {
      return;
    }

    if (humanGateTurn?.status === "busy") {
      const busyEntryId = confirmedEntryId(humanGateTurn, gateIdentity);
      const busyAnchorKey = `${sessionId}:${gateIdentity}:${busyEntryId}`;
      const busyAnchor = anchorsRef.current.get(busyAnchorKey) ?? {
        commandId: newCommandId(),
        sent: false,
        stopped: false,
      };
      busyAnchor.stopped = true;
      anchorsRef.current.set(busyAnchorKey, busyAnchor);
      return;
    }

    if (humanGateTurn?.status === "failed") {
      return;
    }

    const gateConfirmed = humanGateClosure?.decision === "confirm";
    const sessionConfirmed = sessionStatus === "confirmed";
    if (!gateConfirmed && !sessionConfirmed) {
      return;
    }

    const entryId = confirmedEntryId(humanGateTurn, gateIdentity);
    const anchorKey = `${sessionId}:${gateIdentity}:${entryId}`;
    const anchor = anchorsRef.current.get(anchorKey) ?? {
      commandId: newCommandId(),
      sent: false,
      stopped: false,
    };
    anchorsRef.current.set(anchorKey, anchor);

    if (anchor.sent || anchor.stopped) {
      return;
    }

    sendAdvance(anchor.commandId);
    anchor.sent = true;
  }, [
    advanceCommands,
    flowKind,
    humanGateClosure,
    humanGateTurn,
    sendAdvance,
    sessionId,
    sessionStatus,
    stopPoints,
  ]);
}

function confirmedEntryId(
  humanGateTurn: AutopilotState["humanGateTurn"],
  gateIdentity: string,
): string {
  const commandId = humanGateTurn?.command_id;
  return commandId ? `confirmed:${gateIdentity}:${commandId}` : `confirmed:${gateIdentity}`;
}
