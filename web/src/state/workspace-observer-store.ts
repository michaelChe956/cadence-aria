import type { WorkspaceSessionSummary, WsOutMessage } from "../api/types";
import { workspaceSessionWebSocketUrl } from "../api/client";
import type { CockpitInboxItem } from "./workspace-cockpit-projection";
import { selectCockpitInbox } from "./workspace-cockpit-projection";
import {
  normalizeWorkspaceArtifact,
  pendingChoiceRequestsFromSession,
  workItemPlanProjectionArtifactsFromVersions,
  workItemPlanVersionsFromSession,
} from "./workspace-ws-store-helpers";
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
/** F-06：周期轮降级为假死兜底——距上一帧超过该阈值的观察连接才重建；健康长连接不重连。 */
export const OBSERVER_STALL_THRESHOLD_MS = 5 * 60_000;

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
  /** 每收到一帧上报：无 event_seq 的帧（如 pong）也计入连接活跃；有则同步 cursor。 */
  onFrame?(eventSeq: number | null): void;
  onClose(): void;
  onError(): void;
}

export interface WorkspaceObserverSocketOptions {
  /** 本会话已消费的最大 event_seq；重连 hello 携带它换取服务端增量回放。 */
  resumeEventSeq: number | null;
  /** 断线前已归约的观察态；cursor 回放帧直接在其上续算，无需全量基线。 */
  seedState: WorkspaceWsState | null;
}

export type WorkspaceObserverSocketFactory = (
  sessionId: string,
  callbacks: WorkspaceObserverSocketCallbacks,
  options: WorkspaceObserverSocketOptions,
) => WorkspaceObserverSocket;

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


export function watchWindowCopy(watchLimit: number): string {
  return `仅监视最近 ${watchLimit} 个候选；集合外不计入计数，集合内实时推送（假死连接最多 ${Math.ceil(OBSERVER_STALL_THRESHOLD_MS / 60_000)} 分钟自愈）`;
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

/** 浏览器定时器句柄（DOM 环境下 setTimeout/setInterval 返回 number）。 */
export type ObserverTimer = number;

export interface WorkspaceObserverControllerOptions {
  refreshIntervalMs: number;
  reconnectDelayMs: number;
  schedule?(callback: () => void, delayMs: number): ObserverTimer;
  cancel?(timer: ObserverTimer): void;
  now?(): number;
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
  const reconnectTimers = new Map<string, ObserverTimer>();
  const cursors = new Map<string, number>();
  const lastFrameAt = new Map<string, number>();
  const schedule = options.schedule ?? setTimeout;
  const cancel = options.cancel ?? clearTimeout;
  const now = options.now ?? Date.now;
  let refreshTimer: ObserverTimer | null = null;
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
    lastFrameAt.set(sessionId, now());
    sockets.set(
      sessionId,
      socketFactory(
        sessionId,
        {
          onSnapshot: (state) => {
            if (!disposed && watchedSessionIds.has(sessionId)) {
              snapshots.set(sessionId, state);
              notifyRecordsChanged();
            }
          },
          onFrame: (eventSeq) => {
            if (disposed || !watchedSessionIds.has(sessionId)) {
              return;
            }
            lastFrameAt.set(sessionId, now());
            if (eventSeq !== null) {
              cursors.set(sessionId, Math.max(cursors.get(sessionId) ?? eventSeq, eventSeq));
            }
          },
          onClose: () => scheduleReconnect(sessionId),
          onError: () => scheduleReconnect(sessionId),
        },
        {
          resumeEventSeq: cursors.get(sessionId) ?? null,
          seedState: snapshots.get(sessionId) ?? null,
        },
      ),
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
  // F-06：周期轮降级为假死兜底——只有距上一帧超过阈值的连接才重建，健康长连接
  // （事件流或 ping/pong 存活）永不重连，从而消除周期性全量 attach 基线风暴。
  const scheduleRefresh = () => {
    if (disposed || refreshIntervalMs <= 0) {
      return;
    }
    refreshTimer = schedule(() => {
      refreshTimer = null;
      reclaimStalledSockets();
      scheduleRefresh();
    }, refreshIntervalMs);
  };
  const reclaimStalledSockets = () => {
    const currentNow = now();
    for (const sessionId of watchedSessionIds) {
      const lastFrame = lastFrameAt.get(sessionId);
      if (lastFrame !== undefined && currentNow - lastFrame >= OBSERVER_STALL_THRESHOLD_MS) {
        clearSocket(sessionId);
        ensureSocket(sessionId);
      }
    }
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
          cursors.delete(sessionId);
          lastFrameAt.delete(sessionId);
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
      cursors.clear();
      lastFrameAt.clear();
      notifyRecordsChanged();
    },
  };
}

function createWorkspaceObserverSocket(
  sessionId: string,
  callbacks: WorkspaceObserverSocketCallbacks,
  options: WorkspaceObserverSocketOptions,
): WorkspaceObserverSocket {
  const socket = new WebSocket(workspaceSessionWebSocketUrl(sessionId));
  // 断线重连时以 seedState 续算，hello 携带 cursor 换取服务端增量回放而非全量基线。
  let state: WorkspaceWsState | null = options.seedState;
  let lastEventSeq = options.resumeEventSeq;
  let pingTimer: ObserverTimer | null = null;
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
    socket.send(
      JSON.stringify({
        type: "hello",
        session_id: sessionId,
        last_seen_node_id: null,
        role: "observer",
        ...(lastEventSeq !== null ? { after_event_seq: lastEventSeq } : {}),
      }),
    );
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
      const eventSeq = typeof message.event_seq === "number" ? message.event_seq : null;
      // 任何帧（含 pong）都证明连接存活；cursor 只在去重通过后由回调方记单调最大值。
      callbacks.onFrame?.(eventSeq);
      if (eventSeq !== null) {
        if (
          message.type !== "session_state" &&
          lastEventSeq !== null &&
          eventSeq <= lastEventSeq
        ) {
          // cursor 回放与直播的重叠帧按 event_seq 去重（与驾驶连接同一惯例）；
          // session_state 是重基线帧，无条件接受并重置游标。
          return;
        }
        lastEventSeq = eventSeq;
      }
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
  // H2（计划双审裁决）：takeover 观测态同样补全 artifact 轮次与 plan projection，
  // 供 ③ 区计划审批视图消费——session_state 本就携带版本摘要与结构化投影。
  const fullArtifactVersions = (message.artifact_versions ?? []).map(
    (version) => ({ ...version, markdown: version.markdown ?? "" }),
  );
  const artifactVersions =
    message.artifact_version_summaries ?? fullArtifactVersions;
  const { workItemPlanArtifact } = normalizeWorkspaceArtifact(message.artifact);
  const workItemPlanArtifactVersions = workItemPlanVersionsFromSession(
    artifactVersions,
    fullArtifactVersions,
    workItemPlanArtifact,
    message.active_node_id ?? null,
    message.providers.author,
    message.providers.reviewer ?? null,
  );
  return {
    sessionId: message.session_id,
    workspaceType: message.workspace_type,
    stage: message.stage,
    sessionStatus: message.session_status,
    flowKind: message.flow_kind,
    humanGateSnapshot: message.human_gate_snapshot ?? null,
    humanGateTurn: null,
    planRepair: message.plan_repair ?? null,
    humanGateClosure: null,
    pendingReviewerSummary: null,
    chatEntries: message.messages.map((message, index) => ({
      id: `message:${index}`,
      type: "provider_stream",
      role: message.role === "user" ? "user" : "system",
      content: message.content,
      timestamp: message.created_at,
    })),
    // F-59：观测态同样带 pending_choice_requests 归一投影——驾驶连接缺席
    //（issue_0002 现场：日志全 observer）时，驾驶舱观测视图的等待提示条
    // 仍可见。
    pendingChoiceRequests: pendingChoiceRequestsFromSession(
      (message as Record<string, unknown>).pending_choice_requests,
      [],
      false,
    ),
    timelineNodes,
    activeNodeId: message.active_node_id ?? null,
    selectedNodeId: message.active_node_id ?? timelineNodes.at(-1)?.node_id ?? null,
    protocolDiagnostics: [],
    protocolError: null,
    error: null,
    advanceCommands: {},
    snapshotGateOpenedAt: message.human_gate_snapshot ? new Date().toISOString() : null,
    artifactVersions,
    workItemPlanArtifactVersions,
    workItemPlanProjectionArtifacts:
      workItemPlanProjectionArtifactsFromVersions(workItemPlanArtifactVersions),
  } as unknown as WorkspaceWsState;
}

export function reduceObserverMessage(
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
