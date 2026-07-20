import { act } from "@testing-library/react";
import { describe, expect, it } from "vitest";
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
});
