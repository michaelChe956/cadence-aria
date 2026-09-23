import { describe, expect, it, vi } from "vitest";
import type { WsServerMessage } from "../hooks/workspace-ws-message-handler";
import { handleWorkspaceWsMessage } from "../hooks/workspace-ws-message-handler";
import { selectCockpitInbox } from "./workspace-cockpit-projection";
import { useWorkspaceStore } from "./workspace-ws-store";
import type { WorkspaceSessionStatePayload } from "./workspace-ws-store-types";
import {
  installWorkspaceStoreTestHooks,
  makeNodeDetail,
} from "./workspace-ws-store.test-utils";

describe("workspace ws store gate rebuild", () => {
  installWorkspaceStoreTestHooks();

  it("rebuilds the gate entry for a typed turn gate without a human_confirm stage", () => {
    const store = useWorkspaceStore.getState();
    store.setSessionState({
      session_id: "session_typed_gate",
      workspace_type: "work_item_plan",
      stage: "running",
      session_status: "waiting_for_human",
      flow_kind: "single_candidate",
      run_policy: "auto_if_valid",
      run_history: {
        seen_fingerprints: [],
        repairs_used: 0,
        manual_repairs_used: 1,
        transitions_used: 0,
        initial_review_count: 1,
        verification_review_count: 0,
      },
      messages: [],
      checkpoints: [],
      artifact: null,
      providers: { author: "claude_code", reviewer: null },
    });
    useWorkspaceStore.getState().applyHumanGateTurnOpen("turn_1", "cmd_1", 1);

    useWorkspaceStore.getState().rebuildChatEntries();

    const gatePrompts = useWorkspaceStore
      .getState()
      .chatEntries.filter((entry) => entry.type === "gate_prompt");
    expect(gatePrompts).toHaveLength(1);
    expect(gatePrompts[0]).toMatchObject({ id: "turn_1:gate-prompt" });
    expect(gatePrompts[0]?.metadata).toMatchObject({
      turn_id: "turn_1",
      remaining_budget: 1,
      gate_status: "open",
    });
  });

  it("rebuilds the legacy human_confirm gate entry unchanged", () => {
    useWorkspaceStore.getState().setStage("human_confirm");

    useWorkspaceStore.getState().rebuildChatEntries();

    const gatePrompts = useWorkspaceStore
      .getState()
      .chatEntries.filter((entry) => entry.type === "gate_prompt");
    expect(gatePrompts).toHaveLength(1);
    expect(gatePrompts[0]).toMatchObject({ id: "human_confirm:gate-prompt" });
    expect(gatePrompts[0]?.metadata).toMatchObject({ gate_status: "open" });
    expect(gatePrompts[0]?.metadata?.turn_id).toBeUndefined();
  });

  it("keeps the gate entry from the durable snapshot after a reconnect rebuild", () => {
    useWorkspaceStore.getState().setSessionState({
      session_id: "session_snapshot_gate",
      workspace_type: "work_item_plan",
      stage: "running",
      session_status: "waiting_for_human",
      flow_kind: "single_candidate",
      run_policy: "auto_if_valid",
      run_history: {
        seen_fingerprints: [],
        repairs_used: 0,
        manual_repairs_used: 1,
        transitions_used: 0,
        initial_review_count: 1,
        verification_review_count: 0,
      },
      human_gate_snapshot: {
        findings: [],
        repeated_fingerprints: [],
        attempts_used: 1,
        manual_repairs_remaining: 1,
        trigger: "verification_new_findings",
        resumable: true,
      },
      messages: [],
      checkpoints: [],
      artifact: null,
      providers: { author: "claude_code", reviewer: null },
    });

    useWorkspaceStore.getState().rebuildChatEntries();

    const gatePrompts = useWorkspaceStore
      .getState()
      .chatEntries.filter((entry) => entry.type === "gate_prompt");
    expect(gatePrompts).toHaveLength(1);
    expect(gatePrompts[0]?.metadata).toMatchObject({
      action_facade: "typed",
      gate_trigger: "verification_new_findings",
      remaining_budget: 1,
      gate_status: "open",
    });
  });

  // F-42（复验）：实测 workspace_session_0003 的持久形态是「session confirmed +
  // stage completed + human_confirm 节点仍 active + 门快照在场」。确认走 WS
  // confirm/对话门 approve（不重发 human_gate_closed 关门帧），重连/刷新后内存
  // closure 为空——此前门卡按 durable 快照重建为可点的确认/终止按钮，二次点击
  // 被服务端矩阵拒（INVALID_MESSAGE_FOR_STAGE: confirm not allowed in stage
  // completed）。终态判据与 REQ-PCG-03/F-30 的 TERMINAL_SESSION_STATUSES 同源。
  it("closes the rebuilt gate card when the session is already confirmed", () => {
    useWorkspaceStore.getState().setSessionState({
      ...buildSessionState("session_confirmed_gate"),
      stage: "completed",
      session_status: "confirmed",
      human_gate_snapshot: humanGateSnapshotFixture(),
      timeline_nodes: [staleHumanConfirmNode()],
      active_node_id: "timeline_node_004",
    });

    useWorkspaceStore.getState().rebuildChatEntries();

    const gatePrompts = useWorkspaceStore
      .getState()
      .chatEntries.filter((entry) => entry.type === "gate_prompt");
    expect(gatePrompts).toHaveLength(1);
    expect(gatePrompts[0]).toMatchObject({ resolved: true, resolution: "confirm" });
    expect(gatePrompts[0]?.metadata).toMatchObject({ gate_status: "confirm" });
    expect(
      selectCockpitInbox(useWorkspaceStore.getState()).filter((item) => item.kind === "gate"),
    ).toEqual([]);
  });

  it("closes the rebuilt gate card as terminated once the session is terminated", () => {
    useWorkspaceStore.getState().setSessionState({
      ...buildSessionState("session_terminated_gate"),
      stage: "completed",
      session_status: "terminated",
      human_gate_snapshot: humanGateSnapshotFixture(),
      timeline_nodes: [staleHumanConfirmNode()],
      active_node_id: "timeline_node_004",
    });

    useWorkspaceStore.getState().rebuildChatEntries();

    const gatePrompts = useWorkspaceStore
      .getState()
      .chatEntries.filter((entry) => entry.type === "gate_prompt");
    expect(gatePrompts).toHaveLength(1);
    expect(gatePrompts[0]).toMatchObject({ resolved: true, resolution: "terminate" });
    expect(
      selectCockpitInbox(useWorkspaceStore.getState()).filter((item) => item.kind === "gate"),
    ).toEqual([]);
  });

  // 乐观收口态：HTTP confirm 200 已把 session_status 置 confirmed，stage 事件可能
  // 尚未到达（仍是 human_confirm）——同样是合法收口信号，门卡必须同步收口，否则
  // 用户会在「已确认」的会话上看到可点按钮。
  it("closes the rebuilt gate card on the optimistic confirmed state", () => {
    useWorkspaceStore.getState().setSessionState({
      ...buildSessionState("session_optimistic_confirmed_gate"),
      stage: "human_confirm",
      session_status: "confirmed",
      human_gate_snapshot: humanGateSnapshotFixture(),
      timeline_nodes: [staleHumanConfirmNode()],
      active_node_id: "timeline_node_004",
    });

    useWorkspaceStore.getState().rebuildChatEntries();

    const gatePrompts = useWorkspaceStore
      .getState()
      .chatEntries.filter((entry) => entry.type === "gate_prompt");
    expect(gatePrompts).toHaveLength(1);
    expect(gatePrompts[0]).toMatchObject({ resolved: true, resolution: "confirm" });
  });

  it("keeps a typed turn across a same-session snapshot fingerprint change", () => {
    const store = useWorkspaceStore.getState();
    store.setSessionState({
      ...buildSessionState("session_typed_snapshot"),
      human_gate_snapshot: {
        findings: [],
        repeated_fingerprints: [],
        attempts_used: 1,
        manual_repairs_remaining: 2,
        trigger: "verification_new_findings",
        resumable: true,
      },
    });
    store.applyHumanGateTurnOpen("turn_persistent", "cmd_persistent", 2);

    store.setSessionState({
      ...buildSessionState("session_typed_snapshot"),
      human_gate_snapshot: {
        findings: [],
        repeated_fingerprints: [],
        attempts_used: 1,
        manual_repairs_remaining: 1,
        trigger: "verification_new_findings",
        resumable: true,
      },
    });

    expect(useWorkspaceStore.getState().humanGateTurn?.turn_id).toBe("turn_persistent");
    expect(selectCockpitInbox(useWorkspaceStore.getState())[0]?.gate?.turn_id).toBe("turn_persistent");
  });

  it("keeps terminal advance dedup across a same-session rebuild and clears it across sessions", () => {
    const store = useWorkspaceStore.getState();
    store.setSessionState(buildSessionState("session_dedup"));
    useWorkspaceStore.getState().applyAdvanceCompleted("cmd_done", "attempt_1", "coding");
    useWorkspaceStore.getState().applyAdvanceRejected("cmd_rejected", "ADVANCE_NOT_READY", "nope");
    useWorkspaceStore.getState().recordProtocolDiagnostic({
      code: "UNRECOGNIZED_EVENT",
      message: "未识别的出向事件类型：future_event",
      at: "2026-09-13T00:00:00Z",
      type: "future_event",
    });

    // 同会话重建：去重集从持久化会话状态重放恢复、不丢弃（D8）
    useWorkspaceStore.getState().setSessionState(buildSessionState("session_dedup"));

    expect(Object.keys(useWorkspaceStore.getState().advanceCommands).sort()).toEqual([
      "cmd_done",
      "cmd_rejected",
    ]);
    expect(useWorkspaceStore.getState().protocolDiagnostics).toHaveLength(1);

    // 切到另一个会话：去重集必须清空，避免串会话
    useWorkspaceStore.getState().setSessionState(buildSessionState("session_other"));

    expect(useWorkspaceStore.getState().advanceCommands).toEqual({});
    expect(useWorkspaceStore.getState().protocolDiagnostics).toEqual([]);
  });

  it("does not fall back to the inbox for a replayed advance rejection after reconnect", () => {
    useWorkspaceStore.getState().setSessionState(buildSessionState("session_replay"));
    handleWorkspaceWsMessage(
      {
        type: "advance_rejected",
        command_id: "cmd_replay",
        code: "ADVANCE_REPLAY_NOT_READY",
        reason: "durable record exists",
      } as WsServerMessage,
      {
        invalidatedPreStageNodeIds: new Set<string>(),
        scheduleFlush: vi.fn(),
        streamFlushTimeouts: {},
      },
    );

    expect(selectCockpitInbox(useWorkspaceStore.getState())).toHaveLength(0);
  });

  // REQ-PCG-01/02（plan-compile-gate-visibility）：批次确认与 compile recovery 是
  // durable node 门——重建（刷新/重连）不得把它们铸成 typed/legacy 决策卡：卡面
  // 带 `action_facade=typed` 时会露出 feedback 编辑器与 confirm/abandon 三命令动作，
  // 而 REQ-RET-02/REQ-CG-02 明确禁止把两新门映射进该面。两新门只经 Cockpit 收件箱
  // 与流程投影呈现。
  it.each([
    ["work_item_batch_confirm", "author_confirm"],
    ["work_item_plan_compile_recovery", "human_confirm"],
  ] as const)(
    "does not mint a typed decision card for the %s node gate",
    (nodeType, stage) => {
      useWorkspaceStore.setState({
        sessionId: "session_node_gate",
        stage,
        workspaceType: "work_item_plan",
        flowKind: "single_candidate",
        sessionStatus: "waiting_for_human",
        humanGateTurn: null,
        humanGateSnapshot: null,
        humanGateClosure: null,
        chatEntries: [],
        timelineNodes: [
          {
            node_id: "node_gate",
            node_type: nodeType,
            agent: null,
            stage,
            round: null,
            status: "active",
            title: nodeType,
            summary: "Final Compile 需要恢复：provider timeout",
            started_at: "2026-09-22T00:00:00Z",
            completed_at: null,
            duration_ms: null,
            artifact_ref: null,
            provider_config_snapshot: {
              author: "claude_code",
              reviewer: null,
              review_rounds: 1,
            },
            retry: null,
          },
        ],
      });

      useWorkspaceStore.getState().rebuildChatEntries();

      expect(
        useWorkspaceStore
          .getState()
          .chatEntries.filter((entry) => entry.type === "gate_prompt"),
      ).toHaveLength(0);
      // 门本身仍可见——可见面收敛到收件箱，而不是丢失。
      expect(
        selectCockpitInbox(useWorkspaceStore.getState()).filter(
          (item) => item.kind === "gate",
        ),
      ).toHaveLength(1);
    },
  );

  // F-39（review 结论卡常显）：story/design 快照只带 timeline_node_summaries——
  // session_state.rs 的 should_inline_work_item_plan_detail 把 reviewer_run 的 detail
  // 排除在非 plan 会话之外（只有 REST /timeline-node-details 水合才拿得到 verdict）。
  // 水合缺位时 rebuild 一张 review 结论卡都生不出（节点只剩占位 detail → 走「无内容」
  // 分支），对话流里只剩门卡的压缩文案；且 rebuild 路径的 metadata 不携带 round
  //（live 路径 review_complete 带），两条路径不对称。completed reviewer_run 必须每轮
  // 成卡：有 verdict 用完整版，没有则用 node.summary+node.round 兜底，同 id 不叠双。
  describe("F-39 review verdict cards without hydrated detail", () => {
    function reviewRoundsSnapshot(
      rounds: Array<{ round: number; summary: string }>,
    ): WorkspaceSessionStatePayload {
      return {
        session_id: "session_review_rounds",
        workspace_type: "story",
        stage: "human_confirm",
        session_status: "waiting_for_human",
        flow_kind: "legacy",
        run_policy: "interactive",
        run_history: {
          seen_fingerprints: [],
          repairs_used: 0,
          manual_repairs_used: 0,
          transitions_used: 0,
          initial_review_count: rounds.length,
          verification_review_count: 0,
        },
        messages: [],
        checkpoints: [],
        artifact: "# Draft",
        providers: { author: "claude_code", reviewer: "codex" },
        timeline_nodes: rounds.map(({ round, summary }) => ({
          node_id: `timeline_node_review_${round}`,
          node_type: "reviewer_run" as const,
          agent: "codex" as const,
          stage: "cross_review",
          round,
          status: "completed" as const,
          title: `Review Round ${round}`,
          summary,
          started_at: `2026-05-26T10:0${round}:00Z`,
          completed_at: `2026-05-26T10:0${round}:30Z`,
          duration_ms: 30_000,
          artifact_ref: "artifact_current",
          provider_config_snapshot: {
            author: "claude_code",
            reviewer: "codex",
            review_rounds: rounds.length,
          },
        })),
        active_node_id: null,
        artifact_versions: [],
        timeline_node_details: {},
        active_run_id: null,
      };
    }

    it("rebuilds one review verdict card per completed round from the node payload", () => {
      useWorkspaceStore.getState().setSessionState(
        reviewRoundsSnapshot([
          { round: 1, summary: "第一轮需要返修" },
          { round: 2, summary: "第二轮通过" },
        ]),
      );

      const cards = useWorkspaceStore
        .getState()
        .chatEntries.filter((entry) => entry.type === "review_verdict");
      expect(cards).toHaveLength(2);
      expect(cards.map((card) => card.node_id)).toEqual([
        "timeline_node_review_1",
        "timeline_node_review_2",
      ]);
      expect(cards.map((card) => card.metadata?.round)).toEqual([1, 2]);
      expect(cards.map((card) => card.content)).toEqual([
        "第一轮需要返修",
        "第二轮通过",
      ]);
      expect(cards.map((card) => card.metadata?.summary)).toEqual([
        "第一轮需要返修",
        "第二轮通过",
      ]);
      // 兜底卡不得凭空捏造 verdict——没有事实来源就没有该字段。
      expect(cards.every((card) => card.metadata?.verdict === undefined)).toBe(true);
    });

    it("carries the round on the hydrated verdict card without duplicating it", () => {
      useWorkspaceStore.getState().setSessionState({
        ...reviewRoundsSnapshot([{ round: 2, summary: "第二轮通过" }]),
        timeline_node_details: {
          timeline_node_review_2: makeNodeDetail({
            node_id: "timeline_node_review_2",
            node_type: "reviewer_run",
            agent_role: "reviewer",
            status: "completed",
            provider: { name: "codex", model: "gpt-5" },
            verdict: {
              verdict: "pass",
              summary: "第二轮通过",
              comments: "",
              findings: [
                {
                  severity: "suggestion",
                  message: "可选建议一",
                  evidence: "",
                  required_action: "",
                },
              ],
            },
          }),
        },
      });

      const cards = useWorkspaceStore
        .getState()
        .chatEntries.filter((entry) => entry.type === "review_verdict");
      // 有 detail.verdict 时走完整版，兜底不得叠出第二张同 id 卡。
      expect(cards).toHaveLength(1);
      expect(cards[0].metadata).toEqual(
        expect.objectContaining({
          round: 2,
          verdict: "pass",
          summary: "第二轮通过",
        }),
      );
      expect(cards[0].metadata?.findings).toEqual([
        expect.objectContaining({ severity: "suggestion", message: "可选建议一" }),
      ]);
    });
  });
});

function buildSessionState(sessionId: string) {
  return {
    session_id: sessionId,
    workspace_type: "work_item_plan" as const,
    stage: "running",
    session_status: "waiting_for_human" as const,
    flow_kind: "single_candidate" as const,
    run_policy: "auto_if_valid" as const,
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
    providers: { author: "claude_code" as const, reviewer: null },
  };
}

/** F-42：实测 workspace_session_0003 的 SC 门快照（预算 3、native_human_required）。 */
function humanGateSnapshotFixture() {
  return {
    findings: [],
    repeated_fingerprints: [],
    attempts_used: 0,
    manual_repairs_remaining: 3,
    trigger: "native_human_required" as const,
    resumable: true,
  };
}

/** F-42：确认链遗留的 Active human_confirm 节点（service 侧持久层曾如此落盘）。 */
function staleHumanConfirmNode() {
  return {
    node_id: "timeline_node_004",
    node_type: "human_confirm" as const,
    agent: null,
    stage: "human_confirm",
    round: null,
    status: "active" as const,
    title: "人工确认",
    summary: "SingleCandidate 已通过 Evaluate，等待 Approval",
    started_at: "2026-09-23T03:10:44.882807791+00:00",
    completed_at: null,
    duration_ms: null,
    artifact_ref: null,
    provider_config_snapshot: {
      author: "pi" as const,
      reviewer: "kimi_code" as const,
      review_rounds: 1,
    },
  };
}


