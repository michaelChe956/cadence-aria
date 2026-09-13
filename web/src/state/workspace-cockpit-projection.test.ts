import { describe, expect, it } from "vitest";
import type { ChatEntry } from "./chat-entries";
import { useWorkspaceStore, type TimelineNode } from "./workspace-ws-store";
import { installWorkspaceStoreTestHooks } from "./workspace-ws-store.test-utils";
import {
  formatFlowElapsed,
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

describe("workspace cockpit gate projection", () => {
  installWorkspaceStoreTestHooks();

  it("returns no gate projection without a turn, snapshot or legacy stage", () => {
    expect(selectGateProjection(useWorkspaceStore.getState())).toBeNull();
  });

  it("projects the legacy human_confirm stage gate", () => {
    useWorkspaceStore.getState().setStage("human_confirm");

    expect(selectGateProjection(useWorkspaceStore.getState())).toMatchObject({
      key: "legacy:human_confirm",
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
      key: "snapshot:running",
      trigger: "verification_new_findings",
      remaining_budget: 1,
      resumable: true,
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
      id: "gate:snapshot:human_confirm",
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
