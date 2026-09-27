import { describe, expect, it, vi } from "vitest";
import type { WsServerMessage } from "./workspace-ws-message-handler";
import { handleWorkspaceWsMessage } from "./workspace-ws-message-handler";
import { useWorkspaceStore } from "../state/workspace-ws-store";
import { installWorkspaceStoreTestHooks } from "../state/workspace-ws-store.test-utils";
import { selectCockpitInbox } from "../state/workspace-cockpit-projection";
import { linkedWorkspaceAmendmentSnapshotFixture } from "../components/coding-workspace/plan-repair-test-fixtures";

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
