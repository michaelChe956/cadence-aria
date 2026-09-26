import { describe, expect, it, vi } from "vitest";
import type { WsServerMessage } from "./workspace-ws-message-handler";
import { handleWorkspaceWsMessage, providerName } from "./workspace-ws-message-handler";
import { useWorkspaceStore } from "../state/workspace-ws-store";
import {
  installWorkspaceStoreTestHooks,
  makeContextBlockerArtifactPayload,
} from "../state/workspace-ws-store.test-utils";
import { selectCockpitInbox } from "../state/workspace-cockpit-projection";
import { linkedWorkspaceAmendmentSnapshotFixture } from "../components/coding-workspace/plan-repair-test-fixtures";

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

  // F-20：story/design 的 AuthorConfirm 即人工门——live stage_change 必须落门卡
  // 条目（否则仅在 session_state 全量重建时才可见，门开瞬间无决策面）。
  it.each([["story"], ["design"]])(
    "builds the live author_confirm gate prompt for a %s session",
    (workspaceType) => {
      useWorkspaceStore.getState().setSessionState({
        session_id: `session_live_${workspaceType}_author_gate`,
        workspace_type: workspaceType,
        stage: "author_run",
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
        { type: "stage_change", stage: "author_confirm" } as WsServerMessage,
        handlerOptions(),
      );

      const gatePrompt = gatePromptEntry();
      expect(gatePrompt).toBeDefined();
      expect(gatePrompt).toMatchObject({ type: "gate_prompt" });
      expect(gatePrompt?.metadata).toEqual(
        expect.objectContaining({
          gate_identity: "stage:author_confirm",
          action_facade: "legacy",
        }),
      );
    },
  );

  it("does not build a live author_confirm gate prompt for work_item_plan sessions", () => {
    startLiveWorkItemPlanSession();

    handleWorkspaceWsMessage(
      { type: "stage_change", stage: "author_confirm" } as WsServerMessage,
      handlerOptions(),
    );

    expect(gatePromptEntry()).toBeUndefined();
  });
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

  // F-49 A1：live 路径此前把新门卡 append 成第二张（旧卡 id 随门载体
  // node_id→turn_id 变化），两张卡外观等同且都可动作，而页面只挂一个动作门面 ⇒
  // 点旧卡实际作用于当前门。轮次切换必须把旧卡收口为只读留档（B5）。
  it("archives the open snapshot card when the typed revision turn opens (F-49 A1)", () => {
    useWorkspaceStore.getState().setSessionState({
      session_id: "session_gate_supersede",
      workspace_type: "work_item_plan",
      stage: "human_confirm",
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
      human_gate_snapshot: {
        findings: [],
        repeated_fingerprints: [],
        attempts_used: 1,
        manual_repairs_remaining: 2,
        trigger: "native_human_required",
        resumable: true,
      },
      timeline_nodes: [
        {
          node_id: "timeline_node_006",
          node_type: "human_confirm",
          agent: null,
          stage: "human_confirm",
          round: null,
          status: "active",
          title: "人工确认",
          summary: null,
          started_at: "2026-09-24T05:29:45.514Z",
          completed_at: null,
          duration_ms: null,
          artifact_ref: null,
          provider_config_snapshot: { author: "claude_code", reviewer: null, review_rounds: 1 },
        },
      ],
      active_node_id: "timeline_node_006",
      messages: [],
      checkpoints: [],
      artifact: null,
      providers: { author: "claude_code", reviewer: null },
    });
    useWorkspaceStore.getState().rebuildChatEntries();
    const openCard = gateEntries()[0];
    expect(openCard?.id).toBe("timeline_node_006:gate-prompt");
    useWorkspaceStore
      .getState()
      .recordGateFeedbackSubmission(openCard?.id ?? "", "在修复一下 review 审核出来的问题吧");

    handleWorkspaceWsMessage(
      {
        type: "human_gate_turn_open",
        turn_id: "turn_1",
        command_id: "cmd_1",
        remaining_budget: 2,
      } as WsServerMessage,
      options(),
    );

    const gates = gateEntries();
    expect(gates).toHaveLength(2);
    expect(gates.filter((entry) => entry.resolved !== true)).toHaveLength(1);
    expect(gates[0]).toMatchObject({ id: "timeline_node_006:gate-prompt", resolved: true });
    expect(gates[0]?.resolution).toBe("superseded");
    expect(gates[0]?.metadata).toMatchObject({
      gate_archive_round: 1,
      gate_archive_note: "第 1 轮已提交反馈：在修复一下 review 审核出来的问题吧",
    });
    expect(gates[1]).toMatchObject({ id: "turn_1:gate-prompt" });
  });

  // F-49 A6 前端半边：门修订（用户反馈触发的 SC revision）在 stage=human_confirm
  // 下运行 provider，而 ACTIVE_PROVIDER_STAGES 不含该阶段 ⇒ 整段修订流被丢弃，用户
  // 只看到门卡状态变化（引擎侧由 F49Be 落可见 author_run 节点，刷新后 rebuild 可见；
  // live 渲染归前端）。判据：当前门有 in-flight typed turn（open/busy）。
  it("keeps the gate revision stream while a typed gate turn is in flight (F-49 A6)", () => {
    startTypedGateSession();
    useWorkspaceStore.getState().setStage("human_confirm");
    useWorkspaceStore.getState().applyHumanGateTurnOpen("turn_1", "cmd_1", 2);

    handleWorkspaceWsMessage(
      {
        type: "stream_chunk",
        role: "author",
        content: "正在按反馈修订",
        node_id: "timeline_node_011",
      } as WsServerMessage,
      options(),
    );

    expect(useWorkspaceStore.getState().streamBuffers.timeline_node_011).toMatchObject({
      role: "author",
      chunks: ["正在按反馈修订"],
    });

    // 缓冲帧到达即落 provider_stream 气泡（hook 的 scheduleFlush 在生产路径调用本函数）
    useWorkspaceStore.getState().flushBufferedStream("timeline_node_011");
    expect(
      useWorkspaceStore
        .getState()
        .chatEntries.find((entry) => entry.id === "timeline_node_011:stream-active"),
    ).toMatchObject({ type: "provider_stream", role: "author", content: "正在按反馈修订" });
  });

  it("drops provider chunks on a gate without an in-flight revision turn (F-49 A6)", () => {
    startTypedGateSession();
    useWorkspaceStore.getState().setStage("human_confirm");

    handleWorkspaceWsMessage(
      {
        type: "stream_chunk",
        role: "author",
        content: "不该出现",
        node_id: "timeline_node_011",
      } as WsServerMessage,
      options(),
    );

    expect(useWorkspaceStore.getState().streamBuffers.timeline_node_011).toBeUndefined();
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

  // REQ-UI37-02 收敛语义回归：后端 terminate 事件顺序为
  // HumanGateClosed{decision:"terminate", stage:"completed"} → StageChange("completed")；
  // 阶段迁移不得把已收敛的门重新投影成开放门（收件箱「门禁等待」复活）。
  it("keeps a terminated gate converged when the terminal stage change follows", () => {
    startTypedGateSession();
    handleWorkspaceWsMessage(
      {
        type: "human_gate_turn_open",
        turn_id: "turn_terminate",
        command_id: "cmd_1",
        remaining_budget: 2,
      } as WsServerMessage,
      options(),
    );

    handleWorkspaceWsMessage(
      { type: "human_gate_closed", decision: "terminate", stage: "completed" } as WsServerMessage,
      options(),
    );
    handleWorkspaceWsMessage(
      { type: "stage_change", stage: "completed" } as WsServerMessage,
      options(),
    );

    expect(useWorkspaceStore.getState().humanGateClosure).toEqual({
      decision: "terminate",
      stage: "completed",
    });
    expect(gateEntries()[0]).toMatchObject({ resolved: true, resolution: "terminate" });
    expect(selectCockpitInbox(useWorkspaceStore.getState())).toHaveLength(0);
  });

  // confirm 路径不得因后续阶段迁移回退：门一旦已过，之后的 stage_change 只推进流程。
  it("keeps a confirmed gate converged across the stage changes that follow", () => {
    startTypedGateSession();
    handleWorkspaceWsMessage(
      {
        type: "human_gate_turn_open",
        turn_id: "turn_confirm",
        command_id: "cmd_1",
        remaining_budget: 2,
      } as WsServerMessage,
      options(),
    );

    handleWorkspaceWsMessage(
      { type: "stage_change", stage: "compile_plan" } as WsServerMessage,
      options(),
    );
    handleWorkspaceWsMessage(
      { type: "human_gate_closed", decision: "confirm", stage: "compile_plan" } as WsServerMessage,
      options(),
    );
    handleWorkspaceWsMessage(
      { type: "stage_change", stage: "running" } as WsServerMessage,
      options(),
    );

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

  it("keeps an owned gate protocol rejection inline instead of creating a hard error", () => {
    startTypedGateSession();
    handleWorkspaceWsMessage(
      {
        type: "human_gate_turn_open",
        turn_id: "turn_protocol",
        command_id: "cmd_gate",
        remaining_budget: 1,
      } as WsServerMessage,
      options(),
    );

    handleWorkspaceWsMessage(
      {
        type: "protocol_error",
        code: "INVALID_HUMAN_CONFIRM_ACTION",
        message: "confirm is rejected on this gate",
        context: { turn_id: "turn_protocol" },
      } as WsServerMessage,
      options(),
    );

    expect(useWorkspaceStore.getState().protocolError).toBeNull();
    expect(useWorkspaceStore.getState().humanGateTurn?.inlineError).toEqual({
      code: "INVALID_HUMAN_CONFIRM_ACTION",
      message: "confirm is rejected on this gate",
    });
    expect(useWorkspaceStore.getState().protocolDiagnostics).toHaveLength(1);
  });

  it("keeps an owned advance replay protocol rejection inline instead of creating a hard error", () => {
    handleWorkspaceWsMessage(
      {
        type: "advance_rejected",
        command_id: "advance_protocol",
        code: "ADVANCE_NOT_READY",
        reason: "gate still open",
      } as WsServerMessage,
      options(),
    );

    handleWorkspaceWsMessage(
      {
        type: "protocol_error",
        code: "ADVANCE_REPLAY_NOT_READY",
        message: "durable record exists",
        context: { command_id: "advance_protocol" },
      } as WsServerMessage,
      options(),
    );

    expect(useWorkspaceStore.getState().protocolError).toBeNull();
    expect(useWorkspaceStore.getState().advanceCommands.advance_protocol?.inlineError).toEqual({
      code: "ADVANCE_REPLAY_NOT_READY",
      message: "durable record exists",
    });
    expect(useWorkspaceStore.getState().protocolDiagnostics).toHaveLength(1);
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

  it("does not diagnose known protocol members left to downstream consumers", () => {
    useWorkspaceStore.getState().recordProtocolDiagnostic({
      code: "SEED",
      message: "既有诊断",
      at: "2026-09-13T00:00:00Z",
      type: "seed",
    });

    handleWorkspaceWsMessage(
      {
        type: "linked_workspace_amendment_created",
        snapshot: linkedWorkspaceAmendmentSnapshotFixture(),
      },
      options(),
    );
    handleWorkspaceWsMessage(
      {
        type: "provider_select_request",
        stage: "prepare_context",
        defaults: { author: "claude_code", reviewer: null, review_rounds: 1 },
      },
      options(),
    );

    const diagnostics = useWorkspaceStore.getState().protocolDiagnostics;
    expect(diagnostics).toHaveLength(1);
    expect(diagnostics[0]).toMatchObject({ code: "SEED" });
  });
});

// F-27：choice 卡此前依赖单帧 choice_request 送达；degraded 丢帧后卡永不出现。
// 治本契约：session_state 帧顶层带 pending_choice_requests，收帧后按 choice id
// 对账补挂（与 live choice_request handler 同构、幂等；pending 清空不残留）。
describe("workspace websocket session_state pending choice reconciliation", () => {
  installWorkspaceStoreTestHooks();

  const handlerOptions = () => ({
    invalidatedPreStageNodeIds: new Set<string>(),
    scheduleFlush: vi.fn(),
    streamFlushTimeouts: {},
  });

  const pendingChoiceFixture = (overrides: Record<string, unknown> = {}) => ({
    id: "choice_pending_a",
    prompt: "继续方式？",
    options: [
      { id: "opt_0", label: "继续 author", description: "沿用当前 author 会话" },
      { id: "opt_1", label: "切换 provider" },
    ],
    allow_multiple: false,
    allow_free_text: true,
    questions: [
      {
        id: "default",
        prompt: "继续方式？",
        options: [
          { id: "opt_0", label: "继续 author" },
          { id: "opt_1", label: "切换 provider" },
        ],
        allow_multiple: false,
        allow_free_text: true,
      },
    ],
    source: "text_fallback",
    ...overrides,
  });

  const sessionStateMessage = (pendingChoiceRequests: unknown) => ({
    type: "session_state",
    session_id: "session_pending_choice_reconcile",
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
    timeline_nodes: [],
    active_node_id: null,
    artifact_versions: [],
    timeline_node_details: {},
    human_presentation_revisions: [],
    ...(pendingChoiceRequests === undefined
      ? {}
      : { pending_choice_requests: pendingChoiceRequests }),
  });

  const choiceEntries = () =>
    useWorkspaceStore
      .getState()
      .chatEntries.filter((entry) => entry.type === "choice_request");

  it("mounts a missing pending choice entry isomorphic to the live choice_request handler", () => {
    handleWorkspaceWsMessage(
      sessionStateMessage([pendingChoiceFixture()]) as unknown as WsServerMessage,
      handlerOptions(),
    );

    expect(choiceEntries()).toEqual([
      expect.objectContaining({
        id: "choice_request:choice_pending_a",
        type: "choice_request",
        role: "system",
        content: "继续方式？",
        metadata: {
          request_id: "choice_pending_a",
          prompt: "继续方式？",
          options: pendingChoiceFixture().options,
          questions: pendingChoiceFixture().questions,
          allow_multiple: false,
          allow_free_text: true,
          source: "text_fallback",
        },
      }),
    ]);
  });

  it("keeps reconciliation idempotent for repeated session_state frames", () => {
    for (let round = 0; round < 2; round += 1) {
      handleWorkspaceWsMessage(
        sessionStateMessage([pendingChoiceFixture()]) as unknown as WsServerMessage,
        handlerOptions(),
      );
    }

    expect(choiceEntries()).toHaveLength(1);
    expect(choiceEntries()[0]).toMatchObject({
      id: "choice_request:choice_pending_a",
    });
  });

  it("does not duplicate an entry already mounted by the live choice_request frame", () => {
    handleWorkspaceWsMessage(
      {
        type: "choice_request",
        ...pendingChoiceFixture(),
      } as unknown as WsServerMessage,
      handlerOptions(),
    );
    handleWorkspaceWsMessage(
      sessionStateMessage([pendingChoiceFixture()]) as unknown as WsServerMessage,
      handlerOptions(),
    );

    expect(choiceEntries()).toHaveLength(1);
    expect(choiceEntries()[0]).toMatchObject({
      id: "choice_request:choice_pending_a",
      content: "继续方式？",
    });
  });

  it("drops the stale card once the choice leaves the pending projection", () => {
    handleWorkspaceWsMessage(
      sessionStateMessage([pendingChoiceFixture()]) as unknown as WsServerMessage,
      handlerOptions(),
    );
    expect(choiceEntries()).toHaveLength(1);

    handleWorkspaceWsMessage(
      sessionStateMessage([]) as unknown as WsServerMessage,
      handlerOptions(),
    );
    expect(choiceEntries()).toHaveLength(0);

    handleWorkspaceWsMessage(
      sessionStateMessage(undefined) as unknown as WsServerMessage,
      handlerOptions(),
    );
    expect(choiceEntries()).toHaveLength(0);
  });

  it("ignores malformed pending_choice_requests payloads without crashing", () => {
    expect(() =>
      handleWorkspaceWsMessage(
        sessionStateMessage({ not: "an array" }) as unknown as WsServerMessage,
        handlerOptions(),
      ),
    ).not.toThrow();
    expect(choiceEntries()).toHaveLength(0);
  });
});

// F-59（缺陷 2）：等待提示条数据源——session_state 的 pending_choice_requests
// 归一进 store.pendingChoiceRequests：role/created_at_ms 直通（提示条标注
// 发问方 + 已等待时长/901s 倒计时锚点）；旧载荷缺 created_at_ms 时按首见
// 时刻回退且跨帧稳定；pending 清空即收敛空。
describe("workspace websocket pending choice wait state projection", () => {
  installWorkspaceStoreTestHooks();

  const handlerOptions = () => ({
    invalidatedPreStageNodeIds: new Set<string>(),
    scheduleFlush: vi.fn(),
    streamFlushTimeouts: {},
  });

  const sessionStateMessage = (pendingChoiceRequests: unknown) => ({
    type: "session_state",
    session_id: "session_pending_choice_wait",
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
    timeline_nodes: [],
    active_node_id: null,
    artifact_versions: [],
    timeline_node_details: {},
    human_presentation_revisions: [],
    pending_choice_requests: pendingChoiceRequests,
  });

  const projected = () => useWorkspaceStore.getState().pendingChoiceRequests;

  it("projects role and created_at_ms from session_state pending_choice_requests", () => {
    const created_at_ms = Date.now() - 30_000;
    handleWorkspaceWsMessage(
      sessionStateMessage([
        {
          id: "choice_wait_a",
          prompt: "修订口径需要裁定",
          options: [],
          allow_multiple: false,
          allow_free_text: false,
          questions: [],
          source: "ask_user_question",
          role: "author",
          created_at_ms,
        },
      ]) as unknown as WsServerMessage,
      handlerOptions(),
    );

    expect(projected()).toEqual([
      {
        id: "choice_wait_a",
        prompt: "修订口径需要裁定",
        role: "author",
        created_at_ms,
        first_seen_at_ms: expect.any(Number),
        expected_run_id: null,
        options: [],
        allow_multiple: false,
        allow_free_text: false,
        questions: [],
        source: "ask_user_question",
      },
    ]);
  });

  it("falls back to first-seen time when created_at_ms is absent and keeps it stable", () => {
    handleWorkspaceWsMessage(
      sessionStateMessage([
        {
          id: "choice_wait_legacy",
          prompt: "旧载荷无时刻",
          source: "text_fallback",
        },
      ]) as unknown as WsServerMessage,
      handlerOptions(),
    );
    const firstSeen = projected()[0]?.first_seen_at_ms;
    expect(firstSeen).toEqual(expect.any(Number));
    expect(projected()[0]).toMatchObject({
      id: "choice_wait_legacy",
      created_at_ms: null,
      role: "author",
    });

    handleWorkspaceWsMessage(
      sessionStateMessage([
        {
          id: "choice_wait_legacy",
          prompt: "旧载荷无时刻",
          source: "text_fallback",
        },
      ]) as unknown as WsServerMessage,
      handlerOptions(),
    );
    expect(projected()[0]?.first_seen_at_ms).toBe(firstSeen);
  });

  it("clears once the pending projection empties", () => {
    handleWorkspaceWsMessage(
      sessionStateMessage([{ id: "choice_wait_b", prompt: "p", source: "provider_choice" }]) as unknown as WsServerMessage,
      handlerOptions(),
    );
    expect(projected()).toHaveLength(1);

    handleWorkspaceWsMessage(sessionStateMessage([]) as unknown as WsServerMessage, handlerOptions());
    expect(projected()).toHaveLength(0);
  });

  it("defaults role to author for malformed entries that still carry an id", () => {
    handleWorkspaceWsMessage(
      sessionStateMessage([{ id: "choice_wait_c", prompt: "p", source: "provider_choice" }]) as unknown as WsServerMessage,
      handlerOptions(),
    );
    expect(projected()[0]).toMatchObject({ id: "choice_wait_c", role: "author" });
  });
});

// P0 1.3（REQ-WIGA-05）Task 11：驾驶舱就地作答数据源——pending 投影必须保
// 完整 options/questions/source/expected_run_id（严格校验 question id/数组；
// 数据缺失不得给 REST 提交面喂数）。
describe("workspace websocket pending choice answer source projection", () => {
  installWorkspaceStoreTestHooks();

  const handlerOptions = () => ({
    invalidatedPreStageNodeIds: new Set<string>(),
    scheduleFlush: vi.fn(),
    streamFlushTimeouts: {},
  });

  const sessionStateMessage = (pendingChoiceRequests: unknown) => ({
    type: "session_state",
    session_id: "session_pending_choice_answer",
    workspace_type: "work_item_plan",
    stage: "human_confirm",
    session_status: "waiting_for_human",
    flow_kind: "single_candidate",
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
    timeline_nodes: [],
    active_node_id: null,
    artifact_versions: [],
    timeline_node_details: {},
    human_presentation_revisions: [],
    pending_choice_requests: pendingChoiceRequests,
  });

  const projected = () => useWorkspaceStore.getState().pendingChoiceRequests;

  it("keeps the full options/questions/source/expected_run_id for in-place answering", () => {
    handleWorkspaceWsMessage(
      sessionStateMessage([
        {
          id: "choice_answer_a",
          prompt: "拆分方案确认",
          options: [{ id: "opt_0", label: "继续", description: "desc" }],
          allow_multiple: true,
          allow_free_text: false,
          questions: [
            {
              id: "q-1",
              prompt: "是否包含集成测试",
              options: [
                { id: "yes", label: "包含" },
                { id: "no", label: "不包含" },
              ],
              allow_multiple: false,
              allow_free_text: false,
            },
          ],
          source: "ask_user_question",
          role: "author",
          expected_run_id: "run-9",
        },
      ]) as unknown as WsServerMessage,
      handlerOptions(),
    );

    expect(projected()[0]).toMatchObject({
      id: "choice_answer_a",
      expected_run_id: "run-9",
      source: "ask_user_question",
      allow_multiple: true,
      options: [{ id: "opt_0", label: "继续", description: "desc" }],
      questions: [
        {
          id: "q-1",
          prompt: "是否包含集成测试",
          options: [
            { id: "yes", label: "包含", description: null },
            { id: "no", label: "不包含", description: null },
          ],
          allow_multiple: false,
          allow_free_text: false,
        },
      ],
    });
  });

  it("drops malformed question structures instead of feeding the REST submit face", () => {
    handleWorkspaceWsMessage(
      sessionStateMessage([
        {
          id: "choice_answer_bad",
          prompt: "畸形题结构",
          questions: [{ prompt: "缺 id 的题" }],
          source: "ask_user_question",
        },
      ]) as unknown as WsServerMessage,
      handlerOptions(),
    );

    expect(projected()[0]).toMatchObject({ id: "choice_answer_bad" });
    expect(projected()[0].questions).toEqual([]);
  });
});
