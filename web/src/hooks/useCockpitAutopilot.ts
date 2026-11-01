import { useEffect, useRef } from "react";
import { newCommandId } from "./useWorkspaceWs";
import { gateIdentityFromState } from "../state/cockpit-action-routing";
import type { CockpitSettings } from "../state/cockpit-settings";
import type { WorkspaceWsState } from "../state/workspace-ws-store-types";

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
  const {
    sessionId: stateSessionId,
    sessionStatus,
    humanGateTurn,
    humanGateClosure,
    advanceCommands,
    flowKind,
    automation,
  } = state;
  const humanGateStopPoint = settings.stopPoints.includes("human_gate");
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

  // P0 1.2（REQ-WIGA-08/D6）：durable 归属切换（owner/revision/enrollment/
  // enabled 任一变化，含未知）即清空 anchors 与停发标记——server 接管期间
  // 本地不遗留旧命令锚点，client 回归后按新 revision 分配新 commandId。
  // 注意：显式 client（含 enabled=false）不视为未知，保留旧行为。
  const ownershipKey = automation
    ? `${sessionId}:${automation.owner}:${automation.enrollment_id ?? ""}:${automation.policy_revision ?? ""}:${automation.enabled}`
    : `${sessionId}:unknown`;
  const ownershipKeyRef = useRef<string | null>(null);
  useEffect(() => {
    if (ownershipKeyRef.current === ownershipKey) {
      return;
    }
    ownershipKeyRef.current = ownershipKey;
    anchorsRef.current.clear();
    stoppedSessionRef.current = null;
  }, [ownershipKey]);

  useEffect(() => {
    // P0 1.2：归属未知（null）或 server 时不发令也不分配 commandId——
    // D6 退位，服务端 autopilot 负责推进。
    if (automation?.owner !== "client") {
      return;
    }
    if (stoppedSessionRef.current === sessionId || stateSessionId !== sessionId) {
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
    if (!gateIdentity || humanGateStopPoint) {
      return;
    }

    if (humanGateTurn?.status === "busy") {
      const busyEntryId = confirmedEntryId(humanGateTurn, gateIdentity);
      const busyAnchorKey = `${sessionId}:${gateIdentity}:${busyEntryId}`;
      const busyAnchor = anchorsRef.current.get(busyAnchorKey) ?? {
        commandId: "",
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
    automation,
    flowKind,
    humanGateClosure,
    humanGateTurn,
    sendAdvance,
    sessionId,
    sessionStatus,
    humanGateStopPoint,
    stateSessionId,
  ]);
}

function confirmedEntryId(
  humanGateTurn: AutopilotState["humanGateTurn"],
  gateIdentity: string,
): string {
  const commandId = humanGateTurn?.command_id;
  return commandId ? `confirmed:${gateIdentity}:${commandId}` : `confirmed:${gateIdentity}`;
}
