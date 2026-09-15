import { describe, expect, it } from "vitest";
import type { OperationAuditRecord } from "./cockpit-operation-semantics";
import { selectOperationAuditRows } from "./operation-audit-projection";
import type { CodingWorkspaceState } from "./coding-workspace-store";
import { useCodingWorkspaceStore } from "./coding-workspace-store";
import type { WorkspaceWsState } from "./workspace-ws-store";
import { useWorkspaceStore } from "./workspace-ws-store";

function record(
  overrides: Partial<OperationAuditRecord> = {},
): OperationAuditRecord {
  return {
    id: "record_1",
    atMs: 1_726_000_000_000,
    atIso: "2024-09-27T00:00:00.000Z",
    operator: "本地浏览器",
    sessionId: "s1",
    gateId: "g1",
    operation: "confirm",
    source: "chat",
    outcome: "sent",
    detail: null,
    ...overrides,
  };
}

function workspaceStateWith(overrides: Partial<WorkspaceWsState>): WorkspaceWsState {
  return {
    ...useWorkspaceStore.getState(),
    sessionId: "s1",
    ...overrides,
  };
}

function codingStateWith(overrides: Partial<CodingWorkspaceState>): CodingWorkspaceState {
  return {
    ...useCodingWorkspaceStore.getState(),
    attemptId: "coding_attempt_1",
    ...overrides,
  };
}

describe("operation audit projection", () => {
  it("filters audit rows by both session and gate target", () => {
    const rows = selectOperationAuditRows({
      local: [record({ gateId: "g1" }), record({ id: "record_2", gateId: "g2" })],
      workspaceState: null,
      codingState: null,
      target: { sessionId: "s1", gateId: "g1" },
    });

    expect(rows).toHaveLength(1);
    expect(rows[0]?.gateId).toBe("g1");
  });

  it("derives durable advance and closure facts with null command time", () => {
    const rows = selectOperationAuditRows({
      local: [],
      workspaceState: workspaceStateWith({
        stage: "human_confirm",
        humanGateClosure: { decision: "confirm", stage: "human_confirm" },
        advanceCommands: {
          c1: {
            command_id: "c1",
            status: "completed",
            code: null,
            reason: null,
            attempt_id: "a1",
            workspace_entry: "entry1",
            inlineError: null,
          },
        },
      }),
      codingState: null,
      target: null,
    });

    expect(rows).toEqual(expect.arrayContaining([
      expect.objectContaining({
        operation: "confirm",
        operator: "本地浏览器",
        atMs: null,
        atIso: null,
        evidence: "session_state",
      }),
      expect.objectContaining({
        operation: "advance",
        outcome: "completed",
        evidence: "session_state",
      }),
    ]));
  });

  it("orders timed local commands before durable facts without a command time", () => {
    const rows = selectOperationAuditRows({
      local: [record()],
      workspaceState: workspaceStateWith({
        humanGateClosure: { decision: "confirm", stage: "human_confirm" },
      }),
      codingState: null,
      target: null,
    });

    expect(rows[0]?.evidence).toBe("local_command");
    expect(rows.at(-1)?.evidence).toBe("session_state");
  });

  it("does not invent final confirmation for a completed WorkItem review request without a timeline node", () => {
    const rows = selectOperationAuditRows({
      local: [],
      workspaceState: null,
      codingState: codingStateWith({
        attemptScope: "work_item",
        status: "completed",
        stage: "code_review",
        timelineNodes: [],
        pendingGates: [],
      }),
      target: null,
    });

    expect(rows).toEqual([]);
  });

  it("derives final confirmation only from a completed timeline node", () => {
    const rows = selectOperationAuditRows({
      local: [],
      workspaceState: null,
      codingState: codingStateWith({
        status: "completed",
        timelineNodes: [{
          id: "final_confirm_1",
          attempt_id: "coding_attempt_1",
          stage: "final_confirm",
          title: "最终确认",
          status: "completed",
          agent_role: "system",
          summary: null,
          started_at: "2026-09-16T00:00:00.000Z",
          completed_at: "2026-09-16T00:01:00.000Z",
          artifact_refs: [],
        }],
      }),
      target: null,
    });

    expect(rows).toEqual([expect.objectContaining({
      id: "rest_snapshot:coding-final-confirm:coding_attempt_1:final_confirm_1",
      operation: "final_confirm",
      outcome: "completed",
      evidence: "rest_snapshot",
    })]);
  });
});
