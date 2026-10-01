import { describe, expect, it } from "vitest";
import { useWorkspaceStore } from "./workspace-ws-store";
import { installWorkspaceStoreTestHooks } from "./workspace-ws-store.test-utils";
import {
  formatFlowElapsed,
  selectCockpitFlow,
  topologyTokenName,
} from "./workspace-cockpit-projection";
import {
  reviewVerdictEntry,
  timelineNode,
} from "./workspace-cockpit-projection.test-fixtures";

// 从 workspace-cockpit-projection.test.ts 拆出（large_file_guard 1200 行上限，纯移动零行为变化）：
// 执行流投影测试族。
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
  // #4：引擎 create_timeline_node 以终态 status 直接落「流程完成」标记节点，
  // 不带 completed_at/duration_ms——终态行不得拿墙钟续算，计时冻结。
  it("freezes elapsed for terminal nodes lacking completion facts instead of ticking (#4)", () => {
    const store = useWorkspaceStore.getState();
    store.setTimelineNodesForTest([
      timelineNode({
        node_id: "n1",
        node_type: "author_run",
        status: "completed",
        started_at: "2026-09-21T09:40:00Z",
        completed_at: "2026-09-21T09:50:00Z",
      }),
      timelineNode({
        node_id: "n2",
        node_type: "completed",
        status: "completed",
        started_at: "2026-09-21T10:00:00Z",
        completed_at: null,
        duration_ms: null,
      }),
      timelineNode({
        node_id: "n3",
        node_type: "author_run",
        status: "active",
        started_at: "2026-09-21T10:00:00Z",
      }),
    ]);

    const early = selectCockpitFlow(useWorkspaceStore.getState(), Date.parse("2026-09-21T10:01:00Z"));
    const late = selectCockpitFlow(useWorkspaceStore.getState(), Date.parse("2026-09-21T10:11:00Z"));

    // 完成事实齐备：冻结在 completed_at−started_at，不随墙钟变化。
    expect(early[0]).toMatchObject({ elapsed_ms: 10 * 60_000 });
    expect(late[0]).toMatchObject({ elapsed_ms: 10 * 60_000 });
    // 「流程完成」标记节点无结束事实：冻结为 0，不得继续计时。
    expect(early[1]).toMatchObject({ elapsed_ms: 0 });
    expect(late[1]).toMatchObject({ elapsed_ms: 0 });
    // 回归：进行中节点照常随墙钟递增。
    expect(early[2]).toMatchObject({ elapsed_ms: 60_000 });
    expect(late[2]).toMatchObject({ elapsed_ms: 11 * 60_000 });
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
