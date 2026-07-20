import { act } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { useWorkspaceStore } from "../state/workspace-ws-store";
import {
  installWorkspaceWsTestHooks,
  renderWorkspaceHook,
} from "./useWorkspaceWs.test-utils";

describe("useWorkspaceWs plan repair actions", () => {
  installWorkspaceWsTestHooks();

  it("sends the authoritative plan amendment confirmation once", () => {
    const harness = renderWorkspaceHook("workspace_session_repair_0001");
    let sent = false;
    act(() => {
      harness.ws.open();
      harness.ws.receive(sessionState("workspace_session_repair_0001"));
      harness.ws.sent.length = 0;
      sent = harness.api.confirmPlanAmendment("plan_amendment_0001");
    });

    expect(sent).toBe(true);
    expect(harness.ws.sent).toEqual([
      JSON.stringify({
        type: "confirm_plan_amendment",
        amendment_id: "plan_amendment_0001",
      }),
    ]);
  });

  it("sends cancellation and revision requests through existing workspace messages", () => {
    const harness = renderWorkspaceHook("workspace_session_repair_0001");
    let cancelled = false;
    let requested = false;
    act(() => {
      harness.ws.open();
      harness.ws.receive(sessionState("workspace_session_repair_0001"));
      harness.ws.sent.length = 0;
      cancelled = harness.api.cancelPlanAmendment(
        "plan_amendment_0001",
        " 用户取消修订 ",
      );
      requested = harness.api.sendHumanConfirm("request-change", {
        description: "调整 Plan Repair 修订范围",
      });
    });

    expect(cancelled).toBe(true);
    expect(requested).toBe(true);
    expect(harness.ws.sent).toEqual([
      JSON.stringify({
        type: "cancel_plan_amendment",
        amendment_id: "plan_amendment_0001",
        reason: "用户取消修订",
      }),
      JSON.stringify({
        type: "human_confirm",
        decision: "request-change",
        payload: { description: "调整 Plan Repair 修订范围" },
      }),
    ]);
  });

  it("keeps socket sends and readiness isolated when the child session changes", () => {
    const harness = renderWorkspaceHook("workspace_session_repair_A");
    let staleApi = harness.api;
    act(() => {
      harness.ws.open();
      harness.ws.receive(sessionState("workspace_session_repair_A"));
      harness.ws.receive({
        type: "protocol_error",
        code: "PLAN_AMENDMENT_CONFIRMATION_FAILED",
        message: "A conflict",
      });
    });
    staleApi = harness.api;
    expect(staleApi.connectionStatus).toBe("connected");
    expect(useWorkspaceStore.getState().protocolError?.message).toBe("A conflict");

    act(() => {
      harness.switchSession("workspace_session_repair_B");
    });
    const socketB = harness.latestWs;
    expect(harness.ws.closeCodes).toContain(1000);
    expect(harness.api.connectionStatus).not.toBe("connected");

    act(() => {
      socketB.open();
    });
    expect(harness.api.connectionStatus).not.toBe("connected");

    act(() => {
      socketB.receive(sessionState("workspace_session_repair_B"));
    });
    expect(harness.api.connectionStatus).toBe("connected");

    socketB.sent.length = 0;
    let staleSend = true;
    act(() => {
      staleSend = staleApi.confirmPlanAmendment("plan_amendment_A");
    });
    expect(staleSend).toBe(false);
    expect(socketB.sent).toEqual([]);
  });
});

function sessionState(sessionId: string) {
  return {
    type: "session_state",
    session_id: sessionId,
    workspace_type: "work_item",
    stage: "human_confirm",
    superpowers_enabled: false,
    openspec_enabled: false,
    messages: [],
    checkpoints: [],
    artifact: null,
    providers: { author: "claude_code", reviewer: "codex" },
    timeline_nodes: [],
    active_node_id: null,
    artifact_versions: [],
    timeline_node_details: {},
    active_run_id: null,
    human_presentation_revisions: [],
  };
}
