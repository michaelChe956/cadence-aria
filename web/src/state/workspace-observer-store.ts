import type { WorkspaceSessionSummary, WsOutMessage } from "../api/types";
import { workspaceSessionWebSocketUrl } from "../api/client";
import type { CockpitInboxItem } from "./workspace-cockpit-projection";
import { selectCockpitInbox } from "./workspace-cockpit-projection";
import type {
  AdvanceCommandState,
  HumanGateTurnState,
  WorkspaceWsState,
} from "./workspace-ws-store";

const WATCHED_SESSION_STATUSES: ReadonlySet<WorkspaceSessionSummary["status"]> = new Set([
  "open",
  "running",
  "waiting_for_human",
  "confirmed",
  "change_requested",
  "stopped_needs_human",
  "failed",
]);

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

export type WorkspaceObserverSocketFactory = (
  sessionId: string,
  onSnapshot: (state: WorkspaceWsState) => void,
) => WorkspaceObserverSocket;

export interface WorkspaceObserverController {
  replaceWatchedSessionIds(sessionIds: readonly string[]): Promise<void>;
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
  return `仅监视最近 ${watchLimit} 个候选；集合外实时卡壳不计入计数`;
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
): WorkspaceObserverController {
  const sockets = new Map<string, WorkspaceObserverSocket>();
  const snapshots = new Map<string, WorkspaceWsState>();

  const notifyRecordsChanged = () => {
    onRecordsChange(Array.from(snapshots, ([sessionId, state]) => ({ sessionId, state })));
  };

  return {
    async replaceWatchedSessionIds(sessionIds) {
      const nextIds = new Set(sessionIds);
      let changed = false;
      for (const [sessionId, socket] of sockets) {
        if (!nextIds.has(sessionId)) {
          socket.close();
          sockets.delete(sessionId);
          snapshots.delete(sessionId);
          changed = true;
        }
      }

      for (const sessionId of sessionIds) {
        if (sockets.has(sessionId)) {
          continue;
        }
        sockets.set(
          sessionId,
          socketFactory(sessionId, (state) => {
            snapshots.set(sessionId, state);
            notifyRecordsChanged();
          }),
        );
      }

      if (changed) {
        notifyRecordsChanged();
      }
    },

    records() {
      return Array.from(snapshots, ([sessionId, state]) => ({ sessionId, state }));
    },

    dispose() {
      for (const socket of sockets.values()) {
        socket.close();
      }
      sockets.clear();
      snapshots.clear();
      notifyRecordsChanged();
    },
  };
}

function createWorkspaceObserverSocket(
  sessionId: string,
  onSnapshot: (state: WorkspaceWsState) => void,
): WorkspaceObserverSocket {
  const socket = new WebSocket(workspaceSessionWebSocketUrl(sessionId));
  let state: WorkspaceWsState | null = null;

  socket.onopen = () => {
    socket.send(
      JSON.stringify({
        type: "hello",
        session_id: sessionId,
        last_seen_node_id: null,
      }),
    );
  };
  socket.onmessage = (event) => {
    try {
      const message = JSON.parse(event.data) as WorkspaceObserverMessage;
      if (message.type === "session_state" && message.session_id === sessionId) {
        state = observerStateFromSessionState(message as WorkspaceSessionStateMessage);
        onSnapshot(state);
        return;
      }
      if (state) {
        state = reduceObserverMessage(state, message);
        onSnapshot(state);
      }
    } catch {
      // observer 仅消费可识别帧，畸形帧不能影响当前工作区连接。
    }
  };

  return socket;
}

function observerStateFromSessionState(
  message: WorkspaceSessionStateMessage,
): WorkspaceWsState {
  return {
    sessionId: message.session_id,
    stage: message.stage,
    sessionStatus: message.session_status,
    humanGateSnapshot: message.human_gate_snapshot ?? null,
    humanGateTurn: null,
    humanGateClosure: null,
    flowKind: message.flow_kind,
    pendingReviewerSummary: null,
    chatEntries: [],
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
      return {
        ...state,
        protocolError: { code: String(message.code), message: String(message.message) },
      };
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
        humanGateClosure: {
          decision: message.decision,
          stage: String(message.stage),
        },
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
