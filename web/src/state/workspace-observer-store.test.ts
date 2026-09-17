import { describe, expect, it, vi } from "vitest";
import type { PlanProjectionBundle, WorkspaceSessionSummary } from "../api/types";
import type { WorkspaceWsState } from "./workspace-ws-store";
import {
  createObserverController,
  observerStateFromSessionState,
  selectObservedInbox,
  selectWatchedSessionIds,
  watchWindowCopy,
  type WorkspaceObserverSocketFactory,
} from "./workspace-observer-store";

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
    expect(watchWindowCopy(2, 15_000)).toBe("仅监视最近 2 个候选；集合外不计入计数，集合内准实时（最多 15 秒陈旧）");
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


  it("refreshes a record from the next scheduled connection snapshot", async () => {
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
    sockets[0]?.onSnapshot(observedState("a", { sessionStatus: "running" }));

    scheduled[0]?.();
    sockets[1]?.onSnapshot(observedState("a", { sessionStatus: "stopped_needs_human" }));

    expect(sockets).toHaveLength(2);
    expect(controller.records()).toEqual([
      { sessionId: "a", state: expect.objectContaining({ sessionStatus: "stopped_needs_human" }) },
    ]);
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
});
