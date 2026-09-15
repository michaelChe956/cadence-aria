import type { CodingTimelineNode } from "../api/types";
import {
  OPERATOR_LABEL,
  type OperationAuditRecord,
} from "./cockpit-operation-semantics";
import type { CodingWorkspaceState } from "./coding-workspace-store";
import { selectGateProjection } from "./workspace-cockpit-projection";
import type { WorkspaceWsState } from "./workspace-ws-store";

export interface OperationAuditTarget {
  sessionId: string;
  gateId: string | null;
}

export interface OperationAuditRow extends OperationAuditRecord {
  evidence: "local_command" | "session_state" | "rest_snapshot";
}

export function selectOperationAuditRows(input: {
  local: readonly OperationAuditRecord[];
  workspaceState: WorkspaceWsState | null;
  codingState: CodingWorkspaceState | null;
  target: OperationAuditTarget | null;
}): OperationAuditRow[] {
  const rows = [
    ...input.local.map((record) => ({ ...record, evidence: "local_command" as const })),
    ...workspaceAuditRows(input.workspaceState),
    ...codingAuditRows(input.codingState),
  ];

  return rows
    .filter((row) => matchesTarget(row, input.target))
    .sort(compareAuditRows);
}

function workspaceAuditRows(state: WorkspaceWsState | null): OperationAuditRow[] {
  if (state === null || state.sessionId === null) {
    return [];
  }
  const sessionId = state.sessionId;

  const gateId = selectGateProjection(state)?.key ?? null;
  const closure = state.humanGateClosure;
  const closureRows: OperationAuditRow[] = closure
    ? [{
        id: `session_state:closure:${sessionId}:${gateId ?? closure.stage}:${closure.decision}`,
        atMs: null,
        atIso: null,
        operator: OPERATOR_LABEL,
        sessionId,
        gateId,
        operation: closure.decision,
        source: "system_recovery",
        outcome: "completed",
        detail: closure.stage,
        evidence: "session_state",
      }]
    : [];
  const advanceRows = Object.values(state.advanceCommands)
    .filter((command) => command.status !== "pending")
    .map<OperationAuditRow>((command) => ({
      id: `session_state:advance:${sessionId}:${command.command_id}`,
      atMs: null,
      atIso: null,
      operator: OPERATOR_LABEL,
      sessionId,
      gateId,
      operation: "advance",
      source: "system_recovery",
      outcome: command.status === "completed" ? "completed" : "rejected",
      detail: command.code ?? command.reason,
      evidence: "session_state",
    }));

  return [...closureRows, ...advanceRows];
}

function codingAuditRows(state: CodingWorkspaceState | null): OperationAuditRow[] {
  if (state === null || state.attemptId === null) {
    return [];
  }
  const sessionId = state.attemptId;

  const completedFinalConfirm = state.timelineNodes.find(
    (node) => node.stage === "final_confirm" && node.status === "completed",
  );
  if (completedFinalConfirm) {
    return [codingTimelineRow(sessionId, completedFinalConfirm)];
  }

  if (state.status !== "completed" || state.pendingGates.length > 0) {
    return [];
  }

  return [{
    id: `rest_snapshot:coding-completed:${sessionId}`,
    atMs: null,
    atIso: null,
    operator: OPERATOR_LABEL,
    sessionId,
    gateId: null,
    operation: "final_confirm",
    source: "system_recovery",
    outcome: "completed",
    detail: null,
    evidence: "rest_snapshot",
  }];
}

function codingTimelineRow(sessionId: string, node: CodingTimelineNode): OperationAuditRow {
  return {
    id: `rest_snapshot:coding-final-confirm:${sessionId}:${node.id}`,
    atMs: null,
    atIso: null,
    operator: OPERATOR_LABEL,
    sessionId,
    gateId: null,
    operation: "final_confirm",
    source: "system_recovery",
    outcome: "completed",
    detail: node.id,
    evidence: "rest_snapshot",
  };
}

function matchesTarget(row: OperationAuditRow, target: OperationAuditTarget | null): boolean {
  return target === null || (
    row.sessionId === target.sessionId &&
    (target.gateId === null || row.gateId === target.gateId)
  );
}

function compareAuditRows(left: OperationAuditRow, right: OperationAuditRow): number {
  if (left.atMs === null) return right.atMs === null ? 0 : 1;
  if (right.atMs === null) return -1;
  return right.atMs - left.atMs;
}
