import { describe, expect, it } from "vitest";
import type { ChatEntry } from "./chat-entries";
import { useWorkspaceStore } from "./workspace-ws-store";
import { installWorkspaceStoreTestHooks } from "./workspace-ws-store.test-utils";

function gateCard(id: string, gateIdentity: string): ChatEntry {
  return {
    id,
    type: "gate_prompt",
    role: "system",
    content: "等待人工确认",
    timestamp: "2026-09-24T05:29:45.514Z",
    metadata: { gate_identity: gateIdentity, action_facade: "typed", action_block_reason: null },
  };
}

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

  // F-49 A1：门内轮次切换即收口旧门卡——任一时刻只允许一张可动作门卡。
  // 判据是门身份而非条目 id：同一次门内存活期内门载体从 durable snapshot 切到
  // typed turn，id 随之变化（snapshot:<opened_at>:<fp> → <turn_id>），只有身份
  // 能识别「同一张门」。
  it("archives the previous round's gate card when a new round opens (F-49 A1)", () => {
    const store = useWorkspaceStore.getState();
    store.appendChatEntry(gateCard("timeline_node_006:gate-prompt", "snapshot:2026-09-24T05:29:45.514Z|2|true||"));
    store.recordGateFeedbackSubmission("timeline_node_006:gate-prompt", "在修复一下 review 审核出来的问题吧");

    store.upsertGatePromptEntry(gateCard("turn_1:gate-prompt", "turn_1"));

    const gates = useWorkspaceStore
      .getState()
      .chatEntries.filter((entry) => entry.type === "gate_prompt");
    expect(gates).toHaveLength(2);
    expect(gates[0]).toMatchObject({ resolved: true, resolution: "superseded" });
    expect(gates[0]?.metadata).toMatchObject({
      gate_archive_round: 1,
      gate_archive_note: "第 1 轮已提交反馈：在修复一下 review 审核出来的问题吧",
    });
    expect(gates.filter((entry) => entry.resolved !== true)).toHaveLength(1);
    expect(gates[1]?.id).toBe("turn_1:gate-prompt");
  });

  it("truncates the archived feedback snippet to fifty characters (F-49 B5)", () => {
    const store = useWorkspaceStore.getState();
    const longFeedback = "补".repeat(80);
    store.appendChatEntry(gateCard("timeline_node_006:gate-prompt", "snapshot:A"));
    store.recordGateFeedbackSubmission("timeline_node_006:gate-prompt", longFeedback);

    store.upsertGatePromptEntry(gateCard("turn_1:gate-prompt", "turn_1"));

    expect(
      useWorkspaceStore.getState().chatEntries[0]?.metadata?.gate_archive_note,
    ).toBe(`第 1 轮已提交反馈：${"补".repeat(50)}`);
  });

  it("archives a superseded card without feedback under the closed-round copy (F-49 B5)", () => {
    const store = useWorkspaceStore.getState();
    store.appendChatEntry(gateCard("timeline_node_006:gate-prompt", "snapshot:A"));

    store.upsertGatePromptEntry(gateCard("turn_1:gate-prompt", "turn_1"));

    expect(
      useWorkspaceStore.getState().chatEntries[0]?.metadata?.gate_archive_note,
    ).toBe("第 1 轮已收口");
  });

  it("numbers archive rounds in order across rounds (F-49 B5)", () => {
    const store = useWorkspaceStore.getState();
    store.appendChatEntry(gateCard("card_1", "snapshot:A"));
    store.upsertGatePromptEntry(gateCard("turn_1:gate-prompt", "turn_1"));
    store.upsertGatePromptEntry(gateCard("turn_2:gate-prompt", "turn_2"));

    const gates = useWorkspaceStore
      .getState()
      .chatEntries.filter((entry) => entry.type === "gate_prompt");
    expect(gates.map((entry) => entry.metadata?.gate_archive_round)).toEqual([1, 2, undefined]);
  });

  it("keeps one gate card when the same gate identity is upserted again (F-49 A1)", () => {
    const store = useWorkspaceStore.getState();
    store.upsertGatePromptEntry(gateCard("turn_1:gate-prompt", "turn_1"));
    store.upsertGatePromptEntry(gateCard("turn_1:gate-prompt", "turn_1"));

    const gates = useWorkspaceStore
      .getState()
      .chatEntries.filter((entry) => entry.type === "gate_prompt");
    expect(gates).toHaveLength(1);
    expect(gates[0]?.resolved).toBeUndefined();
  });

  // F-49 A3：human_gate_closed 此前只 resolve 倒序命中的第一张未决门卡；门内轮次
  // 切换后残留的旧卡永不被收口，关门后仍留可点旧卡。
  it("resolves every unresolved gate card on a gate closure (F-49 A3)", () => {
    const store = useWorkspaceStore.getState();
    store.appendChatEntry(gateCard("card_old", "snapshot:A"));
    store.appendChatEntry(gateCard("card_current", "turn_1"));

    store.resolveGateEntry("confirm");

    const gates = useWorkspaceStore
      .getState()
      .chatEntries.filter((entry) => entry.type === "gate_prompt");
    expect(gates).toHaveLength(2);
    for (const gate of gates) {
      expect(gate).toMatchObject({ resolved: true, resolution: "confirm" });
    }
  });

  it("records the submitted feedback on the card that carries it (F-49 A4)", () => {
    const store = useWorkspaceStore.getState();
    store.appendChatEntry(gateCard("card_current", "turn_1"));

    store.recordGateFeedbackSubmission("card_current", "请补齐边界");

    expect(useWorkspaceStore.getState().chatEntries[0]?.metadata).toMatchObject({
      submitted_feedback: "请补齐边界",
    });
  });

  it("ignores a feedback record for a card that is not in the stream", () => {
    useWorkspaceStore.getState().recordGateFeedbackSubmission("missing", "请补齐边界");

    expect(useWorkspaceStore.getState().chatEntries).toEqual([]);
  });
});
