import { beforeEach, describe, expect, it } from "vitest";
import { useWorkspaceStore } from "./workspace-ws-store";

/**
 * REQ-WSC-14（k3 「拒收信息刷新后仍可读」）：`session_state` 全量投影携带该
 * 会话最近一次门 turn 拒收时，store 必须据此重建终态 turn（门卡四要素），
 * 否则 owner 刷新一次页面就只剩一条失败行。
 */
function sessionPayload(overrides: Record<string, unknown> = {}) {
  return {
    session_id: "session-rejection",
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
    providers: { author: "codex", reviewer: "claude_code" },
    timeline_nodes: [],
    active_node_id: null,
    artifact_versions: [],
    timeline_node_details: {},
    active_run_id: null,
    human_gate_snapshot: { ...SNAPSHOT },
    ...overrides,
  } as never;
}

const SNAPSHOT = {
  findings: [],
  repeated_fingerprints: [],
  attempts_used: 0,
  manual_repairs_remaining: 2,
  trigger: "native_human_required",
  resumable: true,
};

const REJECTION = {
  turn_id: "turn_0001",
  rejected_feedback: "计划里不写文档更新任务，本次只交付代码",
  rejection_reason: "本轮反馈未通过平台校验，没有被采纳；门仍停在原处。",
  rejection_next_actions: ["把反馈收窄成一条可执行的修改后重新提交"],
  rejection_diagnostics: ["STRUCTURAL_EXISTING_GATE: failure requires the original human gate"],
  recorded_at: "2026-10-30T00:00:00Z",
};

describe("workspace store human gate rejection rebuild", () => {
  beforeEach(() => {
    useWorkspaceStore.getState().reset();
  });

  it("rebuilds the failed turn with all four rejection fields from session_state", () => {
    const store = useWorkspaceStore.getState();
    store.setSessionState(
      sessionPayload({ human_gate_turn_rejection: REJECTION }),
    );

    const turn = useWorkspaceStore.getState().humanGateTurn;
    expect(turn).not.toBeNull();
    expect(turn?.turn_id).toBe("turn_0001");
    expect(turn?.status).toBe("failed");
    expect(turn?.failure_class).toBe("validation_reject");
    expect(turn?.rejection).toEqual({
      rejected_feedback: REJECTION.rejected_feedback,
      rejection_reason: REJECTION.rejection_reason,
      rejection_next_actions: REJECTION.rejection_next_actions,
      rejection_diagnostics: REJECTION.rejection_diagnostics,
    });
  });

  it("leaves no turn when the projection carries no rejection", () => {
    const store = useWorkspaceStore.getState();
    store.setSessionState(sessionPayload({}));

    expect(useWorkspaceStore.getState().humanGateTurn).toBeNull();
  });

  it("does not resurrect a rejected turn once the gate snapshot is gone", () => {
    const store = useWorkspaceStore.getState();
    store.setSessionState(
      sessionPayload({
        human_gate_snapshot: null,
        human_gate_turn_rejection: REJECTION,
      }),
    );

    expect(useWorkspaceStore.getState().humanGateTurn).toBeNull();
  });
});

describe("P3-6 durable rejection rebuild does not override a newer in-flight turn", () => {
  beforeEach(() => {
    useWorkspaceStore.getState().reset();
  });

  it("keeps a newer in-flight turn (different turn id) instead of the stale rejection", () => {
    // 刷新重建出被拒 turn_0001 → owner 已回复，WS human_gate_turn_open 开出
    // 更新的在途 turn_0002。
    useWorkspaceStore
      .getState()
      .setSessionState(sessionPayload({ human_gate_turn_rejection: REJECTION }));
    useWorkspaceStore.getState().applyHumanGateTurnOpen("turn_0002", "cmd_2", 1);

    // 之后任意 session_state 广播仍携带同一 durable 拒收（turn_0001）。
    useWorkspaceStore
      .getState()
      .setSessionState(sessionPayload({ human_gate_turn_rejection: REJECTION }));

    const turn = useWorkspaceStore.getState().humanGateTurn;
    expect(turn?.turn_id).toBe("turn_0002");
    expect(turn?.status).toBe("open");
    expect(turn?.rejection).toBeUndefined();
  });

  it("keeps an in-flight turn that opened after the rejection was recorded (same turn id)", () => {
    // 门开（无拒收）→ owner 回复开出 turn_0001（opened_at=现在，晚于拒收
    // 落盘 recorded_at）→ durable 拒收同 id 但属更早回合——不得把在途 turn
    // 打回 failed。
    useWorkspaceStore.getState().setSessionState(sessionPayload({}));
    useWorkspaceStore.getState().applyHumanGateTurnOpen("turn_0001", "cmd_1", 2);

    useWorkspaceStore
      .getState()
      .setSessionState(sessionPayload({ human_gate_turn_rejection: REJECTION }));

    const turn = useWorkspaceStore.getState().humanGateTurn;
    expect(turn?.turn_id).toBe("turn_0001");
    expect(turn?.status).toBe("open");
    expect(turn?.rejection).toBeUndefined();
  });

  it("does not rebuild a rejection recorded before the current gate episode opened", () => {
    // 门 episode A（manual_repairs_remaining=2，无拒收）。
    useWorkspaceStore.getState().setSessionState(sessionPayload({}));

    // 门重开为新 episode（快照指纹变化）且 durable 仍带旧拒收——旧 episode
    // 的拒收不得重建进新门（否则新门显示「你上一条回复被拒」）。
    useWorkspaceStore.getState().setSessionState(
      sessionPayload({
        human_gate_turn_rejection: REJECTION,
        human_gate_snapshot: { ...SNAPSHOT, manual_repairs_remaining: 1 },
      }),
    );

    expect(useWorkspaceStore.getState().humanGateTurn).toBeNull();
  });

  it("still rebuilds a rejection recorded after the gate opened (same episode)", () => {
    // 同一 episode（指纹不变）内拒收落盘——刷新可读的 REQ-WSC-14 行为不回退。
    useWorkspaceStore.getState().setSessionState(sessionPayload({}));

    useWorkspaceStore
      .getState()
      .setSessionState(sessionPayload({ human_gate_turn_rejection: REJECTION }));

    const turn = useWorkspaceStore.getState().humanGateTurn;
    expect(turn?.turn_id).toBe("turn_0001");
    expect(turn?.status).toBe("failed");
    expect(turn?.failure_class).toBe("validation_reject");
    expect(turn?.rejection).toEqual({
      rejected_feedback: REJECTION.rejected_feedback,
      rejection_reason: REJECTION.rejection_reason,
      rejection_next_actions: REJECTION.rejection_next_actions,
      rejection_diagnostics: REJECTION.rejection_diagnostics,
    });
  });

  it("still rebuilds on a cold refresh with no in-flight turn (no episode evidence)", () => {
    // 刷新（跨会话首帧）无开门正证据——durable 拒收必须照旧重建。
    useWorkspaceStore
      .getState()
      .setSessionState(sessionPayload({ human_gate_turn_rejection: REJECTION }));

    expect(useWorkspaceStore.getState().humanGateTurn?.turn_id).toBe("turn_0001");
  });
});
