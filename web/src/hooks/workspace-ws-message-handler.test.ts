import { describe, expect, it, vi } from "vitest";
import type { WsServerMessage } from "./workspace-ws-message-handler";
import { handleWorkspaceWsMessage, providerName } from "./workspace-ws-message-handler";
import { useWorkspaceStore } from "../state/workspace-ws-store";
import {
  installWorkspaceStoreTestHooks,
  makeContextBlockerArtifactPayload,
} from "../state/workspace-ws-store.test-utils";
import { selectCockpitInbox } from "../state/workspace-cockpit-projection";

describe("workspace websocket provider parser", () => {
  it("accepts kimi_code as a workspace provider", () => {
    expect(providerName("kimi_code")).toBe("kimi_code");
  });

  it("rejects unknown provider names", () => {
    expect(providerName("unknown_provider")).toBeNull();
  });
});

// Bug C live 路径回归：后端时序为先 artifact_update(context_blocker) 再
// stage_change -> human_confirm，且 blocker 之后没有 session_state 全量重建，
// 因此 stage_change 现场构造的 gate_prompt 也必须注入 blocker 元数据并使用
// exploration_summary 内容，行为与 rebuild 路径（workspace-chat-rebuild.ts）一致。
describe("workspace websocket live stage_change gate prompt", () => {
  installWorkspaceStoreTestHooks();

  const handlerOptions = () => ({
    invalidatedPreStageNodeIds: new Set<string>(),
    scheduleFlush: vi.fn(),
    streamFlushTimeouts: {},
  });

  const startLiveWorkItemPlanSession = () => {
    useWorkspaceStore.getState().setSessionState({
      session_id: "session_live_blocker_gate",
      workspace_type: "work_item_plan",
      stage: "running",
      session_status: "running",
      flow_kind: "legacy",
      run_policy: "interactive",
      run_history: {
        seen_fingerprints: [],
        repairs_used: 0,
        manual_repairs_used: 0,
        transitions_used: 0,
        initial_review_count: 0,
        verification_review_count: 0,
      },
      messages: [],
      checkpoints: [],
      artifact: null,
      providers: { author: "claude_code", reviewer: null },
    });
  };

  const gatePromptEntry = () =>
    useWorkspaceStore.getState().chatEntries.find((entry) => entry.type === "gate_prompt");

  it("marks the live human_confirm gate as a context blocker gate", () => {
    const contextBlocker = makeContextBlockerArtifactPayload();
    startLiveWorkItemPlanSession();

    handleWorkspaceWsMessage(
      { type: "artifact_update", version: 3, context_blocker: contextBlocker } as WsServerMessage,
      handlerOptions(),
    );
    handleWorkspaceWsMessage(
      { type: "stage_change", stage: "human_confirm" } as WsServerMessage,
      handlerOptions(),
    );

    const gatePrompt = gatePromptEntry();
    expect(gatePrompt).toBeDefined();
    expect(gatePrompt).toMatchObject({
      type: "gate_prompt",
      content: contextBlocker.exploration_summary,
    });
    expect(gatePrompt?.metadata).toEqual(
      expect.objectContaining({
        gate_kind: "work_item_plan_context_blocker",
        allowed_actions: ["provide_context", "abort"],
      }),
    );
  });

  it("falls back to the waiting content when the blocker exploration summary is empty", () => {
    const contextBlocker = { ...makeContextBlockerArtifactPayload(), exploration_summary: "   " };
    startLiveWorkItemPlanSession();

    handleWorkspaceWsMessage(
      { type: "artifact_update", version: 4, context_blocker: contextBlocker } as WsServerMessage,
      handlerOptions(),
    );
    handleWorkspaceWsMessage(
      { type: "stage_change", stage: "human_confirm" } as WsServerMessage,
      handlerOptions(),
    );

    const gatePrompt = gatePromptEntry();
    expect(gatePrompt).toMatchObject({ content: "等待人工确认" });
    expect(gatePrompt?.metadata).toEqual(
      expect.objectContaining({ gate_kind: "work_item_plan_context_blocker" }),
    );
  });

  it("does not mark a live work item plan gate without a blocker artifact", () => {
    startLiveWorkItemPlanSession();

    handleWorkspaceWsMessage(
      { type: "stage_change", stage: "human_confirm" } as WsServerMessage,
      handlerOptions(),
    );

    const gatePrompt = gatePromptEntry();
    expect(gatePrompt).toBeDefined();
    expect(gatePrompt).toMatchObject({ content: "等待人工确认" });
    expect(gatePrompt?.metadata ?? {}).not.toHaveProperty("gate_kind");
  });

  // workspace-artifact-bug-triage：表驱动覆盖 story/design/work_item 三类 live gate，
  // blocker 标记只允许出现在 work_item_plan context_blocker 场景。
  it.each([["story"], ["design"], ["work_item"]])(
    "does not mark live %s human_confirm gates as context blocker gates",
    (workspaceType) => {
      useWorkspaceStore.getState().setSessionState({
        session_id: `session_live_${workspaceType}_gate`,
        workspace_type: workspaceType,
        stage: "running",
        session_status: "running",
        flow_kind: "legacy",
        run_policy: "interactive",
        run_history: {
          seen_fingerprints: [],
          repairs_used: 0,
          manual_repairs_used: 0,
          transitions_used: 0,
          initial_review_count: 0,
          verification_review_count: 0,
        },
        messages: [],
        checkpoints: [],
        artifact: null,
        providers: { author: "claude_code", reviewer: null },
      });

      handleWorkspaceWsMessage(
        { type: "stage_change", stage: "human_confirm" } as WsServerMessage,
        handlerOptions(),
      );

      const gatePrompt = gatePromptEntry();
      expect(gatePrompt).toBeDefined();
      expect(gatePrompt).toMatchObject({ content: "等待人工确认" });
      expect(gatePrompt?.metadata ?? {}).not.toHaveProperty("gate_kind");
    },
  );
});

// usage 事件按 role 关联到对应 stream 气泡（usage-transparency 契约）
describe("workspace websocket usage event", () => {
  installWorkspaceStoreTestHooks();

  const handlerOptions = () => ({
    invalidatedPreStageNodeIds: new Set<string>(),
    scheduleFlush: vi.fn(),
    streamFlushTimeouts: {},
  });

  it("maps usage execution_event onto the node's stream entry metadata", () => {
    useWorkspaceStore.getState().setSessionState({
      session_id: "session_usage_map",
      workspace_type: "story",
      stage: "running",
      session_status: "running",
      flow_kind: "legacy",
      run_policy: "interactive",
      run_history: {
        seen_fingerprints: [],
        repairs_used: 0,
        manual_repairs_used: 0,
        transitions_used: 0,
        initial_review_count: 0,
        verification_review_count: 0,
      },
      messages: [],
      checkpoints: [],
      artifact: null,
      providers: { author: "claude_code", reviewer: null },
    });

    handleWorkspaceWsMessage(
      { type: "stream_chunk", node_id: "timeline_node_002", role: "author", content: "段落" } as WsServerMessage,
      handlerOptions(),
    );
    useWorkspaceStore.getState().flushBufferedStream("timeline_node_002");
    handleWorkspaceWsMessage(
      {
        type: "execution_event",
        event: {
          event_id: "usage_author",
          node_id: "timeline_node_002",
          agent: "author",
          kind: "usage",
          status: "completed",
          title: "Usage",
          output:
            '{"role":"author","input_tokens":89035,"output_tokens":8896,"cache_read_tokens":230976}',
        },
      } as unknown as WsServerMessage,
      handlerOptions(),
    );

    const entry = useWorkspaceStore
      .getState()
      .chatEntries.find((e) => e.id === "timeline_node_002:stream-active");
    expect(entry?.metadata?.usage).toMatchObject({
      input_tokens: 89035,
      output_tokens: 8896,
      cache_read_tokens: 230976,
    });
  });

  it("ignores malformed usage output without touching other entries", () => {
    const before = useWorkspaceStore.getState().chatEntries.length;
    handleWorkspaceWsMessage(
      {
        type: "execution_event",
        event: {
          event_id: "usage_bad",
          node_id: "timeline_node_002",
          kind: "usage",
          status: "completed",
          title: "Usage",
          output: "not-json",
        },
      } as unknown as WsServerMessage,
      handlerOptions(),
    );
    expect(useWorkspaceStore.getState().chatEntries.length).toBe(before);
  });
});

describe("workspace websocket human gate protocol branches", () => {
  installWorkspaceStoreTestHooks();

  const options = () => ({
    invalidatedPreStageNodeIds: new Set<string>(),
    scheduleFlush: vi.fn(),
    streamFlushTimeouts: {},
  });

  const startTypedGateSession = () => {
    useWorkspaceStore.getState().setSessionState({
      session_id: "session_gate_branches",
      workspace_type: "work_item_plan",
      stage: "running",
      session_status: "waiting_for_human",
      flow_kind: "single_candidate",
      run_policy: "auto_if_valid",
      run_history: {
        seen_fingerprints: [],
        repairs_used: 0,
        manual_repairs_used: 0,
        transitions_used: 0,
        initial_review_count: 1,
        verification_review_count: 0,
      },
      messages: [],
      checkpoints: [],
      artifact: null,
      providers: { author: "claude_code", reviewer: null },
    });
  };

  const gateEntries = () =>
    useWorkspaceStore.getState().chatEntries.filter((entry) => entry.type === "gate_prompt");

  it("opens a typed gate turn and materializes one gate card", () => {
    startTypedGateSession();

    handleWorkspaceWsMessage(
      {
        type: "human_gate_turn_open",
        turn_id: "turn_1",
        command_id: "cmd_1",
        remaining_budget: 2,
      } as WsServerMessage,
      options(),
    );

    expect(useWorkspaceStore.getState().humanGateTurn).toMatchObject({
      turn_id: "turn_1",
      remaining_budget: 2,
      status: "open",
    });
    expect(gateEntries()).toHaveLength(1);
    expect(gateEntries()[0]?.metadata).toMatchObject({ turn_id: "turn_1" });
  });

  it("consumes a replayed turn open idempotently", () => {
    startTypedGateSession();
    const open = {
      type: "human_gate_turn_open",
      turn_id: "turn_1",
      command_id: "cmd_1",
      remaining_budget: 2,
    } as WsServerMessage;

    handleWorkspaceWsMessage(open, options());
    handleWorkspaceWsMessage(open, options());

    expect(gateEntries()).toHaveLength(1);
    expect(useWorkspaceStore.getState().humanGateTurn?.remaining_budget).toBe(2);
  });

  it("moves the gate sub-status to awaiting confirmation on turn completion", () => {
    startTypedGateSession();
    handleWorkspaceWsMessage(
      {
        type: "human_gate_turn_open",
        turn_id: "turn_1",
        command_id: "cmd_1",
        remaining_budget: 2,
      } as WsServerMessage,
      options(),
    );

    handleWorkspaceWsMessage(
      { type: "human_gate_turn_completed", turn_id: "turn_1", artifact_ref: "artifact_9" } as WsServerMessage,
      options(),
    );

    expect(useWorkspaceStore.getState().humanGateTurn).toMatchObject({
      status: "awaiting_confirm",
      artifact_ref: "artifact_9",
    });
    expect(gateEntries()).toHaveLength(1);
  });

  it("keeps the gate visible and inline-fails on turn failure", () => {
    startTypedGateSession();
    handleWorkspaceWsMessage(
      {
        type: "human_gate_turn_open",
        turn_id: "turn_1",
        command_id: "cmd_1",
        remaining_budget: 2,
      } as WsServerMessage,
      options(),
    );

    handleWorkspaceWsMessage(
      {
        type: "human_gate_turn_failed",
        turn_id: "turn_1",
        failure_class: "compile_failed",
        message: "compile failed",
      } as WsServerMessage,
      options(),
    );

    expect(useWorkspaceStore.getState().humanGateTurn).toMatchObject({
      status: "failed",
      failure_class: "compile_failed",
      failure_message: "compile failed",
    });
    expect(gateEntries()).toHaveLength(1);
  });

  it("marks the gate as busy without changing the gate itself", () => {
    startTypedGateSession();
    handleWorkspaceWsMessage(
      {
        type: "human_gate_turn_open",
        turn_id: "turn_1",
        command_id: "cmd_1",
        remaining_budget: 2,
      } as WsServerMessage,
      options(),
    );

    handleWorkspaceWsMessage({ type: "human_gate_busy", turn_id: "turn_1" } as WsServerMessage, options());

    expect(useWorkspaceStore.getState().humanGateTurn?.status).toBe("busy");
    expect(useWorkspaceStore.getState().humanGateClosure).toBeNull();
  });

  it("converges the gate on human_gate_closed instead of dropping it", () => {
    startTypedGateSession();
    const store = useWorkspaceStore.getState();
    store.applyHumanGateTurnOpen("turn_1", "cmd_1", 2);
    store.appendChatEntry({
      id: "turn_1:gate-prompt",
      type: "gate_prompt",
      role: "system",
      content: "等待人工确认",
      timestamp: "2026-09-13T00:00:00Z",
      metadata: { turn_id: "turn_1" },
    });

    handleWorkspaceWsMessage(
      { type: "human_gate_closed", decision: "confirm", stage: "human_confirm" } as WsServerMessage,
      options(),
    );

    expect(useWorkspaceStore.getState().humanGateClosure).toEqual({
      decision: "confirm",
      stage: "human_confirm",
    });
    expect(gateEntries()[0]).toMatchObject({ resolved: true, resolution: "confirm" });
    expect(selectCockpitInbox(useWorkspaceStore.getState())).toHaveLength(0);
  });

  it("records advance completion and rejection by command_id", () => {
    handleWorkspaceWsMessage(
      {
        type: "advance_completed",
        command_id: "cmd_done",
        attempt_id: "coding_attempt_1",
        workspace_entry: "coding",
      } as WsServerMessage,
      options(),
    );
    handleWorkspaceWsMessage(
      {
        type: "advance_rejected",
        command_id: "cmd_rejected",
        code: "ADVANCE_NOT_READY",
        reason: "gate still open",
      } as WsServerMessage,
      options(),
    );

    expect(useWorkspaceStore.getState().advanceCommands).toMatchObject({
      cmd_done: { status: "completed" },
      cmd_rejected: { status: "rejected", code: "ADVANCE_NOT_READY" },
    });
  });

  it("ignores a replayed advance rejection instead of raising a new inbox item", () => {
    handleWorkspaceWsMessage(
      {
        type: "advance_rejected",
        command_id: "cmd_replay",
        code: "ADVANCE_REPLAY_NOT_READY",
        reason: "durable record exists",
      } as WsServerMessage,
      options(),
    );

    expect(selectCockpitInbox(useWorkspaceStore.getState())).toHaveLength(0);
    expect(useWorkspaceStore.getState().advanceCommands.cmd_replay?.status).toBe("rejected");
  });

  it("tolerates an unknown event type and leaves a diagnostic", () => {
    expect(() =>
      handleWorkspaceWsMessage(
        { type: "future_event", payload: 1 } as unknown as WsServerMessage,
        options(),
      ),
    ).not.toThrow();

    const diagnostics = useWorkspaceStore.getState().protocolDiagnostics;
    expect(diagnostics).toHaveLength(1);
    expect(diagnostics[0]).toMatchObject({
      code: "UNRECOGNIZED_EVENT",
      type: "future_event",
    });
    expect(useWorkspaceStore.getState().protocolError).toBeNull();
  });
});
