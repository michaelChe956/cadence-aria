import { describe, expect, it, vi } from "vitest";
import type { WorkspaceSessionSummary } from "../api/types";
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

  it("keeps API order for active candidates and excludes terminal statuses", () => {
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
    ).toEqual(["first", "second", "third"]);
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

  it("keys merged inbox entries by session and sorts severity before session id", () => {
    expect(
      selectObservedInbox([
        { sessionId: "z", state: observedState("z", { sessionStatus: "stopped_needs_human" }) },
        { sessionId: "a", state: observedState("a", { error: "连接失败" }) },
      ]).map((item) => item.id),
    ).toEqual(["a:hard_error:error", "z:stopped:z"]);
  });
});
