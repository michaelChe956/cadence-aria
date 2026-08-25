import { describe, expect, it, vi } from "vitest";
import type { WsServerMessage } from "./workspace-ws-message-handler";
import { handleWorkspaceWsMessage, providerName } from "./workspace-ws-message-handler";
import { useWorkspaceStore } from "../state/workspace-ws-store";
import {
  installWorkspaceStoreTestHooks,
  makeContextBlockerArtifactPayload,
} from "../state/workspace-ws-store.test-utils";

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
