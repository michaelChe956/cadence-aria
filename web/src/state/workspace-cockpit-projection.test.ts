import { describe, expect, it } from "vitest";
import type { ChatEntry } from "./chat-entries";
import type { WorkspaceWsState } from "./workspace-ws-store-types";
import { useWorkspaceStore, type TimelineNode } from "./workspace-ws-store";
import { installWorkspaceStoreTestHooks } from "./workspace-ws-store.test-utils";
import {
  formatFlowElapsed,
  gateActionBlockCopy,
  gateActionBlockReason,
  gateTerminateBlockReason,
  isStaleDriverLeaseItem,
  selectCockpitFlow,
  selectCockpitInbox,
  selectGateProjection,
  topologyTokenName,
} from "./workspace-cockpit-projection";

function timelineNode(overrides: Partial<TimelineNode> = {}): TimelineNode {
  return {
    node_id: "node_1",
    node_type: "reviewer_run",
    agent: "codex",
    stage: "cross_review",
    round: 1,
    status: "active",
    title: "Review Round 1",
    summary: null,
    started_at: "2026-09-13T00:00:00Z",
    completed_at: null,
    duration_ms: null,
    artifact_ref: null,
    provider_config_snapshot: { author: "claude_code", reviewer: "codex", review_rounds: 1 },
    ...overrides,
  };
}

function reviewVerdictEntry(metadata: Record<string, unknown>): ChatEntry {
  return {
    id: "review_verdict:node_1",
    type: "review_verdict",
    role: "reviewer",
    content: "review",
    timestamp: "2026-09-13T00:00:00Z",
    node_id: "node_1",
    metadata,
  };
}

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

  it("keeps a completed-stage gate operable for the amendment reopen shape (0017-indistinguishable)", () => {
    useWorkspaceStore.setState({
      stage: "completed",
      flowKind: "single_candidate",
      singleCandidatePhase: "completed",
      humanGateSnapshot: humanGateSnapshotFixture(),
    });

    expect(gateActionBlockReason(useWorkspaceStore.getState())).toBeNull();
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
    expect(items[0]?.summary).toContain("同一问题重复出现");
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

describe("workspace cockpit execution flow projection", () => {
  installWorkspaceStoreTestHooks();

  it("maps timeline node statuses onto the six cockpit states", () => {
    const store = useWorkspaceStore.getState();
    store.setTimelineNodesForTest([
      timelineNode({ node_id: "n1", node_type: "author_run", status: "active" }),
      timelineNode({ node_id: "n2", node_type: "reviewer_run", status: "paused" }),
      timelineNode({ node_id: "n3", node_type: "revision", status: "skipped" }),
      timelineNode({ node_id: "n4", node_type: "completed", status: "completed" }),
      timelineNode({ node_id: "n5", node_type: "protocol_error", status: "failed" }),
    ]);

    expect(selectCockpitFlow(useWorkspaceStore.getState()).map((row) => row.state)).toEqual([
      "running",
      "blocked",
      "pending",
      "done",
      "failed",
    ]);
  });

  it("marks the active node as awaiting_triage when the gate is a triage gate", () => {
    const store = useWorkspaceStore.getState();
    store.setTimelineNodesForTest([
      timelineNode({ node_id: "n1", node_type: "reviewer_run", status: "active" }),
    ]);
    store.setActiveNodeId("n1");
    store.setStage("human_confirm");
    store.appendChatEntry(reviewVerdictEntry({ review_gate: "user_triage_required" }));

    const rows = selectCockpitFlow(useWorkspaceStore.getState());

    expect(rows[0]).toMatchObject({ state: "awaiting_triage", index: 1, total: 1 });
  });

  it("returns the active node to running once the triage gate is closed", () => {
    const store = useWorkspaceStore.getState();
    store.setTimelineNodesForTest([
      timelineNode({ node_id: "n1", node_type: "reviewer_run", status: "active" }),
    ]);
    store.setActiveNodeId("n1");
    store.setStage("human_confirm");
    store.appendChatEntry(reviewVerdictEntry({ review_gate: "user_triage_required" }));
    expect(selectCockpitFlow(useWorkspaceStore.getState())[0]?.state).toBe("awaiting_triage");

    store.applyHumanGateClosed("confirm", "human_confirm");

    expect(selectCockpitFlow(useWorkspaceStore.getState())[0]?.state).toBe("running");
  });

  it("computes progress, elapsed from duration_ms and the topology strip", () => {
    const store = useWorkspaceStore.getState();
    store.setTimelineNodesForTest([
      timelineNode({ node_id: "n1", status: "completed", duration_ms: 45_000 }),
      timelineNode({ node_id: "n2", status: "active", duration_ms: null }),
    ]);
    // 与组件一致：nowMs 是绝对 epoch 毫秒（组件传 Date.now()）
    const nowMs = Date.parse("2026-09-13T00:01:30Z");

    const rows = selectCockpitFlow(useWorkspaceStore.getState(), nowMs);

    expect(rows[0]).toMatchObject({ index: 1, total: 2, elapsed_ms: 45_000, topology: ["done", "running"] });
    expect(rows[1]).toMatchObject({ index: 2, total: 2, elapsed_ms: 90_000 });
    expect(rows[0]?.started_at).toBe("2026-09-13T00:00:00Z");
  });

  it("formats elapsed durations for display", () => {
    expect(formatFlowElapsed(0)).toBe("0s");
    expect(formatFlowElapsed(45_000)).toBe("45s");
    expect(formatFlowElapsed(90_000)).toBe("1m30s");
    expect(formatFlowElapsed(3_725_000)).toBe("1h2m");
  });

  // REQ-UI37-18「长时间无事件」：静默量取自最近一次引擎事件，而不是节点起点。
  it("measures idle time from the node last event rather than from the start", () => {
    const store = useWorkspaceStore.getState();
    store.setTimelineNodesForTest([
      timelineNode({
        node_id: "n1",
        status: "active",
        started_at: "2026-09-13T00:00:00Z",
        last_event_at: "2026-09-13T00:29:00Z",
      }),
    ]);

    const rows = selectCockpitFlow(useWorkspaceStore.getState(), Date.parse("2026-09-13T00:30:00Z"));

    expect(rows[0]).toMatchObject({ elapsed_ms: 30 * 60_000, idle_ms: 60_000 });
  });

  it("falls back to the node start when a node carries no last event", () => {
    const store = useWorkspaceStore.getState();
    store.setTimelineNodesForTest([timelineNode({ node_id: "n1", status: "active" })]);

    const rows = selectCockpitFlow(useWorkspaceStore.getState(), Date.parse("2026-09-13T00:30:00Z"));

    expect(rows[0]).toMatchObject({ elapsed_ms: 30 * 60_000, idle_ms: 30 * 60_000 });
  });

  it("maps cockpit states onto css token suffixes", () => {
    expect(topologyTokenName("awaiting_triage")).toBe("awaiting-triage");
    expect(topologyTokenName("running")).toBe("running");
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
      title: "门禁等待",
      triage: false,
      source: "gate",
      gate: expect.objectContaining({ key: "stage:author_confirm" }),
    });
  });
});
