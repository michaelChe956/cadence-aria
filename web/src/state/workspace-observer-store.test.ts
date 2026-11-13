import { describe, expect, it, vi } from "vitest";
import type { PlanProjectionBundle, WorkspaceSessionSummary } from "../api/types";
import type { WorkspaceWsState } from "./workspace-ws-store";
import {
  createObserverController,
  observerStateFromSessionState,
  reduceObserverMessage,
  selectObservedInbox,
  selectWatchedSessionIds,
  watchWindowCopy,
  type WorkspaceObserverSocketFactory,
} from "./workspace-observer-store";
import {
  lcFreshOperationFixture,
  lcResumeRecoveryFixture,
} from "./workspace-cockpit-projection.test-fixtures";

function summary(
  workspace_session_id: string,
  status: WorkspaceSessionSummary["status"] = "running",
): WorkspaceSessionSummary {
  return {
    workspace_session_id,
    issue_id: "issue_1",
    entity_id: `entity_${workspace_session_id}`,
    workspace_type: "work_item",
    status,
    author_provider: "claude_code",
    reviewer_provider: "codex",
    review_rounds: 1,
    superpowers_enabled: false,
    openspec_enabled: false,
    automation: { owner: "client", enrollment_id: null, policy_revision: null, enabled: false },
  };
}

function observedState(
  sessionId: string,
  overrides: Partial<WorkspaceWsState> = {},
): WorkspaceWsState {
  return {
    sessionId,
    stage: "running",
    sessionStatus: "running",
    humanGateSnapshot: null,
    humanGateTurn: null,
    humanGateClosure: null,
    flowKind: null,
    pendingReviewerSummary: null,
    chatEntries: [],
    protocolError: null,
    error: null,
    advanceCommands: {},
    ...overrides,
  } as WorkspaceWsState;
}

class ObserverMockWebSocket {
  static instances: ObserverMockWebSocket[] = [];

  readonly sent: string[] = [];
  readyState = 0;
  onopen: (() => void) | null = null;
  onmessage: ((event: { data: string }) => void) | null = null;
  onclose: (() => void) | null = null;
  onerror: (() => void) | null = null;

  constructor(_url: string) {
    ObserverMockWebSocket.instances.push(this);
  }

  send(message: string) {
    this.sent.push(message);
  }

  close() {
    this.readyState = 3;
  }

  open() {
    this.readyState = 1;
    this.onopen?.();
  }

  receive(payload: unknown) {
    this.onmessage?.({ data: JSON.stringify(payload) });
  }

  closed() {
    this.readyState = 3;
    this.onclose?.();
  }

  errored() {
    this.onerror?.();
  }
}

function sessionStateFrame(
  sessionId: string,
  eventSeq: number,
): Record<string, unknown> {
  return {
    type: "session_state",
    session_id: sessionId,
    workspace_type: "work_item",
    stage: "running",
    superpowers_enabled: false,
    openspec_enabled: false,
    messages: [],
    checkpoints: [],
    artifact: null,
    providers: { author: "claude_code", reviewer: null },
    timeline_nodes: [],
    active_node_id: null,
    artifact_versions: [],
    timeline_node_details: {},
    active_run_id: null,
    human_presentation_revisions: [],
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
    event_seq: eventSeq,
  };
}

describe("workspace observer store", () => {
  it("counts only watched records and tells the user K-external events are not real-time", () => {
    const ids = selectWatchedSessionIds(
      [summary("s1"), summary("s2"), summary("s3")],
      2,
    );
    const records = [
      { sessionId: "s1", state: observedState("s1", { sessionStatus: "stopped_needs_human" }) },
      { sessionId: "s3", state: observedState("s3", { error: "K 外事件" }) },
    ];

    expect(ids).toEqual(["s1", "s2"]);
    expect(
      selectObservedInbox(records.filter((record) => ids.includes(record.sessionId))),
    ).toHaveLength(1);
    expect(watchWindowCopy(2)).toBe("仅监视最近 2 个候选；集合外不计入计数，集合内实时推送（假死连接最多 5 分钟自愈）");
  });

  it("keeps API order for active candidates and excludes terminal statuses including failed", () => {
    expect(
      selectWatchedSessionIds(
        [
          summary("first", "open"),
          summary("terminal", "terminated"),
          summary("second", "waiting_for_human"),
          summary("blocked", "blocked_provider_unavailable"),
          summary("third", "failed"),
        ],
        3,
      ),
    ).toEqual(["first", "second"]);
  });

  it("does not let failed sessions consume watch slots when K is contested", () => {
    expect(
      selectWatchedSessionIds(
        [
          summary("f1", "failed"),
          summary("a1", "running"),
          summary("a2", "stopped_needs_human"),
          summary("a3", "open"),
          summary("f2", "failed"),
        ],
        2,
      ),
    ).toEqual(["a1", "a2"]);
  });

  it("declares observer role in the opening hello", async () => {
    ObserverMockWebSocket.instances = [];
    vi.stubGlobal("WebSocket", ObserverMockWebSocket);
    const controller = createObserverController();

    try {
      await controller.replaceWatchedSessionIds(["s1"]);
      const socket = ObserverMockWebSocket.instances[0];
      if (!socket) throw new Error("observer socket was not created");
      socket.open();

      expect(JSON.parse(socket.sent[0] ?? "")).toMatchObject({
        type: "hello",
        role: "observer",
      });
    } finally {
      controller.dispose();
      vi.unstubAllGlobals();
    }
  });

  it("opens only entries newly admitted to K and closes entries leaving K", async () => {
    const opened: string[] = [];
    const closed: string[] = [];
    const fakeSocketFactory: WorkspaceObserverSocketFactory = (sessionId) => {
      opened.push(sessionId);
      return { close: () => closed.push(sessionId) };
    };
    const controller = createObserverController(fakeSocketFactory);

    await controller.replaceWatchedSessionIds(["a", "b"]);
    await controller.replaceWatchedSessionIds(["b", "c"]);

    expect(opened).toEqual(["a", "b", "c"]);
    expect(closed).toEqual(["a"]);
    expect(controller.records()).toEqual([]);
  });

  // P3 WIGA Task 2（REQ-WIGA-07）：durable 业务 snapshot 帧唤醒补读（仅
  // 排程），ping/pong/游标帧不触发——收到 WS 事件不当完成事实。
  it("signals onSnapshotHint only for business snapshots, never for ping/pong frames", async () => {
    type SocketCallbacks = {
      onSnapshot: (state: WorkspaceWsState) => void;
      onFrame: (eventSeq: number | null) => void;
      onClose: () => void;
      onError: () => void;
    };
    const sockets: SocketCallbacks[] = [];
    const onSnapshotHint = vi.fn();
    const controller = createObserverController(
      (_sessionId, callbacks) => {
        sockets.push(callbacks as SocketCallbacks);
        return { close: () => undefined };
      },
      undefined,
      { refreshIntervalMs: 0, reconnectDelayMs: 0, onSnapshotHint },
    );
    await controller.replaceWatchedSessionIds(["a"]);

    sockets[0]?.onSnapshot(observedState("a"));
    expect(onSnapshotHint).toHaveBeenCalledTimes(1);

    // ping/pong 与游标推进只走 onFrame，不触发 hint。
    sockets[0]?.onFrame(null);
    sockets[0]?.onFrame(7);
    sockets[0]?.onFrame(8);
    expect(onSnapshotHint).toHaveBeenCalledTimes(1);

    // 后续业务 snapshot 再次唤醒（每帧都只调度一次补读）。
    sockets[0]?.onSnapshot(observedState("a", { sessionStatus: "waiting_for_human" }));
    expect(onSnapshotHint).toHaveBeenCalledTimes(2);

    controller.dispose();
  });

  // P1（无限轮询修复）：真实 socket 帧路径与既有注释对齐——pong 等
  // 未带来状态变化的帧（reducer 原样返回同一引用）不得触发
  // onSnapshot / onSnapshotHint；实质变化帧（stage_change）照常唤醒。
  it("does not wake snapshot listeners for changeless frames such as pong on the real socket path", async () => {
    ObserverMockWebSocket.instances = [];
    vi.stubGlobal("WebSocket", ObserverMockWebSocket);
    const onRecordsChange = vi.fn();
    const onSnapshotHint = vi.fn();
    const controller = createObserverController(undefined, onRecordsChange, {
      refreshIntervalMs: 0,
      reconnectDelayMs: 1_000,
      schedule: vi.fn(() => 0 as never),
      cancel: vi.fn(),
      onSnapshotHint,
    });

    try {
      await controller.replaceWatchedSessionIds(["s1"]);
      const socket = ObserverMockWebSocket.instances[0];
      if (!socket) throw new Error("observer socket was not created");
      socket.open();

      // 基线 session_state：建立观察态并唤醒一次。
      socket.receive(sessionStateFrame("s1", 7));
      const baselineNotifies = onRecordsChange.mock.calls.length;
      expect(onSnapshotHint).toHaveBeenCalledTimes(1);

      // pong 帧（无 event_seq、走 reducer default 分支）：不产生新快照、不唤醒。
      socket.receive({ type: "pong" });
      expect(onRecordsChange).toHaveBeenCalledTimes(baselineNotifies);
      expect(onSnapshotHint).toHaveBeenCalledTimes(1);

      // 实质变化帧照常上抛并唤醒。
      socket.receive({ type: "stage_change", stage: "human_confirm", event_seq: 8 });
      expect(onSnapshotHint).toHaveBeenCalledTimes(2);
      expect(controller.records()[0]?.state.stage).toBe("human_confirm");
    } finally {
      controller.dispose();
      vi.unstubAllGlobals();
    }
  });


  it("keeps healthy observer sockets long-lived across periodic ticks (F-06: no rebuild, no full-frame refresh)", async () => {
    type SocketCallbacks = {
      onSnapshot: (state: WorkspaceWsState) => void;
      onFrame: (eventSeq: number | null) => void;
      onClose: () => void;
      onError: () => void;
    };
    const sockets: SocketCallbacks[] = [];
    const closedSockets: number[] = [];
    const scheduled: Array<() => void> = [];
    let fakeNow = 0;
    const controller = createObserverController(
      (_sessionId, callbacks) => {
        sockets.push(callbacks as SocketCallbacks);
        const index = sockets.length - 1;
        return { close: () => closedSockets.push(index) };
      },
      undefined,
      {
        refreshIntervalMs: 15_000,
        reconnectDelayMs: 0,
        schedule: (callback) => {
          scheduled.push(callback);
          return 0 as never;
        },
        cancel: vi.fn(),
        now: () => fakeNow,
      },
    );
    await controller.replaceWatchedSessionIds(["a"]);
    sockets[0]?.onSnapshot(observedState("a", { sessionStatus: "running" }));

    // 静止期推进 15s/30s/45s：每 25s 仍有帧（pong/事件）到达 → 不重建、不重发全量
    fakeNow = 15_000;
    scheduled[0]?.();
    fakeNow = 25_000;
    sockets[0]?.onFrame(null);
    fakeNow = 30_000;
    scheduled[1]?.();
    fakeNow = 45_000;
    scheduled[2]?.();
    fakeNow = 50_000;
    sockets[0]?.onFrame(9);
    fakeNow = 60_000;
    scheduled[3]?.();

    expect(sockets).toHaveLength(1);
    expect(closedSockets).toEqual([]);
  });

  it("rebuilds an observer socket only after the stall threshold passes with no frames at all", async () => {
    type SocketCallbacks = {
      onSnapshot: (state: WorkspaceWsState) => void;
      onClose: () => void;
      onError: () => void;
    };
    const sockets: SocketCallbacks[] = [];
    const close = vi.fn();
    const scheduled: Array<() => void> = [];
    let fakeNow = 0;
    const controller = createObserverController(
      (_sessionId, callbacks) => {
        sockets.push(callbacks);
        return { close };
      },
      undefined,
      {
        refreshIntervalMs: 15_000,
        reconnectDelayMs: 0,
        schedule: (callback) => {
          scheduled.push(callback);
          return 0 as never;
        },
        cancel: vi.fn(),
        now: () => fakeNow,
      },
    );
    await controller.replaceWatchedSessionIds(["a"]);
    sockets[0]?.onSnapshot(observedState("a", { sessionStatus: "running" }));

    fakeNow = 15_000;
    scheduled[0]?.(); // 距建连 15s 无帧：未达阈值，不重建
    expect(close).not.toHaveBeenCalled();

    fakeNow = 5 * 60_000;
    scheduled[1]?.(); // ≥5min 无任何帧：假死兜底重建
    expect(close).toHaveBeenCalledTimes(1);
    expect(sockets).toHaveLength(2);

    // 重建后的新连接重新起算假死窗口；其快照仍进入 records
    sockets[1]?.onSnapshot(observedState("a", { sessionStatus: "stopped_needs_human" }));
    expect(controller.records()).toEqual([
      { sessionId: "a", state: expect.objectContaining({ sessionStatus: "stopped_needs_human" }) },
    ]);
    fakeNow = 5 * 60_000 + 4 * 60_000;
    scheduled[2]?.(); // 新连接 4min 无帧：仍不重建
    expect(close).toHaveBeenCalledTimes(1);
  });

  it("carries the last session event_seq cursor in reconnect hello without cross-session leakage", async () => {
    ObserverMockWebSocket.instances = [];
    vi.stubGlobal("WebSocket", ObserverMockWebSocket);
    const scheduled: Array<() => void> = [];
    const controller = createObserverController(undefined, undefined, {
      refreshIntervalMs: 0,
      reconnectDelayMs: 1_000,
      schedule: (callback) => {
        scheduled.push(callback);
        return 0 as never;
      },
      cancel: vi.fn(),
    });

    try {
      await controller.replaceWatchedSessionIds(["s1", "s2"]);
      const s1 = ObserverMockWebSocket.instances[0];
      const s2 = ObserverMockWebSocket.instances[1];
      if (!s1 || !s2) throw new Error("observer sockets were not created");
      s1.open();
      s2.open();
      expect(JSON.parse(s1.sent[0])).not.toHaveProperty("after_event_seq");

      s1.receive(sessionStateFrame("s1", 7));
      s2.receive(sessionStateFrame("s2", 3));

      s1.closed();
      scheduled.at(-1)?.(); // 重连计时器触发 → 只重建 s1

      expect(ObserverMockWebSocket.instances).toHaveLength(3);
      const replacement = ObserverMockWebSocket.instances[2];
      replacement?.open();
      expect(JSON.parse(replacement?.sent[0] ?? "")).toMatchObject({
        type: "hello",
        session_id: "s1",
        role: "observer",
        after_event_seq: 7,
      });
    } finally {
      controller.dispose();
      vi.unstubAllGlobals();
    }
  });

  it("applies replayed incremental frames on the seeded state after cursor reconnect and dedups stale overlap", async () => {
    ObserverMockWebSocket.instances = [];
    vi.stubGlobal("WebSocket", ObserverMockWebSocket);
    const scheduled: Array<() => void> = [];
    const controller = createObserverController(undefined, undefined, {
      refreshIntervalMs: 0,
      reconnectDelayMs: 1_000,
      schedule: (callback) => {
        scheduled.push(callback);
        return 0 as never;
      },
      cancel: vi.fn(),
    });

    try {
      await controller.replaceWatchedSessionIds(["s1"]);
      const first = ObserverMockWebSocket.instances[0];
      if (!first) throw new Error("observer socket was not created");
      first.open();
      first.receive(sessionStateFrame("s1", 7));
      expect(controller.records()[0]?.state.stage).toBe("running");

      first.closed();
      scheduled.at(-1)?.();
      const replacement = ObserverMockWebSocket.instances[1];
      if (!replacement) throw new Error("replacement socket was not created");
      replacement.open();
      expect(JSON.parse(replacement.sent[0])).toMatchObject({ after_event_seq: 7 });

      // cursor 回放帧（无 session_state 基线）直接落在断线前的观察态上
      replacement.receive({ type: "stage_change", stage: "human_confirm", event_seq: 8 });
      expect(controller.records()[0]?.state.stage).toBe("human_confirm");

      // 回放/直播重叠的旧帧按 event_seq 去重，不回退状态
      replacement.receive({ type: "stage_change", stage: "running", event_seq: 6 });
      expect(controller.records()[0]?.state.stage).toBe("human_confirm");

      // 事件驱动更新仍达：新事件照常推进
      replacement.receive({ type: "stage_change", stage: "reviewing", event_seq: 9 });
      expect(controller.records()[0]?.state.stage).toBe("reviewing");
    } finally {
      controller.dispose();
      vi.unstubAllGlobals();
    }
  });

  it("reschedules the next observed refresh when the interval changes", async () => {
    type ScheduledRefresh = {
      callback: () => void;
      delayMs: number;
      cancelled: boolean;
    };
    const scheduled: ScheduledRefresh[] = [];
    const controller = createObserverController(
      () => ({ close: vi.fn() }),
      undefined,
      {
        refreshIntervalMs: 15_000,
        reconnectDelayMs: 1_000,
        schedule: (callback, delayMs) => {
          const refresh = { callback, delayMs, cancelled: false };
          scheduled.push(refresh);
          return refresh as never;
        },
        cancel: (refresh) => {
          (refresh as unknown as ScheduledRefresh).cancelled = true;
        },
      },
    );

    controller.updateRefreshIntervalMs(5_000);

    expect(scheduled).toEqual([
      expect.objectContaining({ delayMs: 15_000, cancelled: true }),
      expect.objectContaining({ delayMs: 5_000, cancelled: false }),
    ]);
    scheduled[1]?.callback();
    expect(scheduled[2]).toEqual(
      expect.objectContaining({ delayMs: 5_000, cancelled: false }),
    );
  });

  it("reopens after a socket closes instead of retaining a dead K slot", async () => {
    type SocketCallbacks = {
      onSnapshot: (state: WorkspaceWsState) => void;
      onClose: () => void;
      onError: () => void;
    };
    const sockets: SocketCallbacks[] = [];
    const scheduled: Array<() => void> = [];
    const controller = createObserverController(
      (_sessionId, callbacks) => {
        sockets.push(callbacks);
        return { close: vi.fn() };
      },
      undefined,
      {
        refreshIntervalMs: 15_000,
        reconnectDelayMs: 0,
        schedule: (callback) => {
          scheduled.push(callback);
          return 0 as never;
        },
        cancel: vi.fn(),
      },
    );
    await controller.replaceWatchedSessionIds(["a"]);
    sockets[0]?.onClose();
    scheduled.at(-1)?.();

    expect(sockets).toHaveLength(2);
  });

  it("reopens after a socket error instead of retaining a dead K slot", async () => {
    type SocketCallbacks = {
      onSnapshot: (state: WorkspaceWsState) => void;
      onClose: () => void;
      onError: () => void;
    };
    const sockets: SocketCallbacks[] = [];
    const scheduled: Array<() => void> = [];
    const controller = createObserverController(
      (_sessionId, callbacks) => {
        sockets.push(callbacks);
        return { close: vi.fn() };
      },
      undefined,
      {
        refreshIntervalMs: 15_000,
        reconnectDelayMs: 0,
        schedule: (callback) => {
          scheduled.push(callback);
          return 0 as never;
        },
        cancel: vi.fn(),
      },
    );
    await controller.replaceWatchedSessionIds(["a"]);
    sockets[0]?.onError();
    scheduled.at(-1)?.();

    expect(sockets).toHaveLength(2);
  });
  it("builds a complete observer snapshot with child conversation content", () => {
    const state = observerStateFromSessionState({
      type: "session_state",
      session_id: "child_001",
      workspace_type: "work_item",
      stage: "running",
      superpowers_enabled: false,
      openspec_enabled: false,
      messages: [{ id: "message_001", role: "author", content: "子会话对话", created_at: "2026-09-14T00:00:00Z" }],
      checkpoints: [],
      artifact: null,
      providers: { author: "claude_code", reviewer: null },
      timeline_nodes: [],
      active_node_id: null,
      artifact_versions: [],
      timeline_node_details: {},
      active_run_id: null,
      human_presentation_revisions: [],
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
    });

    expect(state.timelineNodes).toEqual([]);
    expect(state.protocolDiagnostics).toEqual([]);
    expect(state.chatEntries).toEqual([
      expect.objectContaining({ content: "子会话对话" }),
    ]);
  });

  it("derives artifact versions and plan projection for the takeover plan approval view", () => {
    const planProjection: PlanProjectionBundle = {
      id: "bundle_0001",
      plan_revision_id: "plan_rev_0002",
      dependency_graph_revision_id: "graph_rev_0001",
      work_item_projection_bundle_refs: [],
      human_group_projection: {
        plan_id: "plan_0001",
        goal: "g",
        split_reason: "s",
        work_items: [],
        contract_flow: [],
        risks: [],
        source_refs: [],
        normative: false,
        used_by_provider: false,
      },
      coder_group_context: {
        plan_id: "plan_0001",
        ordered_logical_work_item_ids: [],
        dependency_edges: [],
        group_write_scopes: {},
      },
      reviewer_group_matrix: {
        plan_id: "plan_0001",
        work_items: [],
        dependency_edges: [],
        design_traceability_refs: [],
      },
      human_group_projection_hash: "h1",
      coder_group_context_hash: "h2",
      reviewer_group_matrix_hash: "h3",
      compiler_version: "v1",
      created_at: "2026-09-14T08:00:00Z",
    };
    const state = observerStateFromSessionState({
      type: "session_state",
      session_id: "child_plan_001",
      workspace_type: "work_item_plan",
      stage: "human_confirm",
      superpowers_enabled: false,
      openspec_enabled: false,
      messages: [],
      checkpoints: [],
      artifact: null,
      providers: { author: "claude_code", reviewer: null },
      timeline_nodes: [],
      active_node_id: null,
      artifact_versions: [
        {
          version: 1,
          generated_by: "claude_code",
          reviewed_by: null,
          review_verdict: null,
          confirmed_by: null,
          is_current: false,
          created_at: "2026-09-14T08:00:00Z",
          source_node_id: "node_1",
        },
        {
          version: 2,
          generated_by: "claude_code",
          reviewed_by: null,
          review_verdict: null,
          confirmed_by: null,
          is_current: true,
          created_at: "2026-09-14T09:00:00Z",
          source_node_id: "node_2",
          plan_projection: planProjection,
        },
      ],
      artifact_version_summaries: [
        {
          version: 1,
          generated_by: "claude_code",
          created_at: "2026-09-14T08:00:00Z",
          source_node_id: "node_1",
        },
        {
          version: 2,
          generated_by: "claude_code",
          created_at: "2026-09-14T09:00:00Z",
          source_node_id: "node_2",
          is_current: true,
        },
      ],
      timeline_node_details: {},
      active_run_id: null,
      human_presentation_revisions: [],
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
    });

    expect(state.artifactVersions.map((version) => version.version)).toEqual([1, 2]);
    expect(state.workItemPlanProjectionArtifacts.planProjection?.id).toBe(
      "bundle_0001",
    );
  });

  it("keys merged inbox entries by session and sorts severity before session id", () => {
    expect(
      selectObservedInbox([
        { sessionId: "z", state: observedState("z", { sessionStatus: "stopped_needs_human" }) },
        { sessionId: "a", state: observedState("a", { error: "连接失败" }) },
      ]).map((item) => item.id),
    ).toEqual(["a:hard_error:error", "z:stopped:z"]);
  });
  it("REQ-CFC-06 场景2：human_gate_closed 广播使观察连接的收件箱门消失（无需刷新）", () => {
    const state = observerStateFromSessionState({
      type: "session_state",
      session_id: "s2",
      workspace_type: "work_item",
      stage: "human_confirm",
      superpowers_enabled: false,
      openspec_enabled: false,
      messages: [],
      checkpoints: [],
      artifact: null,
      providers: { author: "claude_code", reviewer: null },
      timeline_nodes: [],
      active_node_id: null,
      artifact_versions: [],
      timeline_node_details: {},
      active_run_id: null,
      human_presentation_revisions: [],
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
    });
    const openBefore = selectObservedInbox([{ sessionId: "s2", state }]).some(
      (item) => item.kind === "gate",
    );
    expect(openBefore).toBe(true);

    const closed = reduceObserverMessage(state, {
      type: "human_gate_closed",
      decision: "confirm",
      stage: "human_confirm",
    });

    expect(selectObservedInbox([{ sessionId: "s2", state: closed }]).some(
      (item) => item.kind === "gate",
    )).toBe(false);
  });

});

describe("automation ownership projection (P0 1.2)", () => {
  const baseSnapshot = {
    type: "session_state",
    session_id: "session_auto_1",
    workspace_type: "work_item_plan",
    stage: "running",
    superpowers_enabled: false,
    openspec_enabled: false,
    messages: [],
    checkpoints: [],
    artifact: null,
    providers: { author: "claude_code", reviewer: null },
    timeline_nodes: [],
    active_node_id: null,
    artifact_versions: [],
    timeline_node_details: {},
    active_run_id: null,
    human_presentation_revisions: [],
    session_status: "running",
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
  } as const;

  it("projects server ownership verbatim from session_state automation", () => {
    const state = observerStateFromSessionState({
      ...baseSnapshot,
      automation: {
        owner: "server",
        enrollment_id: "en-1",
        policy_revision: 2,
        enabled: true,
      },
    } as unknown as Parameters<typeof observerStateFromSessionState>[0]);
    expect(state.automation?.owner).toBe("server");
    expect(state.automation?.policy_revision).toBe(2);
    expect(state.automation?.enabled).toBe(true);
  });

  it("keeps ownership unknown (null) when automation is absent", () => {
    const state = observerStateFromSessionState({
      ...baseSnapshot,
    } as unknown as Parameters<typeof observerStateFromSessionState>[0]);
    expect(state.automation).toBeNull();
  });

  it("projects disabled enrollment as client with revision", () => {
    const state = observerStateFromSessionState({
      ...baseSnapshot,
      automation: {
        owner: "client",
        enrollment_id: "en-1",
        policy_revision: 3,
        enabled: false,
      },
    } as unknown as Parameters<typeof observerStateFromSessionState>[0]);
    expect(state.automation?.owner).toBe("client");
    expect(state.automation?.policy_revision).toBe(3);
    expect(state.automation?.enabled).toBe(false);
  });
});

describe("REQ-HGC-04 observer 镜像：plan_continue_available", () => {
  // observer 镜像是「最易漏的一处」：driver 落位而观察态不落位 ⇒ 观察者看到
  // 的门卡与驾驶者不同（一份说有入口、一份说没有）。
  it("projects the continue fact verbatim (true)", () => {
    const state = observerStateFromSessionState({
      ...sessionStateFrame("session_obs_continue", 1),
      plan_continue_available: true,
    } as never);
    expect(state.planContinueAvailable).toBe(true);
  });

  it("keeps the continue fact unknown (null) when the field is absent", () => {
    const state = observerStateFromSessionState(
      sessionStateFrame("session_obs_continue_absent", 1) as never,
    );
    expect(state.planContinueAvailable).toBeNull();
  });

  it("projects an explicit false (server says no entry) as false", () => {
    const state = observerStateFromSessionState({
      ...sessionStateFrame("session_obs_continue_false", 1),
      plan_continue_available: false,
    } as never);
    expect(state.planContinueAvailable).toBe(false);
  });
});

describe("anchor_t19 observer authoritative lc resume recovery", () => {
  it("anchor_t19_driver_observer_recovery_projection_and_resolved_clear", () => {
    // 全量帧（重连/首连同一入口）：observerStateFromSessionState 显式落
    // lcResumeRecovery 字段（不因 as unknown 掩码漏投影），与 driver 同帧同 ID。
    const waiting = observerStateFromSessionState({
      ...sessionStateFrame("session_obs", 1),
      lc_resume_recovery: lcResumeRecoveryFixture({ state: "waiting" }),
    } as never);
    expect(waiting.lcResumeRecovery?.recovery_id).toBe("rec-1");
    expect(waiting.lcResumeRecovery?.state).toBe("waiting");
    expect(waiting.lcResumeRecovery?.allowed_actions).toEqual([]);

    // 增量推进：权威 SessionState 重基线帧序列，最新帧胜出。
    const reserved = observerStateFromSessionState({
      ...sessionStateFrame("session_obs", 2),
      lc_resume_recovery: lcResumeRecoveryFixture({
        state: "fresh_reserved",
        last_operation: lcFreshOperationFixture({ state: "reserved" }),
      }),
    } as never);
    expect(reserved.lcResumeRecovery).toMatchObject({
      recovery_id: "rec-1",
      state: "fresh_reserved",
      last_operation: { operation_id: "op-1", state: "reserved" },
    });

    // resolved：证据保留在 store，观察收件箱不投恢复卡（卡消失）。
    const resolved = observerStateFromSessionState({
      ...sessionStateFrame("session_obs", 3),
      lc_resume_recovery: lcResumeRecoveryFixture({
        state: "resolved",
        last_operation: lcFreshOperationFixture({ state: "completed", fresh_node: "node-2" }),
      }),
    } as never);
    expect(resolved.lcResumeRecovery).toMatchObject({
      recovery_id: "rec-1",
      state: "resolved",
      last_operation: { state: "completed", fresh_node: "node-2" },
    });
    expect(
      selectObservedInbox([{ sessionId: "session_obs", state: resolved }]).some(
        (item) => item.kind === "lc_resume_recovery",
      ),
    ).toBe(false);

    // 等待态观察收件箱投恢复卡（observer 只见事实，不发写动作）。
    expect(
      selectObservedInbox([{ sessionId: "session_obs", state: waiting }]).some(
        (item) => item.kind === "lc_resume_recovery",
      ),
    ).toBe(true);

    // null：字段缺席的权威帧 → 观察态清空（不保留陈旧恢复）。
    const cleared = observerStateFromSessionState({
      ...sessionStateFrame("session_obs", 4),
    } as never);
    expect(cleared.lcResumeRecovery).toBeNull();
    expect(
      selectObservedInbox([{ sessionId: "session_obs", state: cleared }]).some(
        (item) => item.kind === "lc_resume_recovery",
      ),
    ).toBe(false);

    // 畸形载荷：观察态不落半成品事实（无旧帧可保留 → null，不猜）。
    const malformed = observerStateFromSessionState({
      ...sessionStateFrame("session_obs", 5),
      lc_resume_recovery: { ...lcResumeRecoveryFixture(), state: "bogus" },
    } as never);
    expect(malformed.lcResumeRecovery).toBeNull();
  });
});
