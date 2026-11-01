import { act } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import {
  installWorkspaceWsTestHooks,
  renderWorkspaceHook,
} from "./useWorkspaceWs.test-utils";
import { newCommandId } from "./useWorkspaceWs";

describe("useWorkspaceWs gate commands", () => {
  installWorkspaceWsTestHooks();

  it("sends human gate feedback with the caller-provided command_id", () => {
    const harness = renderWorkspaceHook();

    act(() => {
      harness.ws.open();
      harness.ws.sent.length = 0;
      harness.api.sendHumanGateFeedback("  验收命令缺失  ", "cmd_feedback_1");
    });

    expect(harness.ws.sent).toEqual([
      JSON.stringify({
        type: "human_gate_feedback",
        command_id: "cmd_feedback_1",
        feedback: "验收命令缺失",
      }),
    ]);
  });

  it("reuses the same command_id when a retry re-sends the same action", () => {
    const harness = renderWorkspaceHook();

    act(() => {
      harness.ws.open();
      harness.ws.sent.length = 0;
      harness.api.sendHumanGateFeedback("第一次", "cmd_shared_1");
      harness.api.sendHumanGateFeedback("重试", "cmd_shared_1");
    });

    const payloads = harness.ws.sent.map((raw) => JSON.parse(raw) as { command_id: string });
    expect(payloads.map((payload) => payload.command_id)).toEqual([
      "cmd_shared_1",
      "cmd_shared_1",
    ]);
  });

  it("refuses empty feedback without sending anything", () => {
    const harness = renderWorkspaceHook();

    act(() => {
      harness.ws.open();
      harness.ws.sent.length = 0;
    });

    let sent = true;
    act(() => {
      sent = harness.api.sendHumanGateFeedback("   ", "cmd_empty_1") as boolean;
    });

    expect(sent).toBe(false);
    expect(harness.ws.sent).toEqual([]);
  });

  it("sends advance and generates a uuid command_id when none is provided", () => {
    const harness = renderWorkspaceHook();

    act(() => {
      harness.ws.open();
      harness.ws.sent.length = 0;
      harness.api.sendAdvance();
    });

    const payload = JSON.parse(harness.ws.sent[0] ?? "{}") as {
      type: string;
      command_id: string;
    };
    expect(payload.type).toBe("advance");
    expect(payload.command_id).toMatch(
      /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/,
    );
    expect(newCommandId()).not.toBe(newCommandId());
  });
});
