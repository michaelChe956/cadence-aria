import { describe, expect, it } from "vitest";
import { useWorkspaceStore } from "./workspace-ws-store";
import { installWorkspaceStoreTestHooks } from "./workspace-ws-store.test-utils";

describe("workspace ws store human gate/advance state", () => {
  installWorkspaceStoreTestHooks();

  it("records an opened gate turn", () => {
    const store = useWorkspaceStore.getState();
    store.applyHumanGateTurnOpen("turn_1", "cmd_1", 2);

    expect(useWorkspaceStore.getState().humanGateTurn).toMatchObject({
      turn_id: "turn_1",
      command_id: "cmd_1",
      remaining_budget: 2,
      status: "open",
      failure_class: null,
    });
  });

  it("ignores a replayed turn open for the same turn_id without re-counting budget", () => {
    const store = useWorkspaceStore.getState();
    store.applyHumanGateTurnOpen("turn_1", "cmd_1", 2);
    const openedAt = useWorkspaceStore.getState().humanGateTurn?.opened_at;

    store.applyHumanGateTurnOpen("turn_1", "cmd_1", 5);

    expect(useWorkspaceStore.getState().humanGateTurn).toMatchObject({
      turn_id: "turn_1",
      remaining_budget: 2,
      opened_at: openedAt,
    });
  });

  it("replaces the turn and clears a stale closure for a new turn_id", () => {
    const store = useWorkspaceStore.getState();
    store.applyHumanGateTurnOpen("turn_1", "cmd_1", 2);
    store.applyHumanGateClosed("confirm", "human_confirm");

    store.applyHumanGateTurnOpen("turn_2", "cmd_2", 1);

    expect(useWorkspaceStore.getState().humanGateTurn?.turn_id).toBe("turn_2");
    expect(useWorkspaceStore.getState().humanGateClosure).toBeNull();
  });

  it("moves the turn sub-status through completed, failed and busy", () => {
    const store = useWorkspaceStore.getState();
    store.applyHumanGateTurnOpen("turn_1", "cmd_1", 2);

    store.applyHumanGateTurnCompleted("turn_1", "artifact_9");
    expect(useWorkspaceStore.getState().humanGateTurn).toMatchObject({
      status: "awaiting_confirm",
      artifact_ref: "artifact_9",
    });

    store.applyHumanGateTurnFailed("turn_1", "compile_failed", "compile failed");
    expect(useWorkspaceStore.getState().humanGateTurn).toMatchObject({
      status: "failed",
      failure_class: "compile_failed",
      failure_message: "compile failed",
    });

    store.applyHumanGateBusy("turn_1");
    expect(useWorkspaceStore.getState().humanGateTurn).toMatchObject({ status: "busy" });
  });

  it("records a gate closure with its stage", () => {
    useWorkspaceStore.getState().applyHumanGateClosed("terminate", "human_confirm");

    expect(useWorkspaceStore.getState().humanGateClosure).toEqual({
      decision: "terminate",
      stage: "human_confirm",
    });
  });

  it("keeps the first terminal advance outcome for a command_id", () => {
    const store = useWorkspaceStore.getState();
    store.applyAdvanceCompleted("cmd_1", "attempt_1", "coding");

    store.applyAdvanceRejected("cmd_1", "ADVANCE_REPLAY_NOT_READY", "late replay");

    expect(useWorkspaceStore.getState().advanceCommands.cmd_1).toMatchObject({
      status: "completed",
      attempt_id: "attempt_1",
      code: null,
    });
  });

  it("records rejections with their code and reason", () => {
    useWorkspaceStore
      .getState()
      .applyAdvanceRejected("cmd_9", "ADVANCE_NOT_READY", "gate is still open");

    expect(useWorkspaceStore.getState().advanceCommands.cmd_9).toMatchObject({
      status: "rejected",
      code: "ADVANCE_NOT_READY",
      reason: "gate is still open",
    });
  });

  it("keeps at most fifty protocol diagnostics", () => {
    const store = useWorkspaceStore.getState();
    for (let index = 0; index < 60; index += 1) {
      store.recordProtocolDiagnostic({
        code: "UNRECOGNIZED_EVENT",
        message: `event ${index}`,
        at: `2026-09-13T00:00:${String(index).padStart(2, "0")}Z`,
        type: `future_event_${index}`,
      });
    }

    const diagnostics = useWorkspaceStore.getState().protocolDiagnostics;
    expect(diagnostics).toHaveLength(50);
    expect(diagnostics[0]?.message).toBe("event 10");
    expect(diagnostics.at(-1)?.message).toBe("event 59");
  });
});
