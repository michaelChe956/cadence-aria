import type { WorkspaceSessionSummary, WsOutMessage } from "../api/types";
import { workspaceSessionWebSocketUrl } from "../api/client";
import type { CockpitInboxItem } from "./workspace-cockpit-projection";
import { selectCockpitInbox } from "./workspace-cockpit-projection";
import type {
  AdvanceCommandState,
  HumanGateTurnState,
  WorkspaceWsState,
} from "./workspace-ws-store";

// watch 候选（REQ-UI37-07）= 活跃集合 {open, running, waiting_for_human, confirmed, change_requested, stopped_needs_human}。
// 排除项:failed/terminated 为终态,不占 K 槽,历史卡壳走 K 外降级语义;blocked_provider_unavailable 为供给方
// 临时不可用(非会话卡壳、无可交互动作),同样不占 K 槽。
const WATCHED_SESSION_STATUSES: ReadonlySet<WorkspaceSessionSummary["status"]> = new Set([
  "open",
  "running",
  "waiting_for_human",
  "confirmed",
  "change_requested",
  "stopped_needs_human",
]);
const OBSERVER_PING_INTERVAL_MS = 25_000;

type WorkspaceSessionStateMessage = Extract<
  WsOutMessage,
  { type: "session_state" }
>;
type WorkspaceObserverMessage = WsOutMessage & Record<string, unknown>;

export interface WorkspaceObserverRecord {
  sessionId: string;
  state: WorkspaceWsState;
}

export interface WorkspaceObserverSocket {
  close(): void;
}

export interface WorkspaceObserverSocketCallbacks {
  onSnapshot(state: WorkspaceWsState): void;
  onClose(): void;
  onError(): void;
}

export type WorkspaceObserverSocketFactory = (
  sessionId: string,
  callbacks: WorkspaceObserverSocketCallbacks,
) => WorkspaceObserverSocket;

export interface WorkspaceObserverControllerOptions {
  refreshIntervalMs: number;
  reconnectDelayMs: number;
  schedule?(callback: () => void, delayMs: number): ReturnType<typeof setTimeout>;
  cancel?(timer: ReturnType<typeof setTimeout>): void;
}

export interface WorkspaceObserverController {
  replaceWatchedSessionIds(sessionIds: readonly string[]): Promise<void>;
  updateRefreshIntervalMs(refreshIntervalMs: number): void;
  refresh(): Promise<void>;
  records(): readonly WorkspaceObserverRecord[];
  dispose(): void;
}

export function selectWatchedSessionIds(
  sessions: readonly WorkspaceSessionSummary[],
  watchLimit: number,
): readonly string[] {
  if (watchLimit <= 0) {
    return [];
  }

  return sessions
    .map((session, index) => ({ session, index }))
    .filter(({ session }) => WATCHED_SESSION_STATUSES.has(session.status))
    .sort(
      (left, right) =>
        left.index - right.index ||
        left.session.workspace_session_id.localeCompare(right.session.workspace_session_id),
    )
    .slice(0, watchLimit)
    .map(({ session }) => session.workspace_session_id);
}

export function watchWindowCopy(watchLimit: number, refreshIntervalMs: number): string {
  return `仅监视最近 ${watchLimit} 个候选；集合外不计入计数，集合内准实时（最多 ${Math.ceil(refreshIntervalMs / 1000)} 秒陈旧）`;
}

export function selectObservedInbox(
  records: readonly WorkspaceObserverRecord[],
): CockpitInboxItem[] {
  return records
    .flatMap(({ sessionId, state }) =>
      selectCockpitInbox(state).map((item) => ({ sessionId, item })),
    )
    .sort(
      (left, right) =>
        right.item.severity - left.item.severity ||
        left.sessionId.localeCompare(right.sessionId) ||
        left.item.id.localeCompare(right.item.id),
    )
    .map(({ sessionId, item }) => ({ ...item, id: `${sessionId}:${item.id}` }));
}

export function createObserverController(
  socketFactory: WorkspaceObserverSocketFactory = createWorkspaceObserverSocket,
  onRecordsChange: (records: readonly WorkspaceObserverRecord[]) => void = () => undefined,
  options: WorkspaceObserverControllerOptions = {
    refreshIntervalMs: 15_000,
    reconnectDelayMs: 1_000,
  },
): WorkspaceObserverController {
  const sockets = new Map<string, WorkspaceObserverSocket>();
  const snapshots = new Map<string, WorkspaceWsState>();
  const watchedSessionIds = new Set<string>();
  const reconnectTimers = new Map<string, ReturnType<typeof setTimeout>>();
  const schedule = options.schedule ?? setTimeout;
  const cancel = options.cancel ?? clearTimeout;
  let refreshTimer: ReturnType<typeof setTimeout> | null = null;
  let disposed = false;
  let refreshIntervalMs = options.refreshIntervalMs;

  const notifyRecordsChanged = () => {
    onRecordsChange(Array.from(snapshots, ([sessionId, state]) => ({ sessionId, state })));
  };
  const clearReconnect = (sessionId: string) => {
    const timer = reconnectTimers.get(sessionId);
    if (timer) {
      cancel(timer);
      reconnectTimers.delete(sessionId);
    }
  };
  const clearSocket = (sessionId: string) => {
    clearReconnect(sessionId);
    const socket = sockets.get(sessionId);
    sockets.delete(sessionId);
    socket?.close();
  };
  const ensureSocket = (sessionId: string) => {
    if (disposed || !watchedSessionIds.has(sessionId) || sockets.has(sessionId)) {
      return;
    }
    sockets.set(
      sessionId,
      socketFactory(sessionId, {
        onSnapshot: (state) => {
          if (!disposed && watchedSessionIds.has(sessionId)) {
            snapshots.set(sessionId, state);
            notifyRecordsChanged();
          }
        },
        onClose: () => scheduleReconnect(sessionId),
        onError: () => scheduleReconnect(sessionId),
      }),
    );
  };
  const scheduleReconnect = (sessionId: string) => {
    if (disposed || !watchedSessionIds.has(sessionId) || reconnectTimers.has(sessionId)) {
      return;
    }
    sockets.delete(sessionId);
    const timer = schedule(() => {
      reconnectTimers.delete(sessionId);
      ensureSocket(sessionId);
    }, options.reconnectDelayMs);
    reconnectTimers.set(sessionId, timer);
  };
  const scheduleRefresh = () => {
    if (disposed || refreshIntervalMs <= 0) {
      return;
    }
    refreshTimer = schedule(() => {
      refreshTimer = null;
      void refresh();
      scheduleRefresh();
    }, refreshIntervalMs);
  };
  const refresh = async () => {
    for (const sessionId of watchedSessionIds) {
      clearSocket(sessionId);
      ensureSocket(sessionId);
    }
  };

  scheduleRefresh();

  return {
    async replaceWatchedSessionIds(sessionIds) {
      const nextIds = new Set(sessionIds);
      for (const sessionId of watchedSessionIds) {
        if (!nextIds.has(sessionId)) {
          clearSocket(sessionId);
          watchedSessionIds.delete(sessionId);
          snapshots.delete(sessionId);
        }
      }
      for (const sessionId of sessionIds) {
        watchedSessionIds.add(sessionId);
        ensureSocket(sessionId);
      }
      notifyRecordsChanged();
    },

    updateRefreshIntervalMs(nextRefreshIntervalMs) {
      if (refreshIntervalMs === nextRefreshIntervalMs) {
        return;
      }
      refreshIntervalMs = nextRefreshIntervalMs;
      if (refreshTimer !== null) {
        cancel(refreshTimer);
        refreshTimer = null;
      }
      scheduleRefresh();
    },

    refresh,

    records() {
      return Array.from(snapshots, ([sessionId, state]) => ({ sessionId, state }));
    },

    dispose() {
      disposed = true;
      if (refreshTimer) {
        cancel(refreshTimer);
      }
      for (const sessionId of watchedSessionIds) {
        clearSocket(sessionId);
      }
      watchedSessionIds.clear();
      sockets.clear();
      snapshots.clear();
      notifyRecordsChanged();
    },
  };
}

function createWorkspaceObserverSocket(
  sessionId: string,
  callbacks: WorkspaceObserverSocketCallbacks,
): WorkspaceObserverSocket {
  const socket = new WebSocket(workspaceSessionWebSocketUrl(sessionId));
  let state: WorkspaceWsState | null = null;
  let pingTimer: ReturnType<typeof setInterval> | null = null;
  let closed = false;

  const close = () => {
    if (closed) {
      return;
    }
    closed = true;
    if (pingTimer) {
      clearInterval(pingTimer);
      pingTimer = null;
    }
    socket.close();
  };
  const notifyClosed = () => {
    if (!closed) {
      closed = true;
      if (pingTimer) {
        clearInterval(pingTimer);
        pingTimer = null;
      }
      callbacks.onClose();
    }
  };

  socket.onopen = () => {
    socket.send(JSON.stringify({ type: "hello", session_id: sessionId, last_seen_node_id: null }));
    pingTimer = setInterval(() => {
      if (socket.readyState === WebSocket.OPEN) {
        socket.send(JSON.stringify({ type: "ping" }));
      }
    }, OBSERVER_PING_INTERVAL_MS);
  };
  socket.onclose = notifyClosed;
  socket.onerror = notifyClosed;
  socket.onmessage = (event) => {
    try {
      const message = JSON.parse(event.data) as WorkspaceObserverMessage;
      if (message.type === "session_state" && message.session_id === sessionId) {
        state = observerStateFromSessionState(message as WorkspaceSessionStateMessage);
        callbacks.onSnapshot(state);
      } else if (state) {
        state = reduceObserverMessage(state, message);
        callbacks.onSnapshot(state);
      }
    } catch {
      // observer 仅消费可识别帧，畸形帧不能影响当前工作区连接。
    }
  };

  return { close };
}

export function observerStateFromSessionState(
  message: WorkspaceSessionStateMessage,
): WorkspaceWsState {
  const timelineNodes = message.timeline_nodes ?? [];
  return {
    sessionId: message.session_id,
    workspaceType: message.workspace_type,
    stage: message.stage,
    sessionStatus: message.session_status,
    flowKind: message.flow_kind,
    humanGateSnapshot: message.human_gate_snapshot ?? null,
    humanGateTurn: null,
    humanGateClosure: null,
    pendingReviewerSummary: null,
    chatEntries: message.messages.map((message, index) => ({
      id: `message:${index}`,
      type: "provider_stream",
      role: message.role === "user" ? "user" : "system",
      content: message.content,
      timestamp: message.created_at,
    })),
    timelineNodes,
    activeNodeId: message.active_node_id ?? null,
    selectedNodeId: message.active_node_id ?? timelineNodes.at(-1)?.node_id ?? null,
    protocolDiagnostics: [],
    protocolError: null,
    error: null,
    advanceCommands: {},
    snapshotGateOpenedAt: message.human_gate_snapshot ? new Date().toISOString() : null,
  } as unknown as WorkspaceWsState;
}

function reduceObserverMessage(
  state: WorkspaceWsState,
  message: WorkspaceObserverMessage,
): WorkspaceWsState {
  switch (message.type) {
    case "stage_change":
      return { ...state, stage: String(message.stage) };
    case "error":
      return { ...state, error: String(message.message) };
    case "protocol_error":
      return { ...state, protocolError: { code: String(message.code), message: String(message.message) } };
    case "review_complete":
      return {
        ...state,
        pendingReviewerSummary:
          message.verdict === "needs_human"
            ? { verdict: "needs_human", points: [String(message.summary)] }
            : null,
      };
    case "human_gate_turn_open":
      return {
        ...state,
        humanGateTurn: {
          turn_id: String(message.turn_id),
          command_id: String(message.command_id),
          remaining_budget: Number(message.remaining_budget),
          status: "open",
          artifact_ref: null,
          failure_class: null,
          failure_message: null,
          opened_at: new Date().toISOString(),
          inlineError: null,
        } satisfies HumanGateTurnState,
      };
    case "human_gate_turn_completed":
      return state.humanGateTurn?.turn_id === message.turn_id
        ? {
            ...state,
            humanGateTurn: {
              ...state.humanGateTurn,
              status: "awaiting_confirm",
              artifact_ref: String(message.artifact_ref),
            },
          }
        : state;
    case "human_gate_turn_failed":
      return state.humanGateTurn?.turn_id === message.turn_id
        ? {
            ...state,
            humanGateTurn: {
              ...state.humanGateTurn,
              status: "failed",
              failure_class: String(message.failure_class),
              failure_message: String(message.message),
            },
          }
        : state;
    case "human_gate_busy":
      return state.humanGateTurn?.turn_id === message.turn_id
        ? { ...state, humanGateTurn: { ...state.humanGateTurn, status: "busy" } }
        : state;
    case "human_gate_closed":
      return {
        ...state,
        humanGateClosure: { decision: message.decision, stage: String(message.stage) },
      };
    case "advance_rejected":
      return {
        ...state,
        advanceCommands: {
          ...state.advanceCommands,
          [String(message.command_id)]: {
            command_id: String(message.command_id),
            status: "rejected",
            code: String(message.code),
            reason: String(message.reason),
            attempt_id: null,
            workspace_entry: null,
            inlineError: null,
          } satisfies AdvanceCommandState,
        },
      };
    default:
      return state;
  }
}
