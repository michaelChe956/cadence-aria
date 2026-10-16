import { describe, expect, it } from "vitest";
import type { WorkspaceWsState } from "./workspace-ws-store-types";
import { useWorkspaceStore, type TimelineNode } from "./workspace-ws-store";
import { installWorkspaceStoreTestHooks } from "./workspace-ws-store.test-utils";
import {
  c1WaitingItem,
  codingAttentionIdentity,
  codingAttentionItem,
  cockpitInboxItemSessionId,
  gateActionBlockCopy,
  gateActionBlockReason,
  gateKindOf,
  gateTerminateBlockReason,
  isStaleDriverLeaseItem,
  logicalCodebaseBootstrapItem,
  selectCockpitFlow,
  selectCockpitInbox,
  selectGateProjection,
  topologyTokenName,
} from "./workspace-cockpit-projection";
import type { C1WaitingItem, CodingAttentionItemDto } from "../api/types";
import {
  lcFreshOperationFixture,
  lcResumeRecoveryFixture,
  reviewVerdictEntry,
  timelineNode,
} from "./workspace-cockpit-projection.test-fixtures";

function snapshotGateState(overrides: {
  trigger: "native_human_required" | "verification_new_findings";
  manual_repairs_remaining: number;
}) {
  return {
    session_id: "session_snapshot_gate",
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
    human_gate_snapshot: {
      findings: [],
      repeated_fingerprints: [],
      attempts_used: 1,
      resumable: true,
      ...overrides,
    },
    messages: [],
    checkpoints: [],
    artifact: null,
    providers: { author: "claude_code" as const, reviewer: null },
  };
}

function humanGateSnapshotFixture() {
  return {
    findings: [],
    repeated_fingerprints: [],
    attempts_used: 1,
    manual_repairs_remaining: 1,
    trigger: "verification_new_findings" as const,
    resumable: true,
  };
}

describe("workspace cockpit gate projection", () => {
  installWorkspaceStoreTestHooks();

  it("returns no gate projection without a turn, snapshot or legacy stage", () => {
    expect(selectGateProjection(useWorkspaceStore.getState())).toBeNull();
  });

  it("projects the stage-only human_confirm gate under a stage-prefixed key", () => {
    useWorkspaceStore.getState().setStage("human_confirm");

    expect(selectGateProjection(useWorkspaceStore.getState())).toMatchObject({
      key: "stage:human_confirm",
      turn_id: null,
      status: "open",
      remaining_budget: null,
      closed: null,
    });
  });

  it("projects a typed turn gate keyed by turn_id with its budget", () => {
    const store = useWorkspaceStore.getState();
    store.setStage("running");
    store.applyHumanGateTurnOpen("turn_1", "cmd_1", 3);

    expect(selectGateProjection(useWorkspaceStore.getState())).toMatchObject({
      key: "turn_1",
      turn_id: "turn_1",
      remaining_budget: 3,
      status: "open",
    });
  });

  it("keeps a typed gate visible from the durable snapshot after reconnect", () => {
    const store = useWorkspaceStore.getState();
    store.setSessionState({
      session_id: "session_1",
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

    expect(selectGateProjection(useWorkspaceStore.getState())).toMatchObject({
      key: expect.stringMatching(/^snapshot:/),
      trigger: "verification_new_findings",
      remaining_budget: 1,
      resumable: true,
    });
  });

  it("does not project an old closure onto a newly observed offline snapshot gate", () => {
    const store = useWorkspaceStore.getState();
    store.setSessionState(snapshotGateState({
      trigger: "native_human_required",
      manual_repairs_remaining: 2,
    }));
    store.applyHumanGateClosed("confirm", "running");

    store.setSessionState(snapshotGateState({
      trigger: "verification_new_findings",
      manual_repairs_remaining: 1,
    }));

    expect(selectGateProjection(useWorkspaceStore.getState())).toMatchObject({
      closed: null,
      key: expect.stringMatching(/^snapshot:.*:verification_new_findings\|1\|true\|\|$/),
    });
  });

  it("marks a closed gate with its decision", () => {
    const store = useWorkspaceStore.getState();
    store.setStage("human_confirm");
    store.applyHumanGateClosed("confirm", "human_confirm");

    expect(selectGateProjection(useWorkspaceStore.getState())).toMatchObject({
      closed: "confirm",
      closure_stage: "human_confirm",
    });
  });

  // F-42（复验）：确认链落 Confirmed/Terminated 后不再补发 human_gate_closed 帧
  // （WS confirm / 对话门 approve / HTTP confirm 200 乐观态都不发）。终态必须独立
  // 让门收口，否则重连/刷新后的门卡仍放行确认/终止，二次点击被服务端矩阵拒
  // （INVALID_MESSAGE_FOR_STAGE: confirm not allowed in stage completed）。
  it("closes the gate on a terminal session without a closure frame", () => {
    useWorkspaceStore.getState().setSessionState({
      ...snapshotGateState({ trigger: "native_human_required", manual_repairs_remaining: 3 }),
      stage: "completed",
      session_status: "confirmed",
    });

    expect(selectGateProjection(useWorkspaceStore.getState())).toMatchObject({
      closed: "confirm",
      action_block_reason: "closed",
      terminate_block_reason: "closed",
    });
    expect(
      selectCockpitInbox(useWorkspaceStore.getState()).filter((item) => item.kind === "gate"),
    ).toEqual([]);
  });

  it("closes the gate as terminated on a terminated session", () => {
    useWorkspaceStore.getState().setSessionState({
      ...snapshotGateState({ trigger: "native_human_required", manual_repairs_remaining: 3 }),
      stage: "completed",
      session_status: "terminated",
    });

    expect(selectGateProjection(useWorkspaceStore.getState())).toMatchObject({
      closed: "terminate",
      terminate_block_reason: "closed",
    });
  });

  it("flags triage from the rebuilt review verdict metadata", () => {
    const store = useWorkspaceStore.getState();
    store.setStage("human_confirm");
    store.appendChatEntry(reviewVerdictEntry({ review_gate: "user_triage_required" }));

    expect(selectGateProjection(useWorkspaceStore.getState())?.triage).toBe(true);
  });

  it("blocks a stale gate snapshot while the engine stage is a non-gate stage without reopen path", () => {
    useWorkspaceStore.setState({
      stage: "running",
      humanGateSnapshot: humanGateSnapshotFixture(),
    });

    expect(gateActionBlockReason(useWorkspaceStore.getState())).toBe("terminal_stage");
    expect(gateActionBlockCopy("terminal_stage")).toContain("已离开人工确认门");
  });
  it.each(["compile_plan", "completed"])(
    "blocks the %s stage without a gate snapshot or turn",
    (stage) => {
      useWorkspaceStore.setState({
        stage,
        humanGateSnapshot: null,
        humanGateTurn: null,
        humanGateClosure: null,
      });

      expect(gateActionBlockReason(useWorkspaceStore.getState())).toBe("terminal_stage");
    },
  );

  it("blocks a single-candidate gate while phase is outside the engine-authorized set", () => {
    useWorkspaceStore.setState({
      stage: "human_confirm",
      flowKind: "single_candidate",
      singleCandidatePhase: "generate",
      humanGateSnapshot: humanGateSnapshotFixture(),
    });

    expect(gateActionBlockReason(useWorkspaceStore.getState())).toBe("phase_mismatch");
  });

  it("allows a single-candidate gate in the evaluate phase (close CAS accepts Approval|Evaluate)", () => {
    useWorkspaceStore.setState({
      stage: "human_confirm",
      flowKind: "single_candidate",
      singleCandidatePhase: "evaluate",
      humanGateSnapshot: humanGateSnapshotFixture(),
    });

    expect(gateActionBlockReason(useWorkspaceStore.getState())).toBeNull();
  });

  // A1 迁移（方案 A）：用例保留——completed 载体形态维持放行；夹具按修订后
  // 契约补齐 workspaceType（引擎谓词要求 WorkItemPlan；真实 session_state 帧
  // 中快照与 workspace_type 同帧到达，不存在「有快照无类型」的中间态）。
  it("keeps a completed-stage gate operable for the amendment reopen shape (0017-indistinguishable)", () => {
    useWorkspaceStore.setState({
      workspaceType: "work_item_plan",
      stage: "completed",
      flowKind: "single_candidate",
      singleCandidatePhase: "completed",
      humanGateSnapshot: humanGateSnapshotFixture(),
      humanGateClosure: null,
    });

    expect(gateActionBlockReason(useWorkspaceStore.getState())).toBeNull();
  });

  // A1 方案 A（用户 2026-09-30 裁决，REQ-CFC-05 修订为 amendment-aware）：completed
  // stage 的终态守卫只放行「可构成 REQ-GCE-03 amendment 重开合法载体」的形态——
  // work_item_plan + single_candidate + 相位 completed + 快照在场（镜像引擎
  // probe_amendment_gate_context 的前端可见前置谓词，conversational_gate.rs:165-201；
  // amendment 上下文事实 build_session_state 未暴露，前端不可无歧义判定陈旧，交由
  // 引擎既有拒绝兜底）。其余 completed 形态引擎侧不存在任何合法重开路径 = 可无歧义
  // 判定的陈旧门（0017 形态），锁死并给出原因说明。
  it.each([
    [
      "非 plan 会话",
      { workspaceType: "story", flowKind: "single_candidate", singleCandidatePhase: "completed" },
    ],
    [
      "非 SC 流",
      { workspaceType: "work_item_plan", flowKind: "legacy", singleCandidatePhase: "completed" },
    ],
    [
      "SC 相位非 completed",
      { workspaceType: "work_item_plan", flowKind: "single_candidate", singleCandidatePhase: "evaluate" },
    ],
    [
      "旧会话缺相位字段",
      { workspaceType: "work_item_plan", flowKind: "single_candidate", singleCandidatePhase: null },
    ],
  ] as const)("locks a stale completed-stage snapshot gate for %s (0017 形态)", (_label, overrides) => {
    useWorkspaceStore.setState({
      stage: "completed",
      humanGateSnapshot: humanGateSnapshotFixture(),
      humanGateClosure: null,
      ...overrides,
    } as Partial<WorkspaceWsState>);

    const state = useWorkspaceStore.getState();
    expect(gateActionBlockReason(state)).toBe("terminal_stage");
    expect(gateActionBlockCopy("terminal_stage")).toContain("已离开人工确认门");
    // 投影面贯通：门卡仍渲染（陈旧门可见）但动作/终止判据双双锁定。
    expect(selectGateProjection(state)).toMatchObject({
      kind: "human_gate",
      action_block_reason: "terminal_stage",
      terminate_block_reason: "terminal_stage",
    });
  });

  it("locks a residual completed-stage turn without a gate snapshot (no amendment carrier)", () => {
    useWorkspaceStore.setState({
      workspaceType: "work_item_plan",
      stage: "completed",
      flowKind: "single_candidate",
      singleCandidatePhase: "completed",
      humanGateSnapshot: null,
      humanGateTurn: {
        turn_id: "turn_stale",
        command_id: "cmd_stale",
        remaining_budget: 1,
        status: "open",
        artifact_ref: null,
        failure_class: null,
        failure_message: null,
        opened_at: "2026-09-14T00:00:00.000Z",
        inlineError: null,
      },
      humanGateClosure: null,
    });

    expect(gateActionBlockReason(useWorkspaceStore.getState())).toBe("terminal_stage");
  });

  // F-21（v28 监控）：plan 会话（SC 流）除 approval/evaluate 终审门外，还会停在
  // human_confirm：context blocker（prepare 相位，authoring.rs
  // enter_work_item_plan_context_blocker）/author 连续 validate 失败（generate
  // 相位）/旧会话缺相位字段（null）。矩阵（protocol.rs HumanConfirm SC 臂）与引擎
  // close_human_gate 只校验 stage+flow_kind——terminate 在这些形态同样可达，
  // 相位白名单不得一刀切拦掉终止（confirm/feedback 维持相位纪律）。
  it.each(["prepare", "generate", null] as const)(
    "allows terminate but not confirm for a plan gate parked at human_confirm with phase %s (F-21)",
    (singleCandidatePhase) => {
      useWorkspaceStore.setState({
        workspaceType: "work_item_plan",
        stage: "human_confirm",
        flowKind: "single_candidate",
        singleCandidatePhase,
        sessionStatus: "waiting_for_human",
        humanGateClosure: null,
      });

      const state = useWorkspaceStore.getState();
      expect(gateActionBlockReason(state)).toBe("phase_mismatch");
      expect(gateTerminateBlockReason(state)).toBeNull();
      expect(selectGateProjection(state)?.terminate_block_reason).toBeNull();
      expect(selectGateProjection(state)?.action_block_reason).toBe("phase_mismatch");
    },
  );

  it.each([
    ["terminal_stage", { workspaceType: "work_item_plan", stage: "running", humanGateSnapshot: humanGateSnapshotFixture() }],
    ["closed", { workspaceType: "work_item_plan", stage: "human_confirm", humanGateClosure: { decision: "confirm", stage: "human_confirm" } }],
    ["non-plan phase_mismatch", { workspaceType: null, stage: "human_confirm", flowKind: "single_candidate", singleCandidatePhase: "generate" }],
  ] as const)("keeps terminate blocked for %s", (_label, overrides) => {
    useWorkspaceStore.setState({
      flowKind: "single_candidate",
      singleCandidatePhase: "generate",
      humanGateTurn: null,
      humanGateSnapshot: null,
      ...overrides,
    } as Partial<WorkspaceWsState>);

    const state = useWorkspaceStore.getState();
    // 终止跟随通用阻断判据（terminal_stage / closed / 非会话归属的 phase_mismatch）。
    expect(gateTerminateBlockReason(state)).toBe(gateActionBlockReason(state));
    expect(gateActionBlockReason(state)).not.toBeNull();
  });

});

describe("workspace cockpit inbox projection", () => {
  installWorkspaceStoreTestHooks();

  it("projects a triage gate as one severity-2 item", () => {
    const store = useWorkspaceStore.getState();
    store.setStage("human_confirm");
    store.setHumanGateSnapshot({
      findings: [],
      repeated_fingerprints: [],
      attempts_used: 1,
      manual_repairs_remaining: 2,
      trigger: "repeated_fingerprint",
      resumable: true,
    });
    store.appendChatEntry(reviewVerdictEntry({ verdict: "needs_human" }));

    const items = selectCockpitInbox(useWorkspaceStore.getState());

    expect(items).toHaveLength(1);
    expect(items[0]).toMatchObject({
      id: expect.stringMatching(/^gate:snapshot:/),
      kind: "gate",
      severity: 2,
      triage: true,
      source: "gate",
    });
    // F-50 fix round 标题去重：triage 短语只保留主区门卡卡头——抽屉门条目
    // 与非 triage 门同题（gateSummaryTitle 不再分叉）。
    expect(items[0]?.title).toBe("需要人工确认");
  });

  it("projects a user_triage_required verdict gate under the unified inbox title", () => {
    const store = useWorkspaceStore.getState();
    store.setStage("human_confirm");
    store.appendChatEntry(reviewVerdictEntry({ review_gate: "user_triage_required" }));

    const items = selectCockpitInbox(useWorkspaceStore.getState());

    expect(items[0]?.triage).toBe(true);
    expect(items[0]?.title).toBe("需要人工确认");
  });

  it("projects a stopped session item", () => {
    const store = useWorkspaceStore.getState();
    store.setSessionStatus("stopped_needs_human");

    const items = selectCockpitInbox(useWorkspaceStore.getState());

    expect(items).toEqual([
      expect.objectContaining({
        id: "stopped:session",
        kind: "stopped",
        severity: 2,
        source: "session_status",
        summary: "停点原因：stopped_needs_human · 等待人工接管后继续",
      }),
    ]);
  });

  // 批 2 P5（REQ-WIGA-08 C1 前端半）：stopped 卡的同源托管管理态——durable
  // automation 归属（P0 1.2 WS session_state.automation）owner=server 且
  // enabled 的会话即「enabled 精确绑定当前 enrollment」（与 P4 服务端 takeover
  // 拒绝同源），收件箱据此隐藏接管按钮；前端隐藏只是同步呈现，不以隐藏
  // 代替服务端拒绝。
  it("projects the managed flag and recovery copy on stopped items for server-owned enabled sessions", () => {
    const store = useWorkspaceStore.getState();
    store.setSessionStatus("stopped_needs_human");
    useWorkspaceStore.setState({
      automation: {
        owner: "server",
        enrollment_id: "enrollment_0001",
        policy_revision: 1,
        enabled: true,
      },
    });

    const items = selectCockpitInbox(useWorkspaceStore.getState());

    expect(items).toEqual([
      expect.objectContaining({
        id: "stopped:session",
        kind: "stopped",
        automationManaged: true,
      }),
    ]);
    expect(items[0]?.summary).not.toContain("等待人工接管");
    expect(items[0]?.summary).toContain("对话窗口");
  });

  it("keeps the takeover copy and no managed flag for non-managed stopped sessions", () => {
    for (const automation of [
      null,
      { owner: "client" as const, enrollment_id: "e", policy_revision: 1, enabled: false },
      { owner: "server" as const, enrollment_id: "e", policy_revision: 1, enabled: false },
    ]) {
      useWorkspaceStore.setState({ automation });
      useWorkspaceStore.getState().setSessionStatus("stopped_needs_human");

      const items = selectCockpitInbox(useWorkspaceStore.getState());

      expect(items[0]).toMatchObject({
        kind: "stopped",
        automationManaged: false,
        summary: "停点原因：stopped_needs_human · 等待人工接管后继续",
      });
    }
  });

  it("projects protocol errors as hard error items", () => {
    useWorkspaceStore.getState().setProtocolError({
      code: "INVALID_HUMAN_CONFIRM_ACTION",
      message: "confirm is rejected on this gate",
    });

    const items = selectCockpitInbox(useWorkspaceStore.getState());

    expect(items).toEqual([
      expect.objectContaining({
        id: "hard_error:protocol:INVALID_HUMAN_CONFIRM_ACTION",
        kind: "hard_error",
        severity: 3,
        summary: "confirm is rejected on this gate",
      }),
    ]);
  });

  it("exposes the protocol error code so the inbox can offer a lease retake (F-11)", () => {
    useWorkspaceStore.getState().setProtocolError({
      code: "STALE_DRIVER_LEASE",
      message: "driver connection no longer holds the lease",
    });

    const items = selectCockpitInbox(useWorkspaceStore.getState());

    expect(items).toHaveLength(1);
    expect(items[0]?.protocolErrorCode).toBe("STALE_DRIVER_LEASE");
    expect(isStaleDriverLeaseItem(items[0]!)).toBe(true);
    expect(
      isStaleDriverLeaseItem({ ...items[0]!, protocolErrorCode: "OTHER_CODE" }),
    ).toBe(false);
    expect(isStaleDriverLeaseItem({ ...items[0]!, source: "engine_error" })).toBe(false);
  });

  it("keeps a real advance rejection and drops a replayed one", () => {
    const store = useWorkspaceStore.getState();
    store.applyAdvanceRejected("cmd_real", "ADVANCE_NOT_READY", "gate still open");
    store.applyAdvanceRejected("cmd_replay", "ADVANCE_REPLAY_NOT_READY", "durable record exists");

    const items = selectCockpitInbox(useWorkspaceStore.getState());

    expect(items.map((item) => item.id)).toEqual(["hard_error:advance:cmd_real"]);
  });

  it("removes the gate item once the gate is closed", () => {
    const store = useWorkspaceStore.getState();
    store.setStage("human_confirm");
    expect(selectCockpitInbox(useWorkspaceStore.getState())).toHaveLength(1);

    store.applyHumanGateClosed("confirm", "human_confirm");

    expect(selectCockpitInbox(useWorkspaceStore.getState())).toHaveLength(0);
  });

  // 闭环保留不得让「下一代 legacy 门」被误判为已收：
  // 进入 human_confirm 即新门开启，必须清上一轮闭环（原设计里清 closure 的逃生口）。
  it("clears the previous closure when a new legacy gate opens", () => {
    const store = useWorkspaceStore.getState();
    store.setStage("human_confirm");
    store.applyHumanGateClosed("confirm", "human_confirm");
    store.setStage("compile_plan");

    store.setStage("human_confirm");

    expect(useWorkspaceStore.getState().humanGateClosure).toBeNull();
    expect(
      selectCockpitInbox(useWorkspaceStore.getState()).filter((item) => item.kind === "gate"),
    ).toHaveLength(1);
  });

  it("sorts by severity descending then newest first", () => {
    const store = useWorkspaceStore.getState();
    store.setProtocolError({ code: "SOME_PROTOCOL_ERROR", message: "hard" });
    store.setStage("human_confirm");

    const items = selectCockpitInbox(useWorkspaceStore.getState());

    expect(items.map((item) => item.kind)).toEqual(["hard_error", "gate"]);
  });
});

describe("F-20 story/design author_confirm gate projection", () => {
  installWorkspaceStoreTestHooks();

  it.each(["story", "design"] as const)(
    "projects the %s author_confirm stage as an open stage-prefixed gate",
    (workspaceType) => {
      useWorkspaceStore.setState({
        sessionId: "session_story_gate",
        stage: "author_confirm",
        workspaceType,
        flowKind: "legacy",
        sessionStatus: "waiting_for_human",
        humanGateTurn: null,
        humanGateSnapshot: null,
        humanGateClosure: null,
      });

      expect(selectGateProjection(useWorkspaceStore.getState())).toMatchObject({
        key: "stage:author_confirm",
        turn_id: null,
        stage: "author_confirm",
        status: "open",
        trigger: null,
        remaining_budget: null,
        findings: [],
        closed: null,
        turn: null,
        action_block_reason: null,
      });
    },
  );

  it("keeps work_item_plan author_confirm gateless (SC typed 门不随 stage 投影)", () => {
    useWorkspaceStore.setState({
      stage: "author_confirm",
      workspaceType: "work_item_plan",
      flowKind: "legacy",
      sessionStatus: "waiting_for_human",
      humanGateTurn: null,
      humanGateSnapshot: null,
      humanGateClosure: null,
    });

    expect(selectGateProjection(useWorkspaceStore.getState())).toBeNull();
    expect(gateActionBlockReason(useWorkspaceStore.getState())).toBe("terminal_stage");
  });

  it("keeps other non-gate stages terminal for story sessions", () => {
    useWorkspaceStore.setState({
      stage: "running",
      workspaceType: "story",
      humanGateTurn: null,
      humanGateSnapshot: null,
      humanGateClosure: null,
    });

    expect(gateActionBlockReason(useWorkspaceStore.getState())).toBe("terminal_stage");
  });

  it("allows confirm/terminate while the story author gate is open, closes once confirmed", () => {
    useWorkspaceStore.setState({
      stage: "author_confirm",
      workspaceType: "story",
      sessionStatus: "waiting_for_human",
      humanGateTurn: null,
      humanGateSnapshot: null,
      humanGateClosure: null,
    });
    expect(gateActionBlockReason(useWorkspaceStore.getState())).toBeNull();

    useWorkspaceStore.setState({ sessionStatus: "confirmed" });

    expect(gateActionBlockReason(useWorkspaceStore.getState())).toBe("closed");
    // 已确认门不再进收件箱（等待面收敛）。
    expect(
      selectCockpitInbox(useWorkspaceStore.getState()).filter((item) => item.kind === "gate"),
    ).toHaveLength(0);
  });

  it("closes the story author gate on the server terminate closure", () => {
    useWorkspaceStore.setState({
      stage: "author_confirm",
      workspaceType: "design",
      sessionStatus: "waiting_for_human",
      humanGateClosure: { decision: "terminate", stage: "completed" },
    });

    expect(gateActionBlockReason(useWorkspaceStore.getState())).toBe("closed");
    expect(selectGateProjection(useWorkspaceStore.getState())).toMatchObject({
      closed: "terminate",
      closure_stage: "completed",
    });
  });

  it("adds an actionable inbox gate item for an open story author gate", () => {
    useWorkspaceStore.setState({
      sessionId: "session_story_gate",
      stage: "author_confirm",
      workspaceType: "story",
      flowKind: "legacy",
      sessionStatus: "waiting_for_human",
      humanGateTurn: null,
      humanGateSnapshot: null,
      humanGateClosure: null,
    });

    const items = selectCockpitInbox(useWorkspaceStore.getState());

    expect(items).toHaveLength(1);
    expect(items[0]).toMatchObject({
      id: "gate:stage:author_confirm",
      kind: "gate",
      title: "需要人工确认",
      triage: false,
      source: "gate",
      gate: expect.objectContaining({ key: "stage:author_confirm" }),
    });
  });

  // F-50 fix1（k3 P2）：无 trigger 门（story/design author_confirm 主流程
  // trigger 恒 null，运行时实证复现）的收件箱条目不再以「等待人工确认」与标题
  // 「需要人工确认」近义叠行——headline fallback 让位（parts 已 filter），
  // 无独立事实时 summary 为空串，渲染面判空不渲染。
  it("drops the synonymous fallback headline for a triggerless gate (F-50 fix1)", () => {
    useWorkspaceStore.setState({
      sessionId: "session_story_gate",
      stage: "author_confirm",
      workspaceType: "story",
      flowKind: "legacy",
      sessionStatus: "waiting_for_human",
      humanGateTurn: null,
      humanGateSnapshot: null,
      humanGateClosure: null,
    });

    const item = selectCockpitInbox(useWorkspaceStore.getState())[0]!;

    expect(item.title).toBe("需要人工确认");
    expect(item.summary).toBe("");
  });

  // F-50 裁决 1/6：门条默认标题与门卡同题（需要人工确认），triage intent 门
  // 保留「需要判断 reviewer 意图」；协议错误条目以中文主显 lead 投影（code 由
  // protocolErrorCode 承载，原文留在 summary）。
  it("projects protocol errors with the Chinese lead title (F-50)", () => {
    useWorkspaceStore.getState().setProtocolError({
      code: "STALE_DRIVER_LEASE",
      message: "driver connection no longer holds the lease",
    });

    const item = selectCockpitInbox(useWorkspaceStore.getState())[0]!;

    expect(item.title).toBe("连接租约已失效");
    expect(item.protocolErrorCode).toBe("STALE_DRIVER_LEASE");
    expect(item.summary).toBe("driver connection no longer holds the lease");
  });

  // F-31（v37 复验 #2）：author 门投影携带 review_available（=reviewerEnabled），
  // 待处理抽屉据此露出「确认并评审」——与主区门卡 state.reviewerEnabled 同源。
  it("projects review availability for the story author gate from reviewerEnabled (F-31)", () => {
    useWorkspaceStore.setState({
      stage: "author_confirm",
      workspaceType: "story",
      flowKind: "legacy",
      sessionStatus: "waiting_for_human",
      reviewerEnabled: true,
      humanGateTurn: null,
      humanGateSnapshot: null,
      humanGateClosure: null,
    });
    expect(selectGateProjection(useWorkspaceStore.getState())?.review_available).toBe(true);

    useWorkspaceStore.setState({ reviewerEnabled: false });
    expect(selectGateProjection(useWorkspaceStore.getState())?.review_available).toBe(false);
  });
});

// REQ-PCG-01（plan-compile-gate-visibility）：整组 Draft 确认门——compile 成功与
// compile 失败转 batch-confirm 两条路径都停在 `work_item_batch_confirm`，Cockpit
// 必须把它投影成稳定、去重、可诊断的门，而不是要求用户切到 legacy 页面。
describe("REQ-PCG-01 batch confirm gate projection", () => {
  installWorkspaceStoreTestHooks();

  const batchNode = timelineNode({
    node_id: "node_batch",
    node_type: "work_item_batch_confirm",
    stage: "author_confirm",
    status: "active",
    title: "Work Item Batch 确认",
  });

  function batchSession(overrides: Partial<WorkspaceWsState> = {}) {
    useWorkspaceStore.setState({
      sessionId: "session_batch",
      stage: "author_confirm",
      workspaceType: "work_item_plan",
      flowKind: "single_candidate",
      sessionStatus: "waiting_for_human",
      humanGateTurn: null,
      humanGateSnapshot: null,
      humanGateClosure: null,
      timelineNodes: [batchNode],
      ...overrides,
    });
  }

  it("projects the batch node as a node-keyed gate and one inbox item", () => {
    batchSession();

    expect(selectGateProjection(useWorkspaceStore.getState())).toMatchObject({
      key: "node:node_batch",
      kind: "batch_confirm",
      stage: "author_confirm",
      flow_kind: "single_candidate",
      status: "open",
      turn_id: null,
      turn: null,
      closed: null,
      triage: false,
      action_block_reason: null,
      terminate_block_reason: null,
    });

    const items = selectCockpitInbox(useWorkspaceStore.getState());

    expect(items).toHaveLength(1);
    expect(items[0]).toMatchObject({
      id: "gate:node:node_batch",
      kind: "gate",
      severity: 1,
      title: "确认整组 Work Item Draft",
      source: "gate",
      gate: expect.objectContaining({ kind: "batch_confirm", key: "node:node_batch" }),
    });
    expect(items[0]?.summary).toContain("等待整组 Work Item Draft 确认");
  });

  // REQ-PCG-03：缺少 durable 凭据（flow/stage 与门不符）时不认门、不猜动作。
  it("stays gateless without the durable single_candidate/author_confirm credentials", () => {
    batchSession({ flowKind: "legacy" });
    const state = useWorkspaceStore.getState();

    expect(gateKindOf(state)).toBe("human_gate");
    expect(selectGateProjection(state)).toBeNull();
    expect(gateActionBlockReason(state)).toBe("terminal_stage");
  });

  it("keeps one item with the same key across repeated frames and a reconnect", () => {
    batchSession();
    const first = selectCockpitInbox(useWorkspaceStore.getState());

    // 重复帧：同一 durable node 再 upsert（时间戳可漂移，身份不变）。
    useWorkspaceStore.getState().setTimelineNodesForTest([
      { ...batchNode, started_at: "2026-09-22T00:10:00Z" },
    ]);
    const repeated = selectCockpitInbox(useWorkspaceStore.getState());

    expect(repeated).toHaveLength(1);
    expect(repeated[0]?.id).toBe("gate:node:node_batch");
    expect(repeated[0]?.id).toBe(first[0]?.id);

    // 新一代门（新 node_id）才换身份——key 随 durable node，不随渲染顺序。
    useWorkspaceStore.getState().setTimelineNodesForTest([
      { ...batchNode, node_id: "node_batch_2" },
    ]);
    expect(selectCockpitInbox(useWorkspaceStore.getState())[0]?.id).toBe(
      "gate:node:node_batch_2",
    );
  });

  it.each(["confirmed", "terminated"] as const)(
    "collapses a lingering batch node once the session is %s (F-30)",
    (sessionStatus) => {
      batchSession({ sessionStatus });
      const state = useWorkspaceStore.getState();

      expect(selectGateProjection(state)).toBeNull();
      expect(selectCockpitInbox(state).filter((item) => item.kind === "gate")).toHaveLength(0);
      expect(gateActionBlockReason(state)).toBe("terminal_stage");
    },
  );

  it("closes the batch gate on the durable closure decision", () => {
    batchSession({ humanGateClosure: { decision: "confirm", stage: "author_confirm" } });
    const state = useWorkspaceStore.getState();

    expect(gateActionBlockReason(state)).toBe("closed");
    expect(selectCockpitInbox(state)).toHaveLength(0);
  });
});

// REQ-PCG-02（plan-compile-gate-visibility）：Final Compile recovery 门——此前只在
// legacy 页面可见；Cockpit 必须投影它并带上引擎写入的中断原因。
describe("REQ-PCG-02 compile recovery gate projection", () => {
  installWorkspaceStoreTestHooks();

  function recoveryNode(summary: string | null): TimelineNode {
    return timelineNode({
      node_id: "node_recovery",
      node_type: "work_item_plan_compile_recovery",
      stage: "human_confirm",
      status: "active",
      title: "WorkItemPlan Compile Recovery",
      summary,
    });
  }

  function recoverySession(
    summary: string | null,
    overrides: Partial<WorkspaceWsState> = {},
  ) {
    useWorkspaceStore.setState({
      sessionId: "session_recovery",
      stage: "human_confirm",
      workspaceType: "work_item_plan",
      flowKind: "single_candidate",
      sessionStatus: "waiting_for_human",
      humanGateTurn: null,
      humanGateSnapshot: null,
      humanGateClosure: null,
      timelineNodes: [recoveryNode(summary)],
      ...overrides,
    });
  }

  it("projects the recovery node with the interruption reason and no turn carrier", () => {
    recoverySession("Final Compile 需要恢复：provider timeout");
    const state = useWorkspaceStore.getState();

    expect(selectGateProjection(state)).toMatchObject({
      key: "node:node_recovery",
      kind: "compile_recovery",
      stage: "human_confirm",
      status: "open",
      turn_id: null,
      turn: null,
      trigger: null,
      closed: null,
      action_block_reason: null,
      terminate_block_reason: null,
    });

    const items = selectCockpitInbox(state);

    expect(items).toHaveLength(1);
    expect(items[0]).toMatchObject({
      id: "gate:node:node_recovery",
      title: "Final Compile 恢复",
      gate: expect.objectContaining({ kind: "compile_recovery" }),
    });
    expect(items[0]?.summary).toContain("Final Compile 中断，等待恢复动作");
    expect(items[0]?.summary).toContain("Final Compile 需要恢复：provider timeout");
  });

  it("shows a read-only diagnostic summary when the node carries no reason (REQ-PCG-03)", () => {
    recoverySession(null);

    expect(selectCockpitInbox(useWorkspaceStore.getState())[0]?.summary).toContain(
      "中断原因未同步（只读诊断）",
    );
  });

  // REQ-PCG-03：阶段不一致（节点残留/事件乱序）→ 不投影、不猜动作，落回通用判据。
  it("fails closed for a recovery node observed outside the HumanConfirm stage", () => {
    recoverySession("late frame", { stage: "running" });
    const state = useWorkspaceStore.getState();

    expect(gateKindOf(state)).toBe("human_gate");
    expect(selectGateProjection(state)).toBeNull();
    expect(gateActionBlockReason(state)).toBe("terminal_stage");
  });

  it.each(["confirmed", "terminated"] as const)(
    "collapses a lingering recovery node once the session is %s (F-30)",
    (sessionStatus) => {
      recoverySession("late frame", { sessionStatus });
      const state = useWorkspaceStore.getState();

      expect(selectGateProjection(state)).toBeNull();
      expect(selectCockpitInbox(state).filter((item) => item.kind === "gate")).toHaveLength(0);
      expect(gateActionBlockReason(state)).toBe("terminal_stage");
    },
  );
});

// 回归：没有两新门节点时，既有四路门必须与改前逐项一致（kind 恒 "human_gate"）。
describe("REQ-PCG-01 existing gate regression", () => {
  installWorkspaceStoreTestHooks();

  it("keeps the stage-only, typed turn, snapshot and story author gates on human_gate", () => {
    const store = useWorkspaceStore.getState();

    store.setStage("human_confirm");
    expect(selectGateProjection(useWorkspaceStore.getState())).toMatchObject({
      key: "stage:human_confirm",
      kind: "human_gate",
      turn_id: null,
    });

    store.applyHumanGateTurnOpen("turn_1", "cmd_1", 3);
    expect(selectGateProjection(useWorkspaceStore.getState())).toMatchObject({
      key: "turn_1",
      kind: "human_gate",
      turn_id: "turn_1",
      remaining_budget: 3,
    });

    useWorkspaceStore.setState({
      humanGateTurn: null,
      humanGateSnapshot: humanGateSnapshotFixture(),
    });
    expect(selectGateProjection(useWorkspaceStore.getState())).toMatchObject({
      kind: "human_gate",
      trigger: "verification_new_findings",
      remaining_budget: 1,
    });
    expect(gateKindOf(useWorkspaceStore.getState())).toBe("human_gate");

    useWorkspaceStore.setState({
      stage: "author_confirm",
      workspaceType: "story",
      flowKind: "legacy",
      sessionStatus: "waiting_for_human",
      humanGateSnapshot: null,
    });
    expect(selectGateProjection(useWorkspaceStore.getState())).toMatchObject({
      key: "stage:author_confirm",
      kind: "human_gate",
    });
  });
});

// F-49 A8：门内预算 chip 此前取残留 typed turn 的 remaining_budget。turn 按 D8 跨
// session_state 保留（command_id 去重），而「修订成功经 Evaluate 重建门快照」时
// 快照预算重置为默认值（spec REQ-CG-02 + conversational_gate_amendment_real_chain.rs
// 实测重建后 manual_repairs_remaining=3）——门以新快照重建后必须显示新快照预算，
// 否则低估剩余修复轮次。判据：门快照身份变了（= 门载体换快照），预算以快照为准。
describe("F-49 gate budget authority", () => {
  installWorkspaceStoreTestHooks();

  it("keeps the in-flight turn budget while the gate snapshot is unchanged", () => {
    const store = useWorkspaceStore.getState();
    store.setSessionState(
      snapshotGateState({ trigger: "native_human_required", manual_repairs_remaining: 3 }),
    );
    store.applyHumanGateTurnOpen("turn_1", "cmd_1", 2);

    expect(selectGateProjection(useWorkspaceStore.getState())).toMatchObject({
      key: "turn_1",
      turn_id: "turn_1",
      remaining_budget: 2,
    });
  });

  it("takes the rebuilt snapshot budget once the gate reopens on a new snapshot", () => {
    const store = useWorkspaceStore.getState();
    store.setSessionState(
      snapshotGateState({ trigger: "native_human_required", manual_repairs_remaining: 3 }),
    );
    store.applyHumanGateTurnOpen("turn_1", "cmd_1", 2);

    store.setSessionState({
      ...snapshotGateState({ trigger: "verification_new_findings", manual_repairs_remaining: 3 }),
      stage: "human_confirm",
    });

    expect(selectGateProjection(useWorkspaceStore.getState())).toMatchObject({
      key: "turn_1",
      turn_id: "turn_1",
      remaining_budget: 3,
    });
  });

  it("keeps the turn budget when the turn was opened before any snapshot arrived", () => {
    const store = useWorkspaceStore.getState();
    // 同一会话先落一次无快照的 session_state：turn 开出时门还没有快照凭据。
    store.setSessionState({
      ...snapshotGateState({ trigger: "native_human_required", manual_repairs_remaining: 3 }),
      human_gate_snapshot: null,
      stage: "human_confirm",
    });
    store.applyHumanGateTurnOpen("turn_1", "cmd_1", 2);

    store.setSessionState(
      snapshotGateState({ trigger: "native_human_required", manual_repairs_remaining: 3 }),
    );

    expect(selectGateProjection(useWorkspaceStore.getState())).toMatchObject({
      key: "turn_1",
      remaining_budget: 2,
    });
  });
});

describe("C5 project repository initialization waiting projection", () => {
  const projectWaiting: C1WaitingItem = {
    id: "c1:project:project_0001:repository_init:op_init_0001",
    kind: "repository_initialization_failed",
    reason:
      "repository initialization failed at pre_check (provider_unavailable); awaiting gateway recovery",
    completed_steps: ["cadence_skills"],
    target: null,
    plan_id: null,
    session_id: null,
    attempt_id: null,
    gate_id: null,
    possible_side_effect: null,
    actions: ["resume_repository_initialization"],
    next_phase: "repository_registered",
    action_context: [],
    operation_id: "op_init_0001",
    diagnostics: {
      failed_step: "pre_check",
      reason_code: "provider_unavailable",
      provider: "claude_code",
      stderr_summary: null,
      changed_paths: [],
      retryable: true,
    },
    project_id: "project_0001",
  };

  it("projects a project-level item with the backend id and no issue identity", () => {
    const item = c1WaitingItem(projectWaiting, "project_0001");
    expect(item.id).toBe("c1:project:project_0001:repository_init:op_init_0001");
    expect(item.id).not.toContain("undefined");
    expect(item.kind).toBe("c1_recovery");
    expect(item.c1Info?.issueId).toBeNull();
    expect(item.c1Info?.projectId).toBe("project_0001");
    expect(item.c1Info?.operationId).toBe("op_init_0001");
    expect(item.c1Info?.diagnostics?.failedStep).toBe("pre_check");
    expect(item.c1Info?.diagnostics?.reasonCode).toBe("provider_unavailable");
    expect(item.c1Info?.diagnostics?.provider).toBe("claude_code");
    expect(item.c1Info?.diagnostics?.retryable).toBe(true);
    expect(item.title).toBe("仓库初始化失败，等待恢复");
  });

  it("keeps the issue-level id prefix and tolerates missing diagnostics", () => {
    const item = c1WaitingItem(
      {
        ...projectWaiting,
        id: "c1:advance_retry_failed:advance_0001",
        kind: "advance_retry_failed",
        operation_id: null,
        diagnostics: null,
      },
      "project_0001",
      "issue_0001",
    );
    expect(item.id).toBe("c1:issue_0001:c1:advance_retry_failed:advance_0001");
    expect(item.c1Info?.issueId).toBe("issue_0001");
    expect(item.c1Info?.operationId).toBeNull();
    expect(item.c1Info?.diagnostics).toBeNull();
  });
});

describe("anchor_t19 cockpit inbox lc resume recovery card", () => {
  installWorkspaceStoreTestHooks();

  function recoverySessionState(
    extras: Record<string, unknown> = {},
  ): Record<string, unknown> {
    return {
      session_id: "session_rec",
      workspace_type: "work_item_plan",
      stage: "running",
      session_status: "waiting_for_human",
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
      providers: { author: "codex", reviewer: null },
      ...extras,
    };
  }

  it("anchor_t19_driver_observer_recovery_projection_and_resolved_clear", () => {
    // 三活跃态各投一张恢复卡；卡面携带 D 层渲染所需的完整恢复投影。
    for (const state of ["waiting", "fresh_reserved", "fresh_running"] as const) {
      useWorkspaceStore.getState().setSessionState(
        recoverySessionState({
          lc_resume_recovery: lcResumeRecoveryFixture({
            state,
            last_operation: lcFreshOperationFixture({ state: "failed", reason: "prepare_failed" }),
          }),
        }) as never,
      );
      const items = selectCockpitInbox(useWorkspaceStore.getState());
      const card = items.find((item) => item.kind === "lc_resume_recovery");
      expect(card, `state=${state}`).toMatchObject({
        id: "lc_resume_recovery:rec-1",
        severity: 2,
        source: "lc_resume_recovery",
      });
      expect(card?.recovery).toMatchObject({
        recoveryId: "rec-1",
        state,
        reasonCode: "resume_fingerprint_mismatch",
        provider: "codex",
        failedNodeId: "node-1",
        runKind: "legacy_draft",
        detectedAt: "2026-10-11T00:02:00Z",
        updatedAt: "2026-10-11T00:04:00Z",
        // 不推权限：allowedActions 只镜像 wire allowed_actions（B 未接齐恒空），
        // 不从 reason_code/错误文本推导动作。
        allowedActions: [],
        lastOperation: { operation_id: "op-1", state: "failed", reason: "prepare_failed" },
      });
    }

    // resolved：卡消失（不投 lc_resume_recovery 卡）……
    useWorkspaceStore.getState().setSessionState(
      recoverySessionState({
        lc_resume_recovery: lcResumeRecoveryFixture({
          state: "resolved",
          last_operation: lcFreshOperationFixture({ state: "completed", fresh_node: "node-2" }),
        }),
      }) as never,
    );
    const resolvedState = useWorkspaceStore.getState();
    expect(resolvedState.lcResumeRecovery).toMatchObject({
      recovery_id: "rec-1",
      state: "resolved",
    });
    expect(
      selectCockpitInbox(resolvedState).some((item) => item.kind === "lc_resume_recovery"),
    ).toBe(false);

    // ……且 generic 卡不被永久隐藏（resolved 不阻塞合法后续动作的收件箱事实）。
    useWorkspaceStore.getState().setSessionStatus("stopped_needs_human");
    const afterResolved = selectCockpitInbox(useWorkspaceStore.getState());
    expect(
      afterResolved.some((item) => item.kind === "lc_resume_recovery"),
    ).toBe(false);
    expect(
      afterResolved.some((item) => item.kind === "stopped"),
    ).toBe(true);

    // null：无恢复事实不投卡。
    useWorkspaceStore.getState().reset();
    useWorkspaceStore.getState().setSessionState(recoverySessionState() as never);
    expect(
      selectCockpitInbox(useWorkspaceStore.getState()).some(
        (item) => item.kind === "lc_resume_recovery",
      ),
    ).toBe(false);
  });
});

// 「登记并重试」按钮投影（owner 2026-10-12 直批）：trust 缺失错误按稳定码
// （provider_trust_missing/trust_entry_missing）识别 + provider wire 标签
// 解析；typed 回执对账后同一错误条切 trustRegistered 态。
describe("trust missing inbox projection", () => {
  function trustErrorState(): WorkspaceWsState {
    useWorkspaceStore.getState().reset();
    useWorkspaceStore.getState().setError(
      "provider_trust_missing: trust_entry_missing: canonical root 下未找到 codex 的用户级 workspace trust 条目（entry absent）",
    );
    return useWorkspaceStore.getState();
  }

  it("trust 缺失错误条挂 trustMissing 投影（稳定码+provider 解析）", () => {
    const items = selectCockpitInbox(trustErrorState());
    const errorItem = items.find((item) => item.id === "hard_error:error");
    expect(errorItem?.trustMissing).toEqual({ provider: "codex" });
    expect(errorItem?.trustRegistered).toBe(false);
  });

  it("kimi_code 标签同样识别；无稳定码的普通错误不投登记面", () => {
    useWorkspaceStore.getState().reset();
    useWorkspaceStore.getState().setError(
      "provider_trust_missing: trust_entry_missing: canonical root 下未找到 kimi_code 的用户级 workspace trust 条目",
    );
    expect(
      selectCockpitInbox(useWorkspaceStore.getState()).find((item) => item.id === "hard_error:error")
        ?.trustMissing,
    ).toEqual({ provider: "kimi_code" });

    useWorkspaceStore.getState().reset();
    useWorkspaceStore.getState().setError("provider start failed: command missing");
    expect(
      selectCockpitInbox(useWorkspaceStore.getState()).find((item) => item.id === "hard_error:error")
        ?.trustMissing,
    ).toBeNull();
  });

  it("typed 回执 provider 匹配时切 trustRegistered；不匹配保持可登记", () => {
    const before = selectCockpitInbox(trustErrorState());
    expect(before.find((item) => item.id === "hard_error:error")?.trustRegistered).toBe(false);

    useWorkspaceStore.getState().applyTrustEntryRegistered({
      provider: "codex",
      action: "registered",
      trust_key: "[projects.\"/fixture-root\"]",
      canonical_root: "/fixture-root",
      registered_at: "2026-10-12T00:00:00Z",
    });
    const after = selectCockpitInbox(useWorkspaceStore.getState());
    expect(after.find((item) => item.id === "hard_error:error")?.trustRegistered).toBe(true);

    useWorkspaceStore.getState().applyTrustEntryRegistered({
      provider: "kimi_code",
      action: "registered",
      trust_key: "wd_fixture_000000000000",
      canonical_root: "/fixture-root",
      registered_at: "2026-10-12T00:00:00Z",
    });
    const mismatched = selectCockpitInbox(useWorkspaceStore.getState());
    expect(mismatched.find((item) => item.id === "hard_error:error")?.trustRegistered).toBe(false);
  });

  // 指引卡简化（owner 2026-10-12 直批）：结构化 trust_missing_guidance
  // 帧落 store 后，投影优先用 wire 事实（providers/canonicalRoot/
  // commandMarkdown），不再解析 error 文本；error 缺席不投卡。
  it("结构化指导优先于文本解析：providers/canonicalRoot/commandMarkdown 直挂投影", () => {
    useWorkspaceStore.getState().reset();
    useWorkspaceStore.getState().setError(
      "provider_trust_missing: trust_entry_missing: canonical root 下未找到 kimi_code 的用户级 workspace trust 条目",
    );
    useWorkspaceStore.getState().applyTrustMissingGuidance({
      providers: ["kimi_code", "codex"],
      canonicalRoot: "/aggregate/root",
      reasonCode: "trust_entry_missing",
      retryAction: "人工登记后显式重新生成",
      commandMarkdown:
        "mkdir -p '/home/u/.kimi-code/workspace-trust' && printf '{\"root\":\"/aggregate/root\",\"trustedAt\":%s}' \"$(date +%s%3N)\" > '/home/u/.kimi-code/workspace-trust/wd_root_000000000000'",
    });
    const item = selectCockpitInbox(useWorkspaceStore.getState()).find(
      (entry) => entry.id === "hard_error:error",
    );
    expect(item?.trustMissing).toEqual({
      provider: "kimi_code",
      providers: ["kimi_code", "codex"],
      canonicalRoot: "/aggregate/root",
      commandMarkdown:
        "mkdir -p '/home/u/.kimi-code/workspace-trust' && printf '{\"root\":\"/aggregate/root\",\"trustedAt\":%s}' \"$(date +%s%3N)\" > '/home/u/.kimi-code/workspace-trust/wd_root_000000000000'",
      retryAction: "人工登记后显式重新生成",
    });
  });

  it("结构化指导缺席时回落既有稳定码文本识别（旧服务端兼容）", () => {
    const items = selectCockpitInbox(trustErrorState());
    expect(items.find((item) => item.id === "hard_error:error")?.trustMissing).toEqual({
      provider: "codex",
    });
  });

  it("error 缺席时结构化指导不单独投卡（指导只配对错误条）", () => {
    useWorkspaceStore.getState().reset();
    useWorkspaceStore.getState().applyTrustMissingGuidance({
      providers: ["codex"],
      canonicalRoot: "/aggregate/root",
      reasonCode: "trust_entry_missing",
      retryAction: "人工登记后显式重新生成",
      commandMarkdown: null,
    });
    const items = selectCockpitInbox(useWorkspaceStore.getState());
    expect(items.find((item) => item.id === "hard_error:error")).toBeUndefined();
  });
});

// ① UI 语义（owner 直批 codegraph 错误语义化）：空仓/成员无可索引代码
// 文件时，bootstrap 失败卡沿既有通道呈现业务文案——专用标题 + 成员
// 清单 +「每成员至少 1 个可索引代码文件」指引；通用 CLI 失败仍走既有
// 标题，不新建卡型。
describe("lc_bootstrap 聚合索引覆盖不足文案", () => {
  const projection = {
    project_id: "project_0001",
    logical_codebase_id: "logical_codebase_0001",
    membership_revision: 5,
  };

  it("覆盖不足：专用标题 + 无可索引文件的成员清单 + 每成员至少 1 个可索引代码文件", () => {
    const item = logicalCodebaseBootstrapItem(
      {
        key: "lc:aggregate_index_active:aggregate_index_cov_0001",
        step: "aggregate_index_active",
        object_id: "aggregate_index_cov_0001",
        reason_code: "aggregate_index_member_coverage_failed",
        summary:
          "aggregate_index_failed:aggregate_index_member_coverage_failed: no indexable code file under included members [api, web]; each included member needs at least 1 indexable code file",
        external_side_effect: "aggregate_index_cli",
        allowed_actions: ["retry"],
        next_step: null,
        created_at: "2026-10-12T00:00:00Z",
      },
      projection,
    );
    expect(item.title).toBe("聚合索引失败:成员代码覆盖不足");
    expect(item.summary).toContain("无可索引代码文件的成员：api, web");
    expect(item.summary).toContain("每成员至少 1 个可索引代码文件");
    expect(item.bootstrapInfo?.allowedActions).toEqual(["retry"]);
  });

  it("清单提取不到时保留指引文案；通用失败沿用既有标题", () => {
    const noList = logicalCodebaseBootstrapItem(
      {
        key: "lc:aggregate_index_active:aggregate_index_cov_0002",
        step: "aggregate_index_active",
        object_id: "aggregate_index_cov_0002",
        reason_code: "aggregate_index_member_coverage_failed",
        summary: "aggregate_index_failed:aggregate_index_member_coverage_failed: coverage lost",
        external_side_effect: "aggregate_index_cli",
        allowed_actions: ["retry"],
        next_step: null,
        created_at: "2026-10-12T00:00:00Z",
      },
      projection,
    );
    expect(noList.title).toBe("聚合索引失败:成员代码覆盖不足");
    expect(noList.summary).toContain("每成员至少 1 个可索引代码文件");

    const generic = logicalCodebaseBootstrapItem(
      {
        key: "lc:aggregate_index_active:aggregate_index_gen_0003",
        step: "aggregate_index_active",
        object_id: "aggregate_index_gen_0003",
        reason_code: "aggregate_index_failed",
        summary: "aggregate_index_failed:codegraph_init_failed: cli exploded",
        external_side_effect: "aggregate_index_cli",
        allowed_actions: ["retry"],
        next_step: null,
        created_at: "2026-10-12T00:00:00Z",
      },
      projection,
    );
    expect(generic.title).toBe("聚合索引首建失败，等待显式重试");
    expect(generic.summary).not.toContain("可索引代码文件");
  });
});

// P4-d（REQ-CRO-12/契约 A1 · REQ-UI37-19）：durable 待处理事项 → 收件箱
// 只读投影——7 类 kind（含 plan_final_confirm 聚合条）+ 未知未来 kind 全量
// 映射为 kind=coding_action/source=coding_attention，身份 [project,issue,key]。
describe("P4-d coding attention inbox projection", () => {
  const attention = (
    overrides: Partial<CodingAttentionItemDto> = {},
  ): CodingAttentionItemDto => ({
    key: "gate:attempt_0001:gate_0001",
    kind: "blocked",
    project_id: "project_0001",
    issue_id: "issue_0001",
    plan_id: null,
    attempt_id: "attempt_0001",
    logical_work_item_id: null,
    repository_name: null,
    reason_code: "upstream_source_unavailable",
    title: "执行被阻塞，需要处理",
    summary: "上游源不可读",
    occurred_at: "2026-10-15T08:00:00Z",
    target: { type: "coding_attempt", attempt_id: "attempt_0001" },
    focus: { gate_id: "gate_0001" },
    ...overrides,
  });

  it("projects every attention kind (incl. plan_final_confirm aggregate) without dropping entries", () => {
    const kinds = [
      "choice",
      "triage",
      "blocked",
      "stalled",
      "manual_recovery",
      "final_confirm",
      "plan_final_confirm",
    ];
    for (const kind of kinds) {
      const item = codingAttentionItem(attention({ kind, key: `k:${kind}` }));
      expect(item.kind).toBe("coding_action");
      expect(item.source).toBe("coding_attention");
      expect(item.title).toBe(attention({ kind }).title);
      expect(item.summary).toContain("上游源不可读");
      expect(item.createdAt).toBe("2026-10-15T08:00:00Z");
      expect(item.severity).toBe(2);
      expect(item.gate).toBeNull();
      expect(item.choice).toBeNull();
      expect(item.attentionInfo?.attentionKind).toBe(kind);
      expect(item.attentionInfo?.identity).toBe(
        codingAttentionIdentity(attention({ kind, key: `k:${kind}` })),
      );
    }
  });

  it("keeps unknown future kinds visible instead of filtering them out", () => {
    const item = codingAttentionItem(
      attention({ kind: "some_future_kind", key: "future:1" }),
    );
    expect(item.kind).toBe("coding_action");
    expect(item.attentionInfo?.attentionKind).toBe("some_future_kind");
  });

  it("uses full-scope identity [project,issue,key] and a non-session id prefix", () => {
    const item = attention({
      key: "gate:attempt_0001:gate_0001",
      project_id: "project_0001",
      issue_id: "issue_0001",
    });
    expect(codingAttentionIdentity(item)).toBe(
      JSON.stringify(["project_0001", "issue_0001", "gate:attempt_0001:gate_0001"]),
    );
    // 同 key 跨 project/issue 不合并。
    expect(codingAttentionIdentity(attention({ project_id: "project_0002" }))).not.toBe(
      codingAttentionIdentity(item),
    );
    expect(codingAttentionIdentity(attention({ issue_id: "issue_0002" }))).not.toBe(
      codingAttentionIdentity(item),
    );
    const projected = codingAttentionItem(item);
    // 收件箱 id 前缀不得被误读为 workspace session id（P4-e 走 attention 目标导航）。
    expect(cockpitInboxItemSessionId(projected.id)).toBeNull();
  });

  it("mirrors target/focus and optional identity fields for 去处理 navigation", () => {
    const planItem = codingAttentionItem(
      attention({
        kind: "plan_final_confirm",
        key: "plan-final:plan_0001:run_1",
        plan_id: "plan_0001",
        attempt_id: null,
        repository_name: "repo-a",
        logical_work_item_id: "wi_0009",
        reason_code: null,
        target: { type: "coding_plan", plan_id: "plan_0001" },
        focus: null,
      }),
    );
    expect(planItem.attentionInfo?.projectId).toBe("project_0001");
    expect(planItem.attentionInfo?.issueId).toBe("issue_0001");
    expect(planItem.attentionInfo?.planId).toBe("plan_0001");
    expect(planItem.attentionInfo?.attemptId).toBeNull();
    expect(planItem.attentionInfo?.repositoryName).toBe("repo-a");
    expect(planItem.attentionInfo?.logicalWorkItemId).toBe("wi_0009");
    expect(planItem.attentionInfo?.reasonCode).toBeNull();
    expect(planItem.attentionInfo?.target).toEqual({
      type: "coding_plan",
      plan_id: "plan_0001",
    });
    expect(planItem.attentionInfo?.focus).toBeNull();
    expect(planItem.summary).toContain("repo-a");
    expect(planItem.summary).toContain("wi_0009");

    const choiceItem = codingAttentionItem(
      attention({
        kind: "choice",
        key: "choice:attempt_0001:choice_0001",
        target: { type: "coding_attempt", attempt_id: "attempt_0001" },
        focus: { choice_id: "choice_0001" },
      }),
    );
    expect(choiceItem.attentionInfo?.focus).toEqual({ choice_id: "choice_0001" });
  });
});
